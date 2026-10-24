//! ffmpeg child process for streamed video.
//!
//! [`stream_args`] is a pure argument builder; [`spawn_stream`] runs it and returns an
//! [`FfmpegStream`], a `Read + Send` over the child's stdout. The pipe carries raw RGBA
//! frames of exactly `width * height * 4` bytes at a constant `fps`, which is the timing
//! model [`super::stream_clock`] assumes: frame `n` of a stream opened at `start_secs`
//! is media time `start_secs + n / fps`.
//!
//! A looping stream (`-stream_loop -1`) never ends. After the first pass ffmpeg seeks
//! back to the start of the file, not to `start_secs`, so frame `n` is media time
//! `(start_secs + n / fps) mod duration` (checked by `loop_wraps_to_media_start`).
//! `start_secs` must lie inside the clip; the caller wraps it.
//!
//! ffmpeg 9.0.2 has a bug with `-ss` plus `-stream_loop`: each wrap steps the packet
//! timestamps back by `start_secs`, so `-fps_mode cfr` drops frames after the first
//! wrap. For that one combination [`stream_args`] adds `-vf setpts=N/(<source_fps>*TB)`,
//! which restamps frames by count and makes the wrap seamless. It needs
//! [`StreamSpec::source_fps`]; without it the filter is skipped and a looping stream
//! opened with a seek may drop frames after a wrap on ffmpeg 9.x.
//!
//! [`spawn_pipe`] also hands back a kill closure, so another thread can cut a stream
//! short while the worker is blocked reading it; the read then returns EOF at once.
//! If ffmpeg exits non-zero before or during the stream, the read returns an error with
//! its exit status and stderr excerpt rather than a bare EOF.
//!
//! Dropping the stream kills and reaps the child, so abandoning a clip never leaves a
//! decoder running or a zombie behind. The consumer thread (the reader) is expected to
//! call [`spawn_stream`] again with a new start time to seek.
//!
//! Not yet wired into the app.
#![allow(dead_code)]

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::stream_decoder::FrameSource;
use super::toolchain::VideoToolchain;

/// Bytes of ffmpeg stderr kept for error messages (the tail is what explains a failure).
const STDERR_KEEP_BYTES: usize = 4096;

/// Lines of the stderr tail returned by [`FfmpegStream::stderr_excerpt`].
const STDERR_EXCERPT_LINES: usize = 5;

/// Decoder threads. The show shares the machine with the renderer, so keep this small.
const DECODE_THREADS: u32 = 2;

/// Max time to wait for a killed ffmpeg to exit before leaving it to the OS.
const REAP_TIMEOUT: Duration = Duration::from_secs(2);

/// What to stream and how.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamSpec {
    /// Media time (seconds) of the first frame. `0.0` opens at the start.
    pub start_secs: f64,
    /// Loop forever with `-stream_loop -1`; frame indexes keep counting past the end.
    pub looping: bool,
    /// Constant output frame rate.
    pub fps: f64,
    /// Output frame size in pixels. This must be the displayed size (after rotation and
    /// pixel-aspect correction): `-s` scales to exactly this and does not keep aspect.
    pub width: u32,
    pub height: u32,
    /// Opt in to VideoToolbox hardware decode (`-hwaccel videotoolbox`). Off by default:
    /// the software path is the one that has been checked on the show machine.
    pub hwaccel: bool,
    /// ffmpeg older than 5.1 has no `-fps_mode`; it needs `-vsync cfr`. [`spawn_stream`]
    /// sets this from the toolchain version; leave it `false` otherwise.
    pub legacy_vsync: bool,
    /// The clip's constant frame rate, from ffprobe. Used only to restamp a looping
    /// stream opened with a seek (see the module docs); `None` when the clip has no
    /// constant rate, which skips the restamp.
    pub source_fps: Option<f64>,
}

impl StreamSpec {
    pub fn new(fps: f64, width: u32, height: u32) -> Self {
        Self {
            start_secs: 0.0,
            looping: false,
            fps,
            width,
            height,
            hwaccel: false,
            legacy_vsync: false,
            source_fps: None,
        }
    }

