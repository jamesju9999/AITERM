# 工作看板派工可選 Claude Bridge / 帳號組合 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 工作看板派工時，卡片可以個別選擇「直連 Anthropic / 走橋接（沿用目前設定）/ 走橋接：某個帳號組合」，派工當下據此決定要不要注入 Claude Bridge 的環境變數，並在指定帳號組合時先把快照套用成全域橋接設定。

**Architecture:** `TaskRow` 新增 `use_bridge`/`bridge_tiers`（凍結快照，不是 profile 參照）。新增一個純函式 `dispatch::resolve_bridge_env`（吃 `&ConfigStore` + `&TaskRow` + 橋接 server 的 port/token，回傳 `Option<(u16,String)>`），跟既有的 `pty_create` command 邏輯逐字一致，且完全不需要 `AppHandle` 就能單元測試。`RealDispatcher::dispatch` 呼叫它，把結果餵給 `spawn_and_run` 新增的參數。前端 `TaskEditorDialog.tsx` 新增下拉，選定的帳號組合在存檔當下（有 `localStorage` 存取權的唯一時機）解析成 JSON 快照存進卡片。

**Tech Stack:** Rust（sqlx + tokio test）、React 19 + TypeScript（Vitest + RTL）。

**規格：** `docs/superpowers/specs/2026-09-08-task-dispatch-bridge-design.md`

**已知限制（沿用既有先例，不是這次遺漏）：** 這個專案的 `tauri` 依賴沒開 `test` feature（`Cargo.toml` 裡 `features = []`，`dispatch.rs` 現有測試模組第 798-804 行的註解記錄過一次實際編譯失敗的確認過程）。這代表任何需要真的 `&AppHandle` 或 `State<'_, T>` 的函式（`spawn_and_run` 本身、`RealDispatcher::dispatch`、Tauri command 如 `tasks_create`/`tasks_update`）都無法直接單元測試——這是既有的專案限制，不是這次改動引入的。因此這次的測試策略比照現有慣例：把決策邏輯抽成不需要 `AppHandle` 的純函式（`resolve_bridge_env`）並完整測試它；`spawn_and_run`/`RealDispatcher::dispatch`/command 層只是把已測過的值原樣轉發，屬於「一行、肉眼可核對」的改動，用 `cargo build` 確認能編譯即可。

---

### Task 1: Schema migration + `TaskRow` 欄位 + `set_bridge_config`

**Files:**
- Modify: `src-tauri/src/tasks/mod.rs`（`init_schema`，約第 35-83 行）
- Modify: `src-tauri/src/tasks/store.rs`（`TaskRow` 結構，約第 14-42 行；新函式放在 `set_interactive` 之後，約第 250 行）

- [ ] **Step 1: 加 schema 欄位**

在 `src-tauri/src/tasks/mod.rs` 的 `CREATE TABLE IF NOT EXISTS tasks (...)` 裡，`session_path TEXT` 那行後面加兩欄：

```rust
            session_id      TEXT,
            session_path    TEXT,
            use_bridge      INTEGER NOT NULL DEFAULT 0,
            bridge_tiers    TEXT
        )",
```

在現有的 `ALTER TABLE tasks ADD COLUMN session_path TEXT` 那行（第 81-83 行）後面加：

```rust
    // Migration: existing databases created before `use_bridge`/`bridge_tiers`
    // existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN use_bridge INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN bridge_tiers TEXT")
        .execute(pool)
        .await;
```

- [ ] **Step 2: `TaskRow` 加欄位**

在 `src-tauri/src/tasks/store.rs` 的 `TaskRow` 結構裡，`session_path: Option<String>,` 那行（結構定義的最後一個欄位）後面加：

```rust
    /// 這張卡派工時要不要走 Claude Bridge。預設 `false`＝直連 Anthropic。
    pub use_bridge: bool,
    /// 選了帳號組合時，存檔當下解析出的 `{opus,sonnet,haiku}` JSON 快照
    /// （形狀對齊 `ClaudeBridgeConfig` 的 tier 子集）。`None` 且
    /// `use_bridge=true` 代表「沿用當下的全域橋接設定」，不覆寫任何 tier。
    pub bridge_tiers: Option<String>,
```

- [ ] **Step 3: 寫失敗測試**

在 `src-tauri/src/tasks/store.rs` 的 `mod tests` 區塊（`mem_pool()` 定義之後任意位置，例如緊接在 `create_then_list_roundtrips_a_planning_card` 之後）加：

