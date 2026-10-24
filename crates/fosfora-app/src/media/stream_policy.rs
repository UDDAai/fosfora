//! Which video clips stream from disk, and how much RAM a streamed clip reserves.
//!
//! Pure functions: no I/O, threads or GPU. The all-frames decoder holds every frame as
//! RGBA8, so a long or large clip cannot fit the per-clip cap. A streamed clip instead
//! holds a small fixed set of buffers, whatever its length. Admission (`plan_video`)
//! will use [`should_stream`] to pick the path and [`stream_reserve_bytes`] for the
//! budget share of a streamed clip.
//!
//! Only the env read in [`StreamPolicy::from_env`] touches the process; everything else
//! takes the policy as a parameter.
//!
//! Not yet wired into the app.
#![allow(dead_code)]

use super::library::format_bytes;
use super::stream_decoder::POOL_BUFFERS;

/// Env var that picks the [`StreamPolicy`]: `off`, `auto` or `on`, case-insensitive.
pub const STREAM_ENV: &str = "FOSFORA_VIDEO_STREAM";

/// Under `Auto`, a clip longer than this streams even if it would fit the per-clip cap.
/// Shorter clips keep the all-frames path, which scrubs instantly.
pub const AUTO_STREAM_MIN_SECS: f64 = 20.0;

/// Estimate of ffmpeg's own working set (decoder references, scaler, pipes) at 1080p.
/// Other sizes scale by pixel count. T3 measured about 75-101 MB peak RSS per streamed
/// clip at 720p (test process plus ffmpeg), so 100 MiB at 1080p is a reasonable upper
/// estimate, not a limit.
pub const FFMPEG_WORKING_SET_1080P: u64 = 100 * 1024 * 1024;

const PIXELS_1080P: u64 = 1920 * 1080;

/// Whether video clips stream from disk instead of decoding every frame up front.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StreamPolicy {
    /// Always decode all frames. Clips over the cap are refused, as before streaming.
    Off,
    /// Stream clips that are too big or too long for the all-frames path.
    #[default]
    Auto,
    /// Stream every clip.
    On,
}

impl StreamPolicy {
    /// Parse `off|auto|on`, case-insensitive, ignoring surrounding whitespace.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "auto" => Some(Self::Auto),
            "on" => Some(Self::On),
            _ => None,
        }
    }

    /// Policy for an optional env value. Unset is `Auto`; garbage is `Auto` with a warning.
    pub fn from_env_value(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self::default();
        };
        Self::parse(value).unwrap_or_else(|| {
            log::warn!("{STREAM_ENV}={value:?} is not off, auto or on; using auto");
            Self::default()
        })
    }

    /// Read [`STREAM_ENV`] from the process environment.
    pub fn from_env() -> Self {
        Self::from_env_value(std::env::var(STREAM_ENV).ok().as_deref())
    }
}

/// Whether a clip streams. `all_frames_bytes` is what the all-frames decode would
/// reserve; `per_clip_cap_bytes` is the limit it is held to (`max_clip_ram`).
///
/// `Auto` streams when the decode would exceed the cap (equal fits) or the clip is
/// longer than [`AUTO_STREAM_MIN_SECS`] (equal does not).
pub fn should_stream(
    policy: StreamPolicy,
    duration_secs: f64,
    all_frames_bytes: u64,
    per_clip_cap_bytes: u64,
) -> bool {
    match policy {
        StreamPolicy::Off => false,
        StreamPolicy::On => true,
        StreamPolicy::Auto => {
            all_frames_bytes > per_clip_cap_bytes || duration_secs > AUTO_STREAM_MIN_SECS
        }
    }
}

/// Fixed RAM a streamed clip holds: the pool's RGBA8 buffers plus ffmpeg's working set.
/// Independent of duration.
pub fn stream_reserve_bytes(width: u32, height: u32) -> u64 {
    let pixels = u64::from(width) * u64::from(height);
    let pool = pixels.saturating_mul(4).saturating_mul(POOL_BUFFERS as u64);
    let ffmpeg =
        u128::from(FFMPEG_WORKING_SET_1080P) * u128::from(pixels) / u128::from(PIXELS_1080P);
    pool.saturating_add(u64::try_from(ffmpeg).unwrap_or(u64::MAX))
}

