#[cfg(not(windows))]
use futures::channel::mpsc::UnboundedSender;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};

pub enum CaptureEvent {
    Ready,
    SelectedText(String),
    Failed(String),
}

#[cfg(windows)]
#[path = "capture_windows.rs"]
mod platform;
#[cfg(windows)]
pub use platform::CaptureWorker;

#[cfg(not(windows))]
pub struct CaptureWorker;

#[cfg(not(windows))]
impl CaptureWorker {
    fn start(events: UnboundedSender<CaptureEvent>) -> Self {
        let _ = events.unbounded_send(CaptureEvent::Failed(
            "Global capture is supported only on Windows. Type into the capture field instead."
                .into(),
        ));
        Self
    }

    pub fn stop(&mut self) {}
}

pub fn start() -> (CaptureWorker, UnboundedReceiver<CaptureEvent>) {
    let (events, receiver) = unbounded();
    (CaptureWorker::start(events), receiver)
}
