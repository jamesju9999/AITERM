//! 連到 server 時的領取處理：把 server 的題目派進本機「收集中」的分頁，做完交回 server。
//! 分頁登錄、提問標記、完成偵測沿用本機派送（`dispatcher::Registry` 與 hook）；
//! 這裡只負責「向 server 領題目」「交結果／放回」「續約」。
use super::remote::{self, RemoteState};
use super::runtime::{self, IntakeRuntime};
use super::store::IntakeItem;
use super::{dispatcher, CHANGED_EVENT};
use crate::pty::PtyManager;
use aiterm_intake_client::api::Api;
use aiterm_intake_core::config::Handling;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// 處理中每隔多久向 server 續約（租約 15 分鐘，留很大的餘裕）。
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(120);
/// 派出去很久、提問卻一直沒被 Claude Code 收到，就放回去（與本機派送相同的看門狗）。
const UNARMED_TIMEOUT_SECS: u64 = 90;

fn attach_root() -> std::path::PathBuf {
    dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("AITERM").join("intake-remote")
}

fn changed<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit(CHANGED_EVENT, ());
}

/// 題目的提示詞：與本機派送同一份（含診斷模式的提示）；附件先下載到本機。
/// server 設定裡的參考資料夾是 server 上的路徑，這台電腦不一定有，第一版不帶。
async fn build_prompt(api: &Api, item: &IntakeItem) -> String {
    let images = remote::download_attachments(api, &item.id, &attach_root()).await;
    let diagnose = match remote::get_config(api, &item.project_id).await {
        Ok(v) => v.config.handling(&item.source) == Handling::Diagnose,
        Err(_) => true, // 讀不到設定時用較保守的「只診斷」
    };
    dispatcher::build_prompt(item, &images, diagnose)
}

/// 把領到的題目打進分頁。失敗就把題目交還給 server（回報原因），並釋放分頁。
pub async fn dispatch<R: Runtime>(app: &AppHandle<R>, api: &Api, rs: &RemoteState, rt: &IntakeRuntime, pty: &PtyManager, tab: &str, item: &IntakeItem) {
    rt.registry.assign(tab, &item.id);
    rs.track_claim(&item.id, tab);
    let prompt = build_prompt(api, item).await;
    // 下載附件可能花一點時間：打字前再確認一次分頁還能收。
    let sent = match runtime::can_type_prompt(&rt.registry, pty, tab) {
        Ok(()) => crate::tasks::dispatch::write_then_submit(pty, tab, &prompt).await,
        Err(why) => Err(format!("分頁已不能接收提問：{why}")),
    };
    if let Err(e) = sent {
        rt.registry.release(tab);
        rs.untrack_claim(&item.id);
        remote::submit_error(api, &item.id, &format!("無法送進終端機：{e}")).await.ok();
    }
    changed(app);
}

/// 本機定時掃描（連線模式）：續約、收掉出事的分頁，再讓閒置分頁自動向 server 領下一題。
pub async fn tick<R: Runtime>(app: &AppHandle<R>, rt: &IntakeRuntime, pty: &PtyManager) {
    let Some((api, rs)) = remote::active(app) else { return };
    let mut touched = false;

    // 1. 續約。被收回（逾期）或 server 不認這個成員，就放掉這題，免得白做。
    for id in rs.due_for_heartbeat(HEARTBEAT_EVERY) {
        if !matches!(remote::heartbeat(&api, &id).await, Ok(true)) {
            lose_claim(rt, &rs, &id);
            touched = true;
        }
    }
    // 2. 分頁被關掉：進行中的題目回報失敗。
    for tab in rt.registry.tabs() {
        if pty.size(&tab).is_none() {
            if let Some(id) = rt.registry.unregister(&tab) {
                if rs.untrack_claim(&id).is_some() {
                    remote::submit_error(&api, &id, "接收的分頁已關閉").await.ok();
                }
            }
            touched = true;
        }
    }
    // 3. 看門狗：提問一直沒被 Claude Code 收到，放回 server。
    for (tab, id) in rt.registry.stale_unarmed(UNARMED_TIMEOUT_SECS) {
        rt.registry.release(&tab);
        if rs.untrack_claim(&id).is_some() {
            remote::release(&api, &id).await.ok();
        }
        touched = true;
    }
    // 4. 自動領取：閒置、Claude Code 在跑的分頁，向 server 領它負責的專案的下一題。
    for tab in rt.registry.idle_tabs() {
        if runtime::can_type_prompt(&rt.registry, pty, &tab).is_err() {
            continue;
        }
        let Some(project) = rt.registry.project_of(&tab) else { continue };
        match remote::claim_next(&api, &project).await {
            Ok(Some(item)) => {
                dispatch(app, &api, &rs, rt, pty, &tab, &item).await;
                touched = true;
            }
            Ok(None) => {}
            Err(e) => {
                // 被移除、憑證改變這類要使用者處理的錯誤記起來，一般離線就等下一輪。
                remote::fail(&rs, e);
                break;
            }
        }
    }
    if touched {
        changed(app);
    }
}

