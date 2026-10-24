//! Media clock and frame picker for streamed video.
//!
//! Pure data: no I/O, threads or GPU. A decoder worker streams frames at a constant
//! frame rate, each tagged `(epoch, index)`. A frame's media time is
//! `start_secs + index / fps`, where `start_secs` is where the current epoch began
//! (open or seek). A seek bumps the epoch so older frames are rejected.
//!
//! The consumer calls [`StreamClock::advance`] once per render frame, then offers the
//! next queued frame with [`StreamClock::offer`] until it answers [`FrameVerdict::Keep`]
//! (the last `Show` wins). It polls [`StreamClock::wants_seek`] and, when that returns a
//! time, restarts the decoder there and calls [`StreamClock::seek`].
//!
//! Forward play only: `Reverse` and `PingPong` are not supported for streamed clips, so
//! `TransportState::direction` is ignored and treated as `Forward`. The clock reads
//! `playing`, `looping` and `speed`; it keeps its own duration, and `position` is output
//! only (use [`StreamClock::seek`] to jump).
//!
//! Looping streams (`-stream_loop -1`) keep counting frame indexes past the clip end, so
//! the wanted time is not wrapped internally. [`StreamClock::position_secs`] wraps it.
//! Non-looping clips clamp on the last frame and report [`StreamClock::finished`]; the
//! caller then clears `TransportState::playing`, as `MediaLayer::advance` does.
//!
//! Until the first frame of an epoch is shown the clock holds still, so a slow decoder
//! start is not mistaken for a stall and does not trigger a seek loop.
//!
//! Not yet wired into the app.
#![allow(dead_code)]

use super::types::TransportState;

/// Default lag (seconds) before the clock asks for a seek instead of waiting.
pub const DEFAULT_CATCH_UP_SECS: f64 = 0.5;

/// Tolerance for float error when comparing frame times (0.1 ms).
const EPS_SECS: f64 = 1e-4;

/// What to do with an offered frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameVerdict {
    /// Display this frame now.
    Show,
    /// Not due yet: keep it pending and offer it again next tick.
    Keep,
    /// Stale epoch, already shown, already passed, or past the end: discard it.
    Drop,
}

/// Clock state for one streamed clip.
#[derive(Debug)]
pub struct StreamClock {
    fps: f64,
    duration_secs: f64,
    catch_up_secs: f64,
    /// Number of frames in one pass of the clip.
    frame_count: u64,
    /// Looping flag from the latest `advance` (a new clock loops, like `TransportState`).
    looping: bool,
    epoch: u64,
    /// Media time of index 0 in the current epoch.
    start_secs: f64,
    /// Wanted media time, not wrapped for looping clips.
    wanted_secs: f64,
    /// True once a frame of the current epoch has been shown.
    anchored: bool,
    /// Highest index shown in the current epoch.
    last_shown: Option<u64>,
    /// Media time of the newest frame offered in the current epoch.
    newest_secs: Option<f64>,
}

impl StreamClock {
    /// Returns `None` when `fps` or `duration_secs` is not a positive finite number
    /// (no frames to pick, and the maths would divide by zero). A threshold that is
    /// not a finite non-negative number falls back to [`DEFAULT_CATCH_UP_SECS`].
    pub fn new(fps: f64, duration_secs: f64, catch_up_secs: f64) -> Option<Self> {
        if !(fps.is_finite() && fps > 0.0 && duration_secs.is_finite() && duration_secs > 0.0) {
            return None;
        }
        let catch_up_secs = if catch_up_secs.is_finite() && catch_up_secs >= 0.0 {
            catch_up_secs
        } else {
            DEFAULT_CATCH_UP_SECS
        };
        let frame_count = ((duration_secs * fps) - 1e-6).ceil().max(1.0) as u64;
        Some(Self {
            fps,
            duration_secs,
            catch_up_secs,
            frame_count,
            looping: true,
            epoch: 0,
            start_secs: 0.0,
            wanted_secs: 0.0,
            anchored: false,
            last_shown: None,
            newest_secs: None,
        })
    }

    /// Current epoch. The decoder tags its frames with this after a seek.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Media time of index 0 in the current epoch (where the decoder should start).
    pub fn start_secs(&self) -> f64 {
        self.start_secs
    }

