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
    /// Physical file used when probing. Invalidate cache on media-root switch.
    #[serde(default)]
    source_path: String,
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
    /// Local-only override for the show's video/ and images/ directories.
    /// Stored in gitignored media-sources.json, never in authored presets.
    #[serde(default)]
    primary_media_root: Option<String>,
    #[serde(default)]
    linked_folders: Vec<String>,
    /// Individual files registered from Finder. The bytes stay at this path.
    #[serde(default)]
    linked_files: Vec<String>,
    /// Library ids hidden by the trash icon. The files on disk stay.
    #[serde(default)]
    hidden: Vec<String>,
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
    primary_media_root: Option<PathBuf>,
    linked_folders: Vec<PathBuf>,
    linked_files: Vec<PathBuf>,
    hidden: Vec<String>,
    watch: Option<LibraryWatch>,
    /// ffprobe, fingerprints, thumbnails and copies run here, never on the
    /// render thread. `None` in tests, where a rescan runs inline.
    scanner: Option<Scanner>,
    /// A rescan was asked for while the scanner was busy.
    scan_wanted: bool,
    /// Folder of earlier uploads to copy in on the next scan.
    import_wanted: Option<PathBuf>,
}

/// What one background scan needs. A snapshot, so the render thread keeps
/// the library while the scan runs.
struct ScanRequest {
    pack_root: PathBuf,
    linked_folders: Vec<PathBuf>,
    linked_files: Vec<PathBuf>,
    hidden: Vec<String>,
    import_from: Option<PathBuf>,
}

struct ScanResult {
    items: Vec<LibraryItem>,
    warnings: Vec<String>,
    imported: Option<usize>,
}

struct Scanner {
    tx: crossbeam_channel::Sender<ScanRequest>,
    rx: crossbeam_channel::Receiver<ScanResult>,
    busy: bool,
}

