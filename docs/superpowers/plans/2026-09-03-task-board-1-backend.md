# 工作看板 — 後端（Plan 1/2）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Rust backend for a task board that dispatches queued cards to `claude` — a `tasks.db` store, a long-lived scheduler, dispatch (spawn a visible PTY tab running `claude` + send the composed prompt), and completion monitoring — all driven by Tauri commands + a `tasks://updated` event. No frontend in this plan.

**Architecture:** New `src-tauri/src/tasks/` module: `store.rs` (SQLx CRUD over a dedicated `tasks.db`, same pattern as `db/loop_sessions.rs`), `dispatch.rs` (compose prompt, spawn tab via the existing `PtyManager::create_with_app` + `mcp-coordination-tab-spawned` event, type the prompt in with the same CR-terminated / done-marker sequencing `coordination_ops::send_input` already uses), `monitor.rs` (per-running-task async watcher: bell/marker → success, non-zero OSC133 exit → failed, 120s output-quiet → failed-stuck, cancel → cancelled), `scheduler.rs` (`pick_next` pure selection fn + a `tokio` loop woken by a `Notify`, with a startup recovery scan). `PtySession` gains two accessors it doesn't have yet: last OSC133 exit code, and ms-since-last-output.

**Tech Stack:** Rust — sqlx (SQLite), portable-pty, tokio, tauri. Same stack as the existing MCP tool server and the `db/*` modules. Tests: `cargo test` (unit + `tests/*.rs` integration with `tempfile` + fake shell scripts, same approach as `tests/mcp_tool_server.rs`).

---

## Context for the implementing engineer

This plan assumes zero prior context for this codebase. Read `docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md` first. Facts below were verified by reading the real code during planning:

- **`PtyManager` is already `Arc`-wrapped** (`lib.rs:188` — `.manage(Arc::new(PtyManager::new()))`), with delegating accessors `write`, `close`, `get_recent_output`, `get_recent_raw`, `bell_count(id) -> Option<u64>`, `marker_count(id) -> Option<u64>`, `create_with_app(app, size, cwd, bridge_env) -> PtyResult<String>`, `create_with_callback(size, on_data) -> PtyResult<String>` (tests use the latter — no `AppHandle` needed). See `src-tauri/src/pty/manager.rs`.
- **`PtySession` (`src-tauri/src/pty/session.rs`) has NO "last exit code" and NO "time since last output" accessor.** Its reader thread (inside `spawn_with_id`, the single shared spawn path — `spawn` is a thin wrapper) calls `confirm_pending_cds_from_output(chunk, ...)` which runs `cd_parser::find_exit_codes(chunk)` purely for cd confirmation and drops the codes. Task 1 adds both accessors, mirroring the existing `bell_count`/`marker_count` pattern (an `Arc<AtomicU64>` written in the reader loop, read via a `pub fn`).
- **`cd_parser::find_exit_codes(data: &[u8]) -> Vec<i32>`** (`src-tauri/src/pty/cd_parser.rs:283`) extracts exit codes from OSC 133 `D;<code>` markers. `contains_bare_bell(data) -> bool` (line 339) is bell detection that already excludes the BEL-terminated OSC 133 `A` marker.
- **`pty/session.rs` already exposes**: `pub const DONE_MARKER_PREFIX: &str = "<<AITERM_DONE:"`, `pub const DONE_MARKER_SUFFIX: &str = ">>"`, `pub fn done_marker(tab_id: &str) -> String`. Task 2 adds `done_marker_instruction(tab_id: &str) -> String` next to them.
- **`coordination_ops::send_input`** (`src-tauri/src/mcp_server/coordination_ops.rs:146`) is the reference for typing text into a `claude` TUI safely: write `format!("{text}\r")` (CR, never LF — raw-mode TUIs only honor CR; there's a dedicated regression test), then optionally, once the target rings a fresh bell (`wait_for_new_bell`, private module fn at line 241, `DONE_MARKER_WAIT_SECONDS = 15`), write the done-marker instruction as a **second independent** `\r`-terminated write. Task 2 makes `wait_for_new_bell` `pub(crate)`. The instruction wording deliberately never contains the full contiguous marker string (canonical-mode echo would self-trigger `marker_count`); `done_marker_instruction` preserves that.
- **`coordination_ops::spawn_tab`** (line 111) is the reference for making a backend-spawned PTY visible in the UI: `pty_manager.create_with_app(app.clone(), PtySize { rows: 24, cols: 80, .. }, cwd_path, None)`, then `app.emit("mcp-coordination-tab-spawned", TabSpawnedEvent { session_id, command })`. The frontend already listens for this event and adopts the session as a real tab (that wiring is done — Plan 2 only reads task state, it does not re-implement tab adoption).
- **Dedicated-SQLite pattern** (`src-tauri/src/db/loop_sessions.rs`, `db/knowledge_base.rs`): a struct holding `pub pool: SqlitePool`; `async fn new()` computes `dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM")`, `fs::create_dir_all(&dir).ok()`, opens `SqliteConnectOptions::new().filename(&db_path).create_if_missing(true)` with `.unwrap_or_else(|_| SqlitePool::connect_lazy("sqlite::memory:").unwrap())`, then `db.init().await.ok()` where `init` runs `CREATE TABLE IF NOT EXISTS` statements. Free functions take `pool: &SqlitePool`. Registered in `lib.rs` via the `tokio::join!` block (line ~147) + `.manage(db)` (line ~197). `busy_timeout(Duration::from_secs(5))` is added when a second process shares the file (`knowledge_base.rs`) — the task board's file is single-writer (only this app), so no `busy_timeout` needed, matching `loop_sessions.rs`.
- **Command wrapper pattern** (`src-tauri/src/commands/loop_session.rs`): thin `#[tauri::command] pub async fn` taking `State<'_, TheDb>` + an args struct, delegating to a `store.rs` free function, `.map_err(|e| e.to_string())`. Registered in `lib.rs`'s `use commands::{...}` block + `tauri::generate_handler![...]`.
- **Config pattern** (`src-tauri/src/config/types.rs`): `McpToolServerConfig` is a `#[derive(Debug, Clone, Serialize, Deserialize)]` struct with `#[serde(default = "...")]` on each field, a hand-written `impl Default`, a `#[serde(default)] pub mcp_tool_server: McpToolServerConfig` field on `AppConfig`, and a matching line in `AppConfig::default()`. `ConfigStore` (`config/mod.rs`) has `get() -> AppConfig` and `update<F>(&self, f: F) -> Result<()>` where `F: FnOnce(&mut AppConfig)`.
- **Tauri event naming in this codebase is plain kebab-case** for broadcast events (`"ai-stream"`, `"mcp-coordination-tab-spawned"`). The `scheme://` form is used only for per-session firehoses (`pty://data/{id}`). This feature's event is **`"tasks-updated"`** (the spec's prose says `tasks://updated`; use the kebab-case form to match the codebase — Plan 2's ipc wrapper hides the literal string from callers anyway).
- **`aiterm_lib`** is the lib crate name (`tests/mcp_tool_server.rs` does `use aiterm_lib::...`).
- Before `cargo test` will even compile, `scripts/setup-uv-{mac,linux,win}.{sh,ps1}` must have been run once (see CLAUDE.md — `build.rs` validates `externalBin` on disk at compile time). Assume it has; if `cargo test` fails with a missing `binaries/uv` resource path, run the setup script for this platform first.

---

### Task 1: `PtySession` — expose last OSC133 exit code and ms-since-last-output

**Files:**
- Modify: `src-tauri/src/pty/session.rs` (fields near line 144-151; reader loop in `spawn_with_id` near line 379-482; accessors near line 710-719)
- Modify: `src-tauri/src/pty/manager.rs` (delegating accessors near line 138-146)

- [ ] **Step 1: Write the failing unit tests**

Add to `src-tauri/src/pty/session.rs`'s `#[cfg(test)] mod tests` (near the existing `bell_count_starts_at_zero_for_a_fresh_session` test):

```rust
#[test]
fn last_exit_code_is_none_for_a_fresh_session() {
    let session = PtySession::spawn(
        ShellSpec::from_program("/bin/sh"),
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
        None,
        |_| {},
    )
    .unwrap();
    assert_eq!(session.last_exit_code(), None);
}

#[tokio::test]
#[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
async fn last_exit_code_captures_a_nonzero_osc133_exit() {
    let session = PtySession::spawn(
        ShellSpec::from_program("/bin/sh"),
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
        None,
        |_| {},
    )
    .unwrap();
    // AITerm's shell-integration hook emits OSC133 D;<code> after each command.
    session.write(b"false\n").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if session.last_exit_code() == Some(1) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "never saw exit code 1, got {:?}", session.last_exit_code());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn ms_since_output_grows_while_the_session_is_quiet() {
    let session = PtySession::spawn(
        ShellSpec::from_program("/bin/sh"),
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
        None,
        |_| {},
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(session.ms_since_output() >= 200, "expected quiet time to accumulate, got {}", session.ms_since_output());
}
```

(Check the exact constructor other tests in this file use — if they call `create_with_callback` off a `PtyManager` instead of `PtySession::spawn` directly, match that. `ShellSpec::from_program` exists per `cd_parser.rs:16` / `pty/shell.rs`; if the shell-spec type/constructor differs, copy it verbatim from a neighbouring test.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib pty::session::tests::last_exit_code 2>&1 | tail -20`
Expected: FAIL — `no method named 'last_exit_code' found`.

- [ ] **Step 3: Add the fields**

In the `PtySession` struct (`session.rs`, alongside `bell_count`/`marker_count` near line 144-151):

```rust
    /// Last exit code observed in an OSC 133 `D;<code>` marker in this
    /// session's output, or `None` if none seen yet. The reader thread
    /// already extracts these codes for cd confirmation
    /// (`confirm_pending_cds_from_output`); this stores the most recent one
    /// so the task-board monitor can tell "the foreground command exited
    /// non-zero" (e.g. `claude` not installed → shell prints 127) from a
    /// still-running task. `i64` with `-1` sentinel for "unset" so it fits
    /// one `AtomicI64` (same lock-free pattern as `bell_count`).
    last_exit_code: Arc<AtomicI64>,
    /// Wall-clock `Instant` of the last non-empty output chunk. Read as
    /// `ms_since_output()`; used by the monitor's "no output for 120s ⇒
    /// stuck" check. Spawn time counts as the first "output" so a session
    /// that never prints anything still ages.
    last_output_at: Arc<Mutex<Instant>>,
```

Add imports if missing: `use std::sync::atomic::AtomicI64;` (there's already `AtomicU64`), and `use std::time::Instant;` (check top of file — `Duration` is likely already imported).

- [ ] **Step 4: Initialise and write them in `spawn_with_id`**

Near the existing `let bell_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));` (line ~379):

```rust
    let last_exit_code: Arc<AtomicI64> = Arc::new(AtomicI64::new(-1));
    let last_exit_code_for_thread = Arc::clone(&last_exit_code);
    let last_output_at: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));
    let last_output_at_for_thread = Arc::clone(&last_output_at);