    /// Move the wanted time forward by `dt_secs * speed`. Does nothing while paused,
    /// before the first frame of the epoch is shown, or for a bad `dt` or `speed`.
    pub fn advance(&mut self, dt_secs: f32, transport: &TransportState) {
        self.looping = transport.looping;
        if self.anchored && transport.playing && dt_secs.is_finite() && transport.speed.is_finite()
        {
            let step = dt_secs.max(0.0) as f64 * transport.speed.max(0.0) as f64;
            self.wanted_secs += step;
        }
        if !self.looping {
            self.wanted_secs = self.wanted_secs.min(self.duration_secs);
        }
    }

    /// Decide what to do with the frame `(epoch, index)`.
    pub fn offer(&mut self, epoch: u64, index: u64) -> FrameVerdict {
        if epoch != self.epoch {
            return FrameVerdict::Drop;
        }
        let time = self.start_secs + index as f64 / self.fps;
        if !self.looping && time > self.last_frame_secs() + EPS_SECS {
            return FrameVerdict::Drop; // padding or loop-around past the end
        }
        if self.last_shown.is_some_and(|last| index <= last) {
            return FrameVerdict::Drop;
        }
        let period = 1.0 / self.fps;
        self.newest_secs = Some(self.newest_secs.map_or(time, |n| n.max(time)));

        if !self.anchored {
            // First frame of the epoch is always shown; the clock starts from it.
            self.anchored = true;
            self.last_shown = Some(index);
            self.wanted_secs = self.wanted_secs.max(time);
            return FrameVerdict::Show;
        }

        let target = self.target_secs();
        if time > target + EPS_SECS {
            FrameVerdict::Keep
        } else if time + period <= target + EPS_SECS {
            FrameVerdict::Drop
        } else {
            self.last_shown = Some(index);
            FrameVerdict::Show
        }
    }

    /// Start a new epoch at `to_secs` (clamped into the clip, wrapped when looping).
    /// Returns the new epoch; frames of older epochs are dropped from now on.
    pub fn seek(&mut self, to_secs: f64) -> u64 {
        let to = if !to_secs.is_finite() {
            0.0
        } else if self.looping {
            to_secs.rem_euclid(self.duration_secs)
        } else {
            to_secs.clamp(0.0, self.last_frame_secs())
        };
        self.epoch = self.epoch.wrapping_add(1);
        self.start_secs = to;
        self.wanted_secs = to;
        self.anchored = false;
        self.last_shown = None;
        self.newest_secs = None;
        self.epoch
    }

    /// Time to seek to when the newest decoded frame trails the wanted time by more
    /// than the threshold (4x speed, a stalled decoder, a parked clip resuming).
    /// Call [`StreamClock::seek`] with it; the query then goes quiet.
    pub fn wants_seek(&self) -> Option<f64> {
        let newest = self.newest_secs?;
        let target = self.target_secs();
        if self.anchored && target - newest > self.catch_up_secs {
            Some(self.position_secs())
        } else {
            None
        }
    }

    /// Wanted position inside the clip, in `[0, duration)`.
    pub fn position_secs(&self) -> f64 {
        let target = self.target_secs();
        if self.looping {
            target.rem_euclid(self.duration_secs)
        } else {
            target
        }
    }

    /// True once a non-looping clip has played to its end.
    pub fn finished(&self) -> bool {
        !self.looping && self.wanted_secs >= self.duration_secs - EPS_SECS
    }

    /// Media time of the last frame of one pass.
    fn last_frame_secs(&self) -> f64 {
        (self.frame_count - 1) as f64 / self.fps
    }

