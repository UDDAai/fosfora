//! The open show pack is the only copy of a Hibernation scene.
//!
//! Writes go to `<pack>/presets/<scene-id>.json`. Nothing here writes a second
//! preset under Application Support.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::paths::write_atomic;
use crate::preset::Preset;

pub const WORKSPACE_FILE: &str = "workspace.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceMode {
    Prepare,
    Perform,
}

impl WorkspaceMode {
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("perform") {
            Self::Perform
        } else {
            Self::Prepare
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Perform => "perform",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    Saved,
    Unsaved,
    Saving,
    Failed(String),
}

impl SaveState {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Unsaved => "unsaved",
            Self::Saving => "saving",
            Self::Failed(_) => "failed",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Failed(message) => message.clone(),
            _ => String::new(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceFile {
    #[serde(default)]
    pub scene: Option<String>,
    #[serde(default = "default_prepare")]
    pub mode: String,
    #[serde(default = "default_grade_on")]
    pub grade_enabled: bool,
    #[serde(default = "default_grade_strength")]
    pub grade_strength: f32,
}

fn default_prepare() -> String {
    "prepare".into()
}

fn default_grade_on() -> bool {
    true
}

fn default_grade_strength() -> f32 {
    0.75
}

impl WorkspaceFile {
    pub fn load(pack_root: &Path) -> Self {
        let path = pack_root.join(WORKSPACE_FILE);
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| Self::fresh()),
            Err(_) => Self::fresh(),
        }
    }

    pub fn fresh() -> Self {
        Self {
            scene: None,
            mode: default_prepare(),
            grade_enabled: true,
            grade_strength: 0.75,
        }
    }

    pub fn store(&self, pack_root: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        write_atomic(&pack_root.join(WORKSPACE_FILE), &json)?;
        Ok(())
    }
}

/// Reject a scene id that is not a single file stem.
pub fn scene_file_stem(id: &str) -> Result<String> {
    let stem = id.trim();
    if stem.is_empty()
        || stem == "."
        || stem == ".."
        || stem.contains('/')
        || stem.contains('\\')
        || stem.contains("..")
    {
        bail!("scene id '{id}' is not a file name");
    }
    Ok(stem.to_string())
}

/// Store a path relative to the show pack when the file lives inside it.
pub fn portable_media_path(pack_root: &Path, stored: &str) -> String {
    let path = Path::new(stored);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        pack_root.join(path)
    };
    if let Ok(rel) = absolute.strip_prefix(pack_root) {
        return rel.to_string_lossy().replace('\\', "/");
    }
    stored.replace('\\', "/")
}

pub fn relativize_preset(pack_root: &Path, preset: &mut Preset) {
    for layer in &mut preset.layers {
        if let Some(path) = layer.media_path.as_mut() {
            *path = portable_media_path(pack_root, path);
        }
    }
}

/// Replace one preset in a list. Every other entry stays.
pub fn upsert_named(presets: &mut Vec<(String, Preset)>, id: &str, preset: Preset) {
    if let Some(slot) = presets.iter_mut().find(|(name, _)| name == id) {
        slot.1 = preset;
    } else {
        presets.push((id.to_string(), preset));
    }
}

/// How many distinct previous revisions to keep per scene.
const HISTORY_LIMIT: usize = 5;

/// Write `<pack>/presets/<id>.json`.
///
/// A changed file pushes the previous bytes onto `recovery/history/<id>/`,
/// newest first, capped at five distinct revisions. Identical content does
/// not rotate that ring and does not rewrite the preset. Files under
/// `recovery/baselines/` are never touched.
pub fn write_scene_preset(pack_root: &Path, id: &str, preset: &Preset) -> Result<PathBuf> {
    let stem = scene_file_stem(id)?;
    let dir = pack_root.join("presets");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!("{stem}.json"));
    let json = serde_json::to_string_pretty(preset)?;
    if path.is_file() {
        let existing = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        if existing == json {
            return Ok(path);
        }
        push_scene_history(pack_root, &stem, &existing)?;
    }
    write_atomic(&path, &json).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Park an unsaved Perform edit where a scheduled cue can continue without
