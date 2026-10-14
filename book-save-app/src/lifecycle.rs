//! Per-user background lifecycle for the Windows app.
//!
//! The global keyboard hook only delivers while this process runs, so the app
//! stays alive after its window is closed: closing minimizes to the taskbar
//! instead of quitting. A second launch hands over to the running instance
//! instead of starting a second hook, and logon startup uses `--background`
//! to begin without showing a window.

#[cfg(windows)]
#[path = "lifecycle_windows.rs"]
mod platform;

#[cfg(windows)]
pub use platform::{background_launch_requested, handover_to_running_instance};

#[cfg(not(windows))]
pub fn background_launch_requested() -> bool {
    std::env::args().any(|arg| is_background_arg(&arg))
}

#[cfg(not(windows))]
pub fn handover_to_running_instance() -> bool {
    false
}

pub fn is_background_arg(arg: &str) -> bool {
    arg == "--background"
}
