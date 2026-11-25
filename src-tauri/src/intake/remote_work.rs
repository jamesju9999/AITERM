//! 連到 server 時的領取處理：把 server 的題目派進本機「收集中」的分頁，做完交回 server。
//! 分頁登錄、提問標記、完成偵測沿用本機派送（`dispatcher::Registry` 與 hook）；
//! 這裡只負責「向 server 領題目」「交結果／放回」「續約」。
use super::remote::{self, RemoteState};
use super::runtime::{self, IntakeRuntime};
use super::store::IntakeItem;
use super::{dispatcher, verify, IntakeDb, CHANGED_EVENT};
use crate::pty::PtyManager;
use aiterm_intake_client::api::Api;
use aiterm_intake_core::config::{Config, Handling, RefFolders};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// 處理中每隔多久向 server 續約（租約 15 分鐘，留很大的餘裕）。
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(120);
/// 派出去很久、提問卻一直沒被 Claude Code 收到，就放回去（與本機派送相同的看門狗）。
const UNARMED_TIMEOUT_SECS: u64 = 90;

fn changed<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit(CHANGED_EVENT, ());
}

/// 這台電腦上、某個 server 專案的參考資料夾（只存在本機；server 設定的路徑是 server 上的，帶不過來）。
const LOCAL_REFS_KEY: &str = "remote_refs:";

