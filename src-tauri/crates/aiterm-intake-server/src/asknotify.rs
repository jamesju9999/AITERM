//! 中途問人的通知：Telegram（主人聊天室，附按鈕、可回覆作答）、Email（只通知）、提醒。
//! 通知失敗只記錄，不影響提問（畫布上仍看得到）。
use std::sync::Arc;
use std::time::Duration;

use aiterm_intake_core::board_questions::{self as bq, BoardQuestion};
use aiterm_intake_core::host::{Notifier, Secrets};
use aiterm_intake_core::store;
use serde_json::{json, Value};

use crate::state::AppState;

const AS_VIA: &str = "telegram";

/// 主人聊天室與機器人（都設好才會通知）。
async fn telegram_target(st: &AppState) -> Option<(String, String, i64)> {
    let token = st.secrets.get(crate::telegram::TOKEN_SECRET).filter(|t| !t.trim().is_empty())?;
    let owner = crate::telegram::load(&st.pools.intake).await.owner_chat_id.trim().parse::<i64>().ok()?;
    let base = st.telegram_api_base.lock().clone();
    Some((base, token, owner))
}

async fn notify_email(st: &AppState) -> Option<String> {
    store::get_setting(&st.pools.intake, crate::routes_ask::NOTIFY_EMAIL_KEY).await.ok().flatten().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

async fn project_name(st: &AppState, project_id: &str) -> String {
    crate::routes_data::project_list(st).await.into_iter().find(|p| p.id == project_id).map(|p| p.name).unwrap_or_else(|| project_id.to_string())
}

pub fn question_text(project: &str, q: &BoardQuestion) -> String {
    let mut s = format!("❓ {}｜{}\n{}", q.label(), project, q.question);
    if let Some(c) = q.context.as_deref().filter(|c| !c.is_empty()) {
        s.push_str(&format!("\n\n背景：{c}"));
    }
    if let Some(r) = q.recommended.as_deref().filter(|r| !r.is_empty()) {
        s.push_str(&format!("\n建議：{r}"));
    }
    if q.option_list().is_empty() {
        s.push_str("\n\n請「回覆」這則訊息來回答。");
    }
    s
}

/// 選項按鈕，一列一個。`callback_data = q|<問題 id>|<選項序號>`（Telegram 限 64 bytes）。
pub fn keyboard(q: &BoardQuestion) -> Option<Value> {
    let opts = q.option_list();
    if opts.is_empty() {
        return None;
    }
    let rows: Vec<Value> = opts.iter().enumerate().map(|(i, o)| json!([{"text": o, "callback_data": format!("q|{}|{i}", q.id)}])).collect();
    Some(json!({"inline_keyboard": rows}))
}

pub fn parse_callback(data: &str) -> Option<(i64, usize)> {
    let mut it = data.split('|');
    (it.next()? == "q").then_some(())?;
    let id = it.next()?.parse().ok()?;
    let idx = it.next()?.parse().ok()?;
    it.next().is_none().then_some((id, idx))
}

async fn tg_call(base: &str, token: &str, method: &str, body: Value) -> Result<Value, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let res = client.post(format!("{base}/bot{token}/{method}")).json(&body).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Telegram {method} 失敗：HTTP {}", res.status()));
    }
    res.json().await.map_err(|e| e.to_string())
}

