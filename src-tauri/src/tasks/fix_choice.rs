//! 互動卡被審核者退回後，修正要「新開」還是「接回原對話」。
//!
//! 互動卡是使用者自己在對話裡做的；退回後若一律新開自動卡，原本對話裡的脈絡就丟了。
//! 所以來源是互動卡時，第一次退回先把修正卡擋住，等使用者選；選擇記在模組上，
//! 之後同一模組再退回直接照選。獨立一張小表而不是 `TaskRow` 欄位：TaskRow 在
//! 很多測試裡逐欄位字面建構，加欄位會牽動一大片。

use std::path::Path;

use serde::Serialize;
use sqlx::SqlitePool;

use crate::tasks::store::{self, TaskRow};
use crate::tasks::workflow::{KIND_FIX, KIND_REVIEW, KIND_WORK};

pub const MODE_FRESH: &str = "fresh";
pub const MODE_RESUME: &str = "resume";

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ChoiceRow {
    pub module_id: String,
    /// 記住的選擇（`fresh`／`resume`）；沒記就是 `None`，每次都問。
    pub mode: Option<String>,
    /// 正在等使用者選擇的修正卡。
    pub pending_fix_id: Option<String>,
    /// 選了「接回原對話」的那張修正卡：派工時要 `--resume` 原 session，
    /// 但提示詞仍是修正卡內文（跟「重啟中斷接著做」不同）。
    pub resume_fix_id: Option<String>,
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS fix_choice (
            module_id TEXT PRIMARY KEY NOT NULL,
            mode TEXT,
            pending_fix_id TEXT,
            resume_fix_id TEXT
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(pool: &SqlitePool, module_id: &str) -> Result<Option<ChoiceRow>, sqlx::Error> {
    sqlx::query_as::<_, ChoiceRow>("SELECT * FROM fix_choice WHERE module_id = ?").bind(module_id).fetch_optional(pool).await
}

async fn ensure_row(pool: &SqlitePool, module_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR IGNORE INTO fix_choice (module_id) VALUES (?)").bind(module_id).execute(pool).await?;
    Ok(())
}

pub async fn set_mode(pool: &SqlitePool, module_id: &str, mode: Option<&str>) -> Result<(), sqlx::Error> {
    ensure_row(pool, module_id).await?;
    sqlx::query("UPDATE fix_choice SET mode = ? WHERE module_id = ?").bind(mode).bind(module_id).execute(pool).await?;
    Ok(())
}

pub async fn set_pending(pool: &SqlitePool, module_id: &str, fix_id: Option<&str>) -> Result<(), sqlx::Error> {
    ensure_row(pool, module_id).await?;
    sqlx::query("UPDATE fix_choice SET pending_fix_id = ? WHERE module_id = ?").bind(fix_id).bind(module_id).execute(pool).await?;
    Ok(())
}

pub async fn set_resume_fix(pool: &SqlitePool, module_id: &str, fix_id: &str) -> Result<(), sqlx::Error> {
    ensure_row(pool, module_id).await?;
    sqlx::query("UPDATE fix_choice SET resume_fix_id = ? WHERE module_id = ?").bind(fix_id).bind(module_id).execute(pool).await?;
    Ok(())
}

/// 還在等使用者選擇的修正卡 id（不管那張卡現在的狀態；呼叫端自己篩 queued）。
pub async fn pending_fix_ids(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT pending_fix_id FROM fix_choice WHERE pending_fix_id IS NOT NULL").fetch_all(pool).await
}

/// 這張卡是不是「接回原對話」的修正卡。
pub async fn is_resume_fix(pool: &SqlitePool, task_id: &str) -> Result<bool, sqlx::Error> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fix_choice WHERE resume_fix_id = ?").bind(task_id).fetch_one(pool).await?;
    Ok(n > 0)
}

/// 退回後建修正卡的決定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 新開自動修正卡（現有行為）。
    Fresh,
    /// 接回原對話的互動修正卡。
    Resume,
    /// 卡先擋住，問使用者。
    Ask,
}

/// - 自動卡的模組：永遠 `Fresh`，行為完全不變。
/// - 互動卡、沒記過選擇：`Ask`。
/// - 記的是新開：`Fresh`（新開的代價低，不重問）。
/// - 記的是接回：第 2 輪起（`fails >= 2`）重問。審核結果裡的嚴重度只有 important／nit／
///   pre_existing，而不通過本來就代表有 important，分不出「特別嚴重」，所以改看輪數：
///   一修再修仍不過，多半是原本的對話走偏了，值得讓使用者重新決定。
pub fn decide(module_interactive: bool, mode: Option<&str>, fails: usize) -> Decision {
    if !module_interactive {
        return Decision::Fresh;
    }
    match mode {
        Some(MODE_FRESH) => Decision::Fresh,
        Some(MODE_RESUME) if fails < 2 => Decision::Resume,
        _ => Decision::Ask,
    }
}

