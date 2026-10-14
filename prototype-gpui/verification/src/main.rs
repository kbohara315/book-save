use std::sync::mpsc::Sender;

pub enum CaptureEvent {
    Ready,
    SelectedText(String),
    Failed(String),
}

mod platform {
    include!("../../src/global_capture_windows.rs");
    include!("probe.rs");
}

fn main() {
    if let Err(error) = platform::run_probe() {
        eprintln!("FAIL {error}");
        std::process::exit(1);
    }
}
