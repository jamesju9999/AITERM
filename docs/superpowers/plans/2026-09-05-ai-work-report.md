# AI 工作報告 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 使用者選一個專案，AI 把該專案看板中的所有工作項目整理成一份 HTML 工作報告，存進專案資料夾累積成歷史。

**Architecture:** 兩階段。第一階段把每張已完成卡片的對話記錄各自摘要成 300 字內的文字，寫回 `tasks.db` 的新欄位 `ai_summary` 當快取（已完成的卡片不可變，摘要只需算一次）。第二階段把全部卡片的欄位加上那些摘要，用既有的 `artifact-html` 協定請 AI 產出一份 HTML 文件，用 `splitArtifactFence` 抽出後存進 `<專案>/reports/`。呈現用既有的 `ArtifactHtmlFrame`（sandbox iframe）。

**Tech Stack:** Rust / Tauri 2 / sqlx（SQLite）；React 19 / TypeScript / Vitest / React Testing Library。

**Spec:** `docs/superpowers/specs/2026-09-05-ai-work-report-design.md`

---

## 實作前必讀

### 這份計畫依據的地面實情（都已讀過原始碼確認）

1. **`splitArtifactFence(text)`**（`src/lib/artifactFence.ts`）回傳 `{ prose: string, artifact: { kind, content } | null }`。`kind` 是 `"html" | "chart"`。找不到 fence 時 `artifact` 為 `null`。它自己會把 CRLF 正規化成 LF——模型在 Windows 上可能吐 CRLF，不正規化的話 fence 那行比對不到。

2. **`ArtifactHtmlFrame`**（`src/components/ArtifactPanel/ArtifactHtmlFrame.tsx`）的 props 就兩個：`{ html: string; title: string }`。它是獨立元件，**不需要 `ArtifactPanelProvider`**。它用 `sandbox="allow-scripts"` 且**刻意不給** `allow-same-origin`——不要為了任何理由加上去。

3. **`invokeAiChat(messages, sessionId, providerId?, useMcp, locale, supportsArtifacts)`**（`src/ipc/ai.ts`）。第六個參數 `supportsArtifacts` 傳 `true` 時，後端才會把 artifact 協定的說明接進系統提示（見 `src-tauri/src/ai/artifact_prompt.rs`）。回傳 `AiChatReply { content, tool_calls, ... }`。

4. **AI 錯誤的判別**：看 `err.kind`，`"not_configured"` 代表沒設定（既有寫法見 `src/lib/agentLoop.ts:135`）。

   **但 `normalizeAiError` 不是共用函式**——寫計畫時我以為它從 `src/ipc/ai.ts` 匯出，實際上它在 `CrossDbView/CrossDbAiChat.tsx:118` 與 `DatabaseView/DatabaseAiChat.tsx:112` 各被複製了一份，兩份內容相同。Task 6 因此**自己寫一個小的判別函式**，不 import 也不去動那兩個既有的複本（重構它們超出這個里程碑的範圍；順手改會把不相干的檔案捲進來）。

5. **`TaskRow`**（`src-tauri/src/tasks/store.rs`）目前的欄位：`id, title, body, project_dir, status, parallel_ok, interactive, sort_order, outcome, tab_id, transcript_path, error_message, created_at, dispatched_at, finished_at`。本計畫再加一個 `ai_summary: Option<String>`。

6. **`mark_dispatched`** 目前是 `UPDATE tasks SET status='running', tab_id=?, dispatched_at=? WHERE id=? AND status='queued'`。本計畫要在同一句裡加上 `ai_summary = NULL`。

7. **既有的欄位遷移寫法**（`src-tauri/src/tasks/mod.rs` 的 `init_schema`）：
   ```rust
   let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN interactive INTEGER NOT NULL DEFAULT 0")
       .execute(pool).await;
   ```
   刻意丟掉錯誤——欄位已存在時 `ALTER TABLE` 會失敗，那是正常的。

8. **`ProjectHandle`**（`src-tauri/src/projects/mod.rs`）有 `{ id, name, path: PathBuf, pool: SqlitePool }`，`ProjectRegistry::get(id)` 取出。`commands/tasks.rs` 有一個 `project(&reg, &project_id)` 輔助函式做這件事。

### 這個專案的重要規矩

- **型別檢查用 `npx tsc -b`**，不可用 `tsc --noEmit`——根 `tsconfig.json` 是 solution file（`"files": []`），`--noEmit` 什麼都不檢查而且永遠 exit 0。
- **Rust 一定要跑完整的 `cargo test`**，不可只跑 `--lib`——`--lib` 不編譯 `tests/` 底下的整合測試。
- **不要用 Python 腳本改檔**，這個 repo 有些檔案是 CRLF 行尾。改完檢查 `git diff --stat`。
- 只 `git add` 明確路徑，不要 `git add -A`。
- **跨平台是硬需求**（macOS / Windows / Linux）。路徑一律用 `PathBuf`。**Windows 不允許刪除還有檔案被開著的目錄**——測試裡要刪專案資料夾時，必須先 `pool.close().await`。上一個里程碑實際踩過。
- **原生對話框不可用 `window.confirm`**，要用 `@tauri-apps/plugin-dialog` 的非同步版本。
- i18n 兩個語系（`zhTW` / `enRaw`）都要加，只加一邊會造成語系漂移而且測試抓不到。
- **`src/components/TaskBoard/index.css` 的註解裡絕對不能出現星號緊接斜線的組合**——會提前關閉註解並把緊接著的整條規則吃掉。

### 檔案結構

**新增**

| 檔案 | 職責 |
|---|---|
| `src-tauri/src/commands/reports.rs` | `reports_save` / `reports_list` / `reports_read`。檔案 I/O 留在 command 層，與 `tasks_save_transcript` 同慣例 |
| `src/ipc/reports.ts` | `reports_*` 的 IPC 包裝與型別 |
| `src/components/TaskBoard/reportPrompts.ts` | 兩階段、兩風格的提示詞。純字串組裝、無 I/O |
| `src/components/TaskBoard/useWorkReport.ts` | 產生流程的協調 |
| `src/components/TaskBoard/ReportDialog.tsx` | 報告視窗（`ArtifactHtmlFrame` + 歷史清單） |

**修改**

| 檔案 | 改什麼 |
|---|---|
| `src-tauri/src/tasks/mod.rs` | `init_schema` 加 `ai_summary` 欄位的遷移 |
| `src-tauri/src/tasks/store.rs` | `TaskRow` 加 `ai_summary`；新增 `set_summary`；`mark_dispatched` 清掉 `ai_summary` |
| `src-tauri/src/commands/tasks.rs` | 新增 `tasks_set_summary` |
| `src-tauri/src/commands/mod.rs` | 加 `pub mod reports;` |
| `src-tauri/src/lib.rs` | 註冊 4 個新指令 |
| `src/ipc/tasks.ts` | `TaskRow` 型別加 `ai_summary`；新增 `setSummary` |
| `src/components/TaskBoard/ProjectList.tsx` | 專案卡片加「報告」按鈕 |
| `src/components/TaskBoard/ProjectBoard.tsx` | 工具列加「產生工作報告」按鈕 |
| `src/components/TaskBoard/index.css` | 報告視窗與歷史清單的樣式 |
| `src/lib/i18n.ts` | 新字串（兩個語系） |

---

## Task 1: `ai_summary` 欄位與 store 層

