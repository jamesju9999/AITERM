//! 問題收集（Intake）：外部問題進獨立佇列，循序派給終端機裡的 AI Agent。
//! 設計：docs/superpowers/specs/2026-11-05-intake-queue-design.md
pub mod config;
pub mod dispatcher;
pub mod filter;
pub mod mail_source;
pub mod rules;
pub mod smtp;
pub mod store;

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
    match mail_source::ingest(&db.pool, &cfg, raw).await {
        Ok(mail_source::Ingested::Queued(_)) => {
            let _ = app.emit(CHANGED_EVENT, ());
        }
        Ok(_) => {}
        Err(e) => log::warn!("intake: ingest failed for account {account_id}: {e}"),
    }
}
