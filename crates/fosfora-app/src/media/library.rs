//! Show-pack media library.
//!
//! The files in `assets/video` and `assets/images` are the library. A JSON
//! cache stores probe results and thumbnail names so browsing does not decode
//! clips into RAM. Linked folders are optional and are marked as external.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::show::validation::estimate_rgba_ram;

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "avi", "mkv", "webm", "m4v", "flv"];

pub const MAX_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_LIBRARY_BYTES: u64 = 20 * 1024 * 1024 * 1024;
pub const MAX_DECODED_RAM: u64 = 6 * 1024 * 1024 * 1024;
pub const MAX_CLIP_RAM: u64 = 2 * 1024 * 1024 * 1024;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LibraryItem {
    pub id: String,
    pub name: String,
    /// Pack-relative for portable files, absolute for a linked folder.
    pub path: String,
    pub kind: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_secs: f64,
    pub fingerprint: String,
    /// `pack` or `linked`.
    pub location: String,
    /// Folder this linked file came from. Empty for pack files.
    #[serde(default)]
    pub source: String,
    /// `ok`, `missing`, `unsupported`, or `too_large`.
    pub status: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub scenes: Vec<String>,
    /// File name inside `assets/.thumbs`, when a preview exists.
    #[serde(default)]
    pub thumb: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ClipPlan {
    pub name: String,
    pub ram_bytes: u64,
    pub problem: Option<String>,
}

#[derive(Clone, Debug)]
pub struct UploadReport {
    pub id: String,
    pub name: String,
    pub duplicate: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct ProbeCache {
    entries: Vec<CachedProbe>,
}

#[derive(Clone, Serialize, Deserialize)]
struct CachedProbe {
    path: String,
    bytes: u64,
    modified_ms: u64,
    fingerprint: String,
    kind: String,
    width: u32,
    height: u32,
    fps: f64,
    duration_secs: f64,
    status: String,
    detail: String,
}

#[derive(Default, Serialize, Deserialize)]
struct SourceFile {
    #[serde(default)]
    linked_folders: Vec<String>,
}

struct SeenFile {
    path: PathBuf,
    id: String,
    location: String,
    source: String,
}

pub struct MediaLibrary {
    pub pack_root: PathBuf,
    pub items: Vec<LibraryItem>,
    pub warnings: Vec<String>,
    linked_folders: Vec<PathBuf>,
    watch: Option<LibraryWatch>,
}

struct LibraryWatch {
    _debouncer: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
    rx: std::sync::mpsc::Receiver<notify_debouncer_mini::DebounceEventResult>,
}

impl MediaLibrary {
    pub fn open(pack_root: PathBuf) -> Self {
        let _ = fs::create_dir_all(pack_root.join("assets/video"));
        let _ = fs::create_dir_all(pack_root.join("assets/images"));
        let _ = fs::create_dir_all(pack_root.join("assets/.thumbs"));
        let _ = fs::create_dir_all(pack_root.join("assets/.incoming"));
        let linked_folders = load_sources(&pack_root);
        let watch = start_watch(&pack_root, &linked_folders);
        let mut lib = Self {
            pack_root,
            items: Vec::new(),
            warnings: Vec::new(),
            linked_folders,
            watch,
        };
        lib.rescan();
        lib
    }

    pub fn poll_watch(&mut self) -> bool {
        let Some(watch) = &self.watch else {
            return false;
        };
        let mut dirty = false;
        while watch.rx.try_recv().is_ok() {
            dirty = true;
        }
        dirty
    }

    pub fn rescan(&mut self) {
        let previous = fs::read_to_string(library_json_path(&self.pack_root))
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<LibraryItem>>(&text).ok())
            .unwrap_or_default();
        let cache = load_cache(&self.pack_root);
        let scenes = scenes_using_media(&self.pack_root);
        let mut warnings = Vec::new();
        let mut items = Vec::new();
        let mut next_cache = ProbeCache { entries: Vec::new() };

        for seen in pack_files(&self.pack_root) {
            if let Some(item) = describe_file(&seen, &cache, &mut next_cache, &self.pack_root) {
                items.push(item);
            }
        }

        for folder in &self.linked_folders {
            if !folder.is_dir() {
                warnings.push(format!(
                    "Linked media folder is missing: {}",
                    folder.display()
                ));
                let source = folder.to_string_lossy().to_string();
                for old in previous.iter().filter(|item| item.source == source) {
                    let mut kept = old.clone();
                    kept.status = "missing".into();
                    kept.detail = "Linked folder is not available".into();
                    if !items.iter().any(|item| item.id == kept.id) {
                        items.push(kept);
                    }
                }
                continue;
            }
            for seen in folder_files(folder) {
                if let Some(item) = describe_file(&seen, &cache, &mut next_cache, &self.pack_root)
                {
                    items.push(item);
                }
            }
        }

        for item in &mut items {
            item.scenes = scenes_for(item, &scenes);
        }
        items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let mut seen_ids = std::collections::HashSet::new();
        items.retain(|item| seen_ids.insert(item.id.clone()));

        let _ = write_json(&cache_path(&self.pack_root), &next_cache);
        let _ = write_json(&library_json_path(&self.pack_root), &items);
        self.items = items;
        self.warnings = warnings;
    }

    pub fn link_folder(&mut self, folder: PathBuf) {
        if !self.linked_folders.iter().any(|have| have == &folder) {
            self.linked_folders.push(folder);
        }
        let file = SourceFile {
            linked_folders: self
                .linked_folders
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
        };
        let _ = write_json(&self.pack_root.join("media-sources.json"), &file);
        if let Some(watch) = start_watch(&self.pack_root, &self.linked_folders) {
            self.watch = Some(watch);
        }
        self.rescan();
    }

    pub fn reveal_in_finder(&self) -> Result<(), String> {
        let dir = self.pack_root.join("assets");
        let _ = fs::create_dir_all(&dir);
        open_folder(&dir)
    }

    pub fn import_uploads(&mut self, uploads: &Path) -> Result<usize, String> {
        if !uploads.is_dir() {
            return Ok(0);
        }
        let mut copied = 0;
        for entry in fs::read_dir(uploads).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if media_kind(name).is_none() {
                continue;
            }
            match copy_into_library(&self.pack_root, &path, name) {
                Ok(_) => copied += 1,
                Err(err) => log::warn!("import {}: {err}", path.display()),
            }
        }
        self.rescan();
        Ok(copied)
    }

    pub fn resolve(&self, id: &str) -> Option<PathBuf> {
        if let Some(item) = self.items.iter().find(|item| item.id == id) {
            return item_file(&self.pack_root, item);
        }
        let direct = self.pack_root.join(id);
        direct.is_file().then_some(direct)
    }
}

