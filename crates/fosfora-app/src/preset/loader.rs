use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded};

use std::sync::Arc;

use crate::media::decoder::MediaSource;
use crate::media::jobs::MediaBudget;
use crate::preset::Preset;

/// Request sent to the background decode thread.
#[derive(Debug)]
pub struct PresetDecodeRequest {
    pub preset_index: usize,
    pub preset: Preset,
    /// (layer_index, media_path) pairs to decode.
    pub media_jobs: Vec<(usize, PathBuf)>,
    pub generation: u64,
}

/// Result of decoding a single media file.
pub enum MediaDecodeResult {
    Ok(MediaSource),
    Err(String),
}

/// Result sent back from the background thread.
pub struct PresetDecodeResult {
    pub preset_index: usize,
    pub preset: Preset,
    /// layer_index → decoded media (or error).
    pub decoded_media: HashMap<usize, MediaDecodeResult>,
    pub generation: u64,
    /// Budget held for this scene's decoded frames. Release after applying.
    pub reserved: u64,
    /// Set when the scene could not fit beside the one on screen. Nothing
    /// was decoded and the current scene must stay.
    pub refused: Option<String>,
}

/// Loading state visible to UI.
#[derive(Clone)]
pub enum PresetLoadingState {
    Idle,
    Loading {
        preset_name: String,
        preset_index: usize,
    },
}

/// Manages background preset decoding with generation-based cancellation.
pub struct PresetLoader {
    request_tx: Option<Sender<PresetDecodeRequest>>,
    result_rx: Receiver<PresetDecodeResult>,
    generation: u64,
    /// The newest request that has not yet entered the decode queue.
    pending: Option<PresetDecodeRequest>,
    pub state: PresetLoadingState,
    budget: Arc<MediaBudget>,
    _thread: Option<JoinHandle<()>>,
}

