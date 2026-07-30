// src-tauri/src/api_docs/mod.rs
pub mod types;
pub mod runner;

use std::path::PathBuf;
use tauri::Manager;

/// Absolute path to `tools/ApiDocFetcher/fetcher.py` relative to the
/// Cargo manifest directory (dev) or the app resource bundle (production).
pub fn fetcher_script_path(app_handle: &tauri::AppHandle) -> PathBuf {
    // In production Tauri bundles the resources listed in tauri.conf.json.
    // In dev we resolve relative to CARGO_MANIFEST_DIR.
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest_dir
        .parent()  // workspace root
        .unwrap_or(&manifest_dir)
        .join("tools")
        .join("ApiDocFetcher")
        .join("fetcher.py");

    if dev_path.exists() {
        return dev_path;
    }

    // Production: try app resource dir
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        let prod_path = resource_dir.join("ApiDocFetcher").join("fetcher.py");
        if prod_path.exists() {
            return prod_path;
        }
    }

    dev_path  // return dev path even if missing — caller will get a clear error
}
