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
pub use ffi::{probe, MediaInfo, ProbeStatus};
#[cfg(target_os = "macos")]
pub use session::{phys_footprint, players_alive, NativeClip, PlayStats};
