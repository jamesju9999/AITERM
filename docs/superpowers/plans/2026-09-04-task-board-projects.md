# 工作看板專案化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把扁平的全域工作看板改成「先建專案、再進看板」的兩層結構，其中每個專案是磁碟上自成一體的資料夾（`.aitprj` + 自己的 `tasks.db` + 附件與對話記錄）。

**Architecture:** `TasksDb` 這個持有單一連線池的 Tauri 單例，被 `ProjectRegistry`（`id → ProjectHandle{ id, path, name, pool }`）取代。`tasks.db` 的 schema 一個字都不改，`store.rs` 的每個函式原樣沿用——它們本來就只接收 `&SqlitePool`。排程器改為跨所有已開啟專案收集卡片，`pick_next` 這個純函式保持不變。前端 `TaskBoard/index.tsx` 從看板變成路由器，看板本體原封不動搬進 `ProjectBoard.tsx`。

**Tech Stack:** Rust / Tauri 2 / sqlx（SQLite）/ serde_json / uuid / tempfile；React 19 / TypeScript / Vitest / React Testing Library。

**Spec:** `docs/superpowers/specs/2026-09-04-task-board-projects-design.md`

---

## 實作前必讀

### 這份計畫依據的地面實情（已逐一讀過原始碼確認，非推測）

1. **`monitor.rs` 完全不碰資料庫。** 它是純 PTY 觀察器，只回傳 `TaskOutcome`。spec §4.6 說「監控器要改成向 registry 查詢連線池」是**錯的**，本計畫不照 spec 那節做。真正的資料庫寫回在 `scheduler.rs:106` 的 `store::finish_task`，位於 `RealDispatcher::dispatch` 內 `spawn` 出去的 async block 中。

2. **那個 async block 已經是「捕獲 pool 的複本」的寫法**（`scheduler.rs:93` 的 `let pool = db.pool.clone();`）。因此改成多專案後，watcher **不需要**任何 registry 查詢——只要在 dispatch 當下把該專案的 `pool` 和 `path` 一起捕獲進去即可。這讓 spec 預期的複雜度大幅下降。

3. **`commands/tasks.rs` 既有的三組單元測試（`tests`、`save_transcript_tests`、`mark_done_tests`）全部只操作裸的 `SqlitePool`**，刻意避開 Tauri 的 `State` 提取器（見該檔 386–388 行與 429–431 行的註解）。因此它們在本次重構後**不需要修改**。不要動它們。

4. **`store::list_by_status` 以 `sort_order` 排序**（`store.rs:346`），`list_tasks` 以 `status, sort_order` 排序。

5. **`drain_once` 有兩段迴圈**（`scheduler.rs:141` 與 `:162`）：第一段把所有 `interactive` 的 queued 卡片**無條件**派出去，完全繞過並行上限與獨佔規則；第二段才是受管制的一般卡片。這個行為必須跨專案完整保留。

### 與 spec 的兩處刻意偏離

1. **spec §4.6「監控器要改成向 registry 查詢連線池」不實作。** 理由如上第 1、2 點：`monitor.rs` 不碰資料庫，寫回發生在 dispatch 內 spawn 出去的 async block，那裡本來就捕獲 pool 的複本，只要 dispatch 時把該專案的 pool 與 path 一併捕獲即可。spec 那節是在讀原始碼之前依架構推測寫的，與實情不符。

2. **spec §4.4 的 `projects_used_dirs` 改名為 `tasks_used_dirs`，放在 `commands/tasks.rs`。** 它查的是 `tasks` 表，跟其他 `tasks_*` 指令一樣需要 `project_id`，放在專案指令裡反而不一致。

### 跨專案排隊順序的設計決定

`pick_next` 只看 `queued.first()`（嚴格優先序，絕不跳過卡片）。單一專案內的順序由使用者拖曳決定（`sort_order`），這個不能破壞。

因此跨專案的作法是：**每個專案各取自己佇列的第一張卡片（head），這些 head 之間再依 `created_at` 由舊到新排序，逐一嘗試**。

- 專案內部：嚴格優先序完整保留，拖曳排序照常生效。
- 專案之間：先到先得（依卡片建立時間）。
- 某專案的 head 因為是獨佔卡片而暫時不能啟動時，可以改試別的專案的 head——跨專案本來就是獨立佇列，跳過不違反「不跳過」的規則。
- **不可**把所有專案的 queued 卡片混在一起依 `sort_order` 排序：`sort_order` 是各專案獨立的浮點數，混排毫無意義，而且會破壞拖曳順序。

### 執行順序要求

**Task 1–10（後端）全部完成且 `cd src-tauri && cargo test` 全綠之後，才開始 Task 11（前端）。** 兩端同時破裂會非常難定位。

### 每個 Task 完成時的驗證指令

```bash
npx tsc -b                      # 型別檢查。不可用 tsc --noEmit：
                                # 根 tsconfig.json 是 solution file（"files": []），
                                # 什麼都不檢查而且永遠 exit 0
npm run test                    # 前端 Vitest
cd src-tauri && cargo test      # 完整 Rust 測試。不可只跑 --lib，
                                # 那不會編譯 tests/ 底下的整合測試
```

**注意：** `cargo test` / `cargo check` 在 `binaries/uv` 不存在時會直接失敗（`tauri-build` 的 `build.rs` 在編譯期驗證每個 `externalBin` 都在磁碟上）。開工前先跑對應平台的 `scripts/setup-uv-{mac,linux,win}.{sh,ps1}`。

---

## 檔案結構

### 新增

| 檔案 | 職責 |
|---|---|
| `src-tauri/src/projects/mod.rs` | `ProjectMeta`（`.aitprj` 內容）、`ProjectError`、`ProjectHandle`、`ProjectRegistry`。讀寫 `.aitprj`、開啟/建立/關閉專案 |
| `src-tauri/src/projects/naming.rs` | 專案名稱 → 安全資料夾名（Windows 非法字元與保留名稱） |
| `src-tauri/src/projects/migrate.rs` | 舊 `tasks.db` 的一次性搬遷 |
| `src-tauri/src/commands/projects.rs` | `projects_*` 指令 |
| `src/ipc/projects.ts` | 前端 IPC 包裝與型別 |
| `src/components/TaskBoard/ProjectList.tsx` | 專案總覽 |
| `src/components/TaskBoard/ProjectCreateDialog.tsx` | 建立專案對話框 |
| `src/components/TaskBoard/ProjectTabBar.tsx` | 專案分頁列 |
| `src/components/TaskBoard/ProjectBoard.tsx` | 四欄看板本體（從現行 `index.tsx` 搬入） |
| `src/components/TaskBoard/ProjectList.test.tsx` | 專案總覽測試 |
| `src/components/TaskBoard/ProjectTabBar.test.tsx` | 分頁列測試 |

### 修改

| 檔案 | 改什麼 |
|---|---|
| `src-tauri/src/tasks/mod.rs` | 刪除 `TasksDb`；`task_dir(task_id)` → `task_dir(project_path, task_id)`；`init_schema` 保持不變 |
| `src-tauri/src/tasks/store.rs` | 新增 `count_by_status`、`distinct_project_dirs`、`rewrite_stored_paths` |
| `src-tauri/src/tasks/scheduler.rs` | `Dispatcher` trait 改吃 `&ProjectHandle`；`drain_once` 跨專案；`write_transcript` 加 `project_path` 參數 |
| `src-tauri/src/commands/tasks.rs` | 12 個指令改用 `ProjectRegistry` + `project_id` |
| `src-tauri/src/config/types.rs` | `TaskBoardConfig` 加 `project_paths`；`max_concurrent` 預設 2 → 5 |
| `src-tauri/src/lib.rs` | 註冊 `ProjectRegistry` 取代 `TasksDb`、啟動時載入專案與搬遷、註冊 `projects_*` 指令 |
| `src-tauri/tests/task_board.rs` | 改用臨時專案 |
| `src/ipc/tasks.ts` | 每個函式加 `projectId` 參數 |
| `src/components/TaskBoard/index.tsx` | 變成路由器 |
| `src/components/TaskBoard/TaskEditorDialog.tsx` | 工作目錄快捷選項 |
| `src/components/TaskBoard/index.css` | 分頁列與專案總覽樣式 |
| `src/lib/i18n.ts` | 新增字串 |
| `src/components/TaskBoard/index.test.tsx` | 補上專案層 |
| `src/components/TerminalApp.taskBoard.test.tsx` | 更新 mock |

### 不修改（明確排除）

- `src-tauri/src/tasks/monitor.rs` —— 它不碰資料庫，本次重構完全不需要動它。
- `src-tauri/src/commands/tasks.rs` 的三組 `#[cfg(test)] mod`（`tests`、`save_transcript_tests`、`mark_done_tests`）—— 它們只用裸 `SqlitePool`，重構後仍然有效。
- `src-tauri/src/tasks/store.rs` 既有的每一個函式簽名 —— 它們只吃 `&SqlitePool`，天然就是 per-project 的。

---

## Task 1: `.aitprj` 專案檔的讀寫

**Files:**
- Create: `src-tauri/src/projects/mod.rs`
- Modify: `src-tauri/src/lib.rs`（只加 `pub mod projects;`）

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/projects/mod.rs` 末端新增：

```rust
#[cfg(test)]
mod meta_tests {
    use super::*;

    #[test]
    fn round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let meta = ProjectMeta::new("makemoney", "賺錢用");
        let path = dir.path().join("makemoney.aitprj");
        write_meta(&path, &meta).unwrap();

        let back = read_meta(&path).unwrap();
        assert_eq!(back.id, meta.id);
        assert_eq!(back.name, "makemoney");
        assert_eq!(back.description, "賺錢用");
        assert_eq!(back.schema, SCHEMA_VERSION);
    }

    #[test]
    fn a_missing_file_is_missing_not_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_meta(&dir.path().join("nope.aitprj")).unwrap_err();
        assert!(matches!(err, ProjectError::Missing), "{err:?}");
    }

    #[test]
    fn malformed_json_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.aitprj");
        std::fs::write(&path, "{ not json").unwrap();
        let err = read_meta(&path).unwrap_err();
        assert!(matches!(err, ProjectError::Invalid(_)), "{err:?}");
    }

    #[test]
    fn a_newer_schema_is_incompatible_not_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.aitprj");
        std::fs::write(
            &path,
            r#"{"schema":999,"id":"x","name":"n","description":"","created_at":"2026-09-04T00:00:00Z"}"#,
        )
        .unwrap();
        let err = read_meta(&path).unwrap_err();
        assert!(matches!(err, ProjectError::Incompatible(999)), "{err:?}");
    }

    #[test]
    fn finds_the_aitprj_inside_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.aitprj");
        write_meta(&path, &ProjectMeta::new("hello", "")).unwrap();
        assert_eq!(find_aitprj(dir.path()).unwrap(), path);
    }

    #[test]
    fn a_folder_with_no_aitprj_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let err = find_aitprj(dir.path()).unwrap_err();
        assert!(matches!(err, ProjectError::Missing), "{err:?}");
    }
}
```

- [ ] **Step 2: 執行測試，確認它失敗**

先在 `src-tauri/src/lib.rs` 的模組宣告區（`mod` 宣告集中處，`tasks` 那行附近）加入：

```rust
pub mod projects;
```

Run: `cd src-tauri && cargo test projects::meta_tests`
Expected: FAIL，編譯錯誤 `cannot find type ProjectMeta in this scope`（以及其他未定義項目）。

- [ ] **Step 3: 寫最小實作**

在 `src-tauri/src/projects/mod.rs` 頂端（測試模組之前）寫入：

```rust
//! 專案 = 磁碟上一個自成一體的資料夾：
//!   <folder>/<name>.aitprj    專案清單檔（本模組負責）
//!   <folder>/tasks.db         這個專案的卡片（schema 同 tasks::init_schema）
//!   <folder>/tasks/<id>/      附件與對話記錄
//!
//! 見 docs/superpowers/specs/2026-09-04-task-board-projects-design.md

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 本程式認得的 `.aitprj` 格式版本。讀到比這個大的版本一律拒絕，
/// 不猜測、不嘗試向前相容。
pub const SCHEMA_VERSION: u32 = 1;

/// `.aitprj` 的內容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub schema: u32,
    /// UUID，不是路徑——資料夾改名或搬家後仍是同一個專案。
    pub id: String,
    /// 顯示名稱，與資料夾名稱獨立。重新命名專案只改這裡。
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub created_at: String,
}

impl ProjectMeta {
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Debug)]
pub enum ProjectError {
    /// 資料夾或 `.aitprj` 不存在。
    Missing,
    /// `.aitprj` 存在但無法解析。
    Invalid(String),
    /// `.aitprj` 的 schema 版本高於本程式支援。
    Incompatible(u32),
    /// `tasks.db` 開啟或建立失敗。
    Db(String),
    /// 檔案系統操作失敗。
    Io(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::Missing => write!(f, "專案資料夾或專案檔不存在"),
            ProjectError::Invalid(m) => write!(f, "專案檔無法讀取：{m}"),
            ProjectError::Incompatible(v) => {
                write!(f, "專案檔格式版本 {v} 高於本版本支援的 {SCHEMA_VERSION}，請更新 AITerm")
            }
            ProjectError::Db(m) => write!(f, "專案資料庫錯誤：{m}"),
            ProjectError::Io(m) => write!(f, "檔案操作失敗：{m}"),
        }
    }
}

/// 供 `projects_list` 回報用的機器可讀狀態字串。
impl ProjectError {
    pub fn status_str(&self) -> &'static str {
        match self {
            ProjectError::Missing => "missing",
            ProjectError::Invalid(_) => "invalid",
            ProjectError::Incompatible(_) => "incompatible",
            ProjectError::Db(_) | ProjectError::Io(_) => "invalid",
        }
    }
}

pub fn read_meta(path: &Path) -> Result<ProjectMeta, ProjectError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ProjectError::Missing),
        Err(e) => return Err(ProjectError::Io(e.to_string())),
    };
    // 先只取 schema 欄位判斷版本：未來版本可能加了本版反序列化不了的欄位，
    // 那種情況要回報 Incompatible（可修正：叫使用者更新），
    // 不能回報 Invalid（看起來像檔案壞掉，會誤導使用者去刪檔）。
    let probe: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProjectError::Invalid(e.to_string()))?;
    let schema = probe.get("schema").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if schema > SCHEMA_VERSION {
        return Err(ProjectError::Incompatible(schema));
    }
    serde_json::from_str(&text).map_err(|e| ProjectError::Invalid(e.to_string()))
}

pub fn write_meta(path: &Path, meta: &ProjectMeta) -> Result<(), ProjectError> {
    let text = serde_json::to_string_pretty(meta).map_err(|e| ProjectError::Invalid(e.to_string()))?;
    std::fs::write(path, text).map_err(|e| ProjectError::Io(e.to_string()))
}

/// 找出資料夾裡的 `.aitprj`。多於一個時取檔名排序的第一個——
/// 這種情況只會發生在使用者手動複製檔案，取一個穩定的結果比報錯有用。
pub fn find_aitprj(folder: &Path) -> Result<PathBuf, ProjectError> {
    let entries = match std::fs::read_dir(folder) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ProjectError::Missing),
        Err(e) => return Err(ProjectError::Io(e.to_string())),
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("aitprj"))
        .collect();
    found.sort();
    found.into_iter().next().ok_or(ProjectError::Missing)
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test projects::meta_tests`
Expected: PASS，6 個測試全過。

若 `chrono` 尚未在 `src-tauri/Cargo.toml` 的 `[dependencies]` 中，先確認：

Run: `grep -n '^chrono' src-tauri/Cargo.toml`
若無輸出，在 `[dependencies]` 加入 `chrono = { version = "0.4", features = ["serde"] }` 後重跑。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/projects/mod.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(projects): .aitprj 專案檔的讀寫與錯誤分類"
```

