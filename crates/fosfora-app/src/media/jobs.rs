//! Media work that must not run on the render thread.
//!
//! Adding or replacing a clip probes it with ffprobe, checks the decoded
//! size against [`MediaBudget`], and only then decodes. All three happen on
//! one worker thread with a short queue. The render thread sends a job and,
//! frames later, takes a finished [`MediaSource`] and makes the GPU layer from
//! it. A refused or failed job changes nothing on screen.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded};

use super::decoder::MediaSource;
use super::library::{self, ClipPlan, MAX_DECODED_RAM, format_bytes};

/// Jobs that may wait behind the one decoding. More are refused, not queued.
const QUEUE_DEPTH: usize = 2;

/// Decoded-memory budget shared by the preset loader and the media worker.
///
/// `resident` is what the render thread holds right now: decoded frames of
/// every live layer plus an estimate of GPU targets. The render thread
/// refreshes it each frame. `in_flight` is reserved by decodes that have
/// passed the check and not yet been applied, so the outgoing scene, the
/// incoming one and any add or replace all count toward one peak.
pub struct MediaBudget {
    limit: u64,
    resident: AtomicU64,
    in_flight: AtomicU64,
}

impl MediaBudget {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            resident: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
        }
    }

    /// 6 GB, or 40% of physical memory when that is less.
    ///
    /// `FOSFORA_DECODE_BUDGET_PERCENT` (set by `make server-start`) uses that
    /// share of RAM and does not apply the 6 GB cap. Stopping the process
    /// drops the variable, so the next launch uses the normal cap.
    pub fn for_this_machine() -> Self {
        let raw = std::env::var("FOSFORA_DECODE_BUDGET_PERCENT").ok();
        let percent = raw.as_deref().and_then(parse_budget_percent);
        if raw.is_some() && percent.is_none() {
            log::warn!(
                "FOSFORA_DECODE_BUDGET_PERCENT is not a percent from 1 to 90; using the normal budget"
            );
        }
        let limit = budget_limit(physical_memory(), percent);
        if let Some(percent) = percent {
            log::info!(
                "Decode budget override: {percent}% of RAM ({})",
                format_bytes(limit)
            );
        }
        Self::new(limit)
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    pub fn set_resident(&self, bytes: u64) {
        self.resident.store(bytes, Ordering::Relaxed);
    }

    pub fn resident(&self) -> u64 {
        self.resident.load(Ordering::Relaxed)
    }

    pub fn in_flight(&self) -> u64 {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Bytes that may still be decoded.
    pub fn available(&self) -> u64 {
        self.limit
            .saturating_sub(self.resident())
            .saturating_sub(self.in_flight())
    }

    /// Hold `bytes` for a decode, or say why not. Nothing is held on error.
    pub fn reserve(&self, bytes: u64) -> Result<(), String> {
        let mut held = self.in_flight.load(Ordering::Relaxed);
        loop {
            let total = self.resident().saturating_add(held).saturating_add(bytes);
            if total > self.limit {
                return Err(format!(
                    "Decoding needs {}; {} is in use and {} is loading, of a {} budget.",
                    format_bytes(bytes),
                    format_bytes(self.resident()),
                    format_bytes(held),
                    format_bytes(self.limit)
                ));
            }
            match self.in_flight.compare_exchange_weak(
                held,
                held + bytes,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(()),
                Err(now) => held = now,
            }
        }
    }

    pub fn release(&self, bytes: u64) {
        let _ = self
            .in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |held| {
                Some(held.saturating_sub(bytes))
            });
    }
}

/// `Some` for a whole percent from 1 through 90. Anything else is ignored.
pub fn parse_budget_percent(raw: &str) -> Option<u64> {
    let percent: u64 = raw.trim().parse().ok()?;
    (1..=90).contains(&percent).then_some(percent)
}

/// Decoded-RAM ceiling. An override is that percent of physical memory.
/// The normal ceiling is 40% of RAM, and never more than [`MAX_DECODED_RAM`].
pub fn budget_limit(physical: Option<u64>, percent_override: Option<u64>) -> u64 {
    match (physical, percent_override) {
        (Some(bytes), Some(percent)) => bytes.saturating_mul(percent) / 100,
        (Some(bytes), None) => (bytes / 5 * 2).min(MAX_DECODED_RAM),
        (None, _) => MAX_DECODED_RAM,
    }
}

