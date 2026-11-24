//! 寄出草稿。Server 支援 LINE 與網頁表單（用信件回覆）；Email、Telegram 還沒支援。
use crate::state::AppState;
use aiterm_intake_core::host::{Notifier, Secrets};
use aiterm_intake_core::{filter, line_server, line_source, line_store, line_text, store};

pub async fn send(st: &AppState, item_id: &str) -> Result<(), String> {
    let item = store::get_item(&st.pools.intake, item_id).await.map_err(|e| e.to_string())?.ok_or("找不到項目")?;
    if item.status != store::DRAFT {
        return Err("只有草稿可以寄出".into());
    }
    let body = item.reply_draft.clone().unwrap_or_default();
    if body.trim().is_empty() {
        return Err("回覆內容是空的".into());
    }
    if item.source == "web" {
        crate::mail::send_reply(st, &item.sender, &item.subject, &filter::redact(&body)).await?;
        store::mark_sent(&st.pools.intake, item_id).await.map_err(|e| e.to_string())?;
        st.events.changed();
        return Ok(());
    }
    if item.source != "line" {
        return Err(format!("server 目前還不能用「{}」管道回覆", item.source));
    }
    let to = line_source::reply_target(&item.sender).ok_or("無法判斷要回覆的 LINE 使用者")?;
    let token = st.secrets.get(line_server::LINE_TOKEN_ID).filter(|t| !t.trim().is_empty()).ok_or("尚未設定 LINE 的 Channel access token")?;
    let text = line_text::to_plain(&filter::redact(&body));
    let base = st.line_api_base.lock().clone();
    line_server::push_to(&base, &token, &to, &text).await?;
    // 回到群組的內容也記進前情，下次開題時 AI 看得到自己說過什麼。
    if line_source::is_chat_key(&item.sender) {
        let new = line_store::NewMessage { chat: &item.sender, msg_id: &format!("ai-{item_id}"), user_id: "", name: "AI 助理", text: &text, ts: chrono::Utc::now().timestamp(), outgoing: true };
        line_store::insert_message(&st.pools.intake, &new).await.ok();
    }
    store::mark_sent(&st.pools.intake, item_id).await.map_err(|e| e.to_string())?;
    st.events.changed();
    Ok(())
}