```rust
    #[tokio::test]
    async fn new_card_defaults_to_no_bridge() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert!(!row.use_bridge);
        assert!(row.bridge_tiers.is_none());
    }

    #[tokio::test]
    async fn set_bridge_config_stores_use_bridge_and_snapshot() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();

        set_bridge_config(&pool, &id, true, Some(r#"{"opus":null,"sonnet":null,"haiku":null}"#.to_string()))
            .await
            .unwrap();

        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.use_bridge);
        assert_eq!(row.bridge_tiers.as_deref(), Some(r#"{"opus":null,"sonnet":null,"haiku":null}"#));
    }

    #[tokio::test]
    async fn set_bridge_config_can_clear_back_to_direct() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        set_bridge_config(&pool, &id, true, None).await.unwrap();

        set_bridge_config(&pool, &id, false, None).await.unwrap();

        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert!(!row.use_bridge);
        assert!(row.bridge_tiers.is_none());
    }
```

- [ ] **Step 4: 執行測試確認失敗**

Run: `cd src-tauri && cargo test tasks::store::tests::set_bridge_config -- --nocapture`
Expected: FAIL — `cannot find function 'set_bridge_config' in this scope`（`new_card_defaults_to_no_bridge` 會因為 `TaskRow` 已經有欄位而直接編過、通過；只有用到 `set_bridge_config` 的兩個測試會編譯失敗）

- [ ] **Step 5: 實作 `set_bridge_config`**

在 `src-tauri/src/tasks/store.rs` 的 `set_interactive` 函式（約第 240-251 行）後面加：

```rust
/// 派工方式：要不要走橋接，以及（可選的）凍結的帳號組合快照。跟
/// `parallel_ok`/`interactive` 一樣隨時可改，不受 `edit_allowed` 限制。
pub async fn set_bridge_config(
    pool: &SqlitePool,
    id: &str,
    use_bridge: bool,
    bridge_tiers: Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET use_bridge = ?, bridge_tiers = ? WHERE id = ?")
        .bind(use_bridge as i64)
        .bind(bridge_tiers)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 6: 執行測試確認通過**

Run: `cd src-tauri && cargo test tasks::store::tests:: -- --nocapture`
Expected: PASS（既有的 store 測試 + 新增的 3 個都綠燈）

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/tasks/mod.rs src-tauri/src/tasks/store.rs
git commit -m "$(cat <<'EOF'
feat(tasks): add use_bridge/bridge_tiers columns to TaskRow

Additive migration (CREATE TABLE + best-effort ALTER TABLE, same
pattern as `interactive`/`ai_summary`). set_bridge_config mirrors
set_parallel_ok/set_interactive: always editable, not gated by
edit_allowed.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 2: `resolve_bridge_env` 純函式（派工決策邏輯）

**Files:**
- Modify: `src-tauri/src/tasks/dispatch.rs`（新函式放在 `build_prompt` 之後，約第 24 行；測試放在既有 `mod tests` 區塊）

這個函式吃 `&ConfigStore` 而不是 `Arc<BridgeState>`/`Arc<SecretStore>`：呼叫端自己把 `bridge.port()`、`secrets.get(...)` 的結果算好、以純值傳進來，函式本身才能在沒有真的橋接 server / keychain 的情況下完整單元測試。

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/src/tasks/dispatch.rs` 的 `mod tests` 區塊裡（`use super::*;` 之後任意位置）加：

