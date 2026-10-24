//! Playback thread. AVFoundation objects stay on that thread.
//!
//! Dropping a clip asks the thread to stop and does not wait. Scene changes
//! therefore do not block the render thread on decoder teardown.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::budget::{frame_bytes, staging_ram_bytes};
use crate::ffi::{self, Player};
use crate::mailbox::{Mailbox, VideoFrame};
use crate::timing::{FrameStamp, ShownClock, at_end, dropped_frames, frame_matches_seek};

static PLAYERS_ALIVE: AtomicUsize = AtomicUsize::new(0);

struct PrimeState {
    done: bool,
    error: Option<String>,
    width: u32,
    height: u32,
    fps: f64,
    duration: f64,
    hardware: i32,
    codec: String,
    note: String,
}

impl PrimeState {
    fn pending() -> Self {
        Self {
            done: false,
            error: None,
            width: 0,
            height: 0,
            fps: 0.0,
            duration: 0.0,
            hardware: -1,
            codec: String::new(),
            note: String::new(),
        }
    }
}

struct Shared {
    stop: AtomicBool,
    mailbox: Mailbox,
    position: Mutex<f64>,
    playing: AtomicBool,
    looping: AtomicBool,
    rate_bits: AtomicU32,
    seek_bits: AtomicU64,
    seek_seq: AtomicU64,
    finished: Mutex<bool>,
    finish_cv: Condvar,
    primed: Mutex<PrimeState>,
    prime_cv: Condvar,
    frames: AtomicU64,
    dropped: AtomicU64,
    loops: AtomicU64,
    copy_ns: AtomicU64,
    allocations: AtomicU64,
    first_ns: AtomicU64,
    metal: Mutex<Option<Sender<Result<[u8; 4], String>>>>,
    direction_warned: AtomicBool,
}

impl Shared {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            stop: AtomicBool::new(false),
            mailbox: Mailbox::default(),
            position: Mutex::new(0.0),
            playing: AtomicBool::new(true),
            looping: AtomicBool::new(true),
            rate_bits: AtomicU32::new(1.0_f32.to_bits()),
            seek_bits: AtomicU64::new(0),
            seek_seq: AtomicU64::new(0),
            finished: Mutex::new(false),
            finish_cv: Condvar::new(),
            primed: Mutex::new(PrimeState::pending()),
            prime_cv: Condvar::new(),
            frames: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            loops: AtomicU64::new(0),
            copy_ns: AtomicU64::new(0),
            allocations: AtomicU64::new(0),
            first_ns: AtomicU64::new(0),
            metal: Mutex::new(None),
            direction_warned: AtomicBool::new(false),
        })
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn rate(&self) -> f32 {
        f32::from_bits(self.rate_bits.load(Ordering::Relaxed))
    }

    fn set_position(&self, secs: f64) {
        *self.position.lock().unwrap_or_else(|err| err.into_inner()) = secs;
    }

    fn fail(&self, message: String) {
        let mut primed = self.primed.lock().unwrap_or_else(|err| err.into_inner());
        if !primed.done {
            primed.done = true;
            primed.error = Some(message);
            self.prime_cv.notify_all();
        }
    }
}

struct AliveGuard {
    shared: Arc<Shared>,
}

impl AliveGuard {
    fn enter(shared: Arc<Shared>) -> Self {
        PLAYERS_ALIVE.fetch_add(1, Ordering::Release);
        Self { shared }
    }
}

impl Drop for AliveGuard {
    fn drop(&mut self) {
        {
            let mut primed = self
                .shared
                .primed
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if !primed.done {
                primed.done = true;
                if primed.error.is_none() {
                    primed.error = Some("playback stopped before the first frame".to_string());
                }
                self.shared.prime_cv.notify_all();
            }
        }
        *self
            .shared
            .finished
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = true;
        self.shared.finish_cv.notify_all();
        PLAYERS_ALIVE.fetch_sub(1, Ordering::Release);
    }
}

struct Pump {
    scratch: Vec<u8>,
    seen_seek: u64,
    applied_rate: f32,
    latched: bool,
    last_pts: Option<f64>,
    gate: Option<(f64, Instant)>,
    width: u32,
    height: u32,
    fps: f64,
    duration: f64,
    primed: bool,
    started: Instant,
    clock: ShownClock,
}

