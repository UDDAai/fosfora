// Phase-1 prototype: several mailbox/FFI fields are wired for upcoming app integration.
#![allow(dead_code)]

mod budget;
#[cfg(target_os = "macos")]
mod ffi;
mod mailbox;
#[cfg(target_os = "macos")]
mod session;
mod timing;

pub use budget::{CPU_FRAMES, full_rgba_bytes, staging_ram_bytes};
pub use mailbox::VideoFrame;
pub use timing::FrameStamp;

#[cfg(target_os = "macos")]
pub mod bench;
#[cfg(target_os = "macos")]
pub use ffi::{MediaInfo, ProbeStatus, probe};
#[cfg(target_os = "macos")]
pub use session::{NativeClip, PlayStats, phys_footprint, players_alive};
