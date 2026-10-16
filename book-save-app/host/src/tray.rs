//! Event-driven system tray lifecycle.

#[derive(Clone, Debug)]
pub enum TrayEvent {
    Open,
    Exit,
    Unavailable(String),
}

#[cfg(windows)]
#[path = "tray_windows.rs"]
mod platform;

#[cfg(windows)]
pub use platform::{TrayWorker, start};

#[cfg(not(windows))]
pub struct TrayWorker;

#[cfg(not(windows))]
impl TrayWorker {
    pub fn stop(&mut self) {}
}

#[cfg(not(windows))]
pub fn start() -> Result<
    (
        TrayWorker,
        futures::channel::mpsc::UnboundedReceiver<TrayEvent>,
    ),
    String,
> {
    Err("The system tray is available only on Windows.".into())
}
