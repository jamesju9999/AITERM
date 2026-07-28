use portable_pty::PtySize;
use serde::Deserialize;
use tauri::{AppHandle, State};

use super::error::PtyError;
use super::manager::PtyManager;

#[derive(Debug, Deserialize)]
pub struct PtySizeArg {
    pub rows: u16,
    pub cols: u16,
}

impl From<PtySizeArg> for PtySize {
    fn from(s: PtySizeArg) -> Self {
        PtySize {
            rows: s.rows,
            cols: s.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[tauri::command]
pub fn pty_create(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    size: PtySizeArg,
    cwd: Option<String>,
) -> Result<String, PtyError> {
    let cwd = cwd.map(std::path::PathBuf::from);
    manager.create_with_app(app, size.into(), cwd)
}

#[tauri::command]
pub fn pty_write(
    manager: State<'_, PtyManager>,
    id: String,
    data: String,
) -> Result<(), PtyError> {
    manager.write(&id, data.as_bytes())
}

#[tauri::command]
pub fn pty_resize(
    manager: State<'_, PtyManager>,
    id: String,
    size: PtySizeArg,
) -> Result<(), PtyError> {
    manager.resize(&id, size.into())
}

#[tauri::command]
pub fn pty_close(
    manager: State<'_, PtyManager>,
    id: String,
) -> Result<(), PtyError> {
    manager.close(&id)
}

/// Return the shell type for a PTY session ("pwsh", "cmd", "bash", or "unknown").
#[tauri::command]
pub fn pty_get_shell_type(
    manager: State<'_, PtyManager>,
    id: String,
) -> Option<String> {
    manager.get_shell_variant(&id).map(|v| match v {
        super::cd_parser::ShellVariant::Pwsh => "pwsh".into(),
        super::cd_parser::ShellVariant::Cmd => "cmd".into(),
        super::cd_parser::ShellVariant::Bash => "bash".into(),
        super::cd_parser::ShellVariant::Unknown => "unknown".into(),
    })
}

/// Normalize a path to use forward slashes on all platforms.
fn norm(p: impl AsRef<std::path::Path>) -> String {
    p.as_ref().to_string_lossy().replace('\\', "/")
}

/// Get the current working directory of a PTY session.
#[tauri::command]
pub fn pty_get_cwd(
    manager: State<'_, PtyManager>,
    id: String,
) -> Option<String> {
    manager.get_cwd(&id).map(|p| norm(&p))
}

/// Return the last ~4 KiB of ANSI-stripped PTY output for the session.
#[tauri::command]
pub fn pty_get_recent_output(
    manager: State<'_, PtyManager>,
    id: String,
) -> Option<String> {
    manager.get_recent_output(&id, 4096)
}

/// A single file/directory entry returned by pty_list_dir.
#[derive(serde::Serialize, Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

/// List the immediate children of `path` (or the session's CWD if path is empty).
#[tauri::command]
pub fn pty_list_dir(
    manager: State<'_, PtyManager>,
    id: String,
    path: String,
) -> Result<Vec<DirEntry>, String> {
    let base = if path.is_empty() {
        manager.get_cwd(&id)
            .ok_or_else(|| "Session not found".to_string())?
    } else {
        std::path::PathBuf::from(&path)
    };

    let mut entries: Vec<DirEntry> = std::fs::read_dir(&base)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| {
            let meta = e.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = if is_dir { None } else { meta.as_ref().map(|m| m.len()) };
            DirEntry {
                name: e.file_name().to_string_lossy().to_string(),
                path: norm(e.path()),
                is_dir,
                size,
            }
        })
        .filter(|e| !e.name.starts_with('.')) // hide dotfiles by default
        .collect();

    // Sort: directories first, then alphabetical
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

/// Returned by pty_read_file.
#[derive(serde::Serialize)]
pub struct FileContent {
    pub content: String,
    pub truncated: bool,
}

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// Read a binary file and return its content as a base64-encoded string.
/// Used by the Doc Converter to read files dropped via OS drag-and-drop (tauri://drag-drop).
#[tauri::command]
pub fn read_file_as_bytes(path: String) -> Result<String, String> {
    use base64::{Engine as _, engine::general_purpose};
    let canonical = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&canonical).map_err(|e| e.to_string())?;
    Ok(general_purpose::STANDARD.encode(bytes))
}

/// Read a text file's content. Caps at 10 MB; binary files return an error.
#[tauri::command]
pub fn pty_read_file(path: String) -> Result<FileContent, String> {
    use std::io::Read;

    // Resolve to a canonical absolute path (follows symlinks, eliminates ../)
    let canonical = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;

    // Open the file first, then get metadata from the handle (avoids TOCTOU race)
    let mut file = std::fs::File::open(&canonical).map_err(|e| e.to_string())?;
    let file_size = file.metadata().map_err(|e| e.to_string())?.len();
    let truncated = file_size > MAX_FILE_BYTES;

    let read_size = MAX_FILE_BYTES.min(file_size) as usize;
    let mut buf = vec![0u8; read_size];
    if read_size > 0 {
        file.read_exact(&mut buf).map_err(|e| e.to_string())?;
    }

    let content = String::from_utf8(buf)
        .map_err(|_| "file contains non-UTF-8 bytes and cannot be read as text".to_string())?;

    Ok(FileContent { content, truncated })
}

/// Overwrite (or create) a text file with the given UTF-8 content.
#[tauri::command]
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    // Expand ~ to home directory
    let expanded = if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(&path[2..])
        } else {
            std::path::PathBuf::from(&path)
        }
    } else {
        std::path::PathBuf::from(&path)
    };

    // Create parent directories if needed
    if let Some(parent) = expanded.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    std::fs::write(&expanded, content.as_bytes()).map_err(|e| e.to_string())
}

