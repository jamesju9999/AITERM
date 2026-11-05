//! 執行層：把登記表、佇列與實際的終端機分頁、郵件帳號接起來。
use super::{config, dispatcher, filter, rules, smtp, store, IntakeDb, CHANGED_EVENT};
use crate::claude_activity::ClaudeActivity;
use crate::config::ConfigStore;
use crate::pty::PtyManager;
use crate::secret::SecretStore;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub struct IntakeRuntime {
    pub registry: dispatcher::Registry,
}

fn changed(app: &AppHandle) {
    let _ = app.emit(CHANGED_EVENT, ());
}

/// 在 `setup` 裡呼叫一次：掛 hook 觀察者並啟動派送迴圈。
pub fn install(app: &AppHandle, activity: &Arc<ClaudeActivity>) {
    let rt = Arc::new(IntakeRuntime { registry: dispatcher::Registry::default() });
    app.manage(rt.clone());

    let (h, r) = (app.clone(), rt.clone());
    activity.set_observer(Arc::new(move |sid, kind, body| match kind {
        "prompt" => {
            if let Some(p) = body.get("prompt").and_then(|v| v.as_str()) {
                r.registry.on_prompt(sid, p);
            }
        }
        "turn-end" => {
            if let Some(item_id) = r.registry.on_turn_end(sid) {
                let (h, body) = (h.clone(), body.clone());
                tauri::async_runtime::spawn(async move {
                    complete(&h, &item_id, dispatcher::answer_from_stop(&body)).await;
                });
            }
        }
        _ => {}
    }));

    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            tick(&h).await;
        }
    });
}

/// 一次掃描：清掉已關閉的分頁，再把可派的問題派給閒置分頁。
pub async fn tick(app: &AppHandle) {
    let (Some(db), Some(rt), Some(pty)) = (
        app.try_state::<IntakeDb>(),
        app.try_state::<Arc<IntakeRuntime>>(),
        app.try_state::<Arc<PtyManager>>(),
    ) else {
        return;
    };
    let mut touched = false;
    for tab in rt.registry.tabs() {
        if pty.size(&tab).is_none() {
            if let Some(id) = rt.registry.unregister(&tab) {
                store::fail(&db.pool, &id, "接收的分頁已關閉").await.ok();
            }
            touched = true;
        }
    }
    for tab in rt.registry.idle_tabs() {
        let item = match store::claim_next(&db.pool, &tab).await {
            Ok(Some(i)) => i,
            Ok(None) => break,
            Err(e) => {
                log::warn!("intake: claim failed: {e}");
                break;
            }
        };
        rt.registry.assign(&tab, &item.id);
        touched = true;
        if let Err(e) = crate::tasks::dispatch::write_then_submit(&pty, &tab, &dispatcher::build_prompt(&item)).await {
            rt.registry.release(&tab);
            store::fail(&db.pool, &item.id, &format!("無法送進終端機：{e}")).await.ok();
        }
    }
    if touched {
        changed(app);
    }
}

/// Agent 這一輪結束：存回覆，必要時自動寄出。
pub async fn complete(app: &AppHandle, item_id: &str, answer: Option<String>) {
    let Some(db) = app.try_state::<IntakeDb>() else { return };
    let Some(answer) = answer else {
        store::fail(&db.pool, item_id, "無法取得 Agent 的回答").await.ok();
        changed(app);
        return;
    };
    let reply = filter::redact(&answer);
    // 一律先存成草稿；是否自動寄出另外判斷，寄失敗時草稿還在。
    if !matches!(store::finish(&db.pool, item_id, &reply, false).await, Ok(true)) {
        return;
    }
    let cfg = config::load(&db.pool).await;
    if let Ok(Some(item)) = store::get_item(&db.pool, item_id).await {
        let auto = if item.sender_level == "internal" { cfg.reply_internal_auto } else { cfg.reply_external_auto };
        if auto {
            if let Err(e) = send_reply(app, item_id).await {
                store::set_error(&db.pool, item_id, &format!("自動寄出失敗：{e}")).await.ok();
            }
        }
    }
    changed(app);
}

/// 寄出草稿（`draft → done`）。
pub async fn send_reply(app: &AppHandle, item_id: &str) -> Result<(), String> {
    let db = app.try_state::<IntakeDb>().ok_or("intake 未就緒")?;
    let item = store::get_item(&db.pool, item_id).await.map_err(|e| e.to_string())?.ok_or("找不到項目")?;
    if item.status != store::DRAFT {
        return Err("只有草稿可以寄出".into());
    }
    let body = item.reply_draft.clone().unwrap_or_default();
    if body.trim().is_empty() {
        return Err("回覆內容是空的".into());
    }
    let cfg = config::load(&db.pool).await;
    let account = app
        .state::<Arc<ConfigStore>>()
        .get()
        .mail_accounts
        .into_iter()
        .find(|a| a.id == cfg.account_id)
        .ok_or("找不到收集用的郵件帳號")?;
    let password = app
        .state::<Arc<SecretStore>>()
        .get(&crate::mail::poller::mail_secret_key(&account.id))
        .map_err(|e| e.to_string())?
        .ok_or("這個帳號沒有儲存密碼")?;
    let orig_id = if item.message_id.starts_with("nomid:") { "" } else { item.message_id.as_str() };
    let msg = smtp::build_reply(&smtp::Outgoing {
        from: &account.email,
        to: &item.sender,
        orig_subject: &item.subject,
        orig_message_id: orig_id,
        // 使用者編輯過的草稿也再過一次，避免手誤貼進金鑰。
        body: &filter::redact(&body),
    })?;
    smtp::send(&account.smtp_host, account.smtp_port, &account.username, &password, msg).await?;
    store::mark_sent(&db.pool, item_id).await.map_err(|e| e.to_string())?;
    changed(app);
    Ok(())
}

pub fn level_of(item: &store::IntakeItem) -> rules::Level {
    if item.sender_level == "internal" { rules::Level::Internal } else { rules::Level::External }
}