pub fn library_json_path(pack_root: &Path) -> PathBuf {
    pack_root.join("assets/.library.json")
}

pub fn thumb_path(pack_root: &Path, thumb: &str) -> Option<PathBuf> {
    let name = Path::new(thumb).file_name()?.to_str()?;
    if name != thumb || !name.ends_with(".jpg") {
        return None;
    }
    let path = pack_root.join("assets/.thumbs").join(name);
    path.is_file().then_some(path)
}

pub fn media_kind(name: &str) -> Option<&'static str> {
    let ext = extension(name)?;
    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some("video")
    } else if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        Some("image")
    } else {
        None
    }
}

pub fn safe_media_name(raw: &str) -> Option<String> {
    let name = raw.split(['/', '\\']).next_back()?.trim();
    if name.is_empty() || name == "." || name == ".." || media_kind(name).is_none() {
        return None;
    }
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.starts_with('.') || media_kind(&cleaned).is_none() {
        None
    } else {
        Some(cleaned)
    }
}

/// Move a finished temp file into the library, or drop it when the same bytes
/// are already stored. The original upload path is never the public file.
pub fn commit_upload(pack_root: &Path, partial: &Path, original_name: &str) -> Result<UploadReport, String> {
    let name = safe_media_name(original_name).ok_or_else(|| "unsupported file name".to_string())?;
    let kind = media_kind(&name).ok_or_else(|| "unsupported file type".to_string())?;
    let meta = fs::metadata(partial).map_err(|e| e.to_string())?;
    if meta.len() == 0 {
        let _ = fs::remove_file(partial);
        return Err("empty file".into());
    }
    if meta.len() > MAX_UPLOAD_BYTES {
        let _ = fs::remove_file(partial);
        return Err("file is over 512 MB".into());
    }
    let fingerprint = file_fingerprint(partial).map_err(|e| e.to_string())?;
    if let Some(existing) = find_fingerprint(pack_root, &fingerprint) {
        let _ = fs::remove_file(partial);
        return Ok(UploadReport {
            id: existing.0,
            name: existing.1,
            duplicate: true,
        });
    }
    let used = directory_bytes(&pack_root.join("assets/video"))
        + directory_bytes(&pack_root.join("assets/images"));
    if used.saturating_add(meta.len()) > MAX_LIBRARY_BYTES {
        let _ = fs::remove_file(partial);
        return Err("the show media folder is over 20 GB".into());
    }
    let dir = pack_root.join(if kind == "video" {
        "assets/video"
    } else {
        "assets/images"
    });
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = unique_dest(&dir, &name);
    fs::rename(partial, &dest).map_err(|e| e.to_string())?;
    let rel = dest
        .strip_prefix(pack_root)
        .unwrap_or(&dest)
        .to_string_lossy()
        .replace('\\', "/");
    Ok(UploadReport {
        id: rel,
        name: dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&name)
            .to_string(),
        duplicate: false,
    })
}