/// Expands a `GetLogicalDrives` bitmask into drive roots.
///
/// Bit 0 is `A:`, bit 1 is `B:`, through bit 25 for `Z:`; higher bits are
/// undefined and ignored. Roots use a forward slash to match `norm()`'s output
/// so the frontend never has to normalise separators.
///
/// Kept free of the Win32 call so it is testable on any platform — the bit
/// arithmetic is the part that can actually be wrong.
fn drives_from_mask(mask: u32) -> Vec<String> {
    (0..26u32)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| format!("{}:/", (b'A' + bit as u8) as char))
        .collect()
}

/// Drive roots available on this machine, for the file panel's drive switcher.
///
/// Windows only — every other platform has a single rooted filesystem, so an
/// empty list is the honest answer and the frontend hides the control.
#[tauri::command]
pub fn list_drives() -> Vec<String> {
    #[cfg(windows)]
    {
        // A single kernel32 call reading a bitmask: no per-drive filesystem
        // access, so a mapped-but-disconnected network drive cannot stall it
        // the way probing each letter with fs::metadata would.
        let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        drives_from_mask(mask)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod drive_tests {
    use super::drives_from_mask;

    #[test]
    fn empty_mask_yields_no_drives() {
        assert!(drives_from_mask(0).is_empty());
    }

    #[test]
    fn bit_zero_is_drive_a() {
        assert_eq!(drives_from_mask(0b1), vec!["A:/"]);
    }

    #[test]
    fn typical_windows_machine_has_c_and_d() {
        // bits 2 and 3
        assert_eq!(drives_from_mask(0b1100), vec!["C:/", "D:/"]);
    }

    #[test]
    fn bit_twenty_five_is_drive_z() {
        assert_eq!(drives_from_mask(1 << 25), vec!["Z:/"]);
    }

    #[test]
    fn all_twenty_six_letters_are_expanded_in_order() {
        let all = drives_from_mask((1 << 26) - 1);
        assert_eq!(all.len(), 26);
        assert_eq!(all[0], "A:/");
        assert_eq!(all[25], "Z:/");
    }

    #[test]
    fn bits_above_twenty_five_are_ignored() {
        // Nothing beyond Z: exists, so the high bits must not invent entries.
        assert!(drives_from_mask(1 << 26).is_empty());
        assert_eq!(drives_from_mask((1 << 31) | 0b100), vec!["C:/"]);
    }
}
