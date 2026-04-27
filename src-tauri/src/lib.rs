pub mod ai;
pub mod commands;
pub mod config;
pub mod db;
pub mod guard;
pub mod pty;
pub mod secret;
pub mod telegram;

use std::sync::Arc;


use ai::router::AiRouter;
use commands::{
    ai::{ai_chat, ai_query},
    config::{
        get_config, is_onboarding_done, set_default_tab, set_execution_mode, set_max_agent_steps,
        set_onboarding_done, set_submit_shortcut,
    },
    db::{
        db_add_connection, db_connect, db_disconnect, db_execute_query, db_get_table_schema,
        db_list_connections, db_list_schemas, db_list_tables, db_preview_table,
        db_remove_connection, db_test_connection, db_update_connection,
    },
    design::{design_chat, design_list_sessions, design_load_session, design_start_session, design_update_draft, design_list_messages, design_advance_stage, design_save_file},
    provider::{
        add_provider, get_github_copilot_models, get_github_copilot_models_by_provider,
        get_google_ai_models, get_google_ai_models_by_provider,
        get_ollama_models, github_copilot_device_poll, github_copilot_device_start,
        list_providers, remove_provider, set_default_provider,
        test_provider, update_provider,
    },
    secret::{delete_api_key, has_api_key},
    shell::open_url,
};
use config::ConfigStore;
use db::{design::DesignDb, manager::DbManager, Db2SidecarState};
use pty::commands::{
    pty_close, pty_create, pty_get_cwd, pty_get_recent_output, pty_list_dir, pty_read_file,
    pty_resize, pty_write,
};
use pty::PtyManager;
use secret::SecretStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = Arc::new(ConfigStore::new());
    let secrets = Arc::new(SecretStore::new());
    let router = AiRouter::new(config.clone(), secrets.clone());

    let design_db = tauri::async_runtime::block_on(async { DesignDb::new().await });

    let sidecar_path = {
        #[cfg(target_os = "windows")]
        {
            let exe_dir = std::env::current_exe()
                .expect("current_exe")
                .parent()
                .expect("parent dir")
                .to_path_buf();
            let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

            let candidates = [
                // Production: resources bundle flattens db2-sidecar-win-x64/ into app dir
                exe_dir.join("db2-sidecar.exe"),
                // Legacy: externalBin with target triple suffix (kept for compatibility)
                exe_dir.join("db2-sidecar-x86_64-pc-windows-msvc.exe"),
                // Dev: local publish output
                manifest_dir
                    .parent()
                    .expect("workspace root")
                    .join("db2-sidecar")
                    .join("bin")
                    .join("publish-win-x64-nonsingle")
                    .join("db2-sidecar.exe"),
                // Dev: binaries dir
                manifest_dir
                    .join("binaries")
                    .join("db2-sidecar-win-x64")
                    .join("db2-sidecar.exe"),
            ];

            candidates
                .into_iter()
                .find(|p| p.exists())
                .unwrap_or_else(|| exe_dir.join("db2-sidecar.exe"))
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("binaries")
                .join("db2-sidecar-aarch64-apple-darwin")
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("binaries")
                .join("db2-sidecar-x86_64-apple-darwin")
        }
        #[cfg(target_os = "linux")]
        {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("binaries")
                .join("db2-sidecar-x86_64-unknown-linux-gnu")
        }
    };

    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .manage(PtyManager::new())
        .manage(config)
        .manage(secrets)
        .manage(router)
        .manage(DbManager::new())
        .manage(design_db)
        .manage(Db2SidecarState::new(sidecar_path))
        .manage(tokio::sync::Mutex::new(telegram::TelegramState { active_task: None }))
        .setup(|app| {
            telegram::init(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // PTY
            pty_create,
            pty_write,
            pty_resize,
            pty_close,
            pty_get_cwd,
            pty_get_recent_output,
            pty_list_dir,
            pty_read_file,
            // AI query
            ai_query,
            ai_chat,
            // Config
            get_config,
            set_execution_mode,
            set_max_agent_steps,
            is_onboarding_done,
            set_onboarding_done,
            set_submit_shortcut,
            set_default_tab,
            // Provider management
            list_providers,
            add_provider,
            update_provider,
            remove_provider,
            set_default_provider,
            test_provider,
            get_ollama_models,
            github_copilot_device_start,
            github_copilot_device_poll,
            get_github_copilot_models,
            get_github_copilot_models_by_provider,
            get_google_ai_models,
            get_google_ai_models_by_provider,
            // Secrets
            has_api_key,
            delete_api_key,
            // Shell
            open_url,
            // Database
            db_list_connections,
            db_add_connection,
            db_update_connection,
            db_remove_connection,
            db_test_connection,
            db_connect,
            db_disconnect,
            db_list_schemas,
            db_list_tables,
            db_get_table_schema,
            db_preview_table,
            db_execute_query,
            // Design Tab
            design_start_session,
            design_load_session,
            design_list_sessions,
            design_chat,
            design_update_draft,
            design_list_messages,
            design_advance_stage,
            design_save_file,
            // Telegram
            telegram::telegram_get_config,
            telegram::telegram_set_config,
            telegram::telegram_send_message,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
