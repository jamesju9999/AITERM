use std::path::PathBuf;

use crate::architecture::{scanner, store, types::ScanResult};

/// 檔案數上限；超過時 `truncated = true`。
const MAX_FILES: usize = 20000;
/// 每個關鍵檔案送給 AI 的上限（位元組）。
const MAX_FILE_BYTES: usize = 6000;

#[tauri::command]
pub async fn arch_scan(root: String) -> Result<ScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = PathBuf::from(root);
        let mut res = scanner::scan(&root, MAX_FILES)?;
        res.git_head = store::git_head(&root);
        Ok(res)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn arch_load(root: String) -> Result<Option<String>, String> {
    store::load(&PathBuf::from(root))
}

#[tauri::command]
pub fn arch_save(root: String, json: String) -> Result<(), String> {
    store::save(&PathBuf::from(root), &json)
}

#[tauri::command]
pub async fn arch_read_files(root: String, paths: Vec<String>) -> Result<Vec<(String, String)>, String> {
    tauri::async_runtime::spawn_blocking(move || store::read_files(&PathBuf::from(root), &paths, MAX_FILE_BYTES))
        .await
        .map_err(|e| e.to_string())
}