pub fn copy_into_library(pack_root: &Path, src: &Path, original_name: &str) -> Result<UploadReport, String> {
    let incoming = pack_root.join("assets/.incoming");
    fs::create_dir_all(&incoming).map_err(|e| e.to_string())?;
    let partial = incoming.join(format!("copy-{}", incoming_token()));
    fs::copy(src, &partial).map_err(|e| e.to_string())?;
    commit_upload(pack_root, &partial, original_name)
}

pub fn plan_clip(path: &Path) -> ClipPlan {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("clip")
        .to_string();
    let Some(kind) = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(media_kind)
    else {
        return ClipPlan {
            name,
            ram_bytes: 0,
            problem: Some("unsupported file type".into()),
        };
    };
    if kind == "video" {
        match probe_video_meta(path) {
            Ok(meta) => {
                let ram = estimate_rgba_ram(meta.width, meta.height, meta.fps, meta.duration_secs);
                let mut problem = None;
                if meta.duration_secs > max_predecode_secs() {
                    problem = Some(format!(
                        "{:.0}s is over the {:.0}s decode limit",
                        meta.duration_secs,
                        max_predecode_secs()
                    ));
                } else if ram > MAX_CLIP_RAM {
                    problem = Some(format!(
                        "would decode to {}, over {} per clip",
                        format_bytes(ram),
                        format_bytes(MAX_CLIP_RAM)
                    ));
                }
                ClipPlan {
                    name,
                    ram_bytes: ram,
                    problem,
                }
            }
            Err(err) => ClipPlan {
                name,
                ram_bytes: 0,
                problem: Some(err),
            },
        }
    } else {
        match image::image_dimensions(path) {
            Ok((width, height)) => {
                let ram = u64::from(width) * u64::from(height) * 4;
                ClipPlan {
                    name,
                    ram_bytes: ram,
                    problem: (ram > MAX_CLIP_RAM).then(|| {
                        format!(
                            "would decode to {}, over {} per clip",
                            format_bytes(ram),
                            format_bytes(MAX_CLIP_RAM)
                        )
                    }),
                }
            }
            Err(err) => ClipPlan {
                name,
                ram_bytes: 0,
                problem: Some(err.to_string()),
            },
        }
    }
}

