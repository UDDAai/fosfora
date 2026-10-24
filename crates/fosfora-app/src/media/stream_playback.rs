//! Playback of a streamed video clip: the consumer that joins
//! [`StreamClock`] (which frame to show) and [`StreamDecoder`] (the frames).
//!
//! A media layer owns one [`StreamPlayback`] and calls [`StreamPlayback::tick`] once per
//! render frame. `tick` advances the clock, pulls queued frames with `try_recv` (it never
//! blocks the render thread) and returns the bytes of a new frame only when one should
//! be uploaded; `None` means keep the current texture. The shown frame stays in its
//! pool buffer until the next shown frame replaces it, so RAM is
//! [`StreamPlayback::resident_bytes`] whatever the clip length.
//!
//! One path starts a new epoch: a seek. It comes from the operator
//! ([`StreamPlayback::seek`]), from the clock falling behind the decoder (catch-up),
//! from a retry after a stream error, or from a looping change that needs a fresh
//! stream. At most one seek runs per tick; the first reason in that order wins. After a
//! seek the clock holds until the first frame of the new epoch arrives, and shows it
//! even while paused.
//!
//! Looping is the producer's job (`-stream_loop`), fixed when a stream is opened. Turning
//! it on mid-clip takes effect when the stream ends (the end is then a restart at the
//! current position). Turning it off after the stream has wrapped restarts it at the
//! wrapped position without looping; before the wrap it only stops the decoder from being
//! drained past the end.
//!
//! An error keeps the last frame on screen, is recorded in
//! [`StreamPlayback::last_error`] and is retried once by seeking to the current position.
//! If that fails too the clock freezes on the last frame until the operator seeks. The
//! retry is re-armed after [`RETRY_REARM_SECS`] of frames, so a stream that breaks at
//! the same spot cannot loop forever.
//!
//! Not yet wired into the app.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use super::stream_clock::{DEFAULT_CATCH_UP_SECS, FrameVerdict, StreamClock};
use super::stream_decoder::{FrameSource, PooledBuffer, StreamDecoder, StreamFrame, StreamMsg};
use super::types::TransportState;

/// How long [`StreamPlayback::open`] waits for the first frame.
pub const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(3);

/// Seconds of shown frames after a retry before another failure may be retried again.
pub const RETRY_REARM_SECS: f64 = 2.0;

/// Tolerance for float error when comparing media times (0.1 ms), as in the clock.
const EPS_SECS: f64 = 1e-4;

/// Why a seek happens; earlier variants win when several are due in one tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SeekReason {
    Operator,
    /// A looping change needs a fresh stream.
    Restart,
    /// The stream failed.
    Retry,
    /// The decoder fell behind the clock.
    CatchUp,
}

/// Counters for the controller and the tests.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StreamStats {
    /// Milliseconds from `open` to the first frame.
    pub first_frame_ms: f64,
    /// Seeks by the operator or to catch up with the clock.
    pub seeks: u64,
    /// Seeks that restart the stream: retry after an error, looping changes.
    pub restarts: u64,
    /// Decoded frames that were never shown (late, superseded, or skipped at speed).
    pub dropped_frames: u64,
}

/// One streamed clip, ticked from the render thread.
pub struct StreamPlayback {
    clock: StreamClock,
    decoder: StreamDecoder,
    fps: f64,
    duration_secs: f64,
    /// The frame on screen. A pool buffer on loan, so it is counted in the pool size.
    held: PooledBuffer,
    /// A frame the clock said to `Keep`, offered again next tick.
    pending: Option<StreamFrame>,
    /// Looping flag last seen (and handed to the decoder).
    looping: bool,
    /// The `looping` flag the current stream was opened with.
    stream_looping: bool,
    /// Media time of the shown frame, counted from its epoch's start (not wrapped).
    last_shown_secs: f64,
    operator_seek: Option<f64>,
    /// The stream has nothing more to give: it ended, or ran past the end of a
    /// non-looping clip. Cleared by a seek.
    ended: bool,
    /// An error is waiting for its retry.
    retry_due: bool,
    retry_spent: bool,
    shown_since_retry: u64,
    /// The retry failed too: the clock is frozen until the operator seeks.
    failed: bool,
    last_error: Option<String>,
    stats: StreamStats,
}

