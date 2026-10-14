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

/// Write `<pack>/presets/<id>.json`. The previous file is kept as `<id>.json.bak`.
pub fn write_scene_preset(pack_root: &Path, id: &str, preset: &Preset) -> Result<PathBuf> {
    let stem = scene_file_stem(id)?;
    let dir = pack_root.join("presets");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!("{stem}.json"));
    if path.is_file() {
        let bak = dir.join(format!("{stem}.json.bak"));
        fs::copy(&path, &bak).with_context(|| format!("keep previous {}", path.display()))?;
    }
    let json = serde_json::to_string_pretty(preset)?;
    write_atomic(&path, &json).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
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
        let bak: Preset = serde_json::from_str(
            &fs::read_to_string(root.join("presets/under-ice.json.bak")).unwrap(),
        )
        .unwrap();
        assert!((bak.layers[0].opacity - 0.4).abs() < 0.001);
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
