pub mod ai;
pub mod api_docs;
pub mod commands;
pub mod config;
pub mod db;
pub mod enterprise;
pub mod guard;
pub mod pty;
pub mod secret;
pub mod telegram;
pub mod vcs;

use std::sync::Arc;
use tokio::sync::Mutex;

use ai::router::AiRouter;
use commands::{
    api_docs::{
        api_docs_auth_status, api_docs_detect, api_docs_extract,
        api_docs_fetch_tree, api_docs_login, api_docs_logout,
    },
    ai::{ai_chat, ai_query},
    config::{
        get_config, is_onboarding_done, set_default_tab, set_execution_mode, set_max_agent_steps,
        set_onboarding_done, set_submit_shortcut,
    },
    enterprise::{
        enterprise_accept_task, enterprise_complete_task, enterprise_install_service,
        enterprise_on_complete, enterprise_register_device, enterprise_reject_task,
        enterprise_update_task_progress,
    },
    db::{
        db_add_connection, db_connect, db_disconnect, db_execute_query, db_get_table_schema,
        db_list_connections, db_list_schemas, db_list_tables, db_preview_table,
        db_remove_connection, db_test_connection, db_update_connection,
    },
    design::{design_chat, design_list_sessions, design_load_session, design_start_session, design_update_draft, design_list_messages, design_advance_stage, design_save_file, design_delete_session},
    markitdown::{markitdown_convert, markitdown_pick_file},
    provider::{
        add_provider, get_github_copilot_models, get_github_copilot_models_by_provider,
        get_google_ai_models, get_google_ai_models_by_provider,
        get_ollama_models, github_copilot_device_poll, github_copilot_device_start,
        list_providers, remove_provider, set_default_provider,
        test_provider, update_provider,
    },
    secret::{delete_api_key, has_api_key},
    shell::open_url,
    web::{web_fetch, web_search},
    vcs::{
        pick_folder, vcs_add_connection, vcs_agent_step, vcs_detect_repo, vcs_list_connections,
        vcs_query, vcs_remove_connection, vcs_test_connection, vcs_update_connection,
    },
};
use config::ConfigStore;
use db::{design::DesignDb, manager::DbManager, Db2SidecarState};
use enterprise::agent::EnterpriseTaskState;
use enterprise::task_runner::VcsCredentialManager;
use pty::commands::{
    pty_close, pty_create, pty_get_cwd, pty_get_recent_output, pty_get_shell_type,
    pty_list_dir, pty_read_file, pty_resize, pty_write, read_file_as_bytes, write_text_file,
};
use pty::PtyManager;
use secret::SecretStore;

/// Headless worker entry point — no Tauri GUI, just the enterprise agent loop.
pub fn run_headless() {
    let config = Arc::new(ConfigStore::new());
    let secrets = Arc::new(SecretStore::new());

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(enterprise::headless::run_headless(config, secrets));
}

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
        #[cfg(target_os = "macos")]
        {
            let exe_dir = std::env::current_exe()
                .expect("current_exe")
                .parent()
                .expect("parent dir")
                .to_path_buf();

            let contents_dir = exe_dir.parent()
                .expect("Contents dir")
                .to_path_buf();

            let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

            #[cfg(target_arch = "aarch64")]
            let dev_subdir = "db2-sidecar-mac-arm64";
            #[cfg(target_arch = "x86_64")]
            let dev_subdir = "db2-sidecar-mac-x64";

            let candidates = [
                // Production: Tauri resources land in Contents/Resources/db2-sidecar/
                contents_dir.join("Resources").join("db2-sidecar"),
                // Dev: local build output
                manifest_dir
                    .join("binaries")
                    .join(dev_subdir),
            ];

            candidates
                .into_iter()
                .find(|p| p.join("db2sidecar.jar").exists())
                .unwrap_or_else(|| contents_dir.join("Resources").join("db2-sidecar"))
        }
        #[cfg(target_os = "linux")]
        {
            let exe_dir = std::env::current_exe()
                .expect("current_exe")
                .parent()
                .expect("parent dir")
                .to_path_buf();

            let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

            #[cfg(target_arch = "aarch64")]
            let dev_subdir = "db2-sidecar-linux-arm64";
            #[cfg(target_arch = "x86_64")]
            let dev_subdir = "db2-sidecar-linux-x64";

            // Production: AppImage ← {APPDIR}/usr/bin/aiterm → {APPDIR}/usr/lib/aiterm/db2-sidecar
            //             .deb    ← /usr/bin/aiterm           → /usr/lib/aiterm/db2-sidecar
            let prod_path = exe_dir.join("../../lib/AITerm/db2-sidecar");

            let found = [
                prod_path.clone(),
                // Dev: local binaries directory (CARGO_MANIFEST_DIR is a compile-time path
                // valid only on the build machine — used only when prod_path doesn't exist)
                manifest_dir.join("binaries").join(dev_subdir),
            ]
            .into_iter()
            .find(|p| p.join("db2sidecar.jar").exists());

            // Default to prod_path so error messages reference the expected on-device location,
            // not the CI build machine's CARGO_MANIFEST_DIR.
            found.unwrap_or(prod_path)
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
        .manage(Arc::new(Mutex::new(VcsCredentialManager::new())))
        .manage(Arc::new(Mutex::new(EnterpriseTaskState::new())))
        .manage(tokio::sync::Mutex::new(telegram::TelegramState { active_task: None }))
        .setup(|app| {
            telegram::init(app.handle());
            enterprise::agent::init(app.handle());
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
            pty_get_shell_type,
            pty_list_dir,
            pty_read_file,
            read_file_as_bytes,
            write_text_file,
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
            // Web
            web_search,
            web_fetch,
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
            design_delete_session,
            // Enterprise
            enterprise_accept_task,
            enterprise_reject_task,
            enterprise_register_device,
            enterprise_update_task_progress,
            enterprise_complete_task,
            enterprise_on_complete,
            enterprise_install_service,
            // Telegram
            telegram::telegram_get_config,
            telegram::telegram_set_config,
            telegram::telegram_send_message,
            // VCS
            vcs_list_connections,
            vcs_add_connection,
            vcs_update_connection,
            vcs_remove_connection,
            vcs_test_connection,
            vcs_detect_repo,
            vcs_query,
            vcs_agent_step,
            pick_folder,
            // MarkItDown
            markitdown_convert,
            markitdown_pick_file,
            // API Docs
            api_docs_detect,
            api_docs_fetch_tree,
            api_docs_extract,
            api_docs_login,
            api_docs_logout,
            api_docs_auth_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
