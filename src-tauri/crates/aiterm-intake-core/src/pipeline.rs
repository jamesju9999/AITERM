//! 收進佇列之後、派工之前的流程：LINE 群組的「新問題自動開題」。AITERM 與 server 共用。
use crate::host::AiComplete;
use crate::telegram_source::Candidate;
use crate::{config, line_source, line_store, queue, rules, store};
use sqlx::SqlitePool;
use std::path::Path;
use std::time::Duration;

/// 群組裡客戶一批訊息是不是新問題；沒有 AI、逾時或失敗一律當新問題（寧可多開，不漏客戶）。
pub async fn ai_is_new_problem(ai: &dyn AiComplete, text: &str) -> bool {
    tokio::time::timeout(Duration::from_secs(30), crate::classify::is_new_problem(ai, text)).await.ok().and_then(|r| r.ok()).unwrap_or(true)
}

/// LINE 群組：客戶沒 @ 的新問題自動開題。等客戶講完（靜默）、我方人員還沒回、該群組沒有未結束的題目、AI 判斷是新問題，才開。
/// 回傳有沒有開出新題。
pub async fn line_triage(pool: &SqlitePool, cands: &[Candidate], ai: &dyn AiComplete, attach_root: &Path) -> bool {
    let retention = config::load_global(pool).await.line_retention_days;
    let now = chrono::Utc::now().timestamp();
    line_store::purge_expired(pool, retention, now).await.ok();
    let Ok(chats) = line_store::chats_with_unhandled(pool).await else { return false };
    if chats.is_empty() {
        return false;
    }
    let mut opened = false;
    for key in chats {
        // 這個群組在哪個啟用 LINE 的專案白名單裡（只有一個才處理）。
        let mut hits = Vec::new();
        for c in cands.iter().filter(|c| c.cfg.line_enabled) {
            let senders = store::list_senders(pool, &c.id).await.unwrap_or_default();
            if let Some(s) = senders.iter().find(|s| rules::normalize(&s.address) == key) {
                // 測試模式：沒有「我方人員」，所有人（含自己）都當客戶。
                let staff: std::collections::HashSet<String> = senders
                    .iter()
                    .filter(|_| !c.cfg.line_test_mode)
                    .filter(|x| x.level == rules::Level::Internal)
                    .filter_map(|x| rules::normalize(&x.address).strip_prefix("line:").filter(|id| id.starts_with('u')).map(str::to_string))
                    .collect();
                hits.push((c, s.level, staff));
            }
        }
        let [(project, level, staff)] = hits.as_slice() else { continue };
        // 沒開自動開題：把累積的訊息標成已處理，免得之後才打開時一次湧出一堆舊題。
        if !project.cfg.line_auto_open {
            if let Ok(all) = line_store::unhandled(pool, &key).await {
                if let Some(last) = all.last() {
                    line_store::mark_handled(pool, &key, last.id).await.ok();
                }
            }
            continue;
        }
        let Ok(line_source::Triage::Candidate { messages, upto }) = line_source::triage_chat(pool, &key, staff, line_source::QUIET_SECS, now).await else { continue };
        // 已經有一題在處理：不再開新題（補充請客戶 @ 一次）。
        if store::has_open_for_sender(pool, &project.id, &key).await.unwrap_or(false) {
            line_store::mark_handled(pool, &key, upto).await.ok();
            continue;
        }
        let text = messages.iter().map(|m| format!("{}: {}", if m.name.is_empty() { "customer" } else { &m.name }, m.text)).collect::<Vec<_>>().join("\n");
        if !ai_is_new_problem(ai, &text).await {
            line_store::mark_handled(pool, &key, upto).await.ok();
            continue;
        }
        let mut group = line_store::get_name(pool, key.strip_prefix("line:").unwrap_or(&key)).await.ok().flatten().unwrap_or_default();
        if group.is_empty() {
            group = store::sender_label(pool, &project.id, &key).await.ok().flatten().unwrap_or_default();
        }
        match line_source::open_candidate(pool, project, *level, &key, &group, messages, upto, now, attach_root).await {
            Ok(queue::Ingested::Queued(_)) => opened = true,
            Ok(_) => {}
            Err(e) => log::warn!("intake: LINE 自動開題失敗：{e}"),
        }
    }
    opened
}
