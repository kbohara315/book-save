//! Resident Book Save host: system tray, global capture hook and the control
//! pipe that backs the on-demand GPUI user interface.
//!
//! The host deliberately does not link GPUI. It stays small and idle while the
//! UI process is only spawned when a capture arrives or the user asks to open
//! it. The UI is the sole writer of the capture store.

pub mod capture;
pub mod instance;
pub mod ipc;
pub mod tray;

pub use capture::{CaptureEvent, CaptureWorker};
pub use ipc::{ClientMessage, MAX_CAPTURE_BYTES, Role, ServerMessage};
pub use tray::{TrayEvent, TrayWorker};

/// True when the command line asks for a tray-only background launch.
pub fn background_launch_requested() -> bool {
    std::env::args().skip(1).any(|arg| instance::is_background_arg(&arg))
}
