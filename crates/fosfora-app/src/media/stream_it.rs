//! Streamed video against a real ffmpeg: clips are generated at test time with
//! `lavfi` and H.264, and every test skips when ffmpeg or its H.264 encoder is missing.
//!
//! These drive [`MediaSource::Streamed`] the way a media layer does (the same
//! `step_streamed` per frame, paced at 60 Hz) but without a GPU. The tests share one
//! lock because they count process-wide things: live streams and ffmpeg children.
//! Children are told apart by the unique temp directory in their command line.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::decoder::{MediaSource, live_streams, load_media_with};
use super::stream_playback::StreamPlayback;
use super::stream_policy::{StreamPolicy, stream_reserve_bytes};
use super::toolchain::VideoToolchain;
use super::types::TransportState;

static SERIAL: Mutex<()> = Mutex::new(());
const FRAME: Duration = Duration::from_micros(16_667);

/// Take the shared lock, or `None` (a skip) when there is no ffmpeg to test with.
fn rig(test: &str) -> Option<MutexGuard<'static, ()>> {
    if VideoToolchain::global(None).ffmpeg.is_none() {
        eprintln!("SKIP {test}: ffmpeg not found");
        return None;
    }
    Some(SERIAL.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Write an H.264 clip of moving test pattern. The moov atom is first, so a clip cut
/// short is still probeable.
fn make_clip(dir: &Path, name: &str, w: u32, h: u32, fps: u32, secs: u32) -> Option<PathBuf> {
    let path = dir.join(name);
    let out = VideoToolchain::global(None)
        .ffmpeg_cmd()?
        .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size={w}x{h}:rate={fps}:duration={secs}"))
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-g", "12", "-movflags", "+faststart"])
        .arg(&path)
        .output()
        .ok()?;
    if !out.status.success() {
        eprintln!(
            "SKIP: cannot make a test clip: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(path)
}

fn open(path: &Path) -> Result<MediaSource, String> {
    load_media_with(path, StreamPolicy::On)
}

fn playback(source: &mut MediaSource) -> &mut StreamPlayback {
    match source {
        MediaSource::Streamed { playback, .. } => playback,
        _ => panic!("not a streamed source"),
    }
}

/// Step one layer at 60 Hz of real time until `done(playback, transport, uploads)`.
/// Panics after `limit`. Returns the uploads and the slowest single step.
fn run(
    source: &mut MediaSource,
    transport: &mut TransportState,
    limit: Duration,
    mut done: impl FnMut(&StreamPlayback, &TransportState, usize) -> bool,
) -> (usize, Duration) {
    let begin = Instant::now();
    let mut next = begin;
    let (mut uploads, mut slowest) = (0, Duration::ZERO);
    while !done(playback(source), transport, uploads) {
        assert!(begin.elapsed() < limit, "timed out after {limit:?}");
        next += FRAME;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        let started = Instant::now();
        uploads += usize::from(super::step_streamed(
            playback(source),
            transport,
            FRAME.as_secs_f32(),
        ));
        slowest = slowest.max(started.elapsed());
    }
    (uploads, slowest)
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// ffmpeg processes with `dir` in their command line.
#[cfg(unix)]
fn ffmpeg_children(dir: &Path) -> usize {
    let out = Command::new("pgrep")
        .arg("-f")
        .arg(dir)
        .output()
        .expect("pgrep");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .count()
}

#[test]
fn two_layers_stream_at_once() {
    let Some(_lock) = rig("two_layers_stream_at_once") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (Some(a), Some(b)) = (
        make_clip(dir.path(), "a.mp4", 96, 72, 24, 3),
        make_clip(dir.path(), "b.mp4", 64, 64, 30, 4),
    ) else {
        return;
    };
    let mut layers = [open(&a).unwrap(), open(&b).unwrap()];
    assert_eq!(live_streams(), 2);
    let mut transports = [TransportState::default(), TransportState::default()];
    let mut uploads = [0, 0];
    // Interleaved, as the render loop does.
    let begin = Instant::now();
    let mut next = begin;
    while begin.elapsed() < Duration::from_millis(1500) {
        next += FRAME;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        for i in 0..2 {
            let shown =
                super::step_streamed(playback(&mut layers[i]), &mut transports[i], 1.0 / 60.0);
            uploads[i] += usize::from(shown);
        }
    }
    assert!(uploads[0] >= 24, "layer a showed {} frames", uploads[0]);
    assert!(uploads[1] >= 30, "layer b showed {} frames", uploads[1]);
    for (layer, transport) in layers.iter_mut().zip(&transports) {
        assert!(transport.position > 0.8, "{}", transport.position);
        assert!(playback(layer).last_error().is_none());
    }
    assert_eq!(layers[0].dimensions(), (96, 72));
    assert_eq!(layers[1].dimensions(), (64, 64));
    assert_eq!(layers[0].resident_bytes(), stream_reserve_bytes(96, 72));
    assert!(layers[0].is_animated() && layers[0].is_video());
    assert!(layers[0].first_frame().is_none());
    drop(layers);
    wait_until("both clips to be torn down", || live_streams() == 0);
}

#[test]
fn looping_carries_on_after_a_seek() {
    let Some(_lock) = rig("looping_carries_on_after_a_seek") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some(clip) = make_clip(dir.path(), "loop.mp4", 64, 48, 24, 2) else {
        return;
    };
    let mut source = open(&clip).unwrap();
    let mut transport = TransportState::default(); // playing, looping
    // Seek near the end, then play through the wrap and a good way into the next pass.
    playback(&mut source).seek(1.5);
    let mut wrapped = false;
    let mut last = 0.0;
    let (uploads, _) = run(
        &mut source,
        &mut transport,
        Duration::from_secs(10),
        |_, t, _| {
            wrapped |= t.position + 0.5 < last;
            last = t.position;
            wrapped && t.position > 0.5
        },
    );
    assert!(uploads >= 24, "{uploads} frames after the seek");
    let pb = playback(&mut source);
    assert!(pb.last_error().is_none(), "{:?}", pb.last_error());
    assert!(transport.playing);
    drop(source);
    wait_until("the clip to be torn down", || live_streams() == 0);
}

#[test]
fn a_bad_file_is_an_error_not_a_hang() {
    let Some(_lock) = rig("a_bad_file_is_an_error_not_a_hang") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.mp4");
    let garbage = dir.path().join("garbage.mp4");
    std::fs::write(&garbage, vec![0x5Au8; 100_000]).unwrap();
    let empty = dir.path().join("empty.mp4");
    std::fs::write(&empty, b"").unwrap();
    for path in [missing, garbage, empty] {
        let started = Instant::now();
        let err = match open(&path) {
            Ok(_) => panic!("{} should not open", path.display()),
            Err(err) => err,
        };
        assert!(!err.is_empty());
        assert!(started.elapsed() < Duration::from_secs(5), "{err}");
    }
    assert_eq!(live_streams(), 0);
}

#[test]
fn a_long_clip_holds_flat_memory() {
    let Some(_lock) = rig("a_long_clip_holds_flat_memory") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // Four minutes: 7200 frames. Held as RGBA at 64x64 that would be 28 MB, but the
    // point is that the figure does not depend on the length.
    let Some(clip) = make_clip(dir.path(), "long.mp4", 64, 64, 30, 240) else {
        return;
    };
    let mut source = open(&clip).unwrap();
    assert_eq!(source.resident_bytes(), stream_reserve_bytes(64, 64));
    assert_eq!(source.frame_count(), 7200);
    let before = playback(&mut source).resident_bytes();
    let mut transport = TransportState::default();
    run(
        &mut source,
        &mut transport,
        Duration::from_secs(10),
        |_, t, _| t.position > 1.0,
    );
    // A seek deep into the clip is a fresh stream: a frame arrives in about a second.
    playback(&mut source).seek(200.0);
    let started = Instant::now();
    let (uploads, _) = run(
        &mut source,
        &mut transport,
        Duration::from_secs(10),
        |_, t, n| t.position > 200.5 && n > 0,
    );
    assert!(uploads > 0);
    assert!(started.elapsed() < Duration::from_secs(6));
    assert_eq!(playback(&mut source).resident_bytes(), before);
    assert_eq!(source.resident_bytes(), stream_reserve_bytes(64, 64));
    drop(source);
    wait_until("the clip to be torn down", || live_streams() == 0);
}

#[test]
fn scene_reloads_leave_no_threads_or_children_behind() {
    let Some(_lock) = rig("scene_reloads_leave_no_threads_or_children_behind") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some(clip) = make_clip(dir.path(), "scene.mp4", 64, 48, 24, 3) else {
        return;
    };
    // Preload, apply, retire: the same path again and again, with a seek in some
    // rounds so an extra ffmpeg has been started and abandoned.
    for round in 0..6 {
        let mut source = open(&clip).unwrap();
        let mut transport = TransportState::default();
        let pb = playback(&mut source);
        if round % 2 == 1 {
            pb.seek(1.0);
        }
        let (uploads, _) = run(
            &mut source,
            &mut transport,
            Duration::from_secs(10),
            |_, _, n| n >= 5,
        );
        assert_eq!(uploads, 5, "round {round}");
        assert!(live_streams() >= 1);
        // A retired layer: the render thread only hands the clip over.
        let dropped = Instant::now();
        drop(source);
        assert!(
            dropped.elapsed() < Duration::from_millis(50),
            "{:?}",
            dropped.elapsed()
        );
    }
    // A clip loaded and dropped before it ever played.
    drop(open(&clip).unwrap());
    wait_until("every clip to be torn down", || live_streams() == 0);
    #[cfg(unix)]
    wait_until("ffmpeg to exit", || ffmpeg_children(dir.path()) == 0);
}

#[test]
fn a_clip_that_disappears_reports_the_error_and_recovers() {
    let Some(_lock) = rig("a_clip_that_disappears_reports_the_error_and_recovers") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some(clip) = make_clip(dir.path(), "gone.mp4", 64, 48, 24, 4) else {
        return;
    };
    let mut source = open(&clip).unwrap();
    let mut transport = TransportState::default();
    run(
        &mut source,
        &mut transport,
        Duration::from_secs(10),
        |_, _, n| n >= 3,
    );
    let shown = playback(&mut source).current_frame().to_vec();

    // The file goes away (an unplugged drive), and the next seek needs a new ffmpeg.
    let away = dir.path().join("away.mp4");
    std::fs::rename(&clip, &away).unwrap();
    playback(&mut source).seek(2.0);
    let (_, slowest) = run(
        &mut source,
        &mut transport,
        Duration::from_secs(15),
        |p, _, _| p.last_error().is_some(),
    );
    assert!(
        slowest < Duration::from_millis(500),
        "a step took {slowest:?}"
    );
    assert_eq!(
        playback(&mut source).current_frame(),
        shown,
        "last frame stays"
    );
    let message = playback(&mut source).last_error().unwrap().to_owned();
    assert!(!message.is_empty());

    // It comes back; a seek starts a fresh stream and clears the error.
    std::fs::rename(&away, &clip).unwrap();
    playback(&mut source).seek(1.0);
    run(
        &mut source,
        &mut transport,
        Duration::from_secs(15),
        |p, _, n| n > 0 && p.last_error().is_none(),
    );
    drop(source);
    wait_until("the clip to be torn down", || live_streams() == 0);
}

#[test]
fn a_truncated_clip_stops_with_an_error() {
    let Some(_lock) = rig("a_truncated_clip_stops_with_an_error") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some(full) = make_clip(dir.path(), "full.mp4", 64, 48, 24, 10) else {
        return;
    };
    // Keep the header and the first part of the media data.
    let bytes = std::fs::read(&full).unwrap();
    let cut = dir.path().join("cut.mp4");
    std::fs::write(&cut, &bytes[..bytes.len() * 2 / 5]).unwrap();
    let mut source = match open(&cut) {
        Ok(source) => source,
        Err(err) => {
            // A cut that leaves no decodable first frame is a clean refusal too.
            assert!(!err.is_empty());
            eprintln!("truncated clip refused at open: {err}");
            return;
        }
    };
    let mut transport = TransportState {
        looping: false,
        ..TransportState::default()
    };
    // Play on to a point past the data. The stream cannot reach it: no hang, an error,
    // and the clock does not run on to the end of the clip unseen.
    playback(&mut source).seek(9.0);
    let (_, slowest) = run(
        &mut source,
        &mut transport,
        Duration::from_secs(20),
        |p, _, _| p.last_error().is_some(),
    );
    assert!(
        slowest < Duration::from_millis(500),
        "a step took {slowest:?}"
    );
    drop(source);
    wait_until("the clip to be torn down", || live_streams() == 0);
}