```rust
    mod bridge_env_tests {
        // `ConfigStore`/`TierMapping`/`TaskRow` 已經透過 `use super::*` 從外層
        // （`dispatch.rs` 本體在 Step 3 加的 imports）帶進來，這裡不用重複 import。
        use super::*;
        use crate::tasks::store;
        use sqlx::sqlite::SqlitePoolOptions;

        async fn mem_pool() -> sqlx::SqlitePool {
            let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
            crate::tasks::init_schema(&pool).await.unwrap();
            pool
        }

        async fn task_with_bridge(
            pool: &sqlx::SqlitePool,
            use_bridge: bool,
            tiers: Option<&str>,
        ) -> store::TaskRow {
            let id = store::create_task(pool, "t", "", "/r", true, false).await.unwrap();
            store::set_bridge_config(pool, &id, use_bridge, tiers.map(|s| s.to_string()))
                .await
                .unwrap();
            store::get_task(pool, &id).await.unwrap().unwrap()
        }

        fn temp_config() -> (tempfile::TempDir, ConfigStore) {
            let dir = tempfile::tempdir().unwrap();
            let config = ConfigStore::new_at(dir.path().join("config.toml"));
            (dir, config)
        }

        #[tokio::test]
        async fn no_env_when_task_does_not_want_bridge() {
            let pool = mem_pool().await;
            let task = task_with_bridge(&pool, false, None).await;
            let (_dir, config) = temp_config();

            let env = resolve_bridge_env(&config, &task, Some(8317), Some("tok".to_string()));
            assert_eq!(env, None);
        }

        #[tokio::test]
        async fn no_env_when_bridge_server_not_running() {
            let pool = mem_pool().await;
            let task = task_with_bridge(&pool, true, None).await;
            let (_dir, config) = temp_config();

            let env = resolve_bridge_env(&config, &task, None, None);
            assert_eq!(env, None, "server 沒在跑就不該注入，跟 pty_create 現有行為一致");
        }

        #[tokio::test]
        async fn returns_port_and_token_when_wanted_and_server_running() {
            let pool = mem_pool().await;
            let task = task_with_bridge(&pool, true, None).await;
            let (_dir, config) = temp_config();

            let env = resolve_bridge_env(&config, &task, Some(8317), Some("tok".to_string()));
            assert_eq!(env, Some((8317, "tok".to_string())));
        }

        #[tokio::test]
        async fn snapshot_overwrites_global_tier_config_before_dispatch() {
            let pool = mem_pool().await;
            let snapshot =
                r#"{"opus":{"provider_id":"acct-b","model":"m-b"},"sonnet":null,"haiku":null}"#;
            let task = task_with_bridge(&pool, true, Some(snapshot)).await;
            let (_dir, config) = temp_config();
            config
                .update(|c| {
                    c.claude_bridge.opus =
                        Some(TierMapping { provider_id: "acct-a".into(), model: "m-a".into() });
                })
                .unwrap();

            resolve_bridge_env(&config, &task, Some(8317), Some("tok".to_string()));

            let opus = config.get().claude_bridge.opus.unwrap();
            assert_eq!(opus.provider_id, "acct-b");
            assert_eq!(opus.model, "m-b");
        }

        #[tokio::test]
        async fn null_snapshot_leaves_global_tier_config_untouched() {
            let pool = mem_pool().await;
            let task = task_with_bridge(&pool, true, None).await;
            let (_dir, config) = temp_config();
            config
                .update(|c| {
                    c.claude_bridge.opus =
                        Some(TierMapping { provider_id: "acct-a".into(), model: "m-a".into() });
                })
                .unwrap();

            resolve_bridge_env(&config, &task, Some(8317), Some("tok".to_string()));

            let opus = config.get().claude_bridge.opus.unwrap();
            assert_eq!(opus.provider_id, "acct-a", "bridge_tiers 是 None 時不該覆寫既有設定");
        }
    }
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test tasks::dispatch::tests::bridge_env_tests -- --nocapture`
Expected: FAIL — `cannot find function 'resolve_bridge_env' in this scope`

- [ ] **Step 3: 實作 `resolve_bridge_env`**

在 `src-tauri/src/tasks/dispatch.rs` 頂部 import 區塊（第 6-13 行）加：

```rust
use crate::config::{ConfigStore, TierMapping};
use crate::tasks::store::TaskRow;
```

在 `build_prompt` 函式（第 17-23 行）後面加：

```rust
/// 派工前要不要注入 Claude Bridge 的環境變數，以及要不要先把這張卡指定
/// 的帳號組合快照套用成全域橋接設定。
///
/// 吃純值（`bridge_port`/`bridge_token`）而不是 `Arc<BridgeState>`/
/// `Arc<SecretStore>`——呼叫端自己解析好再傳進來，這個函式才不需要真的
/// 橋接 server 或 OS keychain 就能完整測試。降級邏輯跟 `pty_create`
/// command（`pty/commands.rs`）現有邏輯逐字一致：server 沒在跑就不注入。
#[derive(serde::Deserialize)]
struct BridgeTierSnapshot {
    opus: Option<TierMapping>,
    sonnet: Option<TierMapping>,
    haiku: Option<TierMapping>,
}

pub fn resolve_bridge_env(
    config: &ConfigStore,
    task: &TaskRow,
    bridge_port: Option<u16>,
    bridge_token: Option<String>,
) -> Option<(u16, String)> {
    if task.use_bridge {
        if let Some(json) = &task.bridge_tiers {
            if let Ok(snap) = serde_json::from_str::<BridgeTierSnapshot>(json) {
                // `update` 就算磁碟寫入失敗，記憶體內的設定仍然會被改到
                // （`ConfigStore::update` 的實作先套用閉包、才存檔）——這裡
                // 在乎的是「這次派工當下」讀到的即時設定，忽略持久化失敗
                // 比讓整個派工失敗更合理。
                let _ = config.update(|c| {
                    c.claude_bridge.opus = snap.opus;
                    c.claude_bridge.sonnet = snap.sonnet;
                    c.claude_bridge.haiku = snap.haiku;
                });
            }
        }
    }
    match (task.use_bridge, bridge_port) {
        (true, Some(port)) => bridge_token.map(|t| (port, t)),
        _ => None,
    }
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test tasks::dispatch::tests::bridge_env_tests -- --nocapture`
Expected: PASS（5 個測試都綠燈）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/dispatch.rs
git commit -m "$(cat <<'EOF'
feat(tasks): add resolve_bridge_env — dispatch-time bridge decision

