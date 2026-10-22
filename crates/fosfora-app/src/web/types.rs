use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::midi::types::TriggerAction;

/// Inbound message from a WebSocket client.
#[derive(Debug, Clone)]
pub enum WsInMessage {
    /// Set param on active layer (normalized 0-1).
    SetParam { name: String, value: f32 },
    /// Set param on a specific layer (normalized 0-1).
    SetLayerParam {
        layer: usize,
        name: String,
        value: f32,
    },
    /// Load an effect by index on the active layer.
    LoadEffect { index: usize },
    /// Take the effect off one layer and leave the layer in the stack.
    ClearLayerEffect { index: usize },
    /// Store the live stack as a new named preset.
    SavePreset { name: String },
    /// Rename a scene file. Built-in presets stay as they are.
    RenamePreset { from: String, to: String },
    /// Delete a scene file. Built-in presets stay. A cue that names it leaves the show clock.
    DeletePreset { name: String },
    /// Rename a show-pack palette and the cues that name it.
    RenamePalette { from: String, to: String },
    /// Select the active layer. Does not change visibility.
    SelectLayer { index: usize },
    /// Show only this layer until Solo is cleared. Authored `enabled` flags stay.
    SetSolo { index: usize },
    /// Restore authored visibility.
    ClearSolo,
    /// Older clients sent this for "show all". It only clears Solo.
    ShowAllLayers,
    /// Move a layer. Index 0 is the front of the picture.
    MoveLayer { from: usize, to: usize },
    /// Remove one layer. The last remaining layer is kept.
    RemoveLayer { index: usize },
    /// Save, discard, or cancel a Perform scene change that is waiting.
    ResolvePending { action: String },
    /// Add a media layer from a file already written to disk.
    AddMedia { path: PathBuf },
    /// Files landed in the show library. Rescan; do not create layers.
    RescanLibrary,
    /// Copy Application Support uploads into the show library. Originals stay.
    ImportPreviousUploads,
    /// Open the pack media folder in Finder.
    RevealMediaFolder,
    /// Ask for a native folder dialog and remember that folder as a linked source.
    LinkMediaFolder,
    /// Register Finder paths in place. The files are not copied.
    LinkLibraryFiles { paths: Vec<String> },
    /// Remove one library entry. A linked original stays on disk.
    RemoveLibraryItem { id: String },
    /// Add library items as new layers, in this order. Nothing is added if the
    /// batch does not fit.
    AddLibraryMedia { ids: Vec<String> },
    /// Replace one layer's media. The rest of the stack stays.
    ReplaceLibraryMedia { layer: usize, id: String },
    /// Loop and speed for a media layer.
    SetMediaTransport {
        layer: usize,
        speed: Option<f32>,
        looping: Option<bool>,
    },
    /// Write the current layer stack into the open show pack.
    SaveScene,
    /// `prepare` autosaves. `perform` waits for an explicit save.
    SetWorkspaceMode { mode: String },
    /// Placement of a video or still. Width is a fraction of the canvas.
    SetMediaPlace {
        layer: usize,
        fit: String,
        width: f32,
        x: f32,
        y: f32,
    },
    /// Palette grade strength (0–1) and on/off. Either field may be omitted.
    SetGrade {
        strength: Option<f32>,
        enabled: Option<bool>,
    },
    /// Set layer opacity.
    SetLayerOpacity { layer: usize, value: f32 },
    /// Set layer blend mode (0-6).
    SetLayerBlend { layer: usize, value: u32 },
    /// Set layer enabled.
    SetLayerEnabled { layer: usize, value: bool },
    /// Fire a trigger action.
    Trigger(TriggerAction),
    /// Load a preset by index.
    LoadPreset { index: usize },
    /// Toggle post-processing.
    PostProcessEnabled(bool),
    /// Binding data: source name + field values.
    BindData {
        source: String,
        fields: Vec<(String, f32)>,
    },
    /// Binding schema: source name + field metadata (for auto-discovery).
    #[allow(dead_code)]
    BindSchema {
        source: String,
        fields: Vec<(String, SourceFieldInfo)>,
    },
    /// Preview thumbnail image (JPEG) from a bridge source.
    BindPreview { source: String, jpeg_data: Vec<u8> },
    /// Show orchestration command.
    ShowCommand(crate::show::controller::ShowCommand),
}

/// Metadata for a WebSocket source field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFieldInfo {
    #[serde(default)]
    pub min: f32,
    #[serde(default = "default_one")]
    pub max: f32,
    #[serde(default)]
    pub label: String,
}

fn default_one() -> f32 {
    1.0
}

/// A Solo button press. `Clear` restores the authored visibility mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoloRequest {
    Set(usize),
    Clear,
}