/// waiting, and without replacing the production preset.
pub fn write_scene_draft(pack_root: &Path, id: &str, preset: &Preset) -> Result<PathBuf> {
    let stem = scene_file_stem(id)?;
    let dir = pack_root.join("recovery").join("drafts");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("{stem}-{stamp}.json"));
    let json = serde_json::to_string_pretty(preset)?;
    write_atomic(&path, &json).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn push_scene_history(pack_root: &Path, stem: &str, previous: &str) -> Result<()> {
    let dir = pack_root.join("recovery").join("history").join(stem);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let newest = dir.join("1.json");
    if newest.is_file() {
        if let Ok(text) = fs::read_to_string(&newest) {
            if text == previous {
                return Ok(());
            }
        }
    }
    for index in (1..HISTORY_LIMIT).rev() {
        let from = dir.join(format!("{index}.json"));
        let to = dir.join(format!("{}.json", index + 1));
        if from.is_file() {
            fs::rename(&from, &to).with_context(|| format!("rotate {}", from.display()))?;
        }
    }
    write_atomic(&newest, previous).with_context(|| format!("write {}", newest.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_pack(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "fosfora_scene_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("presets")).unwrap();
        fs::create_dir_all(root.join("assets/video")).unwrap();
        root
    }

    fn preset(path: &str, opacity: f32) -> Preset {
        let json = format!(
            r#"{{"layers":[{{"effect_name":"","opacity":{opacity},"media_path":"{path}","media_fit":"fill","media_width":1.3,"media_pos_x":0.2,"media_pos_y":-0.1,"media_speed":0.8,"media_looping":true}}]}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn pack_path_is_stored_relative_and_the_previous_file_is_kept() {
        let root = temp_pack("rel");
        let abs = root.join("assets/video/clip.mp4");
        fs::write(&abs, b"clip").unwrap();
        let mut first = preset(&abs.to_string_lossy().replace('\\', "/"), 0.4);
        relativize_preset(&root, &mut first);
        write_scene_preset(&root, "under-ice", &first).unwrap();

        let mut second = first.clone();
        second.layers[0].opacity = 0.9;
        write_scene_preset(&root, "under-ice", &second).unwrap();

        let saved: Preset =
            serde_json::from_str(&fs::read_to_string(root.join("presets/under-ice.json")).unwrap())
                .unwrap();
        assert_eq!(
            saved.layers[0].media_path.as_deref(),
            Some("assets/video/clip.mp4")
        );
        assert!((saved.layers[0].opacity - 0.9).abs() < 0.001);
        assert!((saved.layers[0].media_width - 1.3).abs() < 0.001);
        assert!(!root.join("presets/under-ice.json.bak").exists());
        let bak: Preset = serde_json::from_str(
            &fs::read_to_string(root.join("recovery/history/under-ice/1.json")).unwrap(),
        )
        .unwrap();
        assert!((bak.layers[0].opacity - 0.4).abs() < 0.001);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn history_keeps_five_distinct_revisions_and_skips_identical_writes() {
        let root = temp_pack("ring");
        let baseline = root.join("recovery/baselines/under-ice-original.json");
        fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        fs::write(&baseline, b"do-not-touch").unwrap();

        write_scene_preset(&root, "under-ice", &preset("assets/video/a.mp4", 0.1)).unwrap();
        for opacity in [0.2, 0.3, 0.4, 0.5, 0.6, 0.7] {
            write_scene_preset(&root, "under-ice", &preset("assets/video/a.mp4", opacity)).unwrap();
        }
        // The same bytes again must not walk a good copy off the ring.
        write_scene_preset(&root, "under-ice", &preset("assets/video/a.mp4", 0.7)).unwrap();

        let live: Preset =
            serde_json::from_str(&fs::read_to_string(root.join("presets/under-ice.json")).unwrap())
                .unwrap();
        assert!((live.layers[0].opacity - 0.7).abs() < 0.001);
        let history = root.join("recovery/history/under-ice");
        let oldest: Preset =
            serde_json::from_str(&fs::read_to_string(history.join("5.json")).unwrap()).unwrap();
        // Six changes after the first write: 0.2 through 0.6 sit in the ring,
        // 0.1 has rotated off, and the identical 0.7 write did not push 0.6 out.
        assert!((oldest.layers[0].opacity - 0.2).abs() < 0.001);
        let newest: Preset =
            serde_json::from_str(&fs::read_to_string(history.join("1.json")).unwrap()).unwrap();
        assert!((newest.layers[0].opacity - 0.6).abs() < 0.001);
        assert!(!history.join("6.json").exists());
        assert_eq!(fs::read(&baseline).unwrap(), b"do-not-touch");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn upsert_replaces_one_scene_and_leaves_the_others() {
        let root = temp_pack("upsert");
        let mut presets = vec![
            ("deep-sleep".into(), preset("assets/video/a.mp4", 1.0)),
            ("under-ice".into(), preset("assets/video/b.mp4", 0.2)),
        ];
        upsert_named(
            &mut presets,
            "under-ice",
            preset("assets/video/c.mp4", 0.7),
        );
        assert_eq!(presets.len(), 2);
        assert_eq!(presets[0].0, "deep-sleep");
        assert_eq!(
            presets[0].1.layers[0].media_path.as_deref(),
            Some("assets/video/a.mp4")
        );
        assert!((presets[1].1.layers[0].opacity - 0.7).abs() < 0.001);
        assert!(scene_file_stem("../nope").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
