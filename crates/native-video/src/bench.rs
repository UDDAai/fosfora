//! CLI measurements for the standalone harness (`native-video-harness`).

use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::session::{phys_footprint, NativeClip};
use crate::{full_rgba_bytes, staging_ram_bytes};

pub fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return Err("usage: native-video-harness <clip.mp4> [seconds]".into());
    }
    let path = PathBuf::from(&args[1]);
    let seconds = args
        .get(2)
        .map(|s| s.parse::<f64>().unwrap_or(5.0))
        .unwrap_or(5.0)
        .clamp(1.0, 120.0);
    let before = phys_footprint();
    let clip = NativeClip::open(&path)?;
    let after_open = phys_footprint();
    let staging = staging_ram_bytes(clip.width(), clip.height())?;
    let full = full_rgba_bytes(clip.width(), clip.height(), clip.fps(), clip.duration_secs());
    println!(
        "open: {}x{} {:.2}fps {:.2}s codec={} hw={} note={}",
        clip.width(),
        clip.height(),
        clip.fps(),
        clip.duration_secs(),
        clip.codec(),
        clip.hardware(),
        clip.note()
    );
    println!(
        "ram: staging={} full_decode≈{} footprint +{}",
        staging,
        full,
        after_open.saturating_sub(before)
    );
    clip.sync(true, true, 1.0);
    let start = Instant::now();
    let mut frames = 0u64;
    while start.elapsed() < Duration::from_secs_f64(seconds) {
        if clip.latest_frame().is_some() {
            frames += 1;
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    let stats = clip.stats();
    let after = phys_footprint();
    println!(
        "play {:.1}s: frames={} dropped={} loops={} first={}ms avg_copy={}us footprint={}",
        seconds,
        stats.frames,
        stats.dropped,
        stats.loops,
        stats.first_frame.as_secs_f64() * 1000.0,
        stats.average_copy.as_micros(),
        after
    );
    let _ = clip.metal_preview();
    clip.shutdown();
    println!("poll_frames_seen={}", frames);
    Ok(())
}