---

## Task 2: 專案名稱轉安全資料夾名

**Files:**
- Create: `src-tauri/src/projects/naming.rs`
- Modify: `src-tauri/src/projects/mod.rs`（加 `pub mod naming;`）

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/projects/naming.rs` 寫入：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_an_already_safe_name() {
        assert_eq!(safe_folder_name("makemoney"), "makemoney");
        assert_eq!(safe_folder_name("賺錢計畫"), "賺錢計畫");
    }

    #[test]
    fn replaces_windows_illegal_characters() {
        assert_eq!(safe_folder_name("a/b\\c:d*e?f\"g<h>i|j"), "a-b-c-d-e-f-g-h-i-j");
    }

    #[test]
    fn escapes_windows_reserved_device_names_case_insensitively() {
        assert_eq!(safe_folder_name("CON"), "CON_");
        assert_eq!(safe_folder_name("con"), "con_");
        assert_eq!(safe_folder_name("COM1"), "COM1_");
        assert_eq!(safe_folder_name("lpt9"), "lpt9_");
        // 只是開頭像保留名稱的一般名稱不受影響
        assert_eq!(safe_folder_name("console"), "console");
    }

    #[test]
    fn trims_trailing_dots_and_spaces() {
        // Windows 無法建立以 . 或空白結尾的資料夾
        assert_eq!(safe_folder_name("name. "), "name");
        assert_eq!(safe_folder_name("  name  "), "name");
    }

    #[test]
    fn an_empty_or_all_illegal_name_falls_back() {
        assert_eq!(safe_folder_name(""), "project");
        assert_eq!(safe_folder_name("///"), "project");
        assert_eq!(safe_folder_name("..."), "project");
    }
}
```

- [ ] **Step 2: 執行測試，確認它失敗**

先在 `src-tauri/src/projects/mod.rs` 頂端的 `use` 之後加入：

```rust
pub mod naming;
```

Run: `cd src-tauri && cargo test projects::naming`
Expected: FAIL，`cannot find function safe_folder_name in this scope`。

- [ ] **Step 3: 寫最小實作**

在 `src-tauri/src/projects/naming.rs` 的測試模組之前寫入：

```rust
//! 專案顯示名稱 → 可安全建立的資料夾名稱。
//!
//! 一律套用 Windows 的規則，即使在 macOS/Linux 上也一樣——這樣同一個
//! 專案名在三個平台產生相同的資料夾名，專案資料夾複製到別台機器
//! 不會因為平台差異而變成兩個不同的名字。

/// Windows 保留的裝置名稱（不分大小寫，且含副檔名時同樣保留）。
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

pub fn safe_folder_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if (c as u32) < 0x20 => '-',
            c => c,
        })
        .collect();

    // Windows 無法建立以 '.' 或空白結尾的資料夾；開頭空白也一併去掉。
    let trimmed = replaced.trim().trim_end_matches(['.', ' ']).trim();

    if trimmed.is_empty() || trimmed.chars().all(|c| c == '-') {
        return "project".to_string();
    }

    // 保留裝置名稱：整個名稱（或副檔名前的部分）等於保留字時加底線。
    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return format!("{trimmed}_");
    }

    trimmed.to_string()
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test projects::naming`
Expected: PASS，5 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/projects/naming.rs src-tauri/src/projects/mod.rs
git commit -m "feat(projects): 專案名稱轉安全資料夾名（跨平台一致）"
```

---

## Task 3: `ProjectHandle` 與 `ProjectRegistry`

**Files:**
- Modify: `src-tauri/src/projects/mod.rs`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/projects/mod.rs` 末端新增：

```rust
#[cfg(test)]
mod registry_tests {
    use super::*;

    #[tokio::test]
    async fn create_then_get_returns_the_same_project() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();

        let handle = reg.create(parent.path(), "makemoney", "賺錢").await.unwrap();
        assert_eq!(handle.name, "makemoney");
        assert!(handle.path.join("makemoney.aitprj").is_file());
        assert!(handle.path.join("tasks.db").is_file());

        let again = reg.get(&handle.id).unwrap();
        assert_eq!(again.id, handle.id);
        assert_eq!(again.path, handle.path);
    }

    #[tokio::test]
    async fn the_new_projects_db_has_the_tasks_schema() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let handle = reg.create(parent.path(), "p", "").await.unwrap();

        // 能建立卡片就證明 init_schema 跑過了
        let id = crate::tasks::store::create_task(&handle.pool, "t", "b", "/r", true, false)
            .await
            .unwrap();
        let rows = crate::tasks::store::list_tasks(&handle.pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
    }

    #[tokio::test]
    async fn creating_into_an_existing_non_empty_folder_is_rejected() {
        let parent = tempfile::tempdir().unwrap();
        std::fs::create_dir(parent.path().join("taken")).unwrap();
        std::fs::write(parent.path().join("taken").join("something.txt"), "x").unwrap();

        let reg = ProjectRegistry::new();
        let err = reg.create(parent.path(), "taken", "").await.unwrap_err();
        assert!(matches!(err, ProjectError::Io(_)), "{err:?}");
    }

    #[tokio::test]
    async fn open_folder_loads_an_existing_project() {
        let parent = tempfile::tempdir().unwrap();
        let created = {
            let reg = ProjectRegistry::new();
            reg.create(parent.path(), "reopen", "").await.unwrap()
        };

        // 全新的 registry（模擬重新啟動）
        let reg2 = ProjectRegistry::new();
        let opened = reg2.open_folder(&created.path).await.unwrap();
        assert_eq!(opened.id, created.id);
        assert_eq!(opened.name, "reopen");
    }

    #[tokio::test]
    async fn open_folder_on_a_missing_folder_is_missing() {
        let reg = ProjectRegistry::new();
        let err = reg
            .open_folder(std::path::Path::new("/definitely/not/here"))
            .await
            .unwrap_err();
        assert!(matches!(err, ProjectError::Missing), "{err:?}");
    }

    #[tokio::test]
    async fn close_removes_it_from_the_registry_but_leaves_the_folder() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let handle = reg.create(parent.path(), "keepme", "").await.unwrap();

        reg.close(&handle.id);
        assert!(reg.get(&handle.id).is_none());
        assert!(handle.path.join("keepme.aitprj").is_file(), "關閉不可刪檔");
    }

    #[tokio::test]
    async fn all_lists_every_open_project() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        reg.create(parent.path(), "a", "").await.unwrap();
        reg.create(parent.path(), "b", "").await.unwrap();
        let mut names: Vec<String> = reg.all().into_iter().map(|h| h.name).collect();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }
}
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `cd src-tauri && cargo test projects::registry_tests`
Expected: FAIL，`cannot find type ProjectRegistry in this scope`。

- [ ] **Step 3: 寫最小實作**

在 `src-tauri/src/projects/mod.rs` 的 `find_aitprj` 之後、測試模組之前寫入：

```rust
use std::collections::HashMap;

use parking_lot::RwLock;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

/// 一個已開啟的專案。`pool` 與 `path` 都便宜可複製，
/// 所以這個型別直接 `Clone` 出去給呼叫端持有，
/// 不需要在每個使用點回頭查 registry。
// 實作時已確認 sqlx 0.8.6 的 Pool<DB> 本身就有 Debug
// （sqlx-core-0.8.6/src/pool/mod.rs:579），所以直接 derive 即可，
// 不需要手寫 impl。
#[derive(Debug, Clone)]
pub struct ProjectHandle {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub pool: SqlitePool,
}

/// 目前開啟中的專案。由 Tauri `.manage()` 持有，取代舊的 `TasksDb` 單例。
#[derive(Default)]
pub struct ProjectRegistry {
    open: RwLock<HashMap<String, ProjectHandle>>,
}

impl ProjectRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<ProjectHandle> {
        self.open.read().get(id).cloned()
    }

    pub fn all(&self) -> Vec<ProjectHandle> {
        self.open.read().values().cloned().collect()
    }

    /// 從 registry 移除。**不刪磁碟上的任何東西。**
    pub fn close(&self, id: &str) {
        self.open.write().remove(id);
    }

    /// 建立新專案資料夾並開啟。`parent` 必須已存在。
    pub async fn create(
        &self,
        parent: &Path,
        name: &str,
        description: &str,
    ) -> Result<ProjectHandle, ProjectError> {
        let folder_name = naming::safe_folder_name(name);
        let folder = parent.join(&folder_name);

        // 已存在且非空 → 拒絕。空資料夾則可以直接用（使用者可能先建好了）。
        if folder.exists() {
            let empty = std::fs::read_dir(&folder)
                .map_err(|e| ProjectError::Io(e.to_string()))?
                .next()
                .is_none();
            if !empty {
                return Err(ProjectError::Io(format!("資料夾已存在且不是空的：{}", folder.display())));
            }
        }
        std::fs::create_dir_all(&folder).map_err(|e| ProjectError::Io(e.to_string()))?;

        let meta = ProjectMeta::new(name, description);
        write_meta(&folder.join(format!("{folder_name}.aitprj")), &meta)?;

        let pool = open_pool(&folder).await?;
        let handle = ProjectHandle { id: meta.id.clone(), name: meta.name, path: folder, pool };
        self.open.write().insert(handle.id.clone(), handle.clone());
        Ok(handle)
    }

    /// 開啟既有專案資料夾。已開啟時直接回傳現有的 handle。
    pub async fn open_folder(&self, folder: &Path) -> Result<ProjectHandle, ProjectError> {
        let meta_path = find_aitprj(folder)?;
        let meta = read_meta(&meta_path)?;

        if let Some(existing) = self.get(&meta.id) {
            return Ok(existing);
        }

        let pool = open_pool(folder).await?;
        let handle = ProjectHandle {
            id: meta.id.clone(),
            name: meta.name,
            path: folder.to_path_buf(),
            pool,
        };
        self.open.write().insert(handle.id.clone(), handle.clone());
        Ok(handle)
    }
}

/// 開啟（必要時建立）專案資料夾裡的 `tasks.db` 並確保 schema 存在。
///
/// 刻意**不**沿用舊 `TasksDb::new()` 那個「開啟失敗就換成
/// `sqlite::memory:`」的回退——那會靜默吃掉使用者的卡片。
/// 這裡失敗就是失敗，交給呼叫端顯示錯誤。
async fn open_pool(folder: &Path) -> Result<SqlitePool, ProjectError> {
    let options = SqliteConnectOptions::new()
        .filename(folder.join("tasks.db"))
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options)
        .await
        .map_err(|e| ProjectError::Db(e.to_string()))?;
    crate::tasks::init_schema(&pool)
        .await
        .map_err(|e| ProjectError::Db(e.to_string()))?;
    Ok(pool)
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test projects::registry_tests`
Expected: PASS，7 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/projects/mod.rs
git commit -m "feat(projects): ProjectRegistry — 每專案一個連線池，取代 TasksDb 單例"
```

---

## Task 4: store 層的三個新查詢

**Files:**
- Modify: `src-tauri/src/tasks/store.rs`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/tasks/store.rs` 末端新增（若檔案已有 `#[cfg(test)] mod tests`，把這些函式加進去；若無，新增整個模組）：

```rust
#[cfg(test)]
mod project_query_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn count_by_status_counts_only_that_status() {
        let pool = mem_pool().await;
        let a = create_task(&pool, "a", "", "/r", true, false).await.unwrap();
        create_task(&pool, "b", "", "/r", true, false).await.unwrap();
        move_task(&pool, &a, STATUS_QUEUED, 1.0).await.unwrap();

        assert_eq!(count_by_status(&pool, STATUS_PLANNING).await.unwrap(), 1);
        assert_eq!(count_by_status(&pool, STATUS_QUEUED).await.unwrap(), 1);
        assert_eq!(count_by_status(&pool, STATUS_RUNNING).await.unwrap(), 0);
        assert_eq!(count_by_status(&pool, STATUS_DONE).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn distinct_project_dirs_dedupes_and_sorts() {
        let pool = mem_pool().await;
        create_task(&pool, "a", "", "/b/api", true, false).await.unwrap();
        create_task(&pool, "b", "", "/a/web", true, false).await.unwrap();
        create_task(&pool, "c", "", "/b/api", true, false).await.unwrap();

        let dirs = distinct_project_dirs(&pool).await.unwrap();
        assert_eq!(dirs, vec!["/a/web".to_string(), "/b/api".to_string()]);
    }

    #[tokio::test]
    async fn distinct_project_dirs_skips_empty_strings() {
        let pool = mem_pool().await;
        create_task(&pool, "a", "", "", true, false).await.unwrap();
        create_task(&pool, "b", "", "/real", true, false).await.unwrap();
        assert_eq!(distinct_project_dirs(&pool).await.unwrap(), vec!["/real".to_string()]);
    }

    #[tokio::test]
    async fn rewrite_stored_paths_repoints_attachments_and_transcripts() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "a", "", "/r", true, false).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        mark_dispatched(&pool, &id, "tab").await.unwrap();
        finish_task(&pool, &id, "success", None, Some("/old/home/tasks/x/transcript.txt"))
            .await
            .unwrap();
        add_attachment(&pool, &id, "f.png", "/old/home/tasks/x/attachments/f.png")
            .await
            .unwrap();

        rewrite_stored_paths(&pool, "/old/home", "/new/home").await.unwrap();

        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.transcript_path.as_deref(), Some("/new/home/tasks/x/transcript.txt"));
        let atts = list_attachments(&pool, &id).await.unwrap();
        assert_eq!(atts[0].stored_path, "/new/home/tasks/x/attachments/f.png");
    }

    #[tokio::test]
    async fn rewrite_stored_paths_leaves_unrelated_paths_alone() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "a", "", "/r", true, false).await.unwrap();
        add_attachment(&pool, &id, "f.png", "/somewhere/else/f.png").await.unwrap();

        rewrite_stored_paths(&pool, "/old/home", "/new/home").await.unwrap();

        let atts = list_attachments(&pool, &id).await.unwrap();
        assert_eq!(atts[0].stored_path, "/somewhere/else/f.png");
    }
}
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `cd src-tauri && cargo test store::project_query_tests`
Expected: FAIL，`cannot find function count_by_status in this scope`。

- [ ] **Step 3: 寫最小實作**

在 `src-tauri/src/tasks/store.rs` 的 `list_by_status` 之後加入：

```rust
/// 某個 status 目前有幾張卡片。供 `projects_list` 產生專案總覽的計數。
pub async fn count_by_status(pool: &SqlitePool, status: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE status = ?")
        .bind(status)
        .fetch_one(pool)
        .await
}

/// 這個專案的卡片用過的工作目錄，去重後依字母排序。
/// 供新增工作時的目錄快捷選項——專案不綁資料夾，工作可散布在多個 repo，
/// 沒有這個的話使用者每次都得重新瀏覽選取。
pub async fn distinct_project_dirs(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT project_dir FROM tasks WHERE project_dir <> '' ORDER BY project_dir",
    )
    .fetch_all(pool)
    .await
}

