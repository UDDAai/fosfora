use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::clock::{ClockTransport, ShowClock};
use super::definition::ShowDefinition;
use super::scheduler::{SceneScheduler, ScheduledSceneAction};
use super::validation::{MediaEstimate, ValidationReport, hibernation_acceptance, validate_show};
use crate::palette::bindings::PaletteBindingSet;
use crate::palette::controller::{PaletteScheduler, palette_colors_at};
use crate::palette::types::Palette;
use crate::params::ParamValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShowCommand {
    Start,
    Pause,
    Resume,
    Reset,
    Seek {
        secs: u32,
    },
    StopAuto,
    NextVisual,
    PrevVisual,
    NextPalette,
    PrevPalette,
    /// Pick a palette by id. Holds until the next scheduled cue or PaletteAuto.
    SelectPalette {
        id: String,
    },
    /// Drop the manual hold and restore the colour scheduled at the clock.
    PaletteAuto,
    ToggleBlackout,
}

#[derive(Debug, Clone)]
pub enum ShowVisualEvent {
    Scene(ScheduledSceneAction),
    Palette(HashMap<String, ParamValue>),
}

pub struct ShowController {
    pub definition: ShowDefinition,
    pub pack_root: PathBuf,
    pub clock: ShowClock,
    pub auto_enabled: bool,
    pub scene: SceneScheduler,
    pub palette: PaletteScheduler,
    pub palettes: HashMap<String, Palette>,
    pub bindings: HashMap<String, PaletteBindingSet>,
    pub preset_ids: Vec<String>,
    pub last_report: ValidationReport,
}

impl ShowController {
    pub fn new(
        definition: ShowDefinition,
        pack_root: PathBuf,
        palettes: HashMap<String, Palette>,
        bindings: HashMap<String, PaletteBindingSet>,
        preset_ids: Vec<String>,
    ) -> Self {
        Self {
            definition,
            pack_root,
            clock: ShowClock::new(),
            auto_enabled: false,
            scene: SceneScheduler::default(),
            palette: PaletteScheduler::default(),
            palettes,
            bindings,
            preset_ids,
            last_report: ValidationReport { issues: Vec::new() },
        }
    }

    /// Plan p1-paths: every relative show-pack path resolves against the
    /// show.json directory. Preset load uses this so a pack-relative
    /// `media_path` is not looked up from the process working directory.
    pub fn resolve_path(&self, stored: &str) -> PathBuf {
        crate::media::library::resolve_pack_media_path(&self.pack_root, stored)
    }

    pub fn validate(
        &mut self,
        pack_presets: &HashMap<String, crate::preset::Preset>,
        media: &[MediaEstimate],
        conflicts: &[(String, usize, String)],
    ) -> ValidationReport {
        let preset_set = self.preset_ids.iter().cloned().collect();
        // Color targets come from the pack presets themselves: any (layer,
        // param) whose stored value is ParamValue::Color. No caller inventory
        // needed — bindings must point at Color params by construction.
        let mut color_params = std::collections::HashSet::new();
        for (preset_id, set) in &self.bindings {
            let Some(preset) = pack_presets
                .get(preset_id)
                .or_else(|| pack_presets.get(&set.preset))
            else {
                continue;
            };
            for b in &set.bindings {
                if preset
                    .layers
                    .get(b.layer)
                    .and_then(|l| l.params.get(&b.parameter))
                    .is_some_and(|v| matches!(v, ParamValue::Color(_)))
                {
                    color_params.insert((b.layer, b.parameter.clone()));
                }
            }
        }
        let mut report = validate_show(
            &self.definition,
            &self.palettes,
            &self.bindings,
            &preset_set,
            media,
            &color_params,
            conflicts,
        );
        report
            .issues
            .extend(hibernation_acceptance(&self.definition));
        self.last_report = report.clone();
        report
    }

    /// (layer, param) pairs owned by palette automation, for conflict scans.
    /// Derived from the open pack's binding sets — callers pass live MIDI /
    /// OSC / binding-bus targets to [`Self::palette_conflicts`] to build the
    /// validator's `conflicting_owners` list.
    pub fn palette_owned_params(&self) -> std::collections::HashSet<(usize, String)> {
        self.bindings
            .values()
            .flat_map(|set| set.bindings.iter())
            .map(|b| (b.layer, b.parameter.clone()))
            .collect()
    }

