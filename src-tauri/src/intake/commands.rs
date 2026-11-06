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
pub async fn intake_set_config(app: AppHandle, db: State<'_, IntakeDb>, config: Config) -> Result<(), String> {
    config::save(&db.pool, &config).await.map_err(e)?;
    // 網頁表單的開關／埠號可能變了。
    super::web::restart(&app).await;
    Ok(())
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
pub async fn intake_delete(app: AppHandle, db: State<'_, IntakeDb>, id: String) -> Result<bool, String> {
    let r = store::delete_item(&db.pool, &id).await.map_err(e)?;
    if r {
        super::attach::remove_dir(&super::attach::root(), &id);
    }
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

#[tauri::command]
pub async fn intake_attachments(db: State<'_, IntakeDb>, item_id: String) -> Result<Vec<store::AttachmentRow>, String> {
    store::list_attachments(&db.pool, &item_id).await.map_err(e)
}

/// 讀出附圖內容，回傳 `data:` URL 給前端顯示。只讀我們自己存放目錄底下的檔案。
#[tauri::command]
pub async fn intake_attachment_data(db: State<'_, IntakeDb>, id: String) -> Result<String, String> {
    let row = store::get_attachment(&db.pool, &id).await.map_err(e)?.ok_or("找不到附圖")?;
    data_url(&super::attach::root(), &row)
}

pub fn data_url(root: &std::path::Path, row: &store::AttachmentRow) -> Result<String, String> {
    use base64::Engine;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = std::path::Path::new(&row.path).canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("附圖路徑不在允許的目錄內".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    // 以內容判斷 MIME，不信資料庫裡的欄位。
    let (mime, _) = super::attach::sniff(&bytes).ok_or("檔案內容不是支援的圖片")?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &std::path::Path) -> store::AttachmentRow {
        store::AttachmentRow { id: "a".into(), item_id: "i".into(), filename: "f".into(), path: path.to_string_lossy().into(), mime: "image/png".into(), size: 1 }
    }

    #[test]
    fn serves_images_under_the_root_and_refuses_anything_else() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1];
        let inside = root.path().join("i.png");
        std::fs::write(&inside, png).unwrap();
        assert!(data_url(root.path(), &row(&inside)).unwrap().starts_with("data:image/png;base64,"));

        let elsewhere = outside.path().join("secret.png");
        std::fs::write(&elsewhere, png).unwrap();
        assert!(data_url(root.path(), &row(&elsewhere)).is_err(), "資料庫被竄改也不能讀到目錄外的檔案");

        let sneaky = root.path().join("..").join(outside.path().file_name().unwrap()).join("secret.png");
        assert!(data_url(root.path(), &row(&sneaky)).is_err(), "用 .. 繞出去也不行");

        let text = root.path().join("n.png");
        std::fs::write(&text, b"not an image").unwrap();
        assert!(data_url(root.path(), &row(&text)).is_err(), "副檔名是 png 但內容不是圖片");
    }
}
