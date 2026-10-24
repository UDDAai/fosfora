//! Playback decisions that do not touch AVFoundation.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStamp {
    pub pts_secs: f64,
    pub serial: u64,
    pub epoch: u64,
    pub width: u32,
    pub height: u32,
}

/// Frames skipped between two presentation timestamps. A backwards jump is a
/// seek or a loop, not a drop.
pub fn dropped_frames(previous: Option<f64>, pts: f64, frame_secs: f64) -> u64 {
    let Some(previous) = previous else {
        return 0;
    };
    if !pts.is_finite() || !previous.is_finite() || !frame_secs.is_finite() || frame_secs <= 0.0 {
        return 0;
    }
    if pts <= previous {
        return 0;
    }
    let spanned = ((pts - previous) / frame_secs).round();
    if spanned >= 2.0 {
        (spanned as u64).saturating_sub(1)
    } else {
        0
    }
}

pub fn at_end(position: f64, duration: f64, slop: f64) -> bool {
    duration > 0.0 && position.is_finite() && position >= duration - slop.max(0.0)
}

/// A frame is shown when it lands near the seek target. Callers give up after
/// a short wall-clock wait so a coarse seek cannot stall the picture.
pub fn frame_matches_seek(target: f64, pts: f64) -> bool {
    target.is_finite() && pts.is_finite() && (pts - target).abs() <= 0.30
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShownClock {
    pub secs: f64,
    pending_seek: Option<f64>,
}

impl ShownClock {
    pub fn new() -> Self {
        Self {
            secs: 0.0,
            pending_seek: None,
        }
    }

    pub fn seek(&mut self, secs: f64) {
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        self.pending_seek = Some(secs);
        self.secs = secs;
    }

    pub fn on_frame(&mut self, pts: f64) {
        if !pts.is_finite() || pts < 0.0 {
            return;
        }
        if let Some(target) = self.pending_seek
            && frame_matches_seek(target, pts)
        {
            self.pending_seek = None;
        }
        if self.pending_seek.is_none() {
            self.secs = pts;
        }
    }
}

impl Default for ShownClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_count_as_drops_and_loops_do_not() {
        let frame = 1.0 / 30.0;
        assert_eq!(dropped_frames(None, 0.0, frame), 0);
        assert_eq!(dropped_frames(Some(0.0), frame, frame), 0);
        assert_eq!(dropped_frames(Some(frame), frame * 4.0, frame), 2);
        assert_eq!(dropped_frames(Some(0.9), 0.02, frame), 0);
    }

    #[test]
    fn end_of_clip_is_within_one_frame() {
        assert!(at_end(0.98, 1.0, 0.04));
        assert!(!at_end(0.5, 1.0, 0.04));
        assert!(!at_end(1.0, 0.0, 0.04));
    }

    #[test]
    fn seek_keeps_the_target_until_a_matching_frame() {
        let mut clock = ShownClock::new();
        clock.on_frame(0.2);
        clock.seek(1.5);
        assert!((clock.secs - 1.5).abs() < 1e-9);
        clock.on_frame(0.25);
        assert!((clock.secs - 1.5).abs() < 1e-9);
        clock.on_frame(1.42);
        assert!((clock.secs - 1.42).abs() < 1e-9);
        assert!(frame_matches_seek(0.0, 0.05));
        assert!(!frame_matches_seek(0.0, 0.9));
    }
}
