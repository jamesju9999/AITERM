//! Agent 問題（`ask_human`）的收件匣指令，以及 App 與 Telegram 共用的「回答」流程。
//! 見 docs/superpowers/specs/2026-10-16-agent-ask-human-design.md。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::projects::ProjectRegistry;
use crate::tasks::questions::{self, QuestionRow};
use crate::tasks::store;

use super::tasks::project;

/// 收件匣的一列：問題＋它屬於哪個專案、哪張卡、哪個 Agent。
#[derive(Debug, Serialize)]
pub struct QuestionView {
    pub project_id: String,
    pub project_name: String,
    pub task_title: String,
    pub agent_name: Option<String>,
    #[serde(flatten)]
    pub question: QuestionRow,
}

/// 所有已開啟專案裡還沒回答的問題（新的在前）。
#[tauri::command]
pub async fn questions_list(reg: State<'_, ProjectRegistry>) -> Result<Vec<QuestionView>, String> {
    let mut out = Vec::new();
    for p in reg.all() {
        let open = questions::list_open(&p.pool).await.map_err(|e| e.to_string())?;
        for q in open {
            let task_title = store::get_task(&p.pool, &q.task_id).await.ok().flatten().map(|t| t.title).unwrap_or_default();
            let agent_name = match q.agent_id.as_deref() {
                Some(aid) => crate::tasks::agents::get_agent(&p.pool, aid).await.ok().flatten().map(|a| a.name),
                None => None,
            };
            out.push(QuestionView { project_id: p.id.clone(), project_name: p.name.clone(), task_title, agent_name, question: q });
        }
    }
    out.sort_by(|a, b| b.question.created_at.cmp(&a.question.created_at));
    Ok(out)
}

#[tauri::command]
pub async fn questions_answer(app: AppHandle, project_id: String, id: i64, answer: String) -> Result<(), String> {
    deliver_answer(&app, &project_id, id, &answer, "app").await.map(|_| ())
}

/// 回答一題並送回提問的 Agent：記下答案、在它的分頁打字送出、沒有其他未答問題就解除等決策、
/// 通知畫面、把 Telegram 上的原訊息標成已回答。App 與 Telegram 都走這裡。
pub async fn deliver_answer(app: &AppHandle, project_id: &str, id: i64, answer: &str, via: &str) -> Result<QuestionRow, String> {
    let reg = app.state::<ProjectRegistry>();
    let p = project(&reg, project_id)?;
    let q = questions::get(&p.pool, id).await.map_err(|e| e.to_string())?.ok_or("找不到這個問題")?;
    if q.status != questions::OPEN {
        return Err(format!("{} 已經回答過或已取消", q.label()));
    }
    let task = store::get_task(&p.pool, &q.task_id).await.map_err(|e| e.to_string())?.ok_or("找不到提問的卡片")?;
    let tab_id = match (task.status.as_str(), task.tab_id.as_deref()) {
        (store::STATUS_RUNNING, Some(tab)) => tab.to_string(),
        _ => return Err("提問的卡片已經結束，答案送不回去".into()),
    };
    // Codex 卡片（`codex exec`）沒有對話可以打字：答案交給掛著等的 ask_human 請求。
    // 沒人在等（例如 App 重開過）就不能往 shell 打字，回錯誤。
    let quota = app.try_state::<Arc<crate::tasks::quota::QuotaState>>();
    let blocking = quota.as_ref().is_some_and(|qs| qs.is_blocking_ask(&q.task_id));
    if blocking && !quota.as_ref().is_some_and(|qs| qs.has_waiter(&q.task_id, id)) {
        return Err("Agent 目前沒有在等這個答案".into());
    }
    let row = questions::answer(&p.pool, id, answer, via)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{} 已經回答過或已取消", q.label()))?;

    if blocking {
        let given = quota.as_ref().is_some_and(|qs| qs.give_answer(&q.task_id, id, &questions::answer_text(&row)));
        if !given {
            return Err("Agent 目前沒有在等這個答案".into());
        }
    } else {
        let pty = app.state::<Arc<crate::pty::PtyManager>>();
        let text = questions::answer_message(&row, &crate::pty::session::done_marker_instruction(&tab_id));
        crate::tasks::dispatch::write_then_submit(&pty, &tab_id, &text).await?;
    }

    if !questions::has_open(&p.pool, &q.task_id).await.unwrap_or(false) {
        if let Some(quota) = app.try_state::<Arc<crate::tasks::quota::QuotaState>>() {
            quota.set_asking(&q.task_id, false);
        }
    }
    let _ = app.emit("questions-updated", ());
    if let Some(mid) = row.telegram_message_id {
        crate::telegram::mark_question_closed(app, mid, &row, row.answer.as_deref());
    }
    Ok(row)
}

/// Telegram 按鈕：`callback_data` 指到的選項就是答案。
pub async fn answer_from_telegram_button(app: &AppHandle, data: &str) -> Result<QuestionRow, String> {
    let (project_id, id, idx) = crate::telegram::questions::parse_callback(data).ok_or("不是問題的按鈕")?;
    let reg = app.state::<ProjectRegistry>();
    let p = project(&reg, &project_id)?;
    let q = questions::get(&p.pool, id).await.map_err(|e| e.to_string())?.ok_or("找不到這個問題")?;
    let option = q.option_list().get(idx).cloned().ok_or("沒有這個選項")?;
    deliver_answer(app, &project_id, id, &option, "telegram").await
}

/// Telegram「回覆」某則訊息：如果那是問題訊息，就是在回答它。不是問題訊息回 `Ok(None)`。
pub async fn answer_from_telegram_reply(app: &AppHandle, replied_message_id: i64, text: &str) -> Result<Option<QuestionRow>, String> {
    let projects = app.state::<ProjectRegistry>().all();
    for p in projects {
        if let Some(q) = questions::by_telegram_message(&p.pool, replied_message_id).await.map_err(|e| e.to_string())? {
            return deliver_answer(app, &p.id, q.id, text, "telegram").await.map(Some);
        }
    }
    Ok(None)
}
