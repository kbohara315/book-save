use super::{CaptureEvent, Sender};
use std::{
    cell::RefCell,
    mem::size_of,
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, GlobalFree, HGLOBAL, HWND, LPARAM, LRESULT, SetLastError, WPARAM},
    System::{
        DataExchange::*,
        LibraryLoader::GetModuleHandleW,
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        Threading::{GetCurrentProcessId, GetCurrentThreadId},
    },
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const UNICODE_TEXT: u32 = 13;
const MODIFIERS: [u16; 7] = [
    VK_RSHIFT,
    VK_LCONTROL,
    VK_RCONTROL,
    VK_LMENU,
    VK_RMENU,
    VK_LWIN,
    VK_RWIN,
];

enum ShiftGesture {
    Released(Option<u32>),
    Pressed(Option<u32>),
    Triggered(usize),
}

struct HookState {
    gesture: ShiftGesture,
    requests: SyncSender<usize>,
    busy: Arc<AtomicBool>,
}

thread_local! {
    static HOOK: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

pub struct CaptureWorker {
    stop: Arc<AtomicBool>,
    hook_thread: Option<(u32, JoinHandle<()>)>,
}

impl CaptureWorker {
    pub(super) fn start(events: Sender<CaptureEvent>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (ready, receive_ready) = sync_channel(1);
        let thread = thread::spawn(move || unsafe {
            let (requests, receive_requests) = sync_channel(1);
            let busy = Arc::new(AtomicBool::new(false));
            let copy_busy = busy.clone();
            let copy_events = events.clone();
            let copy_stop = worker_stop.clone();
            let copy_thread = thread::spawn(move || {
                let owner = ClipboardWindow::new();
                while !copy_stop.load(Ordering::Acquire) {
                    match receive_requests.recv_timeout(Duration::from_millis(50)) {
                        Ok(source) => {
                            let result = owner.as_ref().map_err(Clone::clone).and_then(|owner| {
                                capture_selection(owner.0, source as HWND, &copy_stop)
                            });
                            let event = match result {
                                Ok(text) => CaptureEvent::SelectedText(text),
                                Err(error) => CaptureEvent::Failed(error),
                            };
                            let _ = copy_events.send(event);
                            copy_busy.store(false, Ordering::Release);
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
            let mut message = MSG::default();
            PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
            HOOK.with(|state| {
                *state.borrow_mut() = Some(HookState {
                    gesture: if GetAsyncKeyState(VK_LSHIFT as i32) < 0 {
                        ShiftGesture::Pressed(None)
                    } else {
                        ShiftGesture::Released(None)
                    },
                    requests,
                    busy,
                });
            });
            let hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_hook),
                GetModuleHandleW(null()),
                0,
            );
            let _ = ready.send(GetCurrentThreadId());
            if hook.is_null() {
                let _ = events.send(CaptureEvent::Failed(format!(
                    "Cannot install the global keyboard hook. {}",
                    std::io::Error::last_os_error()
                )));
            } else {
                let _ = events.send(CaptureEvent::Ready);
                loop {
                    let result = GetMessageW(&mut message, null_mut(), 0, 0);
                    if result <= 0 {
                        if result < 0 {
                            let _ = events.send(CaptureEvent::Failed(
                                "The global keyboard hook stopped unexpectedly.".into(),
                            ));
                        }
                        break;
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                UnhookWindowsHookEx(hook);
            }
            worker_stop.store(true, Ordering::Release);
            HOOK.with(|state| *state.borrow_mut() = None);
            let _ = copy_thread.join();
        });
        let id = receive_ready.recv().unwrap_or(0);
        Self {
            stop,
            hook_thread: Some((id, thread)),
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some((id, thread)) = self.hook_thread.take() {
            unsafe {
                PostThreadMessageW(id, WM_QUIT, 0, 0);
            }
            let _ = thread.join();
        }
    }
}

impl Drop for CaptureWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let key = unsafe { &*(data as *const KBDLLHOOKSTRUCT) };
        if key.flags & (LLKHF_INJECTED | LLKHF_LOWER_IL_INJECTED) == 0 {
            HOOK.with(|state| {
                let mut state = state.borrow_mut();
                let Some(state) = state.as_mut() else {
                    return;
                };
                let down = matches!(message as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                if key.vkCode == VK_LSHIFT as u32 {
                    if down {
                        if let ShiftGesture::Released(first) = state.gesture {
                            state.gesture = if modifiers_held() {
                                ShiftGesture::Pressed(None)
                            } else if first.is_some_and(|first| key.time.wrapping_sub(first) <= 350)
                            {
                                ShiftGesture::Triggered(unsafe { GetForegroundWindow() } as usize)
                            } else {
                                ShiftGesture::Pressed(Some(key.time))
                            };
                        }
                    } else {
                        match std::mem::replace(&mut state.gesture, ShiftGesture::Released(None)) {
                            ShiftGesture::Released(first) | ShiftGesture::Pressed(first) => {
                                state.gesture = ShiftGesture::Released(first);
                            }
                            ShiftGesture::Triggered(source) => {
                                if !modifiers_held()
                                    && state
                                        .busy
                                        .compare_exchange(
                                            false,
                                            true,
                                            Ordering::AcqRel,
                                            Ordering::Acquire,
                                        )
                                        .is_ok()
                                {
                                    if state.requests.try_send(source).is_err() {
                                        state.busy.store(false, Ordering::Release);
                                    }
                                }
                            }
                        }
                    }
                } else if down {
                    state.gesture = match state.gesture {
                        ShiftGesture::Released(_) => ShiftGesture::Released(None),
                        _ => ShiftGesture::Pressed(None),
                    };
                }
            });
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, message, data) }
}

fn modifiers_held() -> bool {
    MODIFIERS
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(*key as i32) < 0 })
}

struct ClipboardWindow(HWND);

impl ClipboardWindow {
    fn new() -> Result<Self, String> {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        if window.is_null() {
            Err("Cannot create the clipboard owner window.".into())
        } else {
            Ok(Self(window))
        }
    }
}

impl Drop for ClipboardWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

struct Clipboard;

impl Clipboard {
    fn open(owner: HWND) -> Result<Self, String> {
        let deadline = Instant::now() + Duration::from_millis(200);
        loop {
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Self);
            }
            if Instant::now() >= deadline {
                return Err("The clipboard is busy. Try again.".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

struct ClipboardData(HGLOBAL);

impl ClipboardData {
    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let data = Self(unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) });
        if data.0.is_null() {
            return Err("Cannot allocate a safe clipboard backup.".into());
        }
        let target = unsafe { GlobalLock(data.0) };
        if target.is_null() {
            return Err("Cannot lock the clipboard backup.".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast(), bytes.len());
            GlobalUnlock(data.0);
        }
        Ok(data)
    }

    fn put(mut self, format: u32) -> Result<(), String> {
        if unsafe { SetClipboardData(format, self.0) }.is_null() {
            return Err("Cannot restore clipboard data.".into());
        }
        self.0 = null_mut();
        Ok(())
    }
}

impl Drop for ClipboardData {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                GlobalFree(self.0);
            }
        }
    }
}

fn clipboard_bytes(format: u32) -> Result<Vec<u8>, String> {
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return Err("Cannot read the clipboard safely.".into());
    }
    let length = unsafe { GlobalSize(handle) };
    if length == 0 || length > 16 * 1024 * 1024 {
        return Err("Clipboard data is empty, too large, or cannot be backed up safely.".into());
    }
    let bytes = unsafe { GlobalLock(handle) };
    if bytes.is_null() {
        return Err("Cannot lock clipboard data safely.".into());
    }
    let result = unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), length) }.to_vec();
    unsafe {
        GlobalUnlock(handle);
    }
    Ok(result)
}

