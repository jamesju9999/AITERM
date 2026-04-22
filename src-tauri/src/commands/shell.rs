#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    open::that(url).map_err(|e| e.to_string())
}