impl StreamPlayback {
    /// Start streaming at 0 and wait up to [`FIRST_FRAME_TIMEOUT`] for the first frame,
    /// which is then available from [`StreamPlayback::current_frame`]. Fails with the
    /// decoder's error text, or on a timeout.
    pub fn open<P>(
        width: u32,
        height: u32,
        fps: f64,
        duration_secs: f64,
        looping: bool,
        producer: P,
    ) -> Result<Self, String>
    where
        P: FnMut(f64, bool) -> Result<FrameSource, String> + Send + 'static,
    {
        Self::open_with_timeout(
            width,
            height,
            fps,
            duration_secs,
            looping,
            producer,
            FIRST_FRAME_TIMEOUT,
        )
    }

    fn open_with_timeout<P>(
        width: u32,
        height: u32,
        fps: f64,
        duration_secs: f64,
        looping: bool,
        producer: P,
        timeout: Duration,
    ) -> Result<Self, String>
    where
        P: FnMut(f64, bool) -> Result<FrameSource, String> + Send + 'static,
    {
        let mut clock = StreamClock::new(fps, duration_secs, DEFAULT_CATCH_UP_SECS)
            .ok_or_else(|| format!("Invalid stream timing: {fps} fps, {duration_secs} s"))?;
        // A zero step only tells the clock whether it loops (a new clock does).
        clock.advance(
            0.0,
            &TransportState {
                looping,
                ..TransportState::default()
            },
        );
        let decoder = StreamDecoder::spawn(
            width,
            height,
            clock.epoch(),
            clock.start_secs(),
            looping,
            producer,
        )?;

        let started = Instant::now();
        let held = loop {
            let left = timeout.saturating_sub(started.elapsed());
            match decoder.recv_timeout(left) {
                Some(StreamMsg::Frame(frame)) => {
                    if clock.offer(frame.epoch, frame.index) == FrameVerdict::Show {
                        break frame.data;
                    }
                }
                Some(StreamMsg::End { .. }) => {
                    return Err("stream ended before the first frame".to_string());
                }
                Some(StreamMsg::Error { message, .. }) => return Err(message),
                None => {
                    return Err(format!(
                        "No video frame within {:.1} s",
                        timeout.as_secs_f64()
                    ));
                }
            }
        };

        Ok(Self {
            clock,
            decoder,
            fps,
            duration_secs,
            held,
            pending: None,
            looping,
            stream_looping: looping,
            last_shown_secs: 0.0,
            operator_seek: None,
            ended: false,
            retry_due: false,
            retry_spent: false,
            shown_since_retry: 0,
            failed: false,
            last_error: None,
            stats: StreamStats {
                first_frame_ms: started.elapsed().as_secs_f64() * 1000.0,
                ..StreamStats::default()
            },
        })
    }

    /// The frame on screen: RGBA8, `width * height * 4` bytes.
    pub fn current_frame(&self) -> &[u8] {
        &self.held
    }

    /// Advance by `dt_secs` and return the bytes of a new frame to upload, if any.
    /// `transport.direction` is ignored (forward only) and `position` is output only;
    /// when [`StreamPlayback::finished`] turns true the caller clears `playing`.
    pub fn tick(&mut self, dt_secs: f32, transport: &TransportState) -> Option<&[u8]> {
        // Seeks due before the drain: the operator's, and a looping change.
        let mut seek = self.operator_seek.take().map(|s| (SeekReason::Operator, s));
        if transport.looping != self.looping {
            // Read the position while the clock still wraps.
            let pos = self.clock.position_secs();
            if !transport.looping && self.stream_looping && self.wrapped() {
                seek = seek.or(Some((SeekReason::Restart, pos)));
            }
            self.decoder.set_looping(transport.looping);
            self.looping = transport.looping;
        }
        // Frozen after a failed retry. The clock also learns the looping flag here,
        // which the seek below relies on (a non-looping seek clamps into the clip).
        let dt = if self.failed { 0.0 } else { dt_secs };
        self.clock.advance(dt, transport);
        let mut seeked = false;
        if let Some((reason, secs)) = seek {
            self.do_seek(secs, reason);
            seeked = true;
        }

        let shown = self.drain();

        if !seeked {
            let due = if self.retry_due {
                Some((SeekReason::Retry, self.clock.position_secs()))
            } else if self.ended && self.looping {
                Some((SeekReason::Restart, self.clock.position_secs()))
            } else if self.ended || self.failed {
                None
            } else {
                self.clock.wants_seek().map(|s| (SeekReason::CatchUp, s))
            };
            if let Some((reason, secs)) = due {
                self.do_seek(secs, reason);
            }
        }

        if shown { Some(&self.held) } else { None }
    }