    /// Build the validator `conflicting_owners` list: every (owner, layer,
    /// param) where an external control path (audio/MIDI/OSC/binding-bus)
    /// targets a palette-owned color param. Empty when clean.
    pub fn palette_conflicts(
        &self,
        external_targets: impl IntoIterator<Item = (String, usize, String)>,
    ) -> Vec<(String, usize, String)> {
        let owned = self.palette_owned_params();
        external_targets
            .into_iter()
            .filter(|(_, layer, param)| owned.contains(&(*layer, param.clone())))
            .collect()
    }

    pub fn handle(&mut self, now: Instant, cmd: ShowCommand) -> Vec<ShowVisualEvent> {
        match cmd {
            ShowCommand::Start => {
                self.auto_enabled = true;
                self.clock.start(now);
                let mut out = Vec::new();
                if let Some(scene) = self.scene.start_cut(&self.definition.scene_track) {
                    out.push(ShowVisualEvent::Scene(scene));
                }
                self.palette.reset();
                if let Some(colors) = self.palette_at(Duration::ZERO) {
                    out.push(ShowVisualEvent::Palette(colors));
                }
                out
            }
            ShowCommand::Pause => {
                self.clock.pause(now);
                Vec::new()
            }
            ShowCommand::Resume => {
                self.clock.resume(now);
                Vec::new()
            }
            ShowCommand::Reset => {
                self.clock.reset();
                self.scene = SceneScheduler::default();
                self.palette.reset();
                self.seek_events(0)
            }
            ShowCommand::Seek { secs } => {
                let pos = Duration::from_secs(u64::from(secs));
                self.clock.seek(now, pos);
                self.seek_events(secs)
            }
            ShowCommand::StopAuto => {
                self.auto_enabled = false;
                Vec::new()
            }
            ShowCommand::NextVisual => {
                if let Some(id) = self.step_preset(1) {
                    self.scene.manual_set_preset(id.clone());
                    vec![ShowVisualEvent::Scene(ScheduledSceneAction {
                        preset_id: id,
                        kind: crate::show::definition::TransitionKind::Cut,
                        duration_ms: 0,
                        snap: true,
                    })]
                } else {
                    Vec::new()
                }
            }
            ShowCommand::PrevVisual => {
                if let Some(id) = self.step_preset(-1) {
                    self.scene.manual_set_preset(id.clone());
                    vec![ShowVisualEvent::Scene(ScheduledSceneAction {
                        preset_id: id,
                        kind: crate::show::definition::TransitionKind::Cut,
                        duration_ms: 0,
                        snap: true,
                    })]
                } else {
                    Vec::new()
                }
            }
            ShowCommand::NextPalette => self.manual_palette(1),
            ShowCommand::PrevPalette => self.manual_palette(-1),
            ShowCommand::SelectPalette { id } => {
                if !self.palettes.contains_key(&id) {
                    return Vec::new();
                }
                self.palette.manual_set(id.clone());
                self.palettes
                    .get(&id)
                    .map(|p| vec![ShowVisualEvent::Palette(p.as_param_map())])
                    .unwrap_or_default()
            }
            ShowCommand::PaletteAuto => {
                let elapsed = self.clock.elapsed_at(now);
                self.palette_at(elapsed)
                    .map(|colors| vec![ShowVisualEvent::Palette(colors)])
                    .unwrap_or_default()
            }
            ShowCommand::ToggleBlackout => Vec::new(),
        }
    }

    pub fn tick(&mut self, now: Instant) -> Vec<ShowVisualEvent> {
        if !self.auto_enabled || self.clock.transport() != ClockTransport::Running {
            return Vec::new();
        }
        let elapsed = self.clock.elapsed_at(now);
        let secs = elapsed.as_secs().min(u64::from(u32::MAX)) as u32;
        let mut out = Vec::new();
        if let Some(scene) = self.scene.tick(&self.definition.scene_track, secs) {
            out.push(ShowVisualEvent::Scene(scene));
        }
        if let Some(colors) =
            self.palette
                .tick(&self.definition.palette_track, &self.palettes, elapsed)
        {
            out.push(ShowVisualEvent::Palette(colors));
        }
        out
    }