/// 模組鏈上最後一張有 session id 的 work／fix 卡（含模組卡本身）的 session id。
/// 只認 Claude 的卡：別家 CLI 的 session 不能用 `claude --resume` 接。
pub fn resume_session_id(tasks: &[TaskRow], module_id: &str) -> Option<String> {
    tasks
        .iter()
        .filter(|t| {
            (t.id == module_id || t.module_id.as_deref() == Some(module_id))
                && (t.card_kind == KIND_WORK || t.card_kind == KIND_FIX)
                && t.cli.as_deref().is_none_or(|c| c.eq_ignore_ascii_case("claude"))
        })
        .filter(|t| t.session_id.as_deref().is_some_and(|s| !s.is_empty()))
        .max_by_key(|t| t.finished_at.unwrap_or(0))
        .and_then(|t| t.session_id.clone())
}

/// 該 session 的對話檔還在（可參考 `continue_chat`）：沒有就接不回。
pub fn session_resumable(projects_root: Option<&Path>, session_id: &str) -> bool {
    projects_root.is_some_and(|root| crate::tasks::session_log::find_session_log(root, session_id).is_some())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FixChoiceItem {
    pub module_id: String,
    pub module_title: String,
    pub fix_id: String,
    pub round: usize,
    pub can_resume: bool,
    pub remembered: Option<String>,
}

pub async fn list_pending(pool: &SqlitePool, projects_root: Option<&Path>) -> Result<Vec<FixChoiceItem>, String> {
    let tasks = store::list_tasks(pool).await.map_err(|e| e.to_string())?;
    let pending: Vec<ChoiceRow> = sqlx::query_as("SELECT * FROM fix_choice WHERE pending_fix_id IS NOT NULL ORDER BY rowid")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in pending {
        let Some(fix_id) = row.pending_fix_id.clone() else { continue };
        // 卡已被刪、封存或拖走就不算在等：殘留的列不該讓畫面一直問一張不存在的卡。
        if !tasks.iter().any(|t| t.id == fix_id && t.status == store::STATUS_QUEUED) {
            continue;
        }
        let Some(module) = tasks.iter().find(|t| t.id == row.module_id) else { continue };
        out.push(FixChoiceItem {
            module_id: row.module_id.clone(),
            module_title: module.title.clone(),
            fix_id,
            round: review_fails(&tasks, &row.module_id),
            can_resume: resume_session_id(&tasks, &row.module_id).is_some_and(|s| session_resumable(projects_root, &s)),
            remembered: row.mode.clone(),
        });
    }
    Ok(out)
}

fn review_fails(tasks: &[TaskRow], module_id: &str) -> usize {
    tasks
        .iter()
        .filter(|t| t.module_id.as_deref() == Some(module_id) && t.card_kind == KIND_REVIEW && t.verdict.as_deref() == Some("fail"))
        .count()
}

/// 套用使用者的選擇。`remember` 為真才記在模組上（之後同模組直接照選）。
/// 選接回但接不回（沒有 session／對話檔不見）直接回錯，不默默退回新開。
pub async fn choose(pool: &SqlitePool, module_id: &str, mode: &str, remember: bool, projects_root: Option<&Path>) -> Result<(), String> {
    if mode != MODE_FRESH && mode != MODE_RESUME {
        return Err(format!("不認得的選擇：{mode}"));
    }
    let pending = list_pending(pool, projects_root).await?.into_iter().find(|i| i.module_id == module_id);
    let Some(item) = pending else {
        return Err("這個模組現在沒有在等選擇的修正卡。".into());
    };
    if mode == MODE_RESUME {
        if !item.can_resume {
            return Err("找不到原本的對話檔（可能被清掉了），沒辦法接回，請選「新開修正卡」。".into());
        }
        let tasks = store::list_tasks(pool).await.map_err(|e| e.to_string())?;
        let Some(sid) = resume_session_id(&tasks, module_id) else {
            return Err("找不到原本的對話編號，沒辦法接回。".into());
        };
        sqlx::query("UPDATE tasks SET interactive = 1, session_id = ? WHERE id = ?")
            .bind(&sid)
            .bind(&item.fix_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
        set_resume_fix(pool, module_id, &item.fix_id).await.map_err(|e| e.to_string())?;
    }
    if remember {
        set_mode(pool, module_id, Some(mode)).await.map_err(|e| e.to_string())?;
    }
    set_pending(pool, module_id, None).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    /// 模組卡＋一張排隊中、等選擇的修正卡；回傳 (module_id, fix_id)。
    async fn module_with_pending_fix(pool: &SqlitePool, module_session: Option<&str>) -> (String, String) {
        let m = store::create_task(pool, "A 登入", "做 X", "/repo", true, true).await.unwrap();
        sqlx::query("UPDATE tasks SET card_kind = 'work', status = 'done', outcome = 'success', session_id = ?, finished_at = 10 WHERE id = ?")
            .bind(module_session)
            .bind(&m)
            .execute(pool)
            .await
            .unwrap();
        let f = store::create_task(pool, "修正：A 登入（第 1 輪）", "改", "/repo", true, false).await.unwrap();
        sqlx::query("UPDATE tasks SET card_kind = 'fix', status = 'queued', module_id = ? WHERE id = ?")
            .bind(&m)
            .bind(&f)
            .execute(pool)
            .await
            .unwrap();
        set_pending(pool, &m, Some(&f)).await.unwrap();
        (m, f)
    }

    fn root_with_log(sid: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("-some-project")).unwrap();
        std::fs::write(d.path().join("-some-project").join(format!("{sid}.jsonl")), "{}").unwrap();
        d
    }

    #[test]
    fn auto_modules_never_ask_and_interactive_modules_ask_until_a_choice_exists() {
        assert_eq!(decide(false, None, 1), Decision::Fresh);
        assert_eq!(decide(false, Some(MODE_RESUME), 1), Decision::Fresh, "自動卡的模組不受記住的選擇影響");
        assert_eq!(decide(true, None, 1), Decision::Ask);
        assert_eq!(decide(true, Some(MODE_FRESH), 1), Decision::Fresh);
        assert_eq!(decide(true, Some(MODE_RESUME), 1), Decision::Resume);
    }

    #[test]
    fn a_remembered_resume_is_asked_again_from_the_second_round_but_a_remembered_fresh_is_not() {
        assert_eq!(decide(true, Some(MODE_RESUME), 2), Decision::Ask);
        assert_eq!(decide(true, Some(MODE_RESUME), 3), Decision::Ask);
        assert_eq!(decide(true, Some(MODE_FRESH), 2), Decision::Fresh);
    }

    #[tokio::test]
    async fn the_resume_session_is_the_latest_work_or_fix_card_with_one() {
        let pool = mem_pool().await;
        let (m, f) = module_with_pending_fix(&pool, Some("sess-module")).await;
        // 審查卡自己也有 session id，但不是工程師的對話，不能被選到。
        let r = store::create_task(&pool, "審查", "", "/repo", true, false).await.unwrap();
        sqlx::query("UPDATE tasks SET card_kind = 'review', module_id = ?, session_id = 'sess-review', finished_at = 99 WHERE id = ?")
            .bind(&m)
            .bind(&r)
            .execute(&pool)
            .await
            .unwrap();
        let tasks = store::list_tasks(&pool).await.unwrap();
        assert_eq!(resume_session_id(&tasks, &m).as_deref(), Some("sess-module"));

        // 之後的修正卡跑過、有自己的 session：以它為準。
        sqlx::query("UPDATE tasks SET session_id = 'sess-fix', finished_at = 20 WHERE id = ?").bind(&f).execute(&pool).await.unwrap();
        let tasks = store::list_tasks(&pool).await.unwrap();
        assert_eq!(resume_session_id(&tasks, &m).as_deref(), Some("sess-fix"));

        // 別家 CLI 跑的卡不能 claude --resume。
        sqlx::query("UPDATE tasks SET cli = 'codex' WHERE id = ?").bind(&f).execute(&pool).await.unwrap();
        let tasks = store::list_tasks(&pool).await.unwrap();
        assert_eq!(resume_session_id(&tasks, &m).as_deref(), Some("sess-module"));
    }

    #[test]
    fn a_session_is_resumable_only_when_its_log_file_exists() {
        let root = root_with_log("sess-1");
        assert!(session_resumable(Some(root.path()), "sess-1"));
        assert!(!session_resumable(Some(root.path()), "sess-other"));
        assert!(!session_resumable(None, "sess-1"));
    }

    #[tokio::test]
    async fn choosing_resume_turns_the_fix_card_interactive_with_the_original_session_and_unblocks_it() {
        let pool = mem_pool().await;
        let (m, f) = module_with_pending_fix(&pool, Some("sess-1")).await;
        let root = root_with_log("sess-1");

        choose(&pool, &m, MODE_RESUME, true, Some(root.path())).await.unwrap();

        let fix = store::get_task(&pool, &f).await.unwrap().unwrap();
        assert!(fix.interactive);
        assert_eq!(fix.session_id.as_deref(), Some("sess-1"));
        let row = get(&pool, &m).await.unwrap().unwrap();
        assert_eq!(row.pending_fix_id, None);
        assert_eq!(row.mode.as_deref(), Some(MODE_RESUME));
        assert!(is_resume_fix(&pool, &f).await.unwrap());
        assert!(pending_fix_ids(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn choosing_fresh_leaves_the_fix_card_automatic_and_unblocks_it() {
        let pool = mem_pool().await;
        let (m, f) = module_with_pending_fix(&pool, Some("sess-1")).await;

        choose(&pool, &m, MODE_FRESH, true, None).await.unwrap();

        let fix = store::get_task(&pool, &f).await.unwrap().unwrap();
        assert!(!fix.interactive);
        assert_eq!(fix.session_id, None);
        let row = get(&pool, &m).await.unwrap().unwrap();
        assert_eq!((row.pending_fix_id, row.mode.as_deref()), (None, Some(MODE_FRESH)));
        assert!(!is_resume_fix(&pool, &f).await.unwrap());
    }

    #[tokio::test]
    async fn without_remember_the_choice_applies_once_and_the_next_round_asks_again() {
        let pool = mem_pool().await;
        let (m, _f) = module_with_pending_fix(&pool, Some("sess-1")).await;
        choose(&pool, &m, MODE_FRESH, false, None).await.unwrap();
        let row = get(&pool, &m).await.unwrap().unwrap();
        assert_eq!((row.pending_fix_id, row.mode), (None, None));
    }

    #[tokio::test]
    async fn choosing_resume_without_a_session_log_is_an_error_and_changes_nothing() {
        let pool = mem_pool().await;
        let (m, f) = module_with_pending_fix(&pool, Some("sess-gone")).await;
        let empty = tempfile::tempdir().unwrap();

        let err = choose(&pool, &m, MODE_RESUME, true, Some(empty.path())).await.unwrap_err();

        assert!(err.contains("對話"), "{err}");
        let fix = store::get_task(&pool, &f).await.unwrap().unwrap();
        assert!(!fix.interactive && fix.session_id.is_none());
        let row = get(&pool, &m).await.unwrap().unwrap();
        assert_eq!((row.pending_fix_id.as_deref(), row.mode), (Some(f.as_str()), None), "還是等選擇，也沒記下失敗的選擇");
    }

    #[tokio::test]
    async fn choosing_with_nothing_pending_is_an_error() {
        let pool = mem_pool().await;
        let (m, _f) = module_with_pending_fix(&pool, Some("s")).await;
        choose(&pool, &m, MODE_FRESH, true, None).await.unwrap();
        assert!(choose(&pool, &m, MODE_FRESH, true, None).await.is_err());
        assert!(choose(&pool, &m, "bogus", true, None).await.is_err());
    }

    #[tokio::test]
    async fn the_pending_list_reports_round_resumability_and_the_remembered_choice() {
        let pool = mem_pool().await;
        let (m, f) = module_with_pending_fix(&pool, Some("sess-1")).await;
        // 一張判 fail 的審查卡 → 第 1 輪。
        let r = store::create_task(&pool, "審查", "", "/repo", true, false).await.unwrap();
        sqlx::query("UPDATE tasks SET card_kind = 'review', module_id = ?, verdict = 'fail', status = 'done' WHERE id = ?")
            .bind(&m)
            .bind(&r)
            .execute(&pool)
            .await
            .unwrap();
        set_mode(&pool, &m, Some(MODE_RESUME)).await.unwrap();

        let with_log = root_with_log("sess-1");
        let items = list_pending(&pool, Some(with_log.path())).await.unwrap();
        assert_eq!(
            items,
            vec![FixChoiceItem {
                module_id: m.clone(),
                module_title: "A 登入".into(),
                fix_id: f.clone(),
                round: 1,
                can_resume: true,
                remembered: Some("resume".into()),
            }]
        );
        let without = tempfile::tempdir().unwrap();
        assert!(!list_pending(&pool, Some(without.path())).await.unwrap()[0].can_resume);
    }

    #[tokio::test]
    async fn a_pending_row_whose_card_is_no_longer_queued_is_not_listed_nor_blocking() {
        let pool = mem_pool().await;
        let (_m, f) = module_with_pending_fix(&pool, Some("sess-1")).await;
        sqlx::query("UPDATE tasks SET status = 'planning' WHERE id = ?").bind(&f).execute(&pool).await.unwrap();
        assert!(list_pending(&pool, None).await.unwrap().is_empty());
    }
}