/// `None` when every clip can be decoded. Otherwise a message and nothing
/// should be loaded.
pub fn admission_error(
    existing_layers: usize,
    existing_ram: u64,
    max_layers: usize,
    clips: &[ClipPlan],
) -> Option<String> {
    let mut problems = Vec::new();
    let free = max_layers.saturating_sub(existing_layers);
    if clips.len() > free {
        problems.push(format!(
            "Need {} free layer slots, {free} left ({max_layers} max).",
            clips.len()
        ));
    }
    for clip in clips {
        if let Some(problem) = &clip.problem {
            problems.push(format!("{}: {problem}", clip.name));
        }
    }
    let added: u64 = clips.iter().map(|clip| clip.ram_bytes).sum();
    let total = existing_ram.saturating_add(added);
    if total > MAX_DECODED_RAM {
        problems.push(format!(
            "Decoded media would be {}, over the {} limit.",
            format_bytes(total),
            format_bytes(MAX_DECODED_RAM)
        ));
    }
    if problems.is_empty() {
        None
    } else {
        Some(format!(
            "Nothing was added.\n{}\nChoose fewer or shorter clips.",
            problems.join("\n")
        ))
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{bytes} B")
    }
}

pub fn file_fingerprint(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut head = [0u8; 65536];
    let head_n = file.read(&mut head)?;
    let mut tail = [0u8; 65536];
    let tail_n = if len > 65536 {
        file.seek(SeekFrom::End(-65536))?;
        file.read(&mut tail)?
    } else {
        0
    };
    let mut hash = 0xcbf29ce484222325u64;
    hash = fnv(&len.to_le_bytes(), hash);
    hash = fnv(&head[..head_n], hash);
    hash = fnv(&tail[..tail_n], hash);
    Ok(format!("{hash:016x}"))
}

fn fnv(bytes: &[u8], mut hash: u64) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn describe_file(
    seen: &SeenFile,
    cache: &ProbeCache,
    next_cache: &mut ProbeCache,
    pack_root: &Path,
) -> Option<LibraryItem> {
    let name = seen.path.file_name()?.to_str()?.to_string();
    if name.starts_with('.') {
        return None;
    }
    let kind = media_kind(&name).unwrap_or("file");
    let meta = fs::metadata(&seen.path).ok()?;
    let modified_ms = modified_ms(&meta);
    let cached = cache.entries.iter().find(|entry| {
        entry.path == seen.id && entry.bytes == meta.len() && entry.modified_ms == modified_ms
    });
    let probe = if let Some(hit) = cached {
        hit.clone()
    } else {
        let probed = probe_uncached(&seen.path, kind);
        let fingerprint = file_fingerprint(&seen.path).unwrap_or_default();
        let entry = CachedProbe {
            path: seen.id.clone(),
            bytes: meta.len(),
            modified_ms,
            fingerprint,
            kind: kind.to_string(),
            width: probed.width,
            height: probed.height,
            fps: probed.fps,
            duration_secs: probed.duration_secs,
            status: probed.status,
            detail: probed.detail,
        };
        entry
    };
    next_cache.entries.push(probe.clone());
    let thumb = if probe.status == "ok" || probe.status == "too_large" {
        ensure_thumb(pack_root, &seen.path, &probe.fingerprint, kind)
    } else {
        None
    };
    Some(LibraryItem {
        id: seen.id.clone(),
        name,
        path: if seen.location == "pack" {
            seen.path
                .strip_prefix(pack_root)
                .unwrap_or(&seen.path)
                .to_string_lossy()
                .replace('\\', "/")
        } else {
            seen.path.to_string_lossy().to_string()
        },
        kind: probe.kind,
        bytes: meta.len(),
        width: probe.width,
        height: probe.height,
        fps: probe.fps,
        duration_secs: probe.duration_secs,
        fingerprint: probe.fingerprint,
        location: seen.location.clone(),
        source: seen.source.clone(),
        status: probe.status,
        detail: probe.detail,
        scenes: Vec::new(),
        thumb,
    })
}

struct Probed {
    width: u32,
    height: u32,
    fps: f64,
    duration_secs: f64,
    status: String,
    detail: String,
}

