//! The dispatch loop. `pick_next` is the pure selection rule (heavily
//! tested); `run` is the long-lived `tokio` task that ties it to `tasks.db`,
//! a `Notify` wake signal, `dispatch`, and `monitor`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{oneshot, Notify};

use crate::config::ConfigStore;
use crate::pty::manager::PtyManager;
use crate::tasks::store::{self};
use crate::tasks::store::TaskRow;
use crate::projects::{ProjectHandle, ProjectRegistry};
use crate::tasks::{dispatch, monitor};

/// Choose the next `queued` card to promote to `running`, or `None` if
/// nothing should start right now. `queued` MUST be sorted by `sort_order`
/// ascending (oldest first). Rules (see the design doc §5):
/// - a `parallel_ok = false` card currently running blocks all starts;
/// - respect the global `max_concurrent` cap;
/// - consider only the head of the queue (strict priority — never skip a
///   card to start a later one);
/// - a head card that is `parallel_ok = false` starts only when nothing is
///   running.
pub fn pick_next<'a>(
    running: &[TaskRow],
    queued: &'a [TaskRow],
    max_concurrent: u32,
) -> Option<&'a TaskRow> {
    if running.iter().any(|r| !r.parallel_ok) {
        return None;
    }
    if running.len() as u32 >= max_concurrent.max(1) {
        return None;
    }
    let head = queued.first()?;
    if !head.parallel_ok && !running.is_empty() {
        return None;
    }
    Some(head)
}

/// 各專案的隊首卡片依 `created_at` 由舊到新排序 —— 跨專案「先到先得」。
///
/// 刻意**不**把所有專案的 queued 卡片混在一起依 `sort_order` 排序：
/// `sort_order` 是各專案獨立的浮點數，混排毫無意義，而且會破壞使用者
/// 在單一專案內用拖曳決定的順序。每個專案只交出自己的隊首，專案內部
/// 的嚴格優先序因此完整保留。
pub fn order_heads(mut heads: Vec<(String, TaskRow)>) -> Vec<(String, TaskRow)> {
    heads.sort_by(|a, b| a.1.created_at.cmp(&b.1.created_at).then(a.1.id.cmp(&b.1.id)));
    heads
}

/// `task-finished` 的酬載：明確指出剛到達終局的是哪個專案的哪張卡片、
/// 用的是哪個分頁。前端靠它把對話記錄換成 xterm 序列化出來的乾淨版本
/// （那需要活著的 xterm 實例，只有前端有）。
#[derive(Clone, serde::Serialize)]
struct TaskFinishedEvent {
    project_id: String,
    task_id: String,
    tab_id: String,
}

/// Abstracts "actually run this card" so the loop is testable without an
/// `AppHandle`. Production impl is `RealDispatcher`.
#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync {
    async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String>;
}

/// The production dispatcher: spawn a visible `claude` tab, type the prompt,
/// mark the card running, then spawn a `monitor::watch` task that finishes
/// the card and re-pokes the scheduler when it ends.
pub struct RealDispatcher {
    pub app: AppHandle,
    pub pty: Arc<PtyManager>,
    pub config: Arc<ConfigStore>,
    pub wake: Arc<Notify>,
    /// task_id → cancel sender, so `tasks_stop` can abort a running watch.
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<monitor::WatchControl>>>>,
}

#[async_trait::async_trait]
impl Dispatcher for RealDispatcher {
    async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
        let attachments = store::list_attachments(&project.pool, &task.id)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| a.stored_path)
            .collect::<Vec<_>>();
        let prompt = dispatch::build_prompt(&task.body, &attachments);
        let claude_cmd = self.config.get().task_board.claude_command;
        // 只有指令確實是 claude 時才給 session id——別的指令接上這個旗標
        // 會直接啟動失敗（見 dispatch::looks_like_claude）。
        let session_id = dispatch::looks_like_claude(&claude_cmd)
            .then(|| uuid::Uuid::new_v4().to_string());

        let (tab_id, disp) = dispatch::spawn_and_run(
            &self.app,
            &self.pty,
            &task.project_dir,
            &claude_cmd,
            session_id.as_deref(),
            &prompt,
            !task.interactive,
        )
        .await?;
        // 卡片在進到這裡之前就已經被 `claim_for_dispatch` 標成 running 了
        // （見 drain_once）——這裡只補上分頁 id。順序不能倒過來：spawn 要
        // 等 claude 的 TUI 起來、最久 30 秒，那段期間卡片若還是 queued，
        // 下一次掃描就會再派一次同一張。
        store::set_tab_id(&project.pool, &task.id, &tab_id)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(sid) = session_id.as_deref() {
            if let Err(e) = store::set_session_id(&project.pool, &task.id, sid).await {
                // 記不起來只代表這張卡片拿不到完整記錄，不值得讓派工失敗。
                eprintln!("set_session_id {}: {e}", task.id);
            }
        }
        let _ = self.app.emit("tasks-updated", ());

        let (cancel_tx, cancel_rx) = oneshot::channel::<monitor::WatchControl>();
        self.cancels.lock().insert(task.id.clone(), cancel_tx);

        // 這個 async block 捕獲該專案的 pool 與資料夾路徑的複本，所以
        // watch 結束後的寫回完全不需要回頭查 registry。
        let pool = project.pool.clone();
        let project_path = project.path.clone();
        let project_id = project.id.clone();
        let pty = self.pty.clone();
        let app = self.app.clone();
        let wake = self.wake.clone();
        let cancels = self.cancels.clone();
        let task_id = task.id.clone();
        let work_dir = std::path::PathBuf::from(&task.project_dir);
        // move 而不是 clone——上面兩處用的都是 `as_deref()`，`session_id`
        // 之後不再被碰。
        let session_id_for_watch = session_id;
        let baselines = monitor::Baselines { bell: disp.bell_baseline, marker: disp.marker_baseline };
        let watch_mode = if task.interactive { monitor::WatchMode::Interactive } else { monitor::WatchMode::Auto };
        tauri::async_runtime::spawn(async move {
            let outcome = monitor::watch(
                &pty, &tab_id, cancel_rx, baselines, monitor::Thresholds::default(), watch_mode,
            ).await;
            let transcript = write_transcript(&pty, &project_path, &task_id, &tab_id);
            persist_outcome(
                &pool,
                crate::tasks::session_log::claude_projects_root().as_deref(),
                &work_dir,
                session_id_for_watch.as_deref(),
                &project_path,
                &task_id,
                transcript.as_deref(),
                &outcome,
            ).await;
            cancels.lock().remove(&task_id);
            let _ = app.emit("tasks-updated", ());
            // 帶資料的完成事件，給對話記錄乾淨化用。不能只靠 tasks-updated：
            // 那個沒有酬載，接收端得自己比對前後狀態才知道「哪一張剛完成」，
            // 而唯一在做這件事的地方是該專案的看板——但看板只有在該專案是
            // 當前分頁時才掛載，別的專案完成時根本沒人在聽。
            let _ = app.emit(
                "task-finished",
                TaskFinishedEvent {
                    project_id: project_id.clone(),
                    task_id: task_id.clone(),
                    tab_id: tab_id.clone(),
                },
            );
            wake.notify_one();
        });
        Ok(())
    }
}