/// One short line saying why a clip streams, for logs and library status.
/// `None` when it does not stream.
pub fn describe_decision(
    policy: StreamPolicy,
    duration_secs: f64,
    frames: u64,
    all_frames_bytes: u64,
    per_clip_cap_bytes: u64,
) -> Option<String> {
    if !should_stream(policy, duration_secs, all_frames_bytes, per_clip_cap_bytes) {
        return None;
    }
    let reason = if all_frames_bytes > per_clip_cap_bytes {
        format!(
            "{frames} frames would need {}",
            format_bytes(all_frames_bytes)
        )
    } else if duration_secs > AUTO_STREAM_MIN_SECS {
        format!("{duration_secs:.0}s is over {AUTO_STREAM_MIN_SECS:.0}s")
    } else {
        format!("{STREAM_ENV}=on")
    };
    Some(format!("streams: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: u64 = 2 * 1024 * 1024 * 1024;
    const FPS_5994: f64 = 60000.0 / 1001.0;

    /// All-frames RGBA8 bytes for a clip, as `decode_allowance` would reserve before slack.
    fn all_frames(width: u32, height: u32, fps: f64, secs: f64) -> u64 {
        (fps * secs).ceil() as u64 * u64::from(width) * u64::from(height) * 4
    }

    fn auto(width: u32, height: u32, fps: f64, secs: f64) -> bool {
        should_stream(
            StreamPolicy::Auto,
            secs,
            all_frames(width, height, fps, secs),
            CAP,
        )
    }

    #[test]
    fn off_and_on_ignore_size_and_length() {
        assert!(!should_stream(StreamPolicy::Off, 600.0, u64::MAX, CAP));
        assert!(should_stream(StreamPolicy::On, 1.0, 1, CAP));
    }

    #[test]
    fn auto_boundary_at_the_cap() {
        assert!(!should_stream(StreamPolicy::Auto, 5.0, CAP, CAP));
        assert!(should_stream(StreamPolicy::Auto, 5.0, CAP + 1, CAP));
    }

    #[test]
    fn auto_boundary_at_the_duration_threshold() {
        assert!(!should_stream(
            StreamPolicy::Auto,
            AUTO_STREAM_MIN_SECS,
            1,
            CAP
        ));
        assert!(should_stream(
            StreamPolicy::Auto,
            AUTO_STREAM_MIN_SECS + 0.001,
            1,
            CAP
        ));
    }

    #[test]
    fn reserve_ignores_duration() {
        // Duration is not an input; the same dimensions always give the same reserve.
        let a = stream_reserve_bytes(1920, 1080);
        let b = stream_reserve_bytes(1920, 1080);
        assert_eq!(a, b);
        assert_eq!(
            a,
            POOL_BUFFERS as u64 * 1920 * 1080 * 4 + FFMPEG_WORKING_SET_1080P
        );
    }

    #[test]
    fn reserve_scales_with_pixels() {
        let hd = stream_reserve_bytes(1920, 1080);
        let uhd = stream_reserve_bytes(3840, 2160);
        assert_eq!(uhd, 4 * hd);
        assert!(stream_reserve_bytes(1280, 720) < hd);
        assert_eq!(stream_reserve_bytes(0, 0), 0);
        assert_eq!(stream_reserve_bytes(u32::MAX, u32::MAX), u64::MAX);
    }

    #[test]
    fn reserve_is_far_below_a_long_clip() {
        // 51.5 s of 1080p59.94 is about 25.6 GB decoded; the stream holds under 0.3 GB.
        let decoded = all_frames(1920, 1080, FPS_5994, 51.5);
        assert!(stream_reserve_bytes(1920, 1080) * 50 < decoded);
    }

    #[test]
    fn robs_1080p60_clips_stream_under_auto() {
        for secs in [6.6, 16.7, 51.5] {
            assert!(auto(1920, 1080, FPS_5994, secs), "{secs}s should stream");
        }
        // The 6.6 s clip is the one that is short but still over the cap.
        let bytes = all_frames(1920, 1080, FPS_5994, 6.6);
        assert!(bytes > CAP && bytes < 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn thaw_keeps_the_all_frames_path() {
        let bytes = all_frames(1280, 720, 18.0, 8.0);
        assert!(bytes < CAP, "thaw is about 0.5 GB, got {bytes}");
        assert!(!auto(1280, 720, 18.0, 8.0));
    }

    #[test]
    fn parse_is_case_insensitive() {
        assert_eq!(StreamPolicy::parse("OFF"), Some(StreamPolicy::Off));
        assert_eq!(StreamPolicy::parse("Auto"), Some(StreamPolicy::Auto));
        assert_eq!(StreamPolicy::parse(" on "), Some(StreamPolicy::On));
        assert_eq!(StreamPolicy::parse("yes"), None);
        assert_eq!(StreamPolicy::parse(""), None);
    }

    #[test]
    fn env_value_defaults_to_auto() {
        assert_eq!(StreamPolicy::from_env_value(None), StreamPolicy::Auto);
        assert_eq!(StreamPolicy::from_env_value(Some("OFF")), StreamPolicy::Off);
        assert_eq!(StreamPolicy::from_env_value(Some("on")), StreamPolicy::On);
        assert_eq!(
            StreamPolicy::from_env_value(Some("garbage")),
            StreamPolicy::Auto
        );
    }

    #[test]
    fn describe_names_the_reason() {
        let bytes = all_frames(1920, 1080, FPS_5994, 51.5);
        let line = describe_decision(StreamPolicy::Auto, 51.5, 3087, bytes, CAP).unwrap();
        assert_eq!(line, "streams: 3087 frames would need 23.8 GB");
        let long = describe_decision(StreamPolicy::Auto, 30.0, 10, 10, CAP).unwrap();
        assert_eq!(long, "streams: 30s is over 20s");
        let forced = describe_decision(StreamPolicy::On, 3.0, 10, 10, CAP).unwrap();
        assert_eq!(forced, "streams: FOSFORA_VIDEO_STREAM=on");
        assert_eq!(
            describe_decision(StreamPolicy::Off, 51.5, 3087, bytes, CAP),
            None
        );
        assert_eq!(
            describe_decision(StreamPolicy::Auto, 8.0, 144, 1, CAP),
            None
        );
    }
}