    /// Time the picker aims at: the wanted time, held on the last frame when not looping.
    fn target_secs(&self) -> f64 {
        if self.looping {
            self.wanted_secs
        } else {
            self.wanted_secs.min(self.last_frame_secs())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT_60: f32 = 1.0 / 60.0;
    /// The fake decoder runs dry after this many frames.
    const STREAM_END: u64 = 1000;

    fn transport(playing: bool, looping: bool, speed: f32) -> TransportState {
        TransportState {
            playing,
            looping,
            speed,
            ..TransportState::default()
        }
    }

    /// One render tick: advance, then drain the decoder queue the way the consumer will.
    /// `next` is the next queued index. Returns the frame shown this tick, if any.
    fn tick(
        clock: &mut StreamClock,
        t: &TransportState,
        next: &mut u64,
        dropped: &mut Vec<u64>,
    ) -> Option<u64> {
        clock.advance(DT_60, t);
        let mut shown = None;
        while *next < STREAM_END {
            match clock.offer(clock.epoch(), *next) {
                FrameVerdict::Show => {
                    shown = Some(*next);
                    *next += 1;
                }
                FrameVerdict::Drop => {
                    dropped.push(*next);
                    *next += 1;
                }
                FrameVerdict::Keep => break,
            }
        }
        shown
    }

    fn run(clock: &mut StreamClock, t: &TransportState, ticks: usize) -> (Vec<u64>, Vec<u64>) {
        let (mut next, mut shown, mut dropped) = (0, Vec::new(), Vec::new());
        for _ in 0..ticks {
            shown.extend(tick(clock, t, &mut next, &mut dropped));
        }
        (shown, dropped)
    }

    #[test]
    fn thirty_fps_at_sixty_hz_shows_each_frame_once_in_order() {
        let mut c = StreamClock::new(30.0, 10.0, 0.5).unwrap();
        let (shown, dropped) = run(&mut c, &transport(true, true, 1.0), 180);
        // 179 advances after the first tick = 2.983 s -> indexes 0..=89.
        assert_eq!(shown, (0..=89).collect::<Vec<_>>());
        assert!(dropped.is_empty());
    }

    #[test]
    fn speed_two_at_thirty_fps_steps_one_frame_per_tick() {
        // 2x of 30 fps is 60 frames/s, exactly the tick rate: nothing is skipped.
        let mut c = StreamClock::new(30.0, 10.0, 0.5).unwrap();
        let (shown, dropped) = run(&mut c, &transport(true, true, 2.0), 90);
        assert_eq!(shown, (0..90).collect::<Vec<_>>());
        assert!(dropped.is_empty());
    }

    #[test]
    fn speed_four_at_thirty_fps_skips_every_other_frame() {
        let mut c = StreamClock::new(30.0, 10.0, 0.5).unwrap();
        let (shown, dropped) = run(&mut c, &transport(true, true, 4.0), 60);
        // Wanted advances two frames per tick.
        assert_eq!(shown, (0..60).map(|k| 2 * k).collect::<Vec<_>>());
        assert_eq!(dropped, (0..59).map(|k| 2 * k + 1).collect::<Vec<_>>());
    }

    #[test]
    fn speed_two_at_sixty_fps_skips_every_other_frame() {
        let mut c = StreamClock::new(60.0, 10.0, 0.5).unwrap();
        let (shown, dropped) = run(&mut c, &transport(true, true, 2.0), 60);
        assert_eq!(shown, (0..60).map(|k| 2 * k).collect::<Vec<_>>());
        assert_eq!(dropped, (0..59).map(|k| 2 * k + 1).collect::<Vec<_>>());
    }

    #[test]
    fn paused_holds_current_frame() {
        let mut c = StreamClock::new(30.0, 10.0, 0.5).unwrap();
        let (mut next, mut dropped) = (0, Vec::new());
        let play = transport(true, true, 1.0);
        for _ in 0..10 {
            tick(&mut c, &play, &mut next, &mut dropped);
        }
        let pos = c.position_secs();
        let pause = transport(false, true, 1.0);
        for _ in 0..120 {
            assert_eq!(tick(&mut c, &pause, &mut next, &mut dropped), None);
        }
        assert_eq!(c.position_secs(), pos);
        assert!(dropped.is_empty());
        assert!(c.wants_seek().is_none());
        // Resuming carries on from the same frame.
        let shown = tick(&mut c, &play, &mut next, &mut dropped);
        assert!(shown.is_none() || shown == Some(next - 1));
        assert!(c.position_secs() > pos);
    }

    #[test]
    fn paused_clock_still_shows_the_first_frame() {
        let mut c = StreamClock::new(30.0, 10.0, 0.5).unwrap();
        let pause = transport(false, true, 1.0);
        c.advance(DT_60, &pause);
        assert_eq!(c.offer(0, 0), FrameVerdict::Show);
        assert_eq!(c.offer(0, 1), FrameVerdict::Keep);
    }

    #[test]
    fn looping_keeps_showing_and_position_wraps() {
        // 1 s clip at 30 fps; indexes keep counting past 29.
        let mut c = StreamClock::new(30.0, 1.0, 0.5).unwrap();
        let t = transport(true, true, 1.0);
        let (mut next, mut dropped, mut shown) = (0, Vec::new(), Vec::new());
        for _ in 0..150 {
            shown.extend(tick(&mut c, &t, &mut next, &mut dropped));
            let pos = c.position_secs();
            assert!((0.0..1.0).contains(&pos), "position {pos}");
        }
        assert_eq!(shown, (0..=74).collect::<Vec<_>>()); // 149/60 s = 2.483 s
        assert!(!c.finished());
        assert!((c.position_secs() - (149.0 / 60.0) % 1.0).abs() < 1e-3);
    }

    #[test]
    fn non_looping_clamps_on_last_frame_and_finishes() {
        let mut c = StreamClock::new(30.0, 1.0, 0.5).unwrap();
        let t = transport(true, false, 1.0);
        let (mut next, mut dropped, mut shown) = (0, Vec::new(), Vec::new());
        for i in 0..120 {
            shown.extend(tick(&mut c, &t, &mut next, &mut dropped));
            if i < 50 {
                assert!(!c.finished(), "finished early at tick {i}");
            }
        }
        assert_eq!(shown, (0..30).collect::<Vec<_>>());
        assert!(c.finished());
        // Frames past the end (stream padding or loop-around) are dropped.
        assert!(dropped.is_empty() || dropped.iter().all(|&i| i >= 30));
        assert_eq!(c.offer(0, 30), FrameVerdict::Drop);
        assert!((c.position_secs() - 29.0 / 30.0).abs() < 1e-9);
        assert!(c.position_secs() < 1.0);
        assert!(c.wants_seek().is_none());
    }

    #[test]
    fn non_looping_seek_into_middle_drops_frames_past_the_end() {
        // 60 s clip at 30 fps (1800 frames); seek to 30 s, so index 900 is the last frame.
        let mut c = StreamClock::new(30.0, 60.0, 0.5).unwrap();
        let t = transport(true, false, 1.0);
        c.advance(DT_60, &t);
        c.seek(30.0);
        let (mut next, mut dropped, mut shown) = (0, Vec::new(), Vec::new());
        for i in 0..2400 {
            shown.extend(tick(&mut c, &t, &mut next, &mut dropped));
            if i < 1700 {
                assert!(!c.finished(), "finished early at tick {i}");
            }
        }
        assert_eq!(shown.last(), Some(&899));
        assert!(c.finished());
        assert!(dropped.iter().all(|&i| i >= 900), "dropped {dropped:?}");
        assert_eq!(c.offer(1, 900), FrameVerdict::Drop);
        assert_eq!(c.offer(1, 1799), FrameVerdict::Drop);
        assert!(c.position_secs() < 60.0);
    }

    #[test]
    fn lag_beyond_threshold_requests_seek() {
        let mut c = StreamClock::new(30.0, 100.0, 0.5).unwrap();
        let t = transport(true, true, 1.0);
        assert_eq!(c.offer(0, 0), FrameVerdict::Show);
        // Decoder stalls; wanted time runs 0.4 s ahead of the newest frame.
        for _ in 0..24 {
            c.advance(DT_60, &t);
        }
        assert!(c.wants_seek().is_none());
        for _ in 0..12 {
            c.advance(DT_60, &t);
        }
        let to = c.wants_seek().expect("0.6 s lag should ask for a seek");
        assert!((to - 0.6).abs() < 1e-3);
        // Seeking quiets the query and starts a new epoch.
        assert_eq!(c.seek(to), 1);
        assert!(c.wants_seek().is_none());
    }

    #[test]
    fn fast_speed_requests_seek_when_decoder_cannot_keep_up() {
        let mut c = StreamClock::new(30.0, 100.0, 0.5).unwrap();
        let t = transport(true, true, 4.0);
        assert_eq!(c.offer(0, 0), FrameVerdict::Show);
        // Decoder delivers only 30 frames/s while the wanted time moves 4x faster.
        let mut next = 1;
        let mut asked = None;
        for k in 0..60 {
            c.advance(DT_60, &t);
            if k % 2 == 0 {
                c.offer(0, next);
                next += 1;
            }
            if let Some(to) = c.wants_seek() {
                asked = Some(to);
                break;
            }
        }
        assert!(asked.is_some());
    }

    #[test]
    fn seek_rejects_old_epoch_and_shows_first_new_frame() {
        let mut c = StreamClock::new(30.0, 100.0, 0.5).unwrap();
        let t = transport(true, true, 1.0);
        for i in 0..4 {
            c.advance(DT_60, &t);
            c.offer(0, i);
        }
        let epoch = c.seek(40.0);
        assert_eq!(epoch, 1);
        assert_eq!(c.start_secs(), 40.0);
        assert!((c.position_secs() - 40.0).abs() < 1e-9);
        // Leftovers from the old epoch.
        assert_eq!(c.offer(0, 4), FrameVerdict::Drop);
        assert_eq!(c.offer(0, 1000), FrameVerdict::Drop);
        // The clock holds until the first new frame arrives.
        for _ in 0..120 {
            c.advance(DT_60, &t);
        }
        assert!(c.wants_seek().is_none());
        assert_eq!(c.offer(1, 0), FrameVerdict::Show);
        assert_eq!(c.offer(1, 0), FrameVerdict::Drop); // already shown
        assert_eq!(c.offer(1, 1), FrameVerdict::Keep);
    }

    #[test]
    fn seek_wraps_when_looping_and_clamps_when_not() {
        let mut c = StreamClock::new(30.0, 10.0, 0.5).unwrap();
        c.seek(25.0);
        assert!((c.start_secs() - 5.0).abs() < 1e-9);
        c.seek(-1.0);
        assert!((c.start_secs() - 9.0).abs() < 1e-9);
        c.advance(DT_60, &transport(true, false, 1.0));
        c.seek(25.0);
        assert!((c.start_secs() - 9.9667).abs() < 1e-3); // last frame (299/30)
        c.seek(f64::NAN);
        assert_eq!(c.start_secs(), 0.0);
    }

    #[test]
    fn invalid_fps_or_duration_is_rejected() {
        for (fps, dur) in [
            (0.0, 10.0),
            (30.0, 0.0),
            (-30.0, 10.0),
            (30.0, -1.0),
            (f64::NAN, 10.0),
            (30.0, f64::INFINITY),
        ] {
            assert!(StreamClock::new(fps, dur, 0.5).is_none(), "{fps} {dur}");
        }
    }

    #[test]
    fn bad_threshold_falls_back_to_default() {
        for th in [f64::NAN, -1.0, f64::INFINITY] {
            let c = StreamClock::new(30.0, 10.0, th).unwrap();
            assert_eq!(c.catch_up_secs, DEFAULT_CATCH_UP_SECS);
        }
        assert_eq!(
            StreamClock::new(30.0, 10.0, 0.0).unwrap().catch_up_secs,
            0.0
        );
    }

    #[test]
    fn bad_dt_speed_and_direction_do_not_move_the_clock() {
        let mut c = StreamClock::new(30.0, 10.0, 0.5).unwrap();
        assert_eq!(c.offer(0, 0), FrameVerdict::Show);
        for (dt, speed) in [
            (f32::NAN, 1.0),
            (DT_60, f32::NAN),
            (-1.0, 1.0),
            (DT_60, -2.0),
        ] {
            c.advance(dt, &transport(true, true, speed));
            assert_eq!(c.position_secs(), 0.0, "dt {dt} speed {speed}");
        }
        // Reverse is treated as Forward.
        let rev = TransportState {
            direction: super::super::types::PlayDirection::Reverse,
            ..TransportState::default()
        };
        c.advance(0.1, &rev);
        assert!((c.position_secs() - 0.1).abs() < 1e-6);
    }
}