/// Snapshot the tab's recent output to `<task_dir>/transcript.txt`. Best
/// effort — returns the path on success, `None` (and logs) on failure.
fn write_transcript(
    pty: &PtyManager,
    project_path: &std::path::Path,
    task_id: &str,
    tab_id: &str,
) -> Option<String> {
    let text = pty.get_recent_output(tab_id, 200_000).unwrap_or_default();
    let dir = crate::tasks::task_dir(project_path, task_id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("task transcript dir {dir:?}: {e}");
        return None;
    }
    let path = dir.join("transcript.txt");
    match std::fs::write(&path, text) {
        Ok(()) => Some(path.to_string_lossy().into_owned()),
        Err(e) => {
            eprintln!("write transcript {path:?}: {e}");
            None
        }
    }
}

/// 把一次執行的結果落地：完整的 session 記錄、然後結案。
///
/// 從 `RealDispatcher::dispatch` 的 watch closure 裡抽出來，因為那個 closure
/// 需要 `AppHandle`（`tauri` 依賴沒開 `test` feature，見 dispatch.rs 測試模組
/// 裡關於 `mock_builder` 的註解），而這一段不需要——抽出來就測得到。
///
/// 值得測的是參數的對應關係：`work_dir`（被施工的 repo）決定去
/// `~/.claude/projects` 的哪個資料夾找，`project_path`（卡片所屬的專案資料夾）
/// 決定複製到哪裡。兩個都是 `&Path`，交換了會編譯通過但靜默出錯——來源找不到
/// 看起來就像「沒有 session 檔」，目的地寫錯則沒有任何訊號。
///
/// `projects_root` 收 `Option<&Path>` 而不是自己呼叫 `claude_projects_root()`，
/// 純粹是為了讓測試能注入一棵假的樹。production 傳的就是那個函式的結果。
///
/// 順序是有意義的：`session_path` 必須在 `finish_task` 之前寫進去，因為
/// 呼叫端在 `finish_task` 之後緊接著發 `tasks-updated`，前端重載時該列
/// 必須已經是完整的。
async fn persist_outcome(
    pool: &sqlx::SqlitePool,
    projects_root: Option<&std::path::Path>,
    work_dir: &std::path::Path,
    session_id: Option<&str>,
    project_path: &std::path::Path,
    task_id: &str,
    transcript: Option<&str>,
    outcome: &monitor::TaskOutcome,
) {
    // 完整的逐輪記錄。找不到就什麼都不做——上面剛寫好的 transcript.txt
    // 還在，沒有東西壞掉。
    if let (Some(sid), Some(root)) = (session_id, projects_root) {
        // 參數分成兩組：前三個決定「去哪裡找」，後兩個決定「放到哪裡」。
        // `work_dir` 與 `project_path` 都是 `&Path` 而且意義完全不同
        // （前者是被施工的 repo，後者是卡片所屬的專案資料夾），交換了會
        // 編譯通過但靜默出錯——所以這裡照著分組寫，不要重排。
        if let Some(path) = crate::tasks::session_log::copy_session_log(
            root, work_dir, sid, project_path, task_id,
        ) {
            // 這一條比 `set_session_id` 的失敗嚴重得多，所以同樣要留下
            // 線索：檔案此刻已經躺在卡片資料夾裡了，路徑寫不進去的話它
            // 就變成沒有任何東西指向的孤兒，而使用者只會看到記錄莫名其妙
            // 退回終端機擷取。`set_session_id` 失敗則無害——watch closure
            // 手上本來就有自己的複本。
            if let Err(e) = store::set_session_path(pool, task_id, &path).await {
                eprintln!("set_session_path {task_id}: {e}");
            }
        }
    }
    let _ = store::finish_task(
        pool, task_id, outcome.as_str(), outcome.error_message(), transcript,
    ).await;
}