```

Inside the reader loop, at the same place the chunk is handed to `confirm_pending_cds_from_output` / bell scanning (near line 440-453), add:

```rust
                            *last_output_at_for_thread.lock() = Instant::now();
                            for code in crate::pty::cd_parser::find_exit_codes(&chunk) {
                                last_exit_code_for_thread.store(code as i64, Ordering::SeqCst);
                            }
```

(`find_exit_codes` is likely already called in that block via `confirm_pending_cds_from_output`; calling it a second time here is cheap and keeps this change self-contained — do NOT refactor `confirm_pending_cds_from_output` to return the codes. If `Mutex` in this file is `parking_lot::Mutex` its `.lock()` returns the guard directly as written; if it's `std::sync::Mutex` use `.lock().unwrap()`. Match the file — `session.rs` uses `parking_lot` per line 15-ish of `coordination_ops.rs` sibling; verify.)

In the struct literal that builds the `PtySession` at the end of `spawn_with_id` (near line 481-482, where `bell_count,` `marker_count,` are listed):

```rust
            last_exit_code,
            last_output_at,
```

- [ ] **Step 5: Add the accessors**

Near `pub fn bell_count(&self) -> u64` (line ~711):

```rust
    /// See the `last_exit_code` field. `None` until an OSC 133 `D;<code>`
    /// marker has been seen in output.
    pub fn last_exit_code(&self) -> Option<i32> {
        match self.last_exit_code.load(Ordering::SeqCst) {
            -1 => None,
            n => Some(n as i32),
        }
    }

    /// Milliseconds since the last non-empty output chunk (spawn time counts
    /// as output zero). See the `last_output_at` field.
    pub fn ms_since_output(&self) -> u64 {
        self.last_output_at.lock().elapsed().as_millis() as u64
    }
```

- [ ] **Step 6: Delegate from `PtyManager`**

In `src-tauri/src/pty/manager.rs`, after `pub fn marker_count(&self, id: &str) -> Option<u64>` (line ~144):

```rust
    pub fn last_exit_code(&self, id: &str) -> Option<i32> {
        self.sessions.lock().get(id).and_then(|s| s.last_exit_code())
    }

    pub fn ms_since_output(&self, id: &str) -> Option<u64> {
        self.sessions.lock().get(id).map(|s| s.ms_since_output())
    }
```

(Match the exact body style of `marker_count` directly above — e.g. if it's `.map(|s| s.marker_count())` vs `.and_then`.)

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib pty::session::tests::last_exit_code pty::session::tests::ms_since_output 2>&1 | tail -20`
Expected: PASS (the `_nonzero_osc133_` one is `#[ignore]`d on Windows).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/pty/session.rs src-tauri/src/pty/manager.rs
git commit -m "feat(pty): expose last OSC133 exit code and ms-since-output on PtySession"
```

---

### Task 2: `done_marker_instruction` helper + make `wait_for_new_bell` reusable

**Files:**
- Modify: `src-tauri/src/pty/session.rs` (near `done_marker`, line ~52)
- Modify: `src-tauri/src/mcp_server/coordination_ops.rs:241` (visibility only)

- [ ] **Step 1: Write the failing test**

In `session.rs` tests (near `done_marker_embeds_the_tab_id_between_fixed_delimiters`, line ~1524):

```rust
#[test]
fn done_marker_instruction_never_contains_the_contiguous_marker() {
    let tab = "abc-123-def";
    let instr = done_marker_instruction(tab);
    // The whole point: canonical-mode echo of this text must not itself
    // form the marker byte sequence, or it self-triggers marker_count.
    assert!(!instr.contains(&done_marker(tab)), "instruction must not contain the full contiguous marker: {instr}");
    // But it must mention each piece so a cooperating agent can assemble it.
    assert!(instr.contains(DONE_MARKER_PREFIX));
    assert!(instr.contains(tab));
    assert!(instr.contains(DONE_MARKER_SUFFIX));
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib session::tests::done_marker_instruction 2>&1 | tail -20`
Expected: FAIL — `cannot find function 'done_marker_instruction'`.

- [ ] **Step 3: Add the helper**

In `session.rs`, immediately after `pub fn done_marker(tab_id: &str) -> String { ... }` (line ~52):

```rust
/// The fixed wording appended (as its own CR-terminated write) after a task
/// prompt, asking a cooperating agent to print `done_marker(tab_id)` on its
/// own line when finished — an optional fast-path completion signal on top
/// of the bell fallback. The three pieces (prefix, id, suffix) are named
/// separately with other text between them so this string never itself
/// contains the contiguous 52-byte marker: terminal echo of this
/// instruction would otherwise increment `marker_count` with no agent
/// involved (verified live during the coordination feature's development).
pub fn done_marker_instruction(tab_id: &str) -> String {
    format!(
        "（可選：完成後請在新的一行印出一個完成標記，格式為三段直接相連、中間不留任何字元：前綴 {DONE_MARKER_PREFIX} ，接著是你的識別碼 {tab_id} ，最後接上 {DONE_MARKER_SUFFIX} 。這能讓協調端提早得知你已完成，不影響任何其他行為。）"
    )
}
```

(This is the exact wording already inlined in `coordination_ops::send_input` at line ~202. Leaving that copy as-is is intentional — do not refactor `send_input` in this plan; the duplication is noted for a future cleanup.)

- [ ] **Step 4: Widen `wait_for_new_bell` visibility**

In `src-tauri/src/mcp_server/coordination_ops.rs:241`, change:

```rust
async fn wait_for_new_bell(pty_manager: &PtyManager, tab_id: &str, baseline: u64, timeout: Duration) -> bool {
```

to:

```rust
pub(crate) async fn wait_for_new_bell(pty_manager: &PtyManager, tab_id: &str, baseline: u64, timeout: Duration) -> bool {
```

No behaviour change — `tasks::dispatch` will call it as `crate::mcp_server::coordination_ops::wait_for_new_bell`.

- [ ] **Step 5: Run tests**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib session::tests::done_marker_instruction coordination_ops 2>&1 | tail -20`
Expected: PASS; all existing `coordination_ops` tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/pty/session.rs src-tauri/src/mcp_server/coordination_ops.rs
git commit -m "feat(pty): add done_marker_instruction helper; expose wait_for_new_bell to crate"
```

---

### Task 3: `TaskBoardConfig` in app config + get/set commands

**Files:**
- Modify: `src-tauri/src/config/types.rs` (near `McpToolServerConfig`, line ~159)
- Create: `src-tauri/src/commands/task_board_config.rs`
- Modify: `src-tauri/src/commands/mod.rs` (add `pub mod task_board_config;`)

- [ ] **Step 1: Write the failing test**

In `src-tauri/src/config/types.rs` `#[cfg(test)] mod tests` (add the module if none — check end of file; if there's no test module, put this in a new `#[cfg(test)] mod tests { use super::*; ... }`):

```rust
#[test]
fn task_board_config_has_sane_defaults() {
    let c = TaskBoardConfig::default();
    assert_eq!(c.max_concurrent, 2);
    assert_eq!(c.claude_command, "claude");
}

#[test]
fn app_config_default_includes_task_board() {
    let c = AppConfig::default();
    assert_eq!(c.task_board.max_concurrent, 2);
}

#[test]
fn task_board_config_deserialises_from_empty_table() {
    // Old config.toml with no [task_board] section must still parse.
    let c: AppConfig = toml::from_str("").unwrap();
    assert_eq!(c.task_board.max_concurrent, 2);
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib config::types::tests::task_board 2>&1 | tail -20`
Expected: FAIL — `cannot find type 'TaskBoardConfig'`.

- [ ] **Step 3: Add the config type**

In `config/types.rs`, after the `impl Default for McpToolServerConfig` block (line ~184):

```rust
pub fn default_task_board_max_concurrent() -> u32 { 2 }
pub fn default_claude_command() -> String { "claude".to_string() }

/// Settings for the task board (see
/// `docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBoardConfig {
    /// Global cap on tasks in the `running` column at once. A per-card
    /// `parallel_ok = false` flag further restricts (a solo card waits for
    /// an empty running set and blocks others while it runs).
    #[serde(default = "default_task_board_max_concurrent")]
    pub max_concurrent: u32,
    /// The CLI launched in each dispatched tab. `claude` by default; a user
    /// could point this at another agent, but that's not a supported feature.
    #[serde(default = "default_claude_command")]
    pub claude_command: String,
}

impl Default for TaskBoardConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_task_board_max_concurrent(),
            claude_command: default_claude_command(),
        }
    }
}
```

On `AppConfig` (after the `pub mcp_tool_server: McpToolServerConfig,` field, line ~108):

```rust
    /// Task board settings. Absent from older config.toml — `default` fills in.
    #[serde(default)]
    pub task_board: TaskBoardConfig,
