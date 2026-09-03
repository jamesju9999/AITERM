//! The dispatch loop. `pick_next` is the pure selection rule (heavily
//! tested); `run` is the long-lived `tokio` task that ties it to `tasks.db`,
//! a `Notify` wake signal, `dispatch`, and `monitor`.

use crate::tasks::store::TaskRow;

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
}