/// Test-only pause inside a decode job so a newer request can arrive mid-flight.
#[cfg(test)]
static TEST_DECODE_DELAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl PresetLoader {
    /// A loader with no memory limit. Tests only use this.
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_budget(Arc::new(MediaBudget::new(u64::MAX)))
    }

    pub fn with_budget(budget: Arc<MediaBudget>) -> Self {
        let (request_tx, request_rx) = bounded::<PresetDecodeRequest>(1);
        let (result_tx, result_rx) = bounded::<PresetDecodeResult>(1);

        let thread_budget = budget.clone();
        let handle = thread::Builder::new()
            .name("fosfora-preset-loader".into())
            .spawn(move || {
                Self::decode_thread(request_rx, result_tx, thread_budget);
            })
            .expect("failed to spawn preset loader thread");

        Self {
            request_tx: Some(request_tx),
            result_rx,
            generation: 0,
            pending: None,
            state: PresetLoadingState::Idle,
            budget,
            _thread: Some(handle),
        }
    }

    /// Give back the budget a result held once it is applied or dropped.
    pub fn release(&self, reserved: u64) {
        self.budget.release(reserved);
    }

    /// Submit a new decode request. Bumps generation to cancel any in-flight work.
    pub fn request_load(
        &mut self,
        preset_index: usize,
        preset: Preset,
        media_jobs: Vec<(usize, PathBuf)>,
        preset_name: String,
    ) {
        self.generation += 1;
        let current_gen = self.generation;

        self.state = PresetLoadingState::Loading {
            preset_name,
            preset_index,
        };

        self.pending = Some(PresetDecodeRequest {
            preset_index,
            preset,
            media_jobs,
            generation: current_gen,
        });
        self.flush_pending();
    }

    /// Drop every request that is not already decoding. The in-flight decode
    /// keeps its old generation and is discarded when it returns.
    pub fn supersede(&mut self) {
        self.generation += 1;
        self.pending = None;
        self.state = PresetLoadingState::Idle;
    }

    /// Push the newest pending request onto the decode queue. If the queue is
    /// full the request stays pending and is retried on the next poll.
    fn flush_pending(&mut self) {
        let Some(tx) = self.request_tx.as_ref() else {
            return;
        };
        let Some(request) = self.pending.take() else {
            return;
        };
        if request.generation != self.generation {
            return;
        }
        match tx.try_send(request) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(request)) => {
                self.pending = Some(request);
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                log::warn!("Preset loader queue is closed; the latest preset was not decoded");
                self.state = PresetLoadingState::Idle;
            }
        }
    }

    /// Poll for a completed decode result. Returns Some if a result matching
    /// the current generation is available.
    pub fn try_recv(&mut self) -> Option<PresetDecodeResult> {
        self.flush_pending();
        loop {
            match self.result_rx.try_recv() {
                Ok(result) if result.generation == self.generation => {
                    self.state = PresetLoadingState::Idle;
                    self.pending = None;
                    return Some(result);
                }
                Ok(result) => {
                    self.budget.release(result.reserved);
                    log::debug!(
                        "Discarded stale preset decode result (gen {} vs current {})",
                        result.generation,
                        self.generation
                    );
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    log::warn!("Preset loader thread disconnected");
                    self.state = PresetLoadingState::Idle;
                    return None;
                }
            }
        }
    }

    /// Background thread: receive requests, decode media, send results.
    fn decode_thread(
        request_rx: Receiver<PresetDecodeRequest>,
        result_tx: Sender<PresetDecodeResult>,
        budget: Arc<MediaBudget>,
    ) {
        loop {
            // Block waiting for next request
            let mut request = match request_rx.recv() {
                Ok(r) => r,
                Err(_) => {
                    // Channel closed — App is shutting down
                    log::debug!("Preset loader thread exiting (channel closed)");
                    return;
                }
            };

            // Before starting work, drain any newer request that arrived
            while let Ok(newer) = request_rx.try_recv() {
                log::debug!(
                    "Preset loader: skipping gen {} for newer gen {}",
                    request.generation,
                    newer.generation
                );
                request = newer;
            }

            'decode: loop {
                let mut decoded_media = HashMap::new();
                let mut cancelled = false;

                // Probe before decoding. The scene on screen stays resident
                // until this one is applied, so both count toward the budget.
                let mut reserved = 0u64;
                for (layer_idx, path) in &request.media_jobs {
                    let plan = crate::media::library::plan_clip(path);
                    match plan.problem {
                        Some(problem) => {
                            decoded_media.insert(
                                *layer_idx,
                                MediaDecodeResult::Err(format!("{}: {problem}", plan.name)),
                            );
                        }
                        None => reserved += plan.ram_bytes,
                    }
                }
                if let Err(message) = budget.reserve(reserved) {
                    log::warn!(
                        "Preset '{}' refused before decode: {message}",
                        request.preset_index
                    );
                    let result = PresetDecodeResult {
                        preset_index: request.preset_index,
                        preset: request.preset,
                        decoded_media: HashMap::new(),
                        generation: request.generation,
                        reserved: 0,
                        refused: Some(format!("Scene not loaded. {message}")),
                    };
                    if result_tx.send(result).is_err() {
                        return;
                    }
                    break;
                }

                let num_jobs = request.media_jobs.len();
                for job_idx in 0..num_jobs {
                    #[cfg(test)]
                    {
                        let delay = TEST_DECODE_DELAY_MS.load(std::sync::atomic::Ordering::Relaxed);
                        if delay > 0 {
                            thread::sleep(std::time::Duration::from_millis(delay));
                        }
                    }
                    // Check for cancellation between jobs
                    if let Ok(newer) = request_rx.try_recv() {
                        log::debug!(
                            "Preset loader: cancelled gen {} mid-decode, starting gen {}",
                            request.generation,
                            newer.generation
                        );
                        request = newer;
                        cancelled = true;
                        break;
                    }

                    let (layer_idx, ref path) = request.media_jobs[job_idx];
                    if decoded_media.contains_key(&layer_idx) {
                        continue;
                    }
                    log::info!("Decoding media for layer {}: {}", layer_idx, path.display());
                    let result = match crate::media::decoder::load_media(path) {
                        Ok(source) => MediaDecodeResult::Ok(source),
                        Err(e) => {
                            log::warn!(
                                "Failed to decode media '{}' for preset: {}",
                                path.display(),
                                e
                            );
                            MediaDecodeResult::Err(e)
                        }
                    };
                    decoded_media.insert(layer_idx, result);
                }

                if cancelled {
                    budget.release(reserved);
                    continue 'decode;
                }
                // A newer request can arrive during the last job. It must win
                // before this result is published.
                if let Ok(newer) = request_rx.try_recv() {
                    budget.release(reserved);
                    request = newer;
                    continue 'decode;
                }

                let result = PresetDecodeResult {
                    preset_index: request.preset_index,
                    preset: request.preset,
                    decoded_media,
                    generation: request.generation,
                    reserved,
                    refused: None,
                };

                // Block until the frame loop takes the result. Dropping it
                // leaves the loader stuck on Loading for that generation.
                if result_tx.send(result).is_err() {
                    log::debug!("Preset loader thread exiting (result channel closed)");
                    return;
                }
                break;
            }
        }
    }
}

