pub mod ai;
pub mod commands;
pub mod config;
pub mod pty;
pub mod secret;

use std::sync::Arc;

use ai::router::AiRouter;
use commands::{
    ai::{ai_chat, ai_query},
    config::{get_config, is_onboarding_done, set_execution_mode, set_onboarding_done},
    provider::{
        add_provider, get_ollama_models, list_providers, remove_provider, set_default_provider,
        test_provider, update_provider,
    },
    secret::{delete_api_key, has_api_key},
};
use config::ConfigStore;
use pty::commands::{pty_close, pty_create, pty_resize, pty_write};
use pty::PtyManager;
use secret::SecretStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = Arc::new(ConfigStore::new());
    let secrets = Arc::new(SecretStore::new());
    let router = AiRouter::new(config.clone(), secrets.clone());

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())
        .manage(PtyManager::new())
        .manage(config)
        .manage(secrets)
        .manage(router)
        .invoke_handler(tauri::generate_handler![
            // PTY
            pty_create,
            pty_write,
            pty_resize,
            pty_close,
            // AI query
            ai_query,
            ai_chat,
            // Config
            get_config,
            set_execution_mode,
            is_onboarding_done,
            set_onboarding_done,
            // Provider management
            list_providers,
            add_provider,
            update_provider,
            remove_provider,
            set_default_provider,
            test_provider,
            get_ollama_models,
            // Secrets
            has_api_key,
            delete_api_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