    /// Bytes in one output frame.
    pub fn frame_bytes(&self) -> Option<usize> {
        (self.width as usize)
            .checked_mul(self.height as usize)?
            .checked_mul(4)
            .filter(|&n| n > 0)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.fps.is_finite() || self.fps <= 0.0 {
            return Err("Stream has an invalid frame rate".into());
        }
        if !self.start_secs.is_finite() || self.start_secs < 0.0 {
            return Err("Stream has an invalid start time".into());
        }
        if self.source_fps.is_some_and(|f| !f.is_finite() || f <= 0.0) {
            return Err("Stream has an invalid source frame rate".into());
        }
        if self.frame_bytes().is_none() {
            return Err("Stream has invalid frame dimensions".into());
        }
        Ok(())
    }
}

/// Whether an `ffmpeg -version` first line names a release older than 5.1, where
/// `-fps_mode` does not exist. Unparseable versions (git builds such as `N-12345-g...`)
/// count as modern.
pub fn needs_legacy_vsync(version_line: Option<&str>) -> bool {
    let Some(rest) = version_line.and_then(|l| l.trim().strip_prefix("ffmpeg version ")) else {
        return false;
    };
    let rest = rest
        .strip_prefix(['n', 'N'])
        .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(rest);
    let mut parts = rest
        .split(|c: char| !c.is_ascii_digit())
        .map(|p| p.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor) < (5, 1),
        (Some(major), None) => major < 5,
        _ => false,
    }
}

/// Restamps frames by count at the source rate. Needed only for `-ss` with
/// `-stream_loop`, whose looped timestamps go backwards at every wrap (module docs).
fn restamp_filter(source_fps: f64) -> String {
    format!("setpts=N/({source_fps}*TB)")
}

/// Arguments for a streaming ffmpeg run that writes raw RGBA frames to stdout.
///
/// `-ss` goes before `-i` (fast input seek) and only when `start_secs > 0`. The path is a
/// single argument, so spaces and non-ASCII are safe. A relative path gets a `./` prefix,
/// since ffmpeg would read `name: thing.mp4` as a protocol.
pub fn stream_args(path: &Path, spec: &StreamSpec) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::with_capacity(32);
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));

    push(&["-v", "error", "-nostdin"]);
    if spec.hwaccel {
        push(&["-hwaccel", "videotoolbox"]);
    }
    push(&["-threads", &DECODE_THREADS.to_string()]);
    if spec.looping {
        push(&["-stream_loop", "-1"]);
    }
    if spec.start_secs > 0.0 {
        push(&["-ss", &format!("{:.3}", spec.start_secs)]);
    }
    args.push("-i".into());
    if path.is_relative() {
        args.push(Path::new(".").join(path).into_os_string());
    } else {
        args.push(path.as_os_str().to_owned());
    }
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));
    push(&["-map", "0:v:0", "-an", "-sn"]);
    if let Some(fps) = spec
        .source_fps
        .filter(|_| spec.looping && spec.start_secs > 0.0)
    {
        push(&["-vf", &restamp_filter(fps)]);
    }
    if spec.legacy_vsync {
        push(&["-vsync", "cfr"]);
    } else {
        push(&["-fps_mode", "cfr"]);
    }
    push(&[
        "-r",
        &format!("{}", spec.fps),
        "-s",
        &format!("{}x{}", spec.width, spec.height),
        "-pix_fmt",
        "rgba",
        "-f",
        "rawvideo",
        "pipe:1",
    ]);
    args
}

/// Last few KB of a child's stderr, filled by a drain thread.
type StderrTail = Arc<Mutex<Vec<u8>>>;

/// Readable pipe of raw RGBA frames from a long-lived ffmpeg child.
///
/// Reading past a failed run returns an error that carries the ffmpeg exit status and
/// stderr excerpt, so the reader needs no side channel for diagnostics. Dropping the
/// stream kills and waits for the child.
pub struct FfmpegStream {
    /// Shared with the closure from [`FfmpegStream::killer`]; never held across a blocking
    /// read, so a kill from another thread is not stuck behind the reader.
    child: Arc<Mutex<Child>>,
    pid: u32,
    /// Set by a kill, so the resulting signal exit reads as end of stream, not an error.
    killed: Arc<AtomicBool>,
    stdout: ChildStdout,
    stderr_tail: StderrTail,
    stderr_thread: Option<JoinHandle<()>>,
    /// Set once EOF has been seen and the exit status checked.
    finished: bool,
}

