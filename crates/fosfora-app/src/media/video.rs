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

    /// RAM for the probed frame count. Every frame is held as RGBA8 (4 bytes/px),
    /// whatever the source pixel format. Admission reserves [`decode_allowance`],
    /// which is larger than this when the decoder's slack still fits.
    pub fn decoded_bytes(&self) -> u64 {
        self.frame_count()
            .saturating_mul(u64::from(self.width))
            .saturating_mul(u64::from(self.height))
            .saturating_mul(4)
    }
}

/// What the all-frames decoder may store for one probed clip.
///
/// ffprobe can under-count variable-frame-rate video, so the decoder accepts
/// `estimated + estimated/4 + 8` frames and never more than
/// [`super::library::max_clip_ram`]. `ram_bytes` is that unclamped slack.
/// Admission reserves it, and refuses the clip when it is over the cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodeAllowance {
    /// Frames `decode_all_frames` will accept before failing the read.
    pub frame_cap: usize,
    /// `(estimate + 25% + 8 frames) * RGBA bytes`, saturating.
    /// Under the per-clip cap this is exactly `frame_cap` frames.
    pub ram_bytes: u64,
}

pub(crate) fn decode_allowance(
    width: u32,
    height: u32,
    estimated_frames: u64,
) -> Result<DecodeAllowance, String> {
    let frame_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| "Video has invalid decoded frame dimensions".to_string())?;
    let estimated = usize::try_from(estimated_frames)
        .map_err(|_| "Video has too many frames to pre-decode".to_string())?;
    let slack_frames = estimated.saturating_add(estimated / 4).saturating_add(8);
    let memory_frames =
        usize::try_from(super::library::max_clip_ram() / frame_bytes).unwrap_or(usize::MAX);
    Ok(DecodeAllowance {
        frame_cap: slack_frames.min(memory_frames),
        ram_bytes: (slack_frames as u64).saturating_mul(frame_bytes),
    })
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
            if detail.is_empty() {
                "file is unreadable or has an unsupported video container"
            } else {
                detail
            }
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

/// Read whole RGBA frames from `reader`.
///
/// End of stream on a frame boundary is success, including zero frames.
/// A short final frame, a read error, or more frames than `frame_cap` is an
/// error, and no frames are returned. `read` returning
/// [`std::io::ErrorKind::Interrupted`] is retried.
fn read_rgba_frames<R: Read>(
    reader: &mut R,
    width: u32,
    height: u32,
    delay_ms: u32,
    frame_cap: usize,
    estimated_frames: usize,
) -> Result<(Vec<DecodedFrame>, Vec<u32>), String> {
    let frame_size = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|&n| n > 0)
        .ok_or("Video has invalid decoded frame dimensions")?;
    if frame_cap == 0 {
        return Err("Even one decoded video frame exceeds the clip memory budget".into());
    }
    let mut frames = Vec::with_capacity(estimated_frames.min(frame_cap));
    let mut delays_ms = Vec::with_capacity(estimated_frames.min(frame_cap));
    let mut buf = vec![0u8; frame_size];
    loop {
        let mut filled = 0usize;
        while filled < frame_size {
            match reader.read(&mut buf[filled..]) {
                Ok(0) if filled == 0 => return Ok((frames, delays_ms)),
                Ok(0) => {
                    return Err(format!(
                        "ffmpeg stopped halfway through frame {} ({} of {} bytes)",
                        frames.len() + 1,
                        filled,
                        frame_size
                    ));
                }
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(format!("ffmpeg frame-read failed: {e}")),
            }
        }
        if frames.len() >= frame_cap {
            return Err(format!(
                "Video decoded more than {frame_cap} frames (ffprobe estimated {estimated_frames}); \
                 use a shorter clip or lower resolution"
            ));
        }
        frames.push(DecodedFrame {
            data: buf.clone(),
            width,
            height,
        });
        delays_ms.push(delay_ms);
    }
}

