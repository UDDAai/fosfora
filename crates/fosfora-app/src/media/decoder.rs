use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

#[cfg(feature = "video")]
use super::stream_playback::StreamPlayback;
use super::stream_policy::StreamPolicy;
#[cfg(feature = "video")]
use super::stream_policy::stream_reserve_bytes;
use super::types::DecodedFrame;

/// Decoded media source: either a static image or animated frames.
/// Video files are pre-decoded to Animated (same as GIF), enabling instant random access.
/// A clip too big to hold that way is `Streamed` instead.
pub enum MediaSource {
    /// Single static image.
    Static(DecodedFrame),
    /// Animated image/video: pre-decoded frames + frame delays in milliseconds.
    Animated {
        frames: Vec<DecodedFrame>,
        delays_ms: Vec<u32>,
        /// True if this was decoded from a video file (affects UI: show time, hide direction).
        #[cfg(feature = "video")]
        from_video: bool,
    },
    /// Video decoded as it plays: a few frames in RAM whatever the clip length.
    /// Forward only; a seek restarts ffmpeg, so it is not instant like `Animated`.
    #[cfg(feature = "video")]
    Streamed {
        playback: StreamedClip,
        /// Displayed size, which is also the size of every frame.
        width: u32,
        height: u32,
    },
    /// Live webcam feed — frames arrive from capture thread, not stored here.
    #[cfg(feature = "webcam")]
    Live { width: u32, height: u32 },
}

impl MediaSource {
    pub fn frame_count(&self) -> usize {
        match self {
            MediaSource::Static(_) => 1,
            MediaSource::Animated { frames, .. } => frames.len(),
            #[cfg(feature = "video")]
            MediaSource::Streamed { playback, .. } => playback.estimated_frames() as usize,
            #[cfg(feature = "webcam")]
            MediaSource::Live { .. } => 1,
        }
    }

    pub fn is_animated(&self) -> bool {
        #[cfg(feature = "video")]
        if matches!(self, MediaSource::Streamed { .. }) {
            return true;
        }
        matches!(self, MediaSource::Animated { .. })
    }

    pub fn is_video(&self) -> bool {
        #[cfg(feature = "video")]
        match self {
            MediaSource::Animated { from_video, .. } => return *from_video,
            MediaSource::Streamed { .. } => return true,
            _ => {}
        }
        false
    }

    pub fn is_live(&self) -> bool {
        #[cfg(feature = "webcam")]
        if let MediaSource::Live { .. } = self {
            return true;
        }
        false
    }

    /// First stored frame. Live and streamed sources have none to lend: a streamed
    /// clip's frame belongs to its playback.
    pub fn first_frame(&self) -> Option<&super::types::DecodedFrame> {
        match self {
            MediaSource::Static(frame) => Some(frame),
            MediaSource::Animated { frames, .. } => frames.first(),
            #[cfg(feature = "video")]
            MediaSource::Streamed { .. } => None,
            #[cfg(feature = "webcam")]
            MediaSource::Live { .. } => None,
        }
    }

    /// Get frame dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            MediaSource::Static(f) => (f.width, f.height),
            MediaSource::Animated { frames, .. } => {
                frames.first().map_or((1, 1), |f| (f.width, f.height))
            }
            #[cfg(feature = "video")]
            MediaSource::Streamed { width, height, .. } => (*width, *height),
            #[cfg(feature = "webcam")]
            MediaSource::Live { width, height } => (*width, *height),
        }
    }

    /// RAM this source holds, as the decode budget counts it. Frames held in memory cost
    /// one RGBA frame each; a streamed clip costs a fixed [`stream_reserve_bytes`],
    /// the same amount admission reserved for it.
    pub fn resident_bytes(&self) -> u64 {
        #[cfg(feature = "video")]
        if let MediaSource::Streamed { width, height, .. } = self {
            return stream_reserve_bytes(*width, *height);
        }
        let (width, height) = self.dimensions();
        u64::from(width)
            .saturating_mul(u64::from(height))
            .saturating_mul(4)
            .saturating_mul(self.frame_count() as u64)
    }
}