```

In `impl Default for AppConfig` (after `mcp_tool_server: McpToolServerConfig::default(),`, line ~245):

```rust
            task_board: TaskBoardConfig::default(),
```

- [ ] **Step 4: Add the commands**

Create `src-tauri/src/commands/task_board_config.rs`:

```rust
//! Get/set for the task-board settings block. Plain config read/write — no
//! server to start or stop, unlike `commands/mcp_server.rs`. The scheduler
//! re-reads `config.get().task_board` on every wake, so a changed
//! `max_concurrent` takes effect on the next scheduler tick with no restart.

use std::sync::Arc;

use tauri::State;

use crate::config::types::TaskBoardConfig;
use crate::config::ConfigStore;

#[tauri::command]
pub fn task_board_get_config(config: State<'_, Arc<ConfigStore>>) -> TaskBoardConfig {
    config.get().task_board
}

#[tauri::command]
pub fn task_board_set_config(
    value: TaskBoardConfig,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<(), String> {
    let clamped = TaskBoardConfig {
        max_concurrent: value.max_concurrent.clamp(1, 16),
        claude_command: {
            let c = value.claude_command.trim();
            if c.is_empty() { "claude".to_string() } else { c.to_string() }
        },
    };
    config.update(|c| c.task_board = clamped).map_err(|e| e.to_string())
}
```

Add `pub mod task_board_config;` to `src-tauri/src/commands/mod.rs` (keep the list's ordering style).

- [ ] **Step 5: Run tests**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib config::types::tests::task_board config::types::tests::app_config_default_includes_task_board 2>&1 | tail -20`
Expected: PASS. (If `toml` isn't a dev-dependency, the `deserialises_from_empty_table` test won't compile — check `Cargo.toml`; `serde`/`toml` round-trips are used elsewhere in config tests. If `toml` is unavailable, drop that one test.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/config/types.rs src-tauri/src/commands/task_board_config.rs src-tauri/src/commands/mod.rs
git commit -m "feat(config): add TaskBoardConfig (max_concurrent, claude_command)"
```

---

### Task 4: `tasks` module skeleton + `TasksDb` + schema

**Files:**
- Create: `src-tauri/src/tasks/mod.rs`
- Create: `src-tauri/src/tasks/store.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod tasks;` near the other `pub mod` lines, ~line 22)

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/tasks/store.rs` with only this test at first (rest added next steps):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn create_then_list_roundtrips_a_planning_card() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "Fix the flaky test", "make it deterministic", "/repo", true)
            .await
            .unwrap();
        let all = list_tasks(&pool).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].title, "Fix the flaky test");
        assert_eq!(all[0].status, "planning");
        assert_eq!(all[0].parallel_ok, true);
        assert!(all[0].outcome.is_none());
    }
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::store::tests::create_then_list 2>&1 | tail -20`
Expected: FAIL to compile — module `tasks` doesn't exist.

- [ ] **Step 3: Create `tasks/mod.rs`**

```rust
//! Task board: a queued work list that dispatches cards to `claude`.
//! See `docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md`.
//!
//! - `store`     — `tasks.db` schema + CRUD (sqlx free functions over a pool)
//! - `dispatch`  — compose the prompt, spawn a visible PTY tab, type it in
//! - `monitor`   — watch one running task's session to a terminal outcome
//! - `scheduler` — pick the next runnable card; the long-lived dispatch loop

pub mod dispatch;
pub mod monitor;
pub mod scheduler;
pub mod store;

use std::fs;
use std::path::PathBuf;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

/// Holds the connection pool for `tasks.db`. Managed by Tauri; free
/// functions in `store` take `&db.pool`. Same shape as `db::loop_sessions::LoopSessionDb`.
pub struct TasksDb {
    pub pool: SqlitePool,
}

/// `<data-dir>/AITERM` — the same app data directory every other
/// dedicated-SQLite module in this codebase uses.
pub fn app_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM")
}

/// `<data-dir>/AITERM/tasks/<task_id>` — per-task scratch dir holding
/// `attachments/` and `transcript.txt`. Created lazily by dispatch/store.
pub fn task_dir(task_id: &str) -> PathBuf {
    app_data_dir().join("tasks").join(task_id)
}