Pure function (no AppHandle needed): given a task's use_bridge/
bridge_tiers and the bridge server's current port/token, decides
whether to inject bridge env vars, applying the task's frozen tier
snapshot to the global ClaudeBridgeConfig first when present. Mirrors
pty_create's existing fallback (no injection when the server isn't
running) exactly.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 3: 串起派工路徑（`spawn_and_run` / `RealDispatcher`）

**Files:**
- Modify: `src-tauri/src/tasks/dispatch.rs`（`spawn_and_run`，約第 300-332 行）
- Modify: `src-tauri/src/tasks/scheduler.rs`（`RealDispatcher` 結構與 `dispatch` 實作，約第 77-125 行；`spawn` 函式建構處，約第 407-439 行）

這個 task 沒有新增自動化測試——`spawn_and_run`/`RealDispatcher::dispatch` 都需要真的 `&AppHandle`，而這個專案的 `tauri` 依賴沒開 `test` feature（見本文件開頭「已知限制」）。決策邏輯已經在 Task 2 測過；這裡純粹是把已測過的值原樣轉發，用編譯通過＋人工核對確認正確。

- [ ] **Step 1: `spawn_and_run` 簽名加 `bridge_env` 參數**

在 `src-tauri/src/tasks/dispatch.rs`，把：

```rust
pub async fn spawn_and_run(
    app: &AppHandle,
    pty: &PtyManager,
    project_dir: &str,
    claude_command: &str,
    session_id: Option<&str>,
    prompt: &str,
    request_done_marker: bool,
) -> Result<(String, DispatchResult), String> {
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let tab_id = pty
        .create_with_app(app.clone(), size, Some(std::path::PathBuf::from(project_dir)), None)
        .map_err(|e| e.to_string())?;
```

改成：

```rust
pub async fn spawn_and_run(
    app: &AppHandle,
    pty: &PtyManager,
    project_dir: &str,
    claude_command: &str,
    session_id: Option<&str>,
    prompt: &str,
    request_done_marker: bool,
    bridge_env: Option<(u16, String)>,
) -> Result<(String, DispatchResult), String> {
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let tab_id = pty
        .create_with_app(
            app.clone(),
            size,
            Some(std::path::PathBuf::from(project_dir)),
            bridge_env,
        )
        .map_err(|e| e.to_string())?;
```

- [ ] **Step 2: 更新 `spawn_and_run` 既有測試呼叫端**

`src-tauri/src/tasks/dispatch.rs` 的 `mod tests` 裡搜尋 `spawn_and_run(` 的呼叫（如果有的話——目前這個函式因為需要 `AppHandle` 應該沒有直接呼叫端在測試裡；若 `grep -n "spawn_and_run(" src-tauri/src/tasks/dispatch.rs` 只找到定義本身，這步不用改任何東西，直接跳到 Step 3）。

- [ ] **Step 3: `RealDispatcher` 新增欄位**

在 `src-tauri/src/tasks/scheduler.rs`，把：

```rust
pub struct RealDispatcher {
    pub app: AppHandle,
    pub pty: Arc<PtyManager>,
    pub config: Arc<ConfigStore>,
    pub wake: Arc<Notify>,
    /// task_id → cancel sender, so `tasks_stop` can abort a running watch.
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<monitor::WatchControl>>>>,
}
```

改成：

```rust
pub struct RealDispatcher {
    pub app: AppHandle,
    pub pty: Arc<PtyManager>,
    pub config: Arc<ConfigStore>,
    pub bridge: Arc<crate::bridge::BridgeState>,
    pub secrets: Arc<crate::secret::SecretStore>,
    pub wake: Arc<Notify>,
    /// task_id → cancel sender, so `tasks_stop` can abort a running watch.
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<monitor::WatchControl>>>>,
}
```

- [ ] **Step 4: `dispatch` 實作裡算出 `bridge_env` 並傳給 `spawn_and_run`**

在 `src-tauri/src/tasks/scheduler.rs` 的 `impl Dispatcher for RealDispatcher`，把：

```rust
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
```

改成：

```rust
        let bridge_env = dispatch::resolve_bridge_env(
            &self.config,
            task,
            self.bridge.port(),
            self.secrets.get(crate::bridge::auth::BRIDGE_TOKEN_KEY).ok().flatten(),
        );
        let (tab_id, disp) = dispatch::spawn_and_run(
            &self.app,
            &self.pty,
            &task.project_dir,
            &claude_cmd,
            session_id.as_deref(),
            &prompt,
            !task.interactive,
            bridge_env,
        )
        .await?;
```

- [ ] **Step 5: `spawn` 函式建構 `RealDispatcher` 時補新欄位**

在 `src-tauri/src/tasks/scheduler.rs::spawn`，把：

```rust
        let config = app.state::<Arc<ConfigStore>>().inner().clone();
        let pty = app.state::<Arc<PtyManager>>().inner().clone();
```