/// A [`StreamPlayback`] that is never torn down on the thread that drops it.
///
/// Dropping a playback joins its decoder worker and kills ffmpeg, which takes
/// milliseconds: too long for the render thread, which drops a layer's media when the
/// layer is removed or replaced. So dropping a clip hands the playback to one
/// background reaper thread (`stream-reaper`) and returns at once. If the reaper is
/// gone, as at shutdown, the playback is dropped in place.
#[cfg(feature = "video")]
pub struct StreamedClip(Option<Box<StreamPlayback>>);

/// Streamed clips alive: opened and not yet fully torn down. Zero when every clip
/// has been dropped and reaped.
#[cfg(feature = "video")]
static LIVE_STREAMS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many streamed clips are alive, reaper queue included.
#[cfg(feature = "video")]
pub fn live_streams() -> usize {
    LIVE_STREAMS.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(feature = "video")]
fn reaper() -> Option<&'static crossbeam_channel::Sender<Box<StreamPlayback>>> {
    static REAPER: std::sync::OnceLock<Option<crossbeam_channel::Sender<Box<StreamPlayback>>>> =
        std::sync::OnceLock::new();
    REAPER
        .get_or_init(|| {
            let (tx, rx) = crossbeam_channel::unbounded::<Box<StreamPlayback>>();
            std::thread::Builder::new()
                .name("stream-reaper".into())
                .spawn(move || {
                    for playback in rx {
                        drop(playback);
                        LIVE_STREAMS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                    }
                })
                .ok()?;
            Some(tx)
        })
        .as_ref()
}

#[cfg(feature = "video")]
impl StreamedClip {
    pub(crate) fn new(playback: StreamPlayback) -> Self {
        LIVE_STREAMS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self(Some(Box::new(playback)))
    }
}

#[cfg(feature = "video")]
impl std::ops::Deref for StreamedClip {
    type Target = StreamPlayback;
    fn deref(&self) -> &StreamPlayback {
        self.0.as_deref().expect("playback is present until drop")
    }
}

#[cfg(feature = "video")]
impl std::ops::DerefMut for StreamedClip {
    fn deref_mut(&mut self) -> &mut StreamPlayback {
        self.0
            .as_deref_mut()
            .expect("playback is present until drop")
    }
}

#[cfg(feature = "video")]
impl Drop for StreamedClip {
    fn drop(&mut self) {
        let Some(playback) = self.0.take() else {
            return;
        };
        let playback = match reaper() {
            Some(tx) => match tx.send(playback) {
                Ok(()) => return,
                Err(failed) => failed.0,
            },
            None => playback,
        };
        drop_now(playback);
    }
}

#[cfg(feature = "video")]
fn drop_now(playback: Box<StreamPlayback>) {
    drop(playback);
    LIVE_STREAMS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
}

/// Video file extensions.
#[cfg(feature = "video")]
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "avi", "mkv", "webm", "m4v", "flv"];

/// Load an image or animation from a file path.
pub fn load_media(path: &Path) -> Result<MediaSource, String> {
    load_media_with(path, StreamPolicy::from_env())
}

/// [`load_media`] with the streaming policy given, not read from the environment.
/// A streamed video blocks up to three seconds for its first frame, so call this from
/// a loader thread, never the render thread.
pub fn load_media_with(path: &Path, policy: StreamPolicy) -> Result<MediaSource, String> {
    #[cfg(not(feature = "video"))]
    let _ = policy;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    #[cfg(feature = "video")]
    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        return load_video(path, policy);
    }

    match ext.as_str() {
        "gif" => load_gif(path),
        "webp" => load_webp(path),
        _ => load_static_image(path),
    }
}

