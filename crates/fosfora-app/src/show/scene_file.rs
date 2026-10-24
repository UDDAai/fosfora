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

/// Name for a scene that does not exist yet. Letters, numbers, spaces, and hyphens.
pub fn new_scene_name(name: &str) -> Result<String> {
    let stem = scene_file_stem(name)?;
    if stem.len() > 64
        || !stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_')
    {
        bail!("Use letters, numbers, spaces, or hyphens");
    }
    Ok(stem)
}

/// True when `presets/<id>.json` already exists, ignoring ASCII case.
pub fn scene_id_in_use(pack_root: &Path, id: &str) -> bool {
    let Ok(stem) = scene_file_stem(id) else {
        return true;
    };
    let Ok(entries) = fs::read_dir(pack_root.join("presets")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path.extension().and_then(|ext| ext.to_str()) == Some("json")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|existing| existing.eq_ignore_ascii_case(&stem))
    })
}

/// Copy palette and audio maps from the scene this stack came from.
/// The new scene keeps the same palette language and music modulation.
pub fn copy_scene_sidecars(pack_root: &Path, source_id: &str, new_id: &str) -> Result<()> {
    let source = scene_file_stem(source_id)?;
    let dest = scene_file_stem(new_id)?;
    if source.eq_ignore_ascii_case(&dest) {
        return Ok(());
    }

    let palette_src = pack_root
        .join("palette-bindings")
        .join(format!("{source}.json"));
    if palette_src.is_file() {
        let raw = fs::read_to_string(&palette_src)
            .with_context(|| format!("read {}", palette_src.display()))?;
        let mut value: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("parse {}", palette_src.display()))?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert("preset".into(), serde_json::Value::String(dest.clone()));
        }
        let dir = pack_root.join("palette-bindings");
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let json = serde_json::to_string_pretty(&value)?;
        write_atomic(&dir.join(format!("{dest}.json")), &json)?;
    }

    let audio_src = pack_root
        .join("audio-bindings")
        .join(format!("{source}.bindings.json"));
    if audio_src.is_file() {
        let dir = pack_root.join("audio-bindings");
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let audio_dest = dir.join(format!("{dest}.bindings.json"));
        fs::copy(&audio_src, &audio_dest)
            .with_context(|| format!("copy {}", audio_src.display()))?;
    }
    Ok(())
}

/// Store a path relative to the show pack when the file lives inside it.
pub fn portable_media_path(pack_root: &Path, stored: &str) -> String {
    let path = Path::new(stored);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        pack_root.join(path)
    };
    // Prefer a configured media-root alias even if the external folder is
    // physically nested under the pack (as in a test fixture or local worktree).
    // Otherwise a path such as "cloud library/video/clip.mp4" leaks into the
    // saved preset instead of its portable assets/video/clip.mp4 identity.
    if let Some(alias) = crate::media::library::virtual_media_path(pack_root, &absolute) {
        return alias;
    }
    if let Ok(rel) = absolute.strip_prefix(pack_root) {
        return rel.to_string_lossy().replace('\\', "/");
    }
    stored.replace('\\', "/")
}

pub fn relativize_preset(pack_root: &Path, preset: &mut Preset) {
    for layer in &mut preset.layers {
        // Particle source images/videos and obstacle image textures can also
        // live in the primary external media root. Preserve their aliases.
        for field in [
            &mut layer.media_path,
            &mut layer.particle_video_path,
            &mut layer.particle_image_path,
            &mut layer.obstacle_image_path,
        ] {
            if let Some(path) = field.as_mut() {
                *path = portable_media_path(pack_root, path);
            }
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
        let root =
            std::env::temp_dir().join(format!("fosfora_scene_{name}_{}", std::process::id()));
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
    fn external_primary_paths_become_pack_relative_without_copying() {
        let root = temp_pack("primary_external");
        let cloud = root.join("cloud library");
        fs::create_dir_all(cloud.join("video")).unwrap();
        fs::create_dir_all(cloud.join("images")).unwrap();
        let clip = cloud.join("video/clip.mp4");
        let still = cloud.join("images/plate.png");
        fs::write(&clip, b"frames").unwrap();
        fs::write(&still, b"still").unwrap();
        fs::write(
            root.join("media-sources.json"),
            serde_json::json!({"primary_media_root":cloud.to_string_lossy()}).to_string(),
        ).unwrap();
        let mut scene = preset(&clip.to_string_lossy(), 1.0);
        scene.layers[0].particle_image_path = Some(still.to_string_lossy().to_string());
        relativize_preset(&root, &mut scene);
        assert_eq!(scene.layers[0].media_path.as_deref(), Some("assets/video/clip.mp4"));
        assert_eq!(scene.layers[0].particle_image_path.as_deref(), Some("assets/images/plate.png"));
        write_scene_preset(&root, "test-scene", &scene).unwrap();
        assert!(clip.exists());
        assert!(still.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn external_primary_outside_the_pack_stays_portable() {
        let root = temp_pack("real_external");
        let cloud = std::env::temp_dir().join(format!(
            "fosfora_external_media_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&cloud);
        fs::create_dir_all(cloud.join("video")).unwrap();
        let clip = cloud.join("video/clip.mp4");
        fs::write(&clip, b"frames").unwrap();
        fs::write(
            root.join("media-sources.json"),
            serde_json::json!({"primary_media_root": cloud.to_string_lossy()}).to_string(),
        )
        .unwrap();

        assert_eq!(
            portable_media_path(&root, &clip.to_string_lossy()),
            "assets/video/clip.mp4"
        );
        assert_eq!(
            portable_media_path(&root, "assets/video/clip.mp4"),
            "assets/video/clip.mp4"
        );
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&cloud);
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
        upsert_named(&mut presets, "under-ice", preset("assets/video/c.mp4", 0.7));
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

    #[test]
    fn new_scene_keeps_palette_and_audio_maps_from_the_source() {
        let root = temp_pack("copy");
        fs::create_dir_all(root.join("palette-bindings")).unwrap();
        fs::create_dir_all(root.join("audio-bindings")).unwrap();
        fs::write(
            root.join("palette-bindings/thaw.json"),
            r#"{"schema_version":1,"preset":"thaw","bindings":[{"slot":"color-3","layer":2,"parameter":"tint"}]}"#,
        )
        .unwrap();
        fs::write(
            root.join("audio-bindings/thaw.bindings.json"),
            r#"{"version":1,"bindings":[]}"#,
        )
        .unwrap();
        fs::write(root.join("presets/thaw.json"), "{}").unwrap();

        assert!(scene_id_in_use(&root, "Thaw"));
        assert!(!scene_id_in_use(&root, "glass-room"));
        assert!(new_scene_name("glass-room").is_ok());
        assert!(new_scene_name("bad/name").is_err());

        copy_scene_sidecars(&root, "thaw", "glass-room").unwrap();
        let palette: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root.join("palette-bindings/glass-room.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(palette["preset"], "glass-room");
        assert_eq!(palette["bindings"][0]["parameter"], "tint");
        assert!(root
            .join("audio-bindings/glass-room.bindings.json")
            .is_file());
        assert_eq!(
            fs::read_to_string(root.join("palette-bindings/thaw.json")).unwrap(),
            r#"{"schema_version":1,"preset":"thaw","bindings":[{"slot":"color-3","layer":2,"parameter":"tint"}]}"#
        );
        let _ = fs::remove_dir_all(&root);
    }
}
