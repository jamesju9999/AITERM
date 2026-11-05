//! Tauri 指令：設定、白名單、清單、派送、分頁接收端。
use super::config::{self, Config};
use super::rules::{Level, Sender};
use super::runtime::{self, IntakeRuntime};
use super::store::{self, IntakeItem};
use super::{IntakeDb, CHANGED_EVENT};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

fn e(err: sqlx::Error) -> String {
    err.to_string()
}

fn changed(app: &AppHandle) {
    let _ = app.emit(CHANGED_EVENT, ());
}

#[tauri::command]
pub async fn intake_get_config(db: State<'_, IntakeDb>) -> Result<Config, String> {
    Ok(config::load(&db.pool).await)
}

#[tauri::command]
pub async fn intake_set_config(db: State<'_, IntakeDb>, config: Config) -> Result<(), String> {
    config::save(&db.pool, &config).await.map_err(e)
}

#[tauri::command]
pub async fn intake_list(db: State<'_, IntakeDb>, status: Option<String>) -> Result<Vec<IntakeItem>, String> {
    store::list_items(&db.pool, status.as_deref()).await.map_err(e)
}

#[derive(Serialize, Deserialize)]
pub struct SenderDto {
    pub address: String,
    pub level: Level,
    pub skip_auth: bool,
}

#[tauri::command]
pub async fn intake_senders(db: State<'_, IntakeDb>) -> Result<Vec<SenderDto>, String> {
    Ok(store::list_senders(&db.pool).await.map_err(e)?.into_iter().map(|s| SenderDto { address: s.address, level: s.level, skip_auth: s.skip_auth }).collect())
}

#[tauri::command]
pub async fn intake_upsert_sender(db: State<'_, IntakeDb>, sender: SenderDto) -> Result<(), String> {
    if !super::rules::valid_address(&sender.address) {
        return Err("請輸入有效的電子郵件地址，或 Telegram 聊天室 tg:<chat_id>".into());
    }
    store::upsert_sender(&db.pool, &Sender { address: sender.address, level: sender.level, skip_auth: sender.skip_auth }).await.map_err(e)
}

#[tauri::command]
pub async fn intake_delete_sender(db: State<'_, IntakeDb>, address: String) -> Result<bool, String> {
    store::delete_sender(&db.pool, &address).await.map_err(e)
}

#[tauri::command]
pub async fn intake_dropped(db: State<'_, IntakeDb>) -> Result<Vec<(String, i64)>, String> {
    store::dropped_counts(&db.pool).await.map_err(e)
}

#[tauri::command]
pub async fn intake_approve(app: AppHandle, db: State<'_, IntakeDb>, id: String) -> Result<bool, String> {
    let r = store::approve(&db.pool, &id).await.map_err(e)?;
    changed(&app);
    Ok(r)
}

#[tauri::command]
pub async fn intake_reject(app: AppHandle, db: State<'_, IntakeDb>, id: String) -> Result<bool, String> {
    let r = store::reject(&db.pool, &id).await.map_err(e)?;
    changed(&app);
    Ok(r)
}

#[tauri::command]
pub async fn intake_update_draft(app: AppHandle, db: State<'_, IntakeDb>, id: String, draft: String) -> Result<bool, String> {
    let r = store::update_draft(&db.pool, &id, &draft).await.map_err(e)?;
    changed(&app);
    Ok(r)
}

#[tauri::command]
pub async fn intake_send_reply(app: AppHandle, db: State<'_, IntakeDb>, id: String) -> Result<(), String> {
    let r = runtime::send_reply(&app, &id).await;
    if let Err(msg) = &r {
        store::set_error(&db.pool, &id, &format!("寄出失敗：{msg}")).await.ok();
        changed(&app);
    }
    r
}

/// 這個分頁開始接收問題。
#[tauri::command]
pub async fn intake_tab_start(app: AppHandle, rt: State<'_, Arc<IntakeRuntime>>, tab_id: String) -> Result<(), String> {
    rt.registry.register(&tab_id);
    changed(&app);
    runtime::tick(&app).await;
    Ok(())
}

/// 這個分頁停止接收；進行中的問題退回 pending。
#[tauri::command]
pub async fn intake_tab_stop(app: AppHandle, db: State<'_, IntakeDb>, rt: State<'_, Arc<IntakeRuntime>>, tab_id: String) -> Result<(), String> {
    if let Some(id) = rt.registry.unregister(&tab_id) {
        store::fail(&db.pool, &id, "已停止問題收集").await.map_err(e)?;
    }
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn intake_tabs(rt: State<'_, Arc<IntakeRuntime>>) -> Vec<String> {
    rt.registry.tabs()
}