/// 新問題：通知出去（背景執行，不擋住 Agent 的工具呼叫）。
pub async fn opened(st: &Arc<AppState>, q: &BoardQuestion) {
    let (st, q) = (st.clone(), q.clone());
    tokio::spawn(async move {
        let project = project_name(&st, &q.project_id).await;
        if let Some((base, token, owner)) = telegram_target(&st).await {
            let mut body = json!({"chat_id": owner, "text": question_text(&project, &q)});
            if let Some(k) = keyboard(&q) {
                body["reply_markup"] = k;
            }
            match tg_call(&base, &token, "sendMessage", body).await {
                Ok(v) => {
                    if let Some(id) = v.pointer("/result/message_id").and_then(|m| m.as_i64()) {
                        bq::set_telegram_message(&st.pools.intake, q.id, id).await.ok();
                    }
                }
                Err(e) => log::warn!("問題 {} 的 Telegram 通知失敗：{e}", q.label()),
            }
        }
        if let Some(to) = notify_email(&st).await {
            let body = format!("{}\n\n請到 AITerm 的共用看板回答（Email 只能通知，不能回覆作答）。", question_text(&project, &q));
            if let Err(e) = crate::mail::send_plain(&st, &to, &format!("[AITerm] Agent 需要你決定：{}", q.label()), &body).await {
                log::warn!("問題 {} 的 Email 通知失敗：{e}", q.label());
            }
        }
    });
}

fn closed_text(q: &BoardQuestion) -> String {
    match q.status.as_str() {
        bq::ANSWERED => format!("✅ {} 已回答：{}（{}）\n\n{}", q.label(), q.answer.as_deref().unwrap_or(""), q.answered_by.as_deref().unwrap_or(""), q.question),
        _ => format!("🚫 {} 已取消\n\n{}", q.label(), q.question),
    }
}

/// 問題結束（已答、取消）：把 Telegram 上那則訊息改成結果並拿掉按鈕。
pub async fn closed_one(st: &Arc<AppState>, q: &BoardQuestion) {
    let Some(mid) = q.telegram_message_id else { return };
    let Some((base, token, owner)) = telegram_target(st).await else { return };
    let body = json!({"chat_id": owner, "message_id": mid, "text": closed_text(q), "reply_markup": {"inline_keyboard": []}});
    if let Err(e) = tg_call(&base, &token, "editMessageText", body).await {
        log::warn!("更新問題 {} 的 Telegram 訊息失敗：{e}", q.label());
    }
}

/// 一張卡結束，它還在等的問題被關了。
pub async fn closed(st: &Arc<AppState>, card_id: &str) {
    st.events.changed();
    let rows: Vec<BoardQuestion> = sqlx::query_as("SELECT * FROM board_questions WHERE card_id = ? AND status = 'cancelled' AND telegram_message_id IS NOT NULL").bind(card_id).fetch_all(&st.pools.intake).await.unwrap_or_default();
    for q in rows {
        closed_one(st, &q).await;
    }
}

/// 主人在 Telegram 上回答：按選項按鈕。回傳是否為問題的回呼（是的話呼叫端不必再處理）。
pub async fn handle_callback(st: &Arc<AppState>, base: &str, token: &str, owner: Option<i64>, cb: &Value) -> bool {
    let Some((id, idx)) = cb["data"].as_str().and_then(parse_callback) else { return false };
    let cb_id = cb["id"].as_str().unwrap_or("").to_string();
    let chat = cb.pointer("/message/chat/id").and_then(|c| c.as_i64());
    let ack = |text: &str| {
        let (b, t, i, text) = (base.to_string(), token.to_string(), cb_id.clone(), text.to_string());
        async move {
            let _ = tg_call(&b, &t, "answerCallbackQuery", json!({"callback_query_id": i, "text": text})).await;
        }
    };
    if owner.is_none() || chat != owner {
        ack("你沒有權限回答這個問題").await;
        return true;
    }
    let Ok(Some(q)) = bq::get(&st.pools.intake, id).await else {
        ack("找不到這個問題").await;
        return true;
    };
    let Some(choice) = q.option_list().get(idx).cloned() else {
        ack("這個選項已經不存在").await;
        return true;
    };
    match bq::answer(&st.pools.intake, id, &choice, "Telegram（主人）", AS_VIA).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, "Telegram（主人）", "board.question.answer", &q.label()).await;
            st.events.changed();
            ack("已送出").await;
            if let Ok(Some(row)) = bq::get(&st.pools.intake, id).await {
                closed_one(st, &row).await;
            }
        }
        _ => ack("這個問題已經有人回答了或已取消").await,
    }
    true
}

