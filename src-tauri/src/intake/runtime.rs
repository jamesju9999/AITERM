//! 執行層：把登記表、佇列與實際的終端機分頁、郵件帳號接起來。
use super::{classify, config, dispatcher, filter, rules, smtp, store, verify, IntakeDb, CHANGED_EVENT};
use crate::ai::router::AiRouter;
use crate::claude_activity::ClaudeActivity;
use crate::config::ConfigStore;
use crate::pty::PtyManager;
use crate::secret::SecretStore;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub struct IntakeRuntime {
    pub registry: dispatcher::Registry,
    /// 派送迴圈與「開始接收」指令都會呼叫 `tick`；不序列化的話兩邊可能同時看到同一個閒置分頁。
    tick_lock: tokio::sync::Mutex<()>,
}

fn changed<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit(CHANGED_EVENT, ());
}

/// 在 `setup` 裡呼叫一次：掛 hook 觀察者並啟動派送迴圈。
pub fn install<R: Runtime>(app: &AppHandle<R>, activity: &Arc<ClaudeActivity>) {
    let rt = Arc::new(IntakeRuntime { registry: dispatcher::Registry::default(), tick_lock: tokio::sync::Mutex::new(()) });
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
                // 驗證（跑測試）可能要好幾分鐘：先把分頁標成忙碌，免得下一題在測試還在跑時就開始改檔。
                r.registry.assign(sid, &item_id);
                let (h, body, tab) = (h.clone(), body.clone(), sid.to_string());
                tauri::async_runtime::spawn(async move {
                    complete(&h, &tab, &item_id, dispatcher::answer_from_stop(&body)).await;
                });
            }
        }
        _ => {}
    }));

    app.manage(Arc::new(super::web::WebServer::default()));
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        super::web::restart(&h).await;
    });

    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            tick(&h).await;
        }
    });
}

/// 一次掃描：清掉已關閉的分頁，再把可派的問題派給閒置分頁。
pub async fn tick<R: Runtime>(app: &AppHandle<R>) {
    let (Some(db), Some(rt), Some(pty)) = (
        app.try_state::<IntakeDb>(),
        app.try_state::<Arc<IntakeRuntime>>(),
        app.try_state::<Arc<PtyManager>>(),
    ) else {
        return;
    };
    let _guard = rt.tick_lock.lock().await;
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
        let project = rt.registry.project_of(&tab).unwrap_or_default();
        let item = match store::claim_next(&db.pool, &project, &tab).await {
            Ok(Some(i)) => i,
            Ok(None) => continue, // 這個專案沒有題目；別的專案的分頁還要看
            Err(e) => {
                log::warn!("intake: claim failed: {e}");
                break;
            }
        };
        rt.registry.assign(&tab, &item.id);
        touched = true;
        prepare(app, &db.pool, &pty, &item, &tab).await;
        if let Err(e) = {
            let images = store::list_attachments(&db.pool, &item.id).await.unwrap_or_default();
            crate::tasks::dispatch::write_then_submit(&pty, &tab, &dispatcher::build_prompt(&item, &images)).await
        } {
            rt.registry.release(&tab);
            store::fail(&db.pool, &item.id, &format!("無法送進終端機：{e}")).await.ok();
        }
    }
    if touched {
        changed(app);
    }
}

/// 派工前：初篩分類；「要改程式」的記下 git HEAD 供事後驗證。任何失敗都退成「要改程式」。
async fn prepare<R: Runtime>(app: &AppHandle<R>, pool: &sqlx::SqlitePool, pty: &PtyManager, item: &store::IntakeItem, tab: &str) {
    let kind = match app.try_state::<AiRouter>() {
        Some(router) => match router.resolve().await {
            Ok(provider) => tokio::time::timeout(Duration::from_secs(30), classify::classify(provider, &item.subject, &item.body))
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or(classify::Kind::CodeChange),
            Err(_) => classify::Kind::CodeChange,
        },
        None => classify::Kind::CodeChange,
    };
    let head = if kind == classify::Kind::CodeChange {
        match pty.get_cwd(tab) {
            Some(cwd) => verify::snapshot(&cwd).await,
            None => None,
        }
    } else {
        None
    };
    store::set_kind(pool, &item.id, kind.as_str(), head.as_deref()).await.ok();
}

/// Agent 這一輪結束：（要改程式的）驗證、存回覆，必要時自動寄出，最後釋放分頁。
pub async fn complete<R: Runtime>(app: &AppHandle<R>, tab: &str, item_id: &str, answer: Option<String>) {
    finish_item(app, tab, item_id, answer).await;
    if let Some(rt) = app.try_state::<Arc<IntakeRuntime>>() {
        rt.registry.release(tab);
    }
    changed(app);
}