fn probe_uncached(path: &Path, kind: &str) -> Probed {
    if kind == "file" {
        return Probed {
            width: 0,
            height: 0,
            fps: 0.0,
            duration_secs: 0.0,
            status: "unsupported".into(),
            detail: "this file type cannot be a layer".into(),
        };
    }
    if kind == "video" {
        match probe_video_meta(path) {
            Ok(meta) => {
                let ram = estimate_rgba_ram(meta.width, meta.height, meta.fps, meta.duration_secs);
                let too_long = meta.duration_secs > max_predecode_secs();
                let too_big = ram > MAX_CLIP_RAM;
                let (status, detail) = if too_long {
                    (
                        "too_large",
                        format!(
                        "{:.0}s is over the {:.0}s decode limit",
                        meta.duration_secs,
                        max_predecode_secs()
                    ),
                    )
                } else if too_big {
                    (
                        "too_large",
                        format!("would decode to {}", format_bytes(ram)),
                    )
                } else {
                    ("ok".into(), String::new())
                };
                Probed {
                    width: meta.width,
                    height: meta.height,
                    fps: meta.fps,
                    duration_secs: meta.duration_secs,
                    status: status.into(),
                    detail,
                }
            }
            Err(err) => Probed {
                width: 0,
                height: 0,
                fps: 0.0,
                duration_secs: 0.0,
                status: "unsupported".into(),
                detail: err,
            },
        }
    } else {
        match image::image_dimensions(path) {
            Ok((width, height)) => {
                let ram = u64::from(width) * u64::from(height) * 4;
                if ram > MAX_CLIP_RAM {
                    Probed {
                        width,
                        height,
                        fps: 0.0,
                        duration_secs: 0.0,
                        status: "too_large".into(),
                        detail: format!("would decode to {}", format_bytes(ram)),
                    }
                } else {
                    Probed {
                        width,
                        height,
                        fps: 0.0,
                        duration_secs: 0.0,
                        status: "ok".into(),
                        detail: String::new(),
                    }
                }
            }
            Err(err) => Probed {
                width: 0,
                height: 0,
                fps: 0.0,
                duration_secs: 0.0,
                status: "unsupported".into(),
                detail: err.to_string(),
            },
        }
    }
}

struct VideoProbe {
    width: u32,
    height: u32,
    fps: f64,
    duration_secs: f64,
}

fn max_predecode_secs() -> f64 {
    #[cfg(feature = "video")]
    {
        super::video::MAX_PREDECODE_SECS
    }
    #[cfg(not(feature = "video"))]
    {
        60.0
    }
}

fn probe_video_meta(path: &Path) -> Result<VideoProbe, String> {
    #[cfg(feature = "video")]
    {
        let meta = super::video::probe_video(path)?;
        Ok(VideoProbe {
            width: meta.width,
            height: meta.height,
            fps: meta.fps,
            duration_secs: meta.duration_secs,
        })
    }
    #[cfg(not(feature = "video"))]
    {
        let _ = path;
        Err("video decoding is not enabled".into())
    }
}

fn ensure_thumb(pack_root: &Path, src: &Path, fingerprint: &str, kind: &str) -> Option<String> {
    if fingerprint.is_empty() {
        return None;
    }
    let name = format!("{fingerprint}.jpg");
    let dest = pack_root.join("assets/.thumbs").join(&name);
    if dest.is_file() {
        return Some(name);
    }
    let _ = fs::create_dir_all(dest.parent()?);
    let ok = if kind == "video" {
        write_video_thumb(src, &dest)
    } else {
        write_image_thumb(src, &dest)
    };
    if ok && dest.is_file() { Some(name) } else { None }
}

fn write_video_thumb(src: &Path, dest: &Path) -> bool {
    let tool = super::toolchain::VideoToolchain::global(None);
    let Some(mut cmd) = tool.ffmpeg_cmd() else {
        return false;
    };
    cmd.args(["-y", "-ss", "0.2", "-i"])
        .arg(src)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=160:-2",
            "-q:v",
            "8",
        ])
        .arg(dest)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn write_image_thumb(src: &Path, dest: &Path) -> bool {
    let Ok(img) = image::open(src) else {
        return false;
    };
    let thumb = img.thumbnail(160, 90);
    thumb.save(dest).is_ok()
}