pub async fn load_local_refs(pool: &sqlx::SqlitePool, project_id: &str) -> RefFolders {
    aiterm_intake_core::store::get_setting(pool, &format!("{LOCAL_REFS_KEY}{project_id}")).await.ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

/// 存本機的參考資料夾。每一項要是絕對路徑，而且在這台電腦上找得到（`~` 開頭的略過檢查）；回傳整理過的內容。
pub async fn save_local_refs(pool: &sqlx::SqlitePool, project_id: &str, refs: &RefFolders) -> Result<RefFolders, String> {
    let refs = refs.cleaned();
    for p in refs.shared.iter().chain(refs.internal.iter()) {
        if !dispatcher::is_absolute_like(p) {
            return Err(format!("請填完整路徑（不是相對路徑）：{p}"));
        }
        if !p.starts_with('~') && !std::path::Path::new(p).exists() {
            return Err(format!("這台電腦上找不到這個資料夾：{p}"));
        }
    }
    let json = serde_json::to_string(&refs).map_err(|e| e.to_string())?;
    aiterm_intake_core::store::set_setting(pool, &format!("{LOCAL_REFS_KEY}{project_id}"), &json).await.map_err(|e| e.to_string())?;
    Ok(refs)
}

/// 派工時實際帶給 Claude Code 的參考資料夾：本機設定的，外部客戶的題目只拿共用那組；已經不存在的略過。
fn usable_refs(refs: &RefFolders, internal: bool) -> Vec<String> {
    let cfg = Config { reference_folders: refs.clone(), ..Config::default() };
    cfg.refs_for(internal).into_iter().filter(|p| p.starts_with('~') || std::path::Path::new(p).exists()).collect()
}

/// 題目的提示詞：與本機派送同一份（含診斷模式的提示與參考資料夾）；附件先下載到本機。
/// 參考資料夾用的是**這台電腦**上設定的（見 `save_local_refs`），不是 server 設定裡的路徑。
/// 同時回傳專案設定（讀不到就是 `None`，此時用較保守的「只診斷」，也不驗證）。
async fn build_prompt(api: &Api, rs: &RemoteState, pool: Option<&sqlx::SqlitePool>, item: &IntakeItem) -> (String, Option<Config>) {
    let images = remote::download_attachments(api, &item.id, &rs.attach_root()).await;
    let cfg = remote::get_config(api, &item.project_id).await.ok().map(|v| v.config);
    let diagnose = cfg.as_ref().map(|c| c.handling(&item.source) == Handling::Diagnose).unwrap_or(true);
    let refs = match pool {
        Some(p) => usable_refs(&load_local_refs(p, &item.project_id).await, item.sender_level == "internal"),
        None => Vec::new(),
    };
    (format!("{}{}", dispatcher::build_prompt(item, &images, diagnose), dispatcher::refs_note(&refs)), cfg)
}

/// 這題做完後要不要驗證：只有「要改程式」的才驗證——server 標了是提問、或這個管道設成只診斷，就不驗證；
/// 沒標種類時保守當成要改程式（與本機派送相同）。
fn wants_verify(item: &IntakeItem, cfg: Option<&Config>) -> bool {
    item.kind.as_deref() != Some("question") && cfg.is_some_and(|c| c.handling(&item.source) != Handling::Diagnose)
}

/// 把領到的題目打進分頁。失敗就把題目交還給 server（回報原因），並釋放分頁。
pub async fn dispatch<R: Runtime>(app: &AppHandle<R>, api: &Api, rs: &RemoteState, rt: &IntakeRuntime, pty: &PtyManager, tab: &str, item: &IntakeItem) {
    rt.registry.assign(tab, &item.id);
    rs.track_claim(&item.id, tab);
    let pool = app.try_state::<IntakeDb>().map(|d| d.pool.clone());
    let (prompt, cfg) = build_prompt(api, rs, pool.as_ref(), item).await;
    if wants_verify(item, cfg.as_ref()) {
        // 派工前記下 git HEAD，做完才知道改了哪些檔案。
        let snapshot = match pty.get_cwd(tab) {
            Some(cwd) => verify::snapshot(&cwd).await,
            None => None,
        };
        let c = cfg.unwrap_or_default();
        rs.set_verify_plan(&item.id, remote::VerifyPlan { command: c.verify_command.clone(), timeout: Duration::from_secs(c.verify_timeout_secs.max(1)), snapshot });
    }
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
    tick_with(app, rt, pty, HEARTBEAT_EVERY).await;
}

/// `tick` 的本體；續約間隔可以指定（測試不必真的等兩分鐘）。
pub async fn tick_with<R: Runtime>(app: &AppHandle<R>, rt: &IntakeRuntime, pty: &PtyManager, heartbeat_every: Duration) {
    let Some((api, rs)) = remote::active(app) else { return };
    let mut touched = false;

    // 1. 續約。被收回（逾期）或 server 不認這個成員，就放掉這題，免得白做。
    for id in rs.due_for_heartbeat(heartbeat_every) {
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
    let Some(claim) = rs.take_claim(item_id) else {
        return false;
    };
    let sent = match answer {
        Some(a) if !a.trim().is_empty() => {
            let mut reply = a;
            let mut verification = None;
            if let Some(plan) = &claim.verify {
                // 在領取者自己的工作資料夾驗證；報告去敏後附在回覆最後，並把是否通過一起交回。
                let cwd = app.try_state::<Arc<PtyManager>>().and_then(|p| p.get_cwd(&claim.tab)).unwrap_or_default();
                let report = verify::verify(&cwd, plan.snapshot.as_deref(), &plan.command, plan.timeout).await;
                let text = aiterm_intake_core::filter::redact(&report.text());
                reply = format!("{reply}\n\n{text}");
                verification = Some((text, report.passed()));
            }
            remote::submit_reply(&api, item_id, &reply, verification.as_ref().map(|(t, ok)| (t.as_str(), *ok))).await
        }
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::intake::remote::Saved;
    use crate::intake::store::{self, NewItem};
    use aiterm_intake_core::api::Role;
    use aiterm_intake_core::rules::Level;
    use aiterm_intake_server::testkit::TestServer;
    use portable_pty::PtySize;
    use std::sync::Arc;

    struct Rig {
        app: tauri::App<tauri::test::MockRuntime>,
        rs: Arc<RemoteState>,
        rt: Arc<IntakeRuntime>,
        pty: Arc<PtyManager>,
        srv: TestServer,
        pid: String,
        tab: String,
        out: Arc<parking_lot::Mutex<Vec<u8>>>,
        _tmp: tempfile::TempDir,
    }

    impl Rig {
        fn saw(&self, needle: &str) -> bool {
            String::from_utf8_lossy(&self.out.lock()).contains(needle)
        }
        async fn wait_for(&self, needle: &str) -> bool {
            for _ in 0..100 {
                if self.saw(needle) {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            false
        }
        async fn add_item(&self, subject: &str, status: &str) -> IntakeItem {
            store::insert_item(&self.srv.state().pools.intake, NewItem { project_id: &self.pid, id: None, source: "line", sender: "line:u0123456789abcdef0123456789abcdef", level: Level::Internal, subject, body: "請幫我看電腦", message_id: subject, status })
                .await
                .unwrap()
                .unwrap()
        }
        async fn add_item_as(&self, subject: &str, source: &str, level: Level) -> IntakeItem {
            store::insert_item(&self.srv.state().pools.intake, NewItem { project_id: &self.pid, id: None, source, sender: "line:u0123456789abcdef0123456789abcdef", level, subject, body: "請幫我看電腦", message_id: subject, status: store::QUEUED })
                .await
                .unwrap()
                .unwrap()
        }
        async fn server_item(&self, id: &str) -> IntakeItem {
            store::get_item(&self.srv.state().pools.intake, id).await.unwrap().unwrap()
        }
        async fn tick(&self) {
            tick_with(self.app.handle(), &self.rt, &self.pty, Duration::ZERO).await;
        }
    }

    /// 一台已連線的 AITERM（假視窗）、一個已登錄為「收集中」的真 PTY 分頁（`cat` 扮演 Claude Code：
    /// 先送出「Claude Code 在跑」與「有指令在跑」的標記，再卡在 cat 等輸入，並把收到的提問原樣回顯）。
    async fn rig() -> Rig {
        rig_in(None).await
    }

    /// 同 `rig`，但假的 Claude Code 分頁一開始就在 `cwd`（驗證要看領取者的工作資料夾）。
    async fn rig_in(cwd: Option<&std::path::Path>) -> Rig {
        let srv = TestServer::start().await;
        let pid = srv.project("範例科技").await;
        let app = tauri::test::mock_app();
        let h = app.handle().clone();
        let local = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&local).await.unwrap();
        h.manage(crate::intake::IntakeDb { pool: local });
        h.manage(Arc::new(RemoteState::default()));
        h.manage(Arc::new(crate::intake::line_server::LineServer::default()));
        h.manage(Arc::new(crate::intake::web::WebServer::default()));
        let rt = Arc::new(IntakeRuntime { registry: dispatcher::Registry::default(), tick_lock: tokio::sync::Mutex::new(()) });
        h.manage(rt.clone());
        let pty = Arc::new(PtyManager::new());
        h.manage(pty.clone());
        let tmp = tempfile::tempdir().unwrap();
        let rs = h.state::<Arc<RemoteState>>().inner().clone();
        rs.set_attach_root(tmp.path().to_path_buf());

        let code = srv.invite("小華", Role::Member).await;
        let paired = aiterm_intake_client::pair::pair(&srv.addr(), &code).await.unwrap();
        let api = Api::new(&srv.addr(), &paired.token, Some(paired.response.fingerprint.clone())).unwrap();
        remote::install(&h, Saved { addr: srv.addr(), fingerprint: Some(paired.response.fingerprint), name: "小華".into(), role: Role::Member }, api).await;

        let out = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = out.clone();
        let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
        let tab = match cwd {
            // 分頁一開始就在指定資料夾：PTY 追蹤的工作資料夾是從這裡開始算的（驗證要看它）。
            Some(dir) => pty.create_with_callback_and_id(size, format!("tab-{}", uuid::Uuid::new_v4()), Some(dir.to_path_buf()), vec![], vec![], move |c| sink.lock().extend_from_slice(&c)).unwrap(),
            None => pty.create_with_callback(size, move |c| sink.lock().extend_from_slice(&c)).unwrap(),
        };
        tokio::time::sleep(Duration::from_millis(1200)).await; // 等 shell 起來
        pty.write(&tab, format!("printf '\\033]7771;claude=1\\007\\033]133;C\\007'; cat\r").as_bytes()).unwrap();
        let rig = Rig { app, rs, rt, pty, srv, pid, tab, out, _tmp: tmp };
        // 要等到真正的標記（ESC 開頭）出現，而不是命令列自己的回顯。
        for _ in 0..100 {
            if rig.pty.get_recent_raw(&rig.tab, 64 * 1024).is_some_and(|r| r.windows(14).any(|w| w == b"\x1b]7771;claude=")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        rig.rt.registry.register(&rig.tab, &rig.pid);
        rig
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_idle_collecting_tab_claims_the_next_item_and_the_answer_goes_back_as_a_draft() {
        let r = rig().await;
        let item = r.add_item("電腦壞了", store::QUEUED).await;
        r.tick().await;
        let claimed = r.server_item(&item.id).await;
        assert_eq!((claimed.status.as_str(), claimed.claimed_by.as_deref()), (store::RUNNING, Some("小華")));
        assert!(r.wait_for(&dispatcher::marker_for(&item.id)).await, "提問（含標記）要真的送進分頁");
        assert!(r.saw("請幫我看電腦") && r.saw("只診斷"), "LINE 預設只診斷，提示詞要帶診斷模式的說明");
        assert_eq!(r.rt.registry.running().len(), 1);
        assert!(r.rs.is_claimed(&item.id));

        // Claude Code 做完：沿用本機的完成流程，結果交回 server。
        runtime::complete(r.app.handle(), &r.tab, &item.id, Some("請重開機再試試".into())).await;
        let done = r.server_item(&item.id).await;
        assert_eq!((done.status.as_str(), done.reply_draft.as_deref()), (store::DRAFT, Some("請重開機再試試")), "變成草稿等人看過再寄出");
        assert!(r.rt.registry.running().is_empty(), "分頁釋放，可以領下一題");
        assert!(!r.rs.is_claimed(&item.id));
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_reference_folders_are_validated_saved_per_project_and_only_this_computers_paths_count() {
        let r = rig().await;
        let pool = r.app.handle().state::<IntakeDb>().pool.clone();
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (pa, pb) = (a.path().to_string_lossy().to_string(), b.path().to_string_lossy().to_string());
        assert!(save_local_refs(&pool, &r.pid, &RefFolders { shared: vec!["docs/faq".into()], internal: vec![] }).await.unwrap_err().contains("完整路徑"));
        assert!(save_local_refs(&pool, &r.pid, &RefFolders { shared: vec!["/不存在/的/資料夾".into()], internal: vec![] }).await.unwrap_err().contains("找不到"));
        let saved = save_local_refs(&pool, &r.pid, &RefFolders { shared: vec![format!(" {pa} "), pa.clone(), String::new()], internal: vec![pb.clone()] }).await.unwrap();
        assert_eq!((saved.shared.clone(), saved.internal.clone()), (vec![pa.clone()], vec![pb.clone()]), "去空白、去重、略過空白項");
        assert_eq!(load_local_refs(&pool, &r.pid).await, saved);
        assert_eq!(load_local_refs(&pool, "別的專案").await, RefFolders::default(), "每個專案各自一份");
        assert_eq!(usable_refs(&saved, true), vec![pa.clone(), pb.clone()]);
        assert_eq!(usable_refs(&saved, false), vec![pa.clone()], "外部客戶的題目拿不到僅內部的");
        drop(b);
        assert_eq!(usable_refs(&saved, true), vec![pa], "資料夾後來被刪掉就略過，不要叫 Claude Code 去讀不存在的路徑");
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_claimers_own_reference_folders_go_into_the_prompt_and_servers_paths_do_not() {
        let r = rig().await;
        let pool = r.app.handle().state::<IntakeDb>().pool.clone();
        let base = tempfile::tempdir().unwrap();
        let (shared, internal) = (base.path().join("shared-notes-xyz"), base.path().join("internal-notes-xyz"));
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::create_dir_all(&internal).unwrap();
        save_local_refs(&pool, &r.pid, &RefFolders { shared: vec![shared.to_string_lossy().into()], internal: vec![internal.to_string_lossy().into()] }).await.unwrap();
        // server 設定裡的路徑是 server 上的，不能被帶出去。
        let mut cfg = aiterm_intake_core::config::load(&r.srv.state().pools.intake, &r.pid).await;
        cfg.reference_folders = RefFolders { shared: vec!["/srv/only-on-the-server".into()], internal: vec![] };
        aiterm_intake_core::config::save(&r.srv.state().pools.intake, &r.pid, &cfg).await.unwrap();

        let item = r.add_item_as("內部的題目", "line", Level::Internal).await;
        r.tick().await;
        assert!(r.wait_for(&dispatcher::marker_for(&item.id)).await);
        assert!(r.saw("shared-notes-xyz") && r.saw("internal-notes-xyz"), "內部題目帶本機設定的兩組");
        assert!(!r.saw("only-on-the-server"), "server 上的路徑不帶");
        runtime::complete(r.app.handle(), &r.tab, &item.id, Some("好".into())).await;

        let before = r.out.lock().len();
        let ext = r.add_item_as("外部客戶的題目", "line", Level::External).await;
        r.tick().await;
        assert!(r.wait_for(&dispatcher::marker_for(&ext.id)).await);
        let after = String::from_utf8_lossy(&r.out.lock()[before..]).to_string();
        assert!(after.contains("shared-notes-xyz") && !after.contains("internal-notes-xyz"), "外部客戶的題目只拿共用那組：{after}");
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_code_change_is_verified_in_the_claimers_folder_and_the_report_goes_back_with_the_draft() {
        // 領取者的終端機在一個 git 專案裡；Claude Code 做完時改了一個檔案。
        let repo = tempfile::tempdir().unwrap();
        let sh = |cmd: &str| std::process::Command::new("sh").arg("-c").arg(cmd).current_dir(repo.path()).output().unwrap();
        sh("git init -q && git config user.email a@b.c && git config user.name t && echo v1 > app.txt && git add -A && git commit -qm init");
        let r = rig_in(Some(repo.path())).await;
        // email 預設是「可改檔案」；驗證指令用一個一定會過的。
        let mut cfg = aiterm_intake_core::config::load(&r.srv.state().pools.intake, &r.pid).await;
        cfg.verify_command = "true".into();
        aiterm_intake_core::config::save(&r.srv.state().pools.intake, &r.pid, &cfg).await.unwrap();

        let item = r.add_item_as("程式有 bug", "email", Level::Internal).await;
        r.tick().await;
        assert!(r.wait_for(&dispatcher::marker_for(&item.id)).await);
        sh("echo v2 > app.txt");
        runtime::complete(r.app.handle(), &r.tab, &item.id, Some("已修好".into())).await;
        let done = r.server_item(&item.id).await;
        assert_eq!(done.status, store::DRAFT);
        assert_eq!(done.verified, Some(true), "有改檔案、驗證指令通過：{:?}", done.verification);
        let report = done.verification.clone().unwrap();
        assert!(report.contains("app.txt") && report.contains("測試通過"), "{report}");
        assert!(done.reply_draft.unwrap().contains("已修好") && report.starts_with("【驗證】"), "草稿帶回覆與驗證報告");
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_or_missing_change_is_marked_unverified_but_still_handed_back_and_a_diagnose_only_item_is_not_verified() {
        let repo = tempfile::tempdir().unwrap();
        let sh = |cmd: &str| std::process::Command::new("sh").arg("-c").arg(cmd).current_dir(repo.path()).output().unwrap();
        sh("git init -q && git config user.email a@b.c && git config user.name t && echo v1 > app.txt && git add -A && git commit -qm init");
        let r = rig_in(Some(repo.path())).await;

        // 沒有任何檔案變更：驗證沒過，但回覆照常交回。
        let a = r.add_item_as("沒改東西", "email", Level::Internal).await;
        r.tick().await;
        assert!(r.wait_for(&dispatcher::marker_for(&a.id)).await);
        runtime::complete(r.app.handle(), &r.tab, &a.id, Some("我看了".into())).await;
        let done = r.server_item(&a.id).await;
        assert_eq!((done.status.as_str(), done.verified), (store::DRAFT, Some(false)));
        assert!(done.verification.unwrap().contains("沒有偵測到任何檔案變更"));

        // LINE 預設只診斷：不驗證，草稿裡沒有驗證報告。
        let b = r.add_item_as("只診斷", "line", Level::Internal).await;
        r.tick().await;
        assert!(r.wait_for(&dispatcher::marker_for(&b.id)).await);
        runtime::complete(r.app.handle(), &r.tab, &b.id, Some("請重開機".into())).await;
        let done = r.server_item(&b.id).await;
        assert_eq!((done.reply_draft.as_deref(), done.verification, done.verified), (Some("請重開機"), None, None));
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nothing_is_claimed_while_the_tab_is_busy_or_the_item_is_not_yet_approved() {
        let r = rig().await;
        let pending = r.add_item("待核准", store::PENDING).await;
        r.tick().await;
        assert_eq!(r.server_item(&pending.id).await.status, store::PENDING, "沒核准的不會被自動領走");
        let a = r.add_item("一", store::QUEUED).await;
        let b = r.add_item("二", store::QUEUED).await;
        r.tick().await;
        r.tick().await;
        let running = [r.server_item(&a.id).await, r.server_item(&b.id).await].iter().filter(|i| i.status == store::RUNNING).count();
        assert_eq!(running, 1, "分頁忙著的時候不會再領第二題");
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_manual_claim_dispatches_to_a_collecting_tab_with_attachments_downloaded_locally() {
        use aiterm_intake_core::attach::{save_all, Raw};
        let r = rig().await;
        let item = r.add_item("附檔的題目", store::QUEUED).await;
        let (saved, _) = save_all(&r.srv.dir.path().join("attachments"), &item.id, vec![Raw { name: "err.log".into(), bytes: b"ERROR boom\n".to_vec() }]);
        store::add_attachment(&r.srv.state().pools.intake, &item.id, &saved[0]).await.unwrap();
        claim_manual(r.app.handle(), &item.id).await.expect("有收集中的分頁就能手動領");
        assert!(r.wait_for(&dispatcher::marker_for(&item.id)).await);
        let local = r._tmp.path().join(&item.id).join(&saved[0].filename);
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "ERROR boom\n", "附件下載到本機");
        assert!(r.wait_for(&local.to_string_lossy()).await, "提示詞裡寫的是本機路徑，Claude Code 才讀得到");
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_manual_claim_explains_when_there_is_no_usable_tab_and_does_not_take_the_item() {
        let r = rig().await;
        r.rt.registry.unregister(&r.tab);
        let item = r.add_item("題目", store::QUEUED).await;
        let err = claim_manual(r.app.handle(), &item.id).await.unwrap_err();
        assert!(err.contains("沒有可用的分頁") && err.contains("啟動問題收集"), "{err}");
        assert_eq!(r.server_item(&item.id).await.status, store::QUEUED, "沒有分頁就不該先把題目領走");
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closing_the_tab_reports_the_failure_to_the_server() {
        let r = rig().await;
        let item = r.add_item("題目", store::QUEUED).await;
        r.tick().await;
        assert_eq!(r.server_item(&item.id).await.status, store::RUNNING);
        r.pty.close(&r.tab).ok();
        r.tick().await;
        let g = r.server_item(&item.id).await;
        assert_eq!((g.status.as_str(), g.error.as_deref(), g.claimed_by), (store::PENDING, Some("接收的分頁已關閉"), None));
        assert!(!r.rs.is_claimed(&item.id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_lease_that_the_server_took_back_is_dropped_locally_and_the_tab_is_freed() {
        let r = rig().await;
        let item = r.add_item("題目", store::QUEUED).await;
        r.tick().await;
        sqlx::query("UPDATE intake_items SET claim_expires_at = 1 WHERE id = ?").bind(&item.id).execute(&r.srv.state().pools.intake).await.unwrap();
        aiterm_intake_server::channels::tick_once(r.srv.state()).await;
        assert_eq!(r.server_item(&item.id).await.status, store::QUEUED, "server 收回了");
        r.tick().await; // 續約失敗 → 放掉；同一輪又閒置了，會自動再領一次（這次是新的租約）
        let again = r.server_item(&item.id).await;
        assert_eq!((again.status.as_str(), again.claimed_by.as_deref()), (store::RUNNING, Some("小華")));
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stopping_collection_puts_the_item_back_for_someone_else() {
        let r = rig().await;
        let item = r.add_item("題目", store::QUEUED).await;
        r.tick().await;
        let id = r.rt.registry.unregister(&r.tab).expect("有進行中的題目");
        assert!(abandon(r.app.handle(), &id, "", true).await, "是從 server 領來的");
        let g = r.server_item(&item.id).await;
        assert_eq!((g.status.as_str(), g.claimed_by), (store::QUEUED, None), "放回待派送，別人可以領");
        r.pty.close(&r.tab).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn items_that_were_not_claimed_from_the_server_are_left_to_the_local_flow() {
        let r = rig().await;
        assert!(!complete(r.app.handle(), "local-item", Some("x".into())).await);
        assert!(!abandon(r.app.handle(), "local-item", "x", false).await);
        r.pty.close(&r.tab).ok();
    }
}
