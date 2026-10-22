//! Video pre-decode via ffmpeg subprocess (feature-gated behind `video`).
//!
//! - `ffprobe` probes metadata (dimensions, fps, duration) synchronously at load time
//! - `ffmpeg -f rawvideo -pix_fmt rgba` decodes ALL frames to memory in one pass
//! - Returns `MediaSource::Animated` — identical to GIF, instant random access
//! - RAM cost: ~3.7MB per frame at 1280x720. A 30s@30fps clip = ~3.3GB.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;

use super::toolchain::VideoToolchain;
use super::types::DecodedFrame;

/// Check if ffmpeg/ffprobe are available on the system. Cached per process.
pub fn ffmpeg_available() -> bool {
    VideoToolchain::global(None).available()
}

/// Video metadata from ffprobe.
#[derive(Debug, Clone)]
pub struct VideoMeta {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_secs: f64,
    /// Container frame count when ffprobe reports one.
    pub frames: Option<u64>,
    /// Source pixel format, e.g. `yuv420p10le`. Decode always yields RGBA8.
    pub pix_fmt: String,
}

impl VideoMeta {
    /// Frames the decoder will produce: the probed count, else duration × fps.
    pub fn frame_count(&self) -> u64 {
        self.frames
            .filter(|&n| n > 0)
            .unwrap_or_else(|| (self.duration_secs * self.fps).ceil().max(0.0) as u64)
    }

    /// RAM for the decoded clip. Every frame is held as RGBA8 (4 bytes/px),
    /// whatever the source pixel format.
    pub fn decoded_bytes(&self) -> u64 {
        self.frame_count()
            .saturating_mul(u64::from(self.width))
            .saturating_mul(u64::from(self.height))
            .saturating_mul(4)
    }
}

/// Probe video metadata using ffprobe.
pub fn probe_video(path: &Path) -> Result<VideoMeta, String> {
    let tool = VideoToolchain::global(None);
    let mut cmd = tool.ffprobe_cmd().ok_or_else(|| {
        format!(
            "ffprobe not found ({}; set ffmpeg path in settings, or install ffmpeg)",
            tool.status_line()
        )
    })?;
    let output = cmd
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Could not run ffprobe for {}: {e}", path.display()))?;

    if !output.status.success() {
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        let detail = diagnostic.trim();
        return Err(format!(
            "ffprobe could not inspect {}: {}",
            path.display(),
            if detail.is_empty() { "file is unreadable or has an unsupported video container" } else { detail }
        ));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse ffprobe JSON for {}: {e}", path.display()))?;
    parse_video_metadata(&json)
}

/// Validate the exact metadata that our all-frames-in-RAM decoder requires.
/// Some MOV/variable-frame-rate files report "0/0" for one of the rates, or
/// omit container duration. Treat those cases explicitly instead of
/// silently estimating zero frames and rejecting them later.
fn parse_video_metadata(json: &serde_json::Value) -> Result<VideoMeta, String> {
    let streams = json["streams"]
        .as_array()
        .ok_or("ffprobe did not return any streams")?;
    let video_stream = streams
        .iter()
        .find(|s| s["codec_type"].as_str() == Some("video"))
        .ok_or("No video stream found in the file")?;

    let width = video_stream["width"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n > 0)
        .ok_or("Video has no valid width")?;
    let height = video_stream["height"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n > 0)
        .ok_or("Video has no valid height")?;

    let fps = ["avg_frame_rate", "r_frame_rate"]
        .iter()
        .find_map(|key| video_stream[*key].as_str().and_then(parse_frame_rate))
        .ok_or("Video has no valid frame rate (ffprobe reported 0/0 or missing FPS)")?;
    let frames = video_stream["nb_frames"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0);
    let duration_secs = [&json["format"]["duration"], &video_stream["duration"]]
        .iter()
        .filter_map(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        .find(|d| d.is_finite() && *d > 0.0)
        .or_else(|| frames.map(|n| n as f64 / fps))
        .ok_or("Video has no usable duration or frame count for pre-decoding")?;

    Ok(VideoMeta {
        width,
        height,
        fps,
        duration_secs,
        frames,
        pix_fmt: video_stream["pix_fmt"].as_str().unwrap_or("").to_string(),
    })
}

fn parse_frame_rate(rate: &str) -> Option<f64> {
    let fps = if let Some((num, den)) = rate.split_once('/') {
        let num = num.parse::<f64>().ok()?;
        let den = den.parse::<f64>().ok()?;
        if den == 0.0 {
            return None;
        }
        num / den
    } else {
        rate.parse::<f64>().ok()?
    };
    (fps.is_finite() && fps > 0.0).then_some(fps)
}

/// Pre-decode all video frames via a single ffmpeg run.
/// Returns (frames, delays_ms) ready for `MediaSource::Animated`.
pub fn decode_all_frames(
    path: &Path,
    meta: &VideoMeta,
) -> Result<(Vec<DecodedFrame>, Vec<u32>), String> {
    let frame_size = (meta.width as usize) * (meta.height as usize) * 4;
    let delay_ms = (1000.0 / meta.fps).round() as u32;

    // Estimate RAM and warn
    let est_frames = meta.frame_count() as usize;
    // The memory check was made against the probed count. A stream that
    // decodes far past it is stopped rather than allowed to grow.
    let frame_cap = est_frames + est_frames / 4 + 8;
    let est_ram_mb = (est_frames * frame_size) / (1024 * 1024);
    log::info!(
        "Pre-decoding video: ~{} frames, ~{}MB RAM",
        est_frames,
        est_ram_mb,
    );

    let tool = VideoToolchain::global(None);
    let mut child = tool
        .ffmpeg_cmd()
        .ok_or_else(|| {
            format!(
                "ffmpeg not found ({}; set ffmpeg path in settings, or install ffmpeg)",
                tool.status_line()
            )
        })?
        .args(["-i"])
        .arg(path)
        .args([
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{}x{}", meta.width, meta.height),
            "-v",
            "quiet",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn ffmpeg: {e}"))?;

    let mut stdout = child.stdout.take().ok_or("ffmpeg: no stdout pipe")?;

    let mut frames = Vec::with_capacity(est_frames);
    let mut delays_ms = Vec::with_capacity(est_frames);
    let mut buf = vec![0u8; frame_size];

    while stdout.read_exact(&mut buf).is_ok() {
        if frames.len() >= frame_cap {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "decoded more than {frame_cap} frames; the probe said {est_frames}"
            ));
        }
        frames.push(DecodedFrame {
            data: buf.clone(),
            width: meta.width,
            height: meta.height,
        });
        delays_ms.push(delay_ms.max(1));
    }

    let _ = child.wait();

    if frames.is_empty() {
        return Err("ffmpeg decoded zero frames".to_string());
    }

    log::info!(
        "Decoded {} video frames ({}MB)",
        frames.len(),
        (frames.len() * frame_size) / (1024 * 1024),
    );

    Ok((frames, delays_ms))
}

/// Maximum video duration (seconds) we'll pre-decode. Beyond this, reject.
pub const MAX_PREDECODE_SECS: f64 = 60.0;