改成：

```rust
        let config = app.state::<Arc<ConfigStore>>().inner().clone();
        let pty = app.state::<Arc<PtyManager>>().inner().clone();
        let bridge = app.state::<Arc<crate::bridge::BridgeState>>().inner().clone();
        let secrets = app.state::<Arc<crate::secret::SecretStore>>().inner().clone();
```

把：

```rust
        let dispatcher = RealDispatcher {
            app: app.clone(),
            pty,
            config: config.clone(),
            wake: wake.clone(),
            cancels,
        };
```

改成：

```rust
        let dispatcher = RealDispatcher {
            app: app.clone(),
            pty,
            config: config.clone(),
            bridge,
            secrets,
            wake: wake.clone(),
            cancels,
        };
```

- [ ] **Step 6: 確認編譯通過**

Run: `cd src-tauri && cargo build`
Expected: 編譯成功，無錯誤。若報 `use of moved value` 之類的錯誤，檢查是否誤把 `bridge`/`secrets` 在 `RealDispatcher { ... }` 之後又用了一次（這兩個變數在 `spawn` 裡只使用一次，直接移入結構體即可，不用 `.clone()`）。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/tasks/dispatch.rs src-tauri/src/tasks/scheduler.rs
git commit -m "$(cat <<'EOF'
feat(tasks): wire resolve_bridge_env into the real dispatch path

RealDispatcher gains bridge/secrets handles (same State sources
pty_create already uses); dispatch() resolves the bridge_env before
spawning and forwards it through spawn_and_run's new parameter. No new
test here — both functions need a real AppHandle, which this project's
tauri dependency can't construct in tests (test feature not enabled);
the decision logic itself is fully covered by Task 2.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 4: Tauri command 層（`tasks_create` / `tasks_update`）

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs`（`CreateArgs`/`tasks_create`：約第 65-93 行；`UpdateArgs`/`tasks_update`：約第 95-136 行）

跟 Task 3 同理，`tasks_create`/`tasks_update` 是 `#[tauri::command]`，需要真的 `AppHandle`/`State`，這個專案沒開 `tauri` 的 `test` feature，所以不寫新的自動化測試——這兩個函式本來就是「thin delegate」（見檔案開頭模組註解），改動只是多轉發兩個欄位，用編譯通過＋人工核對確認正確。

- [ ] **Step 1: `CreateArgs` 加欄位，`tasks_create` 呼叫 `set_bridge_config`**

把：

```rust
pub struct CreateArgs {
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
}

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
    emit_updated(&app);
    Ok(id)
}
```

改成：

```rust
pub struct CreateArgs {
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
    pub use_bridge: bool,
    pub bridge_tiers: Option<String>,
}

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
    emit_updated(&app);
    Ok(id)
}
```

- [ ] **Step 2: `UpdateArgs` 加欄位，`tasks_update` 呼叫 `set_bridge_config`**

把：

```rust
pub struct UpdateArgs {
    pub id: String,
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
}

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
    if edit_allowed(&row.status) {
        store::update_task_fields(
            &p.pool,
            &args.id,
            &args.title,
            &args.body,
            &args.project_dir,
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    emit_updated(&app);
    Ok(())
}
```

改成：

```rust
pub struct UpdateArgs {
    pub id: String,
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
    pub use_bridge: bool,
    pub bridge_tiers: Option<String>,
}

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
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    emit_updated(&app);
    Ok(())
}
```

- [ ] **Step 3: 確認整個 crate 編譯通過**

Run: `cd src-tauri && cargo build`
Expected: 編譯成功。

- [ ] **Step 4: 跑一次完整 `cargo test` 確認沒有連帶弄壞既有測試**

Run: `cd src-tauri && cargo test`
Expected: 全部 PASS（Task 1、2 新增的測試 + 所有既有測試）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/tasks.rs
git commit -m "$(cat <<'EOF'
feat(tasks): expose use_bridge/bridge_tiers on the create/update commands

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 5: 前端型別（`src/ipc/tasks.ts`）

**Files:**
- Modify: `src/ipc/tasks.ts`

- [ ] **Step 1: `TaskRow` 加欄位**

把：

```typescript
  /** 複製進卡片資料夾的 session.jsonl 路徑。有值代表有完整的逐輪記錄。 */
  session_path: string | null;
}
```

改成：

```typescript
  /** 複製進卡片資料夾的 session.jsonl 路徑。有值代表有完整的逐輪記錄。 */
  session_path: string | null;
  /** 派工時要不要走 Claude Bridge。 */
  use_bridge: boolean;
  /**
   * 選了帳號組合時凍結的 `{opus,sonnet,haiku}` JSON 快照字串；`null` 代表
   * 「沿用當下的全域橋接設定」（`use_bridge=true` 時）或不適用
   * （`use_bridge=false` 時）。
   */
  bridge_tiers: string | null;
}
```