impl Drop for PresetLoader {
    fn drop(&mut self) {
        // Drop the sender to signal thread exit
        self.request_tx.take();
        // Join the thread
        if let Some(handle) = self._thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_state_starts_idle() {
        let loader = PresetLoader::new();
        assert!(matches!(loader.state, PresetLoadingState::Idle));
    }

    #[test]
    fn generation_increments() {
        let mut loader = PresetLoader::new();
        assert_eq!(loader.generation, 0);

        let preset = Preset {
            layers: vec![],
            active_layer: 0,
            postprocess: Default::default(),
            volumetric: None,
            master_chain: None,
        };

        loader.request_load(0, preset.clone(), vec![], "Test".into());
        assert_eq!(loader.generation, 1);

        loader.request_load(1, preset, vec![], "Test2".into());
        assert_eq!(loader.generation, 2);
    }

    #[test]
    fn request_sets_loading_state() {
        let mut loader = PresetLoader::new();
        let preset = Preset {
            layers: vec![],
            active_layer: 0,
            postprocess: Default::default(),
            volumetric: None,
            master_chain: None,
        };

        loader.request_load(3, preset, vec![], "My Preset".into());

        match &loader.state {
            PresetLoadingState::Loading {
                preset_name,
                preset_index,
            } => {
                assert_eq!(preset_name, "My Preset");
                assert_eq!(*preset_index, 3);
            }
            _ => panic!("Expected Loading state"),
        }
    }

    #[test]
    fn empty_media_jobs_returns_result() {
        let mut loader = PresetLoader::new();
        let preset = Preset {
            layers: vec![],
            active_layer: 0,
            postprocess: Default::default(),
            volumetric: None,
            master_chain: None,
        };

        loader.request_load(0, preset, vec![], "Empty".into());

        // Give thread time to process
        std::thread::sleep(std::time::Duration::from_millis(50));

        let result = loader.try_recv();
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.preset_index, 0);
        assert!(result.decoded_media.is_empty());
        assert!(matches!(loader.state, PresetLoadingState::Idle));
    }

