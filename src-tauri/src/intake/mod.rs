//! 問題收集（Intake）：外部問題進獨立佇列，循序派給終端機裡的 AI Agent。
//! 設計：docs/superpowers/specs/2026-11-05-intake-queue-design.md
pub mod attach;
pub mod classify;
pub mod commands;
pub mod config;
pub mod dispatcher;
pub mod filter;
pub mod mail_source;
pub mod queue;
pub mod rules;
pub mod runtime;
pub mod smtp;
pub mod telegram_source;
pub mod store;
pub mod verify;
pub mod web;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use std::path::PathBuf;

pub const CHANGED_EVENT: &str = "intake://changed";

pub struct IntakeDb {
    pub pool: SqlitePool,
}

impl IntakeDb {
    pub async fn new() -> Self {
        let dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM");
        std::fs::create_dir_all(&dir).ok();
        let opts = SqliteConnectOptions::new().filename(dir.join("intake.db")).create_if_missing(true);
        let pool = SqlitePool::connect_with(opts)
            .await
            .unwrap_or_else(|_| SqlitePool::connect_lazy("sqlite::memory:").unwrap());
        store::init_schema(&pool).await.ok();
        // 上次中斷留下的 running 退回 pending。
        store::recover_interrupted(&pool).await.ok();
        Self { pool }
    }
}

/// 郵件輪詢每收到一封信就呼叫。未啟用或不是指定帳號時什麼都不做。
pub async fn on_mail(app: &tauri::AppHandle, account_id: &str, raw: &[u8]) {
    use tauri::{Emitter, Manager};
    let Some(db) = app.try_state::<IntakeDb>() else { return };
    let cfg = config::load(&db.pool).await;
    if !cfg.enabled || cfg.account_id != account_id {
        return;
    }
    match mail_source::ingest(&db.pool, &cfg, raw, &attach::root()).await {
        Ok(queue::Ingested::Queued(_)) => {
            let _ = app.emit(CHANGED_EVENT, ());
        }
        Ok(_) => {}
        Err(e) => log::warn!("intake: ingest failed for account {account_id}: {e}"),
    }
}

/// 這個郵件帳號目前是不是問題收集專用的（已啟用且被選為收信帳號）。
pub async fn owns_account(app: &tauri::AppHandle, account_id: &str) -> bool {
    use tauri::Manager;
    let Some(db) = app.try_state::<IntakeDb>() else { return false };
    let cfg = config::load(&db.pool).await;
    cfg.enabled && cfg.account_id == account_id
}

/// Telegram 收到訊息時呼叫（見 `telegram::poll_updates`）。回傳 `true` 表示訊息已由問題收集處理，
/// 呼叫端不要再當一般訊息轉送。
pub async fn on_telegram(app: &tauri::AppHandle, chat_id: i64, message_id: i64, text: &str, is_owner: bool, photos: Vec<String>) -> bool {
    use tauri::{Emitter, Manager};
    let Some(db) = app.try_state::<IntakeDb>() else { return false };
    let cfg = config::load(&db.pool).await;
    let fetch_app = app.clone();
    let has_files = !photos.is_empty();
    let fetch = move || async move {
        let Some(token) = fetch_app
            .try_state::<std::sync::Arc<crate::secret::SecretStore>>()
            .and_then(|s| s.get(crate::telegram::TELEGRAM_SECRET_ID).ok().flatten())
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for id in photos.into_iter().take(attach::MAX_FILES) {
            match crate::telegram::download_file(&token, &id, attach::MAX_BYTES).await {
                Ok(raw) => out.push(raw),
                Err(e) => log::warn!("intake: Telegram 圖片下載失敗：{e}"),
            }
        }
        out
    };
    match telegram_source::ingest(&db.pool, &cfg, chat_id, message_id, text, is_owner, &attach::root(), has_files, fetch).await {
        Ok(queue::Ingested::Queued(_)) => {
            let _ = app.emit(CHANGED_EVENT, ());
            true
        }
        Ok(queue::Ingested::Duplicate) => true,
        Ok(queue::Ingested::Dropped(reason)) => reason != "not_for_us",
        Err(e) => {
            log::warn!("intake: telegram ingest failed: {e}");
            false
        }
    }
}
