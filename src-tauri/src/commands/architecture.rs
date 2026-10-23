use std::path::PathBuf;

use crate::architecture::{scanner, store, types::ScanResult};

/// 檔案數上限；超過時 `truncated = true`。
const MAX_FILES: usize = 20000;
/// 每個關鍵檔案送給 AI 的上限（位元組）。
const MAX_FILE_BYTES: usize = 6000;
/// 呼叫端可以要求的每檔讀取上限的天花板（問答要讀得比分析多）。
const MAX_FILE_BYTES_CEILING: usize = 60000;

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
pub fn arch_qa_load(root: String) -> Result<Option<String>, String> {
    store::load_qa(&PathBuf::from(root))
}

#[tauri::command]
pub fn arch_qa_save(root: String, json: String) -> Result<(), String> {
    store::save_qa(&PathBuf::from(root), &json)
}

#[tauri::command]
pub fn arch_export_text(path: String, content: String) -> Result<(), String> {
    store::export_text(&PathBuf::from(path), &content)
}

#[tauri::command]
pub async fn arch_read_files(
    root: String,
    paths: Vec<String>,
    max_bytes: Option<usize>,
) -> Result<Vec<(String, String)>, String> {
    let limit = store::read_limit(max_bytes, MAX_FILE_BYTES, MAX_FILE_BYTES_CEILING);
    tauri::async_runtime::spawn_blocking(move || store::read_files(&PathBuf::from(root), &paths, limit))
        .await
        .map_err(|e| e.to_string())
}

/// 預覽檔案的大小上限（位元組）。
const MAX_PREVIEW_BYTES: usize = 512 * 1024;

#[tauri::command]
pub async fn arch_read_file(root: String, path: String) -> Result<store::FilePreview, String> {
    tauri::async_runtime::spawn_blocking(move || store::read_file_preview(&PathBuf::from(root), &path, MAX_PREVIEW_BYTES))
        .await
        .map_err(|e| e.to_string())?
}

/// 用系統預設程式開啟專案內的檔案。路徑必須通過 `resolve_in_root`。
#[tauri::command]
pub async fn arch_open_path(root: String, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let full = store::resolve_in_root(&PathBuf::from(root), &path)
            .ok_or_else(|| format!("找不到檔案或路徑不在專案內：{path}"))?;
        open::that(full).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