fn backup_clipboard() -> Result<Vec<(u32, ClipboardData)>, String> {
    let mut backup = Vec::new();
    let mut format = 0;
    loop {
        unsafe {
            SetLastError(0);
        }
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            if unsafe { GetLastError() } != 0 {
                return Err("Cannot enumerate the clipboard safely.".into());
            }
            return Ok(backup);
        }
        if !matches!(format, 1 | 7 | 13 | 16 | 0xC000..=0xFFFF) {
            return Err(format!(
                "Clipboard format {format} cannot be backed up safely. Copy plain text or clear the clipboard before capturing. Clipboard left untouched."
            ));
        }
        backup.push((
            format,
            ClipboardData::from_bytes(&clipboard_bytes(format)?)?,
        ));
    }
}

fn process_id(window: HWND) -> u32 {
    let mut id = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut id);
    }
    id
}

fn capture_selection(owner: HWND, source: HWND, stop: &AtomicBool) -> Result<String, String> {
    let source_process = process_id(source);
    if source_process == 0 || source_process == unsafe { GetCurrentProcessId() } {
        return Err("Select text in another app before double-tapping Left Shift.".into());
    }
    let deadline = Instant::now() + Duration::from_millis(350);
    while unsafe { GetAsyncKeyState(VK_LSHIFT as i32) } < 0 {
        if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err("Release Left Shift to capture selected text.".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    if modifiers_held()
        || unsafe { GetForegroundWindow() } != source
        || stop.load(Ordering::Acquire)
    {
        return Err("Capture canceled because the foreground app or modifiers changed.".into());
    }
    let clipboard = Clipboard::open(owner)?;
    let backup = backup_clipboard()?;
    let marker = ClipboardData::from_bytes(&[0, 0])?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err("Cannot prepare the clipboard. Clipboard left untouched.".into());
    }
    let prepared = marker.put(UNICODE_TEXT);
    let marker_sequence = unsafe { GetClipboardSequenceNumber() };
    drop(clipboard);

    let mut restore_sequence = marker_sequence;
    let result = prepared.and_then(|_| {
        if modifiers_held()
            || unsafe { GetAsyncKeyState(VK_LSHIFT as i32) } < 0
            || unsafe { GetForegroundWindow() } != source
            || stop.load(Ordering::Acquire)
        {
            return Err("Capture canceled because the foreground app or modifiers changed.".into());
        }
        let input = |key, flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let keys = [
            input(VK_LCONTROL, 0),
            input(0x43, 0),
            input(0x43, KEYEVENTF_KEYUP),
            input(VK_LCONTROL, KEYEVENTF_KEYUP),
        ];
        if unsafe { SendInput(keys.len() as u32, keys.as_ptr(), size_of::<INPUT>() as i32) }
            != keys.len() as u32
        {
            let release = [
                input(0x43, KEYEVENTF_KEYUP),
                input(VK_LCONTROL, KEYEVENTF_KEYUP),
            ];
            unsafe {
                SendInput(
                    release.len() as u32,
                    release.as_ptr(),
                    size_of::<INPUT>() as i32,
                );
            }
            return Err(
                "The source app did not accept Ctrl+C. Elevated apps may block capture.".into(),
            );
        }
        let deadline = Instant::now() + Duration::from_millis(800);
        while unsafe { GetClipboardSequenceNumber() } == marker_sequence {
            if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err("No selected text was copied before the capture timeout.".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _clipboard = Clipboard::open(owner)?;
        restore_sequence = unsafe { GetClipboardSequenceNumber() };
        if process_id(unsafe { GetClipboardOwner() }) != source_process {
            return Err("Another app changed the clipboard during capture.".into());
        }
        let bytes = clipboard_bytes(UNICODE_TEXT)?;
        restore_sequence = unsafe { GetClipboardSequenceNumber() };
        if bytes.len() % 2 != 0 {
            return Err("The copied text is not valid Unicode.".into());
        }
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
            .collect();
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .ok_or("The copied text has no terminator.")?;
        let text = String::from_utf16(&units[..end])
            .map_err(|_| "The copied text is not valid Unicode.")?;
        if text.trim().is_empty() {
            return Err("No usable text was selected.".into());
        }
        Ok(text)
    });

    let _clipboard = Clipboard::open(owner)
        .map_err(|error| format!("{error} Could not restore the clipboard."))?;
    if unsafe { GetClipboardSequenceNumber() } != restore_sequence
        || (restore_sequence != marker_sequence
            && process_id(unsafe { GetClipboardOwner() }) != source_process)
    {
        return Err("The clipboard changed during capture. Newer contents were left untouched; capture canceled.".into());
    }
    if unsafe { EmptyClipboard() } == 0 {
        return Err("Could not restore the clipboard.".into());
    }
    for (format, data) in backup {
        data.put(format)?;
    }
    result
}
