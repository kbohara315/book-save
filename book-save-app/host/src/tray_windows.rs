use super::TrayEvent;
use std::{
    mem::size_of,
    ptr::{null, null_mut},
    sync::atomic::{AtomicU32, Ordering},
    sync::mpsc,
    thread::{self, JoinHandle},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
        },
        WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
            DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, IMAGE_ICON,
            LR_DEFAULTSIZE, LoadImageW, MF_SEPARATOR, MF_STRING, MSG, PM_NOREMOVE, PeekMessageW,
            PostMessageW, PostThreadMessageW, RegisterClassW, RegisterWindowMessageW,
            SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
            WM_APP, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_NULL, WM_QUIT, WM_RBUTTONUP, WNDCLASSW,
        },
    },
};

const ICON_ID: u32 = 1;
const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
const WM_TRAY_MENU: u32 = WM_APP + 2;
const WM_TRAY_OPEN: u32 = WM_APP + 3;
const WM_TRAY_READD: u32 = WM_APP + 4;
const MENU_OPEN: usize = 1;
const MENU_EXIT: usize = 2;

static TRAY_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub struct TrayWorker {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl TrayWorker {
    pub fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
            let _ = thread.join();
        }
    }
}

impl Drop for TrayWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start() -> Result<
    (
        TrayWorker,
        futures::channel::mpsc::UnboundedReceiver<TrayEvent>,
    ),
    String,
> {
    #[cfg(debug_assertions)]
    if std::env::var_os("BOOK_SAVE_TEST_FAIL_TRAY_START").is_some() {
        return Err("Injected tray startup failure for lifecycle verification.".into());
    }
    let (events, receiver) = futures::channel::mpsc::unbounded();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let thread = thread::spawn(move || unsafe {
        let thread_id = GetCurrentThreadId();
        TRAY_THREAD_ID.store(thread_id, Ordering::Release);
        let mut message = MSG::default();
        PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);

        let instance = GetModuleHandleW(null());
        let class_name: Vec<u16> = "BookSaveTrayWindow\0".encode_utf16().collect();
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(tray_window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&window_class) == 0 {
            let _ = ready_tx.send(Err(format!(
                "Cannot register the tray window: {}",
                std::io::Error::last_os_error()
            )));
            return;
        }
        // A hidden top-level window receives Explorer's TaskbarCreated broadcast.
        let window = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            let _ = ready_tx.send(Err(format!(
                "Cannot create the tray window: {}",
                std::io::Error::last_os_error()
            )));
            return;
        }
        let icon = LoadImageW(
            instance,
            1usize as *const u16,
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE,
        );
        if icon.is_null() {
            DestroyWindow(window);
            let _ = ready_tx.send(Err(format!(
                "Cannot load the Book Save tray icon: {}",
                std::io::Error::last_os_error()
            )));
            return;
        }
        let taskbar_created = "TaskbarCreated\0".encode_utf16().collect::<Vec<_>>();
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(taskbar_created.as_ptr()),
            Ordering::Release,
        );
        if TASKBAR_CREATED.load(Ordering::Acquire) == 0 {
            DestroyWindow(window);
            DestroyIcon(icon);
            let _ = ready_tx.send(Err(format!(
                "Cannot register Explorer's TaskbarCreated message: {}",
                std::io::Error::last_os_error()
            )));
            return;
        }
        let data = icon_data(window, icon);
        if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
            DestroyWindow(window);
            DestroyIcon(icon);
            let _ = ready_tx.send(Err(format!(
                "Windows could not add the Book Save tray icon: {}",
                std::io::Error::last_os_error()
            )));
            return;
        }
        let _ = ready_tx.send(Ok(thread_id));
        loop {
            let result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result == -1 {
                let _ = events.unbounded_send(TrayEvent::Unavailable(format!(
                    "The tray message loop failed: {}",
                    std::io::Error::last_os_error()
                )));
                break;
            }
            if result == 0 {
                break;
            }
            if message.message == WM_TRAY_MENU {
                if show_menu(window) == MENU_EXIT {
                    let _ = events.unbounded_send(TrayEvent::Exit);
                }
            } else if message.message == WM_TRAY_OPEN {
                let _ = events.unbounded_send(TrayEvent::Open);
            } else if message.message == WM_TRAY_READD {
                #[cfg(debug_assertions)]
                let injected_failure = std::env::var_os("BOOK_SAVE_TEST_FAIL_TRAY_READD").is_some();
                #[cfg(not(debug_assertions))]
                let injected_failure = false;
                // TaskbarCreated means Explorer discarded its icon table. Delete first so
                // duplicate/stale shell state cannot make restoration fail spuriously.
                Shell_NotifyIconW(NIM_DELETE, &data);
                if injected_failure || Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                    let detail = if injected_failure {
                        "injected tray recovery failure".to_owned()
                    } else {
                        std::io::Error::last_os_error().to_string()
                    };
                    let _ = events.unbounded_send(TrayEvent::Unavailable(format!(
                        "Windows could not restore the tray icon after Explorer restarted: {detail}"
                    )));
                    break;
                }
            } else {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Shell_NotifyIconW(NIM_DELETE, &data);
        DestroyWindow(window);
        DestroyIcon(icon);
    });
    let thread_id = ready_rx.recv().map_err(|error| error.to_string())??;
    Ok((
        TrayWorker {
            thread_id,
            thread: Some(thread),
        },
        receiver,
    ))
}

unsafe fn icon_data(
    window: HWND,
    icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON,
) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW::default();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = ICON_ID;
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAY_CALLBACK;
    data.hIcon = icon;
    let tip: Vec<u16> = "Book Save\0".encode_utf16().collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    data
}

unsafe extern "system" fn tray_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_TRAY_CALLBACK {
        match lparam as u32 {
            WM_RBUTTONUP | WM_CONTEXTMENU => unsafe {
                PostThreadMessageW(TRAY_THREAD_ID.load(Ordering::Acquire), WM_TRAY_MENU, 0, 0)
            },
            WM_LBUTTONDBLCLK | windows_sys::Win32::UI::Shell::NIN_SELECT | 0x0401 => unsafe {
                PostThreadMessageW(TRAY_THREAD_ID.load(Ordering::Acquire), WM_TRAY_OPEN, 0, 0)
            },
            _ => 0,
        };
        return 0;
    }
    let taskbar_message = TASKBAR_CREATED.load(Ordering::Acquire);
    if message == taskbar_message && taskbar_message != 0 {
        unsafe { PostThreadMessageW(TRAY_THREAD_ID.load(Ordering::Acquire), WM_TRAY_READD, 0, 0) };
        return 0;
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

unsafe fn show_menu(window: HWND) -> usize {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        let open: Vec<u16> = "Open Book Save\0".encode_utf16().collect();
        let exit: Vec<u16> = "Exit\0".encode_utf16().collect();
        AppendMenuW(menu, MF_STRING, MENU_OPEN, open.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, MENU_EXIT, exit.as_ptr());
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        SetForegroundWindow(window);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            window,
            null(),
        ) as usize;
        // Required so the next click for this foreground window dismisses the menu.
        PostMessageW(window, WM_NULL, 0, 0);
        DestroyMenu(menu);
        if selected == MENU_OPEN {
            PostThreadMessageW(TRAY_THREAD_ID.load(Ordering::Acquire), WM_TRAY_OPEN, 0, 0);
        }
        selected
    }
}
