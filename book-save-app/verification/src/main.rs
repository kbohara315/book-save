pub enum CaptureEvent {
    Ready,
    SelectedText(String),
    Failed(String),
}

mod platform {
    include!("../../host/src/capture_windows.rs");
    include!("probe.rs");
}

fn main() {
    if let Err(error) = platform::run_probe() {
        eprintln!("FAIL {error}");
        std::process::exit(1);
    }
}
