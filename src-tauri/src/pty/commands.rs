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
) -> Result<String, PtyError> {
    manager.create_with_app(app, size.into())
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