impl Scanner {
    fn spawn() -> Option<Self> {
        let (tx, request_rx) = crossbeam_channel::bounded::<ScanRequest>(1);
        let (result_tx, rx) = crossbeam_channel::bounded::<ScanResult>(1);
        std::thread::Builder::new()
            .name("fosfora-media-scan".into())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let imported = request
                        .import_from
                        .as_deref()
                        .map(|from| import_folder(&request.pack_root, from));
                    let (items, warnings) = scan_library(&request);
                    if result_tx
                        .send(ScanResult {
                            items,
                            warnings,
                            imported,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            })
            .ok()?;
        Some(Self {
            tx,
            rx,
            busy: false,
        })
    }
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
        let sources = load_sources(&pack_root);
        let watch = start_watch(
            &pack_root,
            &watch_dirs(
                &sources.linked_folders,
                &sources.linked_files,
                sources.primary_media_root.as_deref(),
            ),
        );
        // The last scan's list shows at once; a fresh scan replaces it.
        let cached = fs::read_to_string(library_json_path(&pack_root))
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<LibraryItem>>(&text).ok())
            .unwrap_or_default();
        let mut lib = Self {
            pack_root,
            items: cached,
            warnings: Vec::new(),
            primary_media_root: sources.primary_media_root,
            linked_folders: sources.linked_folders,
            linked_files: sources.linked_files,
            hidden: sources.hidden,
            watch,
            scanner: if cfg!(test) { None } else { Scanner::spawn() },
            scan_wanted: false,
            import_wanted: None,
        };
        lib.prune_hidden();
        lib.write_sources();
        lib.request_rescan();
        lib
    }

    /// Ask for a rescan. With a scanner it runs in the background and lands
    /// through [`MediaLibrary::poll`]; without one it runs now.
    pub fn request_rescan(&mut self) {
        if self.scanner.is_none() {
            if let Some(from) = self.import_wanted.take() {
                import_folder(&self.pack_root, &from);
            }
            self.rescan();
            return;
        }
        self.scan_wanted = true;
        self.dispatch_scan();
    }

    /// True while a scan is running or waiting.
    pub fn scanning(&self) -> bool {
        self.scan_wanted || self.scanner.as_ref().is_some_and(|scanner| scanner.busy)
    }

    /// Call once per frame. Applies a finished scan and starts a waiting one.
    /// Returns a message when an import of earlier uploads finished.
    pub fn poll(&mut self) -> Option<String> {
        if self.poll_watch() {
            self.scan_wanted = true;
        }
        let mut message = None;
        if let Some(scanner) = &mut self.scanner {
            if let Ok(result) = scanner.rx.try_recv() {
                scanner.busy = false;
                self.items = result.items;
                self.warnings = result.warnings;
                if let Some(count) = result.imported {
                    message = Some(format!("Imported {count} files from previous uploads"));
                }
            }
        }
        self.dispatch_scan();
        message
    }

    fn dispatch_scan(&mut self) {
        if !self.scan_wanted {
            return;
        }
        let request = ScanRequest {
            pack_root: self.pack_root.clone(),
            linked_folders: self.linked_folders.clone(),
            linked_files: self.linked_files.clone(),
            hidden: self.hidden.clone(),
            import_from: self.import_wanted.clone(),
        };
        let Some(scanner) = &mut self.scanner else {
            return;
        };
        if scanner.busy {
            return;
        }
        if scanner.tx.try_send(request).is_ok() {
            scanner.busy = true;
            self.scan_wanted = false;
            self.import_wanted = None;
        }
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

    /// Scan now, on this thread. Startup fallback and tests only.
    pub fn rescan(&mut self) {
        let request = ScanRequest {
            pack_root: self.pack_root.clone(),
            linked_folders: self.linked_folders.clone(),
            linked_files: self.linked_files.clone(),
            hidden: self.hidden.clone(),
            import_from: None,
        };
        let (items, warnings) = scan_library(&request);
        self.items = items;
        self.warnings = warnings;
    }

    pub fn link_folder(&mut self, folder: PathBuf) {
        let folder = fs::canonicalize(&folder).unwrap_or(folder);
        if !self.linked_folders.iter().any(|have| have == &folder) {
            self.linked_folders.push(folder);
        }
        self.persist_sources();
    }

    /// Remember files or folders where they already are. Nothing is copied.
    pub fn link_paths(&mut self, paths: &[PathBuf]) -> Result<String, String> {
        let mut added = Vec::new();
        let mut already = Vec::new();
        let mut problems = Vec::new();
        for path in paths {
            match self.stage_link(path) {
                Ok(true) => added.push(display_name(path)),
                Ok(false) => already.push(display_name(path)),
                Err(message) => problems.push(message),
            }
        }
        if !added.is_empty() {
            self.persist_sources();
        }
        if added.is_empty() && already.is_empty() {
            return Err(if problems.is_empty() {
                "Nothing was registered.".into()
            } else {
                problems.join(" ")
            });
        }
        let mut message = if !added.is_empty() {
            format!(
                "Registered {}. It stays where it is and was not copied.",
                added.join(", ")
            )
        } else {
            format!(
                "{} is already in the library. It was not copied.",
                already.join(", ")
            )
        };
        if !problems.is_empty() {
            message.push(' ');
            message.push_str(&problems.join(" "));
        }
        Ok(message)
    }

    /// Drop a library entry. A linked file stays on disk. A show-pack copy is
    /// deleted only when no scene names it.
    pub fn remove_item(&mut self, id: &str) -> Result<String, String> {
        let item = self
            .items
            .iter()
            .find(|item| item.id == id)
            .cloned()
            .ok_or_else(|| "That library item is not in the list".to_string())?;
        let used = scenes_naming(&self.pack_root, &item);
        if !used.is_empty() {
            return Err(format!(
                "{} is used by {} and was not removed",
                item.name,
                used.join(", ")
            ));
        }
        if item.location == "external" || item.location == "linked" {
            if item.location == "linked" {
                let path = PathBuf::from(&item.path);
                let canon = fs::canonicalize(&path).unwrap_or(path);
                self.linked_files
                    .retain(|have| have != &canon && have.to_string_lossy() != item.path);
            }
            return self.hide_library_item(&item);
        }
        let Some(path) = removable_pack_file(&self.pack_root, &item) else {
            // A show pack can point assets/ at an external library. Hiding the
            // row leaves that file where it is.
            return self.hide_library_item(&item);
        };
        // Preserve imported footage even if an unsaved scene still refers to it.
        // A hidden recovery directory inside the pack keeps the move on the same
        // volume and allows the operator to restore the original file later.
        let recovery = self
            .pack_root
            .join("recovery")
            .join("history")
            .join("removed-media");
        fs::create_dir_all(&recovery).map_err(|err| err.to_string())?;
        let dest = unique_dest(&recovery, &item.name);
        fs::rename(&path, &dest).map_err(|err| err.to_string())?;
        self.items.retain(|have| have.id != item.id);
        self.request_rescan();
        Ok(format!(
            "Removed {} from the library; recover it at {}.",
            item.name,
            dest.display()
        ))
    }

    fn stage_link(&mut self, path: &Path) -> Result<bool, String> {
        let canon =
            fs::canonicalize(path).map_err(|_| format!("{} is not available", path.display()))?;
        for hide in related_ids(&canon, "") {
            self.hidden.retain(|id| id != &hide);
        }
        if canon.is_dir() {
            if self.linked_folders.iter().any(|have| have == &canon) {
                return Ok(false);
            }
            self.linked_folders.push(canon);
            return Ok(true);
        }
        let name = canon
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        if media_kind(name).is_none() {
            return Err(format!("{name} is not a video or image"));
        }
        if self.linked_files.iter().any(|have| have == &canon)
            || path_is_in_pack(&self.pack_root, &canon)
        {
            return Ok(false);
        }
        self.linked_files.push(canon);
        Ok(true)
    }

    fn persist_sources(&mut self) {
        self.prune_hidden();
        self.write_sources();
        if let Some(watch) = start_watch(
            &self.pack_root,
            &watch_dirs(
                &self.linked_folders,
                &self.linked_files,
                self.primary_media_root.as_deref(),
            ),
        ) {
            self.watch = Some(watch);
        }
        self.request_rescan();
    }

    /// Drop a library row and remember the id. The file stays on disk.
    fn hide_library_item(&mut self, item: &LibraryItem) -> Result<String, String> {
        let path = PathBuf::from(&item.path);
        for hide in related_ids(&path, &item.id) {
            if !self.hidden.iter().any(|id| id == &hide) {
                self.hidden.push(hide);
            }
        }
        if !self.hidden.iter().any(|id| id == &item.id) {
            self.hidden.push(item.id.clone());
        }
        self.persist_sources();
        Ok(format!(
            "Removed {} from the library. The file was left where it is.",
            item.name
        ))
    }

    fn prune_hidden(&mut self) {
        self.hidden.retain(|id| {
            if !id.starts_with("linked:") {
                return true;
            }
            self.linked_folders
                .iter()
                .any(|folder| id.starts_with(&format!("linked:{}:", folder.to_string_lossy())))
        });
    }

    fn write_sources(&self) {
        let file = SourceFile {
            primary_media_root: self
                .primary_media_root
                .as_ref()
                .map(|path| path.to_string_lossy().to_string()),
            linked_folders: self
                .linked_folders
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect(),
            linked_files: self
                .linked_files
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect(),
            hidden: self.hidden.clone(),
        };
        let _ = write_json(&self.pack_root.join("media-sources.json"), &file);
    }

    pub fn reveal_in_finder(&self) -> Result<(), String> {
        let dir = self
            .primary_media_root
            .clone()
            .unwrap_or_else(|| self.pack_root.join("assets"));
        if self.primary_media_root.is_some() && (!dir.is_absolute() || !dir.is_dir()) {
            return Err(format!(
                "The configured media folder must be an available absolute path: {}",
                dir.display()
            ));
        }
        if self.primary_media_root.is_none() {
            let _ = fs::create_dir_all(&dir);
        }
        open_folder(&dir)
    }

    /// Copy earlier Application Support uploads into the show library on the
    /// scanner thread. The originals stay. [`MediaLibrary::poll`] reports the
    /// count when it is done.
    pub fn import_uploads(&mut self, uploads: &Path) {
        self.import_wanted = Some(uploads.to_path_buf());
        self.request_rescan();
    }

    pub fn resolve(&self, id: &str) -> Option<PathBuf> {
        if let Some(item) = self.items.iter().find(|item| item.id == id) {
            return item_file(&self.pack_root, item);
        }
        let direct = resolve_pack_media_path(&self.pack_root, id);
        direct.is_file().then_some(direct)
    }
}