fn pack_files(pack_root: &Path) -> Vec<SeenFile> {
    let mut out = Vec::new();
    for (dir, kind_dir) in [("assets/video", "video"), ("assets/images", "images")] {
        let _ = kind_dir;
        let base = pack_root.join(dir);
        let Ok(entries) = fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let rel = path
                .strip_prefix(pack_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(SeenFile {
                path,
                id: rel,
                location: "pack".into(),
                source: String::new(),
            });
        }
    }
    out
}

fn folder_files(folder: &Path) -> Vec<SeenFile> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(folder) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if media_kind(name).is_none() {
            continue;
        }
        let source = folder.to_string_lossy().to_string();
        out.push(SeenFile {
            id: format!("linked:{source}:{name}"),
            path,
            location: "linked".into(),
            source,
        });
    }
    out
}

fn scenes_using_media(pack_root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(pack_root.join("presets")) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(layers) = value.get("layers").and_then(|v| v.as_array()) else {
            continue;
        };
        for layer in layers {
            if let Some(media) = layer.get("media_path").and_then(|v| v.as_str()) {
                out.push((id.to_string(), media.replace('\\', "/")));
            }
        }
    }
    out
}

fn scenes_for(item: &LibraryItem, scenes: &[(String, String)]) -> Vec<String> {
    let mut used = Vec::new();
    for (scene, media) in scenes {
        let matches = media == &item.path || media == &item.id || Path::new(media).file_name() == Path::new(&item.name).file_name();
        if matches && !used.iter().any(|have: &String| have == scene) {
            used.push(scene.clone());
        }
    }
    used.sort();
    used
}

fn item_file(pack_root: &Path, item: &LibraryItem) -> Option<PathBuf> {
    if item.status == "missing" {
        return None;
    }
    let path = if item.location == "linked" {
        PathBuf::from(&item.path)
    } else {
        pack_root.join(&item.path)
    };
    path.is_file().then_some(path)
}

fn find_fingerprint(pack_root: &Path, fingerprint: &str) -> Option<(String, String)> {
    for seen in pack_files(pack_root) {
        if file_fingerprint(&seen.path).ok().as_deref() == Some(fingerprint) {
            let name = seen
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            return Some((seen.id, name));
        }
    }
    None
}

fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let direct = dir.join(name);
    if !direct.exists() {
        return direct;
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("bin");
    for n in 2..1000 {
        let candidate = dir.join(format!("{stem}-{n}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.{}", incoming_token(), ext))
}

pub fn incoming_token() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms:x}-{n}")
}

fn directory_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

fn modified_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn cache_path(pack_root: &Path) -> PathBuf {
    pack_root.join("assets/.library-cache.json")
}