**Files:**
- Modify: `src-tauri/src/tasks/mod.rs`（`init_schema`）
- Modify: `src-tauri/src/tasks/store.rs`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/tasks/store.rs` 末端新增：

```rust
#[cfg(test)]
mod summary_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn a_new_task_has_no_summary() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().ai_summary, None);
    }

    #[tokio::test]
    async fn set_summary_round_trips() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        set_summary(&pool, &id, "做了 A 和 B，結果成功").await.unwrap();
        assert_eq!(
            get_task(&pool, &id).await.unwrap().unwrap().ai_summary.as_deref(),
            Some("做了 A 和 B，結果成功")
        );
    }

    /// 卡片重新派工時，舊摘要描述的是**上一次**的執行——留著會讓報告
    /// 講錯，而且是那種「看起來很合理但其實是錯的」的錯。
    #[tokio::test]
    async fn re_dispatching_clears_a_stale_summary() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        mark_dispatched(&pool, &id, "tab-1").await.unwrap();
        finish_task(&pool, &id, "success", None, None).await.unwrap();
        set_summary(&pool, &id, "第一次執行的摘要").await.unwrap();

        // 拖回規劃、再排一次、再派工
        move_task(&pool, &id, STATUS_PLANNING, 1.0).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        mark_dispatched(&pool, &id, "tab-2").await.unwrap();

        assert_eq!(
            get_task(&pool, &id).await.unwrap().unwrap().ai_summary,
            None,
            "重新派工後不可留著上一次執行的摘要"
        );
    }

    #[tokio::test]
    async fn a_cloned_task_does_not_inherit_the_summary() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        set_summary(&pool, &id, "原卡片的摘要").await.unwrap();
        let clone_id = clone_task_fields(&pool, &id).await.unwrap();
        assert_eq!(get_task(&pool, &clone_id).await.unwrap().unwrap().ai_summary, None);
    }
}
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `cd src-tauri && cargo test --lib store::summary_tests`
Expected: FAIL，編譯錯誤 `no field ai_summary on type TaskRow` 與 `cannot find function set_summary in this scope`。

- [ ] **Step 3: 寫最小實作**

`src-tauri/src/tasks/mod.rs` 的 `init_schema`，在既有的 `interactive` 遷移那行之後加入：

```rust
    // Migration: existing databases created before `ai_summary` existed.
    // 跟上面的 `interactive` 同一個寫法——欄位已存在時 ALTER TABLE 會
    // 失敗，那是正常的，所以刻意丟掉錯誤。
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN ai_summary TEXT")
        .execute(pool)
        .await;
```

同時在 `CREATE TABLE IF NOT EXISTS tasks (...)` 的欄位清單中，`finished_at` 那行之後加入：

```sql
            ai_summary      TEXT
```

`src-tauri/src/tasks/store.rs` 的 `TaskRow`，在 `finished_at` 之後加入欄位：

```rust
    /// 這張卡片的 AI 履行摘要（工作報告用）。只有 `done` 的卡片會有，
    /// 由前端在產生報告時補上。已完成的卡片不可變，所以這是永久快取；
    /// 重新派工時 `mark_dispatched` 會清掉它。
    pub ai_summary: Option<String>,
```

把 `mark_dispatched` 改為：

```rust
pub async fn mark_dispatched(pool: &SqlitePool, id: &str, tab_id: &str) -> Result<(), sqlx::Error> {
    // 一併清掉 ai_summary：這張卡要重跑了，舊摘要描述的是上一次的執行，
    // 留著會讓工作報告講錯。
    sqlx::query(
        "UPDATE tasks SET status = 'running', tab_id = ?, dispatched_at = ?, ai_summary = NULL
         WHERE id = ? AND status = 'queued'",
    )
    .bind(tab_id)
    .bind(now_secs())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
```

在 `set_interactive` 之後加入：

```rust
/// 寫入這張卡片的 AI 履行摘要（工作報告的第一階段產物）。
pub async fn set_summary(pool: &SqlitePool, id: &str, summary: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET ai_summary = ? WHERE id = ?")
        .bind(summary)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

`clone_task_fields` **不用改**——它是呼叫 `create_task` 建一張全新的卡片，本來就不會帶 `ai_summary`。上面那個測試是把這件事釘住，不是要你改它。

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib store::summary_tests`
Expected: PASS，4 個測試全過。

- [ ] **Step 5: 跑完整測試**

Run: `cd src-tauri && cargo test`
Expected: 全綠。`TaskRow` 加欄位會讓 `SELECT *` 多回一欄，既有測試不受影響。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/tasks/mod.rs src-tauri/src/tasks/store.rs
git commit -m "feat(tasks): ai_summary 欄位與快取失效"
```

---

## Task 2: `reports_*` 指令

**Files:**
- Create: `src-tauri/src/commands/reports.rs`
- Modify: `src-tauri/src/commands/mod.rs`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/commands/reports.rs` 寫入：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_report_name_accepts_a_plain_filename() {
        assert!(safe_report_name("2026-09-05-1430.html").is_ok());
        assert!(safe_report_name("2026-09-05-1430-2.html").is_ok());
    }

    /// 前端只會傳 `reports_list` 給過的檔名，但這個指令不能假設呼叫端
    /// 守規矩——路徑穿越必須在這裡就被擋掉。
    #[test]
    fn safe_report_name_rejects_path_traversal() {
        for bad in [
            "../secrets.txt",
            "../../etc/passwd",
            "a/b.html",
            "a\\b.html",
            "/absolute.html",
            "..",
            "",
        ] {
            assert!(safe_report_name(bad).is_err(), "應該被拒絕：{bad}");
        }
    }

    #[test]
    fn report_filename_uses_the_timestamp() {
        let name = report_filename(&[], "2026-09-05-1430");
        assert_eq!(name, "2026-09-05-1430.html");
    }

    /// 同一分鐘內產第二份不可以覆蓋第一份。
    #[test]
    fn report_filename_avoids_collisions_within_the_same_minute() {
        let existing = vec!["2026-09-05-1430.html".to_string()];
        assert_eq!(report_filename(&existing, "2026-09-05-1430"), "2026-09-05-1430-2.html");

        let existing = vec![
            "2026-09-05-1430.html".to_string(),
            "2026-09-05-1430-2.html".to_string(),
        ];
        assert_eq!(report_filename(&existing, "2026-09-05-1430"), "2026-09-05-1430-3.html");
    }

    #[test]
    fn title_from_html_reads_the_title_tag() {
        assert_eq!(
            title_from_html("<html><head><title>第三季進度</title></head><body></body></html>"),
            Some("第三季進度".to_string())
        );
    }

    #[test]
    fn title_from_html_is_none_when_there_is_no_title() {
        assert_eq!(title_from_html("<html><body>hi</body></html>"), None);
    }
}
```

- [ ] **Step 2: 執行測試，確認它失敗**

先在 `src-tauri/src/commands/mod.rs` 的 `pub mod projects;` 之後加入：

```rust
pub mod reports;
```

Run: `cd src-tauri && cargo test --lib reports::tests`
Expected: FAIL，`cannot find function safe_report_name in this scope` 等。

- [ ] **Step 3: 寫最小實作**

在 `src-tauri/src/commands/reports.rs` 的測試模組之前寫入：

```rust
//! 工作報告的存取。報告是 AI 產生的 HTML 文件，存在
//! `<專案>/reports/` 底下累積成歷史——專案資料夾自成一體，所以報告
//! 會跟著專案走。
//!
//! 檔案 I/O 留在 command 層、不下放 `tasks::store`，與
//! `tasks_save_transcript` 同一慣例。

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::projects::{ProjectHandle, ProjectRegistry};

#[derive(Serialize)]
pub struct ReportInfo {
    pub filename: String,
    /// 檔案的修改時間，Unix 秒。
    pub saved_at: i64,
    /// 從 HTML 的 `<title>` 取出；沒有就是 None，前端顯示檔名。
    pub title: Option<String>,
}

fn project(reg: &ProjectRegistry, id: &str) -> Result<ProjectHandle, String> {
    reg.get(id).ok_or_else(|| format!("專案不存在或已關閉：{id}"))
}

fn reports_dir(project: &ProjectHandle) -> PathBuf {
    project.path.join("reports")
}

