use std::sync::mpsc::{Receiver, Sender, channel};

pub enum CaptureEvent {
    Ready,
    SelectedText(String),
    Failed(String),
}

#[cfg(windows)]
#[path = "global_capture_windows.rs"]
mod platform;
#[cfg(windows)]
pub use platform::CaptureWorker;

#[cfg(not(windows))]
pub struct CaptureWorker;

#[cfg(not(windows))]
impl CaptureWorker {
    fn start(events: Sender<CaptureEvent>) -> Self {
        let _ = events.send(CaptureEvent::Failed(
            "Global capture is supported only on Windows. Type into the capture field instead."
                .into(),
        ));
        Self
    }

    pub fn stop(&mut self) {}
}

pub fn start() -> (CaptureWorker, Receiver<CaptureEvent>) {
    let (events, receiver) = channel();
    (CaptureWorker::start(events), receiver)
}