async fn finish_item<R: Runtime>(app: &AppHandle<R>, tab: &str, item_id: &str, answer: Option<String>) {
    let Some(db) = app.try_state::<IntakeDb>() else { return };
    let Some(answer) = answer else {
        store::fail(&db.pool, item_id, "無法取得 Agent 的回答").await.ok();
        return;
    };
    let Ok(Some(item)) = store::get_item(&db.pool, item_id).await else { return };
    let cfg = config::load(&db.pool, &item.project_id).await;

    let mut reply = filter::redact(&answer);
    let mut verified_ok = true;
    if item.kind.as_deref() == Some(classify::Kind::CodeChange.as_str()) {
        let cwd = app.try_state::<Arc<PtyManager>>().and_then(|p| p.get_cwd(tab)).unwrap_or_default();
        let report = verify::verify(&cwd, item.snapshot_head.as_deref(), &cfg.verify_command, Duration::from_secs(cfg.verify_timeout_secs.max(1))).await;
        verified_ok = report.passed();
        store::set_verification(&db.pool, item_id, &report.text(), verified_ok).await.ok();
        reply = format!("{reply}\n\n{}", filter::redact(&report.text()));
    }

    // 一律先存成草稿；是否自動寄出另外判斷，寄失敗時草稿還在。
    if !matches!(store::finish(&db.pool, item_id, &reply, false).await, Ok(true)) {
        return;
    }
    let auto = if item.sender_level == "internal" { cfg.reply_internal_auto } else { cfg.reply_external_auto };
    if !verified_ok {
        // 驗證沒過就不自動寄：把「沒修好」原樣回給提問的人之前，先讓人看過。
        if auto {
            store::set_error(&db.pool, item_id, "驗證未通過，已保留為草稿，請確認後再寄出").await.ok();
        }
    } else if auto {
        if let Err(e) = send_reply(app, item_id).await {
            store::set_error(&db.pool, item_id, &format!("自動寄出失敗：{e}")).await.ok();
        }
    }
}

/// 收集用郵件帳號與它的密碼。
fn mail_creds<R: Runtime>(app: &AppHandle<R>, account_id: &str) -> Result<(crate::config::types::MailAccountConfig, String), String> {
    let account = app
        .try_state::<Arc<ConfigStore>>()
        .ok_or("設定尚未就緒")?
        .get()
        .mail_accounts
        .into_iter()
        .find(|a| a.id == account_id)
        .ok_or("找不到收集用的郵件帳號")?;
    let password = app
        .try_state::<Arc<SecretStore>>()
        .ok_or("密鑰庫尚未就緒")?
        .get(&crate::mail::poller::mail_secret_key(&account.id))
        .map_err(|e| e.to_string())?
        .ok_or("這個帳號沒有儲存密碼")?;
    Ok((account, password))
}

/// 用收集帳號寄一封一般信（網頁表單的驗證碼）。
/// 用某個專案設定的收信帳號寄一封一般信（網頁表單的驗證碼）。
pub async fn send_plain_mail<R: Runtime>(app: &AppHandle<R>, project_id: &str, to: &str, subject: &str, body: &str) -> Result<(), String> {
    let db = app.try_state::<IntakeDb>().ok_or("intake 未就緒")?;
    let cfg = config::load(&db.pool, project_id).await;
    let (account, password) = mail_creds(app, &cfg.account_id)?;
    let msg = smtp::build_plain(&account.email, to, subject, body)?;
    smtp::send(&account.smtp_host, account.smtp_port, &account.username, &password, msg).await
}

/// 寄出草稿（`draft → done`）。
pub async fn send_reply<R: Runtime>(app: &AppHandle<R>, item_id: &str) -> Result<(), String> {
    let db = app.try_state::<IntakeDb>().ok_or("intake 未就緒")?;
    let item = store::get_item(&db.pool, item_id).await.map_err(|e| e.to_string())?.ok_or("找不到項目")?;
    if item.status != store::DRAFT {
        return Err("只有草稿可以寄出".into());
    }
    let body = item.reply_draft.clone().unwrap_or_default();
    if body.trim().is_empty() {
        return Err("回覆內容是空的".into());
    }
    if item.source == "telegram" {
        let (chat, reply_to) = super::telegram_source::reply_target(&item.sender, &item.message_id).ok_or("無法判斷要回覆的 Telegram 聊天室")?;
        let token = app
            .try_state::<Arc<SecretStore>>()
            .ok_or("密鑰庫尚未就緒")?
            .get(crate::telegram::TELEGRAM_SECRET_ID)
            .map_err(|e| e.to_string())?
            .ok_or("尚未設定 Telegram 機器人")?;
        crate::telegram::send_to_chat(&token, chat, &filter::redact(&body), reply_to).await?;
        store::mark_sent(&db.pool, item_id).await.map_err(|e| e.to_string())?;
        changed(app);
        return Ok(());
    }
    // 信件與網頁表單都以電子郵件回覆，用該題所屬專案的收信帳號。
    let cfg = config::load(&db.pool, &item.project_id).await;
    let (account, password) = mail_creds(app, &cfg.account_id)?;
    // 只有真的來自信件的項目才有可以串信的 Message-ID（網頁表單與無 ID 的信沒有）。
    let orig_id = if item.source == "email" && !item.message_id.starts_with("nomid:") { item.message_id.as_str() } else { "" };
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