    /// Jump to `secs` (wrapped into the clip when looping, clamped otherwise). It runs
    /// on the next tick; several calls before then collapse into the last. Also gives
    /// a failed stream a fresh start.
    pub fn seek(&mut self, secs: f64) {
        self.operator_seek = Some(secs);
    }

    /// True once a non-looping clip has played to its end.
    pub fn finished(&self) -> bool {
        self.clock.finished() || (self.ended && !self.looping)
    }

    /// The latest stream error, until a frame shows again.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Position inside the clip, in `[0, duration)`.
    pub fn position_secs(&self) -> f64 {
        self.clock.position_secs()
    }

    pub fn duration_secs(&self) -> f64 {
        self.duration_secs
    }

    /// Frames in one pass of the clip, from the container's duration and frame rate.
    pub fn estimated_frames(&self) -> u64 {
        (self.duration_secs * self.fps).round().max(1.0) as u64
    }

    /// RAM held for playback: the decoder's frame pool, fixed whatever the clip length.
    /// The shown and pending frames are pool buffers on loan, so they are included.
    pub fn resident_bytes(&self) -> usize {
        self.decoder.resident_bytes()
    }

    pub fn stats(&self) -> StreamStats {
        self.stats
    }

    /// Pull queued messages until the clock says wait, the queue is empty or the stream
    /// is over. True when a frame was shown (the last `Show` wins).
    fn drain(&mut self) -> bool {
        let mut shown = false;
        // Nothing more to read from an ended stream, so a looping stream that ran past
        // the end of a clip that stopped looping is left to fill its queue and block.
        while !self.ended {
            let frame = match self.pending.take() {
                Some(frame) => frame,
                None => match self.decoder.try_recv() {
                    Some(StreamMsg::Frame(frame)) => frame,
                    Some(StreamMsg::End { epoch }) => {
                        self.ended |= epoch == self.clock.epoch();
                        continue;
                    }
                    Some(StreamMsg::Error { epoch, message }) => {
                        if epoch == self.clock.epoch() {
                            self.on_error(message);
                        }
                        continue;
                    }
                    None => break,
                },
            };
            let current = frame.epoch == self.clock.epoch();
            let secs = self.clock.start_secs() + frame.index as f64 / self.fps;
            if current && !self.looping && secs + EPS_SECS >= self.duration_secs {
                self.ended = true; // a looping stream ran on past the end
                break;
            }
            match self.clock.offer(frame.epoch, frame.index) {
                FrameVerdict::Show => {
                    // An earlier Show of this tick was superseded before it was uploaded.
                    self.stats.dropped_frames += u64::from(shown);
                    shown = true;
                    self.show(frame, secs);
                }
                FrameVerdict::Keep => {
                    self.pending = Some(frame);
                    break;
                }
                FrameVerdict::Drop => self.stats.dropped_frames += u64::from(current),
            }
        }
        shown
    }

    fn show(&mut self, frame: StreamFrame, secs: f64) {
        self.last_shown_secs = secs;
        self.held = frame.data; // the previous frame goes back to the pool
        self.last_error = None;
        if self.retry_spent {
            self.shown_since_retry += 1;
            if self.shown_since_retry as f64 >= RETRY_REARM_SECS * self.fps {
                self.retry_spent = false;
            }
        }
    }

