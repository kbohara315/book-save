//! Per-user draft session for the UI process.
//!
//! The UI is the only writer of `captures.json`. This sidecar file remembers
//! the in-progress capture or edit so closing the window (which stops the UI)
//! does not lose what the user was typing. It is stored next to
//! `captures.json` and written atomically.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub capture_draft: String,
    #[serde(default)]
    pub edit_target: Option<u64>,
    #[serde(default)]
    pub edit_draft: String,
    #[serde(default)]
    pub capture_draft_before_edit: String,
    #[serde(default)]
    pub deferred_capture: Option<String>,
}

pub fn session_path() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "Windows did not provide the LOCALAPPDATA folder.".to_owned())?;
        Ok(local_app_data.join("Book Save").join("session.json"))
    }
    #[cfg(not(windows))]
    {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("session.json"))
    }
}

fn write_session(path: &Path, session: &Session) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec_pretty(session)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    replace_file(&temporary, path)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let temporary: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
    let moved = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(temporary, destination)
}

impl Session {
    /// Loads the saved session, or an empty one when absent or unreadable.
    pub fn load() -> Self {
        session_path()
            .ok()
            .and_then(|path| fs::read(&path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Best-effort atomic save. Failures never block the user's work.
    pub fn save(&self) -> Result<(), String> {
        if let Ok(path) = session_path() {
            write_session(&path, self).map_err(|error| error.to_string())
        } else {
            Err("LOCALAPPDATA is unavailable".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Session, write_session};
    use std::fs;

    #[test]
    fn session_roundtrips_through_json() {
        let session = Session {
            capture_draft: "draft".into(),
            edit_target: Some(7),
            edit_draft: "editing".into(),
            capture_draft_before_edit: "before".into(),
            deferred_capture: Some("deferred".into()),
        };
        let bytes = serde_json::to_vec(&session).unwrap();
        let back: Session = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(session, back);
    }

    #[test]
    fn missing_fields_default_to_empty() {
        let session: Session = serde_json::from_slice(b"{}").unwrap();
        assert_eq!(session, Session::default());
    }

    #[test]
    fn session_file_replaces_previous_draft() {
        let directory = std::env::temp_dir().join(format!(
            "book-save-session-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("session.json");
        let first = Session { capture_draft: "first".into(), ..Session::default() };
        let second = Session { capture_draft: "second".into(), ..Session::default() };

        write_session(&path, &first).unwrap();
        write_session(&path, &second).unwrap();

        let saved: Session = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, second);
        assert!(!path.with_extension("json.tmp").exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
