//! Agent 的問題（`ask_human`）送到 Telegram：問題＋選項按鈕；使用者按按鈕或「回覆」
//! 那則訊息就是回答。見 docs/superpowers/specs/2026-10-16-agent-ask-human-design.md §4。

use std::sync::Arc;

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::config::ConfigStore;
use crate::secret::SecretStore;
use crate::tasks::questions::QuestionRow;

/// 按鈕上的文字最多幾個字（Telegram 按鈕太長會被截掉）。
const BUTTON_CHARS: usize = 40;

/// 送到 Telegram 的問題內容。
pub fn question_text(project_name: &str, title: &str, q: &QuestionRow) -> String {
    let mut s = format!("❓ {project_name} — {title}\n{}：{}", q.label(), q.question);
    if let Some(c) = q.context.as_deref() {
        s.push_str(&format!("\n\n背景：{c}"));
    }
    if let Some(r) = q.recommended.as_deref() {
        s.push_str(&format!("\n建議：{r}"));
    }
    s.push_str(if q.option_list().is_empty() {
        "\n\n請「回覆」這則訊息打字回答。"
    } else {
        "\n\n按下選項回答，或「回覆」這則訊息打字回答。"
    });
    s
}

/// 選項按鈕，一列一個。`callback_data` = `q|<專案 id>|<問題 id>|<選項序號>`（Telegram 限 64 bytes）。
pub fn keyboard(project_id: &str, q: &QuestionRow) -> Option<Value> {
    let options = q.option_list();
    if options.is_empty() {
        return None;
    }
    let rows: Vec<Value> = options
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let star = if q.recommended.as_deref() == Some(o.as_str()) { "★ " } else { "" };
            let text: String = format!("{star}{o}").chars().take(BUTTON_CHARS).collect();
            json!([{ "text": text, "callback_data": format!("q|{project_id}|{}|{i}", q.id) }])
        })
        .collect();
    Some(json!({ "inline_keyboard": rows }))
}

/// 解析按鈕的 `callback_data`：(專案 id, 問題 id, 選項序號)。
pub fn parse_callback(data: &str) -> Option<(String, i64, usize)> {
    let mut it = data.split('|');
    if it.next()? != "q" {
        return None;
    }
    let project = it.next()?.to_string();
    let id = it.next()?.parse().ok()?;
    let idx = it.next()?.parse().ok()?;
    it.next().is_none().then_some((project, id, idx))
}

/// 回答或作廢之後，原本那則訊息改成的內容（按鈕一併拿掉）。
pub fn closed_text(q: &QuestionRow, answer: Option<&str>) -> String {
    match answer {
        Some(a) => format!("{}：{}\n\n✅ 已回答：{a}", q.label(), q.question),
        None => format!("{}：{}\n\n⏹ 卡片已結束，問題取消", q.label(), q.question),
    }
}

/// 提醒的內容（回覆在原本的問題訊息底下）。
pub fn reminder_text(q: &QuestionRow) -> String {
    format!("⏰ {} 還在等你回答：{}", q.label(), q.question)
}

/// 設定好的 bot token 與聊天室；沒設定就是 `None`（不送 Telegram）。
pub fn credentials(app: &AppHandle) -> Option<(String, String)> {
    let token = app.state::<Arc<SecretStore>>().get(super::TELEGRAM_SECRET_ID).ok().flatten()?;
    let chat = app.state::<Arc<ConfigStore>>().get().telegram_chat_id?;
    (!token.trim().is_empty() && !chat.trim().is_empty()).then_some((token, chat))
}

async fn call(token: &str, method: &str, body: Value) -> Result<Value, String> {
    let url = format!("https://api.telegram.org/bot{token}/{method}");
    let res = reqwest::Client::new().post(url).json(&body).send().await.map_err(|e| e.to_string())?;
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(v.get("description").and_then(Value::as_str).unwrap_or("Telegram API error").to_string());
    }
    Ok(v)
}

/// 送出問題，回傳 Telegram 的 message_id。沒設定 Telegram 就是 `Ok(None)`。
pub async fn send_question(app: &AppHandle, project_id: &str, project_name: &str, title: &str, q: &QuestionRow) -> Result<Option<i64>, String> {
    let Some((token, chat)) = credentials(app) else { return Ok(None) };
    let mut body = json!({ "chat_id": chat, "text": question_text(project_name, title, q) });
    if let Some(k) = keyboard(project_id, q) {
        body["reply_markup"] = k;
    }
    let v = call(&token, "sendMessage", body).await?;
    Ok(v["result"]["message_id"].as_i64())
}