pub(crate) fn physical_memory() -> Option<u64> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

/// Where a finished clip goes. Checked again when it lands, because the
/// stack can change while the clip decodes.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaTarget {
    /// New layers at the back, in order.
    Add,
    /// Swap one layer's media. `previous` is the file it showed when asked.
    Replace {
        layer: usize,
        previous: Option<PathBuf>,
    },
}

struct MediaJob {
    scene_generation: u64,
    target: MediaTarget,
    paths: Vec<PathBuf>,
    free_slots: usize,
}

pub enum MediaJobOutcome {
    Ready(Vec<(PathBuf, MediaSource)>),
    Refused(String),
}

pub struct MediaJobResult {
    pub scene_generation: u64,
    pub target: MediaTarget,
    /// Budget held for this job. Release it once the result is applied or dropped.
    pub reserved: u64,
    pub outcome: MediaJobOutcome,
}

pub struct MediaJobs {
    tx: Sender<MediaJob>,
    rx: Receiver<MediaJobResult>,
    budget: Arc<MediaBudget>,
    outstanding: usize,
}

impl MediaJobs {
    pub fn new(budget: Arc<MediaBudget>) -> Self {
        let (tx, job_rx) = bounded::<MediaJob>(QUEUE_DEPTH);
        let (result_tx, rx) = bounded::<MediaJobResult>(QUEUE_DEPTH + 1);
        let worker_budget = budget.clone();
        thread::Builder::new()
            .name("fosfora-media-jobs".into())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let result = run_job(job, &worker_budget);
                    if result_tx.send(result).is_err() {
                        return;
                    }
                }
            })
            .expect("failed to spawn media job thread");
        Self {
            tx,
            rx,
            budget,
            outstanding: 0,
        }
    }

    /// Queue a job. Refused when the queue is full; nothing waits unbounded.
    pub fn submit(
        &mut self,
        scene_generation: u64,
        target: MediaTarget,
        paths: Vec<PathBuf>,
        free_slots: usize,
    ) -> Result<(), String> {
        let job = MediaJob {
            scene_generation,
            target,
            paths,
            free_slots,
        };
        match self.tx.try_send(job) {
            Ok(()) => {
                self.outstanding += 1;
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                Err("Media is still loading. Try again when the current clip is ready.".into())
            }
            Err(TrySendError::Disconnected(_)) => Err("The media worker stopped".into()),
        }
    }

    pub fn try_recv(&mut self) -> Option<MediaJobResult> {
        match self.rx.try_recv() {
            Ok(result) => {
                self.outstanding = self.outstanding.saturating_sub(1);
                Some(result)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }

    /// Jobs sent and not yet taken back.
    #[cfg(test)]
    pub fn outstanding(&self) -> usize {
        self.outstanding
    }

    pub fn budget(&self) -> &Arc<MediaBudget> {
        &self.budget
    }
}

fn run_job(job: MediaJob, budget: &MediaBudget) -> MediaJobResult {
    let refuse = |job: MediaJob, message: String| MediaJobResult {
        scene_generation: job.scene_generation,
        target: job.target,
        reserved: 0,
        outcome: MediaJobOutcome::Refused(message),
    };
    // ffprobe each clip before any frame is decoded.
    let plans: Vec<ClipPlan> = job
        .paths
        .iter()
        .map(|path| library::plan_clip(path))
        .collect();
    if let Some(message) = library::admission_error(
        crate::bindings::catalog::MAX_LAYERS - job.free_slots,
        budget.available(),
        crate::bindings::catalog::MAX_LAYERS,
        &plans,
    ) {
        return refuse(job, message);
    }
    let bytes: u64 = plans.iter().map(|plan| plan.ram_bytes).sum();
    if let Err(message) = budget.reserve(bytes) {
        return refuse(job, format!("Nothing was added.\n{message}"));
    }
    let mut sources = Vec::with_capacity(job.paths.len());
    for path in &job.paths {
        match super::decoder::load_media(path) {
            Ok(source) => sources.push((path.clone(), source)),
            Err(err) => {
                budget.release(bytes);
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("clip")
                    .to_string();
                return refuse(job, format!("{name}: {err}. The layer was left as it was."));
            }
        }
    }
    MediaJobResult {
        scene_generation: job.scene_generation,
        target: job.target,
        reserved: bytes,
        outcome: MediaJobOutcome::Ready(sources),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::library::MAX_CLIP_RAM;

    #[test]
    fn normal_budget_stays_at_most_six_gigabytes() {
        let sixteen = 16 * 1024 * 1024 * 1024;
        assert_eq!(budget_limit(Some(sixteen), None), MAX_DECODED_RAM);
        let eight = 8 * 1024 * 1024 * 1024;
        assert_eq!(budget_limit(Some(eight), None), eight / 5 * 2);
        assert_eq!(budget_limit(None, Some(70)), MAX_DECODED_RAM);
    }

    #[test]
    fn percent_override_skips_the_six_gigabyte_cap() {
        let sixteen = 16 * 1024 * 1024 * 1024;
        let elevated = budget_limit(Some(sixteen), Some(70));
        assert_eq!(elevated, sixteen.saturating_mul(70) / 100);
        assert!(elevated > MAX_DECODED_RAM);
        assert_eq!(parse_budget_percent("70"), Some(70));
        assert_eq!(parse_budget_percent(" 70 "), Some(70));
        assert_eq!(parse_budget_percent("0"), None);
        assert_eq!(parse_budget_percent("91"), None);
        assert_eq!(parse_budget_percent("nope"), None);
    }

    #[test]
    fn a_55_second_720p_clip_fits_one_elevated_slot() {
        let sixteen = 16 * 1024 * 1024 * 1024;
        let session = budget_limit(Some(sixteen), Some(70)).max(MAX_CLIP_RAM);
        let slumber = 1280u64 * 720 * 4 * 1333;
        assert!(slumber > MAX_CLIP_RAM);
        assert!(slumber < session);
    }

    #[test]
    fn budget_counts_resident_and_in_flight_together() {
        let budget = MediaBudget::new(1000);
        budget.set_resident(600);
        assert!(budget.reserve(300).is_ok());
        assert_eq!(budget.available(), 100);
        let err = budget.reserve(200).unwrap_err();
        assert!(err.contains("loading"), "{err}");
        budget.release(300);
        assert!(budget.reserve(400).is_ok());
        budget.release(10_000);
        assert_eq!(budget.in_flight(), 0);
    }

    #[test]
    fn an_unreadable_clip_is_refused_and_holds_no_budget() {
        let budget = Arc::new(MediaBudget::new(u64::MAX / 2));
        let mut jobs = MediaJobs::new(budget.clone());
        let missing = std::env::temp_dir().join("fosfora-no-such-still.png");
        jobs.submit(1, MediaTarget::Add, vec![missing], 4).unwrap();
        let result = loop {
            if let Some(result) = jobs.try_recv() {
                break result;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(matches!(result.outcome, MediaJobOutcome::Refused(_)));
        assert_eq!(result.reserved, 0);
        assert_eq!(budget.in_flight(), 0);
        assert_eq!(jobs.outstanding(), 0);
    }

    #[test]
    fn a_still_over_the_budget_is_refused_before_decode() {
        let dir = std::env::temp_dir().join(format!("fosfora-jobs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let still = dir.join("plate.png");
        image::RgbaImage::new(64, 64).save(&still).unwrap();
        // 64×64×4 = 16 KiB decoded; leave less than that.
        let budget = Arc::new(MediaBudget::new(20_000));
        budget.set_resident(10_000);
        let mut jobs = MediaJobs::new(budget.clone());
        jobs.submit(1, MediaTarget::Add, vec![still.clone()], 4)
            .unwrap();
        let result = loop {
            if let Some(result) = jobs.try_recv() {
                break result;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        match result.outcome {
            MediaJobOutcome::Refused(message) => {
                assert!(message.contains("memory budget"), "{message}");
            }
            MediaJobOutcome::Ready(_) => panic!("decoded past the budget"),
        }
        assert_eq!(budget.in_flight(), 0);
        budget.set_resident(0);
        jobs.submit(1, MediaTarget::Add, vec![still], 4).unwrap();
        let result = loop {
            if let Some(result) = jobs.try_recv() {
                break result;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(matches!(result.outcome, MediaJobOutcome::Ready(_)));
        assert_eq!(result.reserved, 64 * 64 * 4);
        assert_eq!(budget.in_flight(), 64 * 64 * 4);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