/// Load a video file: pre-decode all frames via ffmpeg, or stream it when the
/// [`StreamPolicy`] says the clip is too big or long for that.
#[cfg(feature = "video")]
fn load_video(path: &Path, policy: StreamPolicy) -> Result<MediaSource, String> {
    use super::library::max_clip_ram;
    use super::stream_policy::{describe_decision, should_stream};
    use super::video::{
        MAX_PREDECODE_SECS, decode_all_frames, decode_allowance, ffmpeg_available, probe_video,
    };

    if !ffmpeg_available() {
        return Err(format!(
            "ffmpeg/ffprobe not found ({})",
            super::toolchain::VideoToolchain::global(None).status_line()
        ));
    }

    let meta = probe_video(path)?;
    log::info!(
        "Video probe: {}x{}, {:.2} fps, {:.1}s",
        meta.width,
        meta.height,
        meta.fps,
        meta.duration_secs,
    );

    // The same inputs admission (`library::plan_video`) used to reserve.
    let all_frames = decode_allowance(meta.width, meta.height, meta.frame_count())?.ram_bytes;
    let cap = max_clip_ram();
    if should_stream(policy, meta.duration_secs, all_frames, cap) {
        let why = describe_decision(
            policy,
            meta.duration_secs,
            meta.frame_count(),
            all_frames,
            cap,
        );
        log::info!("{}: {}", path.display(), why.unwrap_or_default());
        return open_streamed(path, &meta);
    }

    if meta.duration_secs > MAX_PREDECODE_SECS {
        return Err(format!(
            "Video too long for pre-decode ({:.0}s > {:.0}s max). \
             Use a shorter clip or trim with ffmpeg.",
            meta.duration_secs, MAX_PREDECODE_SECS,
        ));
    }

    let (frames, delays_ms) = decode_all_frames(path, &meta)?;
    Ok(MediaSource::Animated {
        frames,
        delays_ms,
        from_video: true,
    })
}

/// Start ffmpeg on the clip and wait (at most three seconds) for its first frame, so
/// call it from a loader thread. Frames come at the clip's own rate, at the displayed
/// size, and the layer starts looping like any other media layer.
#[cfg(feature = "video")]
fn open_streamed(path: &Path, meta: &super::video::VideoMeta) -> Result<MediaSource, String> {
    use super::stream_ffmpeg::{StreamSpec, stream_producer};

    let (width, height) = (meta.display_width, meta.display_height);
    let spec = StreamSpec {
        source_fps: Some(meta.fps),
        ..StreamSpec::new(meta.fps, width, height)
    };
    let playback = StreamPlayback::open(
        width,
        height,
        meta.fps,
        meta.duration_secs,
        true,
        stream_producer(path.to_path_buf(), spec),
    )?;
    Ok(MediaSource::Streamed {
        playback: StreamedClip::new(playback),
        width,
        height,
    })
}