/// Promote as many queued cards as the rules allow, right now. Shared by the
/// loop and by tests. 跨所有已開啟的專案運作。
pub async fn drain_once(reg: &ProjectRegistry, dispatcher: &dyn Dispatcher, max_concurrent: u32) {
    // 資料夾被外部刪掉/搬走（或外接磁碟還沒掛上）的專案，這一輪整個跳過。
    //
    // 驅逐 registry 裡的 handle 是 `projects_list` 在做的，但那只有看板開著
    // 時才會跑，所以這裡不能假設 `reg.all()` 裡的每一個都還健在。而且下面
    // 收集 running 的迴圈遇到查詢失敗是 `return`——不先濾掉的話，**一個讀
    // 不到的專案會讓整個排程器停擺，連好的專案都不派工**。
    //
    // 這裡刻意只跳過、不驅逐：暫時讀不到（磁碟還沒掛上）之後回來時，
    // 下一輪就自動恢復。真正的驅逐留給 `projects_list`。
    let projects: Vec<ProjectHandle> = reg
        .all()
        .into_iter()
        .filter(|p| {
            let alive = p.path.is_dir();
            if !alive {
                eprintln!("scheduler: 專案資料夾不存在，這一輪跳過：{}", p.path.display());
            }
            alive
        })
        .collect();

    // Interactive cards bypass the concurrency cap and the solo-blocking
    // rule entirely — dispatch every queued one, unconditionally, first.
    // 逐專案處理即可：它們既不算並行額度也不受任何東西阻擋。
    for project in &projects {
        loop {
            let queued = match store::list_by_status(&project.pool, store::STATUS_QUEUED).await {
                Ok(q) => q,
                Err(e) => {
                    eprintln!("scheduler list queued (interactive pass, {}): {e}", project.name);
                    break;
                }
            };
            let Some(next) = queued.into_iter().find(|t| t.interactive) else {
                break;
            };
            // 先原子地認領再去 spawn——理由見 store::claim_for_dispatch。
            // 認領不到代表別人先拿走了，直接重掃。
            match store::claim_for_dispatch(&project.pool, &next.id).await {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    eprintln!("claim {} failed ({}): {e}", next.id, project.name);
                    break;
                }
            }
            if let Err(e) = dispatcher.dispatch(project, &next).await {
                eprintln!("dispatch {} failed: {e}", next.id);
                let _ = store::finish_task(&project.pool, &next.id, "failed", Some(&e), None).await;
            }
        }
    }

    // 一般卡片：running 是跨專案的聯集（全域上限），queued 則每個專案
    // 只交出自己的隊首，隊首之間依建立時間排序後逐一嘗試。
    loop {
        let mut running: Vec<TaskRow> = Vec::new();
        for project in &projects {
            match store::list_by_status(&project.pool, store::STATUS_RUNNING).await {
                Ok(r) => running.extend(r.into_iter().filter(|t| !t.interactive)),
                Err(e) => {
                    eprintln!("scheduler list running ({}): {e}", project.name);
                    return;
                }
            }
        }

        let mut heads: Vec<(String, TaskRow)> = Vec::new();
        for project in &projects {
            match store::list_by_status(&project.pool, store::STATUS_QUEUED).await {
                Ok(q) => {
                    if let Some(head) = q.into_iter().find(|t| !t.interactive) {
                        heads.push((project.id.clone(), head));
                    }
                }
                Err(e) => {
                    eprintln!("scheduler list queued ({}): {e}", project.name);
                    return;
                }
            }
        }

        let heads = order_heads(heads);
        let mut dispatched = false;
        for (project_id, head) in heads {
            if pick_next(&running, std::slice::from_ref(&head), max_concurrent).is_none() {
                continue;
            }
            let Some(project) = reg.get(&project_id) else {
                continue; // 專案在這一輪之間被關閉了
            };
            // 先原子地認領再去 spawn——理由見 store::claim_for_dispatch。
            match store::claim_for_dispatch(&project.pool, &head.id).await {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    eprintln!("claim {} failed ({}): {e}", head.id, project.name);
                    return;
                }
            }
            if let Err(e) = dispatcher.dispatch(&project, &head).await {
                eprintln!("dispatch {} failed: {e}", head.id);
                let _ = store::finish_task(&project.pool, &head.id, "failed", Some(&e), None).await;
            }
            dispatched = true;
            break;
        }
        if !dispatched {
            return;
        }
    }
}

/// Handed to the command layer so `tasks_move` / `tasks_stop` can wake the
/// scheduler and abort running watches.
#[derive(Clone)]
pub struct SchedulerHandle {
    pub wake: Arc<Notify>,
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<monitor::WatchControl>>>>,
}

impl SchedulerHandle {
    pub fn poke(&self) {
        self.wake.notify_one();
    }
    /// Fire the cancel signal for a running task, if present. Returns true
    /// if a watch was signalled.
    pub fn cancel(&self, task_id: &str) -> bool {
        self.send(task_id, monitor::WatchControl::Cancel)
    }
    /// Fire the manual-completion signal for a running task, if present.
    /// Returns true if a watch was signalled — see `commands::tasks::tasks_mark_done`
    /// for the fallback when this returns false (no active watch to signal).
    pub fn mark_done(&self, task_id: &str) -> bool {
        self.send(task_id, monitor::WatchControl::MarkDone)
    }
    fn send(&self, task_id: &str, msg: monitor::WatchControl) -> bool {
        if let Some(tx) = self.cancels.lock().remove(task_id) {
            let _ = tx.send(msg);
            true
        } else {
            false
        }
    }
}