fn load_cache(pack_root: &Path) -> ProbeCache {
    fs::read_to_string(cache_path(pack_root))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn load_sources(pack_root: &Path) -> Vec<PathBuf> {
    fs::read_to_string(pack_root.join("media-sources.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<SourceFile>(&text).ok())
        .map(|file| file.linked_folders.into_iter().map(PathBuf::from).collect())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    crate::paths::write_atomic(path, &json).map_err(|e| e.to_string())
}

fn start_watch(pack_root: &Path, linked: &[PathBuf]) -> Option<LibraryWatch> {
    use notify::RecursiveMode;
    use notify_debouncer_mini::new_debouncer;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(800), tx).ok()?;
    for dir in [
        pack_root.join("assets/video"),
        pack_root.join("assets/images"),
    ] {
        if dir.is_dir() {
            let _ = debouncer.watcher().watch(&dir, RecursiveMode::NonRecursive);
        }
    }
    for dir in linked {
        if dir.is_dir() {
            let _ = debouncer.watcher().watch(dir, RecursiveMode::NonRecursive);
        }
    }
    Some(LibraryWatch {
        _debouncer: debouncer,
        rx,
    })
}

fn open_folder(path: &Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer")
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(path)
        .status()
        .map_err(|e| e.to_string())
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!("could not open {}", path.display()))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_pack(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("fosfora_lib_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("assets/images")).unwrap();
        fs::create_dir_all(root.join("assets/video")).unwrap();
        fs::create_dir_all(root.join("presets")).unwrap();
        root
    }

    #[test]
    fn a_still_in_the_pack_shows_up_once_and_again() {
        let root = temp_pack("scan");
        let image = image::RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 255]));
        image
            .save(root.join("assets/images/plate.png"))
            .unwrap();
        fs::write(
            root.join("presets/under-ice.json"),
            r#"{"layers":[{"effect_name":"","media_path":"assets/images/plate.png"}]}"#,
        )
        .unwrap();
        fs::write(root.join("assets/video/notes.txt"), b"nope").unwrap();
        let mut lib = MediaLibrary::open(root.clone());
        assert_eq!(lib.items.len(), 2);
        let plate = lib
            .items
            .iter()
            .find(|item| item.name == "plate.png")
            .unwrap();
        assert_eq!(plate.kind, "image");
        assert_eq!(plate.status, "ok");
        assert_eq!(plate.location, "pack");
        assert_eq!(plate.width, 8);
        assert_eq!(plate.scenes, vec!["under-ice".to_string()]);
        assert!(plate.thumb.is_some());
        let notes = lib.items.iter().find(|item| item.name == "notes.txt").unwrap();
        assert_eq!(notes.status, "unsupported");
        lib.rescan();
        assert_eq!(lib.items.len(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_same_bytes_are_not_copied_twice_and_originals_stay() {
        let root = temp_pack("dup");
        let uploads = root.join("uploads");
        fs::create_dir_all(&uploads).unwrap();
        fs::write(uploads.join("plate.png"), b"same-bytes").unwrap();
        let first = copy_into_library(&root, &uploads.join("plate.png"), "plate.png").unwrap();
        assert!(!first.duplicate);
        let second = copy_into_library(&root, &uploads.join("plate.png"), "other.png").unwrap();
        assert!(second.duplicate);
        assert_eq!(second.id, first.id);
        assert!(uploads.join("plate.png").is_file());
        assert!(root.join("assets/images/plate.png").is_file());
        assert!(!root.join("assets/images/other.png").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_linked_folder_is_a_warning_and_keeps_the_item() {
        let root = temp_pack("link");
        let gone = root.join("external");
        fs::create_dir_all(&gone).unwrap();
        fs::write(gone.join("clip.png"), b"x").unwrap();
        // A 1-byte file is not a PNG; write a real image so the first scan is ok.
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .save(gone.join("clip.png"))
            .unwrap();
        let mut lib = MediaLibrary::open(root.clone());
        lib.link_folder(gone.clone());
        assert!(lib.items.iter().any(|item| item.name == "clip.png" && item.location == "linked"));
        let _ = fs::remove_dir_all(&gone);
        lib.rescan();
        assert!(lib.warnings.iter().any(|w| w.contains("missing")));
        let kept = lib.items.iter().find(|item| item.name == "clip.png").unwrap();
        assert_eq!(kept.status, "missing");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn too_many_clips_are_refused_with_every_name() {
        let clips = vec![
            ClipPlan {
                name: "a.mp4".into(),
                ram_bytes: 100,
                problem: None,
            },
            ClipPlan {
                name: "b.mp4".into(),
                ram_bytes: 100,
                problem: Some("90s is over the 60s decode limit".into()),
            },
        ];
        let message = admission_error(7, 0, 8, &clips).unwrap();
        assert!(message.contains("Nothing was added"));
        assert!(message.contains("a.mp4") == false || message.contains("free layer"));
        assert!(message.contains("b.mp4"));
        assert!(message.contains("free layer"));
        assert!(admission_error(0, 0, 8, &clips[..1]).is_none());
    }
}