/// Start ffmpeg for `path` and return the frame pipe.
pub fn spawn_stream(path: &Path, spec: &StreamSpec) -> Result<FfmpegStream, String> {
    spec.validate()?;
    let tool = VideoToolchain::global(None);
    let mut cmd = tool
        .ffmpeg_cmd()
        .ok_or("ffmpeg not found (set ffmpeg path in settings, or install ffmpeg)")?;
    let mut spec = spec.clone();
    spec.legacy_vsync |= needs_legacy_vsync(tool.ffmpeg_version.as_deref());
    cmd.args(stream_args(path, &spec))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start ffmpeg for {}: {e}", path.display()))?;

    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("ffmpeg: no stdout/stderr pipe".into());
    };
    // Drain stderr so a chatty decoder cannot fill its pipe and stall; keep only a tail.
    let stderr_tail: StderrTail = Arc::default();
    let tail = Arc::clone(&stderr_tail);
    let stderr_thread = std::thread::Builder::new()
        .name("ffmpeg-stream-stderr".into())
        .spawn(move || drain_stderr(stderr, &tail));
    let stderr_thread = match stderr_thread {
        Ok(handle) => Some(handle),
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Failed to spawn ffmpeg stderr thread: {e}"));
        }
    };
    Ok(FfmpegStream {
        pid: child.id(),
        child: Arc::new(Mutex::new(child)),
        killed: Arc::default(),
        stdout,
        stderr_tail,
        stderr_thread,
        finished: false,
    })
}

/// [`spawn_stream`], split into the boxed reader and kill closure the stream decoder
/// takes. `kill` stops the child at once, from any thread, and is safe to call again or
/// after the child has exited.
pub fn spawn_pipe(path: &Path, spec: &StreamSpec) -> Result<FrameSource, String> {
    let stream = spawn_stream(path, spec)?;
    let kill = stream.killer();
    Ok(FrameSource {
        reader: Box::new(stream),
        kill: Box::new(kill),
    })
}

/// A producer for the stream decoder worker: each call opens a stream at `start_secs`.
/// `path`, size and fps are fixed for the clip.
pub fn stream_producer(
    path: PathBuf,
    base: StreamSpec,
) -> impl FnMut(f64, bool) -> Result<FrameSource, String> {
    move |start_secs, looping| {
        let spec = StreamSpec {
            start_secs,
            looping,
            ..base.clone()
        };
        spawn_pipe(&path, &spec)
    }
}

fn lock_child(child: &Mutex<Child>) -> MutexGuard<'_, Child> {
    child.lock().unwrap_or_else(PoisonError::into_inner)
}

fn drain_stderr(mut stderr: impl Read, tail: &Mutex<Vec<u8>>) {
    let mut chunk = [0u8; 4096];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let Ok(mut buf) = tail.lock() else { break };
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > STDERR_KEEP_BYTES {
                    let excess = buf.len() - STDERR_KEEP_BYTES;
                    buf.drain(..excess);
                }
            }
        }
    }
}

impl FfmpegStream {
    /// OS process id of the child.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// A closure that kills the child immediately, from any thread. A read blocked on
    /// this stream then sees EOF (not an error). Safe after the child has exited.
    pub fn killer(&self) -> impl FnOnce() + Send + 'static {
        let child = Arc::clone(&self.child);
        let killed = Arc::clone(&self.killed);
        move || {
            killed.store(true, Ordering::SeqCst);
            let _ = lock_child(&child).kill();
        }
    }

    /// The last few lines ffmpeg wrote to stderr (empty when it said nothing).
    pub fn stderr_excerpt(&self) -> String {
        let Ok(buf) = self.stderr_tail.lock() else {
            return String::new();
        };
        let text = String::from_utf8_lossy(&buf);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let from = lines.len().saturating_sub(STDERR_EXCERPT_LINES);
        lines[from..].join("\n")
    }

    /// Called at EOF: reap the child and turn a failed exit into an error. A clean exit
    /// is a normal end of stream.
    fn on_eof(&mut self) -> std::io::Result<()> {
        self.finished = true;
        // Poll rather than `wait`, so the lock is free for a kill from another thread.
        let status = loop {
            if let Some(status) = lock_child(&self.child).try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        // The pipe is closed, so the drain thread is finishing; join for the full tail.
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
        if status.success() || self.killed.load(Ordering::SeqCst) {
            return Ok(());
        }
        Err(std::io::Error::other(exit_message(
            status,
            &self.stderr_excerpt(),
        )))
    }
}

