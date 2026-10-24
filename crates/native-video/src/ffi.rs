//! Safe wrappers around the single-threaded AVFoundation player.
//!
//! `Player` is created and used on the playback thread only. It is deliberately
//! not `Send`.

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use crate::budget::frame_bytes;

#[repr(C)]
struct NvMediaInfo {
    width: u32,
    height: u32,
    fps: f64,
    duration_secs: f64,
    hardware: i32,
    reserved: i32,
    codec: [u8; 8],
    note: [u8; 256],
}

#[repr(C)]
struct NvFrameInfo {
    pts_secs: f64,
    item_secs: f64,
    width: u32,
    height: u32,
    rgba_bytes: u32,
    held_after: i32,
}

unsafe extern "C" {
    fn nv_media_info_size() -> usize;
    fn nv_frame_info_size() -> usize;
    fn nv_phys_footprint() -> u64;
    fn nv_probe(
        path: *const c_char,
        info: *mut NvMediaInfo,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn nv_open(
        path: *const c_char,
        info: *mut NvMediaInfo,
        err: *mut c_char,
        err_len: usize,
    ) -> *mut c_void;
    fn nv_close(player: *mut c_void);
    fn nv_set_rate(player: *mut c_void, rate: f64);
    fn nv_seek(player: *mut c_void, secs: f64);
    fn nv_pump(player: *mut c_void, seconds: f64);
    fn nv_position(player: *mut c_void) -> f64;
    fn nv_copy_rgba(
        player: *mut c_void,
        dst: *mut u8,
        dst_cap: usize,
        info: *mut NvFrameInfo,
    ) -> c_int;
    fn nv_last_error(player: *mut c_void, err: *mut c_char, err_len: usize);
    fn nv_metal_sample(
        player: *mut c_void,
        rgba_out: *mut u8,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_secs: f64,
    pub hardware: i32,
    pub codec: String,
    pub note: String,
}

#[derive(Debug, Clone, Copy)]
pub struct CopiedFrame {
    pub pts_secs: f64,
    pub item_secs: f64,
    pub width: u32,
    pub height: u32,
    pub rgba_bytes: u32,
    pub held_after: i32,
}

pub enum ProbeStatus {
    Playable(MediaInfo),
    Unsupported(String),
}

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "path contains an interior nul".to_string())
}

