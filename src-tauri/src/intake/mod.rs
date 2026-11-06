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
#[cfg(test)]
mod rehearsal;
pub mod rules;
pub mod runtime;
pub mod smtp;
pub mod telegram_source;
pub mod store;
pub mod tabcd;
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

/// 目前開啟中的看板專案（id、名稱），依名稱排序。
pub fn open_projects<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Vec<(String, String)> {
    use tauri::Manager;
    let Some(reg) = app.try_state::<crate::projects::ProjectRegistry>() else { return Vec::new() };
    let mut v: Vec<(String, String)> = reg.all().into_iter().map(|h| (h.id, h.name)).collect();
    v.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    v
}

/// 要參與歸屬判斷的專案設定：每個開啟中的專案，加上仍有舊設定的「未歸屬」（空字串）。
pub(crate) async fn all_configs<R: tauri::Runtime>(app: &tauri::AppHandle<R>, pool: &sqlx::SqlitePool) -> Vec<(String, String, config::Config)> {
    let mut v = Vec::new();
    for (id, name) in open_projects(app) {
        let cfg = config::load(pool, &id).await;
        v.push((id, name, cfg));
    }
    if config::exists(pool, "").await {
        let cfg = config::load(pool, "").await;
        v.push((String::new(), "未歸屬".to_string(), cfg));
    }
    v
}

/// 郵件輪詢每收到一封信就呼叫。依各專案的別名／標籤決定歸屬；沒有專案符合就什麼都不做。
pub async fn on_mail<R: tauri::Runtime>(app: &tauri::AppHandle<R>, account_id: &str, raw: &[u8]) {
    use tauri::{Emitter, Manager};
    let Some(db) = app.try_state::<IntakeDb>() else { return };
    let candidates: Vec<mail_source::Candidate> = all_configs(app, &db.pool)
        .await
        .into_iter()
        .filter(|(_, _, c)| c.enabled && c.account_id == account_id)
        .map(|(id, _, c)| (id, c))
        .collect();
    if candidates.is_empty() {
        return;
    }
    match mail_source::ingest_routed(&db.pool, &candidates, raw, &attach::root()).await {
        Ok(queue::Ingested::Queued(_)) => {
            let _ = app.emit(CHANGED_EVENT, ());
        }
        Ok(_) => {}
        Err(e) => log::warn!("intake: ingest failed for account {account_id}: {e}"),
    }
}

/// 這個郵件帳號目前是不是問題收集專用的（任一專案已啟用並選它為收信帳號）。
pub async fn owns_account<R: tauri::Runtime>(app: &tauri::AppHandle<R>, account_id: &str) -> bool {
    use tauri::Manager;
    let Some(db) = app.try_state::<IntakeDb>() else { return false };
    all_configs(app, &db.pool).await.iter().any(|(_, _, c)| c.enabled && c.account_id == account_id)
}

/// Telegram 收到訊息時呼叫（見 `telegram::poll_updates`）。回傳 `true` 表示訊息已由問題收集處理，
/// 呼叫端不要再當一般訊息轉送。
pub async fn on_telegram<R: tauri::Runtime>(app: &tauri::AppHandle<R>, chat_id: i64, message_id: i64, text: &str, is_owner: bool, photos: Vec<(String, String)>, sender_name: &str) -> bool {
    use tauri::{Emitter, Manager};
    let Some(db) = app.try_state::<IntakeDb>() else { return false };
    let cands: Vec<telegram_source::Candidate> = all_configs(app, &db.pool)
        .await
        .into_iter()
        .map(|(id, name, cfg)| telegram_source::Candidate { id, name, cfg })
        .collect();
    let default_project = config::load_global(&db.pool).await.telegram_default_project;
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
        for (id, name) in photos.into_iter().take(attach::MAX_FILES) {
            match crate::telegram::download_file(&token, &id, &name, attach::MAX_BYTES).await {
                Ok(raw) => out.push(raw),
                Err(e) => log::warn!("intake: Telegram 附件下載失敗：{e}"),
            }
        }
        out
    };
    match telegram_source::route(&db.pool, &cands, &default_project, chat_id, message_id, text, is_owner, &attach::root(), has_files, fetch).await {
        Ok(queue::Ingested::Queued(id)) => {
            store::set_sender_name(&db.pool, &id, sender_name, &telegram_source::sender_key(chat_id)).await.ok();
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
