//! Tauri commands for keychain queries.
//!
//! Note: secret *writes* happen through `add_provider` / `update_provider`
//! in `commands/provider.rs`. We never expose a raw `set_api_key` command to
//! avoid accidental exposure through the IPC surface.

use std::sync::Arc;
use tauri::State;

use crate::secret::SecretStore;

#[tauri::command]
pub fn has_api_key(provider_id: String, secrets: State<Arc<SecretStore>>) -> bool {
    secrets.has(&provider_id)
}

#[tauri::command]
pub fn delete_api_key(
    provider_id: String,
    secrets: State<Arc<SecretStore>>,
) -> Result<(), String> {
    secrets.delete(&provider_id).map_err(|e| e.to_string())
}
