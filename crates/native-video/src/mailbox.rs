//! One published frame. Older frames are dropped, never queued.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::timing::FrameStamp;

#[derive(Debug)]
pub struct VideoFrame {
    pub rgba: Vec<u8>,
    pub stamp: FrameStamp,
}

#[derive(Debug, Default)]
struct Slot {
    frame: Option<Arc<VideoFrame>>,
    discarded_stale: u64,
}

/// Bounded retention: at most one frame is stored. A publish from an older
/// epoch is ignored so a seek or a replaced clip cannot resurface.
#[derive(Debug, Default)]
pub struct Mailbox {
    slot: Mutex<Slot>,
    epoch: std::sync::atomic::AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publish {
    Stored,
    Stale,
}

impl Mailbox {
    pub fn epoch(&self) -> u64 {
        self.epoch.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Bumps the epoch and drops the stored frame.
    pub fn invalidate(&self) -> u64 {
        let next = self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
        self.lock().frame = None;
        next
    }

    pub fn clear(&self) {
        self.lock().frame = None;
    }

    /// On `Stale`, the frame is returned so the caller can reuse its buffer.
    /// On `Stored`, the previous frame is returned only when this mailbox was
    /// its last owner.
    pub fn publish(&self, frame: VideoFrame) -> (Publish, Option<VideoFrame>) {
        let mut slot = self.lock();
        if frame.stamp.epoch != self.epoch() {
            slot.discarded_stale = slot.discarded_stale.saturating_add(1);
            return (Publish::Stale, Some(frame));
        }
        let stored = Arc::new(frame);
        let previous = slot.frame.replace(stored);
        let recyclable = previous.and_then(|frame| Arc::try_unwrap(frame).ok());
        (Publish::Stored, recyclable)
    }

    pub fn try_latest(&self) -> Option<Arc<VideoFrame>> {
        self.slot
            .try_lock()
            .ok()
            .and_then(|slot| slot.frame.clone())
    }

    pub fn latest(&self) -> Option<Arc<VideoFrame>> {
        self.lock().frame.clone()
    }

    pub fn stored_frames(&self) -> usize {
        usize::from(self.lock().frame.is_some())
    }

    pub fn discarded_stale(&self) -> u64 {
        self.lock().discarded_stale
    }

    fn lock(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(|err| err.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(epoch: u64, serial: u64, pts: f64) -> VideoFrame {
        VideoFrame {
            rgba: vec![serial as u8, 0, 0, 255],
            stamp: FrameStamp {
                pts_secs: pts,
                serial,
                epoch,
                width: 1,
                height: 1,
            },
        }
    }

    #[test]
    fn only_the_newest_frame_is_retained() {
        let mailbox = Mailbox::default();
        for serial in 0..50 {
            let (result, _) = mailbox.publish(frame(0, serial, serial as f64 / 30.0));
            assert_eq!(result, Publish::Stored);
            assert_eq!(mailbox.stored_frames(), 1);
        }
        let latest = mailbox.latest().unwrap();
        assert_eq!(latest.stamp.serial, 49);
        assert_eq!(Arc::strong_count(&latest), 2);
        let weak = Arc::downgrade(&latest);
        drop(latest);
        // Not `latest()` again: that hands out a fresh clone, which is itself an owner.
        assert_eq!(
            weak.strong_count(),
            1,
            "the mailbox is the only owner once readers drop the frame"
        );
    }

    #[test]
    fn a_stale_epoch_does_not_replace_the_visible_frame() {
        let mailbox = Mailbox::default();
        mailbox.publish(frame(0, 1, 0.0));
        let epoch = mailbox.invalidate();
        assert_eq!(epoch, 1);
        assert!(mailbox.latest().is_none());
        let (result, _) = mailbox.publish(frame(0, 2, 1.0));
        assert_eq!(result, Publish::Stale);
        assert_eq!(mailbox.discarded_stale(), 1);
        assert!(mailbox.latest().is_none());
        let (result, _) = mailbox.publish(frame(epoch, 3, 0.1));
        assert_eq!(result, Publish::Stored);
        assert_eq!(mailbox.latest().unwrap().stamp.serial, 3);
    }

    #[test]
    fn try_latest_does_not_wait() {
        let mailbox = Mailbox::default();
        assert!(mailbox.try_latest().is_none());
        mailbox.publish(frame(0, 1, 0.0));
        assert!(mailbox.try_latest().is_some());
    }
}