/// Load a static image (PNG, JPEG, etc.) via the `image` crate.
fn load_static_image(path: &Path) -> Result<MediaSource, String> {
    let img = image::open(path).map_err(|e| format!("Failed to open image: {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();

    Ok(MediaSource::Static(DecodedFrame {
        data: rgba.into_raw(),
        width: w,
        height: h,
    }))
}

/// Canvas size and how many RGBA frames [`load_gif`] / [`load_webp`] will store.
pub(crate) struct StoredImage {
    pub width: u32,
    pub height: u32,
    pub frames: u64,
}

/// Frame count for admission. Does not expand pixels.
///
/// WebP reads `num_frames()` from the ANMF headers while opening the decoder.
/// GIF has no count in the header; image separators are counted and LZW
/// sub-blocks are skipped. [`load_gif`] stores one full canvas per separator,
/// and [`load_webp`] stores one RGBA buffer per counted frame. A count that
/// cannot be read is an error, so planning does not fall back to one frame.
pub(crate) fn stored_image(path: &Path) -> Result<StoredImage, String> {
    match file_extension(path).as_str() {
        "gif" => gif_storage(path),
        "webp" => webp_storage(path),
        _ => {
            let (width, height) = image::image_dimensions(path).map_err(|err| err.to_string())?;
            Ok(StoredImage {
                width,
                height,
                frames: 1,
            })
        }
    }
}

pub(crate) fn stored_rgba_bytes(image: &StoredImage) -> u64 {
    u64::from(image.width)
        .saturating_mul(u64::from(image.height))
        .saturating_mul(4)
        .saturating_mul(image.frames)
}

fn file_extension(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn gif_storage(path: &Path) -> Result<StoredImage, String> {
    let mut file = File::open(path).map_err(|err| format!("Failed to open GIF: {err}"))?;
    read_gif_storage(&mut file)
}

fn read_gif_storage<R: Read>(reader: &mut R) -> Result<StoredImage, String> {
    let mut header = [0u8; 13];
    reader
        .read_exact(&mut header)
        .map_err(|_| gif_count_unknown())?;
    if &header[..6] != b"GIF87a" && &header[..6] != b"GIF89a" {
        return Err(gif_count_unknown());
    }
    let width = u32::from(u16::from_le_bytes([header[6], header[7]]));
    let height = u32::from(u16::from_le_bytes([header[8], header[9]]));
    if width == 0 || height == 0 {
        return Err(gif_count_unknown());
    }
    if header[10] & 0x80 != 0 {
        skip_bytes(reader, color_table_bytes(header[10])?)?;
    }
    let mut frames = 0u64;
    loop {
        let mut marker = [0u8; 1];
        reader
            .read_exact(&mut marker)
            .map_err(|_| gif_count_unknown())?;
        match marker[0] {
            0x3B => {
                if frames == 0 {
                    return Err("GIF has no frames".to_string());
                }
                return Ok(StoredImage {
                    width,
                    height,
                    frames,
                });
            }
            0x2C => {
                frames = frames.saturating_add(1);
                let mut desc = [0u8; 9];
                reader
                    .read_exact(&mut desc)
                    .map_err(|_| gif_count_unknown())?;
                if desc[8] & 0x80 != 0 {
                    skip_bytes(reader, color_table_bytes(desc[8])?)?;
                }
                // LZW minimum code size, then compressed sub-blocks. Not decoded.
                skip_bytes(reader, 1)?;
                skip_sub_blocks(reader)?;
            }
            0x21 => {
                skip_bytes(reader, 1)?;
                skip_sub_blocks(reader)?;
            }
            _ => return Err(gif_count_unknown()),
        }
    }
}

fn gif_count_unknown() -> String {
    "GIF frame count cannot be known".to_string()
}

fn color_table_bytes(packed: u8) -> Result<usize, String> {
    3usize
        .checked_shl(u32::from(packed & 0x07) + 1)
        .ok_or_else(gif_count_unknown)
}

fn skip_sub_blocks<R: Read>(reader: &mut R) -> Result<(), String> {
    loop {
        let mut len = [0u8; 1];
        reader
            .read_exact(&mut len)
            .map_err(|_| gif_count_unknown())?;
        if len[0] == 0 {
            return Ok(());
        }
        skip_bytes(reader, usize::from(len[0]))?;
    }
}

fn skip_bytes<R: Read>(reader: &mut R, mut remaining: usize) -> Result<(), String> {
    let mut buf = [0u8; 256];
    while remaining > 0 {
        let n = remaining.min(buf.len());
        reader
            .read_exact(&mut buf[..n])
            .map_err(|_| gif_count_unknown())?;
        remaining -= n;
    }
    Ok(())
}

fn webp_storage(path: &Path) -> Result<StoredImage, String> {
    use image_webp::WebPDecoder;

    let file = File::open(path).map_err(|err| format!("Failed to open WebP: {err}"))?;
    // Counts ANMF chunks while seeking. `read_frame` is what expands pixels.
    let decoder = WebPDecoder::new(BufReader::new(file))
        .map_err(|err| format!("WebP frame count cannot be known: {err}"))?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err("WebP frame count cannot be known".to_string());
    }
    let frames = if decoder.is_animated() {
        let count = decoder.num_frames();
        if count == 0 {
            return Err("WebP frame count cannot be known".to_string());
        }
        u64::from(count)
    } else {
        1
    };
    Ok(StoredImage {
        width,
        height,
        frames,
    })
}

/// Load an animated GIF, pre-decoding all frames.
fn load_gif(path: &Path) -> Result<MediaSource, String> {
    use std::fs::File;

    let file = File::open(path).map_err(|e| format!("Failed to open GIF: {e}"))?;
    let mut decoder = gif::DecodeOptions::new();
    decoder.set_color_output(gif::ColorOutput::RGBA);
    let mut reader = decoder
        .read_info(file)
        .map_err(|e| format!("Failed to decode GIF: {e}"))?;

    let width = reader.width() as u32;
    let height = reader.height() as u32;

    let mut frames = Vec::new();
    let mut delays_ms = Vec::new();

    // Accumulator for compositing (GIF frames can be partial updates)
    let mut canvas = vec![0u8; (width * height * 4) as usize];

    while let Some(frame) = reader
        .read_next_frame()
        .map_err(|e| format!("GIF frame error: {e}"))?
    {
        let delay = frame.delay as u32 * 10; // GIF delay is in centiseconds
        delays_ms.push(delay.max(20)); // minimum 20ms to prevent zero-delay

        // Composite frame onto canvas at the correct offset
        let fx = frame.left as u32;
        let fy = frame.top as u32;
        let fw = frame.width as u32;
        let fh = frame.height as u32;

        for y in 0..fh {
            for x in 0..fw {
                let src_idx = ((y * fw + x) * 4) as usize;
                let dst_x = fx + x;
                let dst_y = fy + y;
                if dst_x < width && dst_y < height {
                    let dst_idx = ((dst_y * width + dst_x) * 4) as usize;
                    let src = &frame.buffer[src_idx..src_idx + 4];
                    // Only overwrite if source pixel is not fully transparent
                    if src[3] > 0 {
                        canvas[dst_idx..dst_idx + 4].copy_from_slice(src);
                    }
                }
            }
        }

        frames.push(DecodedFrame {
            data: canvas.clone(),
            width,
            height,
        });
    }

    if frames.is_empty() {
        return Err("GIF has no frames".to_string());
    }

    log::info!("Loaded GIF: {}x{}, {} frames", width, height, frames.len());

    Ok(MediaSource::Animated {
        frames,
        delays_ms,
        #[cfg(feature = "video")]
        from_video: false,
    })
}

/// Load a WebP image, detecting animation automatically.
fn load_webp(path: &Path) -> Result<MediaSource, String> {
    use image_webp::WebPDecoder;
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path).map_err(|e| format!("Failed to open WebP: {e}"))?;
    let mut decoder = WebPDecoder::new(BufReader::new(file))
        .map_err(|e| format!("Failed to decode WebP: {e}"))?;

    let (width, height) = decoder.dimensions();

    if !decoder.is_animated() {
        // Static WebP — decode single frame
        let buf_size = decoder
            .output_buffer_size()
            .ok_or("Cannot determine WebP buffer size")?;
        let mut buf = vec![0u8; buf_size];
        decoder
            .read_image(&mut buf)
            .map_err(|e| format!("Failed to read WebP image: {e}"))?;

        // Ensure RGBA (WebP without alpha returns RGB)
        let data = if decoder.has_alpha() {
            buf
        } else {
            rgb_to_rgba(&buf)
        };

        return Ok(MediaSource::Static(DecodedFrame {
            data,
            width,
            height,
        }));
    }

    // Animated WebP — decode all frames
    let num_frames = decoder.num_frames() as usize;
    let mut frames = Vec::with_capacity(num_frames);
    let mut delays_ms = Vec::with_capacity(num_frames);
    let buf_size = decoder
        .output_buffer_size()
        .ok_or("Cannot determine WebP buffer size")?;

    loop {
        let mut buf = vec![0u8; buf_size];
        match decoder.read_frame(&mut buf) {
            Ok(duration_ms) => {
                delays_ms.push(duration_ms.max(20)); // minimum 20ms

                let data = if decoder.has_alpha() {
                    buf
                } else {
                    rgb_to_rgba(&buf)
                };

                frames.push(DecodedFrame {
                    data,
                    width,
                    height,
                });
            }
            Err(_) => break, // NoMoreFrames
        }
    }

    if frames.is_empty() {
        return Err("Animated WebP has no frames".to_string());
    }

    log::info!(
        "Loaded animated WebP: {}x{}, {} frames",
        width,
        height,
        frames.len()
    );

    Ok(MediaSource::Animated {
        frames,
        delays_ms,
        #[cfg(feature = "video")]
        from_video: false,
    })
}

/// Convert RGB buffer to RGBA (opaque).
fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    let pixel_count = rgb.len() / 3;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for chunk in rgb.chunks_exact(3) {
        rgba.extend_from_slice(chunk);
        rgba.push(255);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32) -> DecodedFrame {
        DecodedFrame {
            data: vec![0; width as usize * height as usize * 4],
            width,
            height,
        }
    }

    #[test]
    fn frames_in_memory_cost_one_rgba_frame_each() {
        let still = MediaSource::Static(frame(5, 3));
        assert_eq!(still.resident_bytes(), 5 * 3 * 4);
        let clip = MediaSource::Animated {
            frames: vec![frame(4, 2), frame(4, 2), frame(4, 2)],
            delays_ms: vec![40; 3],
            #[cfg(feature = "video")]
            from_video: true,
        };
        // The formula the budget used before streaming: width x height x 4 x frames.
        assert_eq!(clip.resident_bytes(), 4 * 2 * 4 * 3);
    }

    /// Resident set of a process, in MB, from `ps` (macOS and Linux).
    #[cfg(feature = "video")]
    fn rss_mb(pid: u32) -> f64 {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .expect("ps");
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<f64>()
            .unwrap_or(0.0)
            / 1024.0
    }

    /// Plays a real clip through the same path a media layer uses, at 60 Hz for
    /// `FOSFORA_STREAM_SECS` (default 20) of wall time, and prints one `STREAMCHECK`
    /// line: first-frame time, frames shown against frames expected, drops, seeks and
    /// peak RSS of this process and of its ffmpeg children. No GPU and no window.
    ///
    /// ```text
    /// FOSFORA_STREAM_CLIP=/path/clip.mp4 nice -n 19 cargo test -p fosfora-app \
    ///   --features video streams_a_real_clip -- --ignored --nocapture
    /// ```
    #[cfg(feature = "video")]
    #[test]
    #[ignore = "needs ffmpeg and a clip: set FOSFORA_STREAM_CLIP"]
    fn streams_a_real_clip() {
        use super::super::types::TransportState;
        use std::time::{Duration, Instant};

        let path = std::path::PathBuf::from(
            std::env::var_os("FOSFORA_STREAM_CLIP").expect("set FOSFORA_STREAM_CLIP"),
        );
        let secs: f64 = std::env::var("FOSFORA_STREAM_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20.0);

        let opened = Instant::now();
        let source = load_media_with(&path, StreamPolicy::Auto).expect("load_media");
        let load_ms = opened.elapsed().as_secs_f64() * 1000.0;
        let MediaSource::Streamed {
            mut playback,
            width,
            height,
        } = source
        else {
            panic!("{} did not stream", path.display());
        };
        let fps = playback.estimated_frames() as f64 / playback.duration_secs();
        let reserve = source_reserve(width, height);

        let transport = TransportState::default(); // playing, looping, 1x
        let pid = std::process::id();
        let children_mb = || -> f64 {
            let out = std::process::Command::new("pgrep")
                .args(["-P", &pid.to_string()])
                .output()
                .expect("pgrep");
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .filter_map(|c| c.parse().ok())
                .map(rss_mb)
                .sum()
        };
        let (mut peak_self, mut peak_children) = (rss_mb(pid), children_mb());

        let frame_time = Duration::from_secs_f64(1.0 / 60.0);
        let begin = Instant::now();
        let (mut next, mut last) = (begin, begin);
        let (mut shown, mut ticks, mut wraps) = (0u64, 0u64, 0u64);
        let mut last_pos = 0.0;
        let mut staging = vec![0u8; width as usize * height as usize * 4];
        while begin.elapsed().as_secs_f64() < secs {
            next += frame_time;
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
            let now = Instant::now();
            let dt = now.duration_since(last).as_secs_f32().min(0.05);
            last = now;
            if playback.tick(dt, &transport) {
                shown += 1;
                staging.copy_from_slice(playback.current_frame()); // the upload's CPU cost
            }
            let pos = playback.position_secs();
            wraps += u64::from(pos + 1.0 < last_pos);
            last_pos = pos;
            ticks += 1;
            if ticks % 60 == 0 {
                peak_self = peak_self.max(rss_mb(pid));
                peak_children = peak_children.max(children_mb());
            }
        }
        let wall = begin.elapsed().as_secs_f64();
        let expected = wall * fps;
        let stats = playback.stats();
        println!(
            "STREAMCHECK clip={} size={width}x{height} fps={fps:.2} dur={:.1}s load_ms={load_ms:.0} \
             first_frame_ms={:.0} shown={shown} expected={expected:.0} pct={:.1} dropped={} \
             seeks={} restarts={} wraps={wraps} ticks={ticks} rss_self_peak_mb={peak_self:.0} \
             rss_ffmpeg_peak_mb={peak_children:.0} reserve_mb={:.0} error={:?}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            playback.duration_secs(),
            stats.first_frame_ms,
            shown as f64 / expected * 100.0,
            stats.dropped_frames,
            stats.seeks,
            stats.restarts,
            reserve as f64 / (1024.0 * 1024.0),
            playback.last_error(),
        );
        assert_eq!(staging.len(), width as usize * height as usize * 4);
    }

    #[cfg(feature = "video")]
    fn source_reserve(width: u32, height: u32) -> u64 {
        super::stream_reserve_bytes(width, height)
    }
}