/// 把 `transcript_path` 與附件的 `stored_path` 中的 `old_prefix` 換成
/// `new_prefix`。搬遷舊資料時用——那些欄位存的是絕對路徑，複製資料夾
/// 之後若不改寫，新的專案資料夾就不是自成一體的（複製到別台機器會
/// 掉附件）。只換開頭相符的，其他路徑不動。
pub async fn rewrite_stored_paths(
    pool: &SqlitePool,
    old_prefix: &str,
    new_prefix: &str,
) -> Result<(), sqlx::Error> {
    let like = format!("{old_prefix}%");
    sqlx::query(
        "UPDATE tasks SET transcript_path = ? || SUBSTR(transcript_path, ?)
         WHERE transcript_path LIKE ?",
    )
    .bind(new_prefix)
    .bind(old_prefix.len() as i64 + 1)
    .bind(&like)
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE task_attachments SET stored_path = ? || SUBSTR(stored_path, ?)
         WHERE stored_path LIKE ?",
    )
    .bind(new_prefix)
    .bind(old_prefix.len() as i64 + 1)
    .bind(&like)
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test store::project_query_tests`
Expected: PASS，5 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/store.rs
git commit -m "feat(tasks): store 層新增計數、目錄去重、路徑改寫三個查詢"
```

---

## Task 5: 設定：專案路徑清單與並行預設值

**Files:**
- Modify: `src-tauri/src/config/types.rs:188`、`:198-199`、`:209`、`:884`、`:891`、`:898`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/config/types.rs` 現有測試模組末端新增：

```rust
    #[test]
    fn task_board_defaults_to_five_concurrent() {
        let c = TaskBoardConfig::default();
        assert_eq!(c.max_concurrent, 5);
    }

    #[test]
    fn task_board_project_paths_defaults_to_empty() {
        let c = TaskBoardConfig::default();
        assert!(c.project_paths.is_empty());
    }

    #[test]
    fn a_config_written_before_project_paths_existed_still_parses() {
        // 舊設定檔沒有 project_paths 欄位——必須不報錯，補成空陣列
        let json = r#"{"max_concurrent":3,"claude_command":"claude"}"#;
        let c: TaskBoardConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.max_concurrent, 3);
        assert!(c.project_paths.is_empty());
    }
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `cd src-tauri && cargo test config::types::tests::task_board`
Expected: FAIL — `task_board_defaults_to_five_concurrent` 得到 2 而非 5；另兩個測試編譯失敗（`no field project_paths`）。

- [ ] **Step 3: 寫最小實作**

`src-tauri/src/config/types.rs:188`，把：

```rust
pub fn default_task_board_max_concurrent() -> u32 { 2 }
```

改為：

```rust
pub fn default_task_board_max_concurrent() -> u32 { 5 }
```

在 `TaskBoardConfig` 的 struct 定義中（`:198-199` 的 `max_concurrent` 欄位之後）加入：

```rust
    /// 已知專案資料夾的絕對路徑。只存路徑——名稱與 id 每次啟動時
    /// 從各自的 `.aitprj` 讀取，這樣使用者在 Finder 裡改了專案檔，
    /// App 下次啟動就會看到。
    #[serde(default)]
    pub project_paths: Vec<String>,
```

在 `Default for TaskBoardConfig` 的實作中（`:209` 的 `max_concurrent` 那行之後）加入：

```rust
            project_paths: Vec::new(),
```

把 `:884`、`:891`、`:898` 三處的 `assert_eq!(..., 2)` 改成 `assert_eq!(..., 5)`（這三個斷言原本就在測預設值，改預設值時它們理應一起改）。

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test config::types`
Expected: PASS，含新加的 3 個測試與改過的 3 個既有斷言。

**注意：** `commands/task_board_config.rs:24` 的 `clamp(1, 16)` **不要改**。使用者確認上限維持可調到 16，只有預設值變成 5。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config/types.rs
git commit -m "feat(config): 專案路徑清單，並行預設 2 → 5"
```

---

## Task 6: 舊資料搬遷

**Files:**
- Create: `src-tauri/src/projects/migrate.rs`
- Modify: `src-tauri/src/projects/mod.rs`（加 `pub mod migrate;`）

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/projects/migrate.rs` 寫入：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::store;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::SqlitePool;

    /// 造一份「舊版」資料區：tasks.db 裡有一張卡片，
    /// 附件與對話記錄放在 <root>/tasks/<id>/ 底下，
    /// 而且資料庫裡存的是它們的絕對路徑（就跟真實情況一樣）。
    async fn legacy_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let db_path = root.path().join("tasks.db");
        let options = SqliteConnectOptions::new().filename(&db_path).create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();

        let id = store::create_task(&pool, "舊卡片", "body", "/repo", true, false).await.unwrap();
        let task_dir = root.path().join("tasks").join(&id);
        std::fs::create_dir_all(task_dir.join("attachments")).unwrap();
        let att = task_dir.join("attachments").join("f.txt");
        std::fs::write(&att, "attachment content").unwrap();
        store::add_attachment(&pool, &id, "f.txt", att.to_str().unwrap()).await.unwrap();

        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&pool, &id, "tab").await.unwrap();
        let transcript = task_dir.join("transcript.txt");
        std::fs::write(&transcript, "transcript content").unwrap();
        store::finish_task(&pool, &id, "success", None, transcript.to_str().unwrap().into())
            .await
            .unwrap();

        pool.close().await;
        root
    }

    #[tokio::test]
    async fn copies_cards_files_and_repoints_paths() {
        let root = legacy_root().await;
        let dest = migrate_legacy(root.path()).await.unwrap().expect("應該有搬遷");

        // 卡片進來了
        let options = SqliteConnectOptions::new().filename(dest.join("tasks.db"));
        let pool = SqlitePool::connect_with(options).await.unwrap();
        let rows = store::list_tasks(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "舊卡片");

        // 路徑改寫成新家，而且檔案真的在那裡（＝資料夾是自成一體的）
        let transcript = rows[0].transcript_path.clone().unwrap();
        assert!(transcript.starts_with(dest.to_str().unwrap()), "{transcript}");
        assert_eq!(std::fs::read_to_string(&transcript).unwrap(), "transcript content");

        let atts = store::list_attachments(&pool, &rows[0].id).await.unwrap();
        assert!(atts[0].stored_path.starts_with(dest.to_str().unwrap()));
        assert_eq!(std::fs::read_to_string(&atts[0].stored_path).unwrap(), "attachment content");
    }

    #[tokio::test]
    async fn leaves_the_original_files_untouched() {
        let root = legacy_root().await;
        migrate_legacy(root.path()).await.unwrap().unwrap();
        assert!(root.path().join("tasks.db").is_file(), "舊 db 必須保留作為備份");
        assert!(root.path().join("tasks").is_dir(), "舊 tasks/ 必須保留");
    }

    #[tokio::test]
    async fn writes_a_marker_and_does_not_run_twice() {
        let root = legacy_root().await;
        assert!(migrate_legacy(root.path()).await.unwrap().is_some());
        assert!(root.path().join(MARKER).is_file());
        assert!(
            migrate_legacy(root.path()).await.unwrap().is_none(),
            "第二次呼叫不可再搬一次"
        );
    }

    #[tokio::test]
    async fn does_nothing_when_there_is_no_legacy_db() {
        let root = tempfile::tempdir().unwrap();
        assert!(migrate_legacy(root.path()).await.unwrap().is_none());
        assert!(root.path().join(MARKER).is_file(), "沒有舊資料也要留標記，避免每次啟動重掃");
    }

    #[tokio::test]
    async fn does_nothing_when_the_legacy_db_is_empty() {
        let root = tempfile::tempdir().unwrap();
        let options = SqliteConnectOptions::new()
            .filename(root.path().join("tasks.db"))
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool.close().await;

        assert!(migrate_legacy(root.path()).await.unwrap().is_none());
    }
}
```

- [ ] **Step 2: 執行測試，確認它失敗**

先在 `src-tauri/src/projects/mod.rs` 的 `pub mod naming;` 之後加入：

```rust
pub mod migrate;
```

Run: `cd src-tauri && cargo test projects::migrate`
Expected: FAIL，`cannot find function migrate_legacy in this scope`。

- [ ] **Step 3: 寫最小實作**

在 `src-tauri/src/projects/migrate.rs` 的測試模組之前寫入：

```rust
//! 一次性搬遷：把專案化之前那份全域 `tasks.db` 的卡片，
//! 連同附件與對話記錄，複製成一個名為「預設專案」的專案資料夾。
//!
//! 刻意用**複製**而非搬移：搬遷若有 bug，使用者的原始資料仍在原地。

use std::path::{Path, PathBuf};

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

use super::{write_meta, ProjectError, ProjectMeta};
use crate::tasks::store;

/// 搬遷完成標記。放在磁碟上而非設定檔裡——設定檔被重置時
/// 不該導致資料被重複搬遷一次。
pub const MARKER: &str = ".projects_migrated";

const DEFAULT_PROJECT_NAME: &str = "預設專案";

/// 若 `root`（＝ `tasks::app_data_dir()`）底下有尚未搬遷的舊資料，
/// 搬成一個專案資料夾並回傳它的路徑。已搬過或沒有舊資料時回傳 `None`。
pub async fn migrate_legacy(root: &Path) -> Result<Option<PathBuf>, ProjectError> {
    let marker = root.join(MARKER);
    if marker.exists() {
        return Ok(None);
    }

    let legacy_db = root.join("tasks.db");
    if !legacy_db.is_file() {
        touch_marker(&marker)?;
        return Ok(None);
    }

    // 舊 db 有卡片嗎？沒有的話不值得建一個空專案。
    let count = {
        let options = SqliteConnectOptions::new().filename(&legacy_db);
        let pool = SqlitePool::connect_with(options)
            .await
            .map_err(|e| ProjectError::Db(e.to_string()))?;
        let n = store::count_all(&pool).await.map_err(|e| ProjectError::Db(e.to_string()))?;
        pool.close().await;
        n
    };
    if count == 0 {
        touch_marker(&marker)?;
        return Ok(None);
    }

    let folder_name = super::naming::safe_folder_name(DEFAULT_PROJECT_NAME);
    let dest = root.join("projects").join(&folder_name);
    std::fs::create_dir_all(&dest).map_err(|e| ProjectError::Io(e.to_string()))?;

    std::fs::copy(&legacy_db, dest.join("tasks.db")).map_err(|e| ProjectError::Io(e.to_string()))?;
    let legacy_tasks = root.join("tasks");
    if legacy_tasks.is_dir() {
        copy_dir_all(&legacy_tasks, &dest.join("tasks")).map_err(|e| ProjectError::Io(e.to_string()))?;
    }

    // 資料庫裡存的是絕對路徑，指向舊位置。不改寫的話這個專案資料夾
    // 就不是自成一體的——複製到別台機器會掉附件與對話記錄。
    {
        let options = SqliteConnectOptions::new().filename(dest.join("tasks.db"));
        let pool = SqlitePool::connect_with(options)
            .await
            .map_err(|e| ProjectError::Db(e.to_string()))?;
        store::rewrite_stored_paths(
            &pool,
            &root.to_string_lossy(),
            &dest.to_string_lossy(),
        )
        .await
        .map_err(|e| ProjectError::Db(e.to_string()))?;
        pool.close().await;
    }

    let meta = ProjectMeta::new(DEFAULT_PROJECT_NAME, "專案功能上線前既有的工作");
    write_meta(&dest.join(format!("{folder_name}.aitprj")), &meta)?;

    touch_marker(&marker)?;
    Ok(Some(dest))
}

fn touch_marker(marker: &Path) -> Result<(), ProjectError> {
    std::fs::write(marker, "").map_err(|e| ProjectError::Io(e.to_string()))
}

/// std 沒有遞迴複製目錄的函式，自己寫一個。
fn copy_dir_all(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}
```

同時在 `src-tauri/src/tasks/store.rs` 的 `count_by_status` 之後加入搬遷用的總數查詢：

```rust
/// 全部卡片數，不分 status。搬遷時用來判斷舊資料庫是否值得搬。
pub async fn count_all(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM tasks").fetch_one(pool).await
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test projects::migrate`
Expected: PASS，5 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/projects/migrate.rs src-tauri/src/projects/mod.rs src-tauri/src/tasks/store.rs
git commit -m "feat(projects): 舊 tasks.db 一次性搬遷成「預設專案」資料夾"
```

---

## Task 7: `task_dir` 改為以專案資料夾為根

**Files:**
- Modify: `src-tauri/src/tasks/mod.rs:34`
- Modify: `src-tauri/src/tasks/scheduler.rs:119-134`
- Modify: `src-tauri/src/commands/tasks.rs:220`、`:245`、`:304`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/tasks/mod.rs` 末端新增：

```rust
#[cfg(test)]
mod task_dir_tests {
    use super::*;

    #[test]
    fn is_rooted_at_the_project_folder() {
        let project = std::path::Path::new("/projects/makemoney");
        assert_eq!(task_dir(project, "abc"), project.join("tasks").join("abc"));
    }
}
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `cd src-tauri && cargo test tasks::task_dir_tests`
Expected: FAIL，`this function takes 1 argument but 2 arguments were supplied`。

- [ ] **Step 3: 寫最小實作**

`src-tauri/src/tasks/mod.rs:34`，把：

```rust
/// `<data-dir>/AITERM/tasks/<task_id>` — per-task scratch dir holding
/// `attachments/` and `transcript.txt`. Created lazily by dispatch/store.
pub fn task_dir(task_id: &str) -> PathBuf {
    app_data_dir().join("tasks").join(task_id)
}
```

改為：

```rust
/// `<project-folder>/tasks/<task_id>` — 這張卡片的附件與對話記錄。
/// 由 dispatch/store 在需要時建立。
///
/// 根目錄是**專案資料夾**而非全域資料區——專案資料夾必須自成一體，
/// 這樣複製走就等於匯出。
pub fn task_dir(project_path: &Path, task_id: &str) -> PathBuf {
    project_path.join("tasks").join(task_id)
}
```

同時把該檔頂端的 `use std::path::PathBuf;` 改成 `use std::path::{Path, PathBuf};`。

`src-tauri/src/tasks/scheduler.rs:119`，把 `write_transcript` 的簽名與內容改為：

```rust
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
```

`commands/tasks.rs` 的三個呼叫點暫時無法編譯（它們還沒有 project path），這是預期的——Task 8 會一併改掉。本步驟只要 `tasks::mod` 與 `scheduler` 的簽名正確即可。

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test tasks::task_dir_tests`
Expected: 這一步 **會因為 `commands/tasks.rs` 尚未更新而編譯失敗**。這是計畫預期的中間狀態——`task_dir` 的簽名變更必然同時影響指令層，兩者無法分成獨立可編譯的兩步。**直接進入 Task 8，不要為了讓這步編譯過而寫暫時性的權宜程式碼。** Task 8 結束時會一起驗證。

- [ ] **Step 5: 不 commit**

本 Task 與 Task 8 是同一個不可分割的變更，合併在 Task 8 結束時一起 commit。

---

## Task 8: 指令層改用 `ProjectRegistry`

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs`（12 個指令）

- [ ] **Step 1: 加入 registry 取用的輔助函式**

在 `src-tauri/src/commands/tasks.rs` 的 `emit_updated` 之後加入：

```rust
use crate::projects::{ProjectHandle, ProjectRegistry};