- [ ] **Step 2: `createTask`/`updateTask` 的 args 型別加欄位**

把：

```typescript
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
```

改成：

```typescript
export const createTask = (
  projectId: string,
  args: {
    title: string;
    body: string;
    project_dir: string;
    parallel_ok: boolean;
    interactive: boolean;
    use_bridge: boolean;
    bridge_tiers: string | null;
  },
): Promise<string> => invoke("tasks_create", { projectId, args });
```

把：

```typescript
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
```

改成：

```typescript
export const updateTask = (
  projectId: string,
  args: {
    id: string;
    title: string;
    body: string;
    project_dir: string;
    parallel_ok: boolean;
    interactive: boolean;
    use_bridge: boolean;
    bridge_tiers: string | null;
  },
): Promise<void> => invoke("tasks_update", { projectId, args });
```

- [ ] **Step 3: 型別檢查**

Run: `npx tsc -b`
Expected: 這一步會報錯——`TaskEditorDialog.tsx` 呼叫 `createTask`/`updateTask` 時還沒帶這兩個新的必要欄位。這是預期中的紅燈，Task 6 補上呼叫端後會消失。記下錯誤訊息（應該是類似 `Property 'use_bridge' is missing`），現在不用修。

- [ ] **Step 4: Commit**

```bash
git add src/ipc/tasks.ts
git commit -m "$(cat <<'EOF'
feat(tasks): add use_bridge/bridge_tiers to the frontend TaskRow types

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 6: i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts`（zh-TW 區塊約第 41-44 行附近；en 區塊約第 1539-1542 行附近）

- [ ] **Step 1: zh-TW 字串**

在 `src/lib/i18n.ts` 的 `board_card_solo_hint: "關閉＝必須單獨執行（執行時不會有其他任務一起跑）",` 那行（zh-TW 區塊）後面加：

```typescript
    task_bridge_label: "派工方式",
    task_bridge_direct: "直連 Anthropic（預設）",
    task_bridge_current: "走橋接（沿用目前設定）",
    task_bridge_custom: "走橋接（此卡自訂組合）",
    task_bridge_profile_option: (name: string) => `走橋接：${name}`,
```

- [ ] **Step 2: en 字串**

在 en 區塊對應位置（`board_card_solo_hint: "Off = must run alone (nothing else runs while it does)",` 那行後面，用 `grep -n "board_card_solo_hint" src/lib/i18n.ts` 確認實際行號）加：

```typescript
    task_bridge_label: "Dispatch via",
    task_bridge_direct: "Direct to Anthropic (default)",
    task_bridge_current: "Claude Bridge (use current settings)",
    task_bridge_custom: "Claude Bridge (custom for this card)",
    task_bridge_profile_option: (name: string) => `Claude Bridge: ${name}`,
```

- [ ] **Step 3: 型別檢查確認兩邊 key 對齊**

Run: `npx tsc -b`
Expected: 跟 Task 5 一樣，還是會報 `TaskEditorDialog.tsx` 缺少 `use_bridge`/`bridge_tiers` 呼叫參數的錯——這步只是確認新加的 i18n key 本身沒有拼字/型別問題（en/zh-TW 兩邊都有的話，這批新 key 不會是報錯訊息裡提到的項目）。

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "$(cat <<'EOF'
feat(i18n): add task_bridge_* strings for the dispatch-via picker

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 7: `TaskEditorDialog.tsx` 下拉選單

**Files:**
- Modify: `src/components/TaskBoard/TaskEditorDialog.tsx`
- Test: `src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx`（新檔案，跟既有的 `.refine.test.tsx`/`.usedDirs.test.tsx` 同一種按功能拆檔的慣例）

- [ ] **Step 1: 寫失敗測試**

