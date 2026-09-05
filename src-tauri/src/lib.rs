pub mod ai;
pub mod appimage_env;
pub mod bridge;
pub mod chatgpt_web;
pub mod code_assistant;
pub mod api_docs;
pub mod commands;
pub mod config;
pub mod db;
pub mod document_convert;
pub mod enterprise;
pub mod guard;
pub mod knowledge_base;
pub mod mail;
pub mod mcp;
pub mod mcp_server;
pub mod pty;
pub mod projects;
pub mod python_env;
pub mod secret;
pub mod share;
pub mod tasks;
pub mod telegram;
pub mod usage;
pub mod vcs;

use std::sync::Arc;
use tokio::sync::Mutex;

use ai::router::AiRouter;
use commands::{
    api_docs::{
        api_docs_auth_status, api_docs_detect, api_docs_extract,
        api_docs_fetch_tree, api_docs_login, api_docs_logout,
    },
    appimage::{appimage_integrate, appimage_integration_state, appimage_remove_integration},
    ai::{agent_chat, ai_chat, ai_chat_ctx, ai_query},
    bridge::{bridge_apply, bridge_set_config, bridge_status},
    mcp_server::{mcp_tool_server_apply, mcp_tool_server_set_config, mcp_tool_server_status},
    claude_notif::{claude_notif_enable_bell, claude_notif_needs_prompt},
    code_assistant::code_assistant_chat,
    knowledge_base::{
        kb_create_notebook, kb_list_notebooks, kb_delete_notebook, kb_sync_notebook, kb_chat, kb_open_document,
        kb_create_chat_session, kb_list_chat_sessions, kb_load_chat_session, kb_delete_chat_session,
        kb_list_embedding_models,
    },
    config::{
        get_config, is_appimage_integration_declined, is_claude_notif_declined, is_onboarding_done,
        set_appimage_integration_declined, set_claude_notif_declined,
        set_default_tab, set_doc_convert_engine, set_execution_mode, set_max_agent_steps,
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
    db_export::{
        db_check_import_file, db_export_connections, db_import_connections, db_preview_import,
    },
    design::{design_chat, design_list_sessions, design_load_session, design_start_session, design_update_draft, design_list_messages, design_advance_stage, design_save_file, design_delete_session},
    exec::agent_exec,
    loop_session::{loop_session_save, loop_session_list, loop_session_load, loop_session_delete, loop_session_clear_all, loop_project_pick_open, loop_project_pick_save},
    mail::{
        mail_add_account, mail_remove_account, mail_list_accounts,
        mail_list_messages, mail_mark_read, mail_count_unread, mail_test_connection,
        mail_delete_message,
    },
    doc_convert::{document_convert, document_convert_pick_file},
    python_env::{
        python_env_status, python_env_ensure, python_env_reset, python_env_set_interpreter,
        python_env_set_index_url,
    },
    mcp::{
        list_mcp_servers, add_mcp_server, update_mcp_server, remove_mcp_server,
        get_mcp_tools, execute_mcp_tool, import_claude_desktop_mcp, set_mcp_enabled,
        install_mcp_package, McpManagerState,
    },
    provider::{
        add_provider, get_github_copilot_models, get_github_copilot_models_by_provider,
        get_google_ai_models, get_google_ai_models_by_provider,
        get_ollama_models, github_copilot_device_poll, github_copilot_device_start,
        list_providers, remove_provider, set_default_provider,
        test_provider, update_provider,
        anthropic_oauth_start, anthropic_oauth_complete, anthropic_oauth_logout,
        get_anthropic_oauth_models,
        google_oauth_login, google_oauth_logout, get_google_oauth_models,
        get_openrouter_models, get_openrouter_models_by_provider,
        get_xai_models, get_xai_models_by_provider,
        get_deepseek_models, get_deepseek_models_by_provider,
        get_kimi_models, get_kimi_models_by_provider,
        codex_oauth_login, codex_oauth_logout, get_codex_oauth_models,
        AnthropicOAuthState,
    },
    secret::{delete_api_key, has_api_key},
    share::{
        share_approve, share_deny, share_discover, share_kick, share_pending,
        share_revoke_control, share_start, share_status, share_stop, share_viewers,
    },
    share_viewer::{share_viewer_connect, share_viewer_disconnect, share_viewer_send},
    shell::open_url,
    projects::{
        projects_create, projects_list, projects_open, projects_remove, projects_rename,
    },
    reports::{reports_list, reports_read, reports_save},
    task_board_config::{task_board_get_config, task_board_set_config},
    tasks::{
        tasks_list, tasks_create, tasks_update, tasks_move, tasks_stop, tasks_delete,
        tasks_add_attachment, tasks_remove_attachment, tasks_clone, tasks_read_transcript,
        tasks_save_transcript, tasks_mark_done, tasks_used_dirs, tasks_set_summary,
    },
    updater::updater_supported,
    web::{web_fetch, web_search, npm_mcp_search},
    vcs::{
        pick_folder, vcs_add_connection, vcs_agent_abort_step, vcs_agent_step, vcs_check_overlap,
        vcs_detect_repo, vcs_finish_feature, vcs_get_block_info, vcs_get_default_branch,
        vcs_get_feature_diff, vcs_list_active_features, vcs_list_connections, vcs_merge_feature,
        vcs_query, vcs_remove_connection, vcs_start_feature, vcs_test_connection,
        vcs_update_connection, VcsAgentStepRegistry,
    },
};
use config::ConfigStore;
use db::{design::DesignDb, loop_sessions::LoopSessionDb, mail::MailDb, manager::DbManager, Db2SidecarState};
use enterprise::agent::EnterpriseTaskState;
use enterprise::task_runner::VcsCredentialManager;
use mail::manager::MailState;
use pty::commands::{
    pty_close, pty_create, pty_get_cwd, pty_get_recent_output, pty_get_shell_type,
    pty_list_dir, pty_read_file, pty_resize, pty_write, read_file_as_bytes, write_text_file,
    write_pasted_file, list_drives,
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

/// 工作看板的專案：先做一次性的舊資料搬遷，再把設定裡記錄的每個專案
/// 資料夾開起來。某個專案開不起來不擋住其他專案——`projects_list` 會把
/// 它的狀態回報給前端。
async fn load_projects(config: Arc<ConfigStore>) -> projects::ProjectRegistry {
    let reg = projects::ProjectRegistry::new();

    // 舊資料搬遷：只在第一次執行，複製而非搬移。
    match projects::migrate::migrate_legacy(&tasks::app_data_dir()).await {
        Ok(Some(dest)) => {
            let p = dest.to_string_lossy().into_owned();
            let _ = config.update(|cfg| {
                if !cfg.task_board.project_paths.contains(&p) {
                    cfg.task_board.project_paths.push(p);
                }
            });
        }
        Ok(None) => {}
        Err(e) => log::error!("legacy task migration failed: {e}"),
    }

    for path in config.get().task_board.project_paths {
        if let Err(e) = reg.open_folder(std::path::Path::new(&path)).await {
            log::warn!("open project {path}: {e}");
        }
    }
    reg
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = Arc::new(ConfigStore::new());
    let secrets = Arc::new(SecretStore::new());

    // 這 5 個資料庫彼此獨立、互不相依（各自開自己的連線、建自己的表）。
    // 原本用 5 個分開的 block_on 依序執行，等於白白疊加 5 次的開連線／
    // 建表延遲，而且整段都發生在 tauri::Builder 建立之前——app 連「開始
    // 建立視窗」都還沒開始。改成同一個 block_on 裡用 tokio::join! 平行跑。
    // 工作看板的專案（搬遷＋開啟每個專案的 tasks.db）也一起併進來。
    let (usage_store, design_db, loop_session_db, kb_db, mail_db, project_registry) =
        tauri::async_runtime::block_on(async {
            tokio::join!(
                async {
                    let s = usage::UsageStore::new().await;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    s.prune_expired(now).await;
                    s
                },
                DesignDb::new(),
                LoopSessionDb::new(),
                db::knowledge_base::KnowledgeBaseDb::new(),
                MailDb::new(),
                load_projects(config.clone()),
            )
        });
    let usage_store = Arc::new(usage_store);

    let router = AiRouter::new(config.clone(), secrets.clone(), usage_store.clone());

    // McpManager 本身的建立不涉及 I/O（純記憶體），可以立刻建好、.manage()
    // 進去；真正連線每個設定好的 MCP 伺服器（connect_all，常常要 spawn
    // 子行程，可能很慢）挪到 .setup() 裡背景做——跟下面 bridge server／
    // MCP tool server 同一個模式（失敗只記 log，不擋 app 啟動）。
    // mcp_enabled 預設是 true，這一步原本是本函式最大宗的啟動延遲來源：
    // 只要設定裡有任何 MCP 伺服器，視窗要等它們全部連完才會出現。
    let mcp_manager: McpManagerState = Arc::new(tokio::sync::Mutex::new(mcp::McpManager::new()));

    let sidecar_path = db::resolve_db2_sidecar_path();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .manage(Arc::new(PtyManager::new()))
        .manage(config)
        .manage(secrets)
        .manage(router)
        .manage(usage_store)
        .manage(Arc::new(usage::quota::cache::QuotaCache::new()))
        .manage(DbManager::new())
        .manage(design_db)
        .manage(loop_session_db)
        .manage(kb_db)
        .manage(mail_db)
        .manage(project_registry)
        .manage(tokio::sync::Mutex::new(MailState::new()))
        .manage(Db2SidecarState::new(sidecar_path))
        .manage(Arc::new(Mutex::new(VcsCredentialManager::new())))
        .manage(VcsAgentStepRegistry::new())
        .manage(Arc::new(Mutex::new(EnterpriseTaskState::new())))
        .manage(tokio::sync::Mutex::new(telegram::TelegramState { active_task: None }))
        .manage(mcp_manager)
        .manage(AnthropicOAuthState::new())
        .manage(Arc::new(bridge::BridgeState::new()))
        .manage(Arc::new(mcp_server::McpToolServerState::new()))
        .manage(Arc::new(share::ShareServerState::new()))
        .manage(Arc::new(share::viewer_manager::ViewerManager::new()))
        .setup(|app| {
            telegram::init(app.handle());
            mail::poller::init(app.handle());
            enterprise::agent::init(app.handle());
            commands::appimage::repair_integration_on_startup();

            // ChatGPT Web 供應商的傳輸層。這裡只是把 AppHandle 存起來——
            // webview 要到第一個請求進來時才建立。
            chatgpt_web::session::init(app.handle().clone());

            // MCP 伺服器連線：見上面 mcp_manager 建立處的說明，原本同步
            // connect_all 挪到這裡背景做，不擋視窗顯示。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    let config = handle.state::<Arc<ConfigStore>>().inner().clone();
                    let cfg = config.get();
                    if !cfg.mcp_enabled {
                        return;
                    }
                    let manager = handle.state::<McpManagerState>().inner().clone();
                    let mut manager = manager.lock().await;
                    manager.connect_all(&cfg.mcp_servers).await;
                });
            }

            // 橋接 server：設定為 enabled 時隨 app 啟動。失敗只記 log 不擋啟動
            // ——埠被占用不該讓整個 app 起不來，設定頁會顯示錯誤。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    let bridge = handle.state::<Arc<bridge::BridgeState>>().inner().clone();
                    let config = handle.state::<Arc<ConfigStore>>().inner().clone();
                    let secrets = handle.state::<Arc<SecretStore>>().inner().clone();
                    let cfg = config.get().claude_bridge;
                    if !cfg.enabled {
                        return;
                    }
                    let token = match secrets.get(bridge::auth::BRIDGE_TOKEN_KEY) {
                        Ok(Some(t)) if !t.is_empty() => t,
                        _ => {
                            let t = bridge::auth::generate_token();
                            if let Err(e) = secrets.set(bridge::auth::BRIDGE_TOKEN_KEY, &t) {
                                log::error!("bridge token 寫入 keychain 失敗：{e}");
                                return;
                            }
                            t
                        }
                    };
                    if let Err(e) = bridge.start(config, secrets, token, cfg.port).await {
                        log::error!("bridge server 啟動失敗：{e}");
                    }
                });
            }

            // MCP tool server：設定為 enabled 時隨 app 啟動。失敗只記 log 不擋啟動，
            // 理由同橋接 server——埠被占用不該讓整個 app 起不來。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    let server = handle.state::<Arc<mcp_server::McpToolServerState>>().inner().clone();
                    let config = handle.state::<Arc<ConfigStore>>().inner().clone();
                    let secrets = handle.state::<Arc<SecretStore>>().inner().clone();
                    let pty_manager = handle.state::<Arc<PtyManager>>().inner().clone();
                    let cfg = config.get().mcp_tool_server;
                    if !cfg.enabled {
                        return;
                    }
                    let token = match secrets.get(mcp_server::MCP_TOOL_SERVER_TOKEN_KEY) {
                        Ok(Some(t)) if !t.is_empty() => t,
                        _ => {
                            let t = bridge::auth::generate_token();
                            if let Err(e) = secrets.set(mcp_server::MCP_TOOL_SERVER_TOKEN_KEY, &t) {
                                log::error!("mcp tool server token 寫入 keychain 失敗：{e}");
                                return;
                            }
                            t
                        }
                    };
                    if let Err(e) = server.start(config, secrets, token, cfg.port, Some(handle.clone()), pty_manager).await {
                        log::error!("mcp tool server 啟動失敗：{e}");
                    }
                });
            }

            // Task board scheduler: long-lived, dispatches queued cards to
            // `claude`; also runs a one-time recovery scan on start. The
            // returned handle is managed so tasks_move / tasks_stop can poke
            // it and abort running watches.
            {
                use tauri::Manager;
                let handle = app.handle().clone();
                let sched = tasks::scheduler::spawn(handle);
                app.manage(sched);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 用量／配額
            commands::usage::usage_quota,
            commands::usage::usage_quota_all,
            commands::usage::usage_summary,
            // ChatGPT Web
            chatgpt_web::session::chatgpt_web_take,
            chatgpt_web::session::chatgpt_web_chunk,
            chatgpt_web::session::chatgpt_web_logged_in,
            chatgpt_web::session::chatgpt_web_login,
            chatgpt_web::session::chatgpt_web_models,
            // PTY
            pty_create,
            pty_write,
            pty_resize,
            pty_close,
            pty_get_cwd,
            pty_get_recent_output,
            pty_get_shell_type,
            pty_list_dir,
            list_drives,
            pty_read_file,
            read_file_as_bytes,
            write_text_file,
            write_pasted_file,
            // AI query
            ai_query,
            ai_chat_ctx,
            ai_chat,
            agent_chat,
            code_assistant_chat,
            agent_exec,
            // Knowledge Base
            kb_create_notebook,
            kb_list_embedding_models,
            kb_list_notebooks,
            kb_delete_notebook,
            kb_sync_notebook,
            kb_chat,
            kb_open_document,
            kb_create_chat_session,
            kb_list_chat_sessions,
            kb_load_chat_session,
            kb_delete_chat_session,
            // Mail
            mail_add_account,
            mail_remove_account,
            mail_list_accounts,
            mail_list_messages,
            mail_mark_read,
            mail_count_unread,
            mail_test_connection,
            mail_delete_message,
            // Config
            get_config,
            set_execution_mode,
            set_max_agent_steps,
            is_onboarding_done,
            set_onboarding_done,
            is_appimage_integration_declined,
            set_appimage_integration_declined,
            is_claude_notif_declined,
            set_claude_notif_declined,
            claude_notif_needs_prompt,
            claude_notif_enable_bell,
            set_submit_shortcut,
            set_doc_convert_engine,
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
            anthropic_oauth_start,
            anthropic_oauth_complete,
            anthropic_oauth_logout,
            get_anthropic_oauth_models,
            google_oauth_login,
            google_oauth_logout,
            get_google_oauth_models,
            codex_oauth_login,
            codex_oauth_logout,
            get_codex_oauth_models,
            get_openrouter_models,
            get_openrouter_models_by_provider,
            get_xai_models,
            get_xai_models_by_provider,
            get_deepseek_models,
            get_deepseek_models_by_provider,
            get_kimi_models,
            get_kimi_models_by_provider,
            // Secrets
            has_api_key,
            delete_api_key,
            // Claude Code bridge
            bridge_status,
            bridge_apply,
            bridge_set_config,
            mcp_tool_server_status,
            mcp_tool_server_apply,
            mcp_tool_server_set_config,
            // Remote terminal sharing
            share_start,
            share_stop,
            share_status,
            share_pending,
            share_approve,
            share_deny,
            share_viewers,
            share_discover,
            share_kick,
            share_revoke_control,
            share_viewer_connect,
            share_viewer_send,
            share_viewer_disconnect,
            // Shell
            open_url,
            updater_supported,
            appimage_integration_state,
            appimage_integrate,
            appimage_remove_integration,
            // Web
            web_search,
            web_fetch,
            npm_mcp_search,
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
            db_check_import_file,
            db_export_connections,
            db_preview_import,
            db_import_connections,
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
            // Loop Sessions
            loop_session_save,
            loop_session_list,
            loop_session_load,
            loop_session_delete,
            loop_session_clear_all,
            loop_project_pick_open,
            loop_project_pick_save,
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
            vcs_get_block_info,
            vcs_agent_step,
            vcs_agent_abort_step,
            vcs_list_active_features,
            vcs_check_overlap,
            vcs_get_default_branch,
            vcs_start_feature,
            vcs_finish_feature,
            vcs_get_feature_diff,
            vcs_merge_feature,
            pick_folder,
            // Document conversion (anydoc + MarkItDown)
            document_convert,
            document_convert_pick_file,
            // Python environment
            python_env_status,
            python_env_ensure,
            python_env_reset,
            python_env_set_interpreter,
            python_env_set_index_url,
            // MCP
            list_mcp_servers,
            add_mcp_server,
            update_mcp_server,
            remove_mcp_server,
            get_mcp_tools,
            execute_mcp_tool,
            import_claude_desktop_mcp,
            set_mcp_enabled,
            install_mcp_package,
            // API Docs
            api_docs_detect,
            api_docs_fetch_tree,
            api_docs_extract,
            api_docs_login,
            api_docs_logout,
            api_docs_auth_status,
            // Task board
            task_board_get_config,
            task_board_set_config,
            tasks_list,
            tasks_create,
            tasks_update,
            tasks_move,
            tasks_stop,
            tasks_mark_done,
            tasks_delete,
            tasks_add_attachment,
            tasks_remove_attachment,
            tasks_clone,
            tasks_read_transcript,
            tasks_save_transcript,
            tasks_used_dirs,
            tasks_set_summary,
            // 專案
            projects_list,
            projects_create,
            projects_open,
            projects_remove,
            projects_rename,
            // 工作報告
            reports_save,
            reports_list,
            reports_read,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Mail tasks are the one background task that holds an open,
            // authenticated socket essentially all the time: with IMAP IDLE
            // they park *inside* a live session rather than sleeping between
            // connections. Quitting without this hook abandons one session per
            // account with no LOGOUT, and a provider keeps an abandoned IDLE
            // session until its own autologout (~30 minutes for Gmail) while
            // capping concurrent connections per account (~15) — so a handful
            // of quick restarts, i.e. an ordinary debugging session, is enough
            // to lock the account out of IMAP entirely.
            //
            // `Exit` rather than `ExitRequested`: the latter can still be
            // cancelled, and doing the logout there would tear down live
            // connections for a quit that never happens.
            if let tauri::RunEvent::Exit = event {
                tauri::async_runtime::block_on(mail::poller::stop_all(app_handle));
            }
        });
}
