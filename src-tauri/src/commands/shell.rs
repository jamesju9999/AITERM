// Callers are internal frontend constants (hardcoded HTTPS URLs only); no user input reaches this.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    open::that(url).map_err(|e| e.to_string())
}
