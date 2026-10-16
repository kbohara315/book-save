//! Newline-delimited JSON control protocol over a per-user named pipe.
//!
//! The resident host owns the pipe server; the GPUI UI process is the only
//! client. Messages are newline-delimited JSON objects. A single capture
//! payload is bounded by [`MAX_CAPTURE_BYTES`] and any oversized message
//! disconnects that client without affecting the host.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Maximum size of a single clipboard capture payload carried by the protocol.
pub const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

/// Maximum size of one full framed message, envelope included.
pub const MAX_MESSAGE_BYTES: usize = MAX_CAPTURE_BYTES + 8 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Ui,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello { role: Role, pid: u32 },
    Ready,
    EnsureOpen,
    UiClosing,
    #[cfg(debug_assertions)]
    InjectCapture { text: String },
    Ping,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        tray_available: bool,
        tray_error: Option<String>,
        last_capture_error: Option<String>,
    },
    Capture {
        text: String,
    },
    Open,
    Quit,
    Error {
        message: String,
    },
    CaptureFailed {
        message: String,
    },
    Rejected,
    Pong,
}

pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).expect("protocol messages always serialize");
    bytes.push(b'\n');
    bytes
}

pub fn decode<T: DeserializeOwned>(line: &[u8]) -> serde_json::Result<T> {
    serde_json::from_slice(line)
}