/// Pre-decode all video frames via a single ffmpeg run.
/// Returns (frames, delays_ms) ready for `MediaSource::Animated`.
pub fn decode_all_frames(
    path: &Path,
    meta: &VideoMeta,
) -> Result<(Vec<DecodedFrame>, Vec<u32>), String> {
    let frame_size = (meta.width as usize)
        .checked_mul(meta.height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|&n| n > 0)
        .ok_or("Video has invalid decoded frame dimensions")?;
    if !meta.fps.is_finite() || meta.fps <= 0.0 {
        return Err("Video has an invalid frame rate".into());
    }
    let delay_ms = (1000.0 / meta.fps).round().max(1.0) as u32;
    let allowance = decode_allowance(meta.width, meta.height, meta.frame_count())?;
    let est_frames = usize::try_from(meta.frame_count())
        .map_err(|_| "Video has too many frames to pre-decode")?;
    // Same cap admission reserves. A variable-frame-rate file can under-report
    // its count; never grow past the per-clip RAM cap.
    let frame_cap = allowance.frame_cap;
    if frame_cap == 0 {
        return Err("Even one decoded video frame exceeds the clip memory budget".into());
    }
    log::info!(
        "Pre-decoding {}: ~{} frames, ~{}MB RAM",
        path.display(),
        est_frames,
        est_frames.saturating_mul(frame_size) / (1024 * 1024),
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
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{}x{}", meta.width, meta.height),
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start ffmpeg for {}: {e}", path.display()))?;

    // Drain stderr while reading video frames. Otherwise a decoder emitting
    // errors can fill its pipe and block forever. Keep only a bounded excerpt.
    let stderr = child.stderr.take();
    let stderr_reader = std::thread::spawn(move || {
        let mut diagnostic = String::new();
        if let Some(mut stderr) = stderr {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let remaining = 8192usize.saturating_sub(diagnostic.len());
                        if remaining > 0 {
                            let keep = remaining.min(n);
                            diagnostic.push_str(&String::from_utf8_lossy(&chunk[..keep]));
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }
        diagnostic
    });

    let mut stdout = child.stdout.take().ok_or("ffmpeg: no stdout pipe")?;
    let read_result = read_rgba_frames(
        &mut stdout,
        meta.width,
        meta.height,
        delay_ms,
        frame_cap,
        est_frames,
    );
    drop(stdout);
    if read_result.is_err() {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|e| format!("ffmpeg process wait failed: {e}"))?;
    let diagnostic = stderr_reader.join().unwrap_or_default();
    let detail = diagnostic.trim();
    let (frames, delays_ms) = match read_result {
        Ok(decoded) => decoded,
        Err(failure) => {
            return Err(format!(
                "{}: {failure}{}",
                path.display(),
                if detail.is_empty() {
                    String::new()
                } else {
                    format!("; ffmpeg: {detail}")
                }
            ));
        }
    };
    if !status.success() {
        return Err(format!(
            "ffmpeg failed to decode {} ({status}){}",
            path.display(),
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }
    if frames.is_empty() {
        return Err(format!(
            "ffmpeg decoded zero frames from {}{}",
            path.display(),
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }

    log::info!(
        "Decoded {} video frames ({}MB)",
        frames.len(),
        frames.len().saturating_mul(frame_size) / (1024 * 1024),
    );
    Ok((frames, delays_ms))
}

/// Maximum video duration (seconds) we'll pre-decode. Beyond this, reject.
pub const MAX_PREDECODE_SECS: f64 = 60.0;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_video() -> serde_json::Value {
        json!({
            "streams": [{
                "codec_type": "video",
                "width": 1280,
                "height": 720,
                "avg_frame_rate": "0/0",
                "r_frame_rate": "24/1",
                "nb_frames": "192",
                "pix_fmt": "yuv420p"
            }],
            "format": { "duration": "8.000" }
        })
    }

    #[test]
    fn picks_valid_avg_fps_or_falls_back_to_stream_rate() {
        let mut sample = sample_video();
        let meta = parse_video_metadata(&sample).unwrap();
        assert_eq!(meta.fps, 24.0);
        assert_eq!(meta.frame_count(), 192);
        assert_eq!(meta.decoded_bytes(), 1280 * 720 * 4 * 192);

        sample["streams"][0]["avg_frame_rate"] = json!("30000/1001");
        let meta = parse_video_metadata(&sample).unwrap();
        assert!((meta.fps - 29.97002997).abs() < 0.00001);
    }

    #[test]
    fn derives_missing_container_duration_from_frames() {
        let mut sample = sample_video();
        sample["format"]["duration"] = serde_json::Value::Null;
        let meta = parse_video_metadata(&sample).unwrap();
        assert_eq!(meta.duration_secs, 8.0);
    }

    #[test]
    fn refuses_zero_fps_and_missing_duration_instead_of_failing_during_decode() {
        let mut sample = sample_video();
        sample["streams"][0]["r_frame_rate"] = json!("0/0");
        let err = parse_video_metadata(&sample).unwrap_err();
        assert!(err.contains("frame rate"), "{err}");

        sample["streams"][0]["r_frame_rate"] = json!("24/1");
        sample["streams"][0]["nb_frames"] = serde_json::Value::Null;
        sample["format"]["duration"] = serde_json::Value::Null;
        let err = parse_video_metadata(&sample).unwrap_err();
        assert!(err.contains("duration or frame count"), "{err}");
    }

    #[test]
    fn rejects_invalid_video_dimensions_and_nonvideo_files() {
        let mut sample = sample_video();
        sample["streams"][0]["width"] = json!(0);
        assert!(parse_video_metadata(&sample).unwrap_err().contains("width"));

        let image = json!({
            "streams": [{
                "codec_type": "audio",
                "sample_rate": "48000"
            }],
            "format": {"duration": "8.0"}
        });
        assert!(
            parse_video_metadata(&image)
                .unwrap_err()
                .contains("No video stream")
        );
    }

    #[test]
    fn accepts_frame_aligned_output_even_when_reads_are_short() {
        struct OneByte<R>(R);
        impl<R: Read> Read for OneByte<R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if buf.is_empty() {
                    return Ok(0);
                }
                let mut one = [0u8; 1];
                let n = self.0.read(&mut one)?;
                if n == 0 {
                    return Ok(0);
                }
                buf[0] = one[0];
                Ok(1)
            }
        }
        let mut reader = OneByte(std::io::Cursor::new(vec![1, 2, 3, 4, 5, 6, 7, 8]));
        let (frames, delays) = read_rgba_frames(&mut reader, 1, 1, 40, 4, 2).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, vec![1, 2, 3, 4]);
        assert_eq!(frames[1].data, vec![5, 6, 7, 8]);
        assert_eq!(delays, vec![40, 40]);
    }

    fn expect_read_err<R: Read>(reader: &mut R, frame_cap: usize, estimated: usize) -> String {
        match read_rgba_frames(reader, 1, 1, 40, frame_cap, estimated) {
            Err(err) => err,
            Ok(_) => panic!("expected the frame reader to fail"),
        }
    }

    #[test]
    fn rejects_a_partial_final_frame() {
        let mut reader = std::io::Cursor::new(vec![1u8, 2, 3, 4, 5, 6]);
        let err = expect_read_err(&mut reader, 4, 2);
        assert!(err.contains("halfway"), "{err}");
        assert!(err.contains("2 of 4"), "{err}");
    }

    #[test]
    fn rejects_frames_past_the_cap_and_a_hard_read_error() {
        let mut reader = std::io::Cursor::new(vec![0u8; 8]);
        let err = expect_read_err(&mut reader, 1, 2);
        assert!(err.contains("more than 1"), "{err}");

        struct Boom;
        impl Read for Boom {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gone"))
            }
        }
        let err = expect_read_err(&mut Boom, 4, 1);
        assert!(err.contains("frame-read failed"), "{err}");
    }

    #[test]
    fn empty_stream_is_not_a_partial_frame_and_interrupt_is_retried() {
        let mut reader = std::io::Cursor::new(Vec::<u8>::new());
        let (frames, _) = read_rgba_frames(&mut reader, 1, 1, 40, 4, 1).unwrap();
        assert!(frames.is_empty());

        struct InterruptOnce {
            data: Vec<u8>,
            pos: usize,
            interrupted: bool,
        }
        impl Read for InterruptOnce {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "again",
                    ));
                }
                if self.pos >= self.data.len() || buf.is_empty() {
                    return Ok(0);
                }
                buf[0] = self.data[self.pos];
                self.pos += 1;
                Ok(1)
            }
        }
        let mut reader = InterruptOnce {
            data: vec![9, 8, 7, 6],
            pos: 0,
            interrupted: false,
        };
        let (frames, _) = read_rgba_frames(&mut reader, 1, 1, 40, 4, 1).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, vec![9, 8, 7, 6]);
    }

    #[test]
    fn reservation_uses_decoder_frame_cap_not_the_probe_count() {
        let estimated = 100u64;
        let frame_bytes = 16u64 * 16 * 4;
        let bare = estimated * frame_bytes;
        let allowance = decode_allowance(16, 16, estimated).unwrap();
        let slack_frames = 100 + 25 + 8;
        assert_eq!(allowance.frame_cap, slack_frames);
        assert_eq!(allowance.ram_bytes, slack_frames as u64 * frame_bytes);
        assert_ne!(allowance.ram_bytes, bare);
        assert!(allowance.ram_bytes <= crate::media::library::MAX_CLIP_RAM);
        assert_eq!(
            allowance.ram_bytes,
            allowance.frame_cap as u64 * frame_bytes
        );
    }

    #[test]
    fn slack_over_two_gib_stays_over_the_clip_cap_and_the_decoder_does_not_grow() {
        let estimated = 1_700_000u64;
        let frame_bytes = 16u64 * 16 * 4;
        let bare = estimated * frame_bytes;
        assert!(bare <= crate::media::library::MAX_CLIP_RAM);
        let allowance = decode_allowance(16, 16, estimated).unwrap();
        let slack_frames = estimated as usize + (estimated as usize) / 4 + 8;
        assert!(allowance.ram_bytes > crate::media::library::MAX_CLIP_RAM);
        assert!(allowance.frame_cap < slack_frames);
        let stored = allowance.frame_cap as u64 * frame_bytes;
        assert!(stored <= crate::media::library::MAX_CLIP_RAM);
    }
}