/// Result of WebSystem::update() — mirrors OscFrameResult with extras.
pub struct WebFrameResult {
    pub triggers: Vec<TriggerAction>,
    pub layer_params: Vec<(usize, String, f32)>,
    pub layer_opacity: Vec<(usize, f32)>,
    pub layer_blend: Vec<(usize, u32)>,
    pub layer_enabled: Vec<(usize, bool)>,
    pub postprocess_enabled: Option<bool>,
    pub effect_loads: Vec<usize>,
    pub clear_effects: Vec<usize>,
    pub save_preset_name: Option<String>,
    pub rename_preset: Option<(String, String)>,
    pub delete_preset: Option<String>,
    pub rename_palette: Option<(String, String)>,
    pub select_layer: Option<usize>,
    pub solo: Option<SoloRequest>,
    pub layer_moves: Vec<(usize, usize)>,
    pub layer_removes: Vec<usize>,
    pub resolve_pending: Option<String>,
    pub add_media: Vec<PathBuf>,
    pub rescan_library: bool,
    pub import_uploads: bool,
    pub reveal_media: bool,
    pub link_folder: bool,
    pub link_library_files: Vec<Vec<String>>,
    pub remove_library: Vec<String>,
    pub add_library: Vec<Vec<String>>,
    pub replace_library: Vec<(usize, String)>,
    pub media_transport: Vec<(usize, Option<f32>, Option<bool>)>,
    pub save_scene: bool,
    pub workspace_mode: Option<String>,
    pub touched_params: bool,
    pub media_place: Vec<(usize, String, f32, f32, f32)>,
    pub grade_strength: Option<f32>,
    pub grade_enabled: Option<bool>,
    pub preset_loads: Vec<usize>,
    pub show_commands: Vec<crate::show::controller::ShowCommand>,
}

impl WebFrameResult {
    pub fn empty() -> Self {
        Self {
            triggers: Vec::new(),
            layer_params: Vec::new(),
            layer_opacity: Vec::new(),
            layer_blend: Vec::new(),
            layer_enabled: Vec::new(),
            postprocess_enabled: None,
            effect_loads: Vec::new(),
            clear_effects: Vec::new(),
            save_preset_name: None,
            rename_preset: None,
            delete_preset: None,
            rename_palette: None,
            select_layer: None,
            solo: None,
            layer_moves: Vec::new(),
            layer_removes: Vec::new(),
            resolve_pending: None,
            add_media: Vec::new(),
            rescan_library: false,
            import_uploads: false,
            reveal_media: false,
            link_folder: false,
            link_library_files: Vec::new(),
            remove_library: Vec::new(),
            add_library: Vec::new(),
            replace_library: Vec::new(),
            media_transport: Vec::new(),
            save_scene: false,
            workspace_mode: None,
            touched_params: false,
            media_place: Vec::new(),
            grade_strength: None,
            grade_enabled: None,
            preset_loads: Vec::new(),
            show_commands: Vec::new(),
        }
    }
}

/// Persisted WebSocket server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_true() -> bool {
    true
}
fn default_port() -> u16 {
    9002
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 9002,
        }
    }
}

impl WebConfig {
    pub fn config_path() -> PathBuf {
        crate::paths::config_root().join("web.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(config) => {
                    log::info!("Loaded web config from {}", path.display());
                    config
                }
                Err(e) => {
                    log::warn!("Failed to parse web config: {e}");
                    Self::default()
                }
            },
            Err(_) => {
                log::info!("No web config found, using defaults");
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::error!("Failed to create config dir: {e}");
                return;
            }
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    log::error!("Failed to write web config: {e}");
                } else {
                    log::debug!("Saved web config to {}", path.display());
                }
            }
            Err(e) => log::error!("Failed to serialize web config: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_config_defaults() {
        let c = WebConfig::default();
        assert!(c.enabled);
        assert_eq!(c.port, 9002);
    }

    #[test]
    fn web_config_serde_roundtrip() {
        let c = WebConfig::default();
        let json = serde_json::to_string(&c).unwrap();
        let c2: WebConfig = serde_json::from_str(&json).unwrap();
        assert!(c2.enabled);
        assert_eq!(c2.port, 9002);
    }

    #[test]
    fn web_frame_result_empty() {
        let r = WebFrameResult::empty();
        assert!(r.triggers.is_empty());
        assert!(r.layer_params.is_empty());
        assert!(r.layer_opacity.is_empty());
        assert!(r.layer_blend.is_empty());
        assert!(r.layer_enabled.is_empty());
        assert!(r.postprocess_enabled.is_none());
        assert!(r.effect_loads.is_empty());
        assert!(r.select_layer.is_none());
        assert!(r.preset_loads.is_empty());
    }

    // ---- Additional tests ----

    #[test]
    fn web_config_disabled_roundtrip() {
        let c = WebConfig {
            enabled: false,
            port: 8080,
        };
        let json = serde_json::to_string(&c).unwrap();
        let c2: WebConfig = serde_json::from_str(&json).unwrap();
        assert!(!c2.enabled);
        assert_eq!(c2.port, 8080);
    }

    #[test]
    fn web_config_partial_json_defaults() {
        let json = r#"{"port": 3000}"#;
        let c: WebConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.port, 3000);
        assert!(c.enabled); // default true
    }
}