#[cfg(windows)]
mod imp {
    use std::{
        io,
        ptr::{null, null_mut},
        sync::Arc,
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_BROKEN_PIPE, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
            ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
            INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, OPEN_EXISTING,
            PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
        },
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
            PIPE_WAIT, WaitNamedPipeW,
        },
        System::{
            IO::{GetOverlappedResult, OVERLAPPED},
            Threading::CreateEventW,
        },
    };

    use super::{ClientMessage, MAX_MESSAGE_BYTES, ServerMessage};
    use crate::instance::ServerSecurity;

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn io_error() -> io::Error {
        io::Error::last_os_error()
    }

    fn overlapped() -> io::Result<(OVERLAPPED, HANDLE)> {
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err(io_error());
        }
        let mut operation: OVERLAPPED = unsafe { std::mem::zeroed() };
        operation.hEvent = event;
        Ok((operation, event))
    }

    fn finish_operation(handle: HANDLE, operation: &OVERLAPPED, transferred: &mut u32) -> io::Result<()> {
        let ok = unsafe { GetOverlappedResult(handle, operation, transferred, 1) };
        if ok == 0 { Err(io_error()) } else { Ok(()) }
    }

    fn write_all(handle: HANDLE, bytes: &[u8]) -> io::Result<()> {
        let mut offset = 0usize;
        while offset < bytes.len() {
            let (mut operation, event) = overlapped()?;
            let mut written = 0u32;
            let ok = unsafe {
                WriteFile(
                    handle,
                    bytes[offset..].as_ptr().cast(),
                    (bytes.len() - offset) as u32,
                    &mut written,
                    &mut operation,
                )
            };
            if ok == 0 {
                let error = unsafe { GetLastError() };
                if error != windows_sys::Win32::Foundation::ERROR_IO_PENDING {
                    unsafe { windows_sys::Win32::Foundation::CloseHandle(event); }
                    return Err(io::Error::from_raw_os_error(error as i32));
                }
                if let Err(error) = finish_operation(handle, &operation, &mut written) {
                    unsafe { windows_sys::Win32::Foundation::CloseHandle(event); }
                    return Err(error);
                }
            }
            unsafe { windows_sys::Win32::Foundation::CloseHandle(event); }
            if written == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "pipe write made no progress"));
            }
            offset += written as usize;
        }
        Ok(())
    }

    /// Reads one newline-terminated framed message into `buffer`.
    ///
    /// Returns `Ok(None)` when the peer closed the pipe. Errors if the peer
    /// exceeds [`MAX_MESSAGE_BYTES`] without a terminator.
    fn read_frame(handle: HANDLE, buffer: &mut Vec<u8>) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
                let mut frame: Vec<u8> = buffer.drain(..=position).collect();
                frame.pop();
                if frame.last() == Some(&b'\r') {
                    frame.pop();
                }
                return Ok(Some(frame));
            }
            if buffer.len() > MAX_MESSAGE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "control message exceeds the maximum size",
                ));
            }
            let mut chunk = [0u8; 4096];
            let (mut operation, event) = overlapped()?;
            let mut read = 0u32;
            let ok = unsafe {
                ReadFile(
                    handle,
                    chunk.as_mut_ptr().cast(),
                    chunk.len() as u32,
                    &mut read,
                    &mut operation,
                )
            };
            if ok == 0 {
                let error = unsafe { GetLastError() };
                if error == ERROR_BROKEN_PIPE || error == ERROR_PIPE_NOT_CONNECTED {
                    unsafe { windows_sys::Win32::Foundation::CloseHandle(event); }
                    return Ok(None);
                }
                if error != windows_sys::Win32::Foundation::ERROR_IO_PENDING {
                    unsafe { windows_sys::Win32::Foundation::CloseHandle(event); }
                    return Err(io::Error::from_raw_os_error(error as i32));
                }
                if let Err(error) = finish_operation(handle, &operation, &mut read) {
                    unsafe { windows_sys::Win32::Foundation::CloseHandle(event); }
                    return match error.raw_os_error().map(|value| value as u32) {
                        Some(ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED) => Ok(None),
                        _ => Err(error),
                    };
                }
            }
            unsafe { windows_sys::Win32::Foundation::CloseHandle(event); }
            if read == 0 {
                return Ok(None);
            }
            buffer.extend_from_slice(&chunk[..read as usize]);
        }
    }

    /// A connected server-side pipe instance.
    pub struct ServerConnection {
        handle: HANDLE,
    }

    // Each operation owns its OVERLAPPED structure and event. This permits a
    // server reader and writer to progress simultaneously on one pipe handle.
    unsafe impl Send for ServerConnection {}
    unsafe impl Sync for ServerConnection {}

    impl ServerConnection {
        pub fn send(&self, message: &ServerMessage) -> io::Result<()> {
            write_all(self.handle, &super::encode(message))
        }

        pub fn read(&self, buffer: &mut Vec<u8>) -> io::Result<Option<ClientMessage>> {
            match read_frame(self.handle, buffer)? {
                None => Ok(None),
                Some(frame) => super::decode(&frame)
                    .map(Some)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            }
        }

        pub fn peer_pid(&self) -> Option<u32> {
            let mut pid = 0u32;
            let ok = unsafe { GetNamedPipeClientProcessId(self.handle, &mut pid) };
            (ok != 0).then_some(pid)
        }

        pub fn disconnect(&self) {
            unsafe {
                DisconnectNamedPipe(self.handle);
            }
        }
    }

    impl Drop for ServerConnection {
        fn drop(&mut self) {
            unsafe {
                DisconnectNamedPipe(self.handle);
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }

    /// Creates per-user pipe instances and blocks for each client connection.
    pub struct PipeListener {
        name: Vec<u16>,
        security: Option<Arc<ServerSecurity>>,
    }

    impl PipeListener {
        pub fn new() -> Self {
            Self {
                name: crate::instance::pipe_name(),
                security: crate::instance::server_security().map(Arc::new),
            }
        }

        pub fn name_string() -> String {
            crate::instance::pipe_name_string()
        }

        /// Blocks until a client connects, then returns the connected instance.
        pub fn accept(&self) -> io::Result<ServerConnection> {
            let attributes = self
                .security
                .as_ref()
                .map(|security| security.attributes())
                .unwrap_or(null());
            let handle = unsafe {
                CreateNamedPipeW(
                    self.name.as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    PIPE_UNLIMITED_INSTANCES,
                    MAX_MESSAGE_BYTES as u32,
                    MAX_MESSAGE_BYTES as u32,
                    0,
                    attributes,
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(io_error());
            }
            let (mut operation, event) = match overlapped() {
                Ok(pair) => pair,
                Err(error) => {
                    unsafe { windows_sys::Win32::Foundation::CloseHandle(handle); }
                    return Err(error);
                }
            };
            let connected = unsafe { ConnectNamedPipe(handle, &mut operation) };
            if connected == 0 {
                let error = unsafe { GetLastError() };
                if error == windows_sys::Win32::Foundation::ERROR_IO_PENDING {
                    let mut transferred = 0;
                    if let Err(error) = finish_operation(handle, &operation, &mut transferred) {
                        unsafe {
                            windows_sys::Win32::Foundation::CloseHandle(event);
                            windows_sys::Win32::Foundation::CloseHandle(handle);
                        }
                        return Err(error);
                    }
                } else if error != ERROR_PIPE_CONNECTED {
                    unsafe {
                        windows_sys::Win32::Foundation::CloseHandle(event);
                        windows_sys::Win32::Foundation::CloseHandle(handle);
                    }
                    return Err(io::Error::from_raw_os_error(error as i32));
                }
            }
            unsafe { windows_sys::Win32::Foundation::CloseHandle(event); }
            Ok(ServerConnection { handle })
        }
    }

    impl Default for PipeListener {
        fn default() -> Self {
            Self::new()
        }
    }

    /// A client connection to the resident host. Safe to share: reads and
    /// writes flow over the same kernel handle and never touch shared state.
    pub struct PipeClient {
        handle: HANDLE,
    }

    unsafe impl Send for PipeClient {}
    unsafe impl Sync for PipeClient {}

    impl PipeClient {
        pub fn connect() -> Option<Self> {
            let name = to_wide(&PipeListener::name_string());
            for _ in 0..2 {
                let handle = unsafe {
                    CreateFileW(
                        name.as_ptr(),
                        GENERIC_READ | GENERIC_WRITE,
                        FILE_SHARE_READ,
                        null(),
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                        null_mut(),
                    )
                };
                if handle != INVALID_HANDLE_VALUE {
                    return Some(Self { handle });
                }
                let error = unsafe { GetLastError() };
                if error == ERROR_PIPE_BUSY {
                    unsafe {
                        WaitNamedPipeW(name.as_ptr(), 2000);
                    }
                    continue;
                }
                return None;
            }
            None
        }

        pub fn send(&self, message: &ClientMessage) -> io::Result<()> {
            write_all(self.handle, &super::encode(message))
        }

        pub fn recv(&self, buffer: &mut Vec<u8>) -> io::Result<Option<ServerMessage>> {
            match read_frame(self.handle, buffer)? {
                None => Ok(None),
                Some(frame) => super::decode(&frame)
                    .map(Some)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            }
        }
    }

    impl Drop for PipeClient {
        fn drop(&mut self) {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }

    /// Best-effort request for a running host to show its UI. Used by a second
    /// normal launch that lost the single-instance race. The first message is a
    /// bare `EnsureOpen`, so the host never mistakes it for the UI handshake.
    pub fn request_open() {
        if let Some(client) = PipeClient::connect() {
            let _ = client.send(&ClientMessage::EnsureOpen);
        }
    }
}

#[cfg(windows)]
pub use imp::{PipeClient, PipeListener, ServerConnection, request_open};

#[cfg(not(windows))]
mod stub {
    use super::{ClientMessage, ServerMessage};

    pub struct PipeListener;

    impl PipeListener {
        pub fn new() -> Self {
            Self
        }

        pub fn name_string() -> String {
            String::new()
        }
    }

    impl Default for PipeListener {
        fn default() -> Self {
            Self::new()
        }
    }

    #[derive(Default)]
    pub struct PipeClient;

    impl PipeClient {
        pub fn connect() -> Option<Self> {
            None
        }

        pub fn send(&self, _message: &ClientMessage) -> std::io::Result<()> {
            Ok(())
        }

        pub fn recv(&self, _buffer: &mut Vec<u8>) -> std::io::Result<Option<ServerMessage>> {
            Ok(None)
        }
    }

    pub fn request_open() {}
}

#[cfg(not(windows))]
pub use stub::{PipeClient, PipeListener, request_open};