```tsx
// src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedDirs = vi.fn();
const createTask = vi.fn();
const updateTask = vi.fn();
vi.mock("../../ipc/projects", () => ({ usedDirs: (...a: unknown[]) => usedDirs(...a) }));
vi.mock("../../ipc/tasks", () => ({
  createTask: (...a: unknown[]) => createTask(...a),
  updateTask: (...a: unknown[]) => updateTask(...a),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { saveBridgeProfiles } from "../Settings/bridgeProfiles";
import { TaskEditorDialog } from "./TaskEditorDialog";
import type { TaskWithAttachments } from "../../ipc/tasks";

const PROFILE = {
  id: "p1",
  name: "個人帳號",
  opus: { provider_id: "acct-a", model: "m-a" },
  sonnet: null,
  haiku: null,
};

const mount = (card: TaskWithAttachments | null = null) =>
  render(
    <LocaleProvider>
      <TaskEditorDialog projectId="p1" card={card} onClose={vi.fn()} onSaved={vi.fn()} />
    </LocaleProvider>,
  );

const BASE_CARD: TaskWithAttachments = {
  id: "t1",
  title: "既有卡片",
  body: "body",
  project_dir: "/repo",
  status: "planning",
  parallel_ok: true,
  interactive: false,
  sort_order: 1,
  outcome: null,
  tab_id: null,
  transcript_path: null,
  error_message: null,
  created_at: "2026-01-01",
  dispatched_at: null,
  finished_at: null,
  ai_summary: null,
  archived_at: null,
  session_id: null,
  session_path: null,
  use_bridge: false,
  bridge_tiers: null,
  attachments: [],
};

describe("TaskEditorDialog 派工方式", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    usedDirs.mockResolvedValue([]);
  });

  it("預設是「直連 Anthropic」，選單列出已存的帳號組合", async () => {
    saveBridgeProfiles([PROFILE]);
    mount();
    const select = await screen.findByTestId("task-bridge-select");
    expect(select).toHaveValue("direct");
    expect(screen.getByRole("option", { name: "走橋接：個人帳號" })).toBeInTheDocument();
  });

  it("選帳號組合存檔時，帶入該組合解析後的 JSON 快照", async () => {
    saveBridgeProfiles([PROFILE]);
    const user = userEvent.setup();
    mount();
    await user.type(await screen.findByTestId("task-title-input"), "t");
    await user.type(screen.getByTestId("task-dir-input"), "/repo");
    await user.selectOptions(screen.getByTestId("task-bridge-select"), "p1");

    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));

    expect(createTask).toHaveBeenCalledTimes(1);
    const args = createTask.mock.calls[0][1];
    expect(args.use_bridge).toBe(true);
    expect(JSON.parse(args.bridge_tiers)).toEqual({
      opus: PROFILE.opus,
      sonnet: PROFILE.sonnet,
      haiku: PROFILE.haiku,
    });
  });

  it("選「走橋接（沿用目前設定）」存檔時 bridge_tiers 是 null", async () => {
    const user = userEvent.setup();
    mount();
    await user.type(await screen.findByTestId("task-title-input"), "t");
    await user.type(screen.getByTestId("task-dir-input"), "/repo");
    await user.selectOptions(screen.getByTestId("task-bridge-select"), "current");

    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));

    const args = createTask.mock.calls[0][1];
    expect(args.use_bridge).toBe(true);
    expect(args.bridge_tiers).toBeNull();
  });

  it("重新編輯：快照跟現存的帳號組合匹配時，預選該組合", async () => {
    saveBridgeProfiles([PROFILE]);
    const card = {
      ...BASE_CARD,
      use_bridge: true,
      bridge_tiers: JSON.stringify({ opus: PROFILE.opus, sonnet: null, haiku: null }),
    };
    mount(card);
    const select = await screen.findByTestId("task-bridge-select");
    expect(select).toHaveValue("p1");
  });

  it("重新編輯：快照跟任何現存組合都對不上時，顯示自訂組合、不強迫重選", async () => {
    const card = {
      ...BASE_CARD,
      use_bridge: true,
      bridge_tiers: JSON.stringify({
        opus: { provider_id: "acct-deleted", model: "m" },
        sonnet: null,
        haiku: null,
      }),
    };
    mount(card);
    const select = await screen.findByTestId("task-bridge-select");
    expect(select).toHaveValue("custom");
    expect(screen.getByRole("option", { name: /此卡自訂組合|custom for this card/ })).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx`
Expected: FAIL —找不到 `data-testid="task-bridge-select"`（下拉還不存在）。

（這個測試檔案假設 `task-title-input`/`task-dir-input` 這兩個 `data-testid` 已經存在於 `TaskEditorDialog.tsx`；如果實際執行時發現名稱不同，用 `grep -n "data-testid=\"task-" src/components/TaskBoard/TaskEditorDialog.tsx` 核對實際名稱並修正測試。）

- [ ] **Step 3: 實作下拉選單**

在 `src/components/TaskBoard/TaskEditorDialog.tsx` 頂部 import 區塊，`import { useRefineTask } from "./useRefineTask";` 後面加：

```tsx
import {
  loadBridgeProfiles,
  tiersEqual,
  type BridgeProfile,
} from "../Settings/bridgeProfiles";
import type { TierMapping } from "../../ipc/bridge";
```

在 `const [interactive, setInteractive] = useState(card?.interactive ?? false);`（第 39 行）後面加：

```tsx
  const [profiles] = useState<BridgeProfile[]>(() => loadBridgeProfiles());
  const [bridgeChoice, setBridgeChoice] = useState<string>(() => {
    if (!card?.use_bridge) return "direct";
    if (!card.bridge_tiers) return "current";
    try {
      const snap = JSON.parse(card.bridge_tiers) as {
        opus: TierMapping | null;
        sonnet: TierMapping | null;
        haiku: TierMapping | null;
      };
      const match = profiles.find((p) => tiersEqual(snap, p));
      return match ? match.id : "custom";
    } catch {
      return "custom";
    }
  });
```