/// 主人在 Telegram 上「回覆」問題那則訊息：回覆文字就是答案。是問題的回覆（不論成功與否）回 `true`。
pub async fn handle_reply(st: &Arc<AppState>, owner: Option<i64>, chat_id: i64, reply_to: Option<i64>, text: &str) -> bool {
    let Some(mid) = reply_to else { return false };
    let Ok(Some(q)) = bq::by_telegram_message(&st.pools.intake, mid).await else { return false };
    if owner != Some(chat_id) || text.trim().is_empty() {
        return true;
    }
    if bq::answer(&st.pools.intake, q.id, text.trim(), "Telegram（主人）", AS_VIA).await.unwrap_or(false) {
        crate::audit::record(&st.pools.server, "Telegram（主人）", "board.question.answer", &q.label()).await;
        st.events.changed();
        if let Ok(Some(row)) = bq::get(&st.pools.intake, q.id).await {
            closed_one(st, &row).await;
        }
    }
    true
}

/// 提醒一次：沒人回答的問題，每 30 分鐘提醒一次、最多 3 次。回傳提醒了幾個。
pub async fn remind_once(st: &Arc<AppState>) -> usize {
    let now = crate::db::now();
    let mut n = 0;
    for q in bq::all_open(&st.pools.intake).await.unwrap_or_default() {
        let last = q.last_reminded_at.unwrap_or(q.created_at);
        if q.reminders >= bq::MAX_REMINDERS || now - last < bq::REMIND_EVERY_SECS {
            continue;
        }
        bq::mark_reminded(&st.pools.intake, q.id).await.ok();
        n += 1;
        let project = project_name(st, &q.project_id).await;
        let text = format!("⏰ 提醒：{} 還沒有人回答（第 {} 次提醒）\n{}", q.label(), q.reminders + 1, q.question);
        if let Some((base, token, owner)) = telegram_target(st).await {
            let mut body = json!({"chat_id": owner, "text": text});
            if let Some(mid) = q.telegram_message_id {
                body["reply_parameters"] = json!({"message_id": mid, "allow_sending_without_reply": true});
            }
            if let Err(e) = tg_call(&base, &token, "sendMessage", body).await {
                log::warn!("問題 {} 的 Telegram 提醒失敗：{e}", q.label());
            }
        }
        if let Some(to) = notify_email(st).await {
            let _ = crate::mail::send_plain(st, &to, &format!("[AITerm] 提醒：{} 還沒有人回答", q.label()), &format!("{text}\n\n專案：{project}")).await;
        }
    }
    n
}

/// 每分鐘看一次有沒有該提醒的問題。
pub fn start_reminders(st: &Arc<AppState>) {
    let st = st.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            remind_once(&st).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(options: &[&str]) -> BoardQuestion {
        BoardQuestion { id: 12, project_id: "P".into(), card_id: "c".into(), agent_id: None, question: "用哪個？".into(), options: serde_json::to_string(options).unwrap(), recommended: Some("A".into()), context: None, status: "open".into(), answer: None, answered_by: None, answered_via: None, telegram_message_id: None, reminders: 0, last_reminded_at: None, created_at: 0, answered_at: None }
    }

    #[test]
    fn buttons_fit_telegrams_limit_and_round_trip() {
        let k = keyboard(&q(&["甲", "乙"])).unwrap();
        let data = k["inline_keyboard"][1][0]["callback_data"].as_str().unwrap();
        assert!(data.len() <= 64);
        assert_eq!(parse_callback(data), Some((12, 1)));
        assert!(keyboard(&q(&[])).is_none());
        for bad in ["", "x|1|2", "q|a|1", "q|1", "q|1|2|3"] {
            assert_eq!(parse_callback(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_question_without_options_asks_for_a_reply() {
        assert!(question_text("專案", &q(&[])).contains("回覆"));
        assert!(!question_text("專案", &q(&["甲"])).contains("回覆這則"));
    }
}