/// Probe, fingerprint and thumbnail every library file. Cached entries are
/// reused, so a folder of clips is browsable without decoding any of them.
fn scan_library(request: &ScanRequest) -> (Vec<LibraryItem>, Vec<String>) {
    let pack_root = &request.pack_root;
    let previous = fs::read_to_string(library_json_path(pack_root))
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<LibraryItem>>(&text).ok())
        .unwrap_or_default();
    let cache = load_cache(pack_root);
    let scenes = scenes_using_media(pack_root);
    let mut warnings = Vec::new();
    let mut items = Vec::new();
    let mut next_cache = ProbeCache {
        entries: Vec::new(),
    };

    // The configured primary library takes precedence over legacy pack
    // assets with the same virtual path, without deleting either source.
    if let Some(primary) = primary_media_root(pack_root) {
        if !primary.is_absolute() || !primary.is_dir() {
            warnings.push(format!(
                "Primary media folder is invalid or unavailable (use an absolute, fully downloaded path): {}",
                primary.display()
            ));
            for old in previous.iter().filter(|item| item.location == "external") {
                let mut kept = old.clone();
                kept.status = "missing".into();
                kept.detail = "Primary media folder is unavailable".into();
                items.push(kept);
            }
        } else {
            for seen in primary_files(&primary) {
                if let Some(item) = describe_file(&seen, &cache, &mut next_cache, pack_root) {
                    items.push(item);
                }
            }
        }
    }

    for seen in pack_files(pack_root) {
        if let Some(item) = describe_file(&seen, &cache, &mut next_cache, pack_root) {
            items.push(item);
        }
    }

    for folder in &request.linked_folders {
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
            if let Some(item) = describe_file(&seen, &cache, &mut next_cache, pack_root) {
                items.push(item);
            }
        }
    }

    for path in &request.linked_files {
        if !path.is_file() {
            warnings.push(format!("Linked media file is missing: {}", path.display()));
            let id = linked_file_id(path);
            for old in previous.iter().filter(|item| item.id == id) {
                let mut kept = old.clone();
                kept.status = "missing".into();
                kept.detail = "Linked file is not available".into();
                if !items.iter().any(|item| item.id == kept.id) {
                    items.push(kept);
                }
            }
            continue;
        }
        if let Some(seen) = linked_file_seen(path) {
            if let Some(item) = describe_file(&seen, &cache, &mut next_cache, pack_root) {
                items.push(item);
            }
        }
    }

    for item in &mut items {
        item.scenes = scenes_for(item, &scenes);
    }
    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let hidden = &request.hidden;
    items.retain(|item| !hidden.iter().any(|id| id == &item.id));
    let mut seen_ids = std::collections::HashSet::new();
    items.retain(|item| seen_ids.insert(item.id.clone()));
    let mut seen_paths = std::collections::HashSet::new();
    items.retain(|item| seen_paths.insert(item.path.clone()));

    let _ = write_json(&cache_path(pack_root), &next_cache);
    let _ = write_json(&library_json_path(pack_root), &items);
    (items, warnings)
}

