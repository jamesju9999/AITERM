//! 寄出草稿。Server 支援 LINE、網頁表單、Email（用信件回覆）與 Telegram。
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
    if item.source == "telegram" {
        let (chat, reply_to) = aiterm_intake_core::telegram_source::reply_target(&item.sender, &item.message_id).ok_or("無法判斷要回覆的 Telegram 聊天室")?;
        let token = st.secrets.get(crate::telegram::TOKEN_SECRET).filter(|t| !t.trim().is_empty()).ok_or("尚未設定 Telegram 機器人")?;
        let base = st.telegram_api_base.lock().clone();
        crate::telegram::send_to_chat(&base, &token, chat, &filter::redact(&body), reply_to).await?;
        store::mark_sent(&st.pools.intake, item_id).await.map_err(|e| e.to_string())?;
        st.events.changed();
        return Ok(());
    }
    if item.source == "email" {
        let settings = crate::mail::load(&st.pools.intake).await;
        let cfg = aiterm_intake_core::config::load(&st.pools.intake, &item.project_id).await;
        // 回覆全部時要去掉我們自己（收信帳號與別名），否則會寄給自己。
        let (also_to, cc) = aiterm_intake_core::smtp::reply_all(&item.sender, item.to_addrs.as_deref().unwrap_or(""), item.cc_addrs.as_deref().unwrap_or(""), &[settings.sender(), cfg.alias.as_str()]);
        // 沒有 Message-ID 的信（用內容雜湊代替）沒有可串的對話。
        let orig_id = if item.message_id.starts_with("nomid:") { "" } else { item.message_id.as_str() };
        crate::mail::send_thread_reply(st, &item.sender, &item.subject, orig_id, &also_to, &cc, &filter::redact(&body)).await?;
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