在 `save` 函式（第 128 行）裡，把：

```tsx
      if (isEdit) {
        await updateTask(projectId, {
          id: card.id,
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
        });
      } else {
        const newId = await createTask(projectId, {
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
        });
```

改成：

```tsx
      const bridgeArgs = (() => {
        if (bridgeChoice === "direct") return { use_bridge: false, bridge_tiers: null };
        if (bridgeChoice === "current") return { use_bridge: true, bridge_tiers: null };
        if (bridgeChoice === "custom") {
          return { use_bridge: true, bridge_tiers: card?.bridge_tiers ?? null };
        }
        const profile = profiles.find((p) => p.id === bridgeChoice);
        if (!profile) return { use_bridge: true, bridge_tiers: null };
        return {
          use_bridge: true,
          bridge_tiers: JSON.stringify({
            opus: profile.opus,
            sonnet: profile.sonnet,
            haiku: profile.haiku,
          }),
        };
      })();
      if (isEdit) {
        await updateTask(projectId, {
          id: card.id,
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
          ...bridgeArgs,
        });
      } else {
        const newId = await createTask(projectId, {
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
          ...bridgeArgs,
        });
```

在 JSX 裡，`interactive`/`parallelOk` 那個 `<div className="task-dialog-group">...</div>` 區塊（第 292-319 行）後面、附件區塊（第 321 行 `<div className="task-dialog-group">`）之前，加一個新區塊：

```tsx
        <div className="task-dialog-group">
        <label className="task-field">
          <span className="task-field-label">{t.task_bridge_label}</span>
          <select
            className="task-field-input"
            data-testid="task-bridge-select"
            value={bridgeChoice}
            onChange={(e) => setBridgeChoice(e.target.value)}
          >
            <option value="direct">{t.task_bridge_direct}</option>
            <option value="current">{t.task_bridge_current}</option>
            {profiles.map((p) => (
              <option key={p.id} value={p.id}>
                {t.task_bridge_profile_option(p.name)}
              </option>
            ))}
            {bridgeChoice === "custom" && (
              <option value="custom">{t.task_bridge_custom}</option>
            )}
          </select>
        </label>
        </div>

```

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx`
Expected: PASS（5 個測試）

- [ ] **Step 5: 確認沒有弄壞同一個元件的其他既有測試**

Run: `npx vitest run src/components/TaskBoard/TaskEditorDialog`
Expected: PASS（`.refine.test.tsx`、`.usedDirs.test.tsx`、剛新增的 `.bridge.test.tsx` 全部綠燈）

- [ ] **Step 6: Commit**

```bash
git add src/components/TaskBoard/TaskEditorDialog.tsx src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx
git commit -m "$(cat <<'EOF'
feat(tasks): add dispatch-via picker to TaskEditorDialog

Reuses the bridge-profiles localStorage helpers from Settings — picking
a profile freezes its tier mapping into the card at save time (the
scheduler backend never touches localStorage or the profile concept).
Reopening a card whose snapshot still matches a live profile preselects
it; a snapshot that no longer matches anything shows as a generic
custom option instead of forcing a reselect.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KCiBfURnSHp4US8zS9dCFk
EOF
)"
```

---

### Task 8: 全套驗證

**Files:** 無新增/修改，純驗證。

- [ ] **Step 1: Rust 全套測試**

Run: `cd src-tauri && cargo test`
Expected: 全部 PASS。

- [ ] **Step 2: 前端全套測試**

Run: `npm run test`
Expected: 全部 PASS（既有測試 + 這次新增的都綠燈；忽略跟這次改動無關的既有 pre-existing unhandled-rejection 警告，例如 `TaskEditorDialog.usedDirs.test.tsx` 本來就沒 mock `listProviders` 造成的噪音——那是既有狀況，不是這次引入的）。

- [ ] **Step 3: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤。

- [ ] **Step 4: Lint**

Run: `npm run lint`
Expected: 這次改動的檔案（`TaskEditorDialog.tsx`、`ipc/tasks.ts`、`i18n.ts`、`store.rs`、`dispatch.rs`、`scheduler.rs`、`commands/tasks.rs`）不應出現新的 lint 錯誤。專案裡可能已經有跟這次改動無關的既有 lint 錯誤（其他檔案），那些不是這次的責任，不用修。

- [ ] **Step 5: 若以上任一步驟失敗，回到對應 Task 修正，不要略過**

- [ ] **Step 6: 全部通過後跟使用者回報完成狀態**，附上每個驗證指令的結果摘要。每個 Task 已經各自 commit 過，這步不需要額外 commit。