/// Start the long-lived scheduler. Called once from `lib.rs` `.setup()`.
pub fn spawn(app: AppHandle) -> SchedulerHandle {
    let wake = Arc::new(Notify::new());
    let cancels = Arc::new(parking_lot::Mutex::new(HashMap::new()));
    let handle = SchedulerHandle { wake: wake.clone(), cancels: cancels.clone() };

    tauri::async_runtime::spawn(async move {
        let config = app.state::<Arc<ConfigStore>>().inner().clone();
        let pty = app.state::<Arc<PtyManager>>().inner().clone();

        // Startup recovery: clear orphaned `running` cards (their PTY died
        // with the previous process). 每個已開啟的專案各掃一次。
        {
            let reg = app.state::<ProjectRegistry>();
            let mut total = 0u64;
            for project in reg.all() {
                match store::recover_orphaned_running(&project.pool).await {
                    Ok(n) => total += n,
                    Err(e) => eprintln!("task board recovery scan ({}): {e}", project.name),
                }
            }
            if total > 0 {
                let _ = app.emit("tasks-updated", ());
                eprintln!("task board: recovered {total} orphaned running card(s)");
            }
        }

        let dispatcher = RealDispatcher {
            app: app.clone(),
            pty,
            config: config.clone(),
            wake: wake.clone(),
            cancels,
        };

        loop {
            let max = config.get().task_board.max_concurrent;
            {
                let reg = app.state::<ProjectRegistry>();
                drain_once(&reg, &dispatcher, max).await;
            }
            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
        }
    });

    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::store::TaskRow;

    fn row(id: &str, parallel_ok: bool, sort_order: f64) -> TaskRow {
        TaskRow {
            id: id.into(),
            title: id.into(),
            body: String::new(),
            project_dir: "/r".into(),
            status: "queued".into(),
            parallel_ok,
            interactive: false,
            sort_order,
            outcome: None,
            tab_id: None,
            transcript_path: None,
            error_message: None,
            created_at: String::new(),
            dispatched_at: None,
            finished_at: None,
            ai_summary: None,
            archived_at: None,
            session_id: None,
            session_path: None,
            use_bridge: false,
            bridge_tiers: None,
        }
    }

    #[test]
    fn picks_oldest_queued_when_a_slot_is_free() {
        let running: Vec<TaskRow> = vec![];
        let queued = vec![row("a", true, 1.0), row("b", true, 2.0)];
        assert_eq!(pick_next(&running, &queued, 2).map(|r| r.id.as_str()), Some("a"));
    }

    #[test]
    fn respects_the_global_cap() {
        let running = vec![row("x", true, 0.0), row("y", true, 0.0)];
        let queued = vec![row("a", true, 1.0)];
        assert!(pick_next(&running, &queued, 2).is_none());
    }

    #[test]
    fn a_solo_card_running_blocks_everything() {
        let running = vec![row("solo", false, 0.0)];
        let queued = vec![row("a", true, 1.0)];
        assert!(pick_next(&running, &queued, 4).is_none());
    }

    #[test]
    fn a_solo_card_at_the_head_waits_for_an_empty_running_set() {
        let running = vec![row("x", true, 0.0)];
        let queued = vec![row("solo", false, 1.0), row("b", true, 2.0)];
        // Strict priority: do NOT skip the solo card to run `b`.
        assert!(pick_next(&running, &queued, 4).is_none());
    }

    #[test]
    fn a_solo_card_at_the_head_runs_when_nothing_else_is() {
        let running: Vec<TaskRow> = vec![];
        let queued = vec![row("solo", false, 1.0)];
        assert_eq!(pick_next(&running, &queued, 4).map(|r| r.id.as_str()), Some("solo"));
    }

    #[test]
    fn empty_queue_picks_nothing() {
        let running: Vec<TaskRow> = vec![];
        assert!(pick_next(&running, &[], 4).is_none());
    }

    #[test]
    fn mark_done_sends_the_control_signal_and_returns_true_when_a_watch_is_registered() {
        let cancels = Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let handle = SchedulerHandle { wake: Arc::new(Notify::new()), cancels: cancels.clone() };
        let (tx, mut rx) = oneshot::channel::<monitor::WatchControl>();
        cancels.lock().insert("t1".to_string(), tx);
        assert!(handle.mark_done("t1"));
        assert!(matches!(rx.try_recv().unwrap(), monitor::WatchControl::MarkDone));
    }

    #[test]
    fn mark_done_returns_false_when_nothing_is_registered() {
        let handle = SchedulerHandle {
            wake: Arc::new(Notify::new()),
            cancels: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        };
        assert!(!handle.mark_done("nope"));
    }

    fn queued_row(id: &str, created_at: &str, parallel_ok: bool) -> TaskRow {
        TaskRow {
            id: id.to_string(),
            title: id.to_string(),
            body: String::new(),
            project_dir: "/r".to_string(),
            status: store::STATUS_QUEUED.to_string(),
            parallel_ok,
            interactive: false,
            sort_order: 1.0,
            outcome: None,
            tab_id: None,
            transcript_path: None,
            error_message: None,
            created_at: created_at.to_string(),
            dispatched_at: None,
            finished_at: None,
            ai_summary: None,
            archived_at: None,
            session_id: None,
            session_path: None,
            use_bridge: false,
            bridge_tiers: None,
        }
    }

    #[test]
    fn cross_project_heads_are_tried_oldest_first() {
        // 兩個專案各有一張 head，B 比較早建立 → 先選 B
        let heads = vec![
            ("proj-a".to_string(), queued_row("a1", "2026-09-04T10:00:00Z", true)),
            ("proj-b".to_string(), queued_row("b1", "2026-09-04T09:00:00Z", true)),
        ];
        let ordered = order_heads(heads);
        assert_eq!(ordered[0].1.id, "b1");
        assert_eq!(ordered[1].1.id, "a1");
    }

    #[test]
    fn a_blocked_solo_head_lets_another_project_start() {
        // 已有東西在跑 → A 的獨佔 head 起不來，但 B 的一般 head 可以
        let running = vec![queued_row("r1", "2026-09-04T08:00:00Z", true)];
        let heads = order_heads(vec![
            ("proj-a".to_string(), queued_row("a1", "2026-09-04T09:00:00Z", false)),
            ("proj-b".to_string(), queued_row("b1", "2026-09-04T10:00:00Z", true)),
        ]);

        let mut chosen = None;
        for (project_id, head) in &heads {
            let slice = std::slice::from_ref(head);
            if pick_next(&running, slice, 5).is_some() {
                chosen = Some((project_id.clone(), head.id.clone()));
                break;
            }
        }
        assert_eq!(chosen, Some(("proj-b".to_string(), "b1".to_string())));
    }

    #[test]
    fn a_running_solo_card_blocks_every_project() {
        // D6：獨佔卡片執行中時，所有專案都不派新工作
        let running = vec![queued_row("r1", "2026-09-04T08:00:00Z", false)];
        let heads = order_heads(vec![
            ("proj-a".to_string(), queued_row("a1", "2026-09-04T09:00:00Z", true)),
            ("proj-b".to_string(), queued_row("b1", "2026-09-04T10:00:00Z", true)),
        ]);
        for (_, head) in &heads {
            assert!(pick_next(&running, std::slice::from_ref(head), 5).is_none());
        }
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use crate::tasks::store;

    /// 一個建在暫存目錄裡的單專案 registry。`TempDir` 一併回傳，
    /// 呼叫端必須持有它直到測試結束，否則資料夾會被提前刪掉。
    async fn one_project_registry() -> (ProjectRegistry, ProjectHandle, tempfile::TempDir) {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let handle = reg.create(parent.path(), "test", "").await.unwrap();
        (reg, handle, parent)
    }

    /// 讓一個專案的資料夾「消失」。
    ///
    /// 必須先關掉連線池再刪資料夾：Windows 不允許刪除還有檔案被開著的
    /// 目錄，而 `tasks.db` 正被這個池子開著——直接 `remove_dir_all` 在
    /// Windows CI 上會失敗（實際踩過）。關掉池子不影響這幾個測試要驗的
    /// 東西：`drain_once` 是先看資料夾在不在，根本還沒碰到池子。
    async fn vanish(project: &ProjectHandle) {
        project.pool.close().await;
        std::fs::remove_dir_all(&project.path).unwrap();
    }

    /// 只記錄「誰被派工了」，不碰資料庫。資料夾已經被刪掉的專案沒辦法
    /// 事後查它的資料庫，所以這幾個測試改看派工紀錄。
    #[derive(Default)]
    struct RecordingDispatcher {
        dispatched: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingDispatcher {
        fn ids(&self) -> Vec<String> {
            self.dispatched.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Dispatcher for RecordingDispatcher {
        async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
            self.dispatched.lock().unwrap().push(task.id.clone());
            store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                .await
                .map_err(|e| e.to_string())?;
            store::finish_task(&project.pool, &task.id, "success", None, None)
                .await
                .map_err(|e| e.to_string())
        }
    }

    /// A `Dispatcher` that never really spawns a PTY — it immediately marks
    /// the card running with a fake tab id and finishes it `success`, so we
    /// can assert the loop's promotion behaviour.
    struct FakeDispatcher;
    #[async_trait::async_trait]
    impl Dispatcher for FakeDispatcher {
        async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
            store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                .await
                .map_err(|e| e.to_string())?;
            store::finish_task(&project.pool, &task.id, "success", None, None)
                .await
                .map_err(|e| e.to_string())
        }
    }

    #[tokio::test]
    async fn drain_once_promotes_the_whole_queue_when_the_fake_dispatcher_finishes_instantly() {
        let (reg, project, _parent) = one_project_registry().await;
        for (t, par, ord) in [("a", true, 1.0), ("b", true, 2.0), ("solo", false, 3.0), ("c", true, 4.0)] {
            let id = store::create_task(&project.pool, t, "", "/r", par, false).await.unwrap();
            store::move_task(&project.pool, &id, store::STATUS_QUEUED, ord).await.unwrap();
        }
        drain_once(&reg, &FakeDispatcher, 2).await;
        let all = store::list_tasks(&project.pool).await.unwrap();
        assert!(all.iter().all(|r| r.status == "done"), "{all:#?}");
    }

    #[tokio::test]
    async fn drain_once_stops_at_a_solo_head_while_something_runs() {
        let (reg, project, _parent) = one_project_registry().await;
        // A dispatcher that marks running but never finishes (simulates a
        // long job), so the running set stays non-empty.
        struct StuckDispatcher;
        #[async_trait::async_trait]
        impl Dispatcher for StuckDispatcher {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                    .await.map_err(|e| e.to_string())
            }
        }
        for (t, par, ord) in [("p1", true, 1.0), ("solo", false, 2.0), ("p2", true, 3.0)] {
            let id = store::create_task(&project.pool, t, "", "/r", par, false).await.unwrap();
            store::move_task(&project.pool, &id, store::STATUS_QUEUED, ord).await.unwrap();
        }
        drain_once(&reg, &StuckDispatcher, 5).await;
        // `create_task` assigns a UUID id; match on the human title instead.
        let by = |s: &str, all: &[TaskRow]| all.iter().filter(|r| r.status == s).map(|r| r.title.clone()).collect::<Vec<_>>();
        let all = store::list_tasks(&project.pool).await.unwrap();
        // p1 promoted; solo is the head and something's running → stop; p2 not skipped.
        assert_eq!(by("running", &all), vec!["p1"]);
        assert_eq!(by("queued", &all), vec!["solo", "p2"]);
    }

    #[tokio::test]
    async fn drain_once_dispatches_interactive_cards_even_when_the_auto_lane_is_full() {
        let (reg, project, _parent) = one_project_registry().await;
        // Seed one auto card already running — fills a cap of 1.
        let running_id = store::create_task(&project.pool, "already-running", "", "/r", true, false).await.unwrap();
        store::move_task(&project.pool, &running_id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&project.pool, &running_id, "tab-already").await;

        // A second auto card, queued — cap=1 means this must stay queued.
        let blocked_id = store::create_task(&project.pool, "blocked-auto", "", "/r", true, false).await.unwrap();
        store::move_task(&project.pool, &blocked_id, store::STATUS_QUEUED, 2.0).await.unwrap();

        // An interactive card, queued — must dispatch anyway, cap or no cap.
        let interactive_id = store::create_task(&project.pool, "chat", "", "/r", true, true).await.unwrap();
        store::move_task(&project.pool, &interactive_id, store::STATUS_QUEUED, 3.0).await.unwrap();

        struct RecordingDispatcher {
            dispatched: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl Dispatcher for RecordingDispatcher {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                self.dispatched.lock().unwrap().push(task.title.clone());
                store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                    .await
                    .map_err(|e| e.to_string())
            }
        }
        let dispatcher = RecordingDispatcher { dispatched: std::sync::Mutex::new(vec![]) };
        drain_once(&reg, &dispatcher, 1).await;

        assert_eq!(*dispatcher.dispatched.lock().unwrap(), vec!["chat".to_string()]);

        let all = store::list_tasks(&project.pool).await.unwrap();
        let status_of = |t: &str| all.iter().find(|r| r.title == t).unwrap().status.clone();
        assert_eq!(status_of("already-running"), "running");
        assert_eq!(status_of("blocked-auto"), "queued"); // cap=1 respected for the auto lane
        assert_eq!(status_of("chat"), "running"); // interactive bypassed the cap entirely
    }

    /// `tests::a_blocked_solo_head_lets_another_project_start` only exercises
    /// `pick_next` + `order_heads` through a loop written inside the test, so
    /// it cannot catch a regression in `drain_once` itself. This one drives the
    /// real thing: alpha's head is a solo card that cannot start (something is
    /// already running), and beta's head must still go out.
    #[tokio::test]
    async fn drain_once_skips_a_blocked_solo_head_and_starts_another_projects_card() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let alpha = reg.create(parent.path(), "alpha", "").await.unwrap();
        let beta = reg.create(parent.path(), "beta", "").await.unwrap();

        // Something already running, so a solo head is blocked.
        let busy = store::create_task(&alpha.pool, "busy", "", "/r", true, false).await.unwrap();
        store::move_task(&alpha.pool, &busy, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&alpha.pool, &busy, "tab-busy").await;

        let solo = store::create_task(&alpha.pool, "solo", "", "/r", false, false).await.unwrap();
        store::move_task(&alpha.pool, &solo, store::STATUS_QUEUED, 2.0).await.unwrap();
        let other = store::create_task(&beta.pool, "other", "", "/r", true, false).await.unwrap();
        store::move_task(&beta.pool, &other, store::STATUS_QUEUED, 1.0).await.unwrap();

        // `created_at` has one-second resolution, so cards made in the same
        // test would tie and fall back to a random-UUID tiebreak. Pin them so
        // alpha's blocked head is definitely tried first.
        set_created_at(&alpha.pool, &solo, "2026-09-04T09:00:00Z").await;
        set_created_at(&beta.pool, &other, "2026-09-04T10:00:00Z").await;

        struct StuckDispatcher;
        #[async_trait::async_trait]
        impl Dispatcher for StuckDispatcher {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                    .await
                    .map_err(|e| e.to_string())
            }
        }
        drain_once(&reg, &StuckDispatcher, 5).await;

        assert_eq!(
            store::get_task(&alpha.pool, &solo).await.unwrap().unwrap().status,
            "queued",
            "獨佔卡片在有東西執行中時不可啟動"
        );
        assert_eq!(
            store::get_task(&beta.pool, &other).await.unwrap().unwrap().status,
            "running",
            "別的專案的隊首不該被 alpha 的獨佔卡片擋住"
        );
    }

    async fn set_created_at(pool: &sqlx::SqlitePool, id: &str, created_at: &str) {
        sqlx::query("UPDATE tasks SET created_at = ?1 WHERE id = ?2")
            .bind(created_at)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn drain_once_does_not_let_a_running_interactive_card_count_against_the_auto_cap() {
        let (reg, project, _parent) = one_project_registry().await;
        // An interactive card already running should NOT occupy the auto
        // lane's capacity slot — an auto card queued behind it must still
        // start immediately even at cap=1.
        let chat_id = store::create_task(&project.pool, "chat", "", "/r", true, true).await.unwrap();
        store::move_task(&project.pool, &chat_id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&project.pool, &chat_id, "tab-chat").await;

        let auto_id = store::create_task(&project.pool, "auto", "", "/r", true, false).await.unwrap();
        store::move_task(&project.pool, &auto_id, store::STATUS_QUEUED, 2.0).await.unwrap();

        struct FakeDispatcher;
        #[async_trait::async_trait]
        impl Dispatcher for FakeDispatcher {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                    .await
                    .map_err(|e| e.to_string())
            }
        }
        drain_once(&reg, &FakeDispatcher, 1).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        let status_of = |t: &str| all.iter().find(|r| r.title == t).unwrap().status.clone();
        assert_eq!(status_of("chat"), "running");
        assert_eq!(status_of("auto"), "running"); // not blocked by the already-running interactive card
    }

    /// 派工要等 `claude` 的 TUI 起來，最久 30 秒。如果卡片在那之前還留在
    /// `queued`，第二次掃描就會再派一次同一張——實機出現過同一張卡開出
    /// 兩個 claude 行程（同一個 App、同一個工作目錄、相隔 32 秒，而卡片的
    /// `dispatched_at` 從頭到尾是空的，證明 spawn 還沒回來）。
    ///
    /// 這裡用一個「永遠不回來」的 dispatcher 模擬那 30 秒，然後再掃一次：
    /// 同一張卡不可以被派第二次。
    #[tokio::test]
    async fn a_card_being_dispatched_is_not_dispatched_again_by_the_next_sweep() {
        let (reg, project, _parent) = one_project_registry().await;
        let id = store::create_task(&project.pool, "slow", "", "/r", true, false).await.unwrap();
        store::move_task(&project.pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();

        /// 只記錄、完全不碰資料庫——模擬 spawn_and_run 還卡在等 TUI。
        #[derive(Default)]
        struct NeverFinishes {
            seen: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl Dispatcher for NeverFinishes {
            async fn dispatch(&self, _p: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                self.seen.lock().unwrap().push(task.id.clone());
                Ok(())
            }
        }

        // 包 timeout 而不是直接 await：這個 bug 退化回去的話，drain_once 的
        // 一般通道會**無限迴圈**（派工成功 → 重掃 → 卡片還是 queued → 再派），
        // 直接 await 會讓 CI 卡到整場逾時才死，看不出是誰壞的。
        let d = NeverFinishes::default();
        for _ in 0..2 {
            tokio::time::timeout(std::time::Duration::from_secs(5), drain_once(&reg, &d, 5))
                .await
                .expect("drain_once 沒有結束——卡片派工後仍留在 queued，掃描迴圈停不下來");
        }

        assert_eq!(
            d.seen.lock().unwrap().len(),
            1,
            "同一張卡被派了兩次——第二次掃描時它還留在 queued"
        );
    }

    /// 資料夾被 Finder 刪掉之後，registry 裡的 handle 不一定已經被驅逐
    /// （驅逐發生在 `projects_list`，而那只有看板開著時才會跑）。排程器
    /// 必須自己擋下來：在 Unix 上，已開啟的 SQLite 檔案被 unlink 之後
    /// inode 還在，查詢會照常「成功」——於是會派工給一個使用者已經刪掉
    /// 的專案，寫入還進到一個誰也看不到的檔案。
    #[tokio::test]
    async fn drain_once_skips_a_project_whose_folder_is_gone() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let gone = reg.create(parent.path(), "gone", "").await.unwrap();
        let alive = reg.create(parent.path(), "alive", "").await.unwrap();

        let ghost = store::create_task(&gone.pool, "ghost", "", "/r", true, false).await.unwrap();
        store::move_task(&gone.pool, &ghost, store::STATUS_QUEUED, 1.0).await.unwrap();
        let real = store::create_task(&alive.pool, "real", "", "/r", true, false).await.unwrap();
        store::move_task(&alive.pool, &real, store::STATUS_QUEUED, 1.0).await.unwrap();

        vanish(&gone).await;
        let dispatcher = RecordingDispatcher::default();
        drain_once(&reg, &dispatcher, 5).await;

        // 用派工紀錄判斷，不回頭查 gone 的資料庫——資料夾都刪了，那個
        // 連線池也已經關閉（Windows 上不關就刪不掉），查不動。
        let dispatched = dispatcher.ids();
        assert!(!dispatched.contains(&ghost), "資料夾已經不在的專案不該被派工");
        assert!(dispatched.contains(&real), "其他專案必須照常運作");
    }

    /// 互動卡片走的是另一條完全不受並行上限管制的通道，同樣不該從已消失
    /// 的專案派工。
    ///
    /// **這是行為描述，不是迴歸偵測器**——實際做過突變驗證：把上面
    /// `drain_once` 開頭的資料夾存在檢查整個拿掉，這個測試照樣是綠的。
    /// 因為互動通道的錯誤處理本來就是每個專案各自 `break`，一個讀不到的
    /// 專案只會讓它自己那圈結束，不會波及別人；真正會因為缺少檢查而全面
    /// 停擺的是下面那條一般通道（`return`），那個由
    /// `drain_once_skips_a_project_whose_folder_is_gone` 守著，而它會紅。
    /// 留著這個測試是為了釘住「互動通道也不會派工給消失的專案」這件事，
    /// 別誤以為它在保護那個過濾器。
    #[tokio::test]
    async fn drain_once_skips_a_vanished_project_in_the_interactive_lane_too() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let gone = reg.create(parent.path(), "gone", "").await.unwrap();

        let ghost = store::create_task(&gone.pool, "ghost", "", "/r", true, true).await.unwrap();
        store::move_task(&gone.pool, &ghost, store::STATUS_QUEUED, 1.0).await.unwrap();

        vanish(&gone).await;
        let dispatcher = RecordingDispatcher::default();
        drain_once(&reg, &dispatcher, 5).await;

        assert!(
            !dispatcher.ids().contains(&ghost),
            "互動卡片也不該從已消失的專案派出去"
        );
    }
}