/// 這題不再是我的了（租約被收回）：放掉分頁，之後 Claude Code 就算做完也不會交結果。
fn lose_claim(rt: &IntakeRuntime, rs: &RemoteState, item_id: &str) {
    if let Some(tab) = rs.untrack_claim(item_id) {
        rt.registry.release(&tab);
    }
}

/// 手動「我來處理」：找這個專案「收集中」且閒置的分頁，領這一題並派進去。
pub async fn claim_manual<R: Runtime>(app: &AppHandle<R>, item_id: &str) -> Result<(), String> {
    let (api, rs) = remote::active(app).ok_or("尚未連到 server")?;
    let rt = app.try_state::<Arc<IntakeRuntime>>().ok_or("intake 未就緒")?;
    let pty = app.try_state::<Arc<PtyManager>>().ok_or("終端機未就緒")?;
    let _guard = rt.tick_lock.lock().await;
    let info = remote::get_item(&api, item_id).await.map_err(|e| remote::fail(&rs, e))?;
    let tab = rt
        .registry
        .idle_tabs()
        .into_iter()
        .find(|t| rt.registry.project_of(t).as_deref() == Some(info.project_id.as_str()) && runtime::can_type_prompt(&rt.registry, &pty, t).is_ok())
        .ok_or("沒有可用的分頁：請先在終端機分頁啟動 Claude Code，並按「啟動問題收集」選這個專案（分頁要閒置）")?;
    let item = remote::claim_item(&api, item_id).await.map_err(|e| remote::fail(&rs, e))?;
    dispatch(app, &api, &rs, &rt, &pty, &tab, &item).await;
    Ok(())
}

/// Claude Code 做完一題：交回 server。回傳這題是不是從 server 領來的（不是就交給本機流程）。
pub async fn complete<R: Runtime>(app: &AppHandle<R>, item_id: &str, answer: Option<String>) -> bool {
    let Some((api, rs)) = remote::active(app) else { return false };
    if rs.untrack_claim(item_id).is_none() {
        return false;
    }
    let sent = match answer {
        Some(a) if !a.trim().is_empty() => remote::submit_reply(&api, item_id, &a).await,
        _ => remote::submit_error(&api, item_id, "無法取得 Agent 的回答").await,
    };
    if let Err(e) = sent {
        // 常見原因是租約逾期、題目已被收回；結果不會被採用。
        log::warn!("intake: 交回 server 失敗：{e}");
        remote::fail(&rs, e);
    }
    changed(app);
    true
}

/// 進行中的題目不會再有結果（Claude Code 結束、停止收集）：`release` 為真放回待派送，否則回報失敗。
/// 回傳這題是不是從 server 領來的。
pub async fn abandon<R: Runtime>(app: &AppHandle<R>, item_id: &str, reason: &str, release: bool) -> bool {
    let Some((api, rs)) = remote::active(app) else { return false };
    if rs.untrack_claim(item_id).is_none() {
        return false;
    }
    let r = if release { remote::release(&api, item_id).await.map(|_| ()) } else { remote::submit_error(&api, item_id, reason).await.map(|_| ()) };
    if let Err(e) = r {
        remote::fail(&rs, e);
    }
    changed(app);
    true
}