/// 從 registry 取出專案。找不到時回傳給前端的錯誤訊息——
/// 這在正常使用下不會發生（前端只會送出 `projects_list` 給過的 id），
/// 會發生代表專案在操作進行中被移除了。
fn project(reg: &ProjectRegistry, id: &str) -> Result<ProjectHandle, String> {
    reg.get(id).ok_or_else(|| format!("專案不存在或已關閉：{id}"))
}
```

並移除 `use crate::tasks::{task_dir, TasksDb};`，改為：

```rust
use crate::tasks::task_dir;
```

- [ ] **Step 2: 逐一改寫 12 個指令**

每個指令的改法一致：把 `db: State<'_, TasksDb>` 換成 `project_id: String, reg: State<'_, ProjectRegistry>`，在函式開頭取出 handle，其後所有 `&db.pool` 改成 `&p.pool`。

**`tasks_list`**（`:32`）：

```rust
#[tauri::command]
pub async fn tasks_list(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<TaskWithAttachments>, String> {
    let p = project(&reg, &project_id)?;
    let tasks = store::list_tasks(&p.pool).await.map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        let attachments = store::list_attachments(&p.pool, &task.id)
            .await
            .map_err(|e| e.to_string())?;
        out.push(TaskWithAttachments { task, attachments });
    }
    Ok(out)
}
```

**`tasks_create`**（`:54`）：簽名改為

```rust
pub async fn tasks_create(
    project_id: String,
    args: CreateArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
```

其後 `&db.pool` → `&p.pool`。

**`tasks_update`**（`:84`）、**`tasks_move`**（`:122`）、**`tasks_stop`**（`:139`）、**`tasks_mark_done`**（`:166`）、**`tasks_remove_attachment`**（`:266`）、**`tasks_read_transcript`**（`:327`）、**`tasks_save_transcript`**（`:339`）：同樣套用——`project_id: String` 加在參數列最前，`db: State<'_, TasksDb>` 換成 `reg: State<'_, ProjectRegistry>`，函式體第一行加 `let p = project(&reg, &project_id)?;`，其餘 `&db.pool` 全換成 `&p.pool`。

**`tasks_delete`**（`:199`）：除上述外，`:220` 那行改為

```rust
    let _ = fs::remove_dir_all(task_dir(&p.path, &args.id));
```

**`tasks_add_attachment`**（`:233`）：除上述外，`:245` 那行改為

```rust
    let dir = task_dir(&p.path, &args.id).join("attachments");
```

**`tasks_clone`**（`:293`）：除上述外，`:304` 那行改為

```rust
    let dir = task_dir(&p.path, &new_id).join("attachments");
```

- [ ] **Step 3: 新增「工作目錄快捷選項」指令**

在 `tasks_save_transcript` 之後加入：

```rust
/// 這個專案的卡片用過的工作目錄。專案不綁資料夾（工作可散布在多個
/// repo），這個清單讓新增工作時不必每次重新瀏覽選取。
#[tauri::command]
pub async fn tasks_used_dirs(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<String>, String> {
    let p = project(&reg, &project_id)?;
    store::distinct_project_dirs(&p.pool).await.map_err(|e| e.to_string())
}
```

- [ ] **Step 4: 確認既有測試模組未被更動**

`commands/tasks.rs` 底部的三個 `#[cfg(test)] mod`（`tests`、`save_transcript_tests`、`mark_done_tests`）**完全不需要修改**——它們只操作裸的 `SqlitePool`，刻意避開了 Tauri 的 `State` 提取器（見該檔 386–388 與 429–431 行的註解）。

Run: `cd src-tauri && cargo test commands::tasks`
Expected: 這一步仍會編譯失敗，因為 `scheduler.rs` 與 `lib.rs` 還在用 `TasksDb`。Task 9、10 會補完。**不要為了讓這步過而改測試。**

- [ ] **Step 5: 不 commit**

與 Task 9、10 一起在 Task 10 結束時 commit。

---

## Task 9: 排程器跨專案

**Files:**
- Modify: `src-tauri/src/tasks/scheduler.rs:16`、`:48-50`、`:66-114`、`:138-187`、`:228-262`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/tasks/scheduler.rs` 既有 `#[cfg(test)] mod tests` 末端新增：

```rust
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
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `cd src-tauri && cargo test scheduler::tests::cross_project`
Expected: FAIL，`cannot find function order_heads in this scope`。

- [ ] **Step 3: 寫最小實作**

`src-tauri/src/tasks/scheduler.rs:16`，把：

```rust
use crate::tasks::{dispatch, monitor, TasksDb};
```

改為：

```rust
use crate::projects::{ProjectHandle, ProjectRegistry};
use crate::tasks::{dispatch, monitor};
```

在 `pick_next` 之後加入排序輔助函式：

```rust
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
```

把 `Dispatcher` trait（`:48-50`）改為：

```rust
#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync {
    async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String>;
}
```

把 `RealDispatcher` 的 `dispatch` 實作（`:66-114`）改為（只列出變動處，其餘保持原樣）：

```rust
    async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
        let attachments = store::list_attachments(&project.pool, &task.id)
```

`:85` 的 `store::mark_dispatched(&db.pool, ...)` → `store::mark_dispatched(&project.pool, ...)`。

`:93` 之後的捕獲區塊改為：

```rust
        let pool = project.pool.clone();
        let project_path = project.path.clone();
        let pty = self.pty.clone();
```

`:105` 那行改為：

```rust
            let transcript = write_transcript(&pty, &project_path, &task_id, &tab_id);
```

把 `drain_once`（`:138-187`）整個換成：

```rust
/// Promote as many queued cards as the rules allow, right now. Shared by the
/// loop and by tests. 跨所有已開啟的專案運作。
pub async fn drain_once(reg: &ProjectRegistry, dispatcher: &dyn Dispatcher, max_concurrent: u32) {
    let projects = reg.all();

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
            if let Err(e) = dispatcher.dispatch(project, &next).await {
                eprintln!("dispatch {} failed: {e}", next.id);
                let _ = store::mark_dispatched(&project.pool, &next.id, "").await;
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
            if let Err(e) = dispatcher.dispatch(&project, &head).await {
                eprintln!("dispatch {} failed: {e}", head.id);
                let _ = store::mark_dispatched(&project.pool, &head.id, "").await;
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
```

把 `spawn` 中的啟動復原與主迴圈（`:232-262`）改為：

```rust
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
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test scheduler`
Expected: 這一步仍可能因 `lib.rs` 尚未更新而編譯失敗。若 `lib.rs` 已在 Task 10 更新則應全過。**依序做完 Task 10 再驗證。**

- [ ] **Step 5: 不 commit**

與 Task 10 一起 commit。

---

## Task 10: `lib.rs` 接線與整合測試改寫

**Files:**
- Modify: `src-tauri/src/lib.rs:169`、`:207`、指令註冊區
- Create: `src-tauri/src/commands/projects.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/tests/task_board.rs`

- [ ] **Step 1: 寫 `projects_*` 指令**

建立 `src-tauri/src/commands/projects.rs`：

```rust
//! 專案的 Tauri 指令。`projects_list` 每次都重新掃描設定裡的路徑清單，
//! 這樣使用者在 Finder 裡刪掉或改動專案資料夾，重新整理就會反映出來。

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::config::ConfigStore;
use crate::projects::{find_aitprj, read_meta, ProjectError, ProjectRegistry};
use crate::tasks::store;

#[derive(Serialize)]
pub struct ProjectCounts {
    pub planning: i64,
    pub queued: i64,
    pub running: i64,
    pub done: i64,
}

#[derive(Serialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: String,
    /// `ok` | `missing` | `invalid` | `incompatible`
    pub status: String,
    pub counts: ProjectCounts,
    pub error: Option<String>,
}

fn zero_counts() -> ProjectCounts {
    ProjectCounts { planning: 0, queued: 0, running: 0, done: 0 }
}

fn emit_updated(app: &AppHandle) {
    let _ = app.emit("tasks-updated", ());
}

fn paths(config: &ConfigStore) -> Vec<String> {
    config.get().task_board.project_paths
}

fn set_paths(config: &ConfigStore, next: Vec<String>) -> Result<(), String> {
    let mut cfg = config.get();
    cfg.task_board.project_paths = next;
    config.set(cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn projects_list(
    reg: State<'_, ProjectRegistry>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<Vec<ProjectInfo>, String> {
    let mut out = Vec::new();
    for path in paths(&config) {
        let folder = PathBuf::from(&path);

        // 已開啟的專案：直接用 registry 的 handle 取計數。
        let opened = reg.all().into_iter().find(|h| h.path == folder);
        if let Some(h) = opened {
            let counts = ProjectCounts {
                planning: store::count_by_status(&h.pool, store::STATUS_PLANNING).await.unwrap_or(0),
                queued: store::count_by_status(&h.pool, store::STATUS_QUEUED).await.unwrap_or(0),
                running: store::count_by_status(&h.pool, store::STATUS_RUNNING).await.unwrap_or(0),
                done: store::count_by_status(&h.pool, store::STATUS_DONE).await.unwrap_or(0),
            };
            let description = find_aitprj(&folder)
                .and_then(|p| read_meta(&p))
                .map(|m| m.description)
                .unwrap_or_default();
            out.push(ProjectInfo {
                id: h.id,
                name: h.name,
                description,
                path,
                status: "ok".to_string(),
                counts,
                error: None,
            });
            continue;
        }

        // 未開啟：嘗試現在開啟；失敗則回報狀態，不擋住其他專案。
        match reg.open_folder(&folder).await {
            Ok(h) => {
                let counts = ProjectCounts {
                    planning: store::count_by_status(&h.pool, store::STATUS_PLANNING).await.unwrap_or(0),
                    queued: store::count_by_status(&h.pool, store::STATUS_QUEUED).await.unwrap_or(0),
                    running: store::count_by_status(&h.pool, store::STATUS_RUNNING).await.unwrap_or(0),
                    done: store::count_by_status(&h.pool, store::STATUS_DONE).await.unwrap_or(0),
                };
                let description = find_aitprj(&folder)
                    .and_then(|p| read_meta(&p))
                    .map(|m| m.description)
                    .unwrap_or_default();
                out.push(ProjectInfo {
                    id: h.id,
                    name: h.name,
                    description,
                    path,
                    status: "ok".to_string(),
                    counts,
                    error: None,
                });
            }
            Err(e) => {
                let name = folder
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                out.push(ProjectInfo {
                    // 開不起來就沒有真正的 id，用路徑當暫時識別，
                    // 前端只會拿它來呼叫 projects_remove。
                    id: format!("unopened:{path}"),
                    name,
                    description: String::new(),
                    path,
                    status: e.status_str().to_string(),
                    counts: zero_counts(),
                    error: Some(e.to_string()),
                });
            }
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct CreateProjectArgs {
    pub parent_dir: String,
    pub name: String,
    pub description: String,
}

#[tauri::command]
pub async fn projects_create(
    args: CreateProjectArgs,
    reg: State<'_, ProjectRegistry>,
    config: State<'_, Arc<ConfigStore>>,
    app: AppHandle,
) -> Result<String, String> {
    let handle = reg
        .create(std::path::Path::new(&args.parent_dir), &args.name, &args.description)
        .await
        .map_err(|e| e.to_string())?;
    let mut next = paths(&config);
    let path_str = handle.path.to_string_lossy().into_owned();
    if !next.contains(&path_str) {
        next.push(path_str);
    }
    set_paths(&config, next)?;
    emit_updated(&app);
    Ok(handle.id)
}

/// 開啟既有專案 —— 這同時就是「匯入」：別台機器複製過來的資料夾，
/// 挑它的 `.aitprj` 就進來了。
#[tauri::command]
pub async fn projects_open(
    aitprj_path: String,
    reg: State<'_, ProjectRegistry>,
    config: State<'_, Arc<ConfigStore>>,
    app: AppHandle,
) -> Result<String, String> {
    let folder = PathBuf::from(&aitprj_path)
        .parent()
        .ok_or_else(|| "無效的專案檔路徑".to_string())?
        .to_path_buf();

    let already_open = reg.all().into_iter().any(|h| h.path == folder);
    let handle = reg.open_folder(&folder).await.map_err(|e| e.to_string())?;

    let mut next = paths(&config);
    let path_str = folder.to_string_lossy().into_owned();
    if next.contains(&path_str) {
        return Err(format!("專案「{}」已在清單中", handle.name));
    }
    if already_open {
        return Err(format!("專案「{}」已在清單中", handle.name));
    }
    next.push(path_str);
    set_paths(&config, next)?;
    emit_updated(&app);
    Ok(handle.id)
}

#[derive(Deserialize)]
pub struct RemoveProjectArgs {
    pub id: String,
    pub delete_folder: bool,
}

#[tauri::command]
pub async fn projects_remove(
    args: RemoveProjectArgs,
    reg: State<'_, ProjectRegistry>,
    config: State<'_, Arc<ConfigStore>>,
    app: AppHandle,
) -> Result<(), String> {
    // 未能開啟的專案，前端送來的 id 是 "unopened:<path>"。
    let folder: PathBuf = if let Some(p) = args.id.strip_prefix("unopened:") {
        PathBuf::from(p)
    } else {
        let handle = reg
            .get(&args.id)
            .ok_or_else(|| format!("專案不存在：{}", args.id))?;
        let running = store::count_by_status(&handle.pool, store::STATUS_RUNNING)
            .await
            .map_err(|e| e.to_string())?;
        if running > 0 {
            return Err("這個專案還有工作在執行中，請先停止它們".to_string());
        }
        reg.close(&args.id);
        handle.path
    };

    let path_str = folder.to_string_lossy().into_owned();
    let next: Vec<String> = paths(&config).into_iter().filter(|p| p != &path_str).collect();
    set_paths(&config, next)?;

    if args.delete_folder {
        std::fs::remove_dir_all(&folder).map_err(|e| e.to_string())?;
    }
    emit_updated(&app);
    Ok(())
}

#[derive(Deserialize)]
pub struct RenameProjectArgs {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// 只改 `.aitprj` 裡的顯示名稱與描述，**不動磁碟路徑**。
#[tauri::command]
pub async fn projects_rename(
    args: RenameProjectArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let handle = reg.get(&args.id).ok_or_else(|| format!("專案不存在：{}", args.id))?;
    let meta_path = find_aitprj(&handle.path).map_err(|e: ProjectError| e.to_string())?;
    let mut meta = read_meta(&meta_path).map_err(|e| e.to_string())?;
    meta.name = args.name;
    meta.description = args.description;
    crate::projects::write_meta(&meta_path, &meta).map_err(|e| e.to_string())?;
    // registry 裡的名稱會在下次 projects_list 重新開啟時更新；
    // 這裡先關掉讓它下次重讀。
    reg.close(&args.id);
    emit_updated(&app);
    Ok(())
}
```

在 `src-tauri/src/commands/mod.rs` 加入：

```rust
pub mod projects;
```

- [ ] **Step 2: `lib.rs` 接線**

`src-tauri/src/lib.rs:169`，把建立 `TasksDb` 的那行移除，改為在 `.setup()` 中建立 registry 並載入專案。具體：

把 `:169` 的 `crate::tasks::TasksDb::new(),` 連同其所在的並行初始化項目移除，並把 `:207` 的 `.manage(tasks_db)` 改為：

```rust
        .manage(crate::projects::ProjectRegistry::new())
```

在 `.setup()` 的閉包中，`scheduler::spawn(app)` 被呼叫**之前**，加入：

```rust
        {
            let app_handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let root = crate::tasks::app_data_dir();
                // 舊資料搬遷：只在第一次執行，複製而非搬移。
                let migrated = match crate::projects::migrate::migrate_legacy(&root).await {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("legacy task migration failed: {e}");
                        None
                    }
                };

                let config = app_handle.state::<std::sync::Arc<crate::config::ConfigStore>>();
                let mut cfg = config.get();
                if let Some(dest) = migrated {
                    let p = dest.to_string_lossy().into_owned();
                    if !cfg.task_board.project_paths.contains(&p) {
                        cfg.task_board.project_paths.push(p);
                        let _ = config.set(cfg.clone());
                    }
                }

                // 開啟設定裡記錄的每一個專案。開不起來的不擋住其他專案 ——
                // projects_list 會把它們的狀態回報給前端。
                let reg = app_handle.state::<crate::projects::ProjectRegistry>();
                for path in cfg.task_board.project_paths {
                    if let Err(e) = reg.open_folder(std::path::Path::new(&path)).await {
                        eprintln!("open project {path}: {e}");
                    }
                }
            });
        }
```

在 `invoke_handler` 的 `generate_handler![...]` 清單中，`tasks_*` 那組旁邊加入：

```rust
            commands::projects::projects_list,
            commands::projects::projects_create,
            commands::projects::projects_open,
            commands::projects::projects_remove,
            commands::projects::projects_rename,
            commands::tasks::tasks_used_dirs,
```

- [ ] **Step 3: 改寫整合測試**

把 `src-tauri/tests/task_board.rs` 中對 `TasksDb` 的使用改為 `ProjectRegistry`。具體：

`:13` 改為：

```rust
use aiterm_lib::projects::{ProjectHandle, ProjectRegistry};
use aiterm_lib::tasks::init_schema;
```

`:25` 的假 dispatcher 簽名改為：

```rust
    async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
```

其函式體內的 `db.pool` 全部改為 `project.pool`。

`:69` 的 `mem_db()` 改為建立一個真的臨時專案（`drain_once` 現在需要 registry）：

```rust
/// 一個建在暫存目錄裡的單專案 registry。`TempDir` 一併回傳，
/// 呼叫端必須持有它直到測試結束，否則資料夾會被提前刪掉。
async fn one_project_registry() -> (ProjectRegistry, ProjectHandle, tempfile::TempDir) {
    let parent = tempfile::tempdir().unwrap();
    let reg = ProjectRegistry::new();
    let handle = reg.create(parent.path(), "test", "").await.unwrap();
    (reg, handle, parent)
}
```

`:78` 的 `wait_done(db, id)` 改為 `wait_done(pool: &sqlx::SqlitePool, id: &str)`，內部的 `&db.pool` 改成 `pool`。

`:102`、`:117` 的 `drain_once(&db, &dispatcher, 2).await;` 改為 `drain_once(&reg, &dispatcher, 2).await;`，並在每個測試開頭把 `let db = mem_db().await;` 換成：

```rust
    let (reg, project, _parent) = one_project_registry().await;
```

其後測試中所有 `&db.pool` 改成 `&project.pool`。

新增一個跨專案的整合測試（放在檔案末端）：

```rust
#[tokio::test]
async fn cards_from_two_projects_both_get_dispatched() {
    let parent = tempfile::tempdir().unwrap();
    let reg = ProjectRegistry::new();
    let a = reg.create(parent.path(), "alpha", "").await.unwrap();
    let b = reg.create(parent.path(), "beta", "").await.unwrap();

    let a_id = store::create_task(&a.pool, "a", "", "/r", true, false).await.unwrap();
    store::move_task(&a.pool, &a_id, store::STATUS_QUEUED, 1.0).await.unwrap();
    let b_id = store::create_task(&b.pool, "b", "", "/r", true, false).await.unwrap();
    store::move_task(&b.pool, &b_id, store::STATUS_QUEUED, 1.0).await.unwrap();

    let dispatcher = RecordingDispatcher::default();
    drain_once(&reg, &dispatcher, 5).await;

    let dispatched = dispatcher.dispatched.lock().clone();
    assert!(dispatched.contains(&a_id), "alpha 的卡片沒被派出去");
    assert!(dispatched.contains(&b_id), "beta 的卡片沒被派出去");
}
```

其中 `RecordingDispatcher` 加在檔案的假 dispatcher 旁：

```rust
/// 只記錄被派了哪些 task id，並把卡片標成 running（讓 drain_once 的
/// 迴圈能推進），不真的開 PTY。
///
/// 用 `std::sync::Mutex` 而非 `parking_lot::Mutex`：整合測試是獨立的
/// crate，只能用 `[dev-dependencies]` 裡宣告過的東西，parking_lot 是
/// 主 crate 的一般相依，在這裡不一定拿得到。
#[derive(Default)]
struct RecordingDispatcher {
    dispatched: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Dispatcher for RecordingDispatcher {
    async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
        self.dispatched.lock().unwrap().push(task.id.clone());
        store::mark_dispatched(&project.pool, &task.id, "fake-tab")
            .await
            .map_err(|e| e.to_string())
    }
}
```

該測試中讀取記錄的那行也隨之改為：

```rust
    let dispatched = dispatcher.dispatched.lock().unwrap().clone();
```

另外，`tests/task_board.rs` 頂端需要有 `use aiterm_lib::tasks::store;`（既有測試已在用 `store::` 的話就已經有了，沒有的話補上）。

- [ ] **Step 4: 執行完整測試**

Run: `cd src-tauri && cargo test`
Expected: 全部通過。**必須跑完整的 `cargo test`，不可用 `--lib`** —— `--lib` 不會編譯 `tests/` 底下的整合測試，曾經發生過整個分支本機全綠、推上去才在三平台炸編譯錯誤。

若有編譯錯誤，逐一修正；不要為了通過而刪測試或改斷言。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src
git add src-tauri/tests/task_board.rs
git commit -m "refactor(tasks)!: TasksDb 單例改為 ProjectRegistry，排程器跨專案派工

task_dir 以專案資料夾為根，12 個 tasks_* 指令加上 project_id。
monitor.rs 完全未動——它不碰資料庫，寫回發生在 dispatch 內
spawn 出去的 async block，那裡本來就捕獲 pool 的複本。"
```

---

## Task 11: 前端 IPC 層

**Files:**
- Create: `src/ipc/projects.ts`
- Modify: `src/ipc/tasks.ts`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/ipc/projects.test.ts`：

```ts
import { describe, expect, it, vi, beforeEach } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { createProject, listProjects, removeProject, usedDirs } from "./projects";

describe("projects ipc", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue([]);
  });

  it("listProjects 呼叫 projects_list", async () => {
    await listProjects();
    expect(invoke).toHaveBeenCalledWith("projects_list");
  });

  it("createProject 以 snake_case 傳遞 args", async () => {
    invoke.mockResolvedValue("new-id");
    const id = await createProject({ parentDir: "/p", name: "n", description: "d" });
    expect(id).toBe("new-id");
    expect(invoke).toHaveBeenCalledWith("projects_create", {
      args: { parent_dir: "/p", name: "n", description: "d" },
    });
  });

  it("removeProject 傳遞 delete_folder", async () => {
    invoke.mockResolvedValue(undefined);
    await removeProject("pid", true);
    expect(invoke).toHaveBeenCalledWith("projects_remove", {
      args: { id: "pid", delete_folder: true },
    });
  });

  it("usedDirs 傳遞 projectId", async () => {
    invoke.mockResolvedValue(["/a"]);
    await expect(usedDirs("pid")).resolves.toEqual(["/a"]);
    expect(invoke).toHaveBeenCalledWith("tasks_used_dirs", { projectId: "pid" });
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/ipc/projects.test.ts`
Expected: FAIL，`Failed to resolve import "./projects"`。

- [ ] **Step 3: 寫最小實作**

建立 `src/ipc/projects.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";

// ── Types（鏡射 Rust commands/projects.rs）──────────────────────────────

/** 專案資料夾目前的可用狀態。非 `ok` 時 counts 全為 0。 */
export type ProjectStatus = "ok" | "missing" | "invalid" | "incompatible";

export interface ProjectCounts {
  planning: number;
  queued: number;
  running: number;
  done: number;
}

export interface ProjectInfo {
  id: string;
  name: string;
  description: string;
  path: string;
  status: ProjectStatus;
  counts: ProjectCounts;
  error: string | null;
}

// ── Commands ───────────────────────────────────────────────────────────

export const listProjects = (): Promise<ProjectInfo[]> => invoke("projects_list");

export const createProject = (args: {
  parentDir: string;
  name: string;
  description: string;
}): Promise<string> =>
  invoke("projects_create", {
    args: { parent_dir: args.parentDir, name: args.name, description: args.description },
  });

/** 開啟既有專案。這同時就是「匯入」——別台機器複製過來的資料夾，挑它的 .aitprj。 */
export const openProject = (aitprjPath: string): Promise<string> =>
  invoke("projects_open", { aitprjPath });

/** `deleteFolder` 為 true 時連同磁碟資料夾一起刪除，且無法復原。 */
export const removeProject = (id: string, deleteFolder: boolean): Promise<void> =>
  invoke("projects_remove", { args: { id, delete_folder: deleteFolder } });

export const renameProject = (id: string, name: string, description: string): Promise<void> =>
  invoke("projects_rename", { args: { id, name, description } });

/** 這個專案的卡片用過的工作目錄，供新增工作時快捷選取。 */
export const usedDirs = (projectId: string): Promise<string[]> =>
  invoke("tasks_used_dirs", { projectId });
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/ipc/projects.test.ts`
Expected: PASS，4 個測試全過。

- [ ] **Step 5: 為 `tasks.ts` 加上 projectId**

修改 `src/ipc/tasks.ts`：每個 command 包裝函式的第一個參數改為 `projectId: string`，並在 invoke 的參數物件中加入 `projectId`。完整改法：

```ts
export const listTasks = (projectId: string): Promise<TaskWithAttachments[]> =>
  invoke("tasks_list", { projectId });

export const createTask = (
  projectId: string,
  args: {
    title: string;
    body: string;
    project_dir: string;
    parallel_ok: boolean;
    interactive: boolean;
  },
): Promise<string> => invoke("tasks_create", { projectId, args });

export const cloneTask = (projectId: string, id: string): Promise<string> =>
  invoke("tasks_clone", { projectId, id });

export const updateTask = (
  projectId: string,
  args: {
    id: string;
    title: string;
    body: string;
    project_dir: string;
    parallel_ok: boolean;
    interactive: boolean;
  },
): Promise<void> => invoke("tasks_update", { projectId, args });

export const moveTask = (
  projectId: string,
  id: string,
  to_status: TaskStatus,
  sort_order: number,
): Promise<void> => invoke("tasks_move", { projectId, args: { id, to_status, sort_order } });

export const stopTask = (projectId: string, id: string): Promise<void> =>
  invoke("tasks_stop", { projectId, id });

export const markTaskDone = (projectId: string, id: string): Promise<void> =>
  invoke("tasks_mark_done", { projectId, id });

export const deleteTask = (
  projectId: string,
  id: string,
  close_tab: boolean,
): Promise<void> => invoke("tasks_delete", { projectId, args: { id, close_tab } });

export const addAttachment = (
  projectId: string,
  id: string,
  filename: string,
  bytes: Uint8Array,
): Promise<AttachmentRow> =>
  invoke("tasks_add_attachment", {
    projectId,
    args: { id, filename, bytes: Array.from(bytes) },
  });

export const removeAttachment = (projectId: string, attachmentId: string): Promise<void> =>
  invoke("tasks_remove_attachment", { projectId, attachmentId });

export const readTranscript = (projectId: string, id: string): Promise<string> =>
  invoke("tasks_read_transcript", { projectId, id });

export const saveTranscript = (projectId: string, id: string, text: string): Promise<void> =>
  invoke("tasks_save_transcript", { projectId, id, text });
```

`getTaskBoardConfig` / `setTaskBoardConfig` / `onTasksUpdated` **不變**（它們是全域的，不屬於任何專案）。

- [ ] **Step 6: 型別檢查**

Run: `npx tsc -b`
Expected: 會出現一批錯誤，來自尚未更新的 `TaskCard.tsx` / `TaskEditorDialog.tsx` / `index.tsx` / `transcriptUpgrade.ts`。這是預期的中間狀態，Task 12–17 會逐一補完。

- [ ] **Step 7: Commit**

```bash
git add src/ipc/projects.ts src/ipc/projects.test.ts src/ipc/tasks.ts
git commit -m "feat(ipc): 專案 IPC 層，tasks IPC 全面帶上 projectId"
```

---

## Task 12: 把看板本體抽成 `ProjectBoard`

**Files:**
- Create: `src/components/TaskBoard/ProjectBoard.tsx`
- Modify: `src/components/TaskBoard/index.tsx`
- Modify: `src/components/TaskBoard/TaskCard.tsx`
- Modify: `src/components/TaskBoard/TranscriptDialog.tsx`
- Modify: `src/components/TaskBoard/transcriptUpgrade.ts`

- [ ] **Step 1: 建立 `ProjectBoard.tsx`**

把 `src/components/TaskBoard/index.tsx` 現有的**整個** `TaskBoardView` 函式本體搬到新檔 `ProjectBoard.tsx`，改名為 `ProjectBoard`，加上 `projectId` prop，並把所有 IPC 呼叫帶上它。變動點如下（其餘一字不改）：

函式簽名：

```tsx
export function ProjectBoard({ projectId }: { projectId: string }) {
```

`refresh`（原 `:63`）：

```tsx
  const refresh = useCallback(async () => {
    const rows = await listTasks(projectId);
    if (!mounted.current) return;
    const previous = lastStatusRef.current;
    const next = new Map<string, TaskStatus>();
    for (const row of rows) {
      next.set(row.id, row.status);
      const wasDone = previous.get(row.id) === "done";
      const justFinished = previous.has(row.id) && !wasDone && row.status === "done";
      if (justFinished) {
        void tryUpgradeTranscript(projectId, row.id, row.tab_id);
      }
    }
    lastStatusRef.current = next;
    setTasks(rows);
  }, [projectId]);
```

`handleDrop`（原 `:131`、`:141`）：

```tsx
        await markTaskDone(projectId, id);
```
```tsx
      await moveTask(projectId, id, to, sortOrder);
```

`handleDrop` 的相依陣列加入 `projectId`：

```tsx
    [tasks, isLegalDropTarget, projectId],
```

`TaskCard`、`TaskEditorDialog`、`TranscriptDialog` 三處渲染各加上 `projectId={projectId}`（含拖曳幽靈裡那個）。

最外層 `<div className="task-board">` 改為 `<div className="task-board-inner">`——`.task-board` 的 CSS 變數覆寫改由 `index.tsx` 的外層容器提供（Task 14 會處理），避免變數作用域被拆散。

- [ ] **Step 2: `TaskCard` 與 `TranscriptDialog` 接受 projectId**

`src/components/TaskBoard/TaskCard.tsx`：props 加 `projectId: string`，並把四個 IPC 呼叫帶上它：

```tsx
export function TaskCard({
  projectId,
  card,
  onEdit,
  onViewTranscript,
  onChanged,
}: {
  projectId: string;
  card: TaskWithAttachments;
  onEdit: () => void;
  onViewTranscript: () => void;
  onChanged: () => void;
}) {
```

- `:38` → `await run(() => deleteTask(projectId, card.id, closeTab));`
- `:107` → `onClick={() => void run(() => stopTask(projectId, card.id))}`
- `:111` → `onClick={() => void run(() => markTaskDone(projectId, card.id))}`
- `:123` → `onClick={() => void run(() => cloneTask(projectId, card.id))}`

`src/components/TaskBoard/TranscriptDialog.tsx`：props 加 `projectId: string`，`readTranscript` / `saveTranscript` 呼叫帶上它。

`src/components/TaskBoard/transcriptUpgrade.ts`：`tryUpgradeTranscript(taskId, tabId)` 改為 `tryUpgradeTranscript(projectId, taskId, tabId)`，內部的 `saveTranscript` 呼叫帶上 `projectId`。

- [ ] **Step 3: `index.tsx` 暫時只轉發**

把 `src/components/TaskBoard/index.tsx` 整份換成：

```tsx
import { ProjectBoard } from "./ProjectBoard";
import "./index.css";

export function TaskBoardView() {
  return <ProjectBoard projectId="" />;
}
```

這是**刻意的暫時狀態**，讓 Task 12 可以獨立驗證編譯與既有測試。Task 14 會把它換成真正的路由器。

- [ ] **Step 4: 驗證**

Run: `npx tsc -b`
Expected: PASS，無錯誤。

Run: `npm run test -- src/components/TaskBoard`
Expected: 既有的 `index.test.tsx` 會有失敗——它 mock 的 IPC 簽名變了。**在 Task 18 之前先不要修它**，本步驟只確認 `tsc -b` 通過。

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard
git commit -m "refactor(board): 看板本體抽成 ProjectBoard，全數 IPC 帶上 projectId"
```

---

## Task 13: 專案總覽 `ProjectList`

**Files:**
- Create: `src/components/TaskBoard/ProjectList.tsx`
- Create: `src/components/TaskBoard/ProjectList.test.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/TaskBoard/ProjectList.test.tsx`：

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listProjects = vi.fn();
const removeProject = vi.fn();
const confirmDialog = vi.fn();

vi.mock("../../ipc/projects", () => ({
  listProjects: (...a: unknown[]) => listProjects(...a),
  removeProject: (...a: unknown[]) => removeProject(...a),
  openProject: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: (...a: unknown[]) => confirmDialog(...a),
  open: vi.fn(),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ProjectList } from "./ProjectList";

const project = (over: Record<string, unknown> = {}) => ({
  id: "p1",
  name: "makemoney",
  description: "賺錢",
  path: "/projects/makemoney",
  status: "ok",
  counts: { planning: 2, queued: 1, running: 1, done: 3 },
  error: null,
  ...over,
});

const mount = (onOpen = vi.fn()) =>
  render(
    <LocaleProvider>
      <ProjectList onOpen={onOpen} />
    </LocaleProvider>,
  );

describe("ProjectList", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listProjects.mockResolvedValue([project()]);
    removeProject.mockResolvedValue(undefined);
  });

  it("列出專案與它的工作數", async () => {
    mount();
    expect(await screen.findByText("makemoney")).toBeInTheDocument();
    // 2 + 1 + 1 + 3 = 7
    expect(screen.getByTestId("project-total-p1")).toHaveTextContent("7");
  });

  it("有執行中工作時顯示指示點", async () => {
    mount();
    await screen.findByText("makemoney");
    expect(screen.getByTestId("project-running-p1")).toBeInTheDocument();
  });

  it("沒有執行中工作時不顯示指示點", async () => {
    listProjects.mockResolvedValue([
      project({ counts: { planning: 1, queued: 0, running: 0, done: 0 } }),
    ]);
    mount();
    await screen.findByText("makemoney");
    expect(screen.queryByTestId("project-running-p1")).not.toBeInTheDocument();
  });

  it("完全沒有專案時顯示空狀態", async () => {
    listProjects.mockResolvedValue([]);
    mount();
    expect(await screen.findByTestId("project-empty-state")).toBeInTheDocument();
  });

  it("點專案卡片會呼叫 onOpen", async () => {
    const onOpen = vi.fn();
    mount(onOpen);
    await userEvent.click(await screen.findByText("makemoney"));
    expect(onOpen).toHaveBeenCalledWith("p1");
  });

  it("遺失的專案顯示錯誤而非當機", async () => {
    listProjects.mockResolvedValue([
      project({ status: "missing", error: "專案資料夾或專案檔不存在", counts: { planning: 0, queued: 0, running: 0, done: 0 } }),
    ]);
    mount();
    expect(await screen.findByTestId("project-error-p1")).toHaveTextContent("不存在");
  });

  it("移除專案：兩段式詢問，第二段答否則不刪資料夾", async () => {
    confirmDialog.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    mount();
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));
    await waitFor(() => expect(removeProject).toHaveBeenCalledWith("p1", false));
    expect(confirmDialog).toHaveBeenCalledTimes(2);
  });

  it("移除專案：第二段答是才刪資料夾", async () => {
    confirmDialog.mockResolvedValueOnce(true).mockResolvedValueOnce(true);
    mount();
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));
    await waitFor(() => expect(removeProject).toHaveBeenCalledWith("p1", true));
  });

  it("移除專案：第一段就取消則完全不呼叫 removeProject", async () => {
    confirmDialog.mockResolvedValueOnce(false);
    mount();
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));
    await waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1));
    expect(removeProject).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/components/TaskBoard/ProjectList.test.tsx`
Expected: FAIL，`Failed to resolve import "./ProjectList"`。

- [ ] **Step 3: 加入 i18n 字串**

在 `src/lib/i18n.ts` 的 zh-TW 字典中，`board_` 開頭那組字串旁加入：

```ts
  proj_title: "專案",
  proj_new: "新增專案",
  proj_open_existing: "開啟現有專案",
  proj_empty_title: "還沒有任何專案",
  proj_empty_hint: "先建立一個專案，才能開始排工作。專案是磁碟上一個獨立的資料夾，複製走就等於匯出。",
  proj_tasks_count: "個工作",
  proj_running: "執行中",
  proj_remove: "移除",
  proj_remove_confirm: "確定要從清單移除這個專案嗎？",
  proj_remove_folder_confirm: "要連同磁碟上的資料夾一起刪除嗎？此動作無法復原。",
  proj_status_missing: "找不到專案資料夾",
  proj_status_invalid: "專案檔無法讀取",
  proj_status_incompatible: "專案檔版本不相容，請更新 AITerm",
  proj_create_name: "專案名稱",
  proj_create_desc: "描述（選填）",
  proj_create_parent: "建立位置",
  proj_create_browse: "瀏覽…",
  proj_create_submit: "建立",
  proj_create_cancel: "取消",
  proj_create_preview: "將建立於",
  proj_tab_close: "從分頁列關閉（不影響專案）",
  proj_tab_open_other: "開啟其他專案",
  proj_back_to_list: "專案",
```

在 en 字典中加入對應：

```ts
  proj_title: "Projects",
  proj_new: "New project",
  proj_open_existing: "Open existing project",
  proj_empty_title: "No projects yet",
  proj_empty_hint: "Create a project before queueing work. A project is a self-contained folder on disk — copying it is how you export it.",
  proj_tasks_count: "tasks",
  proj_running: "running",
  proj_remove: "Remove",
  proj_remove_confirm: "Remove this project from the list?",
  proj_remove_folder_confirm: "Also delete the folder on disk? This cannot be undone.",
  proj_status_missing: "Project folder not found",
  proj_status_invalid: "Project file could not be read",
  proj_status_incompatible: "Project file version not supported — update AITerm",
  proj_create_name: "Project name",
  proj_create_desc: "Description (optional)",
  proj_create_parent: "Create in",
  proj_create_browse: "Browse…",
  proj_create_submit: "Create",
  proj_create_cancel: "Cancel",
  proj_create_preview: "Will be created at",
  proj_tab_close: "Close tab (does not affect the project)",
  proj_tab_open_other: "Open another project",
  proj_back_to_list: "Projects",
```

- [ ] **Step 4: 寫最小實作**

建立 `src/components/TaskBoard/ProjectList.tsx`：

```tsx
import { useCallback, useEffect, useState } from "react";
import { confirm, open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import {
  listProjects,
  openProject,
  removeProject,
  type ProjectInfo,
} from "../../ipc/projects";
import { onTasksUpdated } from "../../ipc/tasks";
import { ProjectCreateDialog } from "./ProjectCreateDialog";

export function ProjectList({ onOpen }: { onOpen: (projectId: string) => void }) {
  const { t } = useLocale();
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [creating, setCreating] = useState(false);

  const refresh = useCallback(async () => {
    setProjects(await listProjects());
  }, []);

  useEffect(() => {
    void refresh();
    const un = onTasksUpdated(() => void refresh());
    return () => void un.then((f) => f());
  }, [refresh]);

  const statusLabel = (p: ProjectInfo) =>
    ({
      missing: t.proj_status_missing,
      invalid: t.proj_status_invalid,
      incompatible: t.proj_status_incompatible,
      ok: "",
    })[p.status];

  // 兩段式原生對話框，與 TaskCard 的刪除流程同一套模式。
  // 必須用 @tauri-apps/plugin-dialog 的非同步 confirm，不可用
  // window.confirm——Tauri 的 webview 沒有真正實作它（見 TaskCard.tsx:31）。
  const remove = async (p: ProjectInfo) => {
    if (!(await confirm(t.proj_remove_confirm))) return;
    const deleteFolder = await confirm(t.proj_remove_folder_confirm);
    await removeProject(p.id, deleteFolder);
    await refresh();
  };

  const pickExisting = async () => {
    const picked = await open({ filters: [{ name: "AITerm 專案", extensions: ["aitprj"] }] });
    if (typeof picked !== "string") return;
    await openProject(picked);
    await refresh();
  };

  return (
    <div className="project-list">
      <div className="project-list-head">
        <h2>{t.proj_title}</h2>
        <div className="project-list-actions">
          <button className="tb-btn tb-btn--primary" onClick={() => setCreating(true)}>
            + {t.proj_new}
          </button>
          <button className="tb-btn tb-btn--ghost" onClick={() => void pickExisting()}>
            {t.proj_open_existing}
          </button>
        </div>
      </div>

      {projects.length === 0 ? (
        <div className="project-empty" data-testid="project-empty-state">
          <div className="project-empty-title">{t.proj_empty_title}</div>
          <div className="project-empty-hint">{t.proj_empty_hint}</div>
        </div>
      ) : (
        <div className="project-grid">
          {projects.map((p) => {
            const total = p.counts.planning + p.counts.queued + p.counts.running + p.counts.done;
            const broken = p.status !== "ok";
            return (
              <div
                key={p.id}
                className={`project-card${broken ? " project-card--broken" : ""}`}
                data-testid={`project-card-${p.id}`}
              >
                <button
                  className="project-card-main"
                  disabled={broken}
                  onClick={() => onOpen(p.id)}
                >
                  <div className="project-card-name">{p.name}</div>
                  {p.description && <div className="project-card-desc">{p.description}</div>}
                  {broken ? (
                    <div className="project-card-error" data-testid={`project-error-${p.id}`}>
                      {statusLabel(p)}
                    </div>
                  ) : (
                    <div className="project-card-meta">
                      <span data-testid={`project-total-${p.id}`}>
                        {total} {t.proj_tasks_count}
                      </span>
                      {p.counts.running > 0 && (
                        <span
                          className="project-card-running"
                          data-testid={`project-running-${p.id}`}
                        >
                          ● {p.counts.running} {t.proj_running}
                        </span>
                      )}
                    </div>
                  )}
                </button>
                <button
                  className="tb-btn tb-btn--danger-ghost"
                  data-testid={`project-remove-${p.id}`}
                  onClick={() => void remove(p)}
                >
                  {t.proj_remove}
                </button>
              </div>
            );
          })}
        </div>
      )}

      {creating && (
        <ProjectCreateDialog
          onClose={() => setCreating(false)}
          onCreated={(id) => {
            setCreating(false);
            void refresh();
            onOpen(id);
          }}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 5: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/ProjectList.test.tsx`
Expected: 需要 Task 14 的 `ProjectCreateDialog` 才能解析 import。**先做 Task 14 再回頭跑這個測試。**

- [ ] **Step 6: 不 commit**

與 Task 14 一起 commit。

---

## Task 14: 建立專案對話框

**Files:**
- Create: `src/components/TaskBoard/ProjectCreateDialog.tsx`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/TaskBoard/ProjectCreateDialog.test.tsx`：

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const createProject = vi.fn();
const openDialog = vi.fn();

vi.mock("../../ipc/projects", () => ({
  createProject: (...a: unknown[]) => createProject(...a),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => openDialog(...a) }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ProjectCreateDialog } from "./ProjectCreateDialog";

const mount = (onCreated = vi.fn()) =>
  render(
    <LocaleProvider>
      <ProjectCreateDialog onClose={vi.fn()} onCreated={onCreated} />
    </LocaleProvider>,
  );

describe("ProjectCreateDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    createProject.mockResolvedValue("new-id");
  });

  it("名稱為空時無法送出", async () => {
    mount();
    expect(screen.getByTestId("project-create-submit")).toBeDisabled();
  });

  it("沒有選建立位置時無法送出", async () => {
    mount();
    await userEvent.type(screen.getByTestId("project-create-name"), "makemoney");
    expect(screen.getByTestId("project-create-submit")).toBeDisabled();
  });

  it("名稱與位置都齊了才送出，並回報新 id", async () => {
    openDialog.mockResolvedValue("/Users/me/Projects");
    const onCreated = vi.fn();
    mount(onCreated);

    await userEvent.type(screen.getByTestId("project-create-name"), "makemoney");
    await userEvent.click(screen.getByTestId("project-create-browse"));
    await waitFor(() =>
      expect(screen.getByTestId("project-create-submit")).not.toBeDisabled(),
    );
    await userEvent.click(screen.getByTestId("project-create-submit"));

    await waitFor(() =>
      expect(createProject).toHaveBeenCalledWith({
        parentDir: "/Users/me/Projects",
        name: "makemoney",
        description: "",
      }),
    );
    expect(onCreated).toHaveBeenCalledWith("new-id");
  });

  it("記住上次的建立位置", async () => {
    localStorage.setItem("aiterm_last_project_parent", "/remembered");
    mount();
    await userEvent.type(screen.getByTestId("project-create-name"), "x");
    await waitFor(() =>
      expect(screen.getByTestId("project-create-submit")).not.toBeDisabled(),
    );
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/components/TaskBoard/ProjectCreateDialog.test.tsx`
Expected: FAIL，`Failed to resolve import "./ProjectCreateDialog"`。

- [ ] **Step 3: 寫最小實作**

建立 `src/components/TaskBoard/ProjectCreateDialog.tsx`：

```tsx
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { createProject } from "../../ipc/projects";

/** 沿用 TaskEditorDialog 記住上次目錄的做法，但用自己的 key。 */
const LAST_PARENT_KEY = "aiterm_last_project_parent";

export function ProjectCreateDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (projectId: string) => void;
}) {
  const { t } = useLocale();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [parent, setParent] = useState(localStorage.getItem(LAST_PARENT_KEY) ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pickParent = async () => {
    const picked = await open({ directory: true, defaultPath: parent || undefined });
    if (typeof picked === "string") setParent(picked);
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      localStorage.setItem(LAST_PARENT_KEY, parent);
      const id = await createProject({ parentDir: parent, name: name.trim(), description });
      onCreated(id);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = name.trim().length > 0 && parent.length > 0 && !busy;

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="task-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>{t.proj_new}</h3>

        <label className="task-field">
          <span>{t.proj_create_name}</span>
          <input
            data-testid="project-create-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>

        <label className="task-field">
          <span>{t.proj_create_desc}</span>
          <input
            data-testid="project-create-desc"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </label>

        <label className="task-field">
          <span>{t.proj_create_parent}</span>
          <div className="task-field-row">
            <input readOnly value={parent} data-testid="project-create-parent" />
            <button
              className="tb-btn tb-btn--ghost"
              data-testid="project-create-browse"
              onClick={() => void pickParent()}
            >
              {t.proj_create_browse}
            </button>
          </div>
        </label>

        {parent && name.trim() && (
          <div className="task-field-hint">
            {t.proj_create_preview}：{parent}/{name.trim()}
          </div>
        )}
        {error && <div className="task-field-error">{error}</div>}

        <div className="task-dialog-actions">
          <button className="tb-btn tb-btn--ghost" onClick={onClose}>
            {t.proj_create_cancel}
          </button>
          <button
            className="tb-btn tb-btn--primary"
            data-testid="project-create-submit"
            disabled={!ready}
            onClick={() => void submit()}
          >
            {t.proj_create_submit}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/ProjectCreateDialog.test.tsx src/components/TaskBoard/ProjectList.test.tsx`
Expected: PASS，兩個檔案共 13 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/ProjectList.tsx src/components/TaskBoard/ProjectList.test.tsx
git add src/components/TaskBoard/ProjectCreateDialog.tsx src/components/TaskBoard/ProjectCreateDialog.test.tsx
git add src/lib/i18n.ts
git commit -m "feat(board): 專案總覽與建立專案對話框"
```

---

## Task 15: 專案分頁列

**Files:**
- Create: `src/components/TaskBoard/ProjectTabBar.tsx`
- Create: `src/components/TaskBoard/ProjectTabBar.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/TaskBoard/ProjectTabBar.test.tsx`：

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const removeProject = vi.fn();
vi.mock("../../ipc/projects", () => ({
  removeProject: (...a: unknown[]) => removeProject(...a),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ProjectTabBar } from "./ProjectTabBar";
import type { ProjectInfo } from "../../ipc/projects";

const proj = (id: string, running = 0): ProjectInfo => ({
  id,
  name: id,
  description: "",
  path: `/p/${id}`,
  status: "ok",
  counts: { planning: 0, queued: 0, running, done: 0 },
  error: null,
});

const mount = (over: Partial<Parameters<typeof ProjectTabBar>[0]> = {}) => {
  const props = {
    projects: [proj("alpha"), proj("beta", 2)],
    openIds: ["alpha", "beta"],
    activeId: "alpha",
    onActivate: vi.fn(),
    onClose: vi.fn(),
    onOpenOther: vi.fn(),
    onBackToList: vi.fn(),
    ...over,
  };
  render(
    <LocaleProvider>
      <ProjectTabBar {...props} />
    </LocaleProvider>,
  );
  return props;
};

describe("ProjectTabBar", () => {
  beforeEach(() => vi.clearAllMocks());

  it("只顯示已開啟的專案", () => {
    mount({ openIds: ["alpha"] });
    expect(screen.getByTestId("project-tab-alpha")).toBeInTheDocument();
    expect(screen.queryByTestId("project-tab-beta")).not.toBeInTheDocument();
  });

  it("點分頁切換活躍專案", async () => {
    const props = mount();
    await userEvent.click(screen.getByTestId("project-tab-beta"));
    expect(props.onActivate).toHaveBeenCalledWith("beta");
  });

  it("有執行中工作的分頁顯示指示點", () => {
    mount();
    expect(screen.getByTestId("project-tab-running-beta")).toBeInTheDocument();
    expect(screen.queryByTestId("project-tab-running-alpha")).not.toBeInTheDocument();
  });

  // 這是本功能最容易寫錯的地方：關閉分頁跟移除專案視覺上都是
  // 「把這個專案弄掉」，但語意完全不同。這個測試把它釘死。
  it("關閉分頁只呼叫 onClose，絕不呼叫 removeProject", async () => {
    const props = mount();
    await userEvent.click(screen.getByTestId("project-tab-close-alpha"));
    expect(props.onClose).toHaveBeenCalledWith("alpha");
    expect(removeProject).not.toHaveBeenCalled();
  });

  it("關閉鍵不會順帶切換活躍專案", async () => {
    const props = mount({ activeId: "beta" });
    await userEvent.click(screen.getByTestId("project-tab-close-alpha"));
    expect(props.onActivate).not.toHaveBeenCalled();
  });

  it("回專案總覽", async () => {
    const props = mount();
    await userEvent.click(screen.getByTestId("project-tab-back"));
    expect(props.onBackToList).toHaveBeenCalled();
  });

  it("開啟其他專案的按鈕", async () => {
    const props = mount();
    await userEvent.click(screen.getByTestId("project-tab-add"));
    expect(props.onOpenOther).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/components/TaskBoard/ProjectTabBar.test.tsx`
Expected: FAIL，`Failed to resolve import "./ProjectTabBar"`。

- [ ] **Step 3: 寫最小實作**

建立 `src/components/TaskBoard/ProjectTabBar.tsx`：

```tsx
import { useLocale } from "../../contexts/LocaleContext";
import type { ProjectInfo } from "../../ipc/projects";

/**
 * 開啟中專案的分頁列。
 *
 * 關鍵語意：`onClose`（分頁上的 ×）**只是把分頁從這一列拿掉**，
 * 不移除專案、不刪任何檔案、也不影響派工——該專案的卡片照樣會被
 * 排程器派出去。要真的移除專案得回專案總覽用那裡的「移除」。
 * 這兩件事視覺上都像「把專案弄掉」，很容易寫錯，
 * ProjectTabBar.test.tsx 有測試把它釘住。
 */
export function ProjectTabBar({
  projects,
  openIds,
  activeId,
  onActivate,
  onClose,
  onOpenOther,
  onBackToList,
}: {
  projects: ProjectInfo[];
  openIds: string[];
  activeId: string | null;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onOpenOther: () => void;
  onBackToList: () => void;
}) {
  const { t } = useLocale();
  const open = openIds
    .map((id) => projects.find((p) => p.id === id))
    .filter((p): p is ProjectInfo => p !== undefined);

  return (
    <div className="project-tabbar">
      <button
        className="project-tabbar-back"
        data-testid="project-tab-back"
        onClick={onBackToList}
      >
        ≡ {t.proj_back_to_list}
      </button>

      <div className="project-tabbar-tabs">
        {open.map((p) => (
          <div
            key={p.id}
            className={`project-tab${p.id === activeId ? " project-tab--active" : ""}${
              p.status !== "ok" ? " project-tab--broken" : ""
            }`}
          >
            <button
              className="project-tab-label"
              data-testid={`project-tab-${p.id}`}
              onClick={() => onActivate(p.id)}
            >
              {p.counts.running > 0 && (
                <span
                  className="project-tab-running"
                  data-testid={`project-tab-running-${p.id}`}
                >
                  ●
                </span>
              )}
              {p.name}
            </button>
            <button
              className="project-tab-close"
              data-testid={`project-tab-close-${p.id}`}
              title={t.proj_tab_close}
              onClick={(e) => {
                // 不讓點擊冒泡到分頁本體，否則關閉會順帶把這個分頁
                // 設成活躍的，畫面會閃一下已經關掉的專案。
                e.stopPropagation();
                onClose(p.id);
              }}
            >
              ×
            </button>
          </div>
        ))}
      </div>

      <button
        className="project-tabbar-add"
        data-testid="project-tab-add"
        title={t.proj_tab_open_other}
        onClick={onOpenOther}
      >
        +
      </button>
    </div>
  );
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/ProjectTabBar.test.tsx`
Expected: PASS，7 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/ProjectTabBar.tsx src/components/TaskBoard/ProjectTabBar.test.tsx
git commit -m "feat(board): 專案分頁列（關閉分頁不等於移除專案）"
```

---

## Task 16: `index.tsx` 變成路由器

**Files:**
- Modify: `src/components/TaskBoard/index.tsx`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/TaskBoard/router.test.tsx`：

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listProjects = vi.fn();

vi.mock("../../ipc/projects", () => ({
  listProjects: (...a: unknown[]) => listProjects(...a),
  removeProject: vi.fn(),
  openProject: vi.fn(),
  createProject: vi.fn(),
}));
vi.mock("../../ipc/tasks", () => ({
  listTasks: vi.fn().mockResolvedValue([]),
  onTasksUpdated: vi.fn().mockResolvedValue(() => {}),
  moveTask: vi.fn(),
  markTaskDone: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), open: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskBoardView } from "./index";

const proj = (id: string) => ({
  id,
  name: id,
  description: "",
  path: `/p/${id}`,
  status: "ok" as const,
  counts: { planning: 0, queued: 0, running: 0, done: 0 },
  error: null,
});

const mount = () =>
  render(
    <LocaleProvider>
      <TaskBoardView />
    </LocaleProvider>,
  );

describe("TaskBoardView 路由", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    listProjects.mockResolvedValue([proj("alpha"), proj("beta")]);
  });

  it("一開始顯示專案總覽，不顯示看板", async () => {
    mount();
    expect(await screen.findByText("alpha")).toBeInTheDocument();
    expect(screen.queryByTestId("column-planning")).not.toBeInTheDocument();
  });

  it("點專案後顯示看板與分頁列", async () => {
    mount();
    await userEvent.click(await screen.findByText("alpha"));
    expect(await screen.findByTestId("column-planning")).toBeInTheDocument();
    expect(screen.getByTestId("project-tab-alpha")).toBeInTheDocument();
  });

  it("關閉最後一個分頁後回到專案總覽", async () => {
    mount();
    await userEvent.click(await screen.findByText("alpha"));
    await screen.findByTestId("column-planning");
    await userEvent.click(screen.getByTestId("project-tab-close-alpha"));
    await waitFor(() =>
      expect(screen.queryByTestId("column-planning")).not.toBeInTheDocument(),
    );
  });

  it("開啟中的分頁存進 localStorage 並在重新掛載後還原", async () => {
    const first = mount();
    await userEvent.click(await screen.findByText("alpha"));
    await screen.findByTestId("column-planning");
    first.unmount();

    mount();
    expect(await screen.findByTestId("project-tab-alpha")).toBeInTheDocument();
  });

  it("還原時過濾掉已不存在的專案", async () => {
    localStorage.setItem("aiterm_board_open_projects", JSON.stringify(["gone", "alpha"]));
    localStorage.setItem("aiterm_board_active_project", "gone");
    mount();
    expect(await screen.findByTestId("project-tab-alpha")).toBeInTheDocument();
    expect(screen.queryByTestId("project-tab-gone")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/components/TaskBoard/router.test.tsx`
Expected: FAIL，第一個測試就失敗——目前的 `index.tsx` 直接渲染 `ProjectBoard`，不顯示專案總覽。

- [ ] **Step 3: 寫最小實作**

把 `src/components/TaskBoard/index.tsx` 整份換成：

```tsx
import { useCallback, useEffect, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import { listProjects, openProject, type ProjectInfo } from "../../ipc/projects";
import { onTasksUpdated } from "../../ipc/tasks";
import { ProjectBoard } from "./ProjectBoard";
import { ProjectList } from "./ProjectList";
import { ProjectTabBar } from "./ProjectTabBar";
import "./index.css";

const OPEN_KEY = "aiterm_board_open_projects";
const ACTIVE_KEY = "aiterm_board_active_project";

function readOpenIds(): string[] {
  try {
    const raw = localStorage.getItem(OPEN_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

/**
 * 兩層導覽的路由器。
 *
 * 「開啟中」在這裡純粹是 UI 概念——排程器會派工給**所有**已知專案，
 * 跟分頁列上有沒有它無關（spec D4/D7）。
 */
export function TaskBoardView() {
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [openIds, setOpenIds] = useState<string[]>(readOpenIds);
  const [activeId, setActiveId] = useState<string | null>(
    () => localStorage.getItem(ACTIVE_KEY),
  );
  const [showList, setShowList] = useState(false);

  const refresh = useCallback(async () => {
    setProjects(await listProjects());
  }, []);

  useEffect(() => {
    void refresh();
    const un = onTasksUpdated(() => void refresh());
    return () => void un.then((f) => f());
  }, [refresh]);

  // 還原時過濾掉已不存在的專案——資料夾可能在上次關閉之後被刪掉了。
  // 等 projects 真的載進來才做，否則第一次 render（projects 還是空陣列）
  // 會把所有分頁都當成不存在而清光。
  useEffect(() => {
    if (projects.length === 0) return;
    const known = new Set(projects.map((p) => p.id));
    setOpenIds((prev) => {
      const next = prev.filter((id) => known.has(id));
      return next.length === prev.length ? prev : next;
    });
  }, [projects]);

  useEffect(() => {
    localStorage.setItem(OPEN_KEY, JSON.stringify(openIds));
  }, [openIds]);

  useEffect(() => {
    if (activeId) localStorage.setItem(ACTIVE_KEY, activeId);
    else localStorage.removeItem(ACTIVE_KEY);
  }, [activeId]);

  const activate = useCallback((id: string) => {
    setOpenIds((prev) => (prev.includes(id) ? prev : [...prev, id]));
    setActiveId(id);
    setShowList(false);
  }, []);

  /** 只把分頁從列上拿掉。不移除專案、不刪檔案、不影響派工。 */
  const closeTab = useCallback(
    (id: string) => {
      // 兩個 setState 分開寫，不把 setActiveId 塞進 setOpenIds 的
      // updater 裡——updater 必須是純函式，在裡面觸發另一個 setState
      // 在 StrictMode 的雙重呼叫下會執行兩次。
      setOpenIds((prev) => prev.filter((x) => x !== id));
      setActiveId((current) => {
        if (current !== id) return current;
        const rest = openIds.filter((x) => x !== id);
        return rest[rest.length - 1] ?? null;
      });
    },
    [openIds],
  );

  const openOther = useCallback(async () => {
    const picked = await openFileDialog({
      filters: [{ name: "AITerm 專案", extensions: ["aitprj"] }],
    });
    if (typeof picked !== "string") return;
    const id = await openProject(picked);
    await refresh();
    activate(id);
  }, [activate, refresh]);

  const visibleOpenIds = openIds.filter((id) => projects.some((p) => p.id === id));
  // 先算出一個確定是 string 的 active，再用它做分支——寫成
  // `active !== null` 的布林旗標的話，TypeScript 不會據此收窄
  // 底下 JSX 中 active 的型別，projectId={active} 會是型別錯誤。
  const active = visibleOpenIds.includes(activeId ?? "") ? activeId : null;

  if (showList || active === null) {
    return (
      <div className="task-board">
        <ProjectList onOpen={activate} />
      </div>
    );
  }

  return (
    <div className="task-board">
      <ProjectTabBar
        projects={projects}
        openIds={visibleOpenIds}
        activeId={active}
        onActivate={activate}
        onClose={closeTab}
        onOpenOther={() => void openOther()}
        onBackToList={() => setShowList(true)}
      />
      {/* key 讓切換專案時整個 ProjectBoard 重新掛載，狀態不會殘留。
          這裡刻意「卸載而非隱藏」——TerminalView 那條「隱藏而非卸載」
          的規則是為了 xterm.js 在無尺寸元素上 resize 會崩潰，
          看板沒有 xterm，不適用。 */}
      <ProjectBoard key={active} projectId={active} />
    </div>
  );
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/router.test.tsx`
Expected: PASS，5 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/index.tsx src/components/TaskBoard/router.test.tsx
git commit -m "feat(board): 兩層導覽路由器，分頁狀態持久化"
```

---

## Task 17: 工作目錄快捷選項

**Files:**
- Modify: `src/components/TaskBoard/TaskEditorDialog.tsx`
- Modify: `src/components/TaskBoard/ProjectBoard.tsx`（傳入 projectId）

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/TaskBoard/TaskEditorDialog.usedDirs.test.tsx`：

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedDirs = vi.fn();
vi.mock("../../ipc/projects", () => ({ usedDirs: (...a: unknown[]) => usedDirs(...a) }));
vi.mock("../../ipc/tasks", () => ({
  createTask: vi.fn(),
  updateTask: vi.fn(),
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

describe("TaskEditorDialog 工作目錄快捷選項", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it("列出這個專案用過的目錄", async () => {
    usedDirs.mockResolvedValue(["/repo/web", "/repo/api"]);
    mount();
    expect(await screen.findByTestId("used-dir-/repo/web")).toBeInTheDocument();
    expect(screen.getByTestId("used-dir-/repo/api")).toBeInTheDocument();
    expect(usedDirs).toHaveBeenCalledWith("p1");
  });

  it("點快捷選項會填入目錄欄", async () => {
    usedDirs.mockResolvedValue(["/repo/web"]);
    mount();
    await userEvent.click(await screen.findByTestId("used-dir-/repo/web"));
    expect(screen.getByTestId("task-dir-input")).toHaveValue("/repo/web");
  });

  it("沒有用過的目錄時不顯示這一區", async () => {
    usedDirs.mockResolvedValue([]);
    mount();
    await screen.findByTestId("task-dir-input");
    expect(screen.queryByTestId("used-dirs-row")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試，確認它失敗**

Run: `npm run test -- src/components/TaskBoard/TaskEditorDialog.usedDirs.test.tsx`
Expected: FAIL — `TaskEditorDialog` 尚無 `projectId` prop，也沒有 `used-dir-*` 元素。

- [ ] **Step 3: 寫最小實作**

修改 `src/components/TaskBoard/TaskEditorDialog.tsx`：

props 加入 `projectId: string`：

```tsx
export function TaskEditorDialog({
  projectId,
  card,
  onClose,
  onSaved,
}: {
  projectId: string;
  card: TaskWithAttachments | null;
  onClose: () => void;
  onSaved: () => void;
}) {
```

在 `const [busy, setBusy] = useState(false);` 之後加入：

```tsx
  // 專案不綁資料夾（工作可散布在多個 repo），所以列出這個專案已經
  // 用過的目錄讓使用者一鍵選取，不必每次重新瀏覽。
  const [dirChoices, setDirChoices] = useState<string[]>([]);
  useEffect(() => {
    let alive = true;
    void usedDirs(projectId).then((dirs) => {
      if (alive) setDirChoices(dirs);
    });
    return () => {
      alive = false;
    };
  }, [projectId]);
```

檔案頂端的 import 加上：

```tsx
import { useEffect, useState } from "react";
import { usedDirs } from "../../ipc/projects";
```

把目錄輸入欄加上 `data-testid="task-dir-input"`，並在它下方加入快捷選項列：

```tsx
        {dirChoices.length > 0 && (
          <div className="task-used-dirs" data-testid="used-dirs-row">
            {dirChoices.map((d) => (
              <button
                key={d}
                className="tb-btn tb-btn--ghost tb-btn--tiny"
                data-testid={`used-dir-${d}`}
                onClick={() => setDir(d)}
              >
                📁 {d}
              </button>
            ))}
          </div>
        )}
```

`save()` 中的兩個 IPC 呼叫帶上 `projectId`：

```tsx
        await updateTask(projectId, { id: card.id, title, body, project_dir: dir, parallel_ok: parallelOk, interactive });
```
```tsx
        const newId = await createTask(projectId, { title, body, project_dir: dir, parallel_ok: parallelOk, interactive });
```

`onFiles` 與 `removeAttachmentAt` 中的 `addAttachment` / `removeAttachment` 也各帶上 `projectId`。

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/TaskEditorDialog.usedDirs.test.tsx`
Expected: PASS，3 個測試全過。

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/TaskEditorDialog.tsx
git add src/components/TaskBoard/TaskEditorDialog.usedDirs.test.tsx
git add src/components/TaskBoard/ProjectBoard.tsx
git commit -m "feat(board): 新增工作時列出這個專案用過的工作目錄"
```

---

## Task 18: 樣式

**Files:**
- Modify: `src/components/TaskBoard/index.css`

- [ ] **Step 1: 加入專案總覽與分頁列的樣式**

在 `src/components/TaskBoard/index.css` 的變數覆寫區塊之後加入：

```css
/* ── 專案總覽 ─────────────────────────────────────────────── */

.project-list {
  padding: 20px 24px;
  height: 100%;
  overflow-y: auto;
}

.project-list-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 18px;
}

.project-list-head h2 {
  margin: 0;
  font-size: 18px;
  color: var(--text-primary);
}

.project-list-actions {
  display: flex;
  gap: 8px;
}

.project-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  gap: 12px;
}

.project-card {
  display: flex;
  align-items: stretch;
  gap: 6px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--bg-secondary);
  padding: 6px;
}

.project-card--broken {
  border-color: var(--tb-failed);
  opacity: 0.75;
}

.project-card-main {
  flex: 1;
  min-width: 0;
  text-align: left;
  background: none;
  border: 0;
  color: inherit;
  cursor: pointer;
  padding: 8px 10px;
  border-radius: 8px;
}

.project-card-main:disabled {
  cursor: default;
}

.project-card-name {
  font-weight: 600;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.project-card-desc,
.project-card-meta,
.project-card-error {
  font-size: 12px;
  color: var(--text-secondary);
  margin-top: 4px;
}

.project-card-meta {
  display: flex;
  gap: 10px;
}

.project-card-running {
  color: var(--accent);
}

.project-card-error {
  color: var(--tb-failed);
}

.project-empty {
  text-align: center;
  padding: 60px 20px;
  color: var(--text-secondary);
}

.project-empty-title {
  font-size: 16px;
  color: var(--text-primary);
  margin-bottom: 8px;
}

.project-empty-hint {
  max-width: 420px;
  margin: 0 auto;
  line-height: 1.6;
}

/* ── 專案分頁列 ───────────────────────────────────────────── */

.project-tabbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border);
  /* 分頁很多時水平捲動，與 TabBar 相同的處理 */
  overflow-x: auto;
  overflow-y: hidden;
  flex: 0 0 auto;
}

.project-tabbar-back,
.project-tabbar-add {
  flex: 0 0 auto;
  background: none;
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text-secondary);
  cursor: pointer;
  padding: 4px 10px;
  font-size: 12px;
}

.project-tabbar-tabs {
  display: flex;
  gap: 4px;
  flex: 1 1 auto;
  min-width: 0;
}

.project-tab {
  display: flex;
  align-items: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: var(--bg-secondary);
  flex: 0 0 auto;
}

.project-tab--active {
  border-color: var(--accent);
}

.project-tab--broken {
  border-color: var(--tb-failed);
}

.project-tab-label {
  background: none;
  border: 0;
  color: var(--text-primary);
  cursor: pointer;
  padding: 4px 4px 4px 10px;
  font-size: 12px;
  display: flex;
  align-items: center;
  gap: 5px;
}

.project-tab-running {
  color: var(--accent);
  font-size: 9px;
}

.project-tab-close {
  background: none;
  border: 0;
  color: var(--text-secondary);
  cursor: pointer;
  padding: 4px 8px 4px 4px;
  font-size: 13px;
  line-height: 1;
}

.project-tab-close:hover {
  color: var(--tb-failed);
}

/* 分頁列存在時，看板本體吃掉剩下的高度 */
.task-board-inner {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

/* ── 工作目錄快捷選項 ─────────────────────────────────────── */

.task-used-dirs {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 6px;
}

.tb-btn--tiny {
  font-size: 11px;
  padding: 2px 8px;
}
```

同時確認 `.task-board` 這個選擇器有 `display: flex; flex-direction: column;`（分頁列 + 看板本體的垂直堆疊需要它）。若沒有就加上。

**注意（這個檔案有前科）：** 這個 CSS 檔的註解裡**絕對不能出現星號緊接斜線的組合**。那會提前關閉註解，後面的中文會洩漏成 CSS 垃圾，並把緊接著的整條規則吃掉——`.task-board` 的變數覆寫區塊曾經因此整組失效，害得卡片左側狀態色條完全不顯示，查了很久。檔案裡既有的那段警告註解不要刪。

- [ ] **Step 2: 驗證**

Run: `npx tsc -b && npm run test -- src/components/TaskBoard`
Expected: 型別檢查通過；`index.test.tsx` 仍有失敗（Task 19 處理）。

用 postcss 確認新加的規則真的被解析出來（而不是被壞掉的註解吃掉）：

```bash
node -e "
const fs=require('fs'),postcss=require('postcss');
const root=postcss.parse(fs.readFileSync('src/components/TaskBoard/index.css','utf8'));
const want=['.project-tabbar','.project-tab-close','.project-grid','.task-used-dirs'];
const got=new Set();root.walkRules(r=>got.add(r.selector));
const missing=want.filter(s=>![...got].some(g=>g.includes(s)));
console.log(missing.length?'MISSING: '+missing.join(', '):'all present');
"
```
Expected: `all present`

- [ ] **Step 3: Commit**

```bash
git add src/components/TaskBoard/index.css
git commit -m "style(board): 專案總覽、分頁列、目錄快捷選項的樣式"
```

---

## Task 19: 修復既有測試並完整驗證

**Files:**
- Modify: `src/components/TaskBoard/index.test.tsx`
- Modify: `src/components/TerminalApp.taskBoard.test.tsx`

- [ ] **Step 1: 更新 `index.test.tsx`**

這個檔案（751 行）測的是看板本體的行為（拖曳、卡片動作、對話框）。它現在測的對象是 `ProjectBoard` 而非 `TaskBoardView`。

改法：
1. 檔頂的 import 從 `import { TaskBoardView } from "./index";` 改為 `import { ProjectBoard } from "./ProjectBoard";`
2. 每個 `render(<TaskBoardView />)` 改為 `render(<ProjectBoard projectId="p1" />)`
3. 每個 IPC mock 的斷言加上 projectId：例如 `expect(moveTask).toHaveBeenCalledWith("id", "queued", 1)` 改為 `expect(moveTask).toHaveBeenCalledWith("p1", "id", "queued", 1)`
4. `listTasks` 的 mock 現在收到一個參數，若有 `toHaveBeenCalledWith()` 的斷言改為 `toHaveBeenCalledWith("p1")`

**不要**為了讓測試變綠而放寬斷言（例如改成 `expect.anything()`）——projectId 傳錯是這次重構最可能出的 bug，斷言要真的檢查它。

- [ ] **Step 2: 更新 `TerminalApp.taskBoard.test.tsx`**

這個檔案 mock 了 TaskBoardView 的 IPC 讓看板能乾淨掛載。現在 `TaskBoardView` 一開始渲染的是 `ProjectList`，需要 mock `projects_list`。在既有的 mock 區塊加入：

```tsx
vi.mock("../ipc/projects", () => ({
  listProjects: vi.fn().mockResolvedValue([]),
  removeProject: vi.fn(),
  openProject: vi.fn(),
  createProject: vi.fn(),
  usedDirs: vi.fn().mockResolvedValue([]),
}));
```

- [ ] **Step 3: 跑完整前端測試**

Run: `npm run test`
Expected: 全部通過。

- [ ] **Step 4: 跑完整驗證三件套**

```bash
npx tsc -b
npm run test
cd src-tauri && cargo test
```
Expected: 三者全綠。`cargo test` 必須是完整的，不可用 `--lib`。

- [ ] **Step 5: 對照 lint 基準**

Run: `npm run lint 2>&1 | tail -3`

把問題總數與改動前的 master 比較。**不要求變好，但不可變差。** 若增加了，找出是哪幾條新違規並修掉——常見的是 `react-hooks/exhaustive-deps`（useCallback/useEffect 的相依陣列漏了 `projectId`）與 `react-hooks/refs`。

- [ ] **Step 6: Commit**

```bash
git add src/components/TaskBoard/index.test.tsx src/components/TerminalApp.taskBoard.test.tsx
git commit -m "test: 既有看板測試改測 ProjectBoard 並驗證 projectId 傳遞"
```

---

## Task 20: 手動驗收

**Files:** 無（純驗證）

- [ ] **Step 1: 啟動 App**

Run: `npm run tauri:dev`

- [ ] **Step 2: 逐項確認**

| # | 操作 | 預期 |
|---|---|---|
| 1 | 首次進入工作看板 | 若你先前有卡片：看到一個「預設專案」，卡片都在裡面，附件與對話記錄都打得開。若沒有：看到空狀態 |
| 2 | 建立新專案，位置挑桌面 | 桌面出現該資料夾，內含 `<名稱>.aitprj` 與 `tasks.db` |
| 3 | 進入專案、建一張卡片、拖到「待執行」 | 卡片被派工，開出終端機分頁 |
| 4 | 回專案總覽 | 該專案顯示執行中數量 |
| 5 | 開啟第二個專案，在它裡面也排一張卡 | 兩個專案的卡片都會被派工 |
| 6 | 人在專案 A 時看分頁列 | 專案 B 的分頁上有執行中指示點 |
| 7 | 關閉專案 A 的分頁 | 分頁消失，**但專案仍在總覽清單中，磁碟資料夾還在，卡片照樣繼續跑** |
| 8 | 完全關閉 App 再開 | 分頁列還原成關閉前的狀態 |
| 9 | 在 Finder 把某個專案資料夾改名，回 App 重新整理 | 該專案顯示「找不到專案資料夾」，其他專案正常 |
| 10 | 把一個專案資料夾複製到別處，用「開啟現有專案」挑它的 `.aitprj` | 被拒絕（id 已在清單中）——這是本里程碑的預期行為，複本合併留給後續匯入里程碑 |
| 11 | 移除專案、第二段選「否」 | 從清單消失，磁碟資料夾還在 |
| 12 | 移除專案、第二段選「是」 | 從清單消失，磁碟資料夾也不見了 |
| 13 | 有工作執行中時移除該專案 | 被擋下並提示先停止工作 |
| 14 | 新增工作時看目錄欄 | 這個專案用過的目錄以按鈕列出，點了會填入 |

- [ ] **Step 3: 回報**

把第 2 步表格中任何不符預期的項目列出來，連同實際看到的行為。不要自行擴大修改範圍。

---

## 完成後

依 CLAUDE.md，實作完成時使用 `superpowers:verification-before-completion`，合併前使用 `superpowers:requesting-code-review`。

本里程碑**不含**（各自獨立成後續里程碑，不要順手做）：

- AI 工作報告
- 跨機器匯入的 `project_dir` 路徑重新對應
- 同一專案複本的 id 衝突合併
- 專案層級的並行上限（維持全域）
- 暫停／關閉專案的開關