fn read_buf(buf: &[u8]) -> String {
    let end = buf.iter().position(|byte| *byte == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

fn info_from(raw: &NvMediaInfo) -> MediaInfo {
    MediaInfo {
        width: raw.width,
        height: raw.height,
        fps: raw.fps,
        duration_secs: raw.duration_secs,
        hardware: raw.hardware,
        codec: read_buf(&raw.codec),
        note: read_buf(&raw.note),
    }
}

pub fn footprint() -> u64 {
    // SAFETY: nv_phys_footprint only reads the current task's vm info.
    unsafe { nv_phys_footprint() }
}

pub fn probe(path: &Path) -> Result<ProbeStatus, String> {
    let path = c_path(path)?;
    let mut info = NvMediaInfo {
        width: 0,
        height: 0,
        fps: 0.0,
        duration_secs: 0.0,
        hardware: -1,
        reserved: 0,
        codec: [0; 8],
        note: [0; 256],
    };
    let mut err = [0u8; 512];
    // SAFETY: path is a C string, info and err are writable buffers of the
    // sizes the C side expects, and nv_probe does not retain them.
    let code = unsafe { nv_probe(path.as_ptr(), &mut info, err.as_mut_ptr().cast(), err.len()) };
    let message = read_buf(&err);
    match code {
        0 => Ok(ProbeStatus::Playable(info_from(&info))),
        1 => Ok(ProbeStatus::Unsupported(if message.is_empty() {
            "AVFoundation cannot play this file".to_string()
        } else {
            message
        })),
        _ => Err(if message.is_empty() {
            "could not inspect the video".to_string()
        } else {
            message
        }),
    }
}

pub struct Player {
    raw: *mut c_void,
}

impl Player {
    pub fn open(path: &Path) -> Result<(Self, MediaInfo), String> {
        let path = c_path(path)?;
        let mut info = NvMediaInfo {
            width: 0,
            height: 0,
            fps: 0.0,
            duration_secs: 0.0,
            hardware: -1,
            reserved: 0,
            codec: [0; 8],
            note: [0; 256],
        };
        let mut err = [0u8; 512];
        // SAFETY: nv_open returns a +1 player or NULL. The C string and
        // output buffers live for the call. The player is closed in Drop.
        let raw = unsafe { nv_open(path.as_ptr(), &mut info, err.as_mut_ptr().cast(), err.len()) };
        if raw.is_null() {
            let message = read_buf(&err);
            return Err(if message.is_empty() {
                "could not open the video".to_string()
            } else {
                message
            });
        }
        Ok((Self { raw }, info_from(&info)))
    }

    pub fn set_rate(&self, rate: f64) {
        // SAFETY: `raw` is the live player owned by this thread.
        unsafe { nv_set_rate(self.raw, rate) }
    }

    pub fn seek(&self, secs: f64) {
        // SAFETY: `raw` is the live player owned by this thread.
        unsafe { nv_seek(self.raw, secs) }
    }

    pub fn pump(&self, seconds: f64) {
        // SAFETY: pumps the current thread's run loop; `raw` is unused by C
        // except as a token that this thread owns the player.
        unsafe { nv_pump(self.raw, seconds) }
    }

    pub fn position(&self) -> f64 {
        // SAFETY: `raw` is the live player owned by this thread.
        unsafe { nv_position(self.raw) }
    }

    pub fn copy_rgba(&self, dst: &mut [u8]) -> Result<Option<CopiedFrame>, String> {
        let mut info = NvFrameInfo {
            pts_secs: 0.0,
            item_secs: 0.0,
            width: 0,
            height: 0,
            rgba_bytes: 0,
            held_after: 0,
        };
        // SAFETY: dst is writable for dst.len() bytes. The C side writes at
        // most one tightly packed frame and releases the pixel buffer before
        // returning.
        let code = unsafe { nv_copy_rgba(self.raw, dst.as_mut_ptr(), dst.len(), &mut info) };
        match code {
            0 => Ok(None),
            1 => {
                let bytes = frame_bytes(info.width, info.height).unwrap_or(0);
                if u64::from(info.rgba_bytes) != bytes || info.held_after != 0 {
                    return Err(format!(
                        "copied frame {}x{} retained {} pixel buffers",
                        info.width, info.height, info.held_after
                    ));
                }
                Ok(Some(CopiedFrame {
                    pts_secs: info.pts_secs,
                    item_secs: info.item_secs,
                    width: info.width,
                    height: info.height,
                    rgba_bytes: info.rgba_bytes,
                    held_after: info.held_after,
                }))
            }
            _ => Err(self.last_error()),
        }
    }

    pub fn metal_sample(&self) -> Result<[u8; 4], String> {
        let mut pixel = [0u8; 4];
        let mut err = [0u8; 512];
        // SAFETY: pixel and err are caller-owned buffers. The C side releases
        // the pixel buffer and flushes the texture cache before returning.
        let code = unsafe {
            nv_metal_sample(
                self.raw,
                pixel.as_mut_ptr(),
                err.as_mut_ptr().cast(),
                err.len(),
            )
        };
        match code {
            0 => Ok(pixel),
            -2 => Err(read_buf(&err).if_empty("no Metal device")),
            _ => Err(read_buf(&err).if_empty("Metal preview failed")),
        }
    }

    fn last_error(&self) -> String {
        let mut err = [0u8; 512];
        // SAFETY: err is a writable buffer. The player is still alive.
        unsafe { nv_last_error(self.raw, err.as_mut_ptr().cast(), err.len()) }
        let message = read_buf(&err);
        if message.is_empty() {
            "playback failed".to_string()
        } else {
            message
        }
    }
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: this is the unique +1 reference from nv_open, closed once
            // on the thread that created it.
            unsafe { nv_close(self.raw) }
            self.raw = ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_layouts_match_the_c_headers() {
        // SAFETY: the size helpers have no inputs and return sizeof.
        let (media, frame) = unsafe { (nv_media_info_size(), nv_frame_info_size()) };
        assert_eq!(media, std::mem::size_of::<NvMediaInfo>());
        assert_eq!(frame, std::mem::size_of::<NvFrameInfo>());
    }
}