/// 只接受單純的檔名。路徑分隔字元、`..`、空字串一律拒絕——這個指令
/// 不能假設呼叫端守規矩。
fn safe_report_name(name: &str) -> Result<&str, String> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || PathBuf::from(name).components().count() != 1
    {
        return Err(format!("不合法的報告檔名：{name}"));
    }
    Ok(name)
}

/// `<stamp>.html`，若已存在則往後找 `-2`、`-3`。同一分鐘內產第二份
/// 不可以覆蓋第一份。
fn report_filename(existing: &[String], stamp: &str) -> String {
    let first = format!("{stamp}.html");
    if !existing.iter().any(|e| e == &first) {
        return first;
    }
    for n in 2..1000 {
        let candidate = format!("{stamp}-{n}.html");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
    }
    format!("{stamp}-{}.html", uuid::Uuid::new_v4())
}

/// 從 HTML 抓 `<title>`。刻意用最笨的字串搜尋而不是 HTML 解析器：
/// 只是為了給歷史清單一個好看的標籤，抓不到就顯示檔名，不值得為此
/// 引入一個解析器。
fn title_from_html(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = lower[start..].find("</title>")? + start;
    let title = html[start..end].trim();
    if title.is_empty() { None } else { Some(title.to_string()) }
}

fn list_filenames(dir: &PathBuf) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".html"))
        .collect()
}

#[tauri::command]
pub async fn reports_save(
    project_id: String,
    html: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let dir = reports_dir(&p);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M").to_string();
    let filename = report_filename(&list_filenames(&dir), &stamp);
    std::fs::write(dir.join(&filename), html).map_err(|e| e.to_string())?;
    Ok(filename)
}

#[tauri::command]
pub async fn reports_list(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<ReportInfo>, String> {
    let p = project(&reg, &project_id)?;
    let dir = reports_dir(&p);
    let mut out = Vec::new();
    for filename in list_filenames(&dir) {
        let path = dir.join(&filename);
        let saved_at = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let title = std::fs::read_to_string(&path).ok().and_then(|h| title_from_html(&h));
        out.push(ReportInfo { filename, saved_at, title });
    }
    // 新到舊。時間相同時用檔名遞減當第二順位，讓順序是確定的
    // （同一分鐘的 -2 排在無後綴的前面）。
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then(b.filename.cmp(&a.filename)));
    Ok(out)
}