impl TasksDb {
    pub async fn new() -> Self {
        let dir = app_data_dir();
        fs::create_dir_all(&dir).ok();
        let db_path = dir.join("tasks.db");
        // sqlx defaults create_if_missing to false; without this the open
        // fails and the fallback silently swaps in an in-memory DB that
        // loses every card on restart. See db/loop_sessions.rs.
        let options = SqliteConnectOptions::new().filename(&db_path).create_if_missing(true);
        let pool = SqlitePool::connect_with(options)
            .await
            .unwrap_or_else(|_| SqlitePool::connect_lazy("sqlite::memory:").unwrap());
        let db = Self { pool };
        init_schema(&db.pool).await.ok();
        db
    }
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS tasks (
            id              TEXT PRIMARY KEY NOT NULL,
            title           TEXT NOT NULL,
            body            TEXT NOT NULL DEFAULT '',
            project_dir     TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'planning',
            parallel_ok     INTEGER NOT NULL DEFAULT 1,
            sort_order      REAL NOT NULL DEFAULT 0,
            outcome         TEXT,
            tab_id          TEXT,
            transcript_path TEXT,
            error_message   TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            dispatched_at   INTEGER,
            finished_at     INTEGER
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status, sort_order)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS task_attachments (
            id          TEXT PRIMARY KEY NOT NULL,
            task_id     TEXT NOT NULL,
            filename    TEXT NOT NULL,
            stored_path TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_task_attachments_task ON task_attachments(task_id)")
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 4: Start `tasks/store.rs` — row types + `create_task` + `list_tasks`**

Put this **above** the `#[cfg(test)] mod tests` from Step 1:

```rust
//! `tasks.db` CRUD. Free functions over `&SqlitePool`, same style as
//! `db/loop_sessions.rs`. Status is a plain string column with four values:
//! `planning` → `queued` → `running` → `done`. `outcome` is NULL until
//! `done`, then one of `success` | `failed` | `cancelled`.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

pub const STATUS_PLANNING: &str = "planning";
pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_DONE: &str = "done";

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub status: String,
    #[sqlx(try_from = "i64")]
    pub parallel_ok: bool,
    pub sort_order: f64,
    pub outcome: Option<String>,
    pub tab_id: Option<String>,
    pub transcript_path: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub dispatched_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AttachmentRow {
    pub id: String,
    pub task_id: String,
    pub filename: String,
    pub stored_path: String,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Insert a `planning` card at the bottom of the planning column
/// (`sort_order` = current max + 1). Returns the new id.
pub async fn create_task(
    pool: &SqlitePool,
    title: &str,
    body: &str,
    project_dir: &str,
    parallel_ok: bool,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let next_order: f64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM tasks WHERE status = ?",
    )
    .bind(STATUS_PLANNING)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO tasks (id, title, body, project_dir, status, parallel_ok, sort_order)
         VALUES (?, ?, ?, ?, 'planning', ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(body)
    .bind(project_dir)
    .bind(parallel_ok as i64)
    .bind(next_order)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn list_tasks(pool: &SqlitePool) -> Result<Vec<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks ORDER BY status, sort_order")
        .fetch_all(pool)
        .await
}

pub async fn list_attachments(pool: &SqlitePool, task_id: &str) -> Result<Vec<AttachmentRow>, sqlx::Error> {
    sqlx::query_as::<_, AttachmentRow>("SELECT * FROM task_attachments WHERE task_id = ? ORDER BY filename")
        .bind(task_id)
        .fetch_all(pool)
        .await
}

pub async fn get_task(pool: &SqlitePool, id: &str) -> Result<Option<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}
```

Add `pub mod tasks;` to `src-tauri/src/lib.rs` alongside the other `pub mod` lines.

(`#[sqlx(try_from = "i64")]` on a `bool` field: verify this codebase's sqlx version supports it — grep other `FromRow` structs. If not, drop the attribute and make `parallel_ok: i64`, exposing a `parallel_ok_bool()` helper, and adjust callers/tests accordingly.)

- [ ] **Step 5: Run the test, verify it passes**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::store::tests::create_then_list 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/tasks/ src-tauri/src/lib.rs
git commit -m "feat(tasks): tasks.db schema + create/list store functions"
```

---

### Task 5: `store.rs` — status transitions, move/reorder, attachments, dispatch/finish mutations

**Files:**
- Modify: `src-tauri/src/tasks/store.rs`

- [ ] **Step 1: Write the failing tests**

Add to `store.rs` tests:

```rust
#[tokio::test]
async fn move_planning_to_queued_is_allowed_but_done_to_running_is_not() {
    let pool = mem_pool().await;
    let id = create_task(&pool, "t", "", "/r", true).await.unwrap();
    move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
    assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().status, "queued");

    finish_task(&pool, &id, "success", None, None).await.unwrap();
    let err = move_task(&pool, &id, STATUS_RUNNING, 1.0).await.unwrap_err();
    assert!(err.to_string().contains("illegal transition"), "{err}");
}

#[tokio::test]
async fn midpoint_sort_order_between_two_cards() {
    // Two queued cards at 1.0 and 2.0; moving a third between them → 1.5.
    let pool = mem_pool().await;
    let a = create_task(&pool, "a", "", "/r", true).await.unwrap();
    let b = create_task(&pool, "b", "", "/r", true).await.unwrap();
    move_task(&pool, &a, STATUS_QUEUED, 1.0).await.unwrap();
    move_task(&pool, &b, STATUS_QUEUED, 2.0).await.unwrap();
    let c = create_task(&pool, "c", "", "/r", true).await.unwrap();
    let mid = midpoint_between(&pool, STATUS_QUEUED, Some(&a), Some(&b)).await.unwrap();
    assert!((mid - 1.5).abs() < 1e-9, "got {mid}");
    move_task(&pool, &c, STATUS_QUEUED, mid).await.unwrap();
}

#[tokio::test]
async fn mark_dispatched_and_finish_set_the_right_columns() {
    let pool = mem_pool().await;
    let id = create_task(&pool, "t", "", "/r", true).await.unwrap();
    move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
    mark_dispatched(&pool, &id, "tab-xyz").await.unwrap();
    let row = get_task(&pool, &id).await.unwrap().unwrap();
    assert_eq!(row.status, "running");
    assert_eq!(row.tab_id.as_deref(), Some("tab-xyz"));
    assert!(row.dispatched_at.is_some());

    finish_task(&pool, &id, "failed", Some("boom"), Some("/p/transcript.txt")).await.unwrap();
    let row = get_task(&pool, &id).await.unwrap().unwrap();
    assert_eq!(row.status, "done");
    assert_eq!(row.outcome.as_deref(), Some("failed"));
    assert_eq!(row.error_message.as_deref(), Some("boom"));
    assert!(row.finished_at.is_some());
}

#[tokio::test]
async fn recover_orphaned_running_marks_them_cancelled() {
    let pool = mem_pool().await;
    let id = create_task(&pool, "t", "", "/r", true).await.unwrap();
    move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
    mark_dispatched(&pool, &id, "tab-1").await.unwrap();
    let n = recover_orphaned_running(&pool).await.unwrap();
    assert_eq!(n, 1);
    let row = get_task(&pool, &id).await.unwrap().unwrap();
    assert_eq!(row.status, "done");
    assert_eq!(row.outcome.as_deref(), Some("cancelled"));
}

#[tokio::test]
async fn add_and_remove_attachment_rows() {
    let pool = mem_pool().await;
    let id = create_task(&pool, "t", "", "/r", true).await.unwrap();
    let aid = add_attachment(&pool, &id, "spec.md", "/p/spec.md").await.unwrap();
    assert_eq!(list_attachments(&pool, &id).await.unwrap().len(), 1);
    remove_attachment(&pool, &aid).await.unwrap();
    assert_eq!(list_attachments(&pool, &id).await.unwrap().len(), 0);
}

#[tokio::test]
async fn delete_task_also_deletes_its_attachment_rows() {
    let pool = mem_pool().await;
    let id = create_task(&pool, "t", "", "/r", true).await.unwrap();
    add_attachment(&pool, &id, "a", "/p/a").await.unwrap();
    delete_task(&pool, &id).await.unwrap();
    assert!(get_task(&pool, &id).await.unwrap().is_none());
    assert_eq!(list_attachments(&pool, &id).await.unwrap().len(), 0);
}
```

- [ ] **Step 2: Run them, verify they fail**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::store::tests 2>&1 | tail -25`
Expected: FAIL — `move_task` etc. not found.

- [ ] **Step 3: Implement the mutations**

Append to `store.rs` (above the test module):

```rust
/// Whether `from → to` is a legal user-driven column move. The scheduler and
/// monitor use `mark_dispatched`/`finish_task` for `queued→running` and
/// `→done`; the only moves a user makes by hand are among
/// planning/queued (either direction) and re-queueing is not allowed once
/// running/done — a done card is cloned back to planning instead (frontend).
fn transition_ok(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        (STATUS_PLANNING, STATUS_QUEUED)
            | (STATUS_QUEUED, STATUS_PLANNING)
            | (STATUS_PLANNING, STATUS_PLANNING)
            | (STATUS_QUEUED, STATUS_QUEUED)
    )
}

/// Move a card to `to_status` at `sort_order`. Rejects illegal transitions
/// (see `transition_ok`). `queued→running` / `→done` are NOT done through
/// here — use `mark_dispatched` / `finish_task`.
pub async fn move_task(
    pool: &SqlitePool,
    id: &str,
    to_status: &str,
    sort_order: f64,
) -> Result<(), sqlx::Error> {
    let current: Option<String> = sqlx::query_scalar("SELECT status FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let from = current.ok_or_else(|| sqlx::Error::RowNotFound)?;
    if !transition_ok(&from, to_status) {
        return Err(sqlx::Error::Protocol(format!(
            "illegal transition {from} → {to_status}"
        )));
    }
    sqlx::query("UPDATE tasks SET status = ?, sort_order = ? WHERE id = ?")
        .bind(to_status)
        .bind(sort_order)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The `sort_order` value that drops a card between `before` and `after`
/// (either may be `None` = top/bottom of the column). Frontend computes drop
/// targets; this is here so the reorder rule has one home and is unit-tested.
pub async fn midpoint_between(
    pool: &SqlitePool,
    status: &str,
    before_id: Option<&str>,
    after_id: Option<&str>,
) -> Result<f64, sqlx::Error> {
    async fn order_of(pool: &SqlitePool, id: &str) -> Result<f64, sqlx::Error> {
        sqlx::query_scalar("SELECT sort_order FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
    }
    let lo = match before_id {
        Some(id) => order_of(pool, id).await?,
        None => {
            let min: Option<f64> =
                sqlx::query_scalar("SELECT MIN(sort_order) FROM tasks WHERE status = ?")
                    .bind(status)
                    .fetch_one(pool)
                    .await?;
            min.unwrap_or(1.0) - 1.0
        }
    };
    let hi = match after_id {
        Some(id) => order_of(pool, id).await?,
        None => {
            let max: Option<f64> =
                sqlx::query_scalar("SELECT MAX(sort_order) FROM tasks WHERE status = ?")
                    .bind(status)
                    .fetch_one(pool)
                    .await?;
            max.unwrap_or(0.0) + 2.0
        }
    };
    Ok((lo + hi) / 2.0)
}

pub async fn set_parallel_ok(pool: &SqlitePool, id: &str, parallel_ok: bool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET parallel_ok = ? WHERE id = ?")
        .bind(parallel_ok as i64)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Edit title/body/project_dir. Caller (command layer) restricts this to
/// `planning` cards.
pub async fn update_task_fields(
    pool: &SqlitePool,
    id: &str,
    title: &str,
    body: &str,
    project_dir: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET title = ?, body = ?, project_dir = ? WHERE id = ?")
        .bind(title)
        .bind(body)
        .bind(project_dir)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// `queued → running`: record the spawned tab and dispatch time.
pub async fn mark_dispatched(pool: &SqlitePool, id: &str, tab_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE tasks SET status = 'running', tab_id = ?, dispatched_at = ? WHERE id = ? AND status = 'queued'",
    )
    .bind(tab_id)
    .bind(now_secs())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// `running → done` with an outcome. `outcome` ∈ success | failed | cancelled.
pub async fn finish_task(
    pool: &SqlitePool,
    id: &str,
    outcome: &str,
    error_message: Option<&str>,
    transcript_path: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE tasks SET status = 'done', outcome = ?, error_message = ?, transcript_path = ?, finished_at = ? WHERE id = ?",
    )
    .bind(outcome)
    .bind(error_message)
    .bind(transcript_path)
    .bind(now_secs())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Startup recovery: any card still `running` was orphaned when the app last
/// exited (its PTY died with the process). Mark them done/cancelled. Returns
/// how many were recovered.
pub async fn recover_orphaned_running(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE tasks SET status = 'done', outcome = 'cancelled',
             error_message = 'app 重啟，工作已中斷', finished_at = ?
         WHERE status = 'running'",
    )
    .bind(now_secs())
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn add_attachment(
    pool: &SqlitePool,
    task_id: &str,
    filename: &str,
    stored_path: &str,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO task_attachments (id, task_id, filename, stored_path) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(task_id)
        .bind(filename)
        .bind(stored_path)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn remove_attachment(pool: &SqlitePool, attachment_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM task_attachments WHERE id = ?")
        .bind(attachment_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_attachment(pool: &SqlitePool, attachment_id: &str) -> Result<Option<AttachmentRow>, sqlx::Error> {
    sqlx::query_as::<_, AttachmentRow>("SELECT * FROM task_attachments WHERE id = ?")
        .bind(attachment_id)
        .fetch_optional(pool)
        .await
}

pub async fn delete_task(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM task_attachments WHERE task_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Rows the scheduler needs: everything `queued` (oldest first) and
/// everything `running`.
pub async fn list_by_status(pool: &SqlitePool, status: &str) -> Result<Vec<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks WHERE status = ? ORDER BY sort_order")
        .bind(status)
        .fetch_all(pool)
        .await
}
```

(`sqlx::Error::Protocol` takes a `String` in current sqlx; if this version wants `Box<str>` or a different variant, use whatever `db/*.rs` uses for "logic" errors, or define a small `thiserror` enum for this module. Keep the test's `.contains("illegal transition")` assertion working.)

- [ ] **Step 4: Run tests, verify pass**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::store::tests 2>&1 | tail -25`
Expected: PASS (all ~8 store tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/store.rs
git commit -m "feat(tasks): store transitions, reorder, attachments, dispatch/finish/recover"
```

---

### Task 6: `dispatch.rs` — `build_prompt` (pure)

**Files:**
- Create: `src-tauri/src/tasks/dispatch.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_just_the_body_when_there_are_no_attachments() {
        assert_eq!(build_prompt("Do the thing", &[]), "Do the thing");
    }

    #[test]
    fn prompt_appends_one_line_listing_attachment_paths() {
        let p = build_prompt(
            "Refactor per the spec",
            &["/data/tasks/x/attachments/spec.md".into(), "/data/tasks/x/attachments/before.png".into()],
        );
        assert!(p.starts_with("Refactor per the spec"));
        assert!(p.contains("/data/tasks/x/attachments/spec.md"));
        assert!(p.contains("/data/tasks/x/attachments/before.png"));
        // Attachment note is on its own line, after a blank line.
        assert!(p.contains("\n\n"));
    }

    #[test]
    fn blank_body_still_produces_the_attachment_note() {
        let p = build_prompt("", &["/a/b.txt".into()]);
        assert!(p.contains("/a/b.txt"));
    }
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::dispatch::tests::prompt 2>&1 | tail -20`
Expected: FAIL — module/function missing.

- [ ] **Step 3: Implement `build_prompt` + module header**

```rust
//! Turning a queued card into a live dispatch: compose the prompt text,
//! spawn a visible PTY tab running the configured `claude` command, wait for
//! it to settle, then type the prompt in using the same CR-terminated /
//! done-marker-instruction sequencing `coordination_ops::send_input` uses.

use std::time::Duration;

use portable_pty::PtySize;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::pty::manager::PtyManager;
use crate::pty::session::done_marker_instruction;

/// Max wait for `claude` to finish its cold start (spec measured ~3.7s) before
/// we type the prompt. If it's still noisy at this point we send anyway.
const SETTLE_TIMEOUT_MS: u64 = 30_000;
/// The session is "settled" once it has produced no output for this long.
const SETTLE_QUIET_MS: u64 = 800;
const POLL_MS: u64 = 250;
/// Same as `coordination_ops::DONE_MARKER_WAIT_SECONDS` — how long to wait for
/// `claude` to ring a fresh bell (signalling it finished reading the prompt)
/// before sending the optional done-marker instruction.
const DONE_MARKER_WAIT_SECONDS: u64 = 15;

/// Body text plus, if any attachments, one trailing line pointing `claude` at
/// their on-disk paths (they've already been copied into the task dir).
pub fn build_prompt(body: &str, attachment_paths: &[String]) -> String {
    if attachment_paths.is_empty() {
        return body.to_string();
    }
    let list = attachment_paths.join("、");
    format!("{body}\n\n（相關附件：{list}）")
}
```

- [ ] **Step 4: Run test, verify pass**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::dispatch::tests::prompt 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/dispatch.rs
git commit -m "feat(tasks): dispatch::build_prompt"
```

---

### Task 7: `dispatch.rs` — `run_on_session` + `spawn_and_run`

**Files:**
- Modify: `src-tauri/src/tasks/dispatch.rs`

- [ ] **Step 1: Write the failing test**

Add to `dispatch.rs` tests:

```rust
use crate::pty::manager::PtyManager;

#[tokio::test]
#[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
async fn run_on_session_types_the_prompt_into_an_existing_session() {
    let pty = PtyManager::new();
    // A plain shell stands in for `claude` — it echoes typed lines back.
    let tab_id = pty
        .create_with_callback(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 }, |_| {})
        .unwrap();

    let res = run_on_session(&pty, &tab_id, "echo TASKBOARD_PROMPT_MARKER", false)
        .await
        .unwrap();
    // Baselines are captured, so the monitor can compare against them.
    assert!(res.marker_baseline == 0 || res.marker_baseline > 0); // just: field exists / no panic

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let out = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
        if out.contains("TASKBOARD_PROMPT_MARKER") {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "prompt never echoed: {out}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::dispatch::tests::run_on_session 2>&1 | tail -20`
Expected: FAIL — `run_on_session` not found.

- [ ] **Step 3: Implement**

Append to `dispatch.rs` (above tests):

```rust
/// Bell/marker counts captured right after the prompt (and optional
/// instruction) were written — the baseline the monitor compares fresh
/// counts against to detect "claude replied to *this* prompt".
#[derive(Debug, Clone, Copy)]
pub struct DispatchResult {
    pub bell_baseline: u64,
    pub marker_baseline: u64,
}

/// Payload for `tasks-updated`-adjacent `mcp-coordination-tab-spawned` — the
/// event the frontend already listens for to adopt a backend-spawned session
/// as a visible tab. Same field names as `coordination_ops`'s copy.
#[derive(Serialize, Clone)]
struct TabSpawnedEvent {
    session_id: String,
    command: Option<String>,
}

/// Wait until `tab_id` has produced no output for `SETTLE_QUIET_MS`, or
/// `SETTLE_TIMEOUT_MS` elapses. Used to let `claude` finish booting before we
/// type into it.
async fn wait_until_settled(pty: &PtyManager, tab_id: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(SETTLE_TIMEOUT_MS);
    loop {
        let quiet = pty.ms_since_output(tab_id).unwrap_or(u64::MAX);
        if quiet >= SETTLE_QUIET_MS || tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}

/// Type `prompt` into an already-running session (a `claude` REPL, normally).
/// Mirrors `coordination_ops::send_input`: CR-terminated write; then, if
/// `request_done_marker`, wait for a fresh bell and send
/// `done_marker_instruction` as a second independent CR-terminated write.
/// Returns the post-write bell/marker baselines.
pub async fn run_on_session(
    pty: &PtyManager,
    tab_id: &str,
    prompt: &str,
    request_done_marker: bool,
) -> Result<DispatchResult, String> {
    pty.write(tab_id, format!("{prompt}\r").as_bytes()).map_err(|e| e.to_string())?;

    if request_done_marker {
        let bell_before = pty.bell_count(tab_id).unwrap_or(0);
        let became_idle = crate::mcp_server::coordination_ops::wait_for_new_bell(
            pty,
            tab_id,
            bell_before,
            Duration::from_secs(DONE_MARKER_WAIT_SECONDS),
        )
        .await;
        if became_idle {
            let instr = done_marker_instruction(tab_id);
            pty.write(tab_id, format!("{instr}\r").as_bytes()).map_err(|e| e.to_string())?;
        }
    }

    Ok(DispatchResult {
        bell_baseline: pty.bell_count(tab_id).unwrap_or(0),
        marker_baseline: pty.marker_count(tab_id).unwrap_or(0),
    })
}

/// Full dispatch: create a visible tab in `project_dir` running
/// `claude_command`, emit the adopt event, wait for it to settle, type the
/// prompt in. Returns `(tab_id, DispatchResult)`.
pub async fn spawn_and_run(
    app: &AppHandle,
    pty: &PtyManager,
    project_dir: &str,
    claude_command: &str,
    prompt: &str,
) -> Result<(String, DispatchResult), String> {
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let tab_id = pty
        .create_with_app(app.clone(), size, Some(project_dir.into()), None)
        .map_err(|e| e.to_string())?;

    if let Err(e) = pty.write(&tab_id, format!("{claude_command}\r").as_bytes()) {
        let _ = pty.close(&tab_id);
        return Err(e.to_string());
    }
    if let Err(e) = app.emit(
        "mcp-coordination-tab-spawned",
        TabSpawnedEvent { session_id: tab_id.clone(), command: Some(claude_command.to_string()) },
    ) {
        eprintln!("emit mcp-coordination-tab-spawned failed: {e}");
    }

    wait_until_settled(pty, &tab_id).await;
    let result = run_on_session(pty, &tab_id, prompt, true).await?;
    Ok((tab_id, result))
}
```

- [ ] **Step 4: Run test, verify pass**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::dispatch::tests 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/dispatch.rs
git commit -m "feat(tasks): dispatch run_on_session + spawn_and_run"
```

---

### Task 8: `monitor.rs` — watch a running task to an outcome

**Files:**
- Create: `src-tauri/src/tasks/monitor.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::manager::PtyManager;
    use crate::pty::session::done_marker;
    use portable_pty::PtySize;
    use std::time::Duration;

    fn size() -> PtySize {
        PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 }
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn marker_in_output_yields_success() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel();
        let marker = done_marker(&tab);
        pty.write(&tab, format!("printf '%s\\n' '{marker}'\n").as_bytes()).unwrap();

        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds()).await;
        assert!(matches!(outcome, TaskOutcome::Success), "{outcome:?}");
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn nonzero_exit_yields_failed() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel();
        pty.write(&tab, b"sh -c 'exit 3'\n").unwrap();

        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds()).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains('3'), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn silence_past_the_threshold_yields_failed_stuck() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel();
        // Never write anything: session is quiet from the start.
        let thresholds = Thresholds { quiet_stuck_ms: 300, poll_ms: 50, min_run_ms: 200 };
        let outcome = watch(&pty, &tab, rx, Baselines::default(), thresholds).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains("卡住"), "{msg}"),
            other => panic!("expected Failed(stuck), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_signal_yields_cancelled() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tx.send(()).unwrap();
        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds()).await;
        assert!(matches!(outcome, TaskOutcome::Cancelled), "{outcome:?}");
    }

    fn test_thresholds() -> Thresholds {
        Thresholds { quiet_stuck_ms: 60_000, poll_ms: 50, min_run_ms: 0 }
    }
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::monitor::tests 2>&1 | tail -25`
Expected: FAIL — module missing.

- [ ] **Step 3: Implement**

```rust
//! Watches one dispatched task's PTY session until it reaches a terminal
//! outcome. Signals, in priority order:
//!   1. cancel channel fired               → Cancelled
//!   2. fresh bell or done-marker observed → Success
//!   3. OSC133 non-zero exit code          → Failed("claude 以 exit code N 結束")
//!   4. OSC133 exit code 0                 → Success (claude exited cleanly)
//!   5. no output for `quiet_stuck_ms`     → Failed("疑似卡住（120 秒無輸出）")
//! Reuses `PtyManager`'s existing per-session counters — no new protocol.

