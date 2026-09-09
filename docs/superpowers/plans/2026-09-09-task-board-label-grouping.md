# 工作看板卡片 Label 分組 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a single-value, free-text `label` field to task-board cards and group cards by that label (collapsible, per-column) inside `ProjectBoard`, per `docs/superpowers/specs/2026-09-09-task-board-label-grouping-design.md`.

**Architecture:** A nullable `label` column added via the existing best-effort `ALTER TABLE` migration pattern. Backend exposes it through the existing "create, then set" pattern used for `use_bridge`/`bridge_tiers` (so `create_task`'s 60+ call sites are untouched) plus a `distinct_labels` lookup mirroring `distinct_project_dirs`. Frontend adds a text input + used-label chips to `TaskEditorDialog` (mirrors the existing `dir` field), a pure `groupByLabel` util consumed by `ProjectBoard`, a `TaskLabelGroup` collapsible wrapper, and a `labelColor` hash-to-hue helper for a fixed-dark-palette badge (the task board is always dark regardless of app theme — see `index.css:1-18`).

**Tech Stack:** Rust + sqlx (SQLite) backend, React 19 + TypeScript frontend, Vitest + React Testing Library, `cargo test`.

---

## Task 1: `label` column + migration (Rust)

**Files:**
- Modify: `src-tauri/src/tasks/mod.rs` (schema)
- Modify: `src-tauri/src/tasks/store.rs:15-54` (`TaskRow`)
- Test: `src-tauri/src/tasks/store.rs` (new `#[cfg(test)]` block, see step 1)

- [ ] **Step 1: Write the failing migration test**

Add to the existing `#[cfg(test)] mod ... ` block in `store.rs` that contains `init_schema_migrates_a_database_that_predates_the_session_columns` (the one starting at line ~1246), as a new test in the same module:

```rust
/// 舊資料庫（沒有 `label` 欄位）跑過 `init_schema` 之後必須能補上這個
/// 欄位並正常讀寫——跟旁邊的 session 欄位遷移測試同一個理由：
/// `ALTER TABLE` 失敗會被 `let _ =` 刻意吞掉，錯字不會有任何訊號。
#[tokio::test]
async fn init_schema_migrates_a_database_that_predates_the_label_column() {
    let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();

    // 完整但沒有 label 欄位的舊 schema。
    sqlx::query(
        "CREATE TABLE tasks (
            id              TEXT PRIMARY KEY NOT NULL,
            title           TEXT NOT NULL,
            body            TEXT NOT NULL DEFAULT '',
            project_dir     TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'planning',
            parallel_ok     INTEGER NOT NULL DEFAULT 1,
            interactive     INTEGER NOT NULL DEFAULT 0,
            sort_order      REAL NOT NULL DEFAULT 0,
            outcome         TEXT,
            tab_id          TEXT,
            transcript_path TEXT,
            error_message   TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            dispatched_at   INTEGER,
            finished_at     INTEGER,
            ai_summary      TEXT,
            archived_at     INTEGER,
            session_id      TEXT,
            session_path    TEXT,
            use_bridge      INTEGER NOT NULL DEFAULT 0,
            bridge_tiers    TEXT
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO tasks (id, title, body, project_dir) VALUES ('old1', 't', 'b', '/work/repo')")
        .execute(&pool)
        .await
        .unwrap();

    crate::tasks::init_schema(&pool).await.unwrap();

    let row = get_task(&pool, "old1").await.unwrap().unwrap();
    assert_eq!(row.label, None, "舊資料遷移後 label 應該是 NULL，不是欄位缺席");

    set_label(&pool, "old1", Some("緊急")).await.unwrap();
    assert_eq!(get_task(&pool, "old1").await.unwrap().unwrap().label.as_deref(), Some("緊急"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test init_schema_migrates_a_database_that_predates_the_label_column`
Expected: FAIL — compile error (`TaskRow` has no field `label`, `set_label` doesn't exist yet).

- [ ] **Step 3: Add the column + struct field + `set_label`**

In `src-tauri/src/tasks/mod.rs`, add `label TEXT` to the `CREATE TABLE IF NOT EXISTS tasks (...)` column list (anywhere among the nullable columns, e.g. right after `bridge_tiers TEXT`), and add a migration line right after the existing `bridge_tiers` migration (`mod.rs:91-93`):

```rust
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN bridge_tiers TEXT")
        .execute(pool)
        .await;
    // Migration: existing databases created before `label` existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN label TEXT")
        .execute(pool)
        .await;
```

In `src-tauri/src/tasks/store.rs`, add to `TaskRow` (right after `pub bridge_tiers: Option<String>,` at line 53):

```rust
    /// 使用者自由輸入的分類文字，用來在同一狀態欄內把卡片分組顯示。
    /// `None` 代表未分類。
    pub label: Option<String>,
```

Add `set_label` near `set_parallel_ok`/`set_interactive` (after `set_interactive`, `store.rs:251` area):

```rust
/// 設定卡片的 Label（分類用自由文字，`None` 代表清空）。跟
/// `set_parallel_ok`/`set_interactive`/`set_bridge_config` 同一種「建立後
/// 另外設定的欄位」模式，不擠進 `create_task` 的必要參數清單——`create_task`
/// 有六十幾個呼叫點，加必填參數會波及跟 Label 完全無關的檔案。
pub async fn set_label(pool: &SqlitePool, id: &str, label: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET label = ? WHERE id = ?")
        .bind(label)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 4: Fix the two hand-built `TaskRow` literals**

`TaskRow` now has a new required field, so both literal constructors in `src-tauri/src/tasks/scheduler.rs` fail to compile. Add `label: None,` right after `bridge_tiers: None,` in both:

- `scheduler.rs:499` (inside `fn row(...)`)
- `scheduler.rs:586` (inside `fn queued_row(...)`)

- [ ] **Step 5: Run test to verify it passes, and that nothing else broke**

Run: `cd src-tauri && cargo test init_schema_migrates_a_database_that_predates_the_label_column`
Expected: PASS

Run: `cd src-tauri && cargo test --lib tasks::`
Expected: all existing `tasks` module tests still PASS (this confirms the two `scheduler.rs` fixture fixes were correct and nothing else references `TaskRow` by positional/exhaustive construction).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/tasks/mod.rs src-tauri/src/tasks/store.rs src-tauri/src/tasks/scheduler.rs
git commit -m "feat(tasks): add label column to tasks table"
```

---

## Task 2: `clone_task_fields` copies `label`

**Files:**
- Modify: `src-tauri/src/tasks/store.rs:109-112`
- Test: same file, `#[cfg(test)] mod clone_tests` (or wherever `clone_task_fields` is already tested — search for `clone_task_fields` test names like `clone_task_fields_copies_...`)

- [ ] **Step 1: Write the failing test**

Find the existing test module that already covers `clone_task_fields` (grep `clone_task_fields(&pool` in `store.rs` — around line 1034-1080) and add:

```rust
#[tokio::test]
async fn clone_task_fields_carries_the_label_over() {
    let pool = mem_pool().await;
    let src = create_task(&pool, "Ship it", "the body", "/repo/x", false, false).await.unwrap();
    set_label(&pool, &src, Some("緊急")).await.unwrap();

    let new_id = clone_task_fields(&pool, &src).await.unwrap();

    assert_eq!(get_task(&pool, &new_id).await.unwrap().unwrap().label.as_deref(), Some("緊急"));
}
```

(Use whatever `mem_pool()` helper already exists in that test module — do not redefine it if one is in scope.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test clone_task_fields_carries_the_label_over`
Expected: FAIL (new clone has `label: None` because `clone_task_fields` doesn't copy it yet)

- [ ] **Step 3: Implement**

In `store.rs:109-112`:

```rust
pub async fn clone_task_fields(pool: &SqlitePool, src_id: &str) -> Result<String, sqlx::Error> {
    let src = get_task(pool, src_id).await?.ok_or(sqlx::Error::RowNotFound)?;
    let new_id = create_task(pool, &src.title, &src.body, &src.project_dir, src.parallel_ok, src.interactive).await?;
    set_label(pool, &new_id, src.label.as_deref()).await?;
    Ok(new_id)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test clone_task_fields`
Expected: PASS (both the new test and the existing `clone_task_fields_*` tests)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/store.rs
git commit -m "feat(tasks): clone_task_fields carries the label over"
```

---

## Task 3: `update_task_fields` accepts `label`

**Files:**
- Modify: `src-tauri/src/tasks/store.rs:380-390`
- Modify: `src-tauri/src/commands/tasks.rs:133-143` (its one call site)
- Test: `src-tauri/src/tasks/store.rs`

`update_task_fields` currently has no dedicated unit test and exactly one call site (`commands/tasks.rs:134`), so this is a low-risk signature change — but we still add a test since this is where label editing lands.

- [ ] **Step 1: Write the failing test**

Add near the other `store.rs` tests that call `get_task`/`create_task` directly (any `#[cfg(test)] mod` with `mem_pool()` in scope is fine — e.g. the module around line 930-1030):

```rust
#[tokio::test]
async fn update_task_fields_writes_the_label() {
    let pool = mem_pool().await;
    let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();

    update_task_fields(&pool, &id, "t", "", "/r", Some("文件")).await.unwrap();
    assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().label.as_deref(), Some("文件"));

    update_task_fields(&pool, &id, "t", "", "/r", None).await.unwrap();
    assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().label, None, "傳 None 要能清空既有的 label");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test update_task_fields_writes_the_label`
Expected: FAIL — compile error (wrong number of arguments to `update_task_fields`)

- [ ] **Step 3: Implement**

`store.rs:380-390`:

```rust
/// Edit title/body/project_dir/label. Caller (command layer) restricts this to `planning` cards.
pub async fn update_task_fields(
    pool: &SqlitePool,
    id: &str,
    title: &str,
    body: &str,
    project_dir: &str,
    label: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET title = ?, body = ?, project_dir = ?, label = ? WHERE id = ?")
        .bind(title)
        .bind(body)
        .bind(project_dir)
        .bind(label)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

Update the one call site, `commands/tasks.rs:133-143`:

```rust
    if edit_allowed(&row.status) {
        store::update_task_fields(
            &p.pool,
            &args.id,
            &args.title,
            &args.body,
            &args.project_dir,
            args.label.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
```

(`args.label` doesn't exist on `UpdateArgs` yet — Task 5 adds it. This will not compile until Task 5 lands; that's expected and fine because Task 5 comes right after and is part of the same logical change. If you want `cargo test update_task_fields_writes_the_label` to compile and pass in isolation before touching `commands/tasks.rs`, do Step 3's `store.rs` half only, run the test, then come back and patch `commands/tasks.rs` as part of Task 5.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test update_task_fields_writes_the_label`
Expected: PASS (this only needs the `store.rs` half; `commands/tasks.rs` can still be broken at this point — see note above)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/store.rs
git commit -m "feat(tasks): update_task_fields accepts a label"
```

(Leave `commands/tasks.rs` uncommitted/broken here if you followed the isolated-test path — Task 5 fixes it in the same work session before the next `cargo build` is expected to succeed. If you'd rather keep every commit green, do Task 5's `CreateArgs`/`UpdateArgs` additions first, then come back and finish this task's Step 3 in one shot — either order is fine, just don't leave the tree mid-task at the end of the session.)

---

## Task 4: `distinct_labels`

**Files:**
- Modify: `src-tauri/src/tasks/store.rs` (near `distinct_project_dirs`, line 579)
- Test: `src-tauri/src/tasks/store.rs` (same test module as `distinct_project_dirs_dedupes_and_sorts`, line ~1110)

- [ ] **Step 1: Write the failing tests**

Add next to `distinct_project_dirs_dedupes_and_sorts` / `distinct_project_dirs_skips_empty_strings`:

```rust
#[tokio::test]
async fn distinct_labels_dedupes_and_sorts() {
    let pool = mem_pool().await;
    let a = create_task(&pool, "a", "", "/r", true, false).await.unwrap();
    let b = create_task(&pool, "b", "", "/r", true, false).await.unwrap();
    let c = create_task(&pool, "c", "", "/r", true, false).await.unwrap();
    set_label(&pool, &a, Some("緊急")).await.unwrap();
    set_label(&pool, &b, Some("文件")).await.unwrap();
    set_label(&pool, &c, Some("緊急")).await.unwrap();

    assert_eq!(distinct_labels(&pool).await.unwrap(), vec!["文件".to_string(), "緊急".to_string()]);
}

#[tokio::test]
async fn distinct_labels_skips_null_and_empty() {
    let pool = mem_pool().await;
    let a = create_task(&pool, "a", "", "/r", true, false).await.unwrap();
    let b = create_task(&pool, "b", "", "/r", true, false).await.unwrap();
    set_label(&pool, &a, Some("")).await.unwrap();
    set_label(&pool, &b, Some("真的有分類")).await.unwrap();
    // 第三張卡從不呼叫 set_label，label 維持 NULL。
    create_task(&pool, "c", "", "/r", true, false).await.unwrap();

    assert_eq!(distinct_labels(&pool).await.unwrap(), vec!["真的有分類".to_string()]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test distinct_labels`
Expected: FAIL — `distinct_labels` doesn't exist yet

- [ ] **Step 3: Implement**

Add right after `distinct_project_dirs` (`store.rs:579-585`):

```rust
/// 這個專案的卡片用過的 Label，去重複＋排序。跟 `distinct_project_dirs`
/// 同一個用途——新增/編輯卡片時給一鍵選取，不必每次重新手打。
pub async fn distinct_labels(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT label FROM tasks WHERE label IS NOT NULL AND label <> '' ORDER BY label",
    )
    .fetch_all(pool)
    .await
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test distinct_labels`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/store.rs
git commit -m "feat(tasks): add distinct_labels lookup"
```

---

## Task 5: Command layer — `CreateArgs`/`UpdateArgs`/`tasks_used_labels` + register command

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs:64-146` (`CreateArgs`, `UpdateArgs`, `tasks_create`, `tasks_update`)
- Modify: `src-tauri/src/commands/tasks.rs` (new `tasks_used_labels`, near `tasks_used_dirs` at line 460)
- Modify: `src-tauri/src/lib.rs:112-117` and `:590-602`

No new Rust test here — `tasks_create`/`tasks_update`/`tasks_used_dirs` have no dedicated command-level tests in this codebase (they need a live `State<'_, ProjectRegistry>`, which existing tests avoid by testing `store::` functions directly instead — see `save_transcript_tests`/`transcript_for_row_tests` in the same file for the established pattern). The behavior these three thin wrappers depend on (`set_label`, `update_task_fields`, `distinct_labels`) is already covered by Tasks 1-4. This task is plumbing only.

- [ ] **Step 1: Add `label` to `CreateArgs`/`UpdateArgs`**

`commands/tasks.rs:64-73`:

```rust
#[derive(Deserialize)]
pub struct CreateArgs {
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
    pub use_bridge: bool,
    pub bridge_tiers: Option<String>,
    pub label: Option<String>,
}
```

`commands/tasks.rs:100-110`:

```rust
#[derive(Deserialize)]
pub struct UpdateArgs {
    pub id: String,
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
    pub use_bridge: bool,
    pub bridge_tiers: Option<String>,
    pub label: Option<String>,
}
```

- [ ] **Step 2: Wire `label` through `tasks_create`**

`commands/tasks.rs:75-98`:

```rust
#[tauri::command]
pub async fn tasks_create(
    project_id: String,
    args: CreateArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let id = store::create_task(
        &p.pool,
        &args.title,
        &args.body,
        &args.project_dir,
        args.parallel_ok,
        args.interactive,
    )
    .await
    .map_err(|e| e.to_string())?;
    store::set_bridge_config(&p.pool, &id, args.use_bridge, args.bridge_tiers)
        .await
        .map_err(|e| e.to_string())?;
    store::set_label(&p.pool, &id, args.label.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(id)
}
```

- [ ] **Step 3: Wire `label` through `tasks_update`**

`commands/tasks.rs:112-146` — if Task 3 already patched this file, confirm it matches; otherwise apply now:

```rust
#[tauri::command]
pub async fn tasks_update(
    project_id: String,
    args: UpdateArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &args.id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    store::set_parallel_ok(&p.pool, &args.id, args.parallel_ok)
        .await
        .map_err(|e| e.to_string())?;
    store::set_interactive(&p.pool, &args.id, args.interactive)
        .await
        .map_err(|e| e.to_string())?;
    store::set_bridge_config(&p.pool, &args.id, args.use_bridge, args.bridge_tiers)
        .await
        .map_err(|e| e.to_string())?;
    if edit_allowed(&row.status) {
        store::update_task_fields(
            &p.pool,
            &args.id,
            &args.title,
            &args.body,
            &args.project_dir,
            args.label.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    emit_updated(&app);
    Ok(())
}
```

- [ ] **Step 4: New `tasks_used_labels` command**

Add right after `tasks_used_dirs` (`commands/tasks.rs:460-466`):

```rust
/// 這個專案的卡片用過的 Label。跟 `tasks_used_dirs` 同一個用途——新增/
/// 編輯卡片時給一鍵選取，不必每次重新手打。
#[tauri::command]
pub async fn tasks_used_labels(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<String>, String> {
    let p = project(&reg, &project_id)?;
    store::distinct_labels(&p.pool).await.map_err(|e| e.to_string())
}
```

- [ ] **Step 5: Register the command in `lib.rs`**

`lib.rs:112-117`, add `tasks_used_labels` to the `use` list next to `tasks_used_dirs`:

```rust
    tasks::{
        tasks_list, tasks_create, tasks_update, tasks_move, tasks_stop, tasks_delete,
        tasks_add_attachment, tasks_remove_attachment, tasks_clone, tasks_read_transcript,
        tasks_save_transcript, tasks_mark_done, tasks_used_dirs, tasks_used_labels,
        tasks_set_summary, tasks_archive, tasks_unarchive, tasks_archive_done,
        tasks_list_archived,
    },
```

`lib.rs:590-602`, add it to `generate_handler!` next to `tasks_used_dirs`:

```rust
            tasks_used_dirs,
            tasks_used_labels,
            tasks_set_summary,
```

- [ ] **Step 6: Full backend build + test**

Run: `cd src-tauri && cargo test --lib tasks:: commands::`
Expected: PASS, no compile errors.

Run: `cd src-tauri && cargo build`
Expected: succeeds (confirms `lib.rs` registration compiles).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands/tasks.rs src-tauri/src/lib.rs
git commit -m "feat(tasks): wire label through the create/update commands, add tasks_used_labels"
```

---

## Task 6: Archived-card search matches `label`

**Files:**
- Modify: `src-tauri/src/tasks/store.rs:326-331` (`ARCHIVED_WHERE`)
- Test: `src-tauri/src/tasks/store.rs`, `mod archive_tests` (next to `search_matches_title_body_or_folder`, line ~735)

- [ ] **Step 1: Write the failing test**

Add next to `search_matches_title_body_or_folder`:

```rust
#[tokio::test]
async fn search_matches_label() {
    let pool = mem_pool().await;
    let labeled = create_task(&pool, "修登入 bug", "", "/r", true, false).await.unwrap();
    set_label(&pool, &labeled, Some("緊急")).await.unwrap();
    move_task(&pool, &labeled, STATUS_QUEUED, 1.0).await.unwrap();
    dispatch_for_test(&pool, &labeled, "tab").await;
    finish_task(&pool, &labeled, "success", None, None).await.unwrap();
    archive_task(&pool, &labeled).await.unwrap();

    let unlabeled = create_task(&pool, "沒有分類的卡", "", "/r", true, false).await.unwrap();
    move_task(&pool, &unlabeled, STATUS_QUEUED, 1.0).await.unwrap();
    dispatch_for_test(&pool, &unlabeled, "tab").await;
    finish_task(&pool, &unlabeled, "success", None, None).await.unwrap();
    archive_task(&pool, &unlabeled).await.unwrap();

    let titles = |rows: Vec<TaskRow>| -> Vec<String> { rows.into_iter().map(|r| r.title).collect() };
    assert_eq!(titles(search_archived(&pool, "緊急", 50, 0).await.unwrap()), vec!["修登入 bug"]);
    // NULL label 不該被任何關鍵字誤配到。
    assert!(!titles(search_archived(&pool, "緊急", 50, 0).await.unwrap()).contains(&"沒有分類的卡".to_string()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test search_matches_label`
Expected: FAIL — "修登入 bug" not found (label isn't in the WHERE clause yet)

- [ ] **Step 3: Implement**

`store.rs:326-331`:

```rust
const ARCHIVED_WHERE: &str = "archived_at IS NOT NULL AND (
        ?1 = ''
        OR title       LIKE ?2 ESCAPE '\\'
        OR body        LIKE ?2 ESCAPE '\\'
        OR project_dir LIKE ?2 ESCAPE '\\'
        OR label       LIKE ?2 ESCAPE '\\'
    )";
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test search_matches_label search_matches_title_body_or_folder search_treats_like_wildcards_as_literal_text`
Expected: all PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/store.rs
git commit -m "feat(tasks): archived-card search matches label"
```

---

## Task 7: Full backend regression check

**Files:** none (verification only)

- [ ] **Step 1: Run the full Rust suite**

Run: `cd src-tauri && cargo test`
Expected: PASS. This is the full suite, not `--lib` — this repo's own convention (`feedback_cargo_test_not_lib` lesson) is that `--lib` skips `tests/` integration tests entirely, so a full green `--lib` run is not sufficient signal.

If anything outside `tasks`/`commands` fails, stop and investigate before continuing — Tasks 1-6 should not have touched any other module.

- [ ] **Step 2: No commit needed** (verification-only task)

---

## Task 8: `labelColor.ts` — hash-to-hue helper

**Files:**
- Create: `src/components/TaskBoard/labelColor.ts`
- Test: `src/components/TaskBoard/labelColor.test.ts`

- [ ] **Step 1: Write the failing test**

```ts
import { describe, expect, it } from "vitest";
import { hashLabelHue } from "./labelColor";

describe("hashLabelHue", () => {
  it("回傳 0-359 之間的整數", () => {
    const hue = hashLabelHue("緊急");
    expect(Number.isInteger(hue)).toBe(true);
    expect(hue).toBeGreaterThanOrEqual(0);
    expect(hue).toBeLessThan(360);
  });

  it("同一個字串永遠得到同一個色相", () => {
    expect(hashLabelHue("文件")).toBe(hashLabelHue("文件"));
  });

  it("不同字串通常得到不同色相", () => {
    expect(hashLabelHue("緊急")).not.toBe(hashLabelHue("文件"));
  });

  it("空字串也有一個穩定的結果，不會噴錯", () => {
    expect(() => hashLabelHue("")).not.toThrow();
    expect(hashLabelHue("")).toBe(hashLabelHue(""));
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- labelColor`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement**

```ts
/** 把任意字串雜湊成一個穩定的 0–359 色相值，同一個字串永遠同色。 */
export function hashLabelHue(label: string): number {
  let h = 0;
  for (let i = 0; i < label.length; i++) {
    h = (h * 31 + label.charCodeAt(i)) | 0;
  }
  return Math.abs(h) % 360;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- labelColor`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/labelColor.ts src/components/TaskBoard/labelColor.test.ts
git commit -m "feat(taskboard): add hashLabelHue color helper"
```

---

## Task 9: `groupByLabel.ts` — pure grouping logic

**Files:**
- Create: `src/components/TaskBoard/groupByLabel.ts`
- Test: `src/components/TaskBoard/groupByLabel.test.ts`

- [ ] **Step 1: Write the failing tests**

```ts
import { describe, expect, it } from "vitest";
import { groupByLabel } from "./groupByLabel";
import type { TaskWithAttachments } from "../../ipc/tasks";

const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "c1", title: "Card", body: "", project_dir: "/r", status: "planning",
  parallel_ok: true, interactive: false, sort_order: 1, outcome: null, tab_id: null,
  transcript_path: null, error_message: null, created_at: "2026-01-01 00:00:00",
  dispatched_at: null, finished_at: null, ai_summary: null, archived_at: null,
  session_id: null, session_path: null, use_bridge: false, bridge_tiers: null,
  label: null, attachments: [],
  ...over,
});

describe("groupByLabel", () => {
  it("沒有 label 的卡片全部進 ungrouped，順序不變", () => {
    const cards = [card({ id: "a" }), card({ id: "b" })];
    const { ungrouped, groups } = groupByLabel(cards);
    expect(ungrouped.map((c) => c.id)).toEqual(["a", "b"]);
    expect(groups).toEqual([]);
  });

  it("依 label 分組，group 內順序沿用輸入順序", () => {
    const cards = [
      card({ id: "a", label: "緊急", created_at: "2026-01-01 00:00:00" }),
      card({ id: "b", label: "文件", created_at: "2026-01-02 00:00:00" }),
      card({ id: "c", label: "緊急", created_at: "2026-01-03 00:00:00" }),
    ];
    const { ungrouped, groups } = groupByLabel(cards);
    expect(ungrouped).toEqual([]);
    expect(groups.map((g) => g.label)).toEqual(["緊急", "文件"]);
    expect(groups[0].cards.map((c) => c.id)).toEqual(["a", "c"]);
  });

  it("群組順序依該 label 最早出現的 created_at，不是依數量或字母", () => {
    const cards = [
      // "文件" 只有一張，但比 "緊急" 早出現，應該排在前面。
      card({ id: "a", label: "文件", created_at: "2026-01-01 00:00:00" }),
      card({ id: "b", label: "緊急", created_at: "2026-01-02 00:00:00" }),
      card({ id: "c", label: "緊急", created_at: "2026-01-03 00:00:00" }),
    ];
    const { groups } = groupByLabel(cards);
    expect(groups.map((g) => g.label)).toEqual(["文件", "緊急"]);
  });

  it("空白字串跟只有空白的 label 都當作未分類", () => {
    const cards = [card({ id: "a", label: "" }), card({ id: "b", label: "   " })];
    const { ungrouped, groups } = groupByLabel(cards);
    expect(ungrouped.map((c) => c.id)).toEqual(["a", "b"]);
    expect(groups).toEqual([]);
  });

  it("label 前後空白會被 trim 之後當同一組", () => {
    const cards = [card({ id: "a", label: "緊急" }), card({ id: "b", label: " 緊急 " })];
    const { groups } = groupByLabel(cards);
    expect(groups).toHaveLength(1);
    expect(groups[0].cards.map((c) => c.id)).toEqual(["a", "b"]);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- groupByLabel`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement**

```ts
import type { TaskWithAttachments } from "../../ipc/tasks";

export interface LabelGroup {
  label: string;
  cards: TaskWithAttachments[];
}

/**
 * 把一欄卡片切成「未分類」跟「依 label 分組」兩塊。群組內卡片順序沿用
 * 輸入順序，不重新排序；群組彼此的順序依該 label 在輸入陣列中第一次
 * 出現的 `created_at`，由舊到新——順序穩定，不會因為卡片增減而跳動。
 */
export function groupByLabel(cards: TaskWithAttachments[]): {
  ungrouped: TaskWithAttachments[];
  groups: LabelGroup[];
} {
  const ungrouped: TaskWithAttachments[] = [];
  const byLabel = new Map<string, TaskWithAttachments[]>();
  for (const c of cards) {
    const label = c.label?.trim();
    if (!label) {
      ungrouped.push(c);
      continue;
    }
    const arr = byLabel.get(label) ?? [];
    arr.push(c);
    byLabel.set(label, arr);
  }
  const groups = [...byLabel.entries()]
    // created_at 是 SQLite `datetime('now')` 產生的固定寬度字串
    // （'YYYY-MM-DD HH:MM:SS'），字串比較跟時間先後完全一致——刻意不用
    // `Date.parse`：Tauri 在三個平台各自嵌入不同的 WebView 引擎，對
    // 「非 ISO 8601」日期字串的寬鬆解析行為並不保證一致，字串比較沒有
    // 這個跨平台風險。
    .map(([label, groupCards]) => ({
      label,
      cards: groupCards,
      firstSeen: groupCards.reduce(
        (min, c) => (c.created_at < min ? c.created_at : min),
        groupCards[0].created_at,
      ),
    }))
    .sort((a, b) => (a.firstSeen < b.firstSeen ? -1 : a.firstSeen > b.firstSeen ? 1 : 0))
    .map(({ label, cards: groupCards }) => ({ label, cards: groupCards }));
  return { ungrouped, groups };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- groupByLabel`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/groupByLabel.ts src/components/TaskBoard/groupByLabel.test.ts
git commit -m "feat(taskboard): add groupByLabel"
```

---

## Task 10: IPC types + `usedLabels`

**Files:**
- Modify: `src/ipc/tasks.ts`
- Modify: `src/ipc/projects.ts`

No dedicated test — these are type/thin-wrapper changes exercised by later tasks' tests (Task 12, 13, 14).

- [ ] **Step 1: Add `label` to `TaskRow`**

In `src/ipc/tasks.ts`, inside `interface TaskRow` (starts line 16), add a field. Put it near `project_dir` since it's the closest thing conceptually:

```ts
  project_dir: string;
  label: string | null;
```

- [ ] **Step 2: Add `label` to the create/update argument types**

Find the `createTask`/`updateTask` exported functions in `src/ipc/tasks.ts` (their inline arg object types, next to `use_bridge`/`bridge_tiers`) and add `label: string | null;` to both.

- [ ] **Step 3: Add `usedLabels`**

In `src/ipc/projects.ts`, right after `usedDirs` (`:50-51`):

```ts
export const usedLabels = (projectId: string): Promise<string[]> =>
  invoke("tasks_used_labels", { projectId });
```

- [ ] **Step 4: Type-check**

Run: `npx tsc -b`
Expected: FAILS at this point — every hand-built `TaskWithAttachments`/`TaskRow` literal in test files is now missing `label`. That's expected; Task 11 fixes them. Confirm the errors are exactly the 6 known sites (see Task 11) and nothing else — if `tsc` reports errors in files not listed there, stop and investigate (it means something else constructs a full `TaskRow` literal that this plan missed).

- [ ] **Step 5: Commit**

```bash
git add src/ipc/tasks.ts src/ipc/projects.ts
git commit -m "feat(ipc): add label to task types and usedLabels lookup"
```

(This commit intentionally leaves `tsc -b` red — Task 11 is the fix, in the same sitting.)

---

## Task 11: Fix hand-built `TaskWithAttachments`/`TaskRow` fixtures

**Files:**
- Modify: `src/components/TaskBoard/index.test.tsx:58-64` (`card()` helper)
- Modify: `src/components/TaskBoard/ReportDialog.test.tsx:109-117` (`taskCard()` helper)
- Modify: `src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx:39-62` (`BASE_CARD`)
- Modify: `src/components/TaskBoard/reportPrompts.test.ts:6-30` (`card()` helper)
- Modify: `src-tauri/...` — none, this task is frontend-only (Rust fixtures were fixed in Task 1 Step 4)

This is pure mechanical fixup — no new test, `tsc -b` itself is the check.

- [ ] **Step 1: `index.test.tsx`**

```ts
const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "c1", title: "Card one", body: "", project_dir: "/r", status: "planning",
  parallel_ok: true, interactive: false, sort_order: 1, outcome: null, tab_id: null,
  transcript_path: null, error_message: null, created_at: "", dispatched_at: null,
  finished_at: null, ai_summary: null, archived_at: null,
  session_id: null, session_path: null, use_bridge: false, bridge_tiers: null,
  label: null, attachments: [],
  ...over,
});
```

Also add `usedLabels` to this file's `ipc/projects` mock (`:34-36`) — `TaskEditorDialog` will import it starting in Task 13, and this file mounts `TaskEditorDialog` (via the "new card" button), so an unmocked `usedLabels` would fall through to the real `invoke` and throw outside a Tauri webview:

```ts
vi.mock("../../ipc/projects", () => ({
  usedDirs: vi.fn().mockResolvedValue([]),
  usedLabels: vi.fn().mockResolvedValue([]),
}));
```

- [ ] **Step 2: `ReportDialog.test.tsx`**

```ts
  const taskCard = (over: Partial<TaskWithAttachments> = {}): TaskWithAttachments => ({
    id: "t1", title: "卡片", body: "", project_dir: "/r", status: "done",
    parallel_ok: true, interactive: false, sort_order: 1, outcome: "success",
    tab_id: null, transcript_path: null, error_message: null,
    created_at: "2026-09-05T10:00:00Z", dispatched_at: null, finished_at: null,
    ai_summary: null, archived_at: null, session_id: null, session_path: null,
    use_bridge: false, bridge_tiers: null, label: null,
    attachments: [], ...over,
  });
```

- [ ] **Step 3: `TaskEditorDialog.bridge.test.tsx`**

Add `label: null,` right after `bridge_tiers: null,` in `BASE_CARD` (`:60`).

- [ ] **Step 4: `reportPrompts.test.ts`**

Add `label: null,` right after `bridge_tiers: null,` (`:27`).

- [ ] **Step 5: Verify**

Run: `npx tsc -b`
Expected: PASS, zero errors.

Run: `npm run test`
Expected: PASS (all existing suites still green — this task changed no behavior, only fixture shape).

- [ ] **Step 6: Commit**

```bash
git add src/components/TaskBoard/index.test.tsx src/components/TaskBoard/ReportDialog.test.tsx src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx src/components/TaskBoard/reportPrompts.test.ts
git commit -m "test(taskboard): add label field to hand-built TaskWithAttachments fixtures"
```

---

## Task 12: `TaskLabelGroup` component

**Files:**
- Create: `src/components/TaskBoard/TaskLabelGroup.tsx`
- Test: `src/components/TaskBoard/TaskLabelGroup.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { TaskLabelGroup } from "./TaskLabelGroup";

describe("TaskLabelGroup", () => {
  it("預設展開，看得到 children", () => {
    render(
      <TaskLabelGroup label="緊急" count={2}>
        <div data-testid="child">卡片</div>
      </TaskLabelGroup>,
    );
    expect(screen.getByTestId("child")).toBeInTheDocument();
    expect(screen.getByText("緊急")).toBeInTheDocument();
    expect(screen.getByText("(2)")).toBeInTheDocument();
  });

  it("點標頭切換摺疊，children 消失；再點一次恢復", async () => {
    render(
      <TaskLabelGroup label="緊急" count={1}>
        <div data-testid="child">卡片</div>
      </TaskLabelGroup>,
    );
    const header = screen.getByRole("button");
    await userEvent.click(header);
    expect(screen.queryByTestId("child")).not.toBeInTheDocument();
    await userEvent.click(header);
    expect(screen.getByTestId("child")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- TaskLabelGroup`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement**

```tsx
import { useState, type CSSProperties, type ReactNode } from "react";
import { hashLabelHue } from "./labelColor";

export function TaskLabelGroup({
  label,
  count,
  children,
}: {
  label: string;
  count: number;
  children: ReactNode;
}) {
  const [collapsed, setCollapsed] = useState(false);
  return (
    <div className="task-label-group">
      <button
        type="button"
        className="task-label-group-header"
        style={{ "--label-hue": hashLabelHue(label) } as CSSProperties}
        onClick={() => setCollapsed((c) => !c)}
      >
        <span className="task-label-group-caret">{collapsed ? "▸" : "▾"}</span>
        <span className="task-label-chip">{label}</span>
        <span className="task-label-group-count">({count})</span>
      </button>
      {!collapsed && <div className="task-label-group-body">{children}</div>}
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- TaskLabelGroup`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/TaskLabelGroup.tsx src/components/TaskBoard/TaskLabelGroup.test.tsx
git commit -m "feat(taskboard): add TaskLabelGroup collapsible component"
```

---

## Task 13: CSS for label chip + group

**Files:**
- Modify: `src/components/TaskBoard/index.css`

No test — pure styling. Verified visually in Task 17's manual check.

- [ ] **Step 1: Add styles**

Add right after the existing badge rules (`index.css:589-599`, after the `@keyframes task-badge-pulse` block):

```css
/* Label 分組 — 卡片可依使用者自訂的自由文字分類。這個看板整區是固定
   深色（見檔案開頭的說明），所以色相雜湊配色只需要一套，不用淺色/
   深色雙軌寫法。 */
.task-label-chip {
  background: hsl(var(--label-hue) 45% 22%);
  color: hsl(var(--label-hue) 70% 78%);
  border-radius: 20px;
  padding: 2px 9px;
  font-size: 10px;
  font-weight: 600;
}
.task-label-group { display: flex; flex-direction: column; gap: 8px; }
.task-label-group-header {
  display: flex;
  align-items: center;
  gap: 6px;
  background: none;
  border: none;
  padding: 2px 0;
  cursor: pointer;
  color: var(--text-muted);
}
.task-label-group-caret { font-size: 10px; width: 10px; }
.task-label-group-count { font-size: 11px; color: var(--text-muted); }
.task-label-group-body { display: flex; flex-direction: column; gap: 8px; }
```

- [ ] **Step 2: Add the badge to `TaskCard`'s import**

(No separate step — bundled into Task 14, which is the first consumer of `.task-label-chip` on a card.)

- [ ] **Step 3: Commit**

```bash
git add src/components/TaskBoard/index.css
git commit -m "style(taskboard): add label chip and group styles"
```

---

## Task 14: `TaskCard` renders the label badge

**Files:**
- Modify: `src/components/TaskBoard/TaskCard.tsx`
- Test: extend `src/components/TaskBoard/index.test.tsx` (this is where `TaskCard` rendering is already exercised via `ProjectBoard`)

- [ ] **Step 1: Write the failing test**

Add a new `describe` block to `index.test.tsx` (anywhere after the existing describes, using the file's `card()` helper and `view()`):

```tsx
describe("Label 徽章", () => {
  it("有 label 的卡片顯示徽章", async () => {
    vi.mocked(listTasks).mockResolvedValue([card({ id: "1", label: "緊急" })]);
    view();
    expect(await screen.findByText("緊急")).toBeInTheDocument();
  });

  it("沒有 label 的卡片不顯示徽章", async () => {
    vi.mocked(listTasks).mockResolvedValue([card({ id: "1", label: null })]);
    view();
    await screen.findByText("Card one");
    expect(screen.queryByText("緊急")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- index.test.tsx -t "Label 徽章"`
Expected: FAIL — no badge rendered yet

- [ ] **Step 3: Implement**

In `TaskCard.tsx`, add the import:

```ts
import { hashLabelHue } from "./labelColor";
```

Add to the existing `task-card-badges` block (`:86-99`), as the last badge:

```tsx
      <div className="task-card-badges">
        {card.status === "running" && (
          <span className="task-badge task-badge--running">
            <span className="task-badge-dot" />
            {t.board_col_running}
          </span>
        )}
        {card.interactive && (
          <span className="task-badge task-badge--interactive">{t.board_badge_interactive}</span>
        )}
        {card.status === "done" && card.outcome && (
          <span className={`task-badge task-badge--${card.outcome}`}>{outcomeLabel}</span>
        )}
        {card.label && (
          <span
            className="task-label-chip"
            style={{ "--label-hue": hashLabelHue(card.label) } as CSSProperties}
          >
            {card.label}
          </span>
        )}
      </div>
```

Add `CSSProperties` to the existing `import { useState } from "react";` line at the top:

```ts
import { useState, type CSSProperties } from "react";
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- index.test.tsx -t "Label 徽章"`
Expected: PASS

Run: `npm run test -- index.test.tsx`
Expected: full file still PASS (no regression in the other describes)

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/TaskCard.tsx src/components/TaskBoard/index.test.tsx
git commit -m "feat(taskboard): TaskCard renders a label badge"
```

---

## Task 15: `TaskEditorDialog` — label input + used-label chips + save wiring

**Files:**
- Modify: `src/components/TaskBoard/TaskEditorDialog.tsx`
- Test: `src/components/TaskBoard/TaskEditorDialog.usedLabels.test.tsx` (new file, mirrors `TaskEditorDialog.usedDirs.test.tsx`)

- [ ] **Step 1: Write the failing tests**

Create `src/components/TaskBoard/TaskEditorDialog.usedLabels.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedDirs = vi.fn();
const usedLabels = vi.fn();
vi.mock("../../ipc/projects", () => ({
  usedDirs: (...a: unknown[]) => usedDirs(...a),
  usedLabels: (...a: unknown[]) => usedLabels(...a),
}));
const createTask = vi.fn().mockResolvedValue("new-id");
const updateTask = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tasks", () => ({
  createTask: (...a: unknown[]) => createTask(...a),
  updateTask: (...a: unknown[]) => updateTask(...a),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskEditorDialog } from "./TaskEditorDialog";

const mount = () =>
  render(
    <LocaleProvider>
      <TaskEditorDialog projectId="p1" card={null} onClose={vi.fn()} onSaved={vi.fn()} />
    </LocaleProvider>,
  );

describe("TaskEditorDialog Label 欄位", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    usedDirs.mockResolvedValue([]);
    localStorage.clear();
  });

  it("列出這個專案用過的 Label", async () => {
    usedLabels.mockResolvedValue(["緊急", "文件"]);
    mount();
    expect(await screen.findByTestId("used-label-緊急")).toBeInTheDocument();
    expect(screen.getByTestId("used-label-文件")).toBeInTheDocument();
    expect(usedLabels).toHaveBeenCalledWith("p1");
  });

  it("點快捷選項會填入 Label 欄", async () => {
    usedLabels.mockResolvedValue(["緊急"]);
    mount();
    await userEvent.click(await screen.findByTestId("used-label-緊急"));
    expect(screen.getByTestId("task-label-input")).toHaveValue("緊急");
  });

  it("沒有用過的 Label 時不顯示這一區", async () => {
    usedLabels.mockResolvedValue([]);
    mount();
    await screen.findByTestId("task-label-input");
    expect(screen.queryByTestId("used-labels-row")).not.toBeInTheDocument();
  });

  it("儲存新卡片時，Label 有打字就照原樣送出（trim 過）", async () => {
    usedLabels.mockResolvedValue([]);
    mount();
    await userEvent.type(screen.getByTestId("task-title-input"), "標題");
    await userEvent.type(screen.getByTestId("task-dir-input"), "/repo");
    await userEvent.type(screen.getByTestId("task-label-input"), "  緊急  ");
    await userEvent.click(screen.getByRole("button", { name: /儲存|Save/ }));
    expect(createTask).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({ label: "緊急" }),
    );
  });

  it("儲存新卡片時，Label 是空白就送 null", async () => {
    usedLabels.mockResolvedValue([]);
    mount();
    await userEvent.type(screen.getByTestId("task-title-input"), "標題");
    await userEvent.type(screen.getByTestId("task-dir-input"), "/repo");
    await userEvent.click(screen.getByRole("button", { name: /儲存|Save/ }));
    expect(createTask).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({ label: null }),
    );
  });
});
```

(The save button has no `data-testid` — it's `<button className="aiterm-btn aiterm-btn--primary" disabled={...} onClick={() => void save()}>{t.board_save}</button>` at `TaskEditorDialog.tsx:445-451`, text "儲存"/"Save" per `i18n.ts:53`/`:1558`, hence the regex above.)

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- TaskEditorDialog.usedLabels`
Expected: FAIL — no label input exists yet

- [ ] **Step 3: Implement**

Add state near `dir`/`dirChoices` (`:44`, `:86-95`):

```ts
  const [label, setLabel] = useState(card?.label ?? "");
  const [labelChoices, setLabelChoices] = useState<string[]>([]);
  useEffect(() => {
    let alive = true;
    void usedLabels(projectId).then((labels) => {
      if (alive) setLabelChoices(labels);
    });
    return () => {
      alive = false;
    };
  }, [projectId]);
```

Import `usedLabels` alongside `usedDirs`:

```ts
import { usedDirs, usedLabels } from "../../ipc/projects";
```

Add the field to the JSX, right after the `dir` field's closing `</label>` and its `</div>` (`:343-345`, the `task-dialog-group` that currently only contains the folder field):

```tsx
        <div className="task-dialog-group">
        <label className="task-field">
          <span className="task-field-label">{t.board_card_label}</span>
          <input
            className="task-field-input"
            data-testid="task-label-input"
            placeholder={t.board_card_label_placeholder}
            value={label}
            onChange={(e) => setLabel(e.target.value)}
          />
          {labelChoices.length > 0 && (
            <div className="task-used-dirs" data-testid="used-labels-row">
              {labelChoices.map((l) => (
                <button
                  key={l}
                  type="button"
                  className="tb-btn tb-btn--ghost tb-btn--tiny"
                  data-testid={`used-label-${l}`}
                  onClick={() => setLabel(l)}
                >
                  🏷 {l}
                </button>
              ))}
            </div>
          )}
        </label>
        </div>
```

(This is a new, separate `task-dialog-group` right after the folder one, per the spec's placement decision — folder group, then label group, then the interactive/parallel checkboxes group.)

Update `save()` (`:185-208`) to include `label`:

```ts
      const labelArg = label.trim() || null;
      if (isEdit) {
        await updateTask(projectId, {
          id: card.id,
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
          label: labelArg,
          ...bridgeArgs,
        });
      } else {
        const newId = await createTask(projectId, {
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
          label: labelArg,
          ...bridgeArgs,
        });
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- TaskEditorDialog.usedLabels`
Expected: PASS

Run: `npm run test -- TaskEditorDialog`
Expected: all `TaskEditorDialog.*` test files still PASS

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/TaskEditorDialog.tsx src/components/TaskBoard/TaskEditorDialog.usedLabels.test.tsx
git commit -m "feat(taskboard): TaskEditorDialog gets a label field with used-label chips"
```

---

## Task 16: i18n strings

**Files:**
- Modify: `src/lib/i18n.ts`

No test — string tables. Verified by Task 15's tests already passing (they render `t.board_card_label*`), so this task must land before Task 15's tests can pass; do this task first if executing strictly in file order, or fold it into Task 15's Step 3. Listed separately here for clarity of what changed.

- [ ] **Step 1: zh-TW block**

`i18n.ts:39-40`, add after `board_card_folder_pick`:

```ts
    board_card_folder_pick: "選擇資料夾…",
    board_card_label: "Label",
    board_card_label_placeholder: "例如：緊急、文件",
```

- [ ] **Step 2: en block**

`i18n.ts:1544-1545`, add after `board_card_folder_pick`:

```ts
    board_card_folder_pick: "Choose folder…",
    board_card_label: "Label",
    board_card_label_placeholder: "e.g. urgent, docs",
```

- [ ] **Step 3: Verify**

Run: `npx tsc -b`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(i18n): add board_card_label strings"
```

**Note on ordering:** if executing tasks strictly in numeric order, do this task (16) immediately before Task 15's Step 4 (run tests), not after — otherwise Task 15's tests fail on missing translation keys rendering as `undefined`. The two tasks are independent in content but coupled in test-passing order.

---

## Task 17: `ProjectBoard` — integrate grouping + extend search

**Files:**
- Modify: `src/components/TaskBoard/ProjectBoard.tsx`
- Test: extend `src/components/TaskBoard/index.test.tsx`

- [ ] **Step 1: Write the failing tests**

Add a new `describe` block to `index.test.tsx`:

```tsx
describe("Label 分組", () => {
  it("同一個 Label 的卡片收進同一個可摺疊群組，未分類卡片留在最上面", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "1", title: "沒分類", label: null, sort_order: 1 }),
      card({ id: "2", title: "緊急一", label: "緊急", sort_order: 2, created_at: "2026-01-01 00:00:00" }),
      card({ id: "3", title: "緊急二", label: "緊急", sort_order: 3, created_at: "2026-01-02 00:00:00" }),
    ]);
    view();

    await screen.findByText("沒分類");
    // 群組標頭上看得到 label 文字跟數量。
    expect(screen.getByText("緊急")).toBeInTheDocument();
    expect(screen.getByText("(2)")).toBeInTheDocument();
    expect(screen.getByText("緊急一")).toBeInTheDocument();
    expect(screen.getByText("緊急二")).toBeInTheDocument();
  });

  it("搜尋關鍵字能比對 label", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "1", title: "不相干的卡", label: null }),
      card({ id: "2", title: "有分類的卡", label: "緊急" }),
    ]);
    view();
    await screen.findByText("不相干的卡");

    await userEvent.type(screen.getByTestId("board-search"), "緊急");
    expect(screen.getByText("有分類的卡")).toBeInTheDocument();
    expect(screen.queryByText("不相干的卡")).not.toBeInTheDocument();
  });
});
```

(`userEvent` and `screen` are already imported at the top of `index.test.tsx`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- index.test.tsx -t "Label 分組"`
Expected: FAIL — cards render as a flat list, no group headers, search doesn't match label yet

- [ ] **Step 3: Implement**

Add the import:

```ts
import { groupByLabel } from "./groupByLabel";
import { TaskLabelGroup } from "./TaskLabelGroup";
```

Extend `visibleIn`'s filter (`:131-137`):

```ts
  const visibleIn = (s: TaskStatus) => {
    const q = search.trim().toLowerCase();
    if (!q) return byStatus(s);
    return byStatus(s).filter((c) =>
      [c.title, c.body, c.project_dir, c.label ?? ""].some((f) => f.toLowerCase().includes(q)),
    );
  };
```

Extract the existing per-card render block into a `renderCard` function. Currently (`:294-319`) it's inline inside `visibleIn(s).map((cardRow) => { ... })`. Change the `TaskColumn` children (`:293-320`) from:

```tsx
          >
            {visibleIn(s).map((cardRow) => {
              const draggableCard = ...
              ...
              return (
                <div ...>
                  <TaskCard ... />
                </div>
              );
            })}
          </TaskColumn>
```

to:

```tsx
          >
            {(() => {
              const renderCard = (cardRow: TaskWithAttachments) => {
                const draggableCard =
                  cardRow.status === "planning" ||
                  cardRow.status === "queued" ||
                  (cardRow.status === "running" && cardRow.interactive);
                const isDragging = draggingCardId === cardRow.id;
                const classes = ["task-card-drag-wrap"];
                if (draggableCard) classes.push("task-card-drag-wrap--draggable");
                if (isDragging) classes.push("task-card-drag-wrap--dragging");
                return (
                  <div
                    key={cardRow.id}
                    data-task-drag-id={cardRow.id}
                    className={classes.join(" ")}
                    onMouseDown={(e) => handleCardMouseDown(e, cardRow)}
                  >
                    <TaskCard
                      projectId={projectId}
                      card={cardRow}
                      onEdit={() => setEditing(cardRow)}
                      onViewTranscript={() => setTranscriptFor(cardRow.id)}
                      onChanged={() => void refresh()}
                    />
                  </div>
                );
              };
              const { ungrouped, groups } = groupByLabel(visibleIn(s));
              return (
                <>
                  {ungrouped.map(renderCard)}
                  {groups.map((g) => (
                    <TaskLabelGroup key={g.label} label={g.label} count={g.cards.length}>
                      {g.cards.map(renderCard)}
                    </TaskLabelGroup>
                  ))}
                </>
              );
            })()}
          </TaskColumn>
```

(The `key={cardRow.id}` prop moves with the `div` unchanged — it was already there.)

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- index.test.tsx -t "Label 分組"`
Expected: PASS

Run: `npm run test -- index.test.tsx`
Expected: entire file still PASS — this step touches the core render path and drag setup, so watch specifically for the existing drag/drop tests and the "封存" (archive) column tests, which exercise `visibleIn`/rendering most directly.

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/ProjectBoard.tsx src/components/TaskBoard/index.test.tsx
git commit -m "feat(taskboard): ProjectBoard groups cards by label within each column"
```

---

## Task 18: Full-repo verification

**Files:** none (verification only)

- [ ] **Step 1: Frontend type check**

Run: `npx tsc -b`
Expected: PASS, zero errors

- [ ] **Step 2: Frontend lint**

Run: `npm run lint`
Expected: PASS

- [ ] **Step 3: Frontend test suite**

Run: `npm run test`
Expected: PASS, full suite green

- [ ] **Step 4: Backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS, full suite green (not `--lib` — see Task 7's note)

- [ ] **Step 5: Manual smoke check**

Run: `npm run tauri:dev`, open the task board, create two cards with the same Label, confirm:
- They collapse into one named, colored group with a count
- Clicking the group header collapses/expands it
- A third card with no Label stays outside any group, above the groups
- Typing the Label text into the board search box filters correctly
- Dragging a labeled card from planning to queued keeps its Label and group membership

This step has no automated assertion — it is the "did it actually work in the real app" check called for by this project's own verification-before-completion convention, since Vitest/RTL prove render logic but not real dblclick/drag feel or visual legibility of the color badges against the fixed dark palette.

- [ ] **Step 6: No commit** (verification-only task; if the manual check in Step 5 finds a bug, fix it as a new small commit and re-run Steps 1-4)

---

## Task 19: Finish the branch

**Files:** none

- [ ] **Step 1: Review the full diff against the design spec**

Run: `git log --oneline master..feature/task-label-grouping` and `git diff master...feature/task-label-grouping --stat` — confirm every file in the spec's "後端改動"/"前端改動" sections was touched, and nothing unrelated was.

- [ ] **Step 2: Invoke `superpowers:finishing-a-development-branch`**

This decides merge/PR/cleanup — do not merge or push unilaterally without going through it, per this repo's standing git-safety rules (no unrequested push/merge to `master`).