    fn seek_events(&mut self, secs: u32) -> Vec<ShowVisualEvent> {
        let mut out = Vec::new();
        if let Some(scene) = self.scene.seek_snap(&self.definition.scene_track, secs) {
            out.push(ShowVisualEvent::Scene(scene));
        }
        if let Some(colors) = self.palette_at(Duration::from_secs(u64::from(secs))) {
            out.push(ShowVisualEvent::Palette(colors));
        }
        out
    }

    fn palette_at(&mut self, elapsed: Duration) -> Option<HashMap<String, ParamValue>> {
        self.palette
            .sync_index(&self.definition.palette_track, elapsed);
        palette_colors_at(&self.definition.palette_track, &self.palettes, elapsed)
    }

    /// Manual NEXT/PREV walks the show's scene track and stops at either end.
    ///
    /// Built-in Fosfora presets live in `preset_ids` but are not Hibernation
    /// scenes, so they must not enter this sequence. The show clock is not
    /// moved here; `manual_set_preset` only changes which scene is current.
    fn step_preset(&self, delta: i32) -> Option<String> {
        let ids: Vec<&str> = self
            .definition
            .scene_track
            .iter()
            .map(|cue| cue.preset_id.as_str())
            .collect();
        if ids.is_empty() {
            return None;
        }
        let current = self.scene.active_preset_id.as_deref().unwrap_or("");
        let idx = ids.iter().position(|id| *id == current)?;
        let next = idx as i32 + delta;
        if next < 0 || next >= ids.len() as i32 {
            return None;
        }
        Some(ids[next as usize].to_string())
    }

    fn manual_palette(&mut self, delta: i32) -> Vec<ShowVisualEvent> {
        let ids: Vec<String> = self
            .definition
            .palette_track
            .iter()
            .map(|c| c.palette_id.clone())
            .collect();
        if ids.is_empty() {
            return Vec::new();
        }
        let current = self
            .palette
            .active_palette_id
            .clone()
            .or_else(|| ids.first().cloned())
            .unwrap_or_default();
        let idx = ids.iter().position(|id| id == &current).unwrap_or(0);
        let next = (idx as i32 + delta).rem_euclid(ids.len() as i32) as usize;
        let id = ids[next].clone();
        self.palette.manual_set(id.clone());
        if let Some(p) = self.palettes.get(&id) {
            vec![ShowVisualEvent::Palette(p.as_param_map())]
        } else {
            Vec::new()
        }
    }