use std::time::Duration;

use tokio::sync::oneshot;

use crate::pty::manager::PtyManager;

#[derive(Debug, Clone)]
pub enum TaskOutcome {
    Success,
    Failed(String),
    Cancelled,
}

impl TaskOutcome {
    /// The `outcome` string stored in `tasks.db`.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskOutcome::Success => "success",
            TaskOutcome::Failed(_) => "failed",
            TaskOutcome::Cancelled => "cancelled",
        }
    }
    pub fn error_message(&self) -> Option<&str> {
        match self {
            TaskOutcome::Failed(m) => Some(m.as_str()),
            _ => None,
        }
    }
}

/// bell/marker counts as of the moment the prompt was sent (from
/// `dispatch::DispatchResult`). `Default` = all zero, for tests that don't
/// prime the session first.
#[derive(Debug, Clone, Copy, Default)]
pub struct Baselines {
    pub bell: u64,
    pub marker: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// No output for this long ⇒ stuck. Production: 120_000.
    pub quiet_stuck_ms: u64,
    /// Poll interval. Production: 250.
    pub poll_ms: u64,
    /// Don't declare "stuck" until at least this long after watch start, so a
    /// slow cold start isn't mistaken for a hang. Production: `quiet_stuck_ms`.
    pub min_run_ms: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self { quiet_stuck_ms: 120_000, poll_ms: 250, min_run_ms: 120_000 }
    }
}