pub struct PlayStats {
    pub frames: u64,
    pub dropped: u64,
    pub loops: u64,
    pub allocations: u64,
    pub first_frame: Duration,
    pub average_copy: Duration,
    pub resident_bytes: u64,
    pub hardware: i32,
    pub codec: String,
    pub note: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_secs: f64,
}

pub struct NativeClip {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
    width: u32,
    height: u32,
    fps: f64,
    duration: f64,
    hardware: i32,
    codec: String,
    note: String,
}

impl NativeClip {
    pub fn open(path: &Path) -> Result<Self, String> {
        let shared = Shared::new();
        let thread_shared = Arc::clone(&shared);
        let path = path.to_path_buf();
        let handle = thread::Builder::new()
            .name("fosfora-native-video".into())
            .spawn(move || worker(thread_shared, path))
            .map_err(|err| format!("could not start video playback: {err}"))?;
        let primed = wait_for_prime(&shared, Duration::from_secs(8));
        if let Err(err) = primed {
            shared.stop.store(true, Ordering::Release);
            let _ = handle.join();
            return Err(err);
        }
        let primed = primed?;
        log::info!(
            "native video: {}x{} {:.2} fps {:.2}s codec {} hardware {} ({})",
            primed.width,
            primed.height,
            primed.fps,
            primed.duration,
            primed.codec,
            primed.hardware,
            primed.note
        );
        Ok(Self {
            shared,
            handle: Some(handle),
            width: primed.width,
            height: primed.height,
            fps: primed.fps,
            duration: primed.duration,
            hardware: primed.hardware,
            codec: primed.codec,
            note: primed.note,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn fps(&self) -> f64 {
        self.fps
    }

    pub fn duration_secs(&self) -> f64 {
        self.duration
    }

    pub fn hardware(&self) -> i32 {
        self.hardware
    }

    pub fn note(&self) -> &str {
        &self.note
    }

    pub fn codec(&self) -> &str {
        &self.codec
    }

    pub fn resident_bytes(&self) -> u64 {
        staging_ram_bytes(self.width, self.height).unwrap_or(0)
    }

    pub fn sync(&self, playing: bool, looping: bool, speed: f32) {
        self.shared.playing.store(playing, Ordering::Relaxed);
        self.shared.looping.store(looping, Ordering::Relaxed);
        let speed = if speed.is_finite() {
            speed.clamp(0.0, 4.0)
        } else {
            1.0
        };
        self.shared
            .rate_bits
            .store(speed.to_bits(), Ordering::Relaxed);
    }

    pub fn seek(&self, secs: f64) {
        let mut secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        if self.duration > 0.0 {
            secs = secs.min(self.duration);
        }
        self.shared.seek_bits.store(secs.to_bits(), Ordering::Relaxed);
        self.shared.set_position(secs);
        self.shared.mailbox.invalidate();
        self.shared.seek_seq.fetch_add(1, Ordering::Release);
    }

    pub fn note_forward_only(&self) {
        if self
            .shared
            .direction_warned
            .swap(true, Ordering::Relaxed)
        {
            return;
        }
        log::warn!(
            "native video plays forward only; reverse and ping-pong need the full-frame decoder"
        );
    }

    /// Latest frame, or nothing if the playback thread is publishing right now.
    /// Does not wait.
    pub fn latest_frame(&self) -> Option<Arc<VideoFrame>> {
        self.shared.mailbox.try_latest()
    }

    pub fn try_position(&self) -> Option<f64> {
        self.shared.position.try_lock().ok().map(|secs| *secs)
    }

    /// Samples one pixel through a `CVMetalTextureCache`. The playback thread
    /// does the GPU wait. Not used by the frame renderer.
    pub fn metal_preview(&self) -> Result<[u8; 4], String> {
        let (tx, rx) = mpsc::channel();
        {
            let mut slot = self.shared.metal.lock().unwrap_or_else(|err| err.into_inner());
            if slot.is_some() {
                return Err("a Metal preview is already running".to_string());
            }
            *slot = Some(tx);
        }
        rx.recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| Err("Metal preview timed out".to_string()))
    }

    pub fn stats(&self) -> PlayStats {
        let frames = self.shared.frames.load(Ordering::Relaxed);
        let copy_ns = self.shared.copy_ns.load(Ordering::Relaxed);
        let average = if frames == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos(copy_ns / frames)
        };
        PlayStats {
            frames,
            dropped: self.shared.dropped.load(Ordering::Relaxed),
            loops: self.shared.loops.load(Ordering::Relaxed),
            allocations: self.shared.allocations.load(Ordering::Relaxed),
            first_frame: Duration::from_nanos(self.shared.first_ns.load(Ordering::Relaxed)),
            average_copy: average,
            resident_bytes: self.resident_bytes(),
            hardware: self.hardware,
            codec: self.codec.clone(),
            note: self.note.clone(),
            width: self.width,
            height: self.height,
            fps: self.fps,
            duration_secs: self.duration,
        }
    }

    /// Block until the playback thread has released its player.
    pub fn shutdown(mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for NativeClip {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

pub fn players_alive() -> usize {
    PLAYERS_ALIVE.load(Ordering::Acquire)
}

pub fn phys_footprint() -> u64 {
    ffi::footprint()
}

fn wait_for_prime(shared: &Shared, limit: Duration) -> Result<PrimeState, String> {
    let deadline = Instant::now() + limit;
    let mut primed = shared.primed.lock().unwrap_or_else(|err| err.into_inner());
    loop {
        if primed.done {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let (next, _) = shared
            .prime_cv
            .wait_timeout(primed, deadline.saturating_duration_since(now))
            .unwrap_or_else(|err| err.into_inner());
        primed = next;
    }
    if !primed.done {
        return Err("timed out waiting for the first frame".to_string());
    }
    if let Some(err) = primed.error.clone() {
        return Err(err);
    }
    Ok(PrimeState {
        done: true,
        error: None,
        width: primed.width,
        height: primed.height,
        fps: primed.fps,
        duration: primed.duration,
        hardware: primed.hardware,
        codec: primed.codec.clone(),
        note: primed.note.clone(),
    })
}

fn worker(shared: Arc<Shared>, path: std::path::PathBuf) {
    let _alive = AliveGuard::enter(Arc::clone(&shared));
    let (player, info) = match Player::open(&path) {
        Ok(opened) => opened,
        Err(err) => {
            shared.fail(err);
            return;
        }
    };
    let mut pump = Pump {
        scratch: Vec::new(),
        seen_seek: 0,
        applied_rate: f32::NAN,
        latched: false,
        last_pts: None,
        gate: None,
        width: info.width,
        height: info.height,
        fps: if info.fps > 1.0 { info.fps } else { 30.0 },
        duration: info.duration_secs,
        primed: false,
        started: Instant::now(),
        clock: ShownClock::new(),
    };
    loop {
        if shared.stopped() {
            break;
        }
        if !pump.primed && pump.started.elapsed() > Duration::from_secs(3) {
            shared.fail("timed out waiting for the first frame".to_string());
            break;
        }
        if let Err(err) = pump_once(&player, &shared, &mut pump, &info) {
            shared.fail(err);
            break;
        }
        player.pump(0.004);
    }
}

fn pump_once(
    player: &Player,
    shared: &Shared,
    pump: &mut Pump,
    info: &ffi::MediaInfo,
) -> Result<(), String> {
    service_metal(player, shared);
    apply_seek(player, shared, pump);
    apply_loop(player, shared, pump);
    apply_rate(player, shared, pump);
    copy_frame(player, shared, pump, info)
}

fn service_metal(player: &Player, shared: &Shared) {
    let request = shared
        .metal
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take();
    if let Some(reply) = request {
        let _ = reply.send(player.metal_sample());
    }
}

fn apply_seek(player: &Player, shared: &Shared, pump: &mut Pump) {
    let seq = shared.seek_seq.load(Ordering::Acquire);
    if seq == pump.seen_seek {
        return;
    }
    pump.seen_seek = seq;
    let target = f64::from_bits(shared.seek_bits.load(Ordering::Relaxed));
    player.seek(target);
    pump.gate = Some((target, Instant::now()));
    pump.applied_rate = f32::NAN;
    pump.latched = false;
    pump.clock.seek(target);
    pump.last_pts = None;
}

fn apply_loop(player: &Player, shared: &Shared, pump: &mut Pump) {
    let position = player.position();
    let slop = (1.0 / pump.fps).clamp(0.01, 0.05);
    let playing = shared.playing.load(Ordering::Relaxed) && shared.rate() > 0.0;
    if !pump.latched && shared.looping.load(Ordering::Relaxed) && playing && at_end(position, pump.duration, slop)
    {
        pump.latched = true;
        shared.loops.fetch_add(1, Ordering::Relaxed);
        player.seek(0.0);
        pump.gate = Some((0.0, Instant::now()));
        pump.applied_rate = f32::NAN;
        pump.last_pts = None;
        pump.clock.seek(0.0);
        shared.set_position(0.0);
        return;
    }
    if position < 0.08 {
        pump.latched = false;
    }
}

fn apply_rate(player: &Player, shared: &Shared, pump: &mut Pump) {
    let desired = if shared.playing.load(Ordering::Relaxed) {
        shared.rate()
    } else {
        0.0
    };
    if desired.is_finite() && (desired - pump.applied_rate).abs() <= 0.01 {
        return;
    }
    player.set_rate(f64::from(desired));
    pump.applied_rate = desired;
}

fn copy_frame(
    player: &Player,
    shared: &Shared,
    pump: &mut Pump,
    info: &ffi::MediaInfo,
) -> Result<(), String> {
    let need = frame_bytes(pump.width, pump.height).ok_or_else(|| {
        format!("video frame {}x{} has no usable RGBA size", pump.width, pump.height)
    })?;
    ensure_scratch(&mut pump.scratch, need as usize, &shared.allocations);
    let epoch = shared.mailbox.epoch();
    let seq = shared.seek_seq.load(Ordering::Acquire);
    let started = Instant::now();
    let copied = player.copy_rgba(&mut pump.scratch)?;
    let Some(copied) = copied else {
        return Ok(());
    };
    shared
        .copy_ns
        .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    if shared.seek_seq.load(Ordering::Acquire) != seq {
        return Ok(());
    }
    if let Some((target, since)) = pump.gate
        && !frame_matches_seek(target, copied.pts_secs)
        && since.elapsed() < Duration::from_millis(500)
    {
        return Ok(());
    }
    pump.gate = None;
    if copied.width != pump.width || copied.height != pump.height {
        if pump.primed {
            return Err(format!(
                "frame size changed from {}x{} to {}x{}",
                pump.width, pump.height, copied.width, copied.height
            ));
        }
        pump.width = copied.width;
        pump.height = copied.height;
    }
    let nbytes = usize::try_from(copied.rgba_bytes).unwrap_or(0);
    if nbytes == 0 || nbytes > pump.scratch.len() {
        return Err("copied frame was empty".to_string());
    }
    pump.scratch.truncate(nbytes);
    let rgba = std::mem::take(&mut pump.scratch);
    let serial = shared.frames.fetch_add(1, Ordering::Relaxed);
    let dropped = dropped_frames(pump.last_pts, copied.pts_secs, 1.0 / pump.fps);
    shared.dropped.fetch_add(dropped, Ordering::Relaxed);
    pump.last_pts = Some(copied.pts_secs);
    pump.clock.on_frame(copied.pts_secs);
    if shared.seek_seq.load(Ordering::Acquire) == pump.seen_seek {
        shared.set_position(pump.clock.secs);
    }
    let frame = VideoFrame {
        rgba,
        stamp: FrameStamp {
            pts_secs: copied.pts_secs,
            serial,
            epoch,
            width: copied.width,
            height: copied.height,
        },
    };
    let (result, returned) = shared.mailbox.publish(frame);
    if let Some(returned) = returned {
        recycle_scratch(&mut pump.scratch, returned.rgba, &shared.allocations);
    }
    if !pump.primed && matches!(result, crate::mailbox::Publish::Stored) {
        pump.primed = true;
        shared
            .first_ns
            .store(pump.started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        finish_prime(shared, pump, info);
    }
    Ok(())
}

fn finish_prime(shared: &Shared, pump: &Pump, info: &ffi::MediaInfo) {
    let mut primed = shared.primed.lock().unwrap_or_else(|err| err.into_inner());
    if primed.done {
        return;
    }
    primed.done = true;
    primed.width = pump.width;
    primed.height = pump.height;
    primed.fps = pump.fps;
    primed.duration = pump.duration;
    primed.hardware = info.hardware;
    primed.codec = info.codec.clone();
    primed.note = info.note.clone();
    shared.prime_cv.notify_all();
}

fn ensure_scratch(scratch: &mut Vec<u8>, need: usize, allocations: &AtomicU64) {
    if scratch.len() == need {
        return;
    }
    let grew = scratch.capacity() < need;
    scratch.resize(need, 0);
    if grew {
        allocations.fetch_add(1, Ordering::Relaxed);
    }
}

fn recycle_scratch(scratch: &mut Vec<u8>, mut rgba: Vec<u8>, allocations: &AtomicU64) {
    let need = scratch.len();
    if need == 0 {
        *scratch = rgba;
        return;
    }
    let grew = rgba.capacity() < need;
    rgba.resize(need, 0);
    *scratch = rgba;
    if grew {
        allocations.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::full_rgba_bytes;
    use std::process::Command;
    use std::sync::OnceLock;

    static AV: Mutex<()> = Mutex::new(());

    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn encode(name: &str, seconds: &str, size: &str) -> Option<std::path::PathBuf> {
        static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
        let dir = DIR.get_or_init(|| {
            let dir = std::env::temp_dir().join("fosfora-native-video-tests");
            let _ = std::fs::create_dir_all(&dir);
            dir
        });
        let path = dir.join(name);
        if path.is_file() && std::fs::metadata(&path).map(|meta| meta.len() > 1500).unwrap_or(false) {
            return Some(path);
        }
        if !ffmpeg_available() {
            return None;
        }
        let source = format!("testsrc2=size={size}:rate=30");
        let encoded = |codec: &str, extra: &[&str]| {
            let mut command = Command::new("ffmpeg");
            command.args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                &source,
                "-t",
                seconds,
                "-an",
                "-pix_fmt",
                "yuv420p",
                "-c:v",
                codec,
            ]);
            command.args(extra);
            command.arg(&path);
            command
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        };
        if !encoded("h264_videotoolbox", &["-b:v", "2M"]) && !encoded("libx264", &["-preset", "ultrafast", "-crf", "28"])
        {
            return None;
        }
        path.is_file().then_some(path)
    }

    fn one_second() -> Option<std::path::PathBuf> {
        encode("one-320x180.mp4", "1.0", "320x180")
    }

    fn short_loop() -> Option<std::path::PathBuf> {
        encode("loop-320x180.mp4", "0.45", "320x180")
    }

    fn wait_idle() {
        let start = Instant::now();
        while players_alive() > 0 && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(players_alive(), 0, "a playback thread was still alive");
    }

    #[test]
    fn first_frame_is_ready_and_timestamps_advance() {
        let _guard = AV.lock().unwrap_or_else(|err| err.into_inner());
        let Some(path) = one_second() else {
            eprintln!("skipping native video test; ffmpeg or the encoder is unavailable");
            return;
        };
        let clip = NativeClip::open(&path).expect("open");
        assert!(clip.stats().first_frame < Duration::from_secs(3));
        assert!(clip.latest_frame().is_some(), "first frame was not published");
        assert!(!clip.note().is_empty());
        if clip.hardware() == 0 {
            assert!(clip.note().to_lowercase().contains("not available"));
        }
        let full = full_rgba_bytes(clip.width(), clip.height(), clip.fps(), clip.duration_secs());
        assert!(clip.resident_bytes() * 5 < full);
        assert_eq!(
            clip.resident_bytes(),
            staging_ram_bytes(clip.width(), clip.height()).unwrap()
        );
        let start = Instant::now();
        let mut previous = None;
        while start.elapsed() < Duration::from_millis(450) {
            if let Some(frame) = clip.latest_frame() {
                if let Some(seen) = previous {
                    assert!(
                        frame.stamp.pts_secs + 0.02 >= seen,
                        "timestamp went backwards: {seen} -> {}",
                        frame.stamp.pts_secs
                    );
                }
                previous = Some(frame.stamp.pts_secs);
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(previous.unwrap_or(0.0) > 0.05, "playback did not advance");
        clip.shutdown();
        wait_idle();
    }

    #[test]
    fn pause_seek_and_loop() {
        let _guard = AV.lock().unwrap_or_else(|err| err.into_inner());
        let Some(path) = one_second() else {
            return;
        };
        let clip = NativeClip::open(&path).expect("open");
        thread::sleep(Duration::from_millis(200));
        clip.sync(false, true, 1.0);
        thread::sleep(Duration::from_millis(120));
        let paused = clip.try_position().unwrap_or(0.0);
        thread::sleep(Duration::from_millis(300));
        let still = clip.try_position().unwrap_or(0.0);
        assert!(
            (still - paused).abs() < 0.12,
            "pause did not hold: {paused} -> {still}"
        );
        clip.sync(true, false, 1.0);
        clip.seek(0.62);
        let started = Instant::now();
        let mut landed = false;
        while started.elapsed() < Duration::from_secs(2) {
            if let Some(frame) = clip.latest_frame()
                && frame_matches_seek(0.62, frame.stamp.pts_secs)
            {
                landed = true;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(landed, "seek did not reach 0.62s");
        clip.shutdown();

        let Some(loop_path) = short_loop() else {
            return;
        };
        let looping = NativeClip::open(&loop_path).expect("loop open");
        looping.sync(true, true, 1.0);
        thread::sleep(Duration::from_millis(1400));
        assert!(
            looping.stats().loops >= 1,
            "expected a loop, stats {:?}",
            (
                looping.stats().loops,
                looping.stats().frames,
                looping.duration_secs()
            )
        );
        looping.shutdown();
        wait_idle();
    }

    #[test]
    fn unsupported_file_fails_and_a_later_open_recovers() {
        let _guard = AV.lock().unwrap_or_else(|err| err.into_inner());
        let dir = std::env::temp_dir().join("fosfora-native-video-tests");
        let _ = std::fs::create_dir_all(&dir);
        let bad = dir.join("not-a-video.mp4");
        std::fs::write(&bad, b"this is not an mp4").unwrap();
        let error = NativeClip::open(&bad).expect_err("garbage must not play");
        assert!(!error.is_empty(), "{error}");
        assert_eq!(players_alive(), 0);
        let Some(path) = one_second() else {
            return;
        };
        let clip = NativeClip::open(&path).expect("recovery open");
        assert!(clip.latest_frame().is_some());
        clip.shutdown();
        wait_idle();
    }

    #[test]
    fn replacement_stop_and_three_players_release_memory_owners() {
        let _guard = AV.lock().unwrap_or_else(|err| err.into_inner());
        let Some(path) = one_second() else {
            return;
        };
        for _ in 0..4 {
            let clip = NativeClip::open(&path).expect("reopen");
            assert!(clip.latest_frame().is_some());
            clip.shutdown();
            wait_idle();
        }
        let first = NativeClip::open(&path).expect("first");
        let second = NativeClip::open(&path).expect("second");
        first.shutdown();
        assert!(second.latest_frame().is_some() || second.stats().frames > 0);
        second.shutdown();
        wait_idle();

        let mut clips = Vec::new();
        for _ in 0..3 {
            clips.push(NativeClip::open(&path).expect("trio"));
        }
        assert_eq!(players_alive(), 3);
        let reserved: u64 = clips.iter().map(NativeClip::resident_bytes).sum();
        let full = full_rgba_bytes(320, 180, 30.0, 1.0);
        assert!(reserved * 2 < full * 3);
        drop(clips);
        wait_idle();
    }

    #[test]
    fn metal_preview_samples_a_pixel_without_retaining_the_frame() {
        let _guard = AV.lock().unwrap_or_else(|err| err.into_inner());
        let Some(path) = one_second() else {
            return;
        };
        let clip = NativeClip::open(&path).expect("open");
        let pixel = clip.metal_preview().expect("metal preview");
        assert_eq!(pixel.len(), 4);
        assert!(clip.stats().frames > 0);
        clip.shutdown();
        wait_idle();
    }
}
