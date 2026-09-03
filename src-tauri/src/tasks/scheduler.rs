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
use crate::tasks::{dispatch, monitor, TasksDb};

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

/// Abstracts "actually run this card" so the loop is testable without an
/// `AppHandle`. Production impl is `RealDispatcher`.
#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync {
    async fn dispatch(&self, db: &TasksDb, task: &TaskRow) -> Result<(), String>;
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
    async fn dispatch(&self, db: &TasksDb, task: &TaskRow) -> Result<(), String> {
        let attachments = store::list_attachments(&db.pool, &task.id)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| a.stored_path)
            .collect::<Vec<_>>();
        let prompt = dispatch::build_prompt(&task.body, &attachments);
        let claude_cmd = self.config.get().task_board.claude_command;

        let (tab_id, disp) = dispatch::spawn_and_run(
            &self.app,
            &self.pty,
            &task.project_dir,
            &claude_cmd,
            &prompt,
            !task.interactive,
        )
        .await?;
        store::mark_dispatched(&db.pool, &task.id, &tab_id)
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.app.emit("tasks-updated", ());

        let (cancel_tx, cancel_rx) = oneshot::channel::<monitor::WatchControl>();
        self.cancels.lock().insert(task.id.clone(), cancel_tx);

        let pool = db.pool.clone();
        let pty = self.pty.clone();
        let app = self.app.clone();
        let wake = self.wake.clone();
        let cancels = self.cancels.clone();
        let task_id = task.id.clone();
        let baselines = monitor::Baselines { bell: disp.bell_baseline, marker: disp.marker_baseline };
        let watch_mode = if task.interactive { monitor::WatchMode::Interactive } else { monitor::WatchMode::Auto };
        tauri::async_runtime::spawn(async move {
            let outcome = monitor::watch(
                &pty, &tab_id, cancel_rx, baselines, monitor::Thresholds::default(), watch_mode,
            ).await;
            let transcript = write_transcript(&pty, &task_id, &tab_id);
            let _ = store::finish_task(
                &pool, &task_id, outcome.as_str(), outcome.error_message(), transcript.as_deref(),
            ).await;
            cancels.lock().remove(&task_id);
            let _ = app.emit("tasks-updated", ());
            wake.notify_one();
        });
        Ok(())
    }
}

/// Snapshot the tab's recent output to `<task_dir>/transcript.txt`. Best
/// effort — returns the path on success, `None` (and logs) on failure.
fn write_transcript(pty: &PtyManager, task_id: &str, tab_id: &str) -> Option<String> {
    let text = pty.get_recent_output(tab_id, 200_000).unwrap_or_default();
    let dir = crate::tasks::task_dir(task_id);
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

/// Promote as many queued cards as the rules allow, right now. Shared by the
/// loop and by tests.
pub async fn drain_once(db: &TasksDb, dispatcher: &dyn Dispatcher, max_concurrent: u32) {
    loop {
        let running = match store::list_by_status(&db.pool, store::STATUS_RUNNING).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("scheduler list running: {e}");
                return;
            }
        };
        let queued = match store::list_by_status(&db.pool, store::STATUS_QUEUED).await {
            Ok(q) => q,
            Err(e) => {
                eprintln!("scheduler list queued: {e}");
                return;
            }
        };
        let Some(next) = pick_next(&running, &queued, max_concurrent) else {
            return;
        };
        let next = next.clone();
        if let Err(e) = dispatcher.dispatch(db, &next).await {
            eprintln!("dispatch {} failed: {e}", next.id);
            let _ = store::mark_dispatched(&db.pool, &next.id, "").await;
            let _ = store::finish_task(&db.pool, &next.id, "failed", Some(&e), None).await;
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
        // with the previous process).
        {
            let db = app.state::<TasksDb>();
            match store::recover_orphaned_running(&db.pool).await {
                Ok(n) if n > 0 => {
                    let _ = app.emit("tasks-updated", ());
                    eprintln!("task board: recovered {n} orphaned running card(s)");
                }
                Ok(_) => {}
                Err(e) => eprintln!("task board recovery scan: {e}"),
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
                let db = app.state::<TasksDb>();
                drain_once(&db, &dispatcher, max).await;
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
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use crate::tasks::store;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_db() -> TasksDb {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        TasksDb { pool }
    }

    /// A `Dispatcher` that never really spawns a PTY — it immediately marks
    /// the card running with a fake tab id and finishes it `success`, so we
    /// can assert the loop's promotion behaviour.
    struct FakeDispatcher;
    #[async_trait::async_trait]
    impl Dispatcher for FakeDispatcher {
        async fn dispatch(&self, db: &TasksDb, task: &TaskRow) -> Result<(), String> {
            store::mark_dispatched(&db.pool, &task.id, &format!("fake-{}", task.id))
                .await
                .map_err(|e| e.to_string())?;
            store::finish_task(&db.pool, &task.id, "success", None, None)
                .await
                .map_err(|e| e.to_string())
        }
    }

    #[tokio::test]
    async fn drain_once_promotes_the_whole_queue_when_the_fake_dispatcher_finishes_instantly() {
        let db = mem_db().await;
        for (t, par, ord) in [("a", true, 1.0), ("b", true, 2.0), ("solo", false, 3.0), ("c", true, 4.0)] {
            let id = store::create_task(&db.pool, t, "", "/r", par, false).await.unwrap();
            store::move_task(&db.pool, &id, store::STATUS_QUEUED, ord).await.unwrap();
        }
        drain_once(&db, &FakeDispatcher, 2).await;
        let all = store::list_tasks(&db.pool).await.unwrap();
        assert!(all.iter().all(|r| r.status == "done"), "{all:#?}");
    }

    #[tokio::test]
    async fn drain_once_stops_at_a_solo_head_while_something_runs() {
        let db = mem_db().await;
        // A dispatcher that marks running but never finishes (simulates a
        // long job), so the running set stays non-empty.
        struct StuckDispatcher;
        #[async_trait::async_trait]
        impl Dispatcher for StuckDispatcher {
            async fn dispatch(&self, db: &TasksDb, task: &TaskRow) -> Result<(), String> {
                store::mark_dispatched(&db.pool, &task.id, &format!("fake-{}", task.id))
                    .await.map_err(|e| e.to_string())
            }
        }
        for (t, par, ord) in [("p1", true, 1.0), ("solo", false, 2.0), ("p2", true, 3.0)] {
            let id = store::create_task(&db.pool, t, "", "/r", par, false).await.unwrap();
            store::move_task(&db.pool, &id, store::STATUS_QUEUED, ord).await.unwrap();
        }
        drain_once(&db, &StuckDispatcher, 5).await;
        // `create_task` assigns a UUID id; match on the human title instead.
        let by = |s: &str, all: &[TaskRow]| all.iter().filter(|r| r.status == s).map(|r| r.title.clone()).collect::<Vec<_>>();
        let all = store::list_tasks(&db.pool).await.unwrap();
        // p1 promoted; solo is the head and something's running → stop; p2 not skipped.
        assert_eq!(by("running", &all), vec!["p1"]);
        assert_eq!(by("queued", &all), vec!["solo", "p2"]);
    }
}
