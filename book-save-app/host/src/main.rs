#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! Resident host entry point: owns the tray, the global capture hook and the
//! single-instance control pipe, and spawns the GPUI UI on demand.
//!
//! The host stays in the tray. A normal launch opens the UI once; a
//! `--background` launch (autostart) stays in the tray until the user opens it
//! or a capture arrives. Closing the UI leaves the host running; only the tray
//! Exit item stops the host.

use std::{
    process::ExitCode,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use book_save_host::{
    CaptureEvent, ClientMessage, ServerMessage, TrayEvent, capture, instance, ipc, tray,
};

static NEXT_UI_ID: AtomicU64 = AtomicU64::new(1);

fn next_ui_id() -> u64 {
    NEXT_UI_ID.fetch_add(1, Ordering::Relaxed)
}

fn main() -> ExitCode {
    match instance::acquire_startup() {
        instance::Startup::AlreadyRunning => {
            if !book_save_host::background_launch_requested() {
                ipc::request_open();
            }
            ExitCode::SUCCESS
        }
        instance::Startup::First(_guard) => {
            run();
            ExitCode::SUCCESS
        }
    }
}

#[cfg(not(windows))]
fn run() {
    log("Book Save host is supported only on Windows.");
}

#[cfg(windows)]
fn run() {
    let (events, receiver) = mpsc::channel::<Control>();

    let (mut tray_worker, tray_available, tray_error) = match tray::start() {
        Ok((worker, tray_events)) => {
            bridge_tray(tray_events, events.clone());
            (Some(worker), true, None)
        }
        Err(error) => (None, false, Some(error)),
    };

    let (mut capture_worker, capture_events) = capture::start();
    bridge_capture(capture_events, events.clone());

    if !tray_available {
        log(&format!(
            "tray unavailable: {}",
            tray_error.as_deref().unwrap_or("unknown error")
        ));
    }

    spawn_acceptor(events.clone());

    let mut host = Host::new(events, receiver, tray_available, tray_error);

    if !tray_available {
        // Without a tray the host has no other way to be reached, so surface
        // the failure by opening the window; closing it then stops the host.
        host.request_open();
    } else if !book_save_host::background_launch_requested() {
        host.request_open();
    }

    host.run();

    capture_worker.stop();
    if let Some(worker) = tray_worker.as_mut() {
        worker.stop();
    }
}

fn bridge_capture(
    receiver: futures::channel::mpsc::UnboundedReceiver<CaptureEvent>,
    tx: mpsc::Sender<Control>,
) {
    use futures::StreamExt;
    let mut receiver = receiver;
    thread::spawn(move || {
        while let Some(event) = futures::executor::block_on(receiver.next()) {
            if tx.send(Control::Capture(event)).is_err() {
                break;
            }
        }
    });
}

fn bridge_tray(
    receiver: futures::channel::mpsc::UnboundedReceiver<TrayEvent>,
    tx: mpsc::Sender<Control>,
) {
    use futures::StreamExt;
    let mut receiver = receiver;
    thread::spawn(move || {
        while let Some(event) = futures::executor::block_on(receiver.next()) {
            if tx.send(Control::Tray(event)).is_err() {
                break;
            }
        }
    });
}

#[cfg(windows)]
fn spawn_acceptor(tx: mpsc::Sender<Control>) {
    use std::sync::Arc;

    thread::spawn(move || {
        let listener = ipc::PipeListener::new();
        loop {
            match listener.accept() {
                Ok(connection) => {
                    let id = next_ui_id();
                    spawn_reader(id, Arc::new(connection), tx.clone());
                }
                Err(error) => {
                    log(&format!("pipe accept failed: {error}"));
                    thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        }
    });
}

#[cfg(not(windows))]
fn spawn_acceptor(_tx: mpsc::Sender<Control>) {}

#[cfg(windows)]
fn spawn_reader(id: u64, connection: std::sync::Arc<ipc::ServerConnection>, tx: mpsc::Sender<Control>) {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let first_message = connection.read(&mut buffer);
        log(&format!("first IPC message from client {id}: {first_message:?}"));
        let ui_pid = match first_message {
            Ok(Some(ClientMessage::Hello { pid, .. })) => Some(pid),
            Ok(Some(ClientMessage::EnsureOpen)) => {
                let _ = tx.send(Control::Ipc(HostEvent::EnsureOpen));
                None
            }
            #[cfg(debug_assertions)]
            Ok(Some(ClientMessage::InjectCapture { text })) => {
                log(&format!("debug capture injection received ({} chars)", text.len()));
                let _ = tx.send(Control::Ipc(HostEvent::InjectCapture(text)));
                None
            }
            other => {
                let _ = tx.send(Control::Ipc(HostEvent::ClientError {
                    id,
                    message: format!("expected hello handshake, received {other:?}"),
                }));
                return;
            }
        };

        if let Some(pid) = ui_pid {
            if tx
                .send(Control::Ipc(HostEvent::UiConnected {
                    id,
                    connection: connection.clone(),
                    pid,
                }))
                .is_err()
            {
                return;
            }
        }

        loop {
            match connection.read(&mut buffer) {
                Ok(Some(ClientMessage::Ready)) => {
                    let _ = tx.send(Control::Ipc(HostEvent::UiReady { id }));
                }
                Ok(Some(ClientMessage::UiClosing)) => {
                    let _ = tx.send(Control::Ipc(HostEvent::UiClosing { id }));
                }
                Ok(Some(ClientMessage::EnsureOpen)) => {
                    let _ = tx.send(Control::Ipc(HostEvent::EnsureOpen));
                }
                #[cfg(debug_assertions)]
                Ok(Some(ClientMessage::InjectCapture { text })) => {
                    let _ = tx.send(Control::Ipc(HostEvent::InjectCapture(text)));
                }
                Ok(Some(ClientMessage::Ping)) => {
                    let _ = connection.send(&ServerMessage::Pong);
                }
                Ok(Some(ClientMessage::Hello { .. })) => {}
                Ok(None) => break,
                Err(error) => {
                    let _ = tx.send(Control::Ipc(HostEvent::ClientError {
                        id,
                        message: error.to_string(),
                    }));
                    break;
                }
            }
        }

        if ui_pid.is_some() {
            let _ = tx.send(Control::Ipc(HostEvent::UiDisconnected { id }));
        }
    });
}

enum Control {
    Capture(CaptureEvent),
    Tray(TrayEvent),
    Ipc(HostEvent),
    SpawnTimeout(u64),
    ShutdownTimeout(u64),
}

enum HostEvent {
    UiConnected {
        id: u64,
        connection: std::sync::Arc<ipc::ServerConnection>,
        pid: u32,
    },
    UiReady {
        id: u64,
    },
    UiClosing {
        id: u64,
    },
    UiDisconnected {
        id: u64,
    },
    EnsureOpen,
    #[cfg(debug_assertions)]
    InjectCapture(String),
    ClientError {
        id: u64,
        message: String,
    },
}

struct UiState {
    /// Zero while the UI process is starting and has not handshaken yet.
    id: u64,
    connection: Option<std::sync::Arc<ipc::ServerConnection>>,
    pid: Option<u32>,
    ready: bool,
}

struct Host {
    events: mpsc::Sender<Control>,
    receiver: mpsc::Receiver<Control>,
    tray_available: bool,
    tray_error: Option<String>,
    last_capture_error: Option<String>,
    ui: Option<UiState>,
    pending_captures: Vec<String>,
    pending_open: bool,
    exit_requested: bool,
    spawn_generation: u64,
}

const SPAWN_TIMEOUT_MS: u64 = 15_000;
const SHUTDOWN_TIMEOUT_MS: u64 = 4_000;

impl Host {
    fn new(
        events: mpsc::Sender<Control>,
        receiver: mpsc::Receiver<Control>,
        tray_available: bool,
        tray_error: Option<String>,
    ) -> Self {
        Self {
            events,
            receiver,
            tray_available,
            tray_error,
            last_capture_error: None,
            ui: None,
            pending_captures: Vec::new(),
            pending_open: false,
            exit_requested: false,
            spawn_generation: 0,
        }
    }

    fn run(&mut self) {
        while let Ok(control) = self.receiver.recv() {
            let stop = match control {
                Control::Capture(event) => {
                    self.on_capture(event);
                    false
                }
                Control::Tray(event) => self.on_tray(event),
                Control::Ipc(event) => self.on_ipc(event),
                Control::SpawnTimeout(generation) => {
                    self.on_spawn_timeout(generation);
                    false
                }
                Control::ShutdownTimeout(id) => {
                    self.on_shutdown_timeout(id);
                    true
                }
            };
            if stop {
                break;
            }
            if self.exit_requested && self.ui.is_none() {
                break;
            }
        }
    }

    fn on_capture(&mut self, event: CaptureEvent) {
        match event {
            CaptureEvent::Ready => {
                self.last_capture_error = None;
            }
            CaptureEvent::SelectedText(text) => self.deliver_capture(text),
            CaptureEvent::Failed(error) => {
                self.last_capture_error = Some(error.clone());
                if let Some(ui) = &self.ui {
                    if ui.ready {
                        self.send(&ServerMessage::CaptureFailed { message: error });
                    }
                } else {
                    log(&format!("capture failed with no UI attached: {error}"));
                }
            }
        }
    }

    fn on_tray(&mut self, event: TrayEvent) -> bool {
        match event {
            TrayEvent::Open => self.request_open(),
            TrayEvent::Exit => self.begin_shutdown(),
            TrayEvent::Unavailable(error) => {
                self.tray_available = false;
                self.tray_error = Some(error);
            }
        }
        false
    }

    fn on_ipc(&mut self, event: HostEvent) -> bool {
        match event {
            HostEvent::UiConnected { id, connection, pid } => self.on_ui_connected(id, connection, pid),
            HostEvent::UiReady { id } => {
                if self.ui.as_ref().is_some_and(|ui| ui.id == id) {
                    if let Some(ui) = self.ui.as_mut() {
                        ui.ready = true;
                    }
                    self.send_welcome();
                    self.flush_pending();
                }
            }
            HostEvent::UiClosing { id } => {
                let _ = id;
            }
            HostEvent::UiDisconnected { id } => {
                if self.ui.as_ref().is_some_and(|ui| ui.id == id) {
                    self.ui = None;
                    if self.exit_requested {
                        return true;
                    }
                    if !self.tray_available {
                        log("tray unavailable and the UI closed; stopping host");
                        return true;
                    }
                    if !self.pending_captures.is_empty() || self.pending_open {
                        self.ensure_ui();
                    }
                }
            }
            HostEvent::EnsureOpen => self.request_open(),
            #[cfg(debug_assertions)]
            HostEvent::InjectCapture(text) => self.deliver_capture(text),
            HostEvent::ClientError { id, message } => {
                log(&format!("ui client {id} error: {message}"));
            }
        }
        false
    }

    fn on_ui_connected(
        &mut self,
        id: u64,
        connection: std::sync::Arc<ipc::ServerConnection>,
        pid: u32,
    ) {
        match &mut self.ui {
            Some(ui) if ui.connection.is_none() && ui.id == 0 => {
                ui.id = id;
                ui.pid = Some(pid);
                ui.connection = Some(connection);
            }
            Some(_) => {
                let _ = connection.send(&ServerMessage::Rejected);
            }
            None => {
                self.ui = Some(UiState {
                    id,
                    connection: Some(connection),
                    pid: Some(pid),
                    ready: false,
                });
            }
        }
    }

    fn on_spawn_timeout(&mut self, generation: u64) {
        if generation != self.spawn_generation {
            return;
        }
        let starting = self
            .ui
            .as_ref()
            .is_some_and(|ui| ui.connection.is_none() && !ui.ready);
        if starting {
            log("UI did not become ready before the spawn timeout");
            if let Some(ui) = self.ui.take() {
                if let Some(pid) = ui.pid {
                    terminate(pid);
                }
            }
        }
    }

    fn on_shutdown_timeout(&mut self, id: u64) {
        if let Some(ui) = &self.ui {
            if ui.id == id || ui.id == 0 {
                if let Some(pid) = ui.pid {
                    log("UI did not exit in time; terminating it");
                    terminate(pid);
                }
                self.ui = None;
            }
        }
    }

    fn request_open(&mut self) {
        if self.ui.is_none() {
            self.ensure_ui();
        } else if self.ui.as_ref().is_some_and(|ui| ui.ready) {
            self.send(&ServerMessage::Open);
        } else {
            self.pending_open = true;
        }
    }

    fn deliver_capture(&mut self, text: String) {
        if self.ui.as_ref().is_some_and(|ui| ui.ready) {
            self.send(&ServerMessage::Capture { text });
        } else {
            self.pending_captures.push(text);
            if self.ui.is_none() {
                self.ensure_ui();
            }
        }
    }

    fn send_welcome(&mut self) {
        self.send(&ServerMessage::Welcome {
            tray_available: self.tray_available,
            tray_error: self.tray_error.clone(),
            last_capture_error: self.last_capture_error.clone(),
        });
    }

    fn flush_pending(&mut self) {
        if self.pending_captures.is_empty() {
            if self.pending_open {
                self.pending_open = false;
                self.send(&ServerMessage::Open);
            }
            return;
        }
        self.pending_open = false;
        for text in std::mem::take(&mut self.pending_captures) {
            self.send(&ServerMessage::Capture { text });
        }
    }

    fn ensure_ui(&mut self) {
        match spawn_ui() {
            Ok(pid) => {
                self.spawn_generation = self.spawn_generation.wrapping_add(1);
                let generation = self.spawn_generation;
                self.ui = Some(UiState {
                    id: 0,
                    connection: None,
                    pid: Some(pid),
                    ready: false,
                });
                let events = self.events.clone();
                thread::spawn(move || {
                    thread::sleep(std::time::Duration::from_millis(SPAWN_TIMEOUT_MS));
                    let _ = events.send(Control::SpawnTimeout(generation));
                });
            }
            Err(error) => {
                log(&format!("failed to launch UI: {error}"));
                #[cfg(windows)]
                {
                    let text: Vec<u16> = "Book Save could not open its window."
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect();
                    let title: Vec<u16> =
                        "Book Save".encode_utf16().chain(std::iter::once(0)).collect();
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                            std::ptr::null_mut(),
                            text.as_ptr(),
                            title.as_ptr(),
                            windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                                | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
                        );
                    }
                }
            }
        }
    }

    fn begin_shutdown(&mut self) {
        self.exit_requested = true;
        if let Some(ui) = &self.ui {
            if let Some(connection) = &ui.connection {
                let _ = connection.send(&ServerMessage::Quit);
            }
            let id = ui.id;
            let events = self.events.clone();
            thread::spawn(move || {
                thread::sleep(std::time::Duration::from_millis(SHUTDOWN_TIMEOUT_MS));
                let _ = events.send(Control::ShutdownTimeout(id));
            });
        }
    }

    fn send(&self, message: &ServerMessage) {
        if let Some(ui) = &self.ui {
            if let Some(connection) = &ui.connection {
                if let Err(error) = connection.send(message) {
                    log(&format!("failed to send to ui: {error}"));
                }
            }
        }
    }
}

#[cfg(windows)]
fn terminate(pid: u32) {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
fn terminate(_pid: u32) {}

#[cfg(windows)]
fn spawn_ui() -> std::io::Result<u32> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    let exe = std::env::current_exe()?;
    let directory = exe
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no exe directory"))?;
    let ui = directory.join("book-save-ui.exe");
    let child = std::process::Command::new(ui)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    Ok(child.id())
}

#[cfg(not(windows))]
fn spawn_ui() -> std::io::Result<u32> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "UI launch is supported only on Windows",
    ))
}

fn log(message: &str) {
    eprintln!("book-save-host: {message}");
}
