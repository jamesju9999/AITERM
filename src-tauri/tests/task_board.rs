//! End-to-end: a queued card is dispatched to a real PTY running a fake
//! agent script, and the scheduler+monitor drive it to done/success (and
//! done/failed for a non-zero exit). No `claude`, no `AppHandle`.

use std::sync::Arc;
use std::time::Duration;

use aiterm_lib::pty::manager::PtyManager;
use aiterm_lib::pty::session::done_marker;
use aiterm_lib::tasks::monitor::{Baselines, Thresholds};
use aiterm_lib::tasks::scheduler::{drain_once, Dispatcher};
use aiterm_lib::tasks::store::{self, TaskRow};
use aiterm_lib::tasks::{init_schema, TasksDb};
use portable_pty::PtySize;

struct RealPtyDispatcher {
    pty: Arc<PtyManager>,
    /// Shell snippet to run instead of `claude`. `{marker}` is replaced with
    /// this run's done-marker.
    script: String,
}

#[async_trait::async_trait]
impl Dispatcher for RealPtyDispatcher {
    async fn dispatch(&self, db: &TasksDb, task: &TaskRow) -> Result<(), String> {
        let tab_id = self
            .pty
            .create_with_callback(
                PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 },
                |_| {},
            )
            .map_err(|e| e.to_string())?;
        store::mark_dispatched(&db.pool, &task.id, &tab_id).await.map_err(|e| e.to_string())?;

        let script = self.script.replace("{marker}", &done_marker(&tab_id));
        self.pty
            .write(&tab_id, format!("{script}\n").as_bytes())
            .map_err(|e| e.to_string())?;

        let baselines = Baselines::default();
        let thresholds = Thresholds { quiet_stuck_ms: 10_000, poll_ms: 50, min_run_ms: 0 };
        let pty = self.pty.clone();
        let pool = db.pool.clone();
        let task_id = task.id.clone();
        tokio::spawn(async move {
            let (_tx, rx) = tokio::sync::oneshot::channel();
            let outcome =
                aiterm_lib::tasks::monitor::watch(&pty, &tab_id, rx, baselines, thresholds).await;
            let _ = store::finish_task(
                &pool,
                &task_id,
                outcome.as_str(),
                outcome.error_message(),
                None,
            )
            .await;
        });
        Ok(())
    }
}

async fn mem_db() -> TasksDb {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    TasksDb { pool }
}

async fn wait_done(db: &TasksDb, id: &str) -> TaskRow {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let row = store::get_task(&db.pool, id).await.unwrap().unwrap();
        if row.status == "done" {
            return row;
        }
        assert!(tokio::time::Instant::now() < deadline, "task {id} never finished: {row:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
#[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
async fn queued_card_dispatched_and_completed_via_marker() {
    let db = mem_db().await;
    let pty = Arc::new(PtyManager::new());
    let id = store::create_task(&db.pool, "print marker", "", "/", true).await.unwrap();
    store::move_task(&db.pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();

    let dispatcher = RealPtyDispatcher {
        pty,
        script: "printf 'working...\\n'; printf '%s\\n' '{marker}'".to_string(),
    };
    drain_once(&db, &dispatcher, 2).await;

    let row = wait_done(&db, &id).await;
    assert_eq!(row.outcome.as_deref(), Some("success"), "{row:?}");
}

#[tokio::test]
#[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
async fn card_that_exits_nonzero_is_marked_failed() {
    let db = mem_db().await;
    let pty = Arc::new(PtyManager::new());
    let id = store::create_task(&db.pool, "boom", "", "/", true).await.unwrap();
    store::move_task(&db.pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();

    let dispatcher = RealPtyDispatcher { pty, script: "sh -c 'exit 2'".to_string() };
    drain_once(&db, &dispatcher, 2).await;

    let row = wait_done(&db, &id).await;
    assert_eq!(row.outcome.as_deref(), Some("failed"), "{row:?}");
    assert!(row.error_message.unwrap_or_default().contains('2'));
}
