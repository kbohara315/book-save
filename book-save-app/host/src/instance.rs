//! Per-user identity for the resident host and its UI.
//!
//! The resident host owns a per-user named mutex (single instance) and a
//! per-user named pipe (IPC). Both names include a token derived from the
//! signed-in user's SID, and the pipe carries a DACL that grants access only to
//! that user and to SYSTEM. Test runs set `BOOK_SAVE_TEST_INSTANCE` so they use
//! isolated names and never touch the installed app.

#[cfg(windows)]
mod imp {
    use std::{
        ffi::c_void,
        mem::size_of,
        ptr::{null, null_mut},
        slice::from_raw_parts,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LocalFree},
        Security::{
            GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER, TokenUser,
        },
        Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken},
    };

    pub const TEST_INSTANCE_ENV: &str = "BOOK_SAVE_TEST_INSTANCE";

    fn sanitize(value: &str) -> String {
        value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                    character
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// A stable, filesystem- and pipe-safe token for this user's app instance.
    pub fn instance_token() -> String {
        if let Some(name) = std::env::var_os(TEST_INSTANCE_ENV) {
            let sanitized = sanitize(&name.to_string_lossy());
            if !sanitized.is_empty() {
                return format!("test-{sanitized}");
            }
        }
        match user_sid_string() {
            Ok(sid) => format!("u{:016x}", fnv1a(sid.as_bytes())),
            Err(_) => {
                let fallback = std::env::var("USERNAME").unwrap_or_else(|_| "default".into());
                format!("n{:016x}", fnv1a(fallback.as_bytes()))
            }
        }
    }

    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    pub fn mutex_name() -> Vec<u16> {
        format!("Local\\BookSaveHost-{}\0", instance_token())
            .encode_utf16()
            .collect()
    }

    pub fn pipe_name() -> Vec<u16> {
        format!("\\\\.\\pipe\\book-save-{}\0", instance_token())
            .encode_utf16()
            .collect()
    }

    pub fn pipe_name_string() -> String {
        format!("\\\\.\\pipe\\book-save-{}", instance_token())
    }

    fn user_sid_string() -> Result<String, String> {
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut length = 0u32;
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut length);
            if length == 0 {
                CloseHandle(token);
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut buffer = vec![0u8; length as usize];
            if GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
            ) == 0
            {
                CloseHandle(token);
                return Err(std::io::Error::last_os_error().to_string());
            }
            let sid = (*(buffer.as_ptr() as *const TOKEN_USER)).User.Sid;
            let mut wide: *mut u16 = null_mut();
            if ConvertSidToStringSidW(sid, &mut wide) == 0 {
                CloseHandle(token);
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut count = 0usize;
            while *wide.add(count) != 0 {
                count += 1;
            }
            let text = String::from_utf16_lossy(from_raw_parts(wide, count));
            LocalFree(wide as *mut c_void);
            CloseHandle(token);
            Ok(text)
        }
    }

    /// A DACL granting full control to this user and to SYSTEM only.
    pub struct ServerSecurity {
        descriptor: PSECURITY_DESCRIPTOR,
        attributes: SECURITY_ATTRIBUTES,
    }

    // The descriptor lives until the host exits; it is only read by the OS.
    unsafe impl Send for ServerSecurity {}
    unsafe impl Sync for ServerSecurity {}

    impl ServerSecurity {
        pub fn attributes(&self) -> *const SECURITY_ATTRIBUTES {
            &self.attributes
        }
    }

    impl Drop for ServerSecurity {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.descriptor);
            }
        }
    }

    pub fn server_security() -> Option<ServerSecurity> {
        let sid = user_sid_string().ok()?;
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})\0")
            .encode_utf16()
            .collect();
        unsafe {
            let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
            let mut size = 0u32;
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                &mut size,
            ) == 0
            {
                return None;
            }
            Some(ServerSecurity {
                descriptor,
                attributes: SECURITY_ATTRIBUTES {
                    nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor,
                    bInheritHandle: 0,
                },
            })
        }
    }

    pub enum Startup {
        First(InstanceGuard),
        AlreadyRunning,
    }

    /// Owns the per-user mutex for the life of the host process.
    pub struct InstanceGuard(HANDLE);

    unsafe impl Send for InstanceGuard {}

    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    pub fn acquire_startup() -> Startup {
        let name = mutex_name();
        unsafe {
            let mutex = CreateMutexW(null(), 1, name.as_ptr());
            if mutex.is_null() {
                return Startup::First(InstanceGuard(null_mut()));
            }
            let already = GetLastError() == ERROR_ALREADY_EXISTS;
            if already {
                CloseHandle(mutex);
                Startup::AlreadyRunning
            } else {
                Startup::First(InstanceGuard(mutex))
            }
        }
    }

    /// True when this process was started with `--background`.
    pub fn is_background_arg(arg: &str) -> bool {
        arg == "--background"
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
mod stub {
    pub const TEST_INSTANCE_ENV: &str = "BOOK_SAVE_TEST_INSTANCE";

    pub fn instance_token() -> String {
        "unsupported".into()
    }

    pub fn pipe_name_string() -> String {
        String::new()
    }

    pub fn is_background_arg(arg: &str) -> bool {
        arg == "--background"
    }
}

#[cfg(not(windows))]
pub use stub::*;