/// `persist_outcome` 從 watch closure 抽出來就是為了讓這裡測得到——不需要
/// `AppHandle`、不需要 `PtyManager`，只需要一個真的 `SqlitePool` 與一棵
/// 假的 `~/.claude/projects` 樹。
#[cfg(test)]
mod persist_tests {
    use super::*;
    use crate::tasks::session_log::encode_project_dir;

    /// 獨立的記憶體 pool，帶完整 schema——比照 `store.rs` 測試模組裡
    /// `mem_pool` 的寫法（那邊是私有的，這裡自己開一份）。
    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    /// 建一棵假的 `~/.claude/projects` 樹：`<root>/<encode(work_dir)>/<sid>.jsonl`。
    fn fake_projects_root(work_dir: &std::path::Path, session_id: &str, body: &str) -> tempfile::TempDir {
        let projects = tempfile::tempdir().unwrap();
        let dir = projects.path().join(encode_project_dir(work_dir));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{session_id}.jsonl")), body).unwrap();
        projects
    }

    async fn queued_and_claimed(pool: &sqlx::SqlitePool) -> String {
        let id = store::create_task(pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        assert!(store::claim_for_dispatch(pool, &id).await.unwrap());
        id
    }

    /// 抓「參數對調」的測試。`work_dir`（決定去哪裡找）與 `project_path`
    /// （決定放到哪裡）刻意選成完全不同的路徑——`work_dir` 是一個不存在的
    /// 假路徑，`project_path` 是真的暫存資料夾。`persist_outcome` 內部若把
    /// 兩者對調傳給 `copy_session_log`，來源會在錯的資料夾名下找不到 session
    /// 檔，複製安靜失敗，`session_path` 留空——這裡斷言的正是「兩者都對」，
    /// 對調了一定紅（已用 mutation 手動驗證，見下面 mutation-verify 的說明）。
    #[tokio::test]
    async fn happy_path_copies_the_session_log_and_records_its_path() {
        let pool = mem_pool().await;
        let task_id = queued_and_claimed(&pool).await;

        let work_dir = std::path::Path::new("/fake/work/dir");
        let project = tempfile::tempdir().unwrap();
        let projects_root = fake_projects_root(work_dir, "SID", "{\"type\":\"user\"}\n");

        persist_outcome(
            &pool,
            Some(projects_root.path()),
            work_dir,
            Some("SID"),
            project.path(),
            &task_id,
            Some("raw transcript"),
            &monitor::TaskOutcome::Success,
        )
        .await;

        let row = store::get_task(&pool, &task_id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("success"));
        let session_path = row.session_path.expect("session_path 沒被寫入——參數順序可能對調了");
        assert!(session_path.ends_with("session.jsonl"), "{session_path}");
        assert_eq!(
            std::path::Path::new(&session_path).parent().unwrap(),
            crate::tasks::task_dir(project.path(), &task_id),
            "沒有存進卡片資料夾——work_dir/project_path 疑似對調了"
        );
        assert_eq!(std::fs::read_to_string(&session_path).unwrap(), "{\"type\":\"user\"}\n");
    }

    /// 沒有 session id（`claude_command` 不是 claude，或使用者沒設定）：
    /// 不複製，但卡片照樣結案，帶著原本的 transcript／錯誤訊息。
    #[tokio::test]
    async fn no_session_id_skips_the_copy_but_still_finishes() {
        let pool = mem_pool().await;
        let task_id = queued_and_claimed(&pool).await;
        let projects_root = tempfile::tempdir().unwrap(); // 有根，但沒有 session id 就不會被碰
        let project = tempfile::tempdir().unwrap();

        persist_outcome(
            &pool,
            Some(projects_root.path()),
            std::path::Path::new("/fake/work/dir"),
            None,
            project.path(),
            &task_id,
            Some("raw transcript"),
            &monitor::TaskOutcome::Failed("boom".to_string()),
        )
        .await;

        let row = store::get_task(&pool, &task_id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("failed"));
        assert_eq!(row.error_message.as_deref(), Some("boom"));
        assert_eq!(row.transcript_path.as_deref(), Some("raw transcript"));
        assert!(row.session_path.is_none(), "沒有 session id 卻還是複製了");
    }

    /// 只能觀察最終狀態：`session_path` 與結案欄位都必須被寫入。**這條不
    /// 證明複製先於結案發生**——那個順序是靠 `persist_outcome` 裡的敘述
    /// 順序保證的，不是這個測試。如果有人把 `finish_task` 搬到複製區塊
    /// 之前，這條測試不會注意到（兩個寫入的最終狀態不變，順序本身沒有
    /// 可觀察的副作用）。
    #[tokio::test]
    async fn both_the_session_path_and_the_finish_fields_end_up_persisted() {
        let pool = mem_pool().await;
        let task_id = queued_and_claimed(&pool).await;

        let work_dir = std::path::Path::new("/fake/work/dir2");
        let project = tempfile::tempdir().unwrap();
        let projects_root = fake_projects_root(work_dir, "SID2", "{}\n");

        persist_outcome(
            &pool,
            Some(projects_root.path()),
            work_dir,
            Some("SID2"),
            project.path(),
            &task_id,
            None,
            &monitor::TaskOutcome::Cancelled,
        )
        .await;

        let row = store::get_task(&pool, &task_id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("cancelled"));
        assert!(row.session_path.is_some(), "session_path 沒被寫入");
    }
}