fn exit_message(status: ExitStatus, excerpt: &str) -> String {
    if excerpt.is_empty() {
        format!("ffmpeg failed ({status})")
    } else {
        format!("ffmpeg failed ({status}): {excerpt}")
    }
}

impl Read for FfmpegStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.finished {
            return Ok(0);
        }
        let n = self.stdout.read(buf)?;
        if n == 0 && !buf.is_empty() {
            self.on_eof()?;
        }
        Ok(n)
    }
}

impl Drop for FfmpegStream {
    fn drop(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
        // Kill under the lock, but wait outside it: a slow teardown (VideoToolbox) must
        // not hold up a kill hook running on another thread.
        let _ = lock_child(&self.child).kill();
        let deadline = Instant::now() + REAP_TIMEOUT;
        let reaped = loop {
            match lock_child(&self.child).try_wait() {
                Ok(Some(_)) | Err(_) => break true,
                Ok(None) if Instant::now() >= deadline => break false,
                Ok(None) => {}
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        if reaped {
            // Killing closed both pipes, which ends the drain thread.
            if let Some(handle) = self.stderr_thread.take() {
                let _ = handle.join();
            }
        } else {
            // The drain thread ends when ffmpeg finally exits; do not wait for it.
            log::warn!(
                "ffmpeg pid {} did not exit within {REAP_TIMEOUT:?} of a kill",
                self.pid
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> StreamSpec {
        StreamSpec::new(30.0, 1280, 720)
    }

    fn strs(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn pos(args: &[String], needle: &str) -> Option<usize> {
        args.iter().position(|a| a == needle)
    }

    /// True when `a b` appear adjacent, in that order.
    fn has_pair(args: &[String], a: &str, b: &str) -> bool {
        args.windows(2).any(|w| w[0] == a && w[1] == b)
    }

    #[test]
    fn base_args_for_a_plain_stream() {
        let args = strs(&stream_args(Path::new("/clips/a.mp4"), &spec()));
        assert_eq!(&args[..3], ["-v", "error", "-nostdin"]);
        assert!(has_pair(&args, "-fps_mode", "cfr"));
        assert!(has_pair(&args, "-r", "30"));
        assert!(has_pair(&args, "-s", "1280x720"));
        assert!(has_pair(&args, "-pix_fmt", "rgba"));
        assert!(has_pair(&args, "-threads", "2"));
        assert!(has_pair(&args, "-map", "0:v:0"));
        assert!(args.contains(&"-an".to_string()));
        assert!(args.contains(&"-sn".to_string()));
        assert!(has_pair(&args, "-f", "rawvideo"));
        assert_eq!(args.last().unwrap(), "pipe:1");
        assert!(!args.contains(&"-vsync".to_string()));
        assert!(!args.contains(&"-ss".to_string()));
        assert!(!args.contains(&"-stream_loop".to_string()));
        assert!(!args.contains(&"-hwaccel".to_string()));
        assert!(!args.contains(&"-vf".to_string()));
    }

    #[test]
    fn restamp_filter_only_for_seek_plus_loop_with_a_known_rate() {
        for (start_secs, looping, source_fps, want) in [
            (0.0, false, Some(24.0), None),
            (3.0, false, Some(24.0), None),
            (0.0, true, Some(24.0), None),
            (3.0, true, Some(24.0), Some("setpts=N/(24*TB)")),
            (3.0, true, Some(29.97), Some("setpts=N/(29.97*TB)")),
            // No constant rate known: skip the filter rather than emit a NaN timestamp.
            (3.0, true, None, None),
        ] {
            let args = strs(&stream_args(
                Path::new("a.mp4"),
                &StreamSpec {
                    start_secs,
                    looping,
                    source_fps,
                    ..spec()
                },
            ));
            match want {
                Some(filter) => assert!(has_pair(&args, "-vf", filter), "{args:?}"),
                None => assert!(!args.contains(&"-vf".to_string()), "{args:?}"),
            }
        }
    }

    #[test]
    fn relative_paths_are_not_read_as_protocols() {
        let args = strs(&stream_args(Path::new("take2: night.mp4"), &spec()));
        let at = pos(&args, "-i").unwrap();
        assert_eq!(args[at + 1], "./take2: night.mp4");
        // Absolute paths are left alone.
        let args = strs(&stream_args(Path::new("/clips/take2: night.mp4"), &spec()));
        let at = pos(&args, "-i").unwrap();
        assert_eq!(args[at + 1], "/clips/take2: night.mp4");
    }

    #[test]
    fn seek_only_when_start_is_positive_and_before_input() {
        let at_zero = strs(&stream_args(Path::new("a.mp4"), &spec()));
        assert!(!at_zero.contains(&"-ss".to_string()));

        let args = strs(&stream_args(
            Path::new("a.mp4"),
            &StreamSpec {
                start_secs: 12.5,
                ..spec()
            },
        ));
        let ss = pos(&args, "-ss").expect("-ss present");
        assert_eq!(args[ss + 1], "12.500");
        assert!(ss < pos(&args, "-i").unwrap());
    }

    #[test]
    fn loop_only_when_looping_and_before_input() {
        let args = strs(&stream_args(
            Path::new("a.mp4"),
            &StreamSpec {
                looping: true,
                ..spec()
            },
        ));
        let at = pos(&args, "-stream_loop").expect("-stream_loop present");
        assert_eq!(args[at + 1], "-1");
        assert!(at < pos(&args, "-i").unwrap());
    }

    #[test]
    fn hwaccel_only_when_enabled_and_before_input() {
        let args = strs(&stream_args(
            Path::new("a.mp4"),
            &StreamSpec {
                hwaccel: true,
                ..spec()
            },
        ));
        let at = pos(&args, "-hwaccel").expect("-hwaccel present");
        assert_eq!(args[at + 1], "videotoolbox");
        assert!(at < pos(&args, "-i").unwrap());
    }

    #[test]
    fn legacy_vsync_swaps_the_cfr_flag() {
        let args = strs(&stream_args(
            Path::new("a.mp4"),
            &StreamSpec {
                legacy_vsync: true,
                ..spec()
            },
        ));
        assert!(has_pair(&args, "-vsync", "cfr"));
        assert!(!args.contains(&"-fps_mode".to_string()));
    }

    #[test]
    fn fractional_fps_is_passed_through() {
        let args = strs(&stream_args(
            Path::new("a.mp4"),
            &StreamSpec::new(23.976, 64, 48),
        ));
        assert!(has_pair(&args, "-r", "23.976"));
        assert!(has_pair(&args, "-s", "64x48"));
    }

    #[test]
    fn awkward_paths_stay_one_argument() {
        for p in [
            "/Users/x/Library/Mobile Documents/com~apple~CloudDocs/VJ-Movies/vj-library/video/Slumber Giants (The) 0h-00.mp4",
            "/clips/Bj\u{f6}rk \u{2013} \u{30af}\u{30e9}\u{30a4}\u{30e0}.mov",
            "/clips/-dash and 'quote' \"double\".mp4",
        ] {
            let path = Path::new(p);
            let args = stream_args(path, &spec());
            let at = args.iter().position(|a| a == "-i").unwrap();
            assert_eq!(args[at + 1], path.as_os_str(), "path {p}");
            assert_eq!(args.iter().filter(|a| *a == path.as_os_str()).count(), 1);
        }
    }

    #[test]
    fn spec_validation() {
        assert!(spec().validate().is_ok());
        for bad in [
            StreamSpec::new(0.0, 64, 48),
            StreamSpec::new(f64::NAN, 64, 48),
            StreamSpec::new(30.0, 0, 48),
            StreamSpec::new(30.0, 64, 0),
            StreamSpec {
                start_secs: -1.0,
                ..spec()
            },
            StreamSpec {
                start_secs: f64::INFINITY,
                ..spec()
            },
            StreamSpec {
                source_fps: Some(0.0),
                ..spec()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        assert_eq!(StreamSpec::new(30.0, 4, 2).frame_bytes(), Some(32));
    }

    #[test]
    fn legacy_vsync_version_detection() {
        for old in [
            "ffmpeg version 4.4.2 Copyright (c) 2000-2021 the FFmpeg developers",
            "ffmpeg version n5.0.3 Copyright",
            "ffmpeg version 5.0 Copyright",
            "ffmpeg version 3.4.8-0ubuntu0.2",
        ] {
            assert!(needs_legacy_vsync(Some(old)), "{old}");
        }
        for new in [
            "ffmpeg version 5.1 Copyright",
            "ffmpeg version 5.1.6-0+deb12u1",
            "ffmpeg version 6.1.1-3ubuntu5",
            "ffmpeg version n7.0",
            "ffmpeg version 9.0.2 Copyright (c) 2000-2026 the FFmpeg developers",
            "ffmpeg version N-117000-g1234abcd Copyright",
            "ffmpeg version git-2024-01-01-abcdef",
            "something else",
            "",
        ] {
            assert!(!needs_legacy_vsync(Some(new)), "{new}");
        }
        assert!(!needs_legacy_vsync(None));
    }

    // --- integration tests: need a real ffmpeg, skipped when there is none ---

    use std::process::Command;
    const W: u32 = 32;
    const H: u32 = 24;

    fn ffmpeg_or_skip(test: &str) -> Option<&'static VideoToolchain> {
        let tool = VideoToolchain::global(None);
        if tool.ffmpeg.is_some() {
            Some(tool)
        } else {
            eprintln!("SKIP {test}: ffmpeg not found");
            None
        }
    }

    /// Write a 1 s, 10 fps clip into `dir` whose frame `n` is a flat grey of luma `n * 25`,
    /// so a decoded frame identifies its source frame. Returns its path.
    fn make_clip(tool: &VideoToolchain, dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let out = tool
            .ffmpeg_cmd()
            .unwrap()
            .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
            .arg(format!(
                "color=c=black:s={W}x{H}:r=10:d=1,format=yuv420p,geq=lum='N*25':cb=128:cr=128"
            ))
            .args(["-c:v", "mpeg4", "-q:v", "2"])
            .arg(&path)
            .output()
            .expect("run ffmpeg to make the test clip");
        assert!(
            out.status.success(),
            "clip generation failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        path
    }

    fn read_frames(stream: &mut impl Read, frame_bytes: usize, max: usize) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        while frames.len() < max {
            let mut frame = vec![0u8; frame_bytes];
            match stream.read_exact(&mut frame) {
                Ok(()) => frames.push(frame),
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => panic!("read failed: {e}"),
            }
        }
        frames
    }

    /// Mean red of a frame; the clip is grey, so any channel identifies the source frame.
    fn level(frame: &[u8]) -> f64 {
        let reds: Vec<u32> = frame.chunks_exact(4).map(|p| u32::from(p[0])).collect();
        f64::from(reds.iter().sum::<u32>()) / reds.len() as f64
    }

    /// Index of the reference level nearest to this frame's level.
    fn nearest(refs: &[f64], frame: &[u8]) -> usize {
        let l = level(frame);
        (0..refs.len())
            .min_by(|&a, &b| (refs[a] - l).abs().total_cmp(&(refs[b] - l).abs()))
            .unwrap()
    }

    #[test]
    fn streams_exact_frames_and_ends_cleanly() {
        let Some(tool) = ffmpeg_or_skip("streams_exact_frames_and_ends_cleanly") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let clip = make_clip(tool, dir.path(), "clip.mp4");
        // 10 fps source resampled to 20 fps: 1 s -> 20 frames.
        let spec = StreamSpec::new(20.0, W, H);
        let mut stream = spawn_stream(&clip, &spec).unwrap();
        let frame_bytes = spec.frame_bytes().unwrap();
        assert_eq!(frame_bytes, (W * H * 4) as usize);

        let mut all = Vec::new();
        stream.read_to_end(&mut all).expect("clean ffmpeg exit");
        assert_eq!(all.len() % frame_bytes, 0, "partial trailing frame");
        assert_eq!(all.len() / frame_bytes, 20, "1 s at 20 fps");
        // Reads after EOF keep returning 0.
        assert_eq!(stream.read(&mut [0u8; 8]).unwrap(), 0);
        // RGBA alpha is opaque.
        assert!(all.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn missing_file_reports_ffmpeg_stderr() {
        let Some(_) = ffmpeg_or_skip("missing_file_reports_ffmpeg_stderr") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let mut stream =
            spawn_stream(&dir.path().join("nope.mp4"), &StreamSpec::new(10.0, W, H)).unwrap();
        let err = stream.read(&mut [0u8; 64]).unwrap_err().to_string();
        assert!(err.contains("ffmpeg failed"), "{err}");
        assert!(err.contains("nope.mp4"), "{err}");
        assert!(stream.stderr_excerpt().contains("nope.mp4"));
        // Once failed, further reads are plain EOF.
        assert_eq!(stream.read(&mut [0u8; 8]).unwrap(), 0);
    }

    #[test]
    fn seek_starts_mid_clip() {
        let Some(tool) = ffmpeg_or_skip("seek_starts_mid_clip") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let clip = make_clip(tool, dir.path(), "clip.mp4");
        let fb = (W * H * 4) as usize;
        let mut reference = spawn_stream(&clip, &StreamSpec::new(10.0, W, H)).unwrap();
        let refs: Vec<f64> = read_frames(&mut reference, fb, 100)
            .iter()
            .map(|f| level(f))
            .collect();
        assert_eq!(refs.len(), 10);

        let spec = StreamSpec {
            start_secs: 0.5,
            ..StreamSpec::new(10.0, W, H)
        };
        let mut stream = spawn_stream(&clip, &spec).unwrap();
        let ids: Vec<usize> = read_frames(&mut stream, fb, 100)
            .iter()
            .map(|f| nearest(&refs, f))
            .collect();
        assert_eq!(ids, [5, 6, 7, 8, 9]);
    }

    #[test]
    fn dropping_mid_stream_kills_the_child_promptly() {
        let Some(tool) = ffmpeg_or_skip("dropping_mid_stream_kills_the_child_promptly") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let clip = make_clip(tool, dir.path(), "clip.mp4");
        // Looping never ends on its own, and a 1280x720 frame fills the pipe fast.
        let spec = StreamSpec {
            looping: true,
            ..StreamSpec::new(60.0, 1280, 720)
        };
        let mut stream = spawn_stream(&clip, &spec).unwrap();
        let pid = stream.pid();
        let frames = read_frames(&mut stream, spec.frame_bytes().unwrap(), 3);
        assert_eq!(frames.len(), 3);

        let started = Instant::now();
        drop(stream);
        let took = started.elapsed();
        assert!(took < Duration::from_secs(1), "drop took {took:?}");
        #[cfg(unix)]
        {
            let alive = Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            assert!(!alive, "ffmpeg pid {pid} still running after drop");
        }
        #[cfg(not(unix))]
        let _ = pid;
    }

    #[cfg(unix)]
    #[test]
    fn kill_from_another_thread_unblocks_a_stalled_read() {
        let Some(tool) = ffmpeg_or_skip("kill_from_another_thread_unblocks_a_stalled_read") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let clip = make_clip(tool, dir.path(), "clip.mp4");
        let spec = StreamSpec {
            looping: true,
            ..StreamSpec::new(60.0, 1280, 720)
        };
        let fb = spec.frame_bytes().unwrap();
        let mut stream = spawn_stream(&clip, &spec).unwrap();
        let pid = stream.pid();
        let kill = stream.killer();
        let mut first = vec![0u8; fb];
        stream.read_exact(&mut first).unwrap();

        // Freeze ffmpeg: the reader below blocks once the pipe buffer is empty.
        assert!(
            Command::new("kill")
                .args(["-STOP", &pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
        let reader = std::thread::spawn(move || {
            let mut sink = vec![0u8; fb * 10];
            let result = stream.read_exact(&mut sink);
            (result, stream)
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !reader.is_finished(),
            "read should be blocked on the frozen child"
        );

        let started = Instant::now();
        kill();
        let (result, stream) = reader.join().unwrap();
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(1),
            "read unblocked after {took:?}"
        );
        // A kill ends the stream cleanly (EOF), not with an ffmpeg failure.
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        drop(stream);
        let alive = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "ffmpeg pid {pid} still running");
    }

    #[test]
    fn kill_is_idempotent_and_safe_after_exit() {
        let Some(tool) = ffmpeg_or_skip("kill_is_idempotent_and_safe_after_exit") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let clip = make_clip(tool, dir.path(), "clip.mp4");

        // Two killers on a live stream, then Drop on top.
        let spec = StreamSpec {
            looping: true,
            ..StreamSpec::new(30.0, W, H)
        };
        let mut stream = spawn_stream(&clip, &spec).unwrap();
        let (kill_a, kill_b) = (stream.killer(), stream.killer());
        let mut frame = vec![0u8; spec.frame_bytes().unwrap()];
        stream.read_exact(&mut frame).unwrap();
        kill_a();
        kill_b();
        assert!(
            stream.read_to_end(&mut Vec::new()).is_ok(),
            "kill reads as EOF"
        );
        drop(stream);

        // Killers called after a natural end (child exited and reaped), and after Drop.
        let mut stream = spawn_stream(&clip, &StreamSpec::new(10.0, W, H)).unwrap();
        let (kill_a, kill_b) = (stream.killer(), stream.killer());
        stream.read_to_end(&mut Vec::new()).unwrap();
        kill_a();
        drop(stream);
        kill_b();

        // Through the boxed pipe, after the stream has failed.
        let FrameSource { mut reader, kill } =
            spawn_pipe(&dir.path().join("nope.mp4"), &StreamSpec::new(10.0, W, H)).unwrap();
        assert!(reader.read(&mut [0u8; 8]).is_err());
        kill();
    }

    /// Open question from the streaming design: with `-ss T -stream_loop -1`, does the
    /// second pass restart at media time 0 or at T? Answer: 0, so frame `n` is media time
    /// `(T + n / fps) mod duration`, which is what `StreamClock` assumes.
    #[test]
    fn loop_wraps_to_media_start() {
        let Some(tool) = ffmpeg_or_skip("loop_wraps_to_media_start") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let clip = make_clip(tool, dir.path(), "clip.mp4");
        let fb = (W * H * 4) as usize;
        let mut reference = spawn_stream(&clip, &StreamSpec::new(10.0, W, H)).unwrap();
        let refs: Vec<f64> = read_frames(&mut reference, fb, 100)
            .iter()
            .map(|f| level(f))
            .collect();
        assert_eq!(refs.len(), 10);

        let spec = StreamSpec {
            start_secs: 0.5,
            looping: true,
            source_fps: Some(10.0),
            ..StreamSpec::new(10.0, W, H)
        };
        let mut stream = spawn_stream(&clip, &spec).unwrap();
        // 2.5 s at 10 fps.
        let ids: Vec<usize> = read_frames(&mut stream, fb, 25)
            .iter()
            .map(|f| nearest(&refs, f))
            .collect();
        assert_eq!(ids.len(), 25);
        let expected: Vec<usize> = (0..25).map(|n| (5 + n) % 10).collect();
        assert_eq!(ids, expected, "frame n should be source frame (5 + n) % 10");

        // Output faster than the source repeats each frame; the wrap must stay seamless.
        let spec = StreamSpec {
            start_secs: 0.5,
            looping: true,
            source_fps: Some(10.0),
            ..StreamSpec::new(20.0, W, H)
        };
        let mut stream = spawn_stream(&clip, &spec).unwrap();
        let ids: Vec<usize> = read_frames(&mut stream, fb, 50)
            .iter()
            .map(|f| nearest(&refs, f))
            .collect();
        let expected: Vec<usize> = (0..50).map(|n| (5 + n / 2) % 10).collect();
        assert_eq!(
            ids, expected,
            "20 fps: frame n is source frame (5 + n / 2) % 10"
        );
    }

    #[test]
    fn producer_opens_streams_at_the_requested_start() {
        let Some(tool) = ffmpeg_or_skip("producer_opens_streams_at_the_requested_start") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let clip = make_clip(tool, dir.path(), "clip with space.mp4");
        let mut produce = stream_producer(clip, StreamSpec::new(10.0, W, H));
        let mut all = Vec::new();
        produce(0.5, false)
            .unwrap()
            .reader
            .read_to_end(&mut all)
            .unwrap();
        assert_eq!(all.len(), 5 * (W * H * 4) as usize);
    }
}