    pub fn snapshot(&self, now: Instant, blackout: bool) -> ShowSnapshot {
        let elapsed = self.clock.elapsed_at(now);
        let secs = elapsed.as_secs().min(u64::from(u32::MAX)) as u32;
        let scene_idx = self.definition.scene_index_at(secs);
        let next_scene = scene_idx.and_then(|i| self.definition.scene_track.get(i + 1));
        let pal_idx = self.definition.palette_index_at(secs);
        let next_pal = pal_idx.and_then(|i| self.definition.palette_track.get(i + 1));
        ShowSnapshot {
            loaded: true,
            auto_enabled: self.auto_enabled,
            running: self.clock.transport() == ClockTransport::Running,
            elapsed_ms: elapsed.as_millis() as u64,
            scene: self.scene.active_preset_id.clone(),
            next_scene: next_scene.map(|c| c.preset_id.clone()),
            palette: self.palette.active_palette_id.clone(),
            next_palette: next_pal.map(|c| c.palette_id.clone()),
            next_scene_secs: next_scene.map(|c| c.at_secs),
            next_palette_secs: next_pal.map(|c| c.at_secs),
            palette_manual: self.palette.manual,
            palettes: {
                let mut ids: Vec<String> = self.palettes.keys().cloned().collect();
                ids.sort();
                ids
            },
            palette_colors: palette_strips(&self.palettes),
            transport: match self.clock.transport() {
                ClockTransport::Running => "running",
                ClockTransport::Paused => "paused",
                ClockTransport::Idle => "idle",
            }
            .to_string(),
            blackout,
            validation: self.last_report.summary(),
            grade_enabled: false,
            grade_strength: 0.0,
            notice: String::new(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PaletteStrip {
    pub id: String,
    pub colors: Vec<[f32; 4]>,
}

fn palette_strips(palettes: &HashMap<String, Palette>) -> Vec<PaletteStrip> {
    let mut ids: Vec<String> = palettes.keys().cloned().collect();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| {
            let palette = palettes.get(&id)?;
            let mut swatches = palette.swatches.clone();
            swatches.sort_by(|left, right| left.slot.cmp(&right.slot));
            Some(PaletteStrip {
                id,
                colors: swatches.into_iter().map(|swatch| swatch.rgba).collect(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ShowSnapshot {
    pub loaded: bool,
    pub auto_enabled: bool,
    pub running: bool,
    pub elapsed_ms: u64,
    pub scene: Option<String>,
    pub next_scene: Option<String>,
    pub palette: Option<String>,
    pub next_palette: Option<String>,
    pub next_scene_secs: Option<u32>,
    pub next_palette_secs: Option<u32>,
    pub palette_manual: bool,
    pub palettes: Vec<String>,
    /// One strip per palette, slots in name order, rgba 0–1.
    pub palette_colors: Vec<PaletteStrip>,
    /// `idle`, `running`, or `paused`.
    pub transport: String,
    pub blackout: bool,
    pub validation: String,
    pub grade_enabled: bool,
    pub grade_strength: f32,
    pub notice: String,
}

impl ShowSnapshot {
    /// Web reconnect baseline: FullState always carries a `show` object, so a
    /// browser refresh before any pack loads still renders "NO SHOW" instead
    /// of a missing field (plan p3-web-control).
    pub fn empty(blackout: bool) -> Self {
        Self {
            loaded: false,
            auto_enabled: false,
            running: false,
            elapsed_ms: 0,
            scene: None,
            next_scene: None,
            palette: None,
            next_palette: None,
            next_scene_secs: None,
            next_palette_secs: None,
            palette_manual: false,
            palettes: Vec::new(),
            palette_colors: Vec::new(),
            transport: "idle".into(),
            blackout,
            validation: "NO SHOW".into(),
            grade_enabled: false,
            grade_strength: 0.0,
            notice: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::show::definition::{PaletteCue, SceneCue, SceneTransition, TransitionKind};

    fn controller() -> ShowController {
        // Distinct palettes: "earth" starts at 0.0 gray, "deep-blue" ends at
        // 1.0 gray, so interpolation fractions are directly assertable.
        fn gray(id: &str, v: f32) -> Palette {
            let mut p = Palette::solid_test(id, id);
            for s in &mut p.swatches {
                s.rgba = [v, v, v, 1.0];
            }
            p
        }
        let def = ShowDefinition {
            schema_version: 1,
            id: "test".into(),
            name: "Test".into(),
            duration_secs: 10800,
            scene_track: vec![
                SceneCue {
                    at_secs: 0,
                    preset_id: "A".into(),
                    transition: SceneTransition {
                        kind: TransitionKind::Cut,
                        duration_ms: 0,
                    },
                },
                SceneCue {
                    at_secs: 1200,
                    preset_id: "B".into(),
                    transition: SceneTransition {
                        kind: TransitionKind::Dissolve,
                        duration_ms: 8000,
                    },
                },
            ],
            palette_track: vec![
                PaletteCue {
                    at_secs: 0,
                    palette_id: "earth".into(),
                    transition_ms: 0,
                },
                PaletteCue {
                    at_secs: 3500,
                    palette_id: "deep-blue".into(),
                    transition_ms: 60_000,
                },
            ],
        };
        let mut palettes = HashMap::new();
        palettes.insert("earth".into(), gray("earth", 0.0));
        palettes.insert("deep-blue".into(), gray("deep-blue", 1.0));
        ShowController::new(
            def,
            PathBuf::from("/tmp"),
            palettes,
            HashMap::new(),
            vec!["A".into(), "B".into()],
        )
    }

    #[test]
    fn start_is_cut_zero() {
        let mut c = controller();
        let t0 = Instant::now();
        let events = c.handle(t0, ShowCommand::Start);
        let scene = events.iter().find_map(|e| match e {
            ShowVisualEvent::Scene(s) => Some(s),
            _ => None,
        });
        let scene = scene.expect("scene");
        assert_eq!(scene.preset_id, "A");
        assert_eq!(scene.duration_ms, 0);
        assert!(scene.snap);
    }

    #[test]
    fn stop_auto_does_not_reset_clock() {
        let mut c = controller();
        let t0 = Instant::now();
        c.handle(t0, ShowCommand::Start);
        c.handle(t0 + Duration::from_secs(30), ShowCommand::StopAuto);
        assert!(!c.auto_enabled);
        assert_eq!(
            c.clock.elapsed_at(t0 + Duration::from_secs(30)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn both_tracks_run_independently_from_one_clock() {
        // Scene edge at 1200s and palette edge at 3500s fire independently:
        // each track advances on its own cue times off the same ShowClock.
        let mut c = controller();
        let t0 = Instant::now();
        c.handle(t0, ShowCommand::Start);
        assert_eq!(c.scene.active_preset_id.as_deref(), Some("A"));
        assert_eq!(c.palette.active_palette_id.as_deref(), Some("earth"));
        // Scene edge fires; palette untouched.
        let scene_events = c.tick(t0 + Duration::from_secs(1200));
        assert!(
            scene_events
                .iter()
                .any(|e| matches!(e, ShowVisualEvent::Scene(s) if s.preset_id == "B"))
        );
        assert_eq!(c.palette.active_palette_id.as_deref(), Some("earth"));
        // Palette edge fires; scene untouched.
        let pal_events = c.tick(t0 + Duration::from_secs(3500));
        assert!(
            pal_events
                .iter()
                .any(|e| matches!(e, ShowVisualEvent::Palette(_)))
        );
        assert_eq!(c.scene.active_preset_id.as_deref(), Some("B"));
        assert_eq!(c.palette.active_palette_id.as_deref(), Some("deep-blue"));
    }

    #[test]
    fn seek_atomically_reconstructs_scene_and_palette() {
        // SEEK into the palette transition (3522s = 37% of the 60s blend)
        // yields both the snapped scene AND the exact interpolated colors.
        let mut c = controller();
        let t0 = Instant::now();
        c.handle(t0, ShowCommand::Start);
        let events = c.handle(
            t0 + Duration::from_secs(10),
            ShowCommand::Seek { secs: 3522 },
        );
        let scene = events.iter().find_map(|e| match e {
            ShowVisualEvent::Scene(s) => Some(s),
            _ => None,
        });
        let scene = scene.expect("seek emits scene");
        assert_eq!(scene.preset_id, "B");
        assert!(scene.snap);
        let colors = events.iter().find_map(|e| match e {
            ShowVisualEvent::Palette(m) => Some(m),
            _ => None,
        });
        let colors = colors.expect("seek emits palette");
        match colors.get("color-1") {
            Some(ParamValue::Color(col)) => {
                let expected = 22.0 / 60.0;
                assert!((col[0] - expected).abs() < 0.03, "{col:?}");
            }
            other => panic!("{other:?}"),
        }
        // Schedulers agree with the seek position — no stale replay follows.
        assert_eq!(c.scene.last_processed_scheduled_index, Some(1));
        assert_eq!(c.palette.last_processed_scheduled_index, Some(1));
    }

    #[test]
    fn stall_jump_resolves_both_tracks_to_latest() {
        // One tick leaping over BOTH edges lands each track on its latest
        // cue — stale transitions are never replayed.
        let mut c = controller();
        let t0 = Instant::now();
        c.handle(t0, ShowCommand::Start);
        let events = c.tick(t0 + Duration::from_secs(7200));
        let scene = events.iter().find_map(|e| match e {
            ShowVisualEvent::Scene(s) => Some(s),
            _ => None,
        });
        assert_eq!(scene.map(|s| s.preset_id.as_str()), Some("B"));
        assert_eq!(c.scene.last_processed_scheduled_index, Some(1));
        assert_eq!(c.palette.last_processed_scheduled_index, Some(1));
    }

    #[test]
    fn manual_visual_override_does_not_disturb_palette_track() {
        // Manual NEXT changes the visual but neither scheduled index —
        // automation resumes normally on both tracks afterwards.
        let mut c = controller();
        let t0 = Instant::now();
        c.handle(t0, ShowCommand::Start);
        c.handle(t0 + Duration::from_secs(60), ShowCommand::NextVisual);
        assert_eq!(c.scene.active_preset_id.as_deref(), Some("B"));
        assert_eq!(
            c.clock.elapsed_at(t0 + Duration::from_secs(60)),
            Duration::from_secs(60)
        );
        assert_eq!(c.scene.last_processed_scheduled_index, Some(0));
        assert_eq!(c.palette.last_processed_scheduled_index, Some(0));
        // The 1200s scene edge is a no-op (already showing B) …
        assert!(
            c.tick(t0 + Duration::from_secs(1200))
                .iter()
                .all(|e| matches!(e, ShowVisualEvent::Palette(_)))
        );
        // … and the palette edge still fires on schedule.
        let pal_events = c.tick(t0 + Duration::from_secs(3500));
        assert!(
            pal_events
                .iter()
                .any(|e| matches!(e, ShowVisualEvent::Palette(_)))
        );
        assert_eq!(c.palette.active_palette_id.as_deref(), Some("deep-blue"));
    }

    #[test]
    fn manual_next_prev_follows_the_scene_track_and_stops_at_the_ends() {
        let mut c = controller();
        c.preset_ids.push("builtin-not-a-scene".into());
        let t0 = Instant::now();
        c.handle(t0, ShowCommand::Start);
        let clock = c.clock.elapsed_at(t0);
        assert!(c.handle(t0, ShowCommand::PrevVisual).is_empty());
        assert_eq!(c.scene.active_preset_id.as_deref(), Some("A"));
        c.handle(t0, ShowCommand::NextVisual);
        assert_eq!(c.scene.active_preset_id.as_deref(), Some("B"));
        assert!(c.handle(t0, ShowCommand::NextVisual).is_empty());
        assert_eq!(c.scene.active_preset_id.as_deref(), Some("B"));
        assert_ne!(
            c.scene.active_preset_id.as_deref(),
            Some("builtin-not-a-scene")
        );
        c.handle(t0, ShowCommand::PrevVisual);
        assert_eq!(c.scene.active_preset_id.as_deref(), Some("A"));
        assert_eq!(c.clock.elapsed_at(t0), clock);
    }

    #[test]
    fn manual_palette_override_does_not_move_palette_index() {
        let mut c = controller();
        let t0 = Instant::now();
        c.handle(t0, ShowCommand::Start);
        c.handle(t0 + Duration::from_secs(60), ShowCommand::NextPalette);
        assert_eq!(c.palette.active_palette_id.as_deref(), Some("deep-blue"));
        assert_eq!(c.palette.last_processed_scheduled_index, Some(0));
        // Automation resumes: the 3500s edge still fires on schedule.
        let events = c.tick(t0 + Duration::from_secs(3500));
        assert_eq!(c.palette.last_processed_scheduled_index, Some(1));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ShowVisualEvent::Palette(_)))
        );
    }

    #[test]
    fn manual_palette_holds_until_the_next_scheduled_cue() {
        let mut c = controller();
        let t0 = Instant::now();
        c.handle(t0, ShowCommand::Start);
        c.handle(
            t0 + Duration::from_secs(60),
            ShowCommand::SelectPalette {
                id: "deep-blue".into(),
            },
        );
        assert!(c.palette.manual);
        assert_eq!(c.palette.active_palette_id.as_deref(), Some("deep-blue"));
        let held = c.tick(t0 + Duration::from_secs(90));
        assert!(
            held.iter()
                .all(|e| !matches!(e, ShowVisualEvent::Palette(_))),
            "a manual palette must not be replaced on the next ticks"
        );
        assert_eq!(c.palette.active_palette_id.as_deref(), Some("deep-blue"));
        let events = c.handle(t0 + Duration::from_secs(91), ShowCommand::PaletteAuto);
        assert!(!c.palette.manual);
        assert_eq!(c.palette.active_palette_id.as_deref(), Some("earth"));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ShowVisualEvent::Palette(_)))
        );
    }

    #[test]
    fn relative_media_resolves_against_the_show_pack_not_the_working_directory() {
        let mut c = controller();
        c.pack_root = PathBuf::from("/show/hibernation");
        assert_eq!(
            c.resolve_path("assets/video/under-ice.mp4"),
            PathBuf::from("/show/hibernation/assets/video/under-ice.mp4")
        );
        assert_eq!(
            c.resolve_path("/Volumes/clips/plate.mp4"),
            PathBuf::from("/Volumes/clips/plate.mp4")
        );
    }
}