pub async fn watch(
    pty: &PtyManager,
    tab_id: &str,
    mut cancel: oneshot::Receiver<()>,
    baselines: Baselines,
    thresholds: Thresholds,
) -> TaskOutcome {
    let start = tokio::time::Instant::now();
    loop {
        // 1. cancel
        match cancel.try_recv() {
            Ok(()) => return TaskOutcome::Cancelled,
            Err(oneshot::error::TryRecvError::Closed) => return TaskOutcome::Cancelled,
            Err(oneshot::error::TryRecvError::Empty) => {}
        }

        // Session gone (tab closed out from under us) — treat as cancelled.
        let Some(bell) = pty.bell_count(tab_id) else {
            return TaskOutcome::Cancelled;
        };
        let marker = pty.marker_count(tab_id).unwrap_or(0);

        // 2. reply signal
        if marker > baselines.marker || bell > baselines.bell {
            return TaskOutcome::Success;
        }

        // 3/4. process exited
        if let Some(code) = pty.last_exit_code(tab_id) {
            if code != 0 {
                return TaskOutcome::Failed(format!("claude 以 exit code {code} 結束"));
            }
            return TaskOutcome::Success;
        }

        // 5. stuck
        let ran_ms = start.elapsed().as_millis() as u64;
        let quiet_ms = pty.ms_since_output(tab_id).unwrap_or(0);
        if ran_ms >= thresholds.min_run_ms && quiet_ms >= thresholds.quiet_stuck_ms {
            return TaskOutcome::Failed("疑似卡住（120 秒無輸出）".to_string());
        }

        tokio::time::sleep(Duration::from_millis(thresholds.poll_ms)).await;
    }
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::monitor::tests 2>&1 | tail -25`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/monitor.rs
git commit -m "feat(tasks): monitor::watch outcome state machine"
```

---

### Task 9: `scheduler.rs` — `pick_next` (pure selection)

**Files:**
- Create: `src-tauri/src/tasks/scheduler.rs`

- [ ] **Step 1: Write the failing tests**

```rust
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
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::scheduler::tests::pick 2>&1 | tail -25`
Expected: FAIL — module missing.

- [ ] **Step 3: Implement `pick_next` + module header**

```rust
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
use crate::tasks::store::{self, TaskRow};
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
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::scheduler::tests::pick 2>&1 | tail -25`
Expected: PASS (all 7).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/scheduler.rs
git commit -m "feat(tasks): scheduler::pick_next selection rule"
```

---

### Task 10: `scheduler.rs` — the dispatch loop + handle

**Files:**
- Modify: `src-tauri/src/tasks/scheduler.rs`

- [ ] **Step 1: Write the failing test** (loop-level, no `AppHandle`)

The loop as written needs an `AppHandle` for `dispatch::spawn_and_run`. To keep it testable, factor the "run one card" step behind a small trait so the test can substitute a fake. Add:

```rust
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
    /// the card running with a fake tab id and finishes it `success` after a
    /// beat, so we can assert the loop's promotion order.
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
    async fn loop_promotes_queued_cards_up_to_the_cap_then_stops_at_a_solo_card() {
        let db = mem_db().await;
        // Queue: a(par), b(par), solo(!par), c(par)
        for (t, par, ord) in [("a", true, 1.0), ("b", true, 2.0), ("solo", false, 3.0), ("c", true, 4.0)] {
            let id = store::create_task(&db.pool, t, "", "/r", par).await.unwrap();
            store::move_task(&db.pool, &id, store::STATUS_QUEUED, ord).await.unwrap();
        }
        // One tick with cap 2 and the fake dispatcher: a and b finish; solo is
        // next but at this point nothing's running so it also runs; c waits
        // only if solo is still running — with the fake it finishes instantly,
        // so a full drain leaves everything done in queue order.
        drain_once(&db, &FakeDispatcher, 2).await;
        let all = store::list_tasks(&db.pool).await.unwrap();
        assert!(all.iter().all(|r| r.status == "done"), "{all:#?}");
    }
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::scheduler::loop_tests 2>&1 | tail -25`
Expected: FAIL — `Dispatcher`, `drain_once` not found. (Also needs `async-trait` — check `Cargo.toml`; it's commonly already a dep. If not, add `async-trait` to `[dependencies]` in this task.)

- [ ] **Step 3: Implement the loop**

Append to `scheduler.rs`:

```rust
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
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<()>>>>,
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

        let (tab_id, disp) =
            dispatch::spawn_and_run(&self.app, &self.pty, &task.project_dir, &claude_cmd, &prompt).await?;
        store::mark_dispatched(&db.pool, &task.id, &tab_id)
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.app.emit("tasks-updated", ());

        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.cancels.lock().insert(task.id.clone(), cancel_tx);

        let pool = db.pool.clone();
        let pty = self.pty.clone();
        let app = self.app.clone();
        let wake = self.wake.clone();
        let cancels = self.cancels.clone();
        let task_id = task.id.clone();
        let baselines = monitor::Baselines { bell: disp.bell_baseline, marker: disp.marker_baseline };
        tauri::async_runtime::spawn(async move {
            let outcome =
                monitor::watch(&pty, &tab_id, cancel_rx, baselines, monitor::Thresholds::default()).await;
            let transcript = write_transcript(&pty, &task_id, &tab_id);
            let _ = store::finish_task(
                &pool,
                &task_id,
                outcome.as_str(),
                outcome.error_message(),
                transcript.as_deref(),
            )
            .await;
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
            // Dispatch failed before the card ever ran — finish it failed so
            // it doesn't wedge the queue.
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
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl SchedulerHandle {
    pub fn poke(&self) {
        self.wake.notify_one();
    }
    /// Fire the cancel channel for a running task, if present. Returns true
    /// if a watch was signalled.
    pub fn cancel(&self, task_id: &str) -> bool {
        if let Some(tx) = self.cancels.lock().remove(task_id) {
            let _ = tx.send(());
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
        let db = app.state::<TasksDb>();
        let config = app.state::<Arc<ConfigStore>>().inner().clone();
        let pty = app.state::<Arc<PtyManager>>().inner().clone();

        // Startup recovery: adopt-nothing, just clear orphaned running cards.
        match store::recover_orphaned_running(&db.pool).await {
            Ok(n) if n > 0 => {
                let _ = app.emit("tasks-updated", ());
                eprintln!("task board: recovered {n} orphaned running card(s)");
            }
            Ok(_) => {}
            Err(e) => eprintln!("task board recovery scan: {e}"),
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
            drain_once(&db, &dispatcher, max).await;
            // Wake on: a card queued/stopped (poke), a watch finishing
            // (notify_one), or every 30s as a backstop.
            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
        }
    });

    handle
}
```

Notes for the implementer:
- `app.state::<TasksDb>()` returns a `State<'_, TasksDb>` borrowing the `AppHandle` — since this future is `'static` and owns `app`, call `.state()` inside the async block each iteration (cheap) rather than hoisting a borrow. If the borrow checker fights you, wrap `TasksDb` in `Arc` at the `.manage()` site and clone the `Arc` once before the loop.
- `parking_lot` is already a dependency (used across `pty/`, `mcp_server/`).

- [ ] **Step 4: Run tests, verify pass**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib tasks::scheduler 2>&1 | tail -25`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/scheduler.rs src-tauri/Cargo.toml
git commit -m "feat(tasks): scheduler loop, RealDispatcher, transcript capture, SchedulerHandle"
```

---

### Task 11: `commands/tasks.rs` — Tauri command surface

**Files:**
- Create: `src-tauri/src/commands/tasks.rs`
- Modify: `src-tauri/src/commands/mod.rs` (`pub mod tasks;`)

- [ ] **Step 1: Write the failing test**

Command fns need Tauri `State` and are covered end-to-end by the Task 13 integration test; here add a small unit test for the one bit of logic that isn't a straight delegate — the "attachments only editable while planning" guard — by testing a helper:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_planning_cards_accept_edits() {
        assert!(edit_allowed("planning"));
        assert!(!edit_allowed("queued"));
        assert!(!edit_allowed("running"));
        assert!(!edit_allowed("done"));
    }
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib commands::tasks::tests 2>&1 | tail -20`
Expected: FAIL — module missing.

- [ ] **Step 3: Implement**

```rust
//! Tauri commands for the task board. Thin delegates to `tasks::store`, plus
//! attachment file I/O and a `tasks-updated` emit after every mutation so the
//! board view (a passive renderer) refreshes. Same shape as
//! `commands/loop_session.rs`.

use std::fs;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::tasks::store::{self, AttachmentRow, TaskRow};
use crate::tasks::{task_dir, TasksDb};
use crate::tasks::scheduler::SchedulerHandle;

pub(crate) fn edit_allowed(status: &str) -> bool {
    status == store::STATUS_PLANNING
}

fn emit_updated(app: &AppHandle) {
    let _ = app.emit("tasks-updated", ());
}

#[derive(Serialize)]
pub struct TaskWithAttachments {
    #[serde(flatten)]
    pub task: TaskRow,
    pub attachments: Vec<AttachmentRow>,
}

#[tauri::command]
pub async fn tasks_list(db: State<'_, TasksDb>) -> Result<Vec<TaskWithAttachments>, String> {
    let tasks = store::list_tasks(&db.pool).await.map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        let attachments = store::list_attachments(&db.pool, &task.id).await.map_err(|e| e.to_string())?;
        out.push(TaskWithAttachments { task, attachments });
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct CreateArgs {
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
}

#[tauri::command]
pub async fn tasks_create(args: CreateArgs, db: State<'_, TasksDb>, app: AppHandle) -> Result<String, String> {
    let id = store::create_task(&db.pool, &args.title, &args.body, &args.project_dir, args.parallel_ok)
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(id)
}

#[derive(Deserialize)]
pub struct UpdateArgs {
    pub id: String,
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
}

#[tauri::command]
pub async fn tasks_update(args: UpdateArgs, db: State<'_, TasksDb>, app: AppHandle) -> Result<(), String> {
    let row = store::get_task(&db.pool, &args.id).await.map_err(|e| e.to_string())?
        .ok_or("task not found")?;
    // parallel_ok can be changed any time (takes effect next run); the text
    // fields only while planning.
    store::set_parallel_ok(&db.pool, &args.id, args.parallel_ok).await.map_err(|e| e.to_string())?;
    if edit_allowed(&row.status) {
        store::update_task_fields(&db.pool, &args.id, &args.title, &args.body, &args.project_dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    emit_updated(&app);
    Ok(())
}

#[derive(Deserialize)]
pub struct MoveArgs {
    pub id: String,
    pub to_status: String,
    pub sort_order: f64,
}

#[tauri::command]
pub async fn tasks_move(
    args: MoveArgs,
    db: State<'_, TasksDb>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    store::move_task(&db.pool, &args.id, &args.to_status, args.sort_order)
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    if args.to_status == store::STATUS_QUEUED {
        scheduler.poke();
    }
    Ok(())
}

#[tauri::command]
pub async fn tasks_stop(
    id: String,
    db: State<'_, TasksDb>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
    pty: State<'_, Arc<crate::pty::manager::PtyManager>>,
) -> Result<(), String> {
    let row = store::get_task(&db.pool, &id).await.map_err(|e| e.to_string())?.ok_or("task not found")?;
    if row.status != store::STATUS_RUNNING {
        return Err("task is not running".into());
    }
    if let Some(tab_id) = &row.tab_id {
        let _ = pty.write(tab_id, b"\x03"); // Ctrl+C
    }
    // The running watch will observe the cancel and finish the card
    // `cancelled`; if there is no watch (edge case), finish it here.
    if !scheduler.cancel(&id) {
        store::finish_task(&db.pool, &id, "cancelled", Some("使用者停止"), None)
            .await
            .map_err(|e| e.to_string())?;
        emit_updated(&app);
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct DeleteArgs {
    pub id: String,
    pub close_tab: bool,
}

#[tauri::command]
pub async fn tasks_delete(
    args: DeleteArgs,
    db: State<'_, TasksDb>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
    pty: State<'_, Arc<crate::pty::manager::PtyManager>>,
) -> Result<(), String> {
    if let Some(row) = store::get_task(&db.pool, &args.id).await.map_err(|e| e.to_string())? {
        scheduler.cancel(&args.id);
        if args.close_tab {
            if let Some(tab_id) = &row.tab_id {
                let _ = pty.close(tab_id);
            }
        }
    }
    store::delete_task(&db.pool, &args.id).await.map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(task_dir(&args.id)); // best effort
    emit_updated(&app);
    Ok(())
}

#[derive(Deserialize)]
pub struct AddAttachmentArgs {
    pub id: String,
    pub filename: String,
    pub bytes: Vec<u8>,
}

#[tauri::command]
pub async fn tasks_add_attachment(
    args: AddAttachmentArgs,
    db: State<'_, TasksDb>,
    app: AppHandle,
) -> Result<AttachmentRow, String> {
    let row = store::get_task(&db.pool, &args.id).await.map_err(|e| e.to_string())?.ok_or("task not found")?;
    if !edit_allowed(&row.status) {
        return Err("attachments can only be changed while the card is in 計畫中".into());
    }
    let dir = task_dir(&args.id).join("attachments");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Basename only — never honour a path in `filename`.
    let safe = std::path::Path::new(&args.filename)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_string());
    let stored = dir.join(&safe);
    fs::write(&stored, &args.bytes).map_err(|e| e.to_string())?;
    let att_id = store::add_attachment(&db.pool, &args.id, &safe, &stored.to_string_lossy())
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(AttachmentRow { id: att_id, task_id: args.id, filename: safe, stored_path: stored.to_string_lossy().into_owned() })
}

#[tauri::command]
pub async fn tasks_remove_attachment(
    attachment_id: String,
    db: State<'_, TasksDb>,
    app: AppHandle,
) -> Result<(), String> {
    if let Some(att) = store::get_attachment(&db.pool, &attachment_id).await.map_err(|e| e.to_string())? {
        let _ = fs::remove_file(&att.stored_path);
    }
    store::remove_attachment(&db.pool, &attachment_id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

#[tauri::command]
pub async fn tasks_read_transcript(id: String, db: State<'_, TasksDb>) -> Result<String, String> {
    let row = store::get_task(&db.pool, &id).await.map_err(|e| e.to_string())?.ok_or("task not found")?;
    match row.transcript_path {
        Some(p) => fs::read_to_string(&p).map_err(|e| e.to_string()),
        None => Ok(String::new()),
    }
}
```

Add `pub mod tasks;` to `commands/mod.rs`.

- [ ] **Step 4: Run test, verify pass**

Run: `cd src-tauri && cargo test -p aiterm-lib --lib commands::tasks::tests 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/tasks.rs src-tauri/src/commands/mod.rs
git commit -m "feat(tasks): Tauri command surface (list/create/update/move/stop/delete/attachments/transcript)"
```

---

### Task 12: Wire into `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Build `TasksDb` alongside the other DBs**

In the `tokio::join!` block (line ~147-165), add `crate::tasks::TasksDb::new()` to the tuple and its binding:

```rust
    let (usage_store, design_db, loop_session_db, kb_db, mail_db, tasks_db) =
        tauri::async_runtime::block_on(async {
            tokio::join!(
                async { /* usage_store — unchanged */ },
                DesignDb::new(),
                LoopSessionDb::new(),
                db::knowledge_base::KnowledgeBaseDb::new(),
                MailDb::new(),
                crate::tasks::TasksDb::new(),
            )
        });
```

- [ ] **Step 2: `.manage()` it and a placeholder for the scheduler handle**

After `.manage(mail_db)` (line ~198):

```rust
        .manage(tasks_db)
```

- [ ] **Step 3: Start the scheduler in `.setup()`**

Inside the `.setup(|app| { ... })` closure, after the existing MCP-tool-server spawn block and before `Ok(())`:

```rust
            // Task board scheduler: long-lived, dispatches queued cards to
            // `claude`. Also runs a one-time recovery scan on start. Managed
            // handle lets `tasks_move`/`tasks_stop` poke it.
            {
                let handle = app.handle().clone();
                let sched = tasks::scheduler::spawn(handle);
                app.manage(sched);
            }
```

(`app.manage(...)` inside `.setup` is valid — `app` is the `&mut App`/`AppHandle`-bearing arg; check whether it's `app.manage` or `app.handle().manage` in this codebase — `enterprise` / `chatgpt_web` init calls nearby show the idiom.)

- [ ] **Step 4: Register the commands**

In the `use commands::{ ... }` block near line 30, add:

```rust
    task_board_config::{task_board_get_config, task_board_set_config},
    tasks::{
        tasks_list, tasks_create, tasks_update, tasks_move, tasks_stop, tasks_delete,
        tasks_add_attachment, tasks_remove_attachment, tasks_read_transcript,
    },
```

In `tauri::generate_handler![ ... ]` add all 11 names:

```rust
            task_board_get_config,
            task_board_set_config,
            tasks_list,
            tasks_create,
            tasks_update,
            tasks_move,
            tasks_stop,
            tasks_delete,
            tasks_add_attachment,
            tasks_remove_attachment,
            tasks_read_transcript,
```

- [ ] **Step 5: Build**

Run: `cd src-tauri && cargo build -p aiterm-lib 2>&1 | tail -30`
Expected: compiles clean. Fix any borrow issue in `scheduler::spawn` per that task's notes (wrap `TasksDb` in `Arc` at the manage site if needed — then `app.state::<Arc<TasksDb>>()`).

- [ ] **Step 6: Full test run**

Run: `cd src-tauri && cargo test 2>&1 | tail -30`
Expected: all pass (unit + existing integration). This runs `tests/*.rs` too — see CLAUDE.md, `--lib` alone would skip them.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(tasks): wire TasksDb, scheduler, and commands into the app"
```

---

### Task 13: End-to-end integration test

**Files:**
- Create: `src-tauri/tests/task_board.rs`

- [ ] **Step 1: Write the test**

Drives `store` + `scheduler::drain_once` + `monitor` against a real PTY running a fake "agent" script (no `claude`, no `AppHandle`). Uses a test `Dispatcher` that calls the real `dispatch::run_on_session` after spawning a plain shell via `create_with_callback`.

```rust
//! End-to-end: a queued card is dispatched to a real PTY running a fake
//! agent script, and the scheduler+monitor drive it to done/success (and
//! done/failed for a non-zero exit). No `claude`, no `AppHandle`.

use std::sync::Arc;
use std::time::Duration;

use aiterm_lib::pty::manager::PtyManager;
use aiterm_lib::pty::session::done_marker;
use aiterm_lib::tasks::{init_schema, store, TasksDb};
use aiterm_lib::tasks::scheduler::{drain_once, Dispatcher};
use aiterm_lib::tasks::store::TaskRow;
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
            .create_with_callback(PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 }, |_| {})
            .map_err(|e| e.to_string())?;
        store::mark_dispatched(&db.pool, &task.id, &tab_id).await.map_err(|e| e.to_string())?;

        let script = self.script.replace("{marker}", &done_marker(&tab_id));
        self.pty.write(&tab_id, format!("{script}\n").as_bytes()).map_err(|e| e.to_string())?;

        let baselines = aiterm_lib::tasks::monitor::Baselines::default();
        let thresholds = aiterm_lib::tasks::monitor::Thresholds { quiet_stuck_ms: 10_000, poll_ms: 50, min_run_ms: 0 };
        let pty = self.pty.clone();
        let pool = db.pool.clone();
        let task_id = task.id.clone();
        tokio::spawn(async move {
            let (_tx, rx) = tokio::sync::oneshot::channel();
            let outcome = aiterm_lib::tasks::monitor::watch(&pty, &tab_id, rx, baselines, thresholds).await;
            let _ = store::finish_task(&pool, &task_id, outcome.as_str(), outcome.error_message(), None).await;
        });
        Ok(())
    }
}

async fn mem_db() -> TasksDb {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
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
```

For `use aiterm_lib::tasks::...` to work, `tasks` must be `pub mod tasks;` in `lib.rs` (done in Task 4) and `store`/`monitor`/`scheduler` `pub mod` in `tasks/mod.rs` (done). `Dispatcher`, `drain_once`, `monitor::Baselines`, `monitor::Thresholds`, `monitor::watch` must be `pub` (they are, per Tasks 8–10). `async-trait` must be a dep (added in Task 10 if not already).

- [ ] **Step 2: Run**

Run: `cd src-tauri && cargo test --test task_board 2>&1 | tail -30`
Expected: both tests PASS on macOS/Linux (ignored on Windows).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/task_board.rs src-tauri/Cargo.toml
git commit -m "test(tasks): end-to-end dispatch → monitor → done integration test"
```

---

### Task 14: Verification pass

- [ ] **Step 1:** `cd src-tauri && cargo test 2>&1 | tail -40` — all green (see CLAUDE.md: full `cargo test`, not `--lib`).
- [ ] **Step 2:** `cd src-tauri && cargo clippy --all-targets 2>&1 | tail -40` — no new warnings in `src/tasks/` or `src/commands/tasks.rs`.
- [ ] **Step 3:** `npx tsc -b` from repo root — unaffected (no frontend changes yet), should still pass.
- [ ] **Step 4:** Manual smoke (optional, needs `claude` installed): `npm run tauri:dev`, then from a devtools console `window.__TAURI__.core.invoke('tasks_create', { args: { title:'say hi', body:'reply with the single word: pong', project_dir: '<a real dir>', parallel_ok: true } })` then `invoke('tasks_move', { args: { id:'<id>', toStatus:'queued', sortOrder: 1 } })`. Expect a new tab to open running `claude`, the prompt typed in, and after it replies `invoke('tasks_list')` shows the card `status: "done", outcome: "success"` with a `transcript_path`.
- [ ] **Step 5: Commit** (if any fixes): `git commit -am "fix(tasks): verification-pass cleanup"`

---

## Self-Review

**Spec coverage:**
- Sidebar button / board view / 4 columns → **Plan 2** (frontend). This plan is backend only.
- `tasks.db` dedicated SQLite, schema, `sort_order` REAL midpoint → Tasks 4, 5. ✅
- Per-card `parallel_ok` + global `max_concurrent` (default 2) → Tasks 3, 9. ✅
- Scheduler in Rust, long-lived, DB is source of truth, strict-priority solo rule → Tasks 9, 10. ✅
- Startup recovery of orphaned `running` → Tasks 5 (`recover_orphaned_running`), 10 (called on start). ✅
- Dispatch: compose prompt (body + attachment paths line), spawn visible tab via `create_with_app` + `mcp-coordination-tab-spawned`, wait-for-settle, type prompt with CR + done-marker instruction → Tasks 6, 7. ✅
- Attachments copied into task dir, path listed in prompt → Tasks 6 (`build_prompt`), 11 (`tasks_add_attachment` file write), 10 (`RealDispatcher` passes `stored_path`s). ✅
- Completion: bell/marker → success; non-zero OSC133 exit → failed; 120s quiet → failed-stuck; stop → cancelled → Task 8. ✅
- Transcript snapshot on finish → Task 10 (`write_transcript`), 11 (`tasks_read_transcript`). ✅
- `tasks-updated` event after every mutation → Task 11. ✅
- Spawned tab stays open; delete offers `close_tab` → Task 11 (`tasks_delete` `close_tab` flag). ✅
- Failure of dispatch itself doesn't wedge the queue → Task 10 (`drain_once` error branch). ✅
- `claude` command configurable (default `claude`) → Task 3. ✅
- Out of scope confirmed unaddressed: board-driven multi-turn chat, non-`claude` agent as a feature, surviving app restart mid-run (explicitly handled by marking cancelled).

**Placeholder scan:** No TBD/TODO. Every code step has complete code. Fake-`AppHandle` testability is handled by the `Dispatcher` trait, not deferred.

**Type consistency:** `TaskRow` fields identical across Tasks 4, 9, 10, 13. `TaskOutcome::{as_str,error_message}` used consistently in Tasks 8, 10, 13. `store` fn names (`move_task`, `mark_dispatched`, `finish_task`, `recover_orphaned_running`, `list_by_status`, `midpoint_between`) consistent across Tasks 5, 10, 11, 13. Event string `"tasks-updated"` consistent (Tasks 10, 11). `Thresholds`/`Baselines` fields consistent (Tasks 8, 10, 13). Config field `task_board.{max_concurrent,claude_command}` consistent (Tasks 3, 10).

**Known soft spots flagged in the plan (not gaps):** `#[sqlx(try_from)]` availability (Task 4 Step 4), `sqlx::Error::Protocol` shape (Task 5 Step 3), `async-trait` dependency (Task 10 Step 2), `app.manage` vs `app.handle().manage` inside `.setup` (Task 12 Step 3), possible `Arc<TasksDb>` needed for the `'static` scheduler future (Tasks 10, 12). Each has an inline fallback instruction.