#[tauri::command]
pub async fn reports_read(
    project_id: String,
    filename: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let safe = safe_report_name(&filename)?;
    std::fs::read_to_string(reports_dir(&p).join(safe)).map_err(|e| e.to_string())
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib reports::tests`
Expected: PASS，6 個測試全過。

若 `chrono` 不在相依裡，先確認：Run `grep -n '^chrono' src-tauri/Cargo.toml`（`projects/mod.rs` 已在用 `chrono::Utc`，應該有）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/reports.rs src-tauri/src/commands/mod.rs
git commit -m "feat(reports): 報告的存/列/讀，含路徑穿越防護與同分鐘檔名衝突處理"
```

---

## Task 3: `tasks_set_summary` 指令與 lib.rs 接線

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 加入指令**

在 `src-tauri/src/commands/tasks.rs` 的 `tasks_used_dirs` 之後加入：

```rust
/// 寫入這張卡片的 AI 履行摘要。工作報告的第一階段產物——已完成的卡片
/// 不可變，所以這是永久快取，下次產報告時就不必重跑這張。
#[tauri::command]
pub async fn tasks_set_summary(
    project_id: String,
    task_id: String,
    summary: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    store::set_summary(&p.pool, &task_id, &summary).await.map_err(|e| e.to_string())
}
```

- [ ] **Step 2: 註冊指令**

`src-tauri/src/lib.rs` 的 `use commands::{...}` 區塊中，`tasks::{...}` 那組加入 `tasks_set_summary`；並在 `projects::{...}` 那組之後加入：

```rust
    reports::{reports_list, reports_read, reports_save},
```

`invoke_handler` 的 `generate_handler![...]` 清單中加入：

```rust
            commands::tasks::tasks_set_summary,
            commands::reports::reports_save,
            commands::reports::reports_list,
            commands::reports::reports_read,
```

- [ ] **Step 3: 編譯確認**

Run: `cd src-tauri && cargo check --lib`
Expected: 無新錯誤（既有的兩個 warning 不算）。

- [ ] **Step 4: 跑完整測試**

Run: `cd src-tauri && cargo test`
Expected: 全綠。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/tasks.rs src-tauri/src/lib.rs
git commit -m "feat(tasks): tasks_set_summary 指令與 reports_* 接線"
```

---

## Task 4: 前端 IPC 層

**Files:**
- Create: `src/ipc/reports.ts`
- Create: `src/ipc/reports.test.ts`
- Modify: `src/ipc/tasks.ts`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/ipc/reports.test.ts`：

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { listReports, readReport, saveReport } from "./reports";

describe("reports ipc", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
  });

  it("saveReport 傳 projectId 與 html", async () => {
    invoke.mockResolvedValue("2026-09-05-1430.html");
    await expect(saveReport("p1", "<html></html>")).resolves.toBe("2026-09-05-1430.html");
    expect(invoke).toHaveBeenCalledWith("reports_save", { projectId: "p1", html: "<html></html>" });
  });

  it("listReports 傳 projectId", async () => {
    invoke.mockResolvedValue([]);
    await listReports("p1");
    expect(invoke).toHaveBeenCalledWith("reports_list", { projectId: "p1" });
  });

  it("readReport 傳 projectId 與 filename", async () => {
    invoke.mockResolvedValue("<html></html>");
    await readReport("p1", "a.html");
    expect(invoke).toHaveBeenCalledWith("reports_read", { projectId: "p1", filename: "a.html" });
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/ipc/reports.test.ts`
Expected: FAIL，`Failed to resolve import "./reports"`。

- [ ] **Step 3: 寫最小實作**

建立 `src/ipc/reports.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";

/** 鏡射 Rust 的 `commands::reports::ReportInfo`。 */
export interface ReportInfo {
  filename: string;
  /** Unix 秒。 */
  saved_at: number;
  /** 從 HTML 的 `<title>` 取出；null 時前端顯示檔名。 */
  title: string | null;
}

/** 存進 `<專案>/reports/`，回傳實際使用的檔名。 */
export const saveReport = (projectId: string, html: string): Promise<string> =>
  invoke("reports_save", { projectId, html });

/** 該專案的歷史報告，新到舊。 */
export const listReports = (projectId: string): Promise<ReportInfo[]> =>
  invoke("reports_list", { projectId });

export const readReport = (projectId: string, filename: string): Promise<string> =>
  invoke("reports_read", { projectId, filename });
```

`src/ipc/tasks.ts` 的 `TaskRow` 介面，在 `finished_at` 之後加入：

```ts
  /** AI 履行摘要（工作報告用）。只有跑過的卡片會有。 */
  ai_summary: string | null;
```

並在 `saveTranscript` 之後加入：

```ts
export const setSummary = (projectId: string, id: string, summary: string): Promise<void> =>
  invoke("tasks_set_summary", { projectId, taskId: id, summary });
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/ipc/reports.test.ts && npx tsc -b`
Expected: 3 個測試通過；`tsc -b` 可能因為既有測試的 `card()` fixture 少了 `ai_summary` 而報錯——若是，在 `src/components/TaskBoard/index.test.tsx` 的 `card()` 預設值加上 `ai_summary: null`。

- [ ] **Step 5: Commit**

```bash
git add src/ipc/reports.ts src/ipc/reports.test.ts src/ipc/tasks.ts
git add src/components/TaskBoard/index.test.tsx
git commit -m "feat(ipc): reports IPC 與 tasks 的 ai_summary/setSummary"
```

---

## Task 5: 提示詞模組

**Files:**
- Create: `src/components/TaskBoard/reportPrompts.ts`
- Create: `src/components/TaskBoard/reportPrompts.test.ts`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/TaskBoard/reportPrompts.test.ts`：

```ts
import { describe, expect, it } from "vitest";

import { buildSummaryPrompt, buildReportPrompt, MAX_CARDS } from "./reportPrompts";
import type { TaskWithAttachments } from "../../ipc/tasks";

const card = (over: Partial<TaskWithAttachments> = {}): TaskWithAttachments =>
  ({
    id: "t1",
    title: "做一件事",
    body: "詳細說明",
    project_dir: "/repo",
    status: "done",
    parallel_ok: true,
    interactive: false,
    sort_order: 1,
    outcome: "success",
    tab_id: null,
    transcript_path: null,
    error_message: null,
    created_at: "2026-09-05T10:00:00Z",
    dispatched_at: null,
    finished_at: null,
    ai_summary: null,
    attachments: [],
    ...over,
  }) as TaskWithAttachments;

describe("buildSummaryPrompt", () => {
  it("帶入標題、內容與對話記錄", () => {
    const p = buildSummaryPrompt(card({ title: "重構登入" }), "終端機輸出內容");
    expect(p).toContain("重構登入");
    expect(p).toContain("終端機輸出內容");
  });

  it("沒有對話記錄時也能組出提示詞，並說明只有欄位資料", () => {
    const p = buildSummaryPrompt(card(), null);
    expect(p).toContain("沒有對話記錄");
  });

  it("要求限制字數，避免第二階段輸入爆掉", () => {
    expect(buildSummaryPrompt(card(), "x")).toContain("300");
  });
});

describe("buildReportPrompt", () => {
  const cards = [
    card({ id: "a", title: "已完成的", status: "done", ai_summary: "摘要 A" }),
    card({ id: "b", title: "執行中的", status: "running", outcome: null }),
    card({ id: "c", title: "還沒開始的", status: "planning", outcome: null }),
  ];

  it("兩種風格產生不同的提示詞", () => {
    const review = buildReportPrompt(cards, "review", "我的專案");
    const formal = buildReportPrompt(cards, "formal", "我的專案");
    expect(review).not.toEqual(formal);
  });

  it("包含全部四欄的卡片，不只已完成的", () => {
    const p = buildReportPrompt(cards, "review", "我的專案");
    expect(p).toContain("已完成的");
    expect(p).toContain("執行中的");
    expect(p).toContain("還沒開始的");
  });

  it("帶入已完成卡片的摘要", () => {
    expect(buildReportPrompt(cards, "review", "我的專案")).toContain("摘要 A");
  });

  it("要求輸出 artifact-html", () => {
    expect(buildReportPrompt(cards, "review", "我的專案")).toContain("artifact-html");
  });

  // 卡片太多時第二階段的輸入會爆掉。取最近的，並且要讓報告知道
  // 自己看到的不是全部——不講的話 AI 會把「最近 100 張」當成「全部」。
  it("超過上限時只取最近的並在提示詞中註明", () => {
    const many = Array.from({ length: MAX_CARDS + 20 }, (_, i) =>
      card({ id: `t${i}`, title: `卡片 ${i}` }),
    );
    const p = buildReportPrompt(many, "review", "我的專案");
    expect(p).toContain(String(MAX_CARDS));
    expect(p).not.toContain("卡片 0");
    expect(p).toContain(`卡片 ${MAX_CARDS + 19}`);
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/components/TaskBoard/reportPrompts.test.ts`
Expected: FAIL，`Failed to resolve import "./reportPrompts"`。

- [ ] **Step 3: 寫最小實作**

建立 `src/components/TaskBoard/reportPrompts.ts`：

```ts
import type { TaskWithAttachments } from "../../ipc/tasks";

/** 報告風格。`review` 給自己看，`formal` 給上司／客戶看。 */
export type ReportStyle = "review" | "formal";

/**
 * 第二階段最多帶幾張卡片。超過就取最近的——這是防止極端情況下輸入
 * 爆掉的保險，不是預期會踩到的線。
 */
export const MAX_CARDS = 100;

/** 一張卡片摘要的字數上限，寫進提示詞裡要求模型遵守。 */
const SUMMARY_MAX_CHARS = 300;

const STATUS_LABEL: Record<string, string> = {
  planning: "規劃中",
  queued: "待執行",
  running: "執行中",
  done: "已完成",
};

const OUTCOME_LABEL: Record<string, string> = {
  success: "成功",
  failed: "失敗",
  cancelled: "已取消",
};

/**
 * 第一階段：把一張已完成卡片的對話記錄摘要成一段短文字。
 *
 * `transcript` 為 null 代表記錄檔不存在（被刪掉、或那次執行沒留下）。
 * 這不是錯誤——照樣用欄位資料產生摘要，只是內容會比較粗略。
 */
export function buildSummaryPrompt(
  card: TaskWithAttachments,
  transcript: string | null,
): string {
  const lines = [
    "請把下面這個工作項目的執行過程，整理成一段給工作報告用的摘要。",
    "",
    `標題：${card.title}`,
    `工作內容：${card.body || "（未填寫）"}`,
    `工作目錄：${card.project_dir}`,
    `結果：${card.outcome ? OUTCOME_LABEL[card.outcome] ?? card.outcome : "未知"}`,
  ];
  if (card.error_message) lines.push(`錯誤訊息：${card.error_message}`);
  lines.push("");
  if (transcript) {
    lines.push("以下是這次執行的終端機對話記錄：", "", transcript);
  } else {
    lines.push("（這次執行沒有對話記錄，請只根據上面的欄位資料整理。）");
  }
  lines.push(
    "",
    `請用繁體中文寫一段 ${SUMMARY_MAX_CHARS} 字以內的摘要，說明實際做了什麼、`,
    "過程中遇到什麼問題、最後結果如何。只輸出摘要本文，不要加標題或前言。",
  );
  return lines.join("\n");
}

function cardLine(card: TaskWithAttachments): string {
  const status = STATUS_LABEL[card.status] ?? card.status;
  const outcome = card.outcome ? `／${OUTCOME_LABEL[card.outcome] ?? card.outcome}` : "";
  const parts = [`- 【${status}${outcome}】${card.title}`];
  if (card.body) parts.push(`  內容：${card.body}`);
  parts.push(`  工作目錄：${card.project_dir}`);
  if (card.error_message) parts.push(`  錯誤：${card.error_message}`);
  if (card.ai_summary) parts.push(`  執行摘要：${card.ai_summary}`);
  return parts.join("\n");
}

const STYLE_INSTRUCTIONS: Record<ReportStyle, string> = {
  review: [
    "這份報告是給我自己回顧進度用的。重點放在：",
    "- 目前做到哪個階段",
    "- 哪些工作卡住了、為什麼",
    "- 接下來應該優先處理什麼",
    "可以直接講技術細節，也請直接點出失敗的原因，不需要修飾。",
  ].join("\n"),
  formal: [
    "這份報告是要給主管或客戶看的正式工作報告。重點放在：",
    "- 這段期間完成了哪些工作",
    "- 各項工作的具體成果",
    "語氣正式、精簡，少講技術細節，重點在產出而不是過程。",
    "尚未完成的工作簡短帶過即可，不要強調失敗與錯誤訊息。",
  ].join("\n"),
};

/**
 * 第二階段：把全部卡片（含第一階段的摘要）合成一份 HTML 報告。
 *
 * 卡片超過 `MAX_CARDS` 時只取最近的，**並且在提示詞裡講明**——不講的話
 * 模型會把「最近 100 張」當成專案的全部，報告的結論會失真。
 */
export function buildReportPrompt(
  cards: TaskWithAttachments[],
  style: ReportStyle,
  projectName: string,
): string {
  const truncated = cards.length > MAX_CARDS;
  const used = truncated ? cards.slice(-MAX_CARDS) : cards;

  const lines = [
    `請為專案「${projectName}」整理一份工作報告。`,
    "",
    STYLE_INSTRUCTIONS[style],
    "",
  ];
  if (truncated) {
    lines.push(
      `注意：這個專案共有 ${cards.length} 個工作項目，因為篇幅限制，下面只列出最近的 ${MAX_CARDS} 個。`,
      "請在報告中說明這件事，不要把這些當成專案的全部。",
      "",
    );
  }
  lines.push("工作項目如下：", "", ...used.map(cardLine), "");
  lines.push(
    "請把報告寫成一份完整的 HTML 文件，放在 ```artifact-html 區塊裡。",
    "文件要有 <title>（會成為報告的標題）、清楚的段落結構、以及適當的內嵌 <style>。",
    "內容用繁體中文。",
  );
  return lines.join("\n");
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/reportPrompts.test.ts`
Expected: PASS，8 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/reportPrompts.ts src/components/TaskBoard/reportPrompts.test.ts
git commit -m "feat(board): 工作報告的兩階段提示詞"
```

---

## Task 6: `useWorkReport` 產生流程

**Files:**
- Create: `src/components/TaskBoard/useWorkReport.ts`
- Create: `src/components/TaskBoard/useWorkReport.test.ts`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/TaskBoard/useWorkReport.test.ts`：

```ts
import { renderHook, act, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listTasks = vi.fn();
const readTranscript = vi.fn();
const setSummary = vi.fn();
const saveReport = vi.fn();
const invokeAiChat = vi.fn();

vi.mock("../../ipc/tasks", () => ({
  listTasks: (...a: unknown[]) => listTasks(...a),
  readTranscript: (...a: unknown[]) => readTranscript(...a),
  setSummary: (...a: unknown[]) => setSummary(...a),
}));
vi.mock("../../ipc/reports", () => ({ saveReport: (...a: unknown[]) => saveReport(...a) }));
vi.mock("../../ipc/ai", () => ({ invokeAiChat: (...a: unknown[]) => invokeAiChat(...a) }));

import { useWorkReport } from "./useWorkReport";

const card = (over: Record<string, unknown> = {}) => ({
  id: "t1",
  title: "卡片",
  body: "",
  project_dir: "/r",
  status: "done",
  parallel_ok: true,
  interactive: false,
  sort_order: 1,
  outcome: "success",
  tab_id: "tab-1",
  transcript_path: "/p/t.txt",
  error_message: null,
  created_at: "2026-09-05T10:00:00Z",
  dispatched_at: null,
  finished_at: null,
  ai_summary: null,
  attachments: [],
  ...over,
});

const ARTIFACT = "說明\n\n```artifact-html\n<html><title>報告</title></html>\n```";

describe("useWorkReport", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    readTranscript.mockResolvedValue("終端機輸出");
    setSummary.mockResolvedValue(undefined);
    saveReport.mockResolvedValue("2026-09-05-1430.html");
    invokeAiChat.mockResolvedValue({ content: "摘要內容", tool_calls: [] });
  });

  it("已經有 ai_summary 的卡片不重複呼叫 AI 做摘要", async () => {
    listTasks.mockResolvedValue([card({ ai_summary: "已經有的摘要" })]);
    invokeAiChat.mockResolvedValue({ content: ARTIFACT, tool_calls: [] });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    // 只有第二階段那一次
    expect(invokeAiChat).toHaveBeenCalledTimes(1);
    expect(readTranscript).not.toHaveBeenCalled();
  });

  it("只對已完成的卡片做摘要", async () => {
    listTasks.mockResolvedValue([
      card({ id: "a", status: "done" }),
      card({ id: "b", status: "planning", outcome: null }),
    ]);
    invokeAiChat
      .mockResolvedValueOnce({ content: "摘要 A", tool_calls: [] })
      .mockResolvedValueOnce({ content: ARTIFACT, tool_calls: [] });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(setSummary).toHaveBeenCalledTimes(1);
    expect(setSummary).toHaveBeenCalledWith("p1", "a", "摘要 A");
  });

  // 十張卡因為一張失敗而全部白跑，代價太高。
  it("某張摘要失敗時繼續跑完其他張並產出報告", async () => {
    listTasks.mockResolvedValue([
      card({ id: "a", status: "done" }),
      card({ id: "b", status: "done" }),
    ]);
    invokeAiChat
      .mockRejectedValueOnce(new Error("網路錯誤"))
      .mockResolvedValueOnce({ content: "摘要 B", tool_calls: [] })
      .mockResolvedValueOnce({ content: ARTIFACT, tool_calls: [] });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(setSummary).toHaveBeenCalledWith("p1", "b", "摘要 B");
    expect(saveReport).toHaveBeenCalled();
    expect(result.current.html).toContain("<title>報告</title>");
  });

  it("AI 未設定時在做任何摘要之前就中止", async () => {
    listTasks.mockResolvedValue([card({ status: "done" })]);
    invokeAiChat.mockRejectedValue({ kind: "not_configured" });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(setSummary).not.toHaveBeenCalled();
    expect(saveReport).not.toHaveBeenCalled();
    expect(result.current.error).toBeTruthy();
  });

  // 默默存一份空檔的話，使用者會以為有報告、打開卻是空的。
  it("回覆裡沒有 artifact 區塊時報錯且不存檔", async () => {
    listTasks.mockResolvedValue([card({ ai_summary: "有了" })]);
    invokeAiChat.mockResolvedValue({ content: "我沒有產生文件", tool_calls: [] });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(saveReport).not.toHaveBeenCalled();
    expect(result.current.error).toBeTruthy();
    expect(result.current.rawReply).toContain("我沒有產生文件");
  });

  it("專案沒有任何卡片時擋下，不呼叫 AI", async () => {
    listTasks.mockResolvedValue([]);
    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(invokeAiChat).not.toHaveBeenCalled();
    expect(result.current.error).toBeTruthy();
  });

  it("進度顯示目前處理到第幾張", async () => {
    listTasks.mockResolvedValue([
      card({ id: "a", status: "done" }),
      card({ id: "b", status: "done" }),
    ]);
    let seen: string[] = [];
    invokeAiChat.mockImplementation(async () => {
      seen.push(String(result.current.progress?.done ?? ""));
      return { content: ARTIFACT, tool_calls: [] };
    });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });
    expect(seen.length).toBeGreaterThan(0);
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/components/TaskBoard/useWorkReport.test.ts`
Expected: FAIL，`Failed to resolve import "./useWorkReport"`。

- [ ] **Step 3: 寫最小實作**

建立 `src/components/TaskBoard/useWorkReport.ts`：

```ts
import { useCallback, useRef, useState } from "react";

import { invokeAiChat, type AiError } from "../../ipc/ai";
import { saveReport } from "../../ipc/reports";
import {
  listTasks,
  readTranscript,
  setSummary,
  type TaskWithAttachments,
} from "../../ipc/tasks";
import { splitArtifactFence } from "../../lib/artifactFence";
import { buildReportPrompt, buildSummaryPrompt, type ReportStyle } from "./reportPrompts";

/**
 * 這個錯誤是不是「AI 還沒設定」。
 *
 * 刻意寫成本地小函式而不 import：`normalizeAiError` 在這個 repo 裡沒有
 * 共用版本，`CrossDbAiChat.tsx` 與 `DatabaseAiChat.tsx` 各自有一份複本。
 * 把它們抽成共用模組是對的，但那會把兩個不相干的檔案捲進這個里程碑。
 * 這裡只需要判斷一種 kind，寫五行比開那個頭划算。
 */
function isNotConfigured(err: unknown): boolean {
  if (err && typeof err === "object" && "kind" in err) {
    return (err as AiError).kind === "not_configured";
  }
  if (err instanceof Error) {
    try {
      const parsed: unknown = JSON.parse(err.message);
      return !!parsed && typeof parsed === "object" && "kind" in parsed
        && (parsed as AiError).kind === "not_configured";
    } catch {
      return false;
    }
  }
  return false;
}

export interface ReportProgress {
  /** 已經處理完的卡片數。 */
  done: number;
  /** 這一輪要處理的卡片總數（只算需要補摘要的）。 */
  total: number;
}

/**
 * 工作報告的產生流程。
 *
 * 兩階段：先把每張「已完成且還沒有摘要」的卡片各自摘要並寫回快取，
 * 再把全部卡片合成一份 HTML 報告。已完成的卡片不可變，所以摘要是永久
 * 快取——第二次產報告時只有新完成的卡片需要重跑。
 */
export function useWorkReport(projectId: string, projectName: string) {
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<ReportProgress | null>(null);
  const [html, setHtml] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** artifact 抽不出來時保留原始回覆給使用者看，不要默默失敗。 */
  const [rawReply, setRawReply] = useState<string | null>(null);
  const cancelled = useRef(false);

  const cancel = useCallback(() => {
    cancelled.current = true;
  }, []);

  const generate = useCallback(
    async (style: ReportStyle) => {
      setBusy(true);
      setError(null);
      setHtml(null);
      setRawReply(null);
      cancelled.current = false;

      try {
        const cards = await listTasks(projectId);
        if (cards.length === 0) {
          setError("這個專案還沒有任何工作項目，無法產生報告。");
          return;
        }

        // ── 第一階段：補上缺少的摘要 ──
        const needSummary = cards.filter((c) => c.status === "done" && !c.ai_summary);
        setProgress({ done: 0, total: needSummary.length });

        const summaries = new Map<string, string>();
        for (const [i, c] of needSummary.entries()) {
          if (cancelled.current) return;
          try {
            const transcript = c.transcript_path
              ? await readTranscript(projectId, c.id).catch(() => null)
              : null;
            const reply = await invokeAiChat(
              [{ role: "user", content: buildSummaryPrompt(c, transcript) }],
              `report-summary-${c.id}`,
            );
            const text = (reply.content ?? "").trim();
            if (text) {
              await setSummary(projectId, c.id, text);
              summaries.set(c.id, text);
            }
          } catch (e) {
            // AI 根本沒設定的話，後面每一張都會失敗——直接中止比讓
            // 使用者等一輪無意義的重試有意義。
            if (isNotConfigured(e)) {
              setError("尚未設定 AI 供應商，請先到設定裡完成設定。");
              return;
            }
            // 其他錯誤：略過這張，繼續下一張。一張失敗不該讓整份報告白跑。
          }
          setProgress({ done: i + 1, total: needSummary.length });
        }
        if (cancelled.current) return;

        // ── 第二階段：合成 ──
        const enriched = cards.map((c): TaskWithAttachments => {
          const fresh = summaries.get(c.id);
          return fresh ? { ...c, ai_summary: fresh } : c;
        });
        const reply = await invokeAiChat(
          [{ role: "user", content: buildReportPrompt(enriched, style, projectName) }],
          `report-${projectId}`,
          undefined,
          false,
          "zh-TW",
          true, // supportsArtifacts — 後端才會接上 artifact 協定的說明
        );
        if (cancelled.current) return;

        const text = reply.content ?? "";
        const { artifact } = splitArtifactFence(text);
        if (!artifact || artifact.kind !== "html") {
          setRawReply(text);
          setError("AI 沒有產生報告文件。下面是它的原始回覆。");
          return;
        }

        setHtml(artifact.content);
        try {
          await saveReport(projectId, artifact.content);
        } catch (e) {
          // 報告本身已經產生了，存檔失敗不該讓它消失——顯示出來並說明。
          setError(`報告已產生，但存檔失敗：${String(e)}`);
        }
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
        setProgress(null);
      }
    },
    [projectId, projectName],
  );

  return { generate, cancel, busy, progress, html, error, rawReply, setHtml };
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/useWorkReport.test.ts`
Expected: PASS，7 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/useWorkReport.ts src/components/TaskBoard/useWorkReport.test.ts
git commit -m "feat(board): 工作報告的產生流程與摘要快取"
```

---

## Task 7: 報告視窗

**Files:**
- Create: `src/components/TaskBoard/ReportDialog.tsx`
- Create: `src/components/TaskBoard/ReportDialog.test.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 加入 i18n 字串**

`src/lib/i18n.ts` 的 zhTW 字典中，`proj_remove_failed` 之後加入：

```ts
    report_title: "工作報告",
    report_generate: "產生工作報告",
    report_short: "報告",
    report_style_review: "回顧進度（給自己）",
    report_style_review_hint: "做到哪了、哪裡卡住、下一步該做什麼",
    report_style_formal: "工作報告（給主管／客戶）",
    report_style_formal_hint: "完成了哪些工作、成果是什麼",
    report_generating: "正在整理…",
    report_progress: "正在整理第 {done}/{total} 個工作項目…",
    report_cancel: "取消",
    report_history: "歷史報告",
    report_history_empty: "還沒有產生過報告",
    report_save_as: "另存為…",
    report_raw_reply: "AI 的原始回覆",
    report_close: "關閉",
```

en 字典中對應加入：

```ts
    report_title: "Work report",
    report_generate: "Generate work report",
    report_short: "Report",
    report_style_review: "Progress review (for yourself)",
    report_style_review_hint: "Where things stand, what is stuck, what is next",
    report_style_formal: "Work report (for a manager or client)",
    report_style_formal_hint: "What was completed and what it delivered",
    report_generating: "Generating…",
    report_progress: "Summarising task {done} of {total}…",
    report_cancel: "Cancel",
    report_history: "Past reports",
    report_history_empty: "No reports yet",
    report_save_as: "Save as…",
    report_raw_reply: "The AI's raw reply",
    report_close: "Close",
```

- [ ] **Step 2: 寫失敗的測試**

建立 `src/components/TaskBoard/ReportDialog.test.tsx`：

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listReports = vi.fn();
const readReport = vi.fn();
const generate = vi.fn();

vi.mock("../../ipc/reports", () => ({
  listReports: (...a: unknown[]) => listReports(...a),
  readReport: (...a: unknown[]) => readReport(...a),
  saveReport: vi.fn(),
}));
vi.mock("./useWorkReport", () => ({
  useWorkReport: () => ({
    generate,
    cancel: vi.fn(),
    busy: false,
    progress: null,
    html: null,
    error: null,
    rawReply: null,
    setHtml: vi.fn(),
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
vi.mock("../../ipc/fs", () => ({ writeTextFile: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ReportDialog } from "./ReportDialog";

const mount = () =>
  render(
    <LocaleProvider>
      <ReportDialog projectId="p1" projectName="我的專案" onClose={vi.fn()} />
    </LocaleProvider>,
  );

describe("ReportDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listReports.mockResolvedValue([]);
    readReport.mockResolvedValue("<html><title>舊報告</title></html>");
  });

  it("開啟時先讓使用者選風格", async () => {
    mount();
    expect(await screen.findByTestId("report-style-review")).toBeInTheDocument();
    expect(screen.getByTestId("report-style-formal")).toBeInTheDocument();
  });

  it("選了風格才開始產生", async () => {
    mount();
    await userEvent.click(await screen.findByTestId("report-style-review"));
    expect(generate).toHaveBeenCalledWith("review");
  });

  it("列出歷史報告", async () => {
    listReports.mockResolvedValue([
      { filename: "2026-09-05-1430.html", saved_at: 1788600000, title: "第三季進度" },
    ]);
    mount();
    expect(await screen.findByText("第三季進度")).toBeInTheDocument();
  });

  it("沒有標題的報告顯示檔名", async () => {
    listReports.mockResolvedValue([
      { filename: "2026-09-05-1430.html", saved_at: 1788600000, title: null },
    ]);
    mount();
    expect(await screen.findByText("2026-09-05-1430.html")).toBeInTheDocument();
  });

  it("點歷史報告會讀回它的內容", async () => {
    listReports.mockResolvedValue([
      { filename: "2026-09-05-1430.html", saved_at: 1788600000, title: "舊報告" },
    ]);
    mount();
    await userEvent.click(await screen.findByText("舊報告"));
    await waitFor(() => expect(readReport).toHaveBeenCalledWith("p1", "2026-09-05-1430.html"));
  });

  it("沒有歷史報告時顯示空狀態", async () => {
    mount();
    expect(await screen.findByTestId("report-history-empty")).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: 執行測試，確認它失敗**

Run: `npm run test -- src/components/TaskBoard/ReportDialog.test.tsx`
Expected: FAIL，`Failed to resolve import "./ReportDialog"`。

- [ ] **Step 4: 寫最小實作**

建立 `src/components/TaskBoard/ReportDialog.tsx`：

```tsx
import { useCallback, useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { ArtifactHtmlFrame } from "../ArtifactPanel/ArtifactHtmlFrame";
import { writeTextFile } from "../../ipc/fs";
import { listReports, readReport, type ReportInfo } from "../../ipc/reports";
import { useWorkReport } from "./useWorkReport";
import type { ReportStyle } from "./reportPrompts";

/**
 * 工作報告視窗：先選風格 → 產生 → 呈現，側邊可切換這個專案的歷史報告。
 *
 * HTML 用既有的 `ArtifactHtmlFrame` 渲染——那是個 sandbox iframe，
 * 刻意不給 `allow-same-origin`，所以報告裡的 script 碰不到主視窗，
 * 更碰不到 Tauri 的 IPC。不要為了任何理由改那個設定。
 */
export function ReportDialog({
  projectId,
  projectName,
  onClose,
}: {
  projectId: string;
  projectName: string;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const { generate, cancel, busy, progress, html, error, rawReply, setHtml } = useWorkReport(
    projectId,
    projectName,
  );
  const [history, setHistory] = useState<ReportInfo[]>([]);
  const [picked, setPicked] = useState<string | null>(null);
  const [started, setStarted] = useState(false);

  const refreshHistory = useCallback(async () => {
    setHistory(await listReports(projectId));
  }, [projectId]);

  useEffect(() => {
    void refreshHistory();
  }, [refreshHistory]);

  // 產生完成後歷史多一筆，重新抓一次。
  useEffect(() => {
    if (html && started) void refreshHistory();
  }, [html, started, refreshHistory]);

  const start = (style: ReportStyle) => {
    setStarted(true);
    setPicked(null);
    void generate(style);
  };

  const openHistory = async (info: ReportInfo) => {
    setPicked(info.filename);
    setHtml(await readReport(projectId, info.filename));
  };

  const saveAs = async () => {
    if (!html) return;
    const path = await save({
      defaultPath: `${projectName}-報告.html`,
      filters: [{ name: "HTML", extensions: ["html"] }],
    });
    if (typeof path === "string") await writeTextFile(path, html);
  };

  const progressText =
    progress
      ? t.report_progress
          .replace("{done}", String(progress.done))
          .replace("{total}", String(progress.total))
      : t.report_generating;

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="report-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="report-head">
          <h3>{t.report_title} — {projectName}</h3>
          <div className="report-head-actions">
            {html && (
              <button className="tb-btn tb-btn--ghost" onClick={() => void saveAs()}>
                {t.report_save_as}
              </button>
            )}
            <button className="tb-btn tb-btn--ghost" onClick={onClose}>{t.report_close}</button>
          </div>
        </div>

        <div className="report-body">
          <aside className="report-history">
            <div className="report-history-title">{t.report_history}</div>
            {history.length === 0 ? (
              <div className="report-history-empty" data-testid="report-history-empty">
                {t.report_history_empty}
              </div>
            ) : (
              history.map((h) => (
                <button
                  key={h.filename}
                  className={`report-history-item${picked === h.filename ? " report-history-item--active" : ""}`}
                  onClick={() => void openHistory(h)}
                >
                  {h.title ?? h.filename}
                </button>
              ))
            )}
          </aside>

          <main className="report-main">
            {!started && !html && (
              <div className="report-style-picker">
                <button
                  className="report-style"
                  data-testid="report-style-review"
                  onClick={() => start("review")}
                >
                  <strong>{t.report_style_review}</strong>
                  <span>{t.report_style_review_hint}</span>
                </button>
                <button
                  className="report-style"
                  data-testid="report-style-formal"
                  onClick={() => start("formal")}
                >
                  <strong>{t.report_style_formal}</strong>
                  <span>{t.report_style_formal_hint}</span>
                </button>
              </div>
            )}

            {busy && (
              <div className="report-progress">
                <span>{progressText}</span>
                <button className="tb-btn tb-btn--ghost" onClick={cancel}>{t.report_cancel}</button>
              </div>
            )}

            {error && <div className="report-error">{error}</div>}
            {rawReply && (
              <details className="report-raw">
                <summary>{t.report_raw_reply}</summary>
                <pre>{rawReply}</pre>
              </details>
            )}

            {html && <ArtifactHtmlFrame html={html} title={t.report_title} />}
          </main>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 5: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/ReportDialog.test.tsx && npx tsc -b`
Expected: 6 個測試通過，型別檢查通過。

- [ ] **Step 6: Commit**

```bash
git add src/components/TaskBoard/ReportDialog.tsx src/components/TaskBoard/ReportDialog.test.tsx src/lib/i18n.ts
git commit -m "feat(board): 工作報告視窗與歷史清單"
```

---

## Task 8: 兩個入口與樣式

**Files:**
- Modify: `src/components/TaskBoard/ProjectList.tsx`
- Modify: `src/components/TaskBoard/ProjectBoard.tsx`
- Modify: `src/components/TaskBoard/index.css`

- [ ] **Step 1: 寫失敗的測試**

在 `src/components/TaskBoard/ProjectList.test.tsx` 的最後一個 `it` 之後加入：

```tsx
  it("每張專案卡片都有產生報告的入口", async () => {
    mount([project({ counts: { planning: 1, queued: 0, running: 0, done: 2 } })]);
    await screen.findByText("makemoney");
    expect(screen.getByTestId("project-report-p1")).toBeInTheDocument();
  });

  it("點報告按鈕不會連帶進入該專案的看板", async () => {
    const onOpen = vi.fn();
    mount([project({ counts: { planning: 1, queued: 0, running: 0, done: 2 } })], onOpen);
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-report-p1"));
    expect(onOpen).not.toHaveBeenCalled();
  });
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/components/TaskBoard/ProjectList.test.tsx`
Expected: FAIL，找不到 `project-report-p1`。

- [ ] **Step 3: 寫最小實作**

`src/components/TaskBoard/ProjectList.tsx`：

props 加入 `onReport`：

```tsx
export function ProjectList({
  projects,
  onRefresh,
  onOpen,
  onReport,
}: {
  projects: ProjectInfo[];
  onRefresh: () => Promise<void>;
  onOpen: (projectId: string) => void;
  onReport: (projectId: string) => void;
}) {
```

在每張專案卡片的「移除」按鈕之前加入：

```tsx
                <button
                  className="tb-btn tb-btn--ghost"
                  data-testid={`project-report-${p.id}`}
                  disabled={broken}
                  onClick={() => onReport(p.id)}
                >
                  {t.report_short}
                </button>
```

`src/components/TaskBoard/index.tsx`（路由器）：加上報告視窗的狀態，並把 `onReport` 傳下去。在 `const [showList, setShowList] = useState(false);` 之後加入：

```tsx
  /** 開著報告視窗的專案 id；null 代表沒開。 */
  const [reportFor, setReportFor] = useState<string | null>(null);
```

`<ProjectList ... />` 加上 `onReport={setReportFor}`，`<ProjectBoard ... />` 加上 `onReport={() => setReportFor(active)}`，並在兩個 return 的最外層 `<div className="task-board">` 內部末端各加上：

```tsx
      {reportFor && (
        <ReportDialog
          projectId={reportFor}
          projectName={projects.find((p) => p.id === reportFor)?.name ?? ""}
          onClose={() => setReportFor(null)}
        />
      )}
```

並在檔頂 import：

```tsx
import { ReportDialog } from "./ReportDialog";
```

`src/components/TaskBoard/ProjectBoard.tsx`：props 加入 `onReport: () => void`，工具列的「+ 新工作」按鈕之後加入：

```tsx
        <button className="tb-btn tb-btn--ghost" onClick={onReport}>
          {t.report_generate}
        </button>
```

- [ ] **Step 4: 加入樣式**

在 `src/components/TaskBoard/index.css` 末端加入：

```css
/* ── 工作報告視窗 ─────────────────────────────────────────── */

.report-dialog {
  width: min(1100px, 94vw);
  height: min(760px, 88vh);
  display: flex;
  flex-direction: column;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 12px;
  overflow: hidden;
}

.report-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  flex: 0 0 auto;
}

.report-head h3 {
  margin: 0;
  font-size: 15px;
  color: var(--text-primary);
}

.report-head-actions {
  display: flex;
  gap: 8px;
}

.report-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

.report-history {
  width: 220px;
  flex: 0 0 auto;
  border-right: 1px solid var(--border);
  padding: 10px;
  overflow-y: auto;
}

.report-history-title {
  font-size: 12px;
  color: var(--text-secondary);
  margin-bottom: 8px;
}

.report-history-empty {
  font-size: 12px;
  color: var(--text-secondary);
  padding: 8px 0;
}

.report-history-item {
  display: block;
  width: 100%;
  text-align: left;
  background: none;
  border: 0;
  border-radius: 6px;
  color: var(--text-primary);
  cursor: pointer;
  padding: 6px 8px;
  font-size: 12px;
}

.report-history-item--active {
  background: var(--bg-secondary);
}

.report-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  padding: 12px;
  gap: 10px;
  overflow-y: auto;
}

.report-style-picker {
  display: flex;
  gap: 12px;
  flex-wrap: wrap;
}

.report-style {
  flex: 1 1 260px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  text-align: left;
  padding: 16px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--bg-secondary);
  color: var(--text-primary);
  cursor: pointer;
}

.report-style span {
  font-size: 12px;
  color: var(--text-secondary);
}

.report-progress {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 13px;
  color: var(--text-secondary);
}

.report-error {
  color: var(--tb-failed);
  font-size: 13px;
}

.report-raw pre {
  max-height: 260px;
  overflow: auto;
  font-size: 12px;
  white-space: pre-wrap;
}

.report-main .aiterm-artifact-html-frame {
  flex: 1;
  min-height: 400px;
  width: 100%;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: #fff;
}
```

**注意**：這個檔案的註解裡**絕對不能出現星號緊接斜線的組合**——會提前關閉註解並把緊接著的整條規則吃掉，害那條規則靜默失效。檔案裡既有的那段警告註解不要刪。

- [ ] **Step 5: 用 postcss 驗證新規則真的被解析出來**

用眼睛看 CSS 看不出註解提前關閉的問題，一定要跑這個：

```bash
node -e "
const fs=require('fs'),postcss=require('postcss');
const root=postcss.parse(fs.readFileSync('src/components/TaskBoard/index.css','utf8'));
const got=[];root.walkRules(r=>got.push(r.selector));
const want=['.report-dialog','.report-history-item','.report-style','.report-main'];
const missing=want.filter(s=>!got.some(g=>g.includes(s)));
console.log(missing.length?'MISSING: '+missing.join(', '):'all present');
"
```
Expected: `all present`

- [ ] **Step 6: 執行測試，確認通過**

Run: `npx tsc -b && npm run test`
Expected: 全綠。既有的 `ProjectList.test.tsx` 與 `router.test.tsx` 會因為新 prop 而需要調整——它們的 `mount()` 要傳 `onReport={vi.fn()}`。

- [ ] **Step 7: Commit**

```bash
git add src/components/TaskBoard/ProjectList.tsx src/components/TaskBoard/ProjectList.test.tsx
git add src/components/TaskBoard/ProjectBoard.tsx src/components/TaskBoard/index.tsx
git add src/components/TaskBoard/router.test.tsx src/components/TaskBoard/index.css
git commit -m "feat(board): 專案總覽與看板工具列的報告入口，含視窗樣式"
```

---

## Task 9: 完整驗證

**Files:** 無（純驗證）

- [ ] **Step 1: 四道關卡**

```bash
npx tsc -b                      # 不可用 tsc --noEmit
npm run test                    # 必須全綠
cd src-tauri && cargo test      # 完整，不可用 --lib
npm run lint                    # 總數不得超過 125
```

lint 若增加，最可能的來源是 `react-hooks/exhaustive-deps`（`useCallback`/`useEffect` 的相依陣列漏了 `projectId` 或 `projectName`）與 `react-hooks/set-state-in-effect`。**不可以用 `eslint-disable` 蓋掉**——那不是修好，只是藏起來。

- [ ] **Step 2: 突變驗證關鍵測試**

改測試最容易滑進「改到不會紅」。對這三個做突變，確認它們真的會咬：

| 突變 | 預期會紅的測試 |
|---|---|
| `useWorkReport` 的第一階段不再檢查 `!c.ai_summary`（每張都重跑） | 「已經有 ai_summary 的卡片不重複呼叫 AI」 |
| `useWorkReport` 的 catch 改成 `throw`（一張失敗就中斷） | 「某張摘要失敗時繼續跑完其他張」 |
| `safe_report_name` 直接回 `Ok(name)` | 「拒絕路徑穿越」 |

每次驗證完**用 `git checkout <path>` 還原**，不要用 `mv file.bak file`——`.bak` 保留舊 mtime，cargo/vite 會判定不需重編，你會對著舊產物下錯結論。

- [ ] **Step 3: 回報**

把四道關卡的實際數字與突變驗證的結果列出來。

---

## Task 10: 手動驗收

**Files:** 無（純驗證）

- [ ] **Step 1: 啟動**

Run: `npm run tauri:dev`

- [ ] **Step 2: 逐項確認**

| # | 操作 | 預期 |
|---|---|---|
| 1 | 專案總覽的專案卡片按「報告」 | 開出報告視窗、先問要哪種風格 |
| 2 | 選「回顧進度」 | 顯示「正在整理第 N/M 個…」的具體進度，不是只轉圈圈 |
| 3 | 等它跑完 | 報告顯示在視窗裡，內容涵蓋四欄的卡片 |
| 4 | 看專案資料夾 | 多出 `reports/2026-09-05-XXXX.html` |
| 5 | 再產一次，選「工作報告」 | 這次**明顯更快**（摘要已快取），且語氣與上一份不同 |
| 6 | 看側邊歷史清單 | 兩份都在，可以點回去看 |
| 7 | 按「另存為…」 | 可存成 `.html`，用瀏覽器打得開 |
| 8 | 把一張已完成的卡片拖回「規劃」再重跑一次，然後產報告 | 那張卡的摘要是新的，不是上一次執行的內容 |
| 9 | 進到某個專案的看板，用工具列的「產生工作報告」 | 行為與從總覽進入相同 |
| 10 | 產生途中按「取消」 | 停下來、不產生報告檔；已經整理好的摘要保留（下次更快） |

- [ ] **Step 3: 回報**

把不符預期的項目連同實際看到的行為列出來。不要自行擴大修改範圍。

---

## 完成後

依 CLAUDE.md，實作完成時使用 `superpowers:verification-before-completion`，合併前使用 `superpowers:requesting-code-review`。

本里程碑**不做**（spec §10）：自訂報告提示詞、跨專案合併報告、排程自動產生、PDF/Word 匯出、報告內容編輯。