/// 回覆在原本的問題訊息底下提醒一次。
pub async fn send_reminder(app: &AppHandle, q: &QuestionRow) -> Result<(), String> {
    let Some((token, chat)) = credentials(app) else { return Ok(()) };
    let mut body = json!({ "chat_id": chat, "text": reminder_text(q) });
    if let Some(mid) = q.telegram_message_id {
        body["reply_to_message_id"] = json!(mid);
    }
    call(&token, "sendMessage", body).await.map(|_| ())
}

/// 把原本的問題訊息改成已回答／已取消（拿掉按鈕）。失敗只記 log：訊息可能已被刪除。
pub fn mark_question_closed(app: &AppHandle, message_id: i64, q: &QuestionRow, answer: Option<&str>) {
    let Some((token, chat)) = credentials(app) else { return };
    let text = closed_text(q, answer);
    tauri::async_runtime::spawn(async move {
        let body = json!({ "chat_id": chat, "message_id": message_id, "text": text });
        if let Err(e) = call(&token, "editMessageText", body).await {
            log::warn!("telegram editMessageText {message_id}: {e}");
        }
    });
}

/// 按鈕按下後一定要回 `answerCallbackQuery`，不然 Telegram 的按鈕會一直轉圈。
pub async fn ack_callback(token: &str, callback_id: &str, text: &str) {
    if let Err(e) = call(token, "answerCallbackQuery", json!({ "callback_query_id": callback_id, "text": text })).await {
        log::warn!("telegram answerCallbackQuery: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(options: &[&str], recommended: Option<&str>) -> QuestionRow {
        QuestionRow {
            id: 12,
            task_id: "t".into(),
            agent_id: None,
            question: "名稱用中文？".into(),
            options: serde_json::to_string(&options).unwrap(),
            recommended: recommended.map(str::to_string),
            context: Some("README 標題".into()),
            status: "open".into(),
            answer: None,
            answered_via: None,
            telegram_message_id: None,
            reminders: 0,
            last_reminded_at: None,
            created_at: 0,
            answered_at: None,
        }
    }

    const PROJECT: &str = "0d6f1c2a-1111-2222-3333-444455556666";

    #[test]
    fn the_message_names_project_card_question_and_how_to_answer() {
        let text = question_text("Pomodoro", "番茄鐘網頁小工具", &row(&["中文", "英文"], Some("英文")));
        assert!(text.starts_with("❓ Pomodoro — 番茄鐘網頁小工具\nQ-12：名稱用中文？"), "{text}");
        assert!(text.contains("背景：README 標題") && text.contains("建議：英文") && text.contains("按下選項"));
        assert!(question_text("P", "T", &row(&[], None)).contains("「回覆」這則訊息打字回答"));
    }

    #[test]
    fn buttons_carry_project_question_and_option_within_telegram_limits() {
        let k = keyboard(PROJECT, &row(&["中文", "英文"], Some("英文"))).unwrap();
        let rows = k["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][0]["text"], "★ 英文", "建議的選項標出來");
        let data = rows[1][0]["callback_data"].as_str().unwrap();
        assert!(data.len() <= 64, "Telegram callback_data 最多 64 bytes：{}", data.len());
        assert_eq!(parse_callback(data), Some((PROJECT.to_string(), 12, 1)));
        assert!(keyboard(PROJECT, &row(&[], None)).is_none(), "沒有選項就不放按鈕");
        let long = "很長".repeat(40);
        let k = keyboard(PROJECT, &row(&[long.as_str()], None)).unwrap();
        assert!(k["inline_keyboard"][0][0]["text"].as_str().unwrap().chars().count() <= BUTTON_CHARS);
    }

    #[test]
    fn foreign_callback_data_is_ignored() {
        for bad in ["", "x|p|1|0", "q|p|abc|0", "q|p|1", "q|p|1|0|extra"] {
            assert_eq!(parse_callback(bad), None, "{bad}");
        }
    }

    #[test]
    fn closing_and_reminding_texts() {
        let q = row(&["中文"], None);
        assert!(closed_text(&q, Some("中文")).ends_with("✅ 已回答：中文"));
        assert!(closed_text(&q, None).contains("問題取消"));
        assert_eq!(reminder_text(&q), "⏰ Q-12 還在等你回答：名稱用中文？");
    }
}
