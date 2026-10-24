//! Working-set size for one streamed clip.
//!
//! Three RGBA copies cover the worker's scratch buffer, the published frame,
//! and the copy the render thread holds while uploading. The count does not
//! grow with duration.

/// CPU frame copies reserved for one playing clip.
pub const CPU_FRAMES: u64 = 3;

pub fn frame_bytes(width: u32, height: u32) -> Option<u64> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes > 0)
}

/// Bytes reserved for one streamed clip. Independent of duration and frame count.
pub fn staging_ram_bytes(width: u32, height: u32) -> Result<u64, String> {
    let frame = frame_bytes(width, height).ok_or_else(|| {
        format!("video frame {width}x{height} has no usable RGBA size")
    })?;
    frame
        .checked_mul(CPU_FRAMES)
        .ok_or_else(|| "streamed video staging size overflowed".to_string())
}

/// Uncompressed RGBA for every frame. This is the cost the pre-decode path pays.
pub fn full_rgba_bytes(width: u32, height: u32, fps: f64, duration_secs: f64) -> u64 {
    let frame = frame_bytes(width, height).unwrap_or(0);
    if !fps.is_finite() || !duration_secs.is_finite() || fps <= 0.0 || duration_secs <= 0.0 {
        return 0;
    }
    let frames = (duration_secs * fps).ceil().max(0.0) as u64;
    frames.saturating_mul(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_bytes_do_not_change_with_duration() {
        let staging = staging_ram_bytes(1920, 1080).unwrap();
        let ten = full_rgba_bytes(1920, 1080, 30.0, 10.0);
        let thirty = full_rgba_bytes(1920, 1080, 30.0, 30.0);
        let sixty = full_rgba_bytes(1920, 1080, 30.0, 60.0);
        assert_eq!(staging, 1920 * 1080 * 4 * CPU_FRAMES);
        assert!(sixty > thirty && thirty > ten);
        assert!(staging * 20 < ten);
        assert!(sixty > ten.saturating_mul(5));
    }

    #[test]
    fn rejects_an_empty_frame() {
        assert!(staging_ram_bytes(0, 1080).is_err());
        assert_eq!(full_rgba_bytes(1920, 1080, 0.0, 10.0), 0);
    }
}