/// Copy every media file in `from` into the pack. Returns how many landed.
fn import_folder(pack_root: &Path, from: &Path) -> usize {
    let Ok(entries) = fs::read_dir(from) else {
        return 0;
    };
    let mut copied = 0;
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
        match copy_into_library(pack_root, &path, name) {
            Ok(_) => copied += 1,
            Err(err) => log::warn!("import {}: {err}", path.display()),
        }
    }
    copied
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
pub fn commit_upload(
    pack_root: &Path,
    partial: &Path,
    original_name: &str,
) -> Result<UploadReport, String> {
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
    let primary = primary_media_root(pack_root);
    // Existing project-local library retains its 20 GB limit. A deliberate
    // external iCloud/media-root configuration is managed by the user.
    if primary.is_none() {
        let used = directory_bytes(&pack_root.join("assets/video"))
            + directory_bytes(&pack_root.join("assets/images"));
        if used.saturating_add(meta.len()) > MAX_LIBRARY_BYTES {
            let _ = fs::remove_file(partial);
            return Err("the show media folder is over 20 GB".into());
        }
    }
    let subfolder = if kind == "video" { "video" } else { "images" };
    let dir = primary
        .as_ref()
        .map(|root| root.join(subfolder))
        .unwrap_or_else(|| pack_root.join("assets").join(subfolder));
    // Never create a replacement cloud folder elsewhere if iCloud has gone
    // missing: the operator needs an actionable error, not split libraries.
    if let Some(root) = primary.as_ref() {
        if !root.is_absolute() || !root.is_dir() {
            return Err(format!(
                "Primary media folder must be an available absolute path: {}",
                root.display()
            ));
        }
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = unique_dest(&dir, &name);
    if primary.is_some() {
        // The upload temp lives under the show pack, potentially on another
        // filesystem. Copy to a hidden sibling first, then publish atomically.
        let stage = dir.join(format!(".fosfora-incoming-{}", incoming_token()));
        if let Err(err) = fs::copy(partial, &stage) {
            let _ = fs::remove_file(&stage);
            return Err(err.to_string());
        }
        if let Err(err) = fs::rename(&stage, &dest) {
            let _ = fs::remove_file(&stage);
            return Err(err.to_string());
        }
        let _ = fs::remove_file(partial);
    } else {
        fs::rename(partial, &dest).map_err(|e| e.to_string())?;
    }
    let rel = format!(
        "assets/{subfolder}/{}",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or(&name)
    );
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

pub fn copy_into_library(
    pack_root: &Path,
    src: &Path,
    original_name: &str,
) -> Result<UploadReport, String> {
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
                let ram = meta.decoded_bytes;
                let mut problem = None;
                if meta.duration_secs > max_predecode_secs() {
                    problem = Some(format!(
                        "{:.0}s is over the {:.0}s decode limit",
                        meta.duration_secs,
                        max_predecode_secs()
                    ));
                } else if ram > MAX_CLIP_RAM {
                    problem = Some(format!(
                        "{}x{} {} × {} frames would decode to {}, over {} per clip",
                        meta.width,
                        meta.height,
                        meta.pix_fmt,
                        meta.frames,
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
/// should be loaded. `available` is what the decoded-memory budget still
/// allows right now (see `media::jobs::MediaBudget`).
pub fn admission_error(
    existing_layers: usize,
    available: u64,
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
    if added > available {
        problems.push(format!(
            "Decoding needs {}; the memory budget has {} left.",
            format_bytes(added),
            format_bytes(available)
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
        entry.path == seen.id
            && entry.source_path == seen.path.to_string_lossy().as_ref()
            && entry.bytes == meta.len()
            && entry.modified_ms == modified_ms
    });
    let probe = if let Some(hit) = cached {
        hit.clone()
    } else {
        let probed = probe_uncached(&seen.path, kind);
        let fingerprint = file_fingerprint(&seen.path).unwrap_or_default();
        let entry = CachedProbe {
            path: seen.id.clone(),
            source_path: seen.path.to_string_lossy().to_string(),
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
        path: if seen.location == "linked" {
            seen.path.to_string_lossy().to_string()
        } else {
            seen.id.clone()
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
                let ram = meta.decoded_bytes;
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
    frames: u64,
    pix_fmt: String,
    decoded_bytes: u64,
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
            frames: meta.frame_count(),
            pix_fmt: meta.pix_fmt.clone(),
            decoded_bytes: meta.decoded_bytes(),
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
    if ok && dest.is_file() {
        Some(name)
    } else {
        None
    }
}

fn write_video_thumb(src: &Path, dest: &Path) -> bool {
    let tool = super::toolchain::VideoToolchain::global(None);
    let Some(mut cmd) = tool.ffmpeg_cmd() else {
        return false;
    };
    cmd.args(["-y", "-ss", "0.2", "-i"])
        .arg(src)
        .args(["-frames:v", "1", "-vf", "scale=160:-2", "-q:v", "8"])
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

fn primary_files(primary: &Path) -> Vec<SeenFile> {
    let mut files = Vec::new();
    for folder in ["video", "images", ""] {
        let dir = primary.join(folder);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            // Own the name before moving path into SeenFile. Borrowing it
            // from path.file_name() would keep path borrowed across the move.
            let Some(name) = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if media_kind(&name).is_none() {
                continue;
            }
            let kind = if folder.is_empty() {
                if media_kind(&name) == Some("video") {
                    "video"
                } else {
                    "images"
                }
            } else {
                folder
            };
            files.push(SeenFile {
                path,
                id: format!("assets/{kind}/{name}"),
                location: "external".into(),
                source: primary.to_string_lossy().to_string(),
            });
        }
    }
    files
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
        let matches = media == &item.path
            || media == &item.id
            || Path::new(media).file_name() == Path::new(&item.name).file_name();
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
        resolve_pack_media_path(pack_root, &item.path)
    };
    path.is_file().then_some(path)
}

fn find_fingerprint(pack_root: &Path, fingerprint: &str) -> Option<(String, String)> {
    // Reuse the background scanner's fingerprints for iCloud assets: reading
    // every clip again could trigger downloads just to check a new upload.
    for cached in load_cache(pack_root).entries {
        if cached.fingerprint == fingerprint && !cached.fingerprint.is_empty() {
            let path = resolve_pack_media_path(pack_root, &cached.path);
            if cached.source_path == path.to_string_lossy().as_ref()
                && fs::metadata(&path).ok().is_some_and(|meta| {
                    meta.len() == cached.bytes && modified_ms(&meta) == cached.modified_ms
                })
            {
                let name = Path::new(&cached.path)
                    .file_name()?
                    .to_string_lossy()
                    .to_string();
                return Some((cached.path, name));
            }
        }
    }
    // Retain the old uncached fallback for small, local show-pack imports.
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

/// Optional primary media directory, usually on an external disk or iCloud Drive.
/// This is local configuration: the config file is excluded from show-pack Git.
pub fn primary_media_root(pack_root: &Path) -> Option<PathBuf> {
    // Return even an invalid value, so uploads will refuse it rather than
    // silently writing to the bundled directory and splitting the library.
    load_sources(pack_root).primary_media_root
}

/// Keep saved paths portable even when the physical media lives outside the
/// show pack. Only assets/video/* and assets/images/* are redirected.
pub fn resolve_pack_media_path(pack_root: &Path, stored: &str) -> PathBuf {
    let path = Path::new(stored);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let pack_path = pack_root.join(path);
    let mut components = path.components();
    let in_media = match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(a)), Some(std::path::Component::Normal(kind)))
            if a == "assets" && (kind == "video" || kind == "images") =>
        {
            Some(kind.to_os_string())
        }
        _ => None,
    };
    if let (Some(kind), Some(root)) = (in_media, primary_media_root(pack_root)) {
        if !root.is_absolute() {
            return pack_path;
        }
        // Reject traversal; only ordinary file components can extend the media root.
        let mut rest = PathBuf::new();
        for component in components {
            if let std::path::Component::Normal(name) = component {
                rest.push(name);
            } else {
                return pack_path;
            }
        }
        if !rest.as_os_str().is_empty() {
            let external = root.join(kind).join(&rest);
            if external.is_file() {
                return external;
            }
            // Existing iCloud folders may contain a flat set of files.
            // Accept those without forcing a destructive reorganisation.
            if rest.components().count() == 1 {
                let flat = root.join(&rest);
                if flat.is_file() {
                    return flat;
                }
            }
            if !pack_path.is_file() {
                return external;
            }
        }
    }
    pack_path
}

/// Convert a primary-library absolute path back into the stable stored alias.
pub fn virtual_media_path(pack_root: &Path, media: &Path) -> Option<String> {
    let root = primary_media_root(pack_root)?;
    if !root.is_absolute() {
        return None;
    }
    let relative = if let Ok(relative) = media.strip_prefix(&root) {
        relative.to_path_buf()
    } else {
        let real_root = fs::canonicalize(&root).ok()?;
        let real_media = fs::canonicalize(media).ok()?;
        real_media.strip_prefix(real_root).ok()?.to_path_buf()
    };
    let mut parts = relative.components();
    let first = parts.next()?;
    let mut tail = PathBuf::new();
    let kind = match first {
        std::path::Component::Normal(name) if name == "video" || name == "images" => {
            for part in parts {
                if let std::path::Component::Normal(name) = part {
                    tail.push(name);
                } else {
                    return None;
                }
            }
            name.to_string_lossy().into_owned()
        }
        // A flat primary directory is supported as well as video/ and images/.
        std::path::Component::Normal(name) if parts.next().is_none() => {
            let kind = media_kind(name.to_str()?)?;
            tail.push(name);
            if kind == "video" {
                "video".into()
            } else {
                "images".into()
            }
        }
        _ => return None,
    };
    if tail.as_os_str().is_empty() {
        return None;
    }
    Some(format!("assets/{kind}/{}", tail.to_string_lossy()).replace('\\', "/"))
}

fn load_sources(pack_root: &Path) -> SourcePaths {
    let file = fs::read_to_string(pack_root.join("media-sources.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<SourceFile>(&text).ok())
        .unwrap_or_default();
    SourcePaths {
        primary_media_root: file.primary_media_root.map(PathBuf::from),
        linked_folders: file.linked_folders.into_iter().map(PathBuf::from).collect(),
        linked_files: file.linked_files.into_iter().map(PathBuf::from).collect(),
        hidden: file.hidden,
    }
}

struct SourcePaths {
    primary_media_root: Option<PathBuf>,
    linked_folders: Vec<PathBuf>,
    linked_files: Vec<PathBuf>,
    hidden: Vec<String>,
}

fn watch_dirs(folders: &[PathBuf], files: &[PathBuf], primary: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = folders.to_vec();
    if let Some(primary) = primary {
        if primary.is_dir() {
            dirs.push(primary.to_path_buf());
        }
        for subfolder in ["video", "images"] {
            let path = primary.join(subfolder);
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    for path in files {
        if let Some(parent) = path.parent() {
            if parent.is_dir() && !dirs.iter().any(|dir| dir == parent) {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    dirs
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file")
        .to_string()
}

fn linked_file_id(path: &Path) -> String {
    format!("file:{}", path.to_string_lossy())
}

fn linked_file_seen(path: &Path) -> Option<SeenFile> {
    let name = path.file_name()?.to_str()?;
    if media_kind(name).is_none() {
        return None;
    }
    Some(SeenFile {
        id: linked_file_id(path),
        path: path.to_path_buf(),
        location: "linked".into(),
        source: path
            .parent()
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_default(),
    })
}

fn related_ids(path: &Path, item_id: &str) -> Vec<String> {
    let mut ids = Vec::new();
    if !item_id.is_empty() {
        ids.push(item_id.to_string());
    }
    ids.push(linked_file_id(path));
    if let (Some(parent), Some(name)) = (
        path.parent(),
        path.file_name().and_then(|name| name.to_str()),
    ) {
        ids.push(format!("linked:{}:{name}", parent.to_string_lossy()));
    }
    ids
}

fn path_is_in_pack(pack_root: &Path, path: &Path) -> bool {
    let Ok(root) = fs::canonicalize(pack_root) else {
        return false;
    };
    path.starts_with(root)
}

fn scenes_naming(pack_root: &Path, item: &LibraryItem) -> Vec<String> {
    let mut used = Vec::new();
    for (scene, media) in scenes_using_media(pack_root) {
        if (media == item.path || media == item.id) && !used.contains(&scene) {
            used.push(scene);
        }
    }
    used
}

fn removable_pack_file(pack_root: &Path, item: &LibraryItem) -> Option<PathBuf> {
    if item.location != "pack" {
        return None;
    }
    let path = fs::canonicalize(pack_root.join(&item.path)).ok()?;
    for dir in ["assets/video", "assets/images"] {
        let media_dir = pack_root.join(dir);
        // A show pack can symlink these directories into an iCloud library.
        // Never move a target out of a linked directory into pack recovery:
        // that would silently remove the original from all synced devices.
        if fs::symlink_metadata(&media_dir)
            .ok()
            .is_some_and(|meta| meta.file_type().is_symlink())
        {
            continue;
        }
        let Ok(root) = fs::canonicalize(media_dir) else {
            continue;
        };
        if path.starts_with(&root) && path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Paths currently on the Finder drag pasteboard. Empty when nothing is dragging
/// or the pasteboard has no media. This does not read or copy file bytes.
pub fn finder_drag_paths() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let mut paths = Vec::new();
        for line in macos_drag_paths() {
            let Some(path) = drag_line_to_path(&line) else {
                continue;
            };
            let text = path.to_string_lossy().to_string();
            if !paths.contains(&text) {
                paths.push(text);
            }
        }
        paths
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn macos_drag_paths() -> Vec<String> {
    let script = r#"use framework "Foundation"
set pb to current application's NSPasteboard's pasteboardWithName:"NSDragPboard"
set collected to ""
try
  set boardItems to pb's pasteboardItems()
  set n to count of boardItems
  repeat with i from 0 to (n - 1)
    set one to (boardItems's objectAtIndex:i)
    try
      set raw to (one's stringForType:"public.file-url")
      if raw is not missing value then set collected to collected & (raw as text) & linefeed
    end try
  end repeat
end try
try
  set names to pb's propertyListForType:"NSFilenamesPboardType"
  repeat with oneName in names
    set collected to collected & (oneName as text) & linefeed
  end repeat
end try
return collected"#;
    let Ok(mut child) = std::process::Command::new("osascript")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    {
        use std::io::Write;
        let Some(mut stdin) = child.stdin.take() else {
            return Vec::new();
        };
        if stdin.write_all(script.as_bytes()).is_err() {
            return Vec::new();
        }
    }
    let Ok(output) = child.wait_with_output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn drag_line_to_path(line: &str) -> Option<PathBuf> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let path = if line.starts_with("file:") {
        file_url_to_path(line)?
    } else if line.starts_with('/') {
        PathBuf::from(line)
    } else {
        return None;
    };
    let canon = fs::canonicalize(&path).ok()?;
    if canon.is_dir() {
        return Some(canon);
    }
    let name = canon.file_name()?.to_str()?;
    media_kind(name)?;
    canon.is_file().then_some(canon)
}

fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.trim().strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    Some(PathBuf::from(percent_decode(rest)))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(
                std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                16,
            ) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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

    #[cfg(unix)]
    #[test]
    fn trash_refuses_pack_media_directories_symlinked_into_external_library() {
        use std::os::unix::fs::symlink;

        let root = temp_pack("linked_pack_media");
        let external = root.join("iCloud-originals");
        fs::create_dir_all(&external).unwrap();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]))
            .save(external.join("keep.png"))
            .unwrap();

        // Mirror the existing Mac pack: assets/images is a symlink to
        // iCloud, and no primary_media_root setting is required for exposure.
        fs::remove_dir(root.join("assets/images")).unwrap();
        symlink(&external, root.join("assets/images")).unwrap();
        let mut lib = MediaLibrary::open(root.clone());
        assert!(
            lib.items
                .iter()
                .any(|item| item.id == "assets/images/keep.png")
        );
        assert!(lib.remove_item("assets/images/keep.png").is_ok());
        assert!(external.join("keep.png").exists());
        assert!(
            !root
                .join("recovery/history/removed-media/keep.png")
                .exists()
        );
        assert!(
            lib.items
                .iter()
                .all(|item| item.id != "assets/images/keep.png")
        );
        let again = MediaLibrary::open(root.clone());
        assert!(
            again
                .items
                .iter()
                .all(|item| item.id != "assets/images/keep.png")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn external_primary_library_uses_portable_ids_and_never_trashes_cloud_files() {
        let root = temp_pack("primary");
        let external = root.join("Cloud Movies");
        fs::create_dir_all(external.join("video")).unwrap();
        fs::create_dir_all(external.join("images")).unwrap();
        let config = serde_json::json!({
            "primary_media_root": external.to_string_lossy(),
            "linked_folders": [],
            "linked_files": [],
            "hidden": []
        });
        fs::write(root.join("media-sources.json"), config.to_string()).unwrap();

        // Support a flat user-managed folder without requiring a move.
        image::RgbaImage::from_pixel(8, 4, image::Rgba([5, 6, 7, 255]))
            .save(external.join("plate.png"))
            .unwrap();
        fs::write(
            root.join("presets/under-ice.json"),
            r#"{"layers":[{"effect_name":"","media_path":"assets/images/plate.png"}]}"#,
        )
        .unwrap();
        let mut lib = MediaLibrary::open(root.clone());
        let plate = lib
            .items
            .iter()
            .find(|item| item.name == "plate.png")
            .unwrap();
        assert_eq!(plate.id, "assets/images/plate.png");
        assert_eq!(plate.path, "assets/images/plate.png");
        assert_eq!(plate.location, "external");
        assert_eq!(plate.scenes, vec!["under-ice".to_string()]);
        assert_eq!(lib.resolve(&plate.id).unwrap(), external.join("plate.png"));
        assert_eq!(
            resolve_pack_media_path(&root, &plate.path),
            external.join("plate.png")
        );
        assert_eq!(
            virtual_media_path(&root, &external.join("plate.png")).unwrap(),
            "assets/images/plate.png"
        );
        assert!(lib.remove_item("assets/images/plate.png").is_err());
        assert!(external.join("plate.png").exists());
        image::RgbaImage::from_pixel(4, 4, image::Rgba([8, 8, 8, 255]))
            .save(external.join("images/spare.png"))
            .unwrap();
        lib.rescan();
        assert!(lib.remove_item("assets/images/spare.png").is_ok());
        assert!(external.join("images/spare.png").exists());
        assert!(lib.items.iter().all(|item| item.name != "spare.png"));
        let again = MediaLibrary::open(root.clone());
        assert!(again.items.iter().all(|item| item.name != "spare.png"));

        // Uploads/imports go into the cloud location, but their IDs remain
        // portable and the show pack keeps only metadata and thumbnails.
        let src = root.join("upload.png");
        image::RgbaImage::from_pixel(3, 3, image::Rgba([9, 10, 11, 255]))
            .save(&src)
            .unwrap();
        let result = copy_into_library(&root, &src, "upload.png").unwrap();
        assert_eq!(result.id, "assets/images/upload.png");
        assert!(external.join("images/upload.png").is_file());
        assert!(!root.join("assets/images/upload.png").exists());
        assert_eq!(
            virtual_media_path(&root, &external.join("images/upload.png")).unwrap(),
            result.id
        );
        lib.rescan();
        assert!(lib.items.iter().any(|item| item.id == result.id));
        // Opening again must preserve the local config across write_sources().
        let lib2 = MediaLibrary::open(root.clone());
        assert_eq!(
            lib2.resolve(&result.id),
            Some(external.join("images/upload.png"))
        );
        let stored: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("media-sources.json")).unwrap())
                .unwrap();
        assert_eq!(
            stored["primary_media_root"],
            external.to_string_lossy().as_ref()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn external_primary_unavailable_is_reported_and_does_not_create_fake_files() {
        let root = temp_pack("missing_primary");
        let missing = root.join("missing-iCloud-folder");
        fs::write(
            root.join("media-sources.json"),
            serde_json::json!({"primary_media_root":missing.to_string_lossy()}).to_string(),
        )
        .unwrap();
        let lib = MediaLibrary::open(root.clone());
        assert!(lib.warnings.iter().any(|w| w.contains("unavailable")));
        let path = resolve_pack_media_path(&root, "assets/video/clip.mp4");
        assert_eq!(path, missing.join("video/clip.mp4"));
        assert!(!missing.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_still_in_the_pack_shows_up_once_and_again() {
        let root = temp_pack("scan");
        let image = image::RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 255]));
        image.save(root.join("assets/images/plate.png")).unwrap();
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
        let notes = lib
            .items
            .iter()
            .find(|item| item.name == "notes.txt")
            .unwrap();
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
        assert!(
            lib.items
                .iter()
                .any(|item| item.name == "clip.png" && item.location == "linked")
        );
        let _ = fs::remove_dir_all(&gone);
        lib.rescan();
        assert!(lib.warnings.iter().any(|w| w.contains("missing")));
        let kept = lib
            .items
            .iter()
            .find(|item| item.name == "clip.png")
            .unwrap();
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
        assert!(admission_error(0, u64::MAX, 8, &clips[..1]).is_none());
        let over = admission_error(0, 99, 8, &clips[..1]).unwrap();
        assert!(over.contains("memory budget"));
    }

    #[test]
    fn a_file_url_keeps_spaces() {
        let path = file_url_to_path("file:///Users/me/My%20Clip.mov").unwrap();
        assert_eq!(path, PathBuf::from("/Users/me/My Clip.mov"));
    }

    #[test]
    fn linking_a_file_does_not_copy_it_and_trash_leaves_it() {
        let root = temp_pack("linkfile");
        let outside = std::env::temp_dir().join(format!("fosfora_outside_{}", std::process::id()));
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([4, 5, 6, 255]))
            .save(outside.join("kept.png"))
            .unwrap();
        let mut lib = MediaLibrary::open(root.clone());
        let message = lib.link_paths(&[outside.join("kept.png")]).unwrap();
        assert!(message.contains("was not copied"));
        assert!(outside.join("kept.png").is_file());
        assert!(!root.join("assets/images/kept.png").exists());
        let item = lib
            .items
            .iter()
            .find(|item| item.name == "kept.png")
            .unwrap();
        assert_eq!(item.location, "linked");
        let id = item.id.clone();
        let removed = lib.remove_item(&id).unwrap();
        assert!(removed.contains("left where it is"));
        assert!(outside.join("kept.png").is_file());
        assert!(lib.items.iter().all(|item| item.name != "kept.png"));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn trash_recovers_an_unused_pack_file_and_refuses_one_a_scene_names() {
        let root = temp_pack("trash");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 1, 1, 255]))
            .save(root.join("assets/images/spare.png"))
            .unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([2, 2, 2, 255]))
            .save(root.join("assets/images/plate.png"))
            .unwrap();
        fs::write(
            root.join("presets/under-ice.json"),
            r#"{"layers":[{"media_path":"assets/images/plate.png"}]}"#,
        )
        .unwrap();
        let mut lib = MediaLibrary::open(root.clone());
        let spare = lib
            .items
            .iter()
            .find(|item| item.name == "spare.png")
            .unwrap()
            .id
            .clone();
        let plate = lib
            .items
            .iter()
            .find(|item| item.name == "plate.png")
            .unwrap()
            .id
            .clone();
        assert!(lib.remove_item(&plate).unwrap_err().contains("under-ice"));
        assert!(root.join("assets/images/plate.png").is_file());
        let notice = lib.remove_item(&spare).unwrap();
        assert!(notice.contains("recover it at"));
        assert!(!root.join("assets/images/spare.png").exists());
        assert!(
            root.join("recovery/history/removed-media/spare.png")
                .exists()
        );
        assert!(lib.items.iter().all(|item| item.name != "spare.png"));
        let _ = fs::remove_dir_all(&root);
    }
}
