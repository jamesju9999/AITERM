//! 執行層：把登記表、佇列與實際的終端機分頁、郵件帳號接起來。
use super::{board, classify, config, dispatcher, filter, rules, smtp, store, verify, IntakeDb, CHANGED_EVENT};
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
        "session-end" => {
            r.registry.mark_claude(sid, false);
            // Claude Code 在這題做到一半結束了：不會再有回覆，把題目退回而不是讓它卡在處理中。
            if let Some(item_id) = r.registry.unregister_active(sid) {
                let h = h.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(db) = h.try_state::<IntakeDb>() {
                        store::fail(&db.pool, &item_id, "Claude Code 已結束，這題沒有完成").await.ok();
                    }
                    changed(&h);
                });
            } else {
                changed(&h);
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

/// 派出去的提問超過這麼多秒還沒被 Claude Code 收到，就當作派送失敗。
const UNARMED_TIMEOUT_SECS: u64 = 90;

/// 這個分頁現在能不能把提問打進去。條件（缺一不可）：
/// 1. 知道有 Claude Code 在跑：`claude` 函式送出的啟動標記是最後一個；沒有標記（舊分頁、使用者自己的別名、
///    標記被擠出緩衝區）時，退而看 hook 事件（提問／回覆事件代表 Claude Code 活著）。
/// 2. shell 不是停在提示字元——停在提示字元代表 Claude Code 已經不在了，打進去就是在 shell 裡執行。
///    沒有 shell 整合、無從判斷時，以 1 為準。
pub fn can_type_prompt(registry: &dispatcher::Registry, pty: &PtyManager, tab: &str) -> Result<(), &'static str> {
    let raw = pty.get_recent_raw(tab, 256 * 1024).unwrap_or_default();
    match super::tabcd::claude_running(&raw) {
        Some(true) => {}
        Some(false) => return Err("Claude Code 已結束"),
        None if registry.claude_up(tab) => {}
        None => return Err("尚未偵測到 Claude Code"),
    }
    if super::tabcd::shell_idle(&raw) == Some(true) {
        return Err("分頁停在 shell 提示字元，Claude Code 已不在了");
    }
    Ok(())
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
    // 看門狗：派出去很久、提問卻一直沒被 Claude Code 收到，退回待派送並釋放分頁，不要永遠卡住。
    for (tab, id) in rt.registry.stale_unarmed(UNARMED_TIMEOUT_SECS) {
        rt.registry.release(&tab);
        store::fail(&db.pool, &id, "Claude Code 沒有收到提問，已退回待派送").await.ok();
        touched = true;
    }
    if route_and_settle(app, &db.pool).await {
        touched = true;
    }
    for tab in rt.registry.idle_tabs() {
        // 絕不能把提問打進沒有 Claude Code 的分頁：那是 shell，會把問題內容當指令執行。
        if let Err(why) = can_type_prompt(&rt.registry, &pty, &tab) {
            log::debug!("intake: 不派送給分頁 {tab}：{why}");
            continue;
        }
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
        // 初篩可能花了幾十秒，這段時間 Claude Code 可能已經結束：打字前再確認一次。
        if let Err(why) = can_type_prompt(&rt.registry, &pty, &tab) {
            rt.registry.release(&tab);
            store::fail(&db.pool, &item.id, &format!("派送前分頁已不能接收提問：{why}")).await.ok();
            continue;
        }
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

/// 初篩分類，逾時或失敗一律當「要改程式」。
async fn classify_item<R: Runtime>(app: &AppHandle<R>, item: &store::IntakeItem) -> classify::Kind {
    match app.try_state::<AiRouter>() {
        Some(router) => match router.resolve().await {
            Ok(provider) => tokio::time::timeout(Duration::from_secs(30), classify::classify(provider, &item.subject, &item.body))
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or(classify::Kind::CodeChange),
            Err(_) => classify::Kind::CodeChange,
        },
        None => classify::Kind::CodeChange,
    }
}

/// 看板／桌布：把新問題建成卡片，並收回已做完的卡片。回傳有沒有任何變動。
///
/// 只處理專案已開啟的；派送目標是終端機、或條件不足（看板要程式庫路徑）就一律不碰，照舊走終端機分頁。
async fn route_and_settle<R: Runtime>(app: &AppHandle<R>, pool: &sqlx::SqlitePool) -> bool {
    let Some(reg) = app.try_state::<crate::projects::ProjectRegistry>() else { return false };
    let mut touched = false;
    for handle in reg.all() {
        let cfg = config::load(pool, &handle.id).await;
        match cfg.route {
            config::Route::Terminal => {}
            config::Route::Canvas if board::pick_bug_assignee(&handle.pool).await.ok().flatten().is_some() => {
                touched |= route_canvas(app, pool, &handle).await;
            }
            // 桌布沒有可用的工程師，退回看板。
            config::Route::Canvas | config::Route::Board => touched |= route_board(app, pool, &handle).await,
        }
    }
    let Ok(on_board) = store::list_on_board(pool).await else { return touched };
    for item in on_board {
        let Some(handle) = reg.get(&item.project_id) else { continue };
        match board::settle(pool, &handle, &item).await {
            Ok(board::Settled::Waiting) => {}
            Ok(board::Settled::Returned) => touched = true,
            Ok(board::Settled::Drafted { success }) => {
                touched = true;
                let cfg = config::load(pool, &item.project_id).await;
                let auto = if item.sender_level == "internal" { cfg.reply_internal_auto } else { cfg.reply_external_auto };
                if auto && success {
                    if let Err(e) = send_reply(app, &item.id).await {
                        store::set_error(pool, &item.id, &format!("自動寄出失敗：{e}")).await.ok();
                    }
                } else if auto {
                    store::set_error(pool, &item.id, "看板卡片沒有成功，已保留為草稿，請確認後再寄出").await.ok();
                }
            }
            Err(e) => log::warn!("intake: 收看板卡片失敗：{e}"),
        }
    }
    touched
}

/// 看板路線：「要改程式」的新問題建成看板卡片；提問留給終端機分頁。
async fn route_board<R: Runtime>(app: &AppHandle<R>, pool: &sqlx::SqlitePool, handle: &crate::projects::ProjectHandle) -> bool {
    let mut touched = false;
    let Ok(Some(repo)) = crate::tasks::formations::repo_path(&handle.pool).await else { return false };
    let Ok(items) = store::list_unrouted(pool, &handle.id).await else { return false };
    for item in items {
        let kind = classify_item(app, &item).await;
        // 分類要先存：建卡失敗時項目留在原狀態，之後由分頁領取也不必再分類。
        store::set_kind(pool, &item.id, kind.as_str(), None).await.ok();
        if kind == classify::Kind::CodeChange {
            match board::route_item(pool, handle, &item, &repo).await {
                Ok(_) => touched = true,
                Err(e) => log::warn!("intake: 建看板卡失敗：{e}"),
            }
        } else {
            touched = true;
        }
    }
    touched
}

/// 桌布路線：要改程式→bug 卡（指派給最閒的工程師，走審核）；提問→問答卡（交給最閒的 Agent 讀程式庫回答）。
async fn route_canvas<R: Runtime>(app: &AppHandle<R>, pool: &sqlx::SqlitePool, handle: &crate::projects::ProjectHandle) -> bool {
    let mut touched = false;
    let Ok(items) = store::list_unrouted_canvas(pool, &handle.id).await else { return false };
    for item in items {
        let kind = match item.kind.as_deref() {
            Some("question") => classify::Kind::Question,
            Some(_) => classify::Kind::CodeChange,
            None => {
                let k = classify_item(app, &item).await;
                store::set_kind(pool, &item.id, k.as_str(), None).await.ok();
                touched = true;
                k
            }
        };
        match kind {
            classify::Kind::CodeChange => {
                let Ok(Some(agent)) = board::pick_bug_assignee(&handle.pool).await else { continue };
                match board::route_canvas_bug(pool, handle, &item, &agent).await {
                    Ok(_) => touched = true,
                    Err(e) => log::warn!("intake: 建桌布 bug 卡失敗：{e}"),
                }
            }
            // 手動模式的提問要等放行（queued）才建問答卡；放行前只存分類。
            classify::Kind::Question if item.status == store::QUEUED => {
                let all = crate::tasks::agents::list_agents(&handle.pool).await.unwrap_or_default();
                let Ok(Some(agent)) = crate::tasks::workflow::pick_ask_agent(&handle.pool, &all).await else { continue };
                match board::route_canvas_ask(pool, handle, &item, &agent).await {
                    Ok(_) => touched = true,
                    Err(e) => log::warn!("intake: 建桌布問答卡失敗：{e}"),
                }
            }
            classify::Kind::Question => {}
        }
    }
    if touched {
        // 不等排程器 30 秒一輪的掃描，也讓畫面重讀。
        if let Some(s) = app.try_state::<crate::tasks::scheduler::SchedulerHandle>() {
            s.poke();
        }
        let _ = app.emit("tasks-updated", ());
    }
    touched
}

/// 派工前：初篩分類；「要改程式」的記下 git HEAD 供事後驗證。任何失敗都退成「要改程式」。
async fn prepare<R: Runtime>(app: &AppHandle<R>, pool: &sqlx::SqlitePool, pty: &PtyManager, item: &store::IntakeItem, tab: &str) {
    // 看板路由階段可能已經分類過了：不再問一次 AI。
    let kind = match item.kind.as_deref() {
        Some("question") => classify::Kind::Question,
        Some(_) => classify::Kind::CodeChange,
        None => classify_item(app, item).await,
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
    // 回覆全部：原信的其他收件人與副本都要收到（只有真的來自信件的項目才有這些資料）。
    let (also_to, cc) = smtp::reply_all(&item.sender, item.to_addrs.as_deref().unwrap_or(""), item.cc_addrs.as_deref().unwrap_or(""), &[account.email.as_str(), cfg.alias.as_str()]);
    let msg = smtp::build_reply(&smtp::Outgoing {
        from: &account.email,
        to: &item.sender,
        orig_subject: &item.subject,
        orig_message_id: orig_id,
        also_to: &also_to,
        cc: &cc,
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

#[cfg(test)]
mod board_wiring_tests {
    use super::*;
    use crate::intake::rules::Level;
    use crate::intake::{store::{insert_item, NewItem}, IntakeDb};
    use crate::projects::ProjectRegistry;
    use crate::tasks::store as tasks;

    /// 整條接線：`tick` 把「要改程式」的問題建成卡片，卡片做完後下一次 `tick` 收回並存成回覆草稿。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tick_routes_a_code_change_to_the_board_and_settles_it_when_the_card_is_done() {
        let app = tauri::test::mock_app();
        let h = app.handle().clone();
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&pool).await.unwrap();
        h.manage(IntakeDb { pool: pool.clone() });
        h.manage(Arc::new(IntakeRuntime { registry: dispatcher::Registry::default(), tick_lock: tokio::sync::Mutex::new(()) }));
        h.manage(Arc::new(PtyManager::new()));
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let project = reg.create(parent.path(), "demo", "").await.unwrap();
        h.manage(reg);
        // 沒有程式庫路徑：不碰，留給終端機分頁。
        let first = insert_item(&pool, NewItem { project_id: &project.id, id: None, source: "email", sender: "a@corp.com", level: Level::Internal, subject: "修登入", body: "請修", message_id: "m0", status: store::QUEUED }).await.unwrap().unwrap();
        tick(&h).await;
        assert_eq!(store::get_item(&pool, &first.id).await.unwrap().unwrap().status, store::QUEUED);

        // 設好程式庫路徑後才建卡。
        crate::tasks::repo_check::save_project_repo(&project.pool, "/work/repo").await.unwrap();
        tick(&h).await;
        let got = store::get_item(&pool, &first.id).await.unwrap().unwrap();
        assert_eq!(got.status, store::ON_BOARD);
        let card_id = got.board_task_id.clone().unwrap();
        let card = tasks::get_task(&project.pool, &card_id).await.unwrap().unwrap();
        assert_eq!((card.status.as_str(), card.project_dir.as_str()), ("queued", "/work/repo"));

        // 卡片還沒做完：再 tick 不會動它，也不會建第二張。
        tick(&h).await;
        assert_eq!(tasks::list_tasks(&project.pool).await.unwrap().len(), 1);
        assert_eq!(store::get_item(&pool, &first.id).await.unwrap().unwrap().status, store::ON_BOARD);

        tasks::dispatch_for_test(&project.pool, &card_id, "tab").await;
        tasks::set_summary(&project.pool, &card_id, "已修好").await.unwrap();
        tasks::finish_task(&project.pool, &card_id, "success", None, None).await.unwrap();
        tick(&h).await;
        let done = store::get_item(&pool, &first.id).await.unwrap().unwrap();
        // 內部自動寄出但測試環境沒有郵件帳號：寄不出去，草稿與錯誤原因都要留著。
        assert_eq!(done.status, store::DRAFT);
        assert!(done.reply_draft.unwrap().contains("已修好"));
        assert!(done.error.is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn turning_the_switch_off_keeps_everything_on_the_terminal_path() {
        let app = tauri::test::mock_app();
        let h = app.handle().clone();
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&pool).await.unwrap();
        h.manage(IntakeDb { pool: pool.clone() });
        h.manage(Arc::new(IntakeRuntime { registry: dispatcher::Registry::default(), tick_lock: tokio::sync::Mutex::new(()) }));
        h.manage(Arc::new(PtyManager::new()));
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let project = reg.create(parent.path(), "demo", "").await.unwrap();
        crate::tasks::repo_check::save_project_repo(&project.pool, "/work/repo").await.unwrap();
        h.manage(reg);
        config::save(&pool, &project.id, &config::Config { route: config::Route::Terminal, ..config::Config::default() }).await.unwrap();
        let item = insert_item(&pool, NewItem { project_id: &project.id, id: None, source: "email", sender: "a@corp.com", level: Level::Internal, subject: "s", body: "b", message_id: "m", status: store::QUEUED }).await.unwrap().unwrap();
        tick(&h).await;
        let got = store::get_item(&pool, &item.id).await.unwrap().unwrap();
        assert_eq!((got.status.as_str(), got.kind), (store::QUEUED, None));
        assert!(tasks::list_tasks(&project.pool).await.unwrap().is_empty());
    }

    async fn canvas_app() -> (tauri::App<tauri::test::MockRuntime>, sqlx::SqlitePool, crate::projects::ProjectHandle, tempfile::TempDir) {
        let app = tauri::test::mock_app();
        let h = app.handle().clone();
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&pool).await.unwrap();
        h.manage(IntakeDb { pool: pool.clone() });
        h.manage(Arc::new(IntakeRuntime { registry: dispatcher::Registry::default(), tick_lock: tokio::sync::Mutex::new(()) }));
        h.manage(Arc::new(PtyManager::new()));
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let project = reg.create(parent.path(), "demo", "").await.unwrap();
        h.manage(reg);
        config::save(&pool, &project.id, &config::Config { route: config::Route::Canvas, ..config::Config::default() }).await.unwrap();
        (app, pool, project, parent)
    }

    async fn new_agent(p: &crate::projects::ProjectHandle, kind: &str, name: &str) -> String {
        crate::tasks::agents::create_agent(&p.pool, &crate::tasks::agents::NewAgent { kind, name, role: "r", prompt_prefix: "", project_dir: "/repo", canvas_x: 0.0, canvas_y: 0.0 }).await.unwrap()
    }

    /// 桌布路線整條接線：bug 指派給工程師、提問建問答卡，做完後下一次 tick 收回成草稿。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tick_routes_to_the_canvas_and_settles_bug_and_question() {
        let (app, pool, project, _d) = canvas_app().await;
        let h = app.handle().clone();
        let coder = new_agent(&project, "coder", "甲").await;
        let bug = insert_item(&pool, NewItem { project_id: &project.id, id: None, source: "email", sender: "a@corp.com", level: Level::Internal, subject: "登入壞了", body: "請修", message_id: "m1", status: store::QUEUED }).await.unwrap().unwrap();
        let ask = insert_item(&pool, NewItem { project_id: &project.id, id: None, source: "email", sender: "a@corp.com", level: Level::Internal, subject: "怎麼登入", body: "請問", message_id: "m2", status: store::QUEUED }).await.unwrap().unwrap();
        store::set_kind(&pool, &ask.id, "question", None).await.unwrap();
        tick(&h).await;

        let b = store::get_item(&pool, &bug.id).await.unwrap().unwrap();
        let q = store::get_item(&pool, &ask.id).await.unwrap().unwrap();
        assert_eq!((b.status.as_str(), b.route_kind.as_deref()), (store::ON_BOARD, Some("canvas_bug")));
        assert_eq!((q.status.as_str(), q.route_kind.as_deref()), (store::ON_BOARD, Some("canvas_ask")));
        let bug_card = tasks::get_task(&project.pool, b.board_task_id.as_deref().unwrap()).await.unwrap().unwrap();
        assert_eq!((bug_card.status.as_str(), bug_card.agent_id.as_deref()), ("queued", Some(coder.as_str())));
        let ask_card = tasks::get_task(&project.pool, q.board_task_id.as_deref().unwrap()).await.unwrap().unwrap();
        assert_eq!(ask_card.card_kind, "ask");

        // 再 tick 不會重複建卡。
        tick(&h).await;
        assert_eq!(tasks::list_tasks(&project.pool).await.unwrap().len(), 2);

        tasks::dispatch_for_test(&project.pool, &bug_card.id, "t1").await;
        tasks::finish_task(&project.pool, &bug_card.id, "success", None, None).await.unwrap();
        tasks::dispatch_for_test(&project.pool, &ask_card.id, "t2").await;
        tasks::set_summary(&project.pool, &ask_card.id, "看 auth.rs").await.unwrap();
        tasks::finish_task(&project.pool, &ask_card.id, "success", None, None).await.unwrap();
        tick(&h).await;
        let (b, q) = (store::get_item(&pool, &bug.id).await.unwrap().unwrap(), store::get_item(&pool, &ask.id).await.unwrap().unwrap());
        assert_eq!((b.status.as_str(), q.status.as_str()), (store::DRAFT, store::DRAFT));
        assert!(b.reply_draft.unwrap().contains("Agent 協作結果：成功"));
        assert!(q.reply_draft.unwrap().starts_with("看 auth.rs"));
    }

    /// 選了桌布但沒有工程師：退回看板（要有程式庫路徑）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canvas_without_a_coder_falls_back_to_the_board() {
        let (app, pool, project, _d) = canvas_app().await;
        let h = app.handle().clone();
        new_agent(&project, "reviewer", "審核者").await;
        crate::tasks::repo_check::save_project_repo(&project.pool, "/work/repo").await.unwrap();
        let item = insert_item(&pool, NewItem { project_id: &project.id, id: None, source: "email", sender: "a@corp.com", level: Level::Internal, subject: "修", body: "b", message_id: "m1", status: store::QUEUED }).await.unwrap().unwrap();
        tick(&h).await;
        let got = store::get_item(&pool, &item.id).await.unwrap().unwrap();
        assert_eq!((got.status.as_str(), got.route_kind.as_deref()), (store::ON_BOARD, Some("board")));
    }

    /// 手動模式：bug 卡留在規劃欄不指派；提問要等放行才建問答卡。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canvas_manual_mode_keeps_the_bug_unassigned_and_holds_the_question() {
        let (app, pool, project, _d) = canvas_app().await;
        let h = app.handle().clone();
        new_agent(&project, "coder", "甲").await;
        let bug = insert_item(&pool, NewItem { project_id: &project.id, id: None, source: "email", sender: "a@corp.com", level: Level::External, subject: "修", body: "b", message_id: "m1", status: store::PENDING }).await.unwrap().unwrap();
        let ask = insert_item(&pool, NewItem { project_id: &project.id, id: None, source: "email", sender: "a@corp.com", level: Level::External, subject: "問", body: "b", message_id: "m2", status: store::PENDING }).await.unwrap().unwrap();
        store::set_kind(&pool, &ask.id, "question", None).await.unwrap();
        tick(&h).await;
        let b = store::get_item(&pool, &bug.id).await.unwrap().unwrap();
        let card = tasks::get_task(&project.pool, b.board_task_id.as_deref().unwrap()).await.unwrap().unwrap();
        assert_eq!((card.status.as_str(), card.agent_id), ("planning", None));
        assert_eq!(store::get_item(&pool, &ask.id).await.unwrap().unwrap().status, store::PENDING, "提問放行前不建卡");
        store::approve(&pool, &ask.id).await.unwrap();
        tick(&h).await;
        assert_eq!(store::get_item(&pool, &ask.id).await.unwrap().unwrap().route_kind.as_deref(), Some("canvas_ask"));
    }
}