    #[test]
    fn missing_file_returns_error() {
        let mut loader = PresetLoader::new();
        let preset = Preset {
            layers: vec![],
            active_layer: 0,
            postprocess: Default::default(),
            volumetric: None,
            master_chain: None,
        };

        let jobs = vec![(0, PathBuf::from("/nonexistent/fake_image.png"))];
        loader.request_load(0, preset, jobs, "Bad".into());

        std::thread::sleep(std::time::Duration::from_millis(100));

        let result = loader.try_recv();
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.decoded_media.len(), 1);
        assert!(matches!(
            result.decoded_media.get(&0),
            Some(MediaDecodeResult::Err(_))
        ));
    }

    #[test]
    fn stale_result_discarded() {
        let mut loader = PresetLoader::new();
        let preset = Preset {
            layers: vec![],
            active_layer: 0,
            postprocess: Default::default(),
            volumetric: None,
            master_chain: None,
        };

        // Send first request
        loader.request_load(0, preset.clone(), vec![], "First".into());
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Bump generation without sending (simulates a rapid request cycle)
        loader.generation = 999;

        // The result from gen 1 should be discarded
        let result = loader.try_recv();
        assert!(result.is_none());
    }

    #[test]
    fn rapid_requests_apply_the_latest_preset() {
        let _guard = DelayGuard;
        TEST_DECODE_DELAY_MS.store(80, std::sync::atomic::Ordering::Relaxed);
        let mut loader = PresetLoader::new();
        let preset = empty_preset();
        let job = vec![(0, PathBuf::from("/nonexistent/a.png"))];
        loader.request_load(0, preset.clone(), job.clone(), "A".into());
        std::thread::sleep(std::time::Duration::from_millis(20));
        loader.request_load(1, preset.clone(), job.clone(), "B".into());
        loader.request_load(2, preset, job, "C".into());

        let mut applied = None;
        for _ in 0..50 {
            if let Some(result) = loader.try_recv() {
                applied = Some(result.preset_index);
            }
            if applied == Some(2) && matches!(loader.state, PresetLoadingState::Idle) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        assert_eq!(
            applied,
            Some(2),
            "the last requested preset must be the one applied"
        );
        assert!(matches!(loader.state, PresetLoadingState::Idle));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(
            loader.try_recv().is_none(),
            "a later decode must not overwrite C"
        );
        assert!(matches!(loader.state, PresetLoadingState::Idle));
    }

    struct DelayGuard;
    impl Drop for DelayGuard {
        fn drop(&mut self) {
            TEST_DECODE_DELAY_MS.store(0, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[test]
    fn a_scene_that_cannot_fit_beside_the_current_one_is_refused_before_decode() {
        let dir =
            std::env::temp_dir().join(format!("fosfora-loader-budget-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let still = dir.join("plate.png");
        image::RgbaImage::new(32, 32).save(&still).unwrap();
        let need = 32 * 32 * 4;

        // The scene on screen holds most of the budget.
        let budget = Arc::new(MediaBudget::new(need + 100));
        budget.set_resident(200);
        let mut loader = PresetLoader::with_budget(budget.clone());
        loader.request_load(0, empty_preset(), vec![(0, still.clone())], "Big".into());
        let result = wait_for(&mut loader);
        assert!(
            result
                .refused
                .as_deref()
                .is_some_and(|m| m.contains("Scene not loaded"))
        );
        assert!(result.decoded_media.is_empty(), "nothing may be decoded");
        assert_eq!(budget.in_flight(), 0);

        // With room, the same scene decodes and holds its bytes until released.
        budget.set_resident(0);
        loader.request_load(0, empty_preset(), vec![(0, still)], "Big".into());
        let result = wait_for(&mut loader);
        assert!(result.refused.is_none());
        assert_eq!(result.reserved, need);
        assert_eq!(budget.in_flight(), need);
        loader.release(result.reserved);
        assert_eq!(budget.in_flight(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn wait_for(loader: &mut PresetLoader) -> PresetDecodeResult {
        for _ in 0..400 {
            if let Some(result) = loader.try_recv() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("loader did not answer");
    }

    fn empty_preset() -> Preset {
        Preset {
            layers: vec![],
            active_layer: 0,
            postprocess: Default::default(),
            volumetric: None,
            master_chain: None,
        }
    }
}
