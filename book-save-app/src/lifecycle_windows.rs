//! Windows single-instance handover.
//!
//! The first process owns a named per-user mutex for its lifetime. A later
//! launch sees the mutex, reveals the running window, and exits before it
//! installs a second global keyboard hook.

use super::is_background_arg;
use std::ptr::null;
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM},
    System::Threading::{CreateMutexW, GetCurrentProcessId},
    UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId, SW_RESTORE, SetForegroundWindow,
        ShowWindowAsync,
    },
};

pub fn background_launch_requested() -> bool {
    std::env::args().any(|arg| is_background_arg(&arg))
}

/// Reveal the running instance and report true when the caller should exit.
pub fn handover_to_running_instance() -> bool {
    unsafe {
        let name: Vec<u16> = "Local\\BookSaveSingleInstance\0".encode_utf16().collect();
        let mutex = CreateMutexW(null(), 1, name.as_ptr());
        if mutex.is_null() {
            return false;
        }
        if GetLastError() != ERROR_ALREADY_EXISTS {
            // First instance. The handle stays open for the life of the process.
            return false;
        }
        CloseHandle(mutex);
        reveal_running_window();
        true
    }
}

unsafe extern "system" fn reveal_candidate(window: HWND, found: LPARAM) -> i32 {
    unsafe {
        let mut process = 0u32;
        GetWindowThreadProcessId(window, &mut process);
        if process == 0 || process == GetCurrentProcessId() {
            return 1;
        }
        let mut title = [0u16; 256];
        let length = GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32);
        if length <= 0 {
            return 1;
        }
        if String::from_utf16_lossy(&title[..length as usize]) != "Book Save" {
            return 1;
        }
        ShowWindowAsync(window, SW_RESTORE);
        SetForegroundWindow(window);
        *(found as *mut bool) = true;
        0
    }
}

fn reveal_running_window() {
    unsafe {
        let mut found = false;
        EnumWindows(Some(reveal_candidate), &mut found as *mut bool as LPARAM);
        let _ = found;
    }
}