    fn on_error(&mut self, message: String) {
        self.last_error = Some(message);
        if self.retry_spent {
            self.failed = true;
        } else {
            self.retry_due = true;
        }
    }

    /// True when the shown frame lies past the end of the clip: the stream has wrapped.
    fn wrapped(&self) -> bool {
        self.last_shown_secs + EPS_SECS >= self.duration_secs
    }

    /// The one path that starts a new epoch.
    fn do_seek(&mut self, secs: f64, reason: SeekReason) {
        let epoch = self.clock.seek(secs);
        self.decoder.seek(epoch, self.clock.start_secs());
        self.stream_looping = self.looping;
        self.pending = None; // frame of the old epoch: back to the pool
        self.ended = false;
        self.retry_due = false;
        match reason {
            SeekReason::Operator => {
                self.failed = false;
                self.retry_spent = false;
                self.stats.seeks += 1;
            }
            SeekReason::CatchUp => self.stats.seeks += 1,
            SeekReason::Restart => self.stats.restarts += 1,
            SeekReason::Retry => {
                self.retry_spent = true;
                self.shown_since_retry = 0;
                self.stats.restarts += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::stream_decoder::{POOL_BUFFERS, QUEUE_DEPTH};
    use crossbeam_channel::{Receiver, bounded};
    use std::io::{self, Read};
    use std::sync::{Arc, Mutex};

    const W: u32 = 2;
    const H: u32 = 2;
    const FRAME_BYTES: usize = 16;
    const FPS: f64 = 30.0;
    const DT: f32 = 1.0 / 60.0;
    /// Generous: every wait polls and returns as soon as its condition holds.
    const WAIT: Duration = Duration::from_secs(10);

    /// Stream of generated frames. Frame bytes: `start_idx` and the index since the
    /// stream's start as u32 little endian, then filler, so a test can tell which
    /// frame of the clip it is looking at.
    struct FakeClip {
        start_idx: u32,
        idx: u32,
        pos: usize,
        /// EOF after this many frames; `None` is endless (a looping stream).
        total: Option<u32>,
        /// Read error instead of this frame.
        fail_at: Option<u32>,
        /// Block instead of delivering this frame, until the kill hook fires.
        stall_at: Option<(u32, Receiver<()>)>,
    }

    impl Read for FakeClip {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos == 0 {
                if self.total.is_some_and(|t| self.idx >= t) {
                    return Ok(0);
                }
                if self.fail_at == Some(self.idx) {
                    return Err(io::Error::other("fake stream broke"));
                }
                if let Some((at, rx)) = &self.stall_at
                    && *at == self.idx
                {
                    let _ = rx.recv();
                    return Ok(0);
                }
            }
            let mut frame = [0xCDu8; FRAME_BYTES];
            frame[0..4].copy_from_slice(&self.start_idx.to_le_bytes());
            frame[4..8].copy_from_slice(&self.idx.to_le_bytes());
            let n = buf.len().min(FRAME_BYTES - self.pos);
            buf[..n].copy_from_slice(&frame[self.pos..self.pos + n]);
            self.pos += n;
            if self.pos == FRAME_BYTES {
                self.pos = 0;
                self.idx += 1;
            }
            Ok(n)
        }
    }

    /// Frame of the whole clip a frame's bytes stand for.
    fn abs_index(bytes: &[u8]) -> u64 {
        let start = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let idx = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        u64::from(start) + u64::from(idx)
    }

    /// How a fake stream misbehaves.
    #[derive(Clone, Copy, Default)]
    struct Opts {
        fail_at: Option<u32>,
        stall_at: Option<u32>,
    }

    fn frames_in(duration: f64) -> u32 {
        (duration * FPS - 1e-6).ceil() as u32
    }

    fn source(duration: f64, start: f64, looping: bool, opts: Opts) -> FrameSource {
        let start_idx = (start * FPS).round() as u32;
        let (tx, rx) = bounded::<()>(0);
        FrameSource {
            reader: Box::new(FakeClip {
                start_idx,
                idx: 0,
                pos: 0,
                total: (!looping).then(|| frames_in(duration).saturating_sub(start_idx)),
                fail_at: opts.fail_at,
                stall_at: opts.stall_at.map(|at| (at, rx)),
            }),
            kill: Box::new(move || drop(tx)),
        }
    }

    type Calls = Arc<Mutex<Vec<(f64, bool)>>>;

    /// Producer that records `(start, looping)` and lets `script(call_number)` pick the
    /// behaviour of each call (an `Err` fails the call).
    fn producer<F>(
        duration: f64,
        calls: Calls,
        mut script: F,
    ) -> impl FnMut(f64, bool) -> Result<FrameSource, String> + Send + 'static
    where
        F: FnMut(usize) -> Result<Opts, String> + Send + 'static,
    {
        move |start, looping| {
            let n = {
                let mut c = calls.lock().unwrap();
                c.push((start, looping));
                c.len() - 1
            };
            script(n).map(|opts| source(duration, start, looping, opts))
        }
    }

    fn start_with<F>(duration: f64, looping: bool, script: F) -> (StreamPlayback, Calls)
    where
        F: FnMut(usize) -> Result<Opts, String> + Send + 'static,
    {
        let calls = Calls::default();
        let p = producer(duration, calls.clone(), script);
        let pb = StreamPlayback::open(W, H, FPS, duration, looping, p).unwrap();
        (pb, calls)
    }

    fn start(duration: f64, looping: bool) -> (StreamPlayback, Calls) {
        start_with(duration, looping, |_| Ok(Opts::default()))
    }

    fn transport(playing: bool, looping: bool) -> TransportState {
        TransportState {
            playing,
            looping,
            ..TransportState::default()
        }
    }

    fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Wait until a tick has something to look at, so a slow decoder thread never makes
    /// the virtual clock outrun it (which would trigger a catch-up seek).
    fn wait_ready(pb: &StreamPlayback) {
        wait_for("the decoder", || {
            pb.pending.is_some() || pb.decoder.queued() > 0 || pb.ended || pb.failed
        });
    }

    /// One paced tick at 60 Hz. Returns the clip index of the frame to upload.
    fn step(pb: &mut StreamPlayback, t: &TransportState) -> Option<u64> {
        wait_ready(pb);
        pb.tick(DT, t).map(abs_index)
    }

    /// Paced ticks until `done`, collecting the shown frame indexes.
    fn step_until(
        pb: &mut StreamPlayback,
        t: &TransportState,
        mut done: impl FnMut(&StreamPlayback) -> bool,
    ) -> Vec<u64> {
        let mut shown = Vec::new();
        let deadline = Instant::now() + WAIT;
        while !done(pb) {
            assert!(Instant::now() < deadline, "timed out; shown {shown:?}");
            shown.extend(step(pb, t));
        }
        shown
    }

    fn assert_consecutive(shown: &[u64], first: u64) {
        let want: Vec<u64> = (first..first + shown.len() as u64).collect();
        assert_eq!(shown, want.as_slice());
    }

    #[test]
    fn open_delivers_frame_zero_immediately() {
        let (pb, calls) = start(10.0, false);
        assert_eq!(pb.current_frame().len(), FRAME_BYTES);
        assert_eq!(abs_index(pb.current_frame()), 0);
        assert_eq!(pb.position_secs(), 0.0);
        assert_eq!(pb.duration_secs(), 10.0);
        assert_eq!(pb.estimated_frames(), 300);
        assert!(!pb.finished());
        assert!(pb.last_error().is_none());
        assert!(pb.stats().first_frame_ms > 0.0);
        assert_eq!(*calls.lock().unwrap(), vec![(0.0, false)]);
    }

    #[test]
    fn open_returns_the_producer_error() {
        let r = StreamPlayback::open(W, H, FPS, 10.0, false, |_, _| Err("no such file".into()));
        assert_eq!(r.err().as_deref(), Some("no such file"));
    }

    #[test]
    fn open_times_out_when_nothing_arrives() {
        let calls = Calls::default();
        let p = producer(10.0, calls, |_| {
            Ok(Opts {
                stall_at: Some(0),
                ..Opts::default()
            })
        });
        let t = Instant::now();
        let r = StreamPlayback::open_with_timeout(
            W,
            H,
            FPS,
            10.0,
            false,
            p,
            Duration::from_millis(300),
        );
        let err = r.err().expect("open should fail");
        assert!(err.contains("No video frame"), "{err}");
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        assert_eq!(FIRST_FRAME_TIMEOUT, Duration::from_secs(3));
    }

    #[test]
    fn open_rejects_bad_timing_and_size() {
        let r = StreamPlayback::open(W, H, 0.0, 10.0, false, |_, _| Err("unused".into()));
        assert!(r.err().is_some_and(|e| e.contains("Invalid stream timing")));
        let r = StreamPlayback::open(0, H, FPS, 10.0, false, |_, _| Err("unused".into()));
        assert!(r.err().is_some_and(|e| e.contains("frame size")));
    }

    #[test]
    fn sixty_hz_over_thirty_fps_shows_each_frame_once_in_order() {
        let (mut pb, _) = start(10.0, false);
        let t = transport(true, false);
        let mut shown = Vec::new();
        for _ in 0..180 {
            shown.extend(step(&mut pb, &t));
        }
        assert_consecutive(&shown, 1);
        assert!((88..=90).contains(&shown.len()), "{} frames", shown.len());
        assert_eq!(pb.stats().dropped_frames, 0);
        assert_eq!(pb.stats().seeks, 0);
        assert_eq!(abs_index(pb.current_frame()), *shown.last().unwrap());
        assert!((pb.position_secs() - 3.0).abs() < 0.05);
    }

    #[test]
    fn paused_shows_nothing_and_a_paused_seek_shows_the_target() {
        let (mut pb, calls) = start(10.0, false);
        let paused = transport(false, false);
        for _ in 0..30 {
            assert_eq!(step(&mut pb, &paused), None);
        }
        assert_eq!(pb.position_secs(), 0.0);
        assert_eq!(abs_index(pb.current_frame()), 0);

        pb.seek(2.0);
        let shown = step_until(&mut pb, &paused, |pb| abs_index(pb.current_frame()) == 60);
        assert_eq!(shown, vec![60]);
        assert_eq!(pb.position_secs(), 2.0);
        // Still paused: the clock holds on the target.
        for _ in 0..30 {
            assert_eq!(step(&mut pb, &paused), None);
        }
        assert_eq!(pb.position_secs(), 2.0);
        assert_eq!(*calls.lock().unwrap(), vec![(0.0, false), (2.0, false)]);
    }

    #[test]
    fn several_seeks_in_one_tick_collapse_into_one() {
        let (mut pb, calls) = start(10.0, false);
        let paused = transport(false, false);
        pb.seek(1.0);
        pb.seek(3.0);
        pb.seek(2.0);
        let shown = step_until(&mut pb, &paused, |pb| abs_index(pb.current_frame()) == 60);
        assert_eq!(shown, vec![60]);
        assert_eq!(pb.stats().seeks, 1);
        assert_eq!(*calls.lock().unwrap(), vec![(0.0, false), (2.0, false)]);
    }

    #[test]
    fn looping_keeps_playing_across_the_end_and_position_wraps() {
        let (mut pb, calls) = start(1.0, true);
        let t = transport(true, true);
        let mut shown = Vec::new();
        for _ in 0..150 {
            shown.extend(step(&mut pb, &t));
            let pos = pb.position_secs();
            assert!((0.0..1.0).contains(&pos), "position {pos}");
        }
        // Indexes keep counting past the 30 frames of the clip.
        assert_consecutive(&shown, 1);
        assert!(shown.last().unwrap() > &70, "{shown:?}");
        assert!(!pb.finished());
        assert!((pb.position_secs() - 0.5).abs() < 0.05);
        assert_eq!(*calls.lock().unwrap(), vec![(0.0, true)]);
        assert_eq!(pb.stats().restarts, 0);
    }

    #[test]
    fn non_looping_reaches_finished_and_stops_on_the_last_frame() {
        let (mut pb, _) = start(1.0, false);
        let t = transport(true, false);
        let shown = step_until(&mut pb, &t, |pb| pb.finished());
        assert_consecutive(&shown, 1);
        // The last frame of the 30 is on screen once the clip is finished.
        for _ in 0..20 {
            assert_eq!(step(&mut pb, &t), None);
        }
        assert!(pb.finished());
        assert_eq!(abs_index(pb.current_frame()), 29);
        assert!(pb.position_secs() < 1.0);
        assert!(pb.last_error().is_none());
    }

    #[test]
    fn a_lagging_producer_makes_playback_seek_ahead() {
        // The first stream stalls after three frames and never recovers.
        let (mut pb, calls) = start_with(60.0, false, |n| {
            Ok(Opts {
                stall_at: (n == 0).then_some(3),
                ..Opts::default()
            })
        });
        let t = transport(true, false);
        // Virtual time runs on with no new frames: about half a second behind, the
        // playback gives up on the stream and seeks.
        let deadline = Instant::now() + WAIT;
        while pb.stats().seeks == 0 {
            assert!(Instant::now() < deadline, "no catch-up seek");
            pb.tick(DT, &t);
            std::thread::sleep(Duration::from_millis(1));
        }
        wait_for("the second producer call", || {
            calls.lock().unwrap().len() == 2
        });
        let to = calls.lock().unwrap()[1].0;
        assert!(to > 0.4 && to < 3.0, "seek target {to}");

        // The new stream is healthy: playback carries on from the target.
        let first = (to * FPS).round() as u64;
        let shown = step_until(&mut pb, &t, |pb| abs_index(pb.current_frame()) >= first + 5);
        assert!(shown.windows(2).all(|w| w[0] < w[1]), "{shown:?}");
        assert!(shown.first().unwrap() >= &first, "{shown:?}");
        assert!(pb.stats().seeks >= 1);
        assert_eq!(pb.stats().restarts, 0);
    }

    #[test]
    fn resident_bytes_do_not_depend_on_clip_length() {
        let (mut short, _) = start(2.0, false); // 60 frames
        let (mut long, _) = start(20_000.0, true); // 600,000 frames
        assert_eq!(short.estimated_frames(), 60);
        assert_eq!(long.estimated_frames(), 600_000);
        assert_eq!(short.resident_bytes(), long.resident_bytes());
        assert_eq!(short.resident_bytes(), POOL_BUFFERS * FRAME_BYTES);

        for _ in 0..120 {
            step(&mut short, &transport(true, false));
            step(&mut long, &transport(true, true));
        }
        assert_eq!(short.resident_bytes(), long.resident_bytes());
        assert_eq!(long.decoder.pool_allocations(), POOL_BUFFERS);
        assert_eq!(short.decoder.pool_allocations(), POOL_BUFFERS);
    }

    #[test]
    fn looping_off_after_a_wrap_restarts_once_and_finishes() {
        let (mut pb, calls) = start(1.0, true);
        let on = transport(true, true);
        step_until(&mut pb, &on, |pb| abs_index(pb.current_frame()) >= 40);
        assert_eq!(pb.stats().restarts, 0);

        let off = transport(true, false);
        step_until(&mut pb, &off, |pb| pb.finished());
        assert_eq!(pb.stats().restarts, 1);
        // The stream restarted at the wrapped position, without looping.
        {
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 2, "{calls:?}");
            assert!(!calls[1].1);
            assert!((0.0..1.0).contains(&calls[1].0), "{calls:?}");
        }
        // Played out to the last frame of the clip, then stopped.
        wait_for("the last frame", || {
            pb.tick(DT, &off);
            abs_index(pb.current_frame()) == 29
        });
        for _ in 0..20 {
            assert_eq!(step(&mut pb, &off), None);
        }
        assert_eq!(pb.stats().restarts, 1);
        assert!(pb.finished());
        assert!(pb.last_error().is_none());
    }

    #[test]
    fn looping_off_before_a_wrap_needs_no_restart_and_stops_draining() {
        let (mut pb, calls) = start(1.0, true);
        let on = transport(true, true);
        step_until(&mut pb, &on, |pb| abs_index(pb.current_frame()) >= 5);
        let off = transport(true, false);
        step_until(&mut pb, &off, |pb| pb.ended);
        assert_eq!(pb.stats().restarts, 0);
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(pb.finished());

        // The still-looping stream is no longer read: its queue fills and it blocks.
        wait_for("a full queue", || pb.decoder.queued() == QUEUE_DEPTH);
        for _ in 0..20 {
            assert_eq!(pb.tick(DT, &off), None);
        }
        assert_eq!(pb.decoder.queued(), QUEUE_DEPTH);
        assert_eq!(abs_index(pb.current_frame()), 29);
    }

    #[test]
    fn looping_on_after_the_end_restarts_the_stream() {
        let (mut pb, calls) = start(1.0, false);
        let off = transport(true, false);
        step_until(&mut pb, &off, |pb| pb.finished());

        let on = transport(true, true);
        step_until(&mut pb, &on, |pb| pb.stats().restarts == 1);
        assert!(!pb.finished());
        let shown = step_until(&mut pb, &on, |pb| abs_index(pb.current_frame()) >= 35);
        assert!(!shown.is_empty());
        assert_eq!(pb.stats().restarts, 1);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].1, "{calls:?}");
    }

    #[test]
    fn an_error_keeps_the_frame_retries_once_and_exposes_the_error() {
        // Frames 0..=9 arrive, then the stream breaks; the retry is refused.
        let (mut pb, calls) = start_with(10.0, false, |n| match n {
            0 => Ok(Opts {
                fail_at: Some(10),
                ..Opts::default()
            }),
            _ => Err("retry refused".to_string()),
        });
        let t = transport(true, false);
        step_until(&mut pb, &t, |pb| pb.last_error() == Some("retry refused"));

        assert_eq!(abs_index(pb.current_frame()), 9);
        assert_eq!(pb.stats().restarts, 1);
        let (retry_at, _) = calls.lock().unwrap()[1];
        assert!((0.2..0.6).contains(&retry_at), "retry at {retry_at}");
        // Failed: the last frame stays, the clock is frozen and nobody retries again.
        let pos = pb.position_secs();
        for _ in 0..60 {
            assert_eq!(pb.tick(DT, &t), None);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(pb.position_secs(), pos);
        assert_eq!(abs_index(pb.current_frame()), 9);
        assert_eq!(calls.lock().unwrap().len(), 2);
        assert_eq!(pb.stats().restarts, 1);
        assert_eq!(pb.last_error(), Some("retry refused"));
    }

    #[test]
    fn a_successful_retry_resumes_and_clears_the_error() {
        let (mut pb, calls) = start_with(10.0, false, |n| {
            Ok(Opts {
                fail_at: (n == 0).then_some(10),
                ..Opts::default()
            })
        });
        let t = transport(true, false);
        step_until(&mut pb, &t, |pb| pb.stats().restarts == 1);
        assert!(pb.last_error().is_some());
        step_until(&mut pb, &t, |pb| abs_index(pb.current_frame()) >= 20);
        assert!(pb.last_error().is_none());
        assert_eq!(pb.stats().restarts, 1);
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn an_operator_seek_gives_a_failed_stream_a_fresh_start() {
        let (mut pb, calls) = start_with(10.0, false, |n| match n {
            0 => Ok(Opts {
                fail_at: Some(10),
                ..Opts::default()
            }),
            1 => Err("retry refused".to_string()),
            _ => Ok(Opts::default()),
        });
        let t = transport(true, false);
        step_until(&mut pb, &t, |pb| pb.last_error() == Some("retry refused"));

        pb.seek(5.0);
        step_until(&mut pb, &t, |pb| abs_index(pb.current_frame()) >= 150);
        assert!(pb.last_error().is_none());
        assert_eq!(calls.lock().unwrap().len(), 3);
        assert_eq!(pb.stats().seeks, 1);
    }
}
