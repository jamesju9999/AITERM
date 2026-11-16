# Agent-to-Agent Coordination Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 4 MCP tools (`spawn_tab`, `send_input`, `get_tab_status`, `wait_for_idle`) to AITerm's existing MCP tool server so a Claude Code (or Codex) session running in one AITerm tab can spawn and coordinate with another agent in a second, genuinely visible AITerm tab.

**Architecture:** Backend: `PtyManager` becomes `Arc`-shared between the main app and the MCP server (no more independent-instance pattern — these tools must touch the same tabs the user sees); `PtySession`'s reader loop gains bell-byte (`\x07`) detection as an idle signal, reusing the codebase's existing OSC133-scanning pattern; a new `CoordinationRegistry` tracks which tabs were spawned via MCP (the only tabs `send_input` is ever allowed to touch) and each one's bell-count baseline. Frontend: a new Tauri event (`mcp-coordination-tab-spawned`) tells `TerminalApp` to create a tab bound to an already-existing backend session (`TerminalView` gains an "adopt, don't create" mode), with a distinct icon and a focus-switch heuristic driven by a new `onRunningChange` signal from each tab's command-block state.

**Tech Stack:** Rust (portable-pty, rmcp, axum — same stack as the existing MCP tool server), React/TypeScript (existing Tab/TerminalView/TerminalApp architecture), Vitest, cargo test.

---

## Context for the implementing engineer

This plan assumes zero prior context. Read `docs/superpowers/specs/2026-08-20-agent-coordination-design.md` for the full rationale first. Key facts established during design/planning (verified by reading the real code, not assumed):

- **`PtyManager` is currently NOT `Arc`-wrapped** (`.manage(PtyManager::new())` in `lib.rs`), with 12 `State<'_, PtyManager>` call sites across `src-tauri/src/pty/commands.rs` (8), `src-tauri/src/commands/design.rs` (1), `src-tauri/src/commands/ai.rs` (3). This plan's first task wraps it in `Arc`, matching the existing pattern already used for `ConfigStore`/`SecretStore`/`BridgeState`/`McpToolServerState`. **Unlike the original MCP tool server plan's DB/KB tools**, this feature's MCP server does NOT get its own independent `PtyManager` — it must share the app's real one, or spawned tabs would be invisible to the user.
- **Rust already parses OSC 133 out of raw PTY output** — `src-tauri/src/pty/cd_parser.rs::find_exit_codes`, called from `PtySession`'s reader thread in both `spawn` and `spawn_with_id` (`src-tauri/src/pty/session.rs`). This plan adds a second, much simpler scan (a single byte, `0x07`) to the exact same reader-loop location, following the same "cheap to call unconditionally" pattern.
- **The frontend already has an identical concept**: `TerminalView.tsx`'s `emitAttention("waiting")` fires on `termState.onBell()` (an xterm.js bell event) — i.e. the frontend's existing "waiting" attention state IS a bell-detection mechanism, just implemented client-side via xterm.js instead of server-side by scanning raw bytes. This plan's backend bell detection is the same signal, moved earlier in the pipeline so the Rust MCP server can see it directly.
- **A tab spawned purely via `PtyManager::create_with_app` is invisible in the UI.** AITerm's tab list is frontend `React` state (`TerminalApp.tsx`'s `tabs`); a `TerminalView` component only starts listening to a PTY session's output once it exists and has been told that session's id. The normal flow is frontend-initiated (`TerminalView` calls `createPty()` itself, then reports the resulting id up via `onSessionCreated`). This plan's `spawn_tab` needs the **reverse** flow: backend creates the session first, then tells the frontend about it via a new Tauri event, and `TerminalView` needs a new "adopt an existing session id instead of creating one" mode.
- **`Tab.ptySessionId`** (`src/components/TabBar/index.tsx`) already exists and is normally set *after* the fact via `TerminalView`'s `onSessionCreated` callback. This plan reuses the same field for the reverse direction: a coordination-spawned `Tab` is created with `ptySessionId` **pre-populated**, and `TerminalView` receives it as a new `externalSessionId` prop — if present at mount, it skips calling `createPty()` and adopts the given id directly.
- **There is no existing per-tab "is a command currently running" signal reported up to `TerminalApp`.** `useTerminalBlocks.ts`'s `blocks` array (the source of truth for this) is private to each `TerminalView` instance. This plan adds a new `onRunningChange` callback prop, mirroring the existing `onAttention`/`onSessionCreated` ref-bridging pattern already used in the same file, to support the focus-switch heuristic.
- **Tauri event naming convention in this codebase is plain kebab-case** (`"ai-stream"`, `"api-docs-progress"`, `"code-assistant-event"` — confirmed via grep), not the `scheme://` style used only for the per-session `pty://data/{id}` firehose. This plan's new event is `"mcp-coordination-tab-spawned"`.
- **The established frontend pattern for listening to a Tauri event** is a small typed wrapper in `src/ipc/*.ts` (see `src/ipc/enterprise.ts::onEnterpriseSkillInstalled`) returning `Promise<UnlistenFn>`, consumed in a `useEffect` that stores the unlisten function and calls it on cleanup (see `TerminalApp.tsx`'s existing `unlistenSkill` pattern).

---

### Task 1: `Arc`-wrap `PtyManager`

**Files:**
- Modify: `src-tauri/src/lib.rs:175`
- Modify: `src-tauri/src/pty/commands.rs` (8 signatures)
- Modify: `src-tauri/src/commands/design.rs:315`
- Modify: `src-tauri/src/commands/ai.rs:200,318,563`

- [ ] **Step 1: Wrap the managed state**

In `src-tauri/src/lib.rs`, find:

```rust
        .manage(PtyManager::new())
```

Replace with:

```rust
        .manage(Arc::new(PtyManager::new()))
```

- [ ] **Step 2: Update the 8 signatures in `pty/commands.rs`**

In `src-tauri/src/pty/commands.rs`, every parameter typed `State<'_, PtyManager>` becomes `State<'_, std::sync::Arc<PtyManager>>` — this file has no top-level `Arc` import, so use the fully-qualified path inline, matching how `pty_create` in the same file already does it for `bridge`/`secrets` (`State<'_, std::sync::Arc<crate::bridge::BridgeState>>`). Change all 8 occurrences of:

```rust
    manager: State<'_, PtyManager>,
```

to:

```rust
    manager: State<'_, std::sync::Arc<PtyManager>>,
```

(These are the parameter lists of `pty_create`, `pty_write`, `pty_resize`, `pty_close`, `pty_get_shell_type`, `pty_get_cwd`, `pty_get_recent_output`, `pty_list_dir` — every function body already calls methods like `manager.create_with_app(...)`/`manager.write(...)` etc., which continue to work unchanged: `State<Arc<PtyManager>>` derefs to `Arc<PtyManager>` derefs to `PtyManager`, and method calls resolve through both levels automatically.)

- [ ] **Step 3: Update `commands/design.rs`**

Find:

```rust
    pty_manager: State<'_, PtyManager>,
```

Replace with:

```rust
    pty_manager: State<'_, std::sync::Arc<PtyManager>>,
```

- [ ] **Step 4: Update the 3 signatures in `commands/ai.rs`**

Find all 3 occurrences of:

```rust
    pty_manager: State<'_, PtyManager>,
```

Replace each with:

```rust
    pty_manager: State<'_, std::sync::Arc<PtyManager>>,
```

Note: all 3 of these functions call `context::snapshot(&pty_manager, &session_id)`, where `context::snapshot` (`src-tauri/src/ai/context.rs`) takes `pty_manager: &PtyManager`. This relies on Rust chaining deref coercion through both `State` and `Arc` layers at the call site (`&pty_manager` where `pty_manager: State<'_, Arc<PtyManager>>`) — the same pattern already proven to work for `Db2SidecarState` in an earlier feature on this codebase. If `cargo build` in the next step disagrees, this is the first place to look — but do not preemptively rewrite `context::snapshot`'s signature; let the compiler tell you if this assumption is wrong.

- [ ] **Step 5: Build**

Run: `cd src-tauri && cargo build 2>&1 | tail -80`
Expected: Compiles. If it doesn't, read the actual error before changing anything — this step touches 12 call sites across 3 files and a relatively subtle deref-coercion assumption; fix based on the real compiler output, not speculation.

- [ ] **Step 6: Run the existing PTY test suite for regressions**

Run: `cd src-tauri && cargo test --lib pty::`
Expected: All existing PTY tests still pass — this task is a pure type-signature change, no logic touched.

- [ ] **Step 7: Commit**

```bash
cd src-tauri && git add src/lib.rs src/pty/commands.rs src/commands/design.rs src/commands/ai.rs
git commit -m "refactor(pty): Arc-wrap PtyManager so the MCP tool server can share it with the main app"
```

## Before You Begin

If you have questions, ask them now.

## Your Job

1. Implement exactly what the task specifies
2. Verify build + existing PTY tests pass
3. Commit your work
4. Self-review
5. Report back

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- `cargo build` and test output (pass/fail)
- Files changed
- Self-review findings
- Any issues or concerns

---

### Task 2: Bell detection in `PtySession`

**Files:**
- Modify: `src-tauri/src/pty/session.rs`
- Modify: `src-tauri/src/pty/manager.rs`

- [ ] **Step 1: Write the failing tests**

Add this test module to the bottom of `src-tauri/src/pty/session.rs` (the file already has a `#[cfg(test)] mod tests` block — find it and add these as new `#[test]` functions inside the existing module, using whatever helper the existing tests already use to construct a `PtySession` for testing; if the existing tests spawn a real shell via `PtySession::spawn`, follow that same pattern):

```rust
    #[tokio::test]
    async fn bell_byte_in_output_increments_bell_count() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let session = PtySession::spawn(
            crate::pty::shell::default_shell().unwrap(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            move |chunk| { let _ = tx.send(chunk); },
        ).unwrap();

        assert_eq!(session.bell_count(), 0);

        // Write a command that emits a bell (printf is available on both bash/zsh
        // and, if this ever runs on Windows CI, will simply fail harmlessly since
        // the write still exercises the same code path via whatever the default
        // shell reports as an error).
        session.write(b"printf '\\007'\n").unwrap();

        // Poll briefly for the reader thread to observe it — this is inherently
        // asynchronous (real PTY I/O on a background thread), so a short poll
        // loop is appropriate here, not a fixed sleep.
        let mut seen = false;
        for _ in 0..50 {
            if session.bell_count() > 0 { seen = true; break; }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(seen, "expected bell_count() to increment after a bell byte was written and echoed");

        let _ = rx.try_recv(); // drain, avoid unused warning
    }

    #[test]
    fn bell_count_starts_at_zero_for_a_fresh_session() {
        let session = PtySession::spawn(
            crate::pty::shell::default_shell().unwrap(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        ).unwrap();
        assert_eq!(session.bell_count(), 0);
    }
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cd src-tauri && cargo test --lib pty::session::tests::bell`
Expected: Compile error — `bell_count` method doesn't exist yet.

- [ ] **Step 3: Add the field**

In `src-tauri/src/pty/session.rs`, add the new import at the top (alongside the existing `use std::sync::Arc;`):

```rust
use std::sync::atomic::{AtomicU64, Ordering};
```

Find the `PtySession` struct definition:

```rust
pub struct PtySession {
    pub id: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
    shell_variant: ShellVariant,
    /// Shared with the reader thread, which commits a pending cd (see
    /// `pending_cds`) once it observes a confirming OSC 133 D exit code.
    cwd: Arc<Mutex<PathBuf>>,
    previous_cwd: Arc<Mutex<Option<PathBuf>>>,
    line_buffer: Mutex<Vec<u8>>,
    /// ANSI escape-sequence state for the line buffer: 0=normal, 1=saw ESC, 2=in CSI (ESC [).
    line_esc_state: Mutex<u8>,
    /// Ring buffer capturing raw PTY output for AI context. Shared with the reader thread.
    output_ring: Arc<Mutex<VecDeque<u8>>>,
    /// cd attempts staged by `apply_cd_if_any` (write path) for Bash/Pwsh
    /// sessions, each removed by the reader thread once it sees the matching
    /// OSC 133 D marker — committed to cwd/previous_cwd only if that marker
    /// reports exit code 0. See `cd_parser::find_exit_codes`.
    pending_cds: Arc<Mutex<VecDeque<ParsedCd>>>,
}
```

Add a new field after `pending_cds`:

```rust
pub struct PtySession {
    pub id: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
    shell_variant: ShellVariant,
    /// Shared with the reader thread, which commits a pending cd (see
    /// `pending_cds`) once it observes a confirming OSC 133 D exit code.
    cwd: Arc<Mutex<PathBuf>>,
    previous_cwd: Arc<Mutex<Option<PathBuf>>>,
    line_buffer: Mutex<Vec<u8>>,
    /// ANSI escape-sequence state for the line buffer: 0=normal, 1=saw ESC, 2=in CSI (ESC [).
    line_esc_state: Mutex<u8>,
    /// Ring buffer capturing raw PTY output for AI context. Shared with the reader thread.
    output_ring: Arc<Mutex<VecDeque<u8>>>,
    /// cd attempts staged by `apply_cd_if_any` (write path) for Bash/Pwsh
    /// sessions, each removed by the reader thread once it sees the matching
    /// OSC 133 D marker — committed to cwd/previous_cwd only if that marker
    /// reports exit code 0. See `cd_parser::find_exit_codes`.
    pending_cds: Arc<Mutex<VecDeque<ParsedCd>>>,
    /// Counts how many output chunks have contained at least one bell byte
    /// (`0x07`). Used by the MCP tool server's agent-coordination tools as an
    /// idle signal: both Claude Code and Codex CLI fall back to a plain
    /// terminal bell for "waiting for input" notifications when they can't
    /// detect a richer-notification-capable terminal (verified against both
    /// projects' source — see the design doc). A monotonic counter rather
    /// than a boolean so a caller can detect "a *new* bell happened since I
    /// last checked" by comparing against a remembered baseline, without this
    /// field needing any consuming/resetting behavior of its own.
    bell_count: Arc<AtomicU64>,
}
```

- [ ] **Step 4: Initialize it in both `spawn` and `spawn_with_id`, and scan for bells in both reader loops**

In `src-tauri/src/pty/session.rs`, `spawn`'s setup code, find:

```rust
        let pending_cds: Arc<Mutex<VecDeque<ParsedCd>>> = Arc::new(Mutex::new(VecDeque::new()));
        let pending_cds_for_thread = Arc::clone(&pending_cds);

        let reader_thread = thread::Builder::new()
            .name(format!("pty-reader-{}", id))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break, // EOF: shell exited
                        Ok(n) => {
                            let chunk = buf[..n].to_vec();
                            {
                                let mut ring = ring_for_thread.lock();
                                const RING_CAP: usize = 8 * 1024;
                                for &b in &chunk {
                                    if ring.len() >= RING_CAP { ring.pop_front(); }
                                    ring.push_back(b);
                                }
                            }
                            if matches!(shell_variant, ShellVariant::Bash | ShellVariant::Pwsh) {
                                confirm_pending_cds_from_output(
                                    &chunk,
                                    &pending_cds_for_thread,
                                    &cwd_for_thread,
                                    &previous_cwd_for_thread,
                                );
                            }
                            on_data(chunk);
                        }
                        Err(e) => {
                            eprintln!("pty reader error: {e}");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| PtyError::Internal(format!("spawn reader thread: {e}")))?;

        Ok(Self {
            id,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader_thread: Mutex::new(Some(reader_thread)),
            shell_variant,
            cwd,
            previous_cwd,
            line_buffer: Mutex::new(Vec::new()),
            line_esc_state: Mutex::new(0),
            output_ring,
            pending_cds,
        })
    }
```

Replace with (adds `bell_count`/`bell_count_for_thread` setup, one new `if` inside the loop, and the new field in the returned `Self`):

```rust
        let pending_cds: Arc<Mutex<VecDeque<ParsedCd>>> = Arc::new(Mutex::new(VecDeque::new()));
        let pending_cds_for_thread = Arc::clone(&pending_cds);
        let bell_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let bell_count_for_thread = Arc::clone(&bell_count);

        let reader_thread = thread::Builder::new()
            .name(format!("pty-reader-{}", id))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break, // EOF: shell exited
                        Ok(n) => {
                            let chunk = buf[..n].to_vec();
                            {
                                let mut ring = ring_for_thread.lock();
                                const RING_CAP: usize = 8 * 1024;
                                for &b in &chunk {
                                    if ring.len() >= RING_CAP { ring.pop_front(); }
                                    ring.push_back(b);
                                }
                            }
                            if matches!(shell_variant, ShellVariant::Bash | ShellVariant::Pwsh) {
                                confirm_pending_cds_from_output(
                                    &chunk,
                                    &pending_cds_for_thread,
                                    &cwd_for_thread,
                                    &previous_cwd_for_thread,
                                );
                            }
                            // Cheap to call unconditionally, same reasoning as the
                            // OSC133 scan above: a chunk with no bell byte is the
                            // overwhelmingly common case. One increment per chunk
                            // containing at least one bell is enough — callers only
                            // ever check "did the count change since my baseline",
                            // never the exact number of bells.
                            if chunk.contains(&0x07) {
                                bell_count_for_thread.fetch_add(1, Ordering::SeqCst);
                            }
                            on_data(chunk);
                        }
                        Err(e) => {
                            eprintln!("pty reader error: {e}");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| PtyError::Internal(format!("spawn reader thread: {e}")))?;

        Ok(Self {
            id,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader_thread: Mutex::new(Some(reader_thread)),
            shell_variant,
            cwd,
            previous_cwd,
            line_buffer: Mutex::new(Vec::new()),
            line_esc_state: Mutex::new(0),
            output_ring,
            pending_cds,
            bell_count,
        })
    }
```

Now do the **exact same change** in `spawn_with_id` (the second, near-identical copy of this setup/loop/`Ok(Self { ... })` a bit further down the same file) — same 3 edits: add `bell_count`/`bell_count_for_thread` setup right after `pending_cds_for_thread`, add the same `if chunk.contains(&0x07) { ... }` block right after the `confirm_pending_cds_from_output` call inside that copy's reader loop, add `bell_count,` to that copy's `Ok(Self { ... })`. This file already has this exact kind of duplication between `spawn`/`spawn_with_id` (pre-existing, not something to unify as part of this task — see the plan's context notes on surgical changes) — mirror the pattern, don't refactor it away.

- [ ] **Step 5: Add the accessor method**

In the same file, find the existing accessor methods (e.g. right after `get_recent_output`):

```rust
    pub fn get_recent_output(&self, max_bytes: usize) -> Option<String> {
```

Add a new method nearby (after `get_recent_output`'s closing brace):

```rust
    /// Monotonic count of output chunks that have contained at least one bell
    /// byte (`0x07`) since this session started. See the field doc comment on
    /// `bell_count` for why this is a counter, not a boolean.
    pub fn bell_count(&self) -> u64 {
        self.bell_count.load(Ordering::SeqCst)
    }
```

- [ ] **Step 6: Add the `PtyManager`-level accessor**

In `src-tauri/src/pty/manager.rs`, find:

```rust
    /// Return recent terminal output (ANSI-stripped) for the given session.
    pub fn get_recent_output(&self, id: &str, max_bytes: usize) -> Option<String> {
        self.sessions.lock().get(id).and_then(|s| s.get_recent_output(max_bytes))
    }
```

Add a new method right after it:

```rust
    /// Bell-byte count for the given session, or `None` if the session
    /// doesn't exist. See `PtySession::bell_count` for what this counts.
    pub fn bell_count(&self, id: &str) -> Option<u64> {
        self.sessions.lock().get(id).map(|s| s.bell_count())
    }
```

- [ ] **Step 7: Run the tests**

Run: `cd src-tauri && cargo test --lib pty::session::tests::bell`
Expected: Both new tests pass.

Run: `cd src-tauri && cargo test --lib pty::`
Expected: All PTY tests pass, no regressions.

- [ ] **Step 8: Commit**

```bash
cd src-tauri && git add src/pty/session.rs src/pty/manager.rs
git commit -m "feat(pty): detect terminal bell bytes as an idle signal for agent coordination"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Test results
- Files changed
- Self-review findings (confirm both `spawn` AND `spawn_with_id` got the identical 3-part change — this is the easiest place to miss one copy)
- Any issues or concerns

---

### Task 3: `McpToolServerConfig` — add `coordination_enabled`

**Files:**
- Modify: `src-tauri/src/config/types.rs`

- [ ] **Step 1: Add the field**

Find:

```rust
pub struct McpToolServerConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_mcp_tool_server_port")]
    pub port: u16,
}

impl Default for McpToolServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_mcp_tool_server_port(),
        }
    }
}
```

Replace with:

```rust
pub struct McpToolServerConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_mcp_tool_server_port")]
    pub port: u16,

    /// Separately gates the 4 agent-coordination tools (spawn_tab/send_input/
    /// get_tab_status/wait_for_idle) — a strictly higher-risk capability than
    /// the DB/knowledge-base tools (it can run arbitrary commands in tabs it
    /// spawns), so it defaults off even when the server itself is enabled.
    #[serde(default)]
    pub coordination_enabled: bool,
}

impl Default for McpToolServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_mcp_tool_server_port(),
            coordination_enabled: false,
        }
    }
}
```

- [ ] **Step 2: Build**

Run: `cd src-tauri && cargo build`
Expected: Compiles. `#[serde(default)]` means old config files without this field still deserialize fine.

- [ ] **Step 3: Commit**

```bash
cd src-tauri && git add src/config/types.rs
git commit -m "feat(config): add coordination_enabled, separately gating the agent-coordination MCP tools"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- `cargo build` output
- Files changed
- Self-review findings

---

### Task 4: `mcp_server/coordination_ops.rs` — business logic

**Files:**
- Create: `src-tauri/src/mcp_server/coordination_ops.rs`
- Modify: `src-tauri/src/mcp_server/mod.rs` (add `pub mod coordination_ops;`)

- [ ] **Step 1: Write the implementation and tests together**

Create `src-tauri/src/mcp_server/coordination_ops.rs`:

```rust
//! Business logic for the agent-coordination MCP tools (`spawn_tab`,
//! `send_input`, `get_tab_status`, `wait_for_idle`). Kept separate from
//! `tools.rs`'s `#[tool]`-annotated methods so this logic is unit-testable
//! without going through the MCP protocol layer — same split as
//! `db_ops.rs`/`kb_ops.rs`.
//!
//! Security note: `send_input`/`get_tab_status`/`wait_for_idle` all refuse to
//! touch any `tab_id` that isn't in `CoordinationRegistry` — i.e. a tab this
//! server itself spawned via `spawn_tab`. A tab the human opened by hand is
//! never a valid target. See the design doc for the full rationale.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;
use portable_pty::PtySize;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::pty::manager::PtyManager;

const DEFAULT_WAIT_SECONDS: u64 = 300;
const MAX_WAIT_SECONDS: u64 = 1800;
const POLL_INTERVAL_MS: u64 = 250;
/// Matches `pty_get_recent_output`'s existing cap (`src-tauri/src/pty/commands.rs`).
const RECENT_OUTPUT_BYTES: usize = 4096;

/// Tracks which tabs this MCP server has spawned (the only valid `send_input`/
/// `get_tab_status`/`wait_for_idle` targets) and, for each, the `PtySession`
/// bell count as of the last `send_input` call (or spawn time) — the baseline
/// a fresh bell count is compared against to answer "has this tab replied
/// since we last prompted it", without needing any consuming/stateful latch
/// on `PtySession` itself.
///
/// Constructed once per running MCP tool server (in `McpToolServerState::start`)
/// and shared — via `Arc` — across every `AiTermTools` instance the
/// `StreamableHttpService` factory closure creates (one per MCP session), so
/// a tab spawned by one MCP session is visible to `get_tab_status` calls from
/// any other session talking to the same running server.
#[derive(Default)]
pub struct CoordinationRegistry {
    tabs: Mutex<HashMap<String, u64>>,
}

impl CoordinationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_known(&self, tab_id: &str) -> bool {
        self.tabs.lock().contains_key(tab_id)
    }

    fn record_baseline(&self, tab_id: &str, baseline: u64) {
        self.tabs.lock().insert(tab_id.to_string(), baseline);
    }

    fn baseline(&self, tab_id: &str) -> Option<u64> {
        self.tabs.lock().get(tab_id).copied()
    }
}

#[derive(Serialize)]
struct TabStatus {
    idle: bool,
    recent_output: String,
}

#[derive(Serialize)]
struct WaitResult {
    idle: bool,
    recent_output: String,
    timed_out: bool,
}

/// Payload for the `mcp-coordination-tab-spawned` Tauri event, telling the
/// frontend to create a tab bound to this already-existing backend session.
#[derive(Serialize, Clone)]
struct TabSpawnedEvent {
    session_id: String,
    command: Option<String>,
}

pub(crate) async fn spawn_tab(
    app: &AppHandle,
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    cwd: Option<String>,
    command: Option<String>,
) -> Result<String, String> {
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let cwd_path = cwd.map(std::path::PathBuf::from);
    let tab_id = pty_manager
        .create_with_app(app.clone(), size, cwd_path, None)
        .map_err(|e| e.to_string())?;

    // Fresh session: bell_count() is 0 right now. Recording that as the
    // baseline means get_tab_status/wait_for_idle report "not idle" until the
    // first bell — reasonable, since nothing has run yet either way.
    registry.record_baseline(&tab_id, 0);

    if let Some(cmd) = &command {
        pty_manager
            .write(&tab_id, format!("{cmd}\n").as_bytes())
            .map_err(|e| e.to_string())?;
    }

    let _ = app.emit("mcp-coordination-tab-spawned", TabSpawnedEvent {
        session_id: tab_id.clone(),
        command,
    });

    Ok(tab_id)
}

pub(crate) fn send_input(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
    text: &str,
) -> Result<String, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    pty_manager
        .write(tab_id, format!("{text}\n").as_bytes())
        .map_err(|e| e.to_string())?;

    // Reset the baseline to the count *as of right now*, before any reply has
    // had a chance to arrive — so idle only flips true again once a *new*
    // bell (a reply to this specific input) is observed, not a stale one from
    // before this send.
    let current = pty_manager.bell_count(tab_id).unwrap_or(0);
    registry.record_baseline(tab_id, current);

    Ok(format!("sent to {tab_id}"))
}

fn status_for(pty_manager: &PtyManager, registry: &CoordinationRegistry, tab_id: &str) -> Result<TabStatus, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    let baseline = registry.baseline(tab_id).unwrap_or(0);
    let current = pty_manager
        .bell_count(tab_id)
        .ok_or_else(|| format!("tab_id '{tab_id}' is no longer running (it may have been closed)"))?;
    let recent_output = pty_manager
        .get_recent_output(tab_id, RECENT_OUTPUT_BYTES)
        .unwrap_or_default();
    Ok(TabStatus { idle: current > baseline, recent_output })
}

pub(crate) fn get_tab_status(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
) -> Result<String, String> {
    let status = status_for(pty_manager, registry, tab_id)?;
    serde_json::to_string_pretty(&status).map_err(|e| e.to_string())
}

pub(crate) async fn wait_for_idle(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
    timeout_seconds: Option<u64>,
) -> Result<String, String> {
    // Validate up front so an unknown tab_id fails fast instead of after a
    // full timeout's worth of silent polling.
    status_for(pty_manager, registry, tab_id)?;

    let timeout = Duration::from_secs(timeout_seconds.unwrap_or(DEFAULT_WAIT_SECONDS).min(MAX_WAIT_SECONDS));
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let status = status_for(pty_manager, registry, tab_id)?;
        if status.idle {
            let result = WaitResult { idle: true, recent_output: status.recent_output, timed_out: false };
            return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
        }
        if tokio::time::Instant::now() >= deadline {
            let result = WaitResult { idle: false, recent_output: status.recent_output, timed_out: true };
            return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::shell::default_shell;

    fn pty_size() -> PtySize {
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }
    }

    #[tokio::test]
    async fn send_input_rejects_a_tab_id_not_in_the_registry() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let err = send_input(&pty_manager, &registry, "not-a-real-tab", "hello").unwrap_err();
        assert!(err.contains("was not created by spawn_tab"), "{err}");
    }

    #[tokio::test]
    async fn get_tab_status_rejects_a_tab_id_not_in_the_registry() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let err = get_tab_status(&pty_manager, &registry, "not-a-real-tab").unwrap_err();
        assert!(err.contains("was not created by spawn_tab"), "{err}");
    }

    #[tokio::test]
    async fn send_input_and_get_tab_status_work_on_a_spawned_session() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();

        // Bypass spawn_tab (it needs a real AppHandle) and construct the
        // PtySession + registry entry directly, exactly as spawn_tab itself
        // would — this tests send_input/get_tab_status's own logic, not
        // spawn_tab's AppHandle-dependent plumbing (covered separately by the
        // integration test in Task 10).
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0);

        let sent = send_input(&pty_manager, &registry, &tab_id, "echo hi").unwrap();
        assert_eq!(sent, format!("sent to {tab_id}"));

        let status_json = get_tab_status(&pty_manager, &registry, &tab_id).unwrap();
        assert!(status_json.contains("\"idle\""), "{status_json}");
        assert!(status_json.contains("\"recent_output\""), "{status_json}");
    }

    #[tokio::test]
    async fn wait_for_idle_times_out_when_no_bell_arrives() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0);

        let result_json = wait_for_idle(&pty_manager, &registry, &tab_id, Some(1)).await.unwrap();
        assert!(result_json.contains("\"timed_out\": true"), "{result_json}");
        assert!(result_json.contains("\"idle\": false"), "{result_json}");
    }

    #[tokio::test]
    async fn wait_for_idle_returns_idle_once_a_bell_is_observed() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0);

        pty_manager.write(&tab_id, b"printf '\\007'\n").unwrap();

        let result_json = wait_for_idle(&pty_manager, &registry, &tab_id, Some(10)).await.unwrap();
        assert!(result_json.contains("\"idle\": true"), "{result_json}");
        assert!(result_json.contains("\"timed_out\": false"), "{result_json}");
    }

    #[tokio::test]
    async fn wait_for_idle_rejects_a_tab_id_not_in_the_registry() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let err = wait_for_idle(&pty_manager, &registry, "not-a-real-tab", None).await.unwrap_err();
        assert!(err.contains("was not created by spawn_tab"), "{err}");
    }
}
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/mcp_server/mod.rs`, find:

```rust
pub mod db_ops;
pub mod kb_ops;
pub mod server;
pub mod tools;
```

Replace with:

```rust
pub mod coordination_ops;
pub mod db_ops;
pub mod kb_ops;
pub mod server;
pub mod tools;
```

- [ ] **Step 3: This can't compile yet — that's expected**

`AiTermTools` (Task 5) doesn't call any of these functions yet, and `PtyManager::create_with_callback` (used by the tests above) already exists (`src-tauri/src/pty/manager.rs` — it's the low-level test helper already used by `PtyManager`'s own existing tests). Run `cd src-tauri && cargo test --lib mcp_server::coordination_ops` now — it should actually compile and pass on its own, since this file only depends on already-existing `PtyManager`/`CoordinationRegistry` (defined in this same file) and doesn't need `tools.rs`'s changes. If it doesn't compile, fix based on the real error (likely a missing import or a `PtyManager` method signature mismatch — re-check against `src-tauri/src/pty/manager.rs` directly).

- [ ] **Step 4: Run the tests**

Run: `cd src-tauri && cargo test --lib mcp_server::coordination_ops`
Expected: 7 tests pass.

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add src/mcp_server/coordination_ops.rs src/mcp_server/mod.rs
git commit -m "feat(mcp-server): agent-coordination tool business logic (coordination_ops)"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Test results (exact pass/fail counts)
- Files changed
- Self-review findings
- Any issues or concerns

---

### Task 5: `mcp_server/tools.rs` — 4 new `#[tool]` methods

**Files:**
- Modify: `src-tauri/src/mcp_server/tools.rs`

- [ ] **Step 1: Add imports and new `AiTermTools` fields**

Find:

```rust
use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};
use sqlx::SqlitePool;

use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::secret::SecretStore;

use super::{db_ops, kb_ops};
```

Replace with:

```rust
use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};
use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::pty::manager::PtyManager;
use crate::secret::SecretStore;

use super::coordination_ops::CoordinationRegistry;
use super::{coordination_ops, db_ops, kb_ops};
```

Find:

```rust
#[derive(Clone)]
pub struct AiTermTools {
    db_manager: Arc<DbManager>,
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    sidecar: Arc<Db2SidecarState>,
    kb_pool: SqlitePool,
    tool_router: ToolRouter<AiTermTools>,
}

#[tool_router]
impl AiTermTools {
    pub fn new(
        db_manager: Arc<DbManager>,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        sidecar: Arc<Db2SidecarState>,
        kb_pool: SqlitePool,
    ) -> Self {
        Self {
            db_manager,
            config,
            secrets,
            sidecar,
            kb_pool,
            tool_router: Self::tool_router(),
        }
    }
```

Replace with:

```rust
#[derive(Clone)]
pub struct AiTermTools {
    db_manager: Arc<DbManager>,
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    sidecar: Arc<Db2SidecarState>,
    kb_pool: SqlitePool,
    pty_manager: Arc<PtyManager>,
    app: AppHandle,
    coordination_registry: Arc<CoordinationRegistry>,
    tool_router: ToolRouter<AiTermTools>,
}

#[tool_router]
impl AiTermTools {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db_manager: Arc<DbManager>,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        sidecar: Arc<Db2SidecarState>,
        kb_pool: SqlitePool,
        pty_manager: Arc<PtyManager>,
        app: AppHandle,
        coordination_registry: Arc<CoordinationRegistry>,
    ) -> Self {
        Self {
            db_manager,
            config,
            secrets,
            sidecar,
            kb_pool,
            pty_manager,
            app,
            coordination_registry,
            tool_router: Self::tool_router(),
        }
    }

    /// Every coordination tool calls this first. `Ok(())` means proceed;
    /// `Err` is a ready-to-return tool-level error explaining the toggle is
    /// off. Kept as one shared check (not duplicated per tool) so the wording
    /// can't drift between the 4 call sites.
    fn require_coordination_enabled(&self) -> Result<(), String> {
        if self.config.get().mcp_tool_server.coordination_enabled {
            Ok(())
        } else {
            Err("Agent coordination tools are disabled. Enable them in AITerm's Settings → MCP Tool Server page.".to_string())
        }
    }
```

- [ ] **Step 2: Add the 4 argument structs**

Find:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadDocumentArgs {
    /// Notebook id as returned by `list_notebooks`.
    pub notebook_id: String,
    /// Document path exactly as returned by `search_documents`.
    pub path: String,
}
```

Add right after it (before `#[derive(Clone)] pub struct AiTermTools`):

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SpawnTabArgs {
    /// Working directory for the new tab. Defaults to the user's home directory if omitted.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Initial command to run in the new tab once it's ready (e.g. "claude" or "codex").
    #[serde(default)]
    pub command: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendInputArgs {
    /// Tab id as returned by `spawn_tab`. Must be a tab this server spawned — never one the user opened by hand.
    pub tab_id: String,
    /// Text to send, as if typed into the tab followed by Enter.
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TabIdArgs {
    /// Tab id as returned by `spawn_tab`.
    pub tab_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WaitForIdleArgs {
    /// Tab id as returned by `spawn_tab`.
    pub tab_id: String,
    /// Max seconds to wait (default 300, capped at 1800).
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}
```

- [ ] **Step 3: Add the 4 `#[tool]` methods**

Find the end of the `#[tool_router] impl AiTermTools { ... }` block — the last tool method, `read_document`, followed by the closing `}` of the impl block:

```rust
    #[tool(description = "Read a knowledge base document's full converted content by its exact path (as shown in search_documents results).")]
    async fn read_document(
        &self,
        Parameters(ReadDocumentArgs { notebook_id, path }): Parameters<ReadDocumentArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(kb_ops::read_document(&self.kb_pool, &notebook_id, &path).await)
    }
}
```

Replace with (adds the 4 new tools before the closing `}`):

```rust
    #[tool(description = "Read a knowledge base document's full converted content by its exact path (as shown in search_documents results).")]
    async fn read_document(
        &self,
        Parameters(ReadDocumentArgs { notebook_id, path }): Parameters<ReadDocumentArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(kb_ops::read_document(&self.kb_pool, &notebook_id, &path).await)
    }

    #[tool(description = "Spawn a new AITerm terminal tab, visible to the user, optionally running an initial command (e.g. 'claude' or 'codex') to start another coding agent in it. Returns the new tab's id. Disabled by default — must be enabled in Settings.")]
    async fn spawn_tab(
        &self,
        Parameters(SpawnTabArgs { cwd, command }): Parameters<SpawnTabArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.require_coordination_enabled() {
            return to_call_result(Err(e));
        }
        to_call_result(coordination_ops::spawn_tab(&self.app, &self.pty_manager, &self.coordination_registry, cwd, command).await)
    }

    #[tool(description = "Send text (as if typed, followed by Enter) to a tab previously created by spawn_tab. Cannot target a tab the user opened by hand. Disabled by default — must be enabled in Settings.")]
    async fn send_input(
        &self,
        Parameters(SendInputArgs { tab_id, text }): Parameters<SendInputArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.require_coordination_enabled() {
            return to_call_result(Err(e));
        }
        to_call_result(coordination_ops::send_input(&self.pty_manager, &self.coordination_registry, &tab_id, &text))
    }

    #[tool(description = "Check whether a spawn_tab-created tab is idle (a terminal bell was observed since the last send_input, meaning the agent inside replied and is waiting for more input) and get its recent output. Disabled by default — must be enabled in Settings.")]
    async fn get_tab_status(
        &self,
        Parameters(TabIdArgs { tab_id }): Parameters<TabIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.require_coordination_enabled() {
            return to_call_result(Err(e));
        }
        to_call_result(coordination_ops::get_tab_status(&self.pty_manager, &self.coordination_registry, &tab_id))
    }

    #[tool(description = "Block until a spawn_tab-created tab becomes idle (see get_tab_status) or the timeout elapses, then return its status. Disabled by default — must be enabled in Settings.")]
    async fn wait_for_idle(
        &self,
        Parameters(WaitForIdleArgs { tab_id, timeout_seconds }): Parameters<WaitForIdleArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.require_coordination_enabled() {
            return to_call_result(Err(e));
        }
        to_call_result(coordination_ops::wait_for_idle(&self.pty_manager, &self.coordination_registry, &tab_id, timeout_seconds).await)
    }
}
```

- [ ] **Step 4: This can't compile yet — `server.rs` (Task 6) still constructs `AiTermTools::new()` with the old 5-argument signature.** Do NOT try to fix `server.rs` from this task — that's Task 6. Just confirm the error you get is specifically about `AiTermTools::new`'s call site in `server.rs` having too few arguments, nothing else:

Run: `cd src-tauri && cargo build 2>&1 | grep -A5 "AiTermTools::new"`
Expected: An arity-mismatch error pointing at `server.rs`'s call site. If you see any OTHER kind of error (e.g. inside `tools.rs` itself), stop and report BLOCKED — that would mean something in this task's own edits is wrong, not just "the next task hasn't happened yet."

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add src/mcp_server/tools.rs
git commit -m "feat(mcp-server): add spawn_tab/send_input/get_tab_status/wait_for_idle MCP tools"
```

(This commit will leave the crate in a temporarily non-building state — that's expected and matches how the original MCP tool server plan sequenced its own `mod.rs`/`tools.rs`/`server.rs` additions before wiring them together. Task 6 fixes it.)

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Confirmation the ONLY compile error is the expected `AiTermTools::new` arity mismatch in `server.rs`
- Files changed
- Self-review findings
- Any issues or concerns

---

### Task 6: `mcp_server/server.rs` and `mcp_server/mod.rs` — thread the new dependencies through

**Files:**
- Modify: `src-tauri/src/mcp_server/server.rs`
- Modify: `src-tauri/src/mcp_server/mod.rs`

- [ ] **Step 1: Update `server.rs`'s imports and `router()` signature**

Find:

```rust
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use sqlx::SqlitePool;

use crate::bridge::auth as bridge_auth;
use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::secret::SecretStore;

use super::tools::AiTermTools;
```

Replace with:

```rust
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::bridge::auth as bridge_auth;
use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::pty::manager::PtyManager;
use crate::secret::SecretStore;

use super::coordination_ops::CoordinationRegistry;
use super::tools::AiTermTools;
```

Find:

```rust
pub fn router(
    token: Arc<String>,
    db_manager: Arc<DbManager>,
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    sidecar: Arc<Db2SidecarState>,
    kb_pool: SqlitePool,
) -> Router {
    let service = StreamableHttpService::new(
        move || {
            Ok(AiTermTools::new(
                db_manager.clone(),
                config.clone(),
                secrets.clone(),
                sidecar.clone(),
                kb_pool.clone(),
            ))
        },
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
```

Replace with:

```rust
#[allow(clippy::too_many_arguments)]
pub fn router(
    token: Arc<String>,
    db_manager: Arc<DbManager>,
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    sidecar: Arc<Db2SidecarState>,
    kb_pool: SqlitePool,
    pty_manager: Arc<PtyManager>,
    app: AppHandle,
    coordination_registry: Arc<CoordinationRegistry>,
) -> Router {
    let service = StreamableHttpService::new(
        move || {
            Ok(AiTermTools::new(
                db_manager.clone(),
                config.clone(),
                secrets.clone(),
                sidecar.clone(),
                kb_pool.clone(),
                pty_manager.clone(),
                app.clone(),
                coordination_registry.clone(),
            ))
        },
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
```

- [ ] **Step 2: Update `mod.rs`'s imports, `start()` signature, and body**

In `src-tauri/src/mcp_server/mod.rs`, find:

```rust
use crate::config::ConfigStore;
use crate::db::{resolve_db2_sidecar_path, Db2SidecarState};
use crate::db::knowledge_base::KnowledgeBaseDb;
use crate::db::manager::DbManager;
use crate::secret::SecretStore;
```

Replace with:

```rust
use crate::config::ConfigStore;
use crate::db::{resolve_db2_sidecar_path, Db2SidecarState};
use crate::db::knowledge_base::KnowledgeBaseDb;
use crate::db::manager::DbManager;
use crate::pty::manager::PtyManager;
use crate::secret::SecretStore;

use coordination_ops::CoordinationRegistry;
```

Find:

```rust
    pub async fn start(
        &self,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        token: String,
        port: u16,
    ) -> anyhow::Result<()> {
        self.stop();

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            anyhow::anyhow!("無法綁定 127.0.0.1:{port}（{e}）。請在設定裡換一個埠。")
        })?;

        let db_manager = Arc::new(DbManager::new());
        let sidecar = Arc::new(Db2SidecarState::new(resolve_db2_sidecar_path()));
        let kb_pool = KnowledgeBaseDb::new().await.pool;

        let app = server::router(Arc::new(token), db_manager, config, secrets, sidecar, kb_pool);
```

Replace with:

```rust
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        token: String,
        port: u16,
        app_handle: tauri::AppHandle,
        pty_manager: Arc<PtyManager>,
    ) -> anyhow::Result<()> {
        self.stop();

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            anyhow::anyhow!("無法綁定 127.0.0.1:{port}（{e}）。請在設定裡換一個埠。")
        })?;

        let db_manager = Arc::new(DbManager::new());
        let sidecar = Arc::new(Db2SidecarState::new(resolve_db2_sidecar_path()));
        let kb_pool = KnowledgeBaseDb::new().await.pool;
        let coordination_registry = Arc::new(CoordinationRegistry::new());

        let app = server::router(
            Arc::new(token), db_manager, config, secrets, sidecar, kb_pool,
            pty_manager, app_handle, coordination_registry,
        );
```

Note the local variable rename risk here: the existing code already has a local variable named `app` (the axum `Router` returned by `server::router(...)`) — this task's new parameter is deliberately named `app_handle`, not `app`, specifically to avoid colliding with that existing local. Do not rename the existing `app` local to something else; keep this task's new parameter as `app_handle` throughout.

- [ ] **Step 3: Build**

Run: `cd src-tauri && cargo build 2>&1 | tail -80`
Expected: New errors now, this time pointing at `commands/mcp_server.rs`'s call to `server.start(...)` (still the old 4-argument form) and `lib.rs`'s auto-start block's call to `server.start(...)` (same). This is expected — Task 7 and Task 8 fix those two remaining call sites. If you see errors anywhere else (inside `mod.rs`/`server.rs`/`tools.rs` themselves), stop and report BLOCKED.

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/mcp_server/server.rs src/mcp_server/mod.rs
git commit -m "feat(mcp-server): thread PtyManager/AppHandle/CoordinationRegistry through router() and start()"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Confirmation the only remaining compile errors are in `commands/mcp_server.rs` and `lib.rs` (both expected, fixed in later tasks)
- Files changed
- Self-review findings
- Any issues or concerns

---

### Task 7: `commands/mcp_server.rs` — thread the new dependencies into the Tauri command

**Files:**
- Modify: `src-tauri/src/commands/mcp_server.rs`

- [ ] **Step 1: Update imports and `mcp_tool_server_apply`**

Find:

```rust
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::bridge::auth as bridge_auth;
use crate::config::types::McpToolServerConfig;
use crate::config::ConfigStore;
use crate::mcp_server::{McpToolServerState, MCP_TOOL_SERVER_TOKEN_KEY};
use crate::secret::SecretStore;
```

Replace with:

```rust
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::bridge::auth as bridge_auth;
use crate::config::types::McpToolServerConfig;
use crate::config::ConfigStore;
use crate::mcp_server::{McpToolServerState, MCP_TOOL_SERVER_TOKEN_KEY};
use crate::pty::manager::PtyManager;
use crate::secret::SecretStore;
```

Find:

```rust
/// Starts or stops the server according to the currently saved config. Called
/// after the settings page saves.
#[tauri::command]
pub async fn mcp_tool_server_apply(
    server: State<'_, Arc<McpToolServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<McpToolServerStatus, String> {
    let cfg = config.get().mcp_tool_server;
    if !cfg.enabled {
        server.stop();
        return Ok(McpToolServerStatus { running: false, port: None, token: None, error: None });
    }

    let token = ensure_token(&secrets).map_err(|e| e.to_string())?;
    match server
        .start(config.inner().clone(), secrets.inner().clone(), token.clone(), cfg.port)
        .await
    {
```

Replace with:

```rust
/// Starts or stops the server according to the currently saved config. Called
/// after the settings page saves.
#[tauri::command]
pub async fn mcp_tool_server_apply(
    app: AppHandle,
    server: State<'_, Arc<McpToolServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    pty_manager: State<'_, Arc<PtyManager>>,
) -> Result<McpToolServerStatus, String> {
    let cfg = config.get().mcp_tool_server;
    if !cfg.enabled {
        server.stop();
        return Ok(McpToolServerStatus { running: false, port: None, token: None, error: None });
    }

    let token = ensure_token(&secrets).map_err(|e| e.to_string())?;
    match server
        .start(config.inner().clone(), secrets.inner().clone(), token.clone(), cfg.port, app, pty_manager.inner().clone())
        .await
    {
```

- [ ] **Step 2: `mcp_tool_server_set_config` also calls `mcp_tool_server_apply` — update its call site too**

Find:

```rust
/// Saves the config and immediately applies it (starts/stops as needed).
#[tauri::command]
pub async fn mcp_tool_server_set_config(
    server: State<'_, Arc<McpToolServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    value: McpToolServerConfig,
) -> Result<McpToolServerStatus, String> {
    config
        .update(|c| c.mcp_tool_server = value.clone())
        .map_err(|e| e.to_string())?;
    mcp_tool_server_apply(server, config, secrets).await
}
```

Replace with:

```rust
/// Saves the config and immediately applies it (starts/stops as needed).
#[tauri::command]
pub async fn mcp_tool_server_set_config(
    app: AppHandle,
    server: State<'_, Arc<McpToolServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    pty_manager: State<'_, Arc<PtyManager>>,
    value: McpToolServerConfig,
) -> Result<McpToolServerStatus, String> {
    config
        .update(|c| c.mcp_tool_server = value.clone())
        .map_err(|e| e.to_string())?;
    mcp_tool_server_apply(app, server, config, secrets, pty_manager).await
}
```

- [ ] **Step 3: Build**

Run: `cd src-tauri && cargo build 2>&1 | tail -80`
Expected: The only remaining error should be in `lib.rs`'s auto-start block (Task 8). If you see anything else, stop and report BLOCKED.

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/commands/mcp_server.rs
git commit -m "feat(mcp-server): pass AppHandle and PtyManager through the mcp_tool_server_apply/set_config commands"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Confirmation the only remaining compile error is in `lib.rs` (expected, fixed next)
- Files changed
- Self-review findings

---

### Task 8: `lib.rs` — finish wiring, first real compile

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Update the MCP tool server auto-start block**

Find:

```rust
            // MCP tool server：設定為 enabled 時隨 app 啟動。失敗只記 log 不擋啟動，
            // 理由同橋接 server——埠被占用不該讓整個 app 起不來。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    let server = handle.state::<Arc<mcp_server::McpToolServerState>>().inner().clone();
                    let config = handle.state::<Arc<ConfigStore>>().inner().clone();
                    let secrets = handle.state::<Arc<SecretStore>>().inner().clone();
                    let cfg = config.get().mcp_tool_server;
                    if !cfg.enabled {
                        return;
                    }
                    let token = match secrets.get(mcp_server::MCP_TOOL_SERVER_TOKEN_KEY) {
                        Ok(Some(t)) if !t.is_empty() => t,
                        _ => {
                            let t = bridge::auth::generate_token();
                            if let Err(e) = secrets.set(mcp_server::MCP_TOOL_SERVER_TOKEN_KEY, &t) {
                                log::error!("mcp tool server token 寫入 keychain 失敗：{e}");
                                return;
                            }
                            t
                        }
                    };
                    if let Err(e) = server.start(config, secrets, token, cfg.port).await {
                        log::error!("mcp tool server 啟動失敗：{e}");
                    }
                });
            }
```

Replace with:

```rust
            // MCP tool server：設定為 enabled 時隨 app 啟動。失敗只記 log 不擋啟動，
            // 理由同橋接 server——埠被占用不該讓整個 app 起不來。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    let server = handle.state::<Arc<mcp_server::McpToolServerState>>().inner().clone();
                    let config = handle.state::<Arc<ConfigStore>>().inner().clone();
                    let secrets = handle.state::<Arc<SecretStore>>().inner().clone();
                    let pty_manager = handle.state::<Arc<PtyManager>>().inner().clone();
                    let cfg = config.get().mcp_tool_server;
                    if !cfg.enabled {
                        return;
                    }
                    let token = match secrets.get(mcp_server::MCP_TOOL_SERVER_TOKEN_KEY) {
                        Ok(Some(t)) if !t.is_empty() => t,
                        _ => {
                            let t = bridge::auth::generate_token();
                            if let Err(e) = secrets.set(mcp_server::MCP_TOOL_SERVER_TOKEN_KEY, &t) {
                                log::error!("mcp tool server token 寫入 keychain 失敗：{e}");
                                return;
                            }
                            t
                        }
                    };
                    if let Err(e) = server.start(config, secrets, token, cfg.port, handle.clone(), pty_manager).await {
                        log::error!("mcp tool server 啟動失敗：{e}");
                    }
                });
            }
```

- [ ] **Step 2: Build**

Run: `cd src-tauri && cargo build 2>&1 | tail -100`
Expected: Compiles clean. This is the first point everything from Tasks 1-8 gets compiled together — if there's a real bug in any earlier task's code (not just "the next task hasn't happened yet"), it surfaces here. Fix based on the actual compiler error; if something looks like it needs a design reconsideration rather than a line-level fix, stop and report BLOCKED with the full error and your assessment.

- [ ] **Step 3: Run the full backend test suite**

Run: `cd src-tauri && cargo test --lib mcp_server:: 2>&1 | tail -40`
Expected: All `coordination_ops` tests (7) plus the existing `db_ops`/`kb_ops` tests (12) pass.

Run: `cd src-tauri && cargo test --lib pty:: 2>&1 | tail -40`
Expected: All PTY tests, including the 2 new bell-detection tests, pass.

Run: `cd src-tauri && cargo test --lib 2>&1 | tail -20`
Expected: Full crate regression check — no failures anywhere.

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/lib.rs
git commit -m "feat(mcp-server): wire PtyManager into the MCP tool server auto-start path"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Any compile errors encountered across Tasks 5-8 and how you fixed them (be specific)
- Full test results (exact commands + pass/fail counts)
- Files changed
- Self-review findings
- Any issues or concerns

---

### Task 9: Backend integration test

**Files:**
- Modify: `src-tauri/tests/mcp_tool_server.rs`

- [ ] **Step 1: Read the existing test file first**

Open `src-tauri/tests/mcp_tool_server.rs` and read it in full — this task extends `test_router()` (its helper that builds a real axum `Router` for testing) and adds new test functions alongside the existing ones. Do not remove or restructure the existing tests.

- [ ] **Step 2: Update `test_router()` to build and pass the new dependencies**

Find the `test_router()` function's signature and body (it currently calls `router(Arc::new("t0ken".to_string()), db_manager, config, secrets, sidecar, kb_pool.clone())`). Add the 3 new arguments `router()` now requires (per Task 6):

```rust
use std::sync::Arc;

use aiterm_lib::mcp_server::server::router;
use aiterm_lib::config::ConfigStore;
use aiterm_lib::db::db2_sidecar::Db2SidecarState;
use aiterm_lib::db::knowledge_base::create_notebook;
use aiterm_lib::db::manager::DbManager;
use aiterm_lib::mcp_server::coordination_ops::CoordinationRegistry;
use aiterm_lib::pty::manager::PtyManager;
use aiterm_lib::secret::SecretStore;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
```

(Keep whatever of the existing imports you find — this is the expected final set, adding `CoordinationRegistry` and `PtyManager`; do not remove `create_notebook` or anything else already imported and used by existing tests.)

In `test_router()`'s body, find where `router(...)` is called and add the 3 new arguments — construct a fresh `Arc::new(PtyManager::new())` and `Arc::new(CoordinationRegistry::new())`, and obtain an `AppHandle` the same way any other Tauri-app-less integration test in this codebase would (search this file and sibling test files for an existing pattern of constructing a bare `AppHandle` for testing without a real running app — if `tauri::test::mock_app()` or similar is already used anywhere in `src-tauri/tests/`, follow that pattern exactly; if nothing like that exists yet, this is a case to flag: report BLOCKED describing what you found, rather than inventing an unverified way to get a test `AppHandle`, since spawn_tab's own test coverage already exists at the `coordination_ops` unit-test level via `create_with_callback` and doesn't strictly require a real `AppHandle` here — the router-level integration test can instead just verify that `spawn_tab` is reachable and either succeeds or fails gracefully, without asserting on `AppHandle`-dependent event emission).

- [ ] **Step 3: Add tests for the new tools**

Add these test functions to the file (using whatever `post()`/session-setup helper the file already has, following the exact pattern of the existing `full_round_trip_lists_connections` test — initialize, notify, then call):

```rust
#[tokio::test]
async fn coordination_tools_are_listed_but_refuse_to_run_when_disabled() {
    let (app, _pool) = test_router().await;

    let init_resp = app.clone().oneshot(post(
        "/mcp", Some("t0ken"),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.0.1"}
            }
        }),
    )).await.unwrap();
    let session_id = init_resp.headers().get("mcp-session-id").unwrap().to_str().unwrap().to_string();

    let mut notify_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
    }));
    notify_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    app.clone().oneshot(notify_req).await.unwrap();

    // The test's ConfigStore defaults to coordination_enabled: false (nothing
    // in test_router() enables it), so this must come back as a tool-level
    // error, not a protocol error and not a silent no-op.
    let mut call_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "get_tab_status", "arguments": {"tab_id": "whatever"}}
    }));
    call_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    let call_resp = app.oneshot(call_req).await.unwrap();
    assert_eq!(call_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(call_resp.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("disabled"), "{text}");
}

#[tokio::test]
async fn send_input_rejects_a_tab_the_test_never_spawned() {
    let (app, _pool) = test_router().await;

    let init_resp = app.clone().oneshot(post(
        "/mcp", Some("t0ken"),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.0.1"}
            }
        }),
    )).await.unwrap();
    let session_id = init_resp.headers().get("mcp-session-id").unwrap().to_str().unwrap().to_string();

    let mut notify_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
    }));
    notify_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    app.clone().oneshot(notify_req).await.unwrap();

    let mut call_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "send_input", "arguments": {"tab_id": "nonexistent", "text": "hi"}}
    }));
    call_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    let call_resp = app.oneshot(call_req).await.unwrap();
    assert_eq!(call_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(call_resp.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    // Either the "coordination disabled" error or the "not spawned by
    // spawn_tab" error is an acceptable pass here — both are the correct
    // deny, and which one fires first depends on test_router()'s config
    // defaults (established in Step 2 above).
    assert!(text.contains("disabled") || text.contains("was not created by spawn_tab"), "{text}");
}
```

- [ ] **Step 4: Run**

Run: `cd src-tauri && cargo test --test mcp_tool_server 2>&1 | tail -40`
Expected: All tests (the 4 pre-existing ones plus these 2 new ones) pass.

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add tests/mcp_tool_server.rs
git commit -m "test(mcp-server): integration coverage for the disabled-by-default and unknown-tab-id coordination tool paths"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- How you obtained a test `AppHandle` (or, if you couldn't find an established pattern, your BLOCKED report on that)
- Test results
- Files changed
- Self-review findings

---

### Task 10: Frontend IPC — event payload type and listener helper

**Files:**
- Modify: `src/ipc/mcpToolServer.ts`

- [ ] **Step 1: Add the config field, event payload type, and listener helper**

Find:

```typescript
export interface McpToolServerConfig {
  enabled: boolean;
  port: number;
}
```

Replace with:

```typescript
export interface McpToolServerConfig {
  enabled: boolean;
  port: number;
  coordinationEnabled: boolean;
}
```

Wait — check `src-tauri/src/config/types.rs`'s `McpToolServerConfig` serde behavior before assuming a rename: that struct has no `#[serde(rename_all = "camelCase")]` (confirmed in Task 3 — it's a plain `pub struct` with per-field `#[serde(default...)]` only), so the JSON field name is exactly the Rust field name, snake_case: `coordination_enabled`, NOT `coordinationEnabled`. Use this instead:

```typescript
export interface McpToolServerConfig {
  enabled: boolean;
  port: number;
  coordination_enabled: boolean;
}
```

At the end of the file, add:

```typescript
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface CoordinationTabSpawnedPayload {
  session_id: string;
  command: string | null;
}

/** Fired when an MCP coordination tool call spawns a new terminal tab. */
export function onCoordinationTabSpawned(
  cb: (payload: CoordinationTabSpawnedPayload) => void,
): Promise<UnlistenFn> {
  return listen<CoordinationTabSpawnedPayload>("mcp-coordination-tab-spawned", (e) => cb(e.payload));
}
```

Move the `import { listen, type UnlistenFn } from "@tauri-apps/api/event";` line to the top of the file, alongside the existing `import { invoke } from "@tauri-apps/api/core";` — don't leave an import statement in the middle of the file.

- [ ] **Step 2: Type-check**

Run: `npx tsc -b`
Expected: Fails — `McpToolServerPage.test.tsx`'s `BASE_CONFIG` fixture and `McpToolServerPage.tsx` itself don't have `coordination_enabled` yet. This is expected; Tasks 11 and 13 fix it. Confirm the errors are specifically about the missing `coordination_enabled` field, nothing else.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/mcpToolServer.ts
git commit -m "feat(mcp-server): IPC types for coordination_enabled and the tab-spawned event (frontend still incomplete)"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Confirmation the only new `tsc -b` errors are the expected missing-field ones
- Files changed
- Self-review findings

---

### Task 11: `TerminalView.tsx` — adopt-an-existing-session mode + `onRunningChange`

**Files:**
- Modify: `src/components/TerminalView.tsx`

- [ ] **Step 1: Read the relevant sections first**

Read `src/components/TerminalView.tsx` around its prop type definition (search for `onSessionCreated?:`) and around the `createPty(...)` call (search for `const id = await createPty`) before editing — this task adds one new prop next to `onSessionCreated`, and branches the existing async setup block on it.

- [ ] **Step 2: Add the two new props**

Find:

```typescript
  onSessionCreated?: (sessionId: string) => void;
```

Replace with:

```typescript
  onSessionCreated?: (sessionId: string) => void;
  /** If set, this tab adopts an already-existing backend PTY session instead
   *  of creating a new one — used for tabs spawned by the MCP tool server's
   *  agent-coordination tools (spawn_tab), where the backend already has a
   *  live session before any TerminalView exists to render it. */
  externalSessionId?: string;
  /** Reports whether this tab's most recent command block is currently
   *  running (as opposed to completed/failed/no blocks yet). Used by
   *  TerminalApp to decide whether to steal focus for a newly agent-spawned
   *  tab — never switch away from a tab the user is actively watching run
   *  something. */
  onRunningChange?: (isRunning: boolean) => void;
```

Find the destructured props list:

```typescript
export function TerminalView({ isActive = true, onToggleSidebar, isSidebarOpen = true, onSessionCreated, initialCwd, initialMission, enterpriseTask, onAgentProgress, onMissionEnd, onSummaryUpdate, onCwdChange, onAttention, onClaudeDetected, claudeBridge, tabId, remoteOwner = null, onRemoteOwnerChange }: TerminalViewProps) {
```

Replace with:

```typescript
export function TerminalView({ isActive = true, onToggleSidebar, isSidebarOpen = true, onSessionCreated, externalSessionId, onRunningChange, initialCwd, initialMission, enterpriseTask, onAgentProgress, onMissionEnd, onSummaryUpdate, onCwdChange, onAttention, onClaudeDetected, claudeBridge, tabId, remoteOwner = null, onRemoteOwnerChange }: TerminalViewProps) {
```

- [ ] **Step 3: Branch the PTY setup on `externalSessionId`**

Find:

```typescript
    (async () => {
      try {
        const { rows, cols } = term;
        const lastCwd = initialCwd ?? localStorage.getItem("aiterm_last_cwd") ?? undefined;
        const id = await createPty({ rows, cols }, lastCwd, claudeBridge);
        sessionRef.current = id;
        // A real resize can land while createPty() was still in flight (no
        // session id yet to resize) — see pendingResizeRef's comment. Flush
        // it now that a session finally exists, same as the paste-guard
        // case does on Enter/timeout.
        if (pendingResizeRef.current) {
          const pending = pendingResizeRef.current;
          pendingResizeRef.current = null;
          resizePty(id, pending).catch(console.error);
        }
        setSessionId(id);
        onSessionCreated?.(id);
        setStatus(`connected (${id.slice(0, 8)}…)`);
```

Replace with:

```typescript
    (async () => {
      try {
        const { rows, cols } = term;
        let id: string;
        if (externalSessionId) {
          // Backend already has a live PTY session for us (spawned by an MCP
          // coordination tool) — adopt it instead of creating a new one.
          id = externalSessionId;
        } else {
          const lastCwd = initialCwd ?? localStorage.getItem("aiterm_last_cwd") ?? undefined;
          id = await createPty({ rows, cols }, lastCwd, claudeBridge);
        }
        sessionRef.current = id;
        // A real resize can land while createPty() was still in flight (no
        // session id yet to resize) — see pendingResizeRef's comment. Flush
        // it now that a session finally exists, same as the paste-guard
        // case does on Enter/timeout.
        if (pendingResizeRef.current) {
          const pending = pendingResizeRef.current;
          pendingResizeRef.current = null;
          resizePty(id, pending).catch(console.error);
        }
        setSessionId(id);
        onSessionCreated?.(id);
        setStatus(`connected (${id.slice(0, 8)}…)`);
```

- [ ] **Step 4: Report the running/idle state of the latest block**

Find the `useTerminalBlocks` hook call (search for `useTerminalBlocks(` in this file) to see its return shape — it returns `{ blocks, ... }` per `src-tauri`... no, `src/hooks/useTerminalBlocks.ts`'s `blocks: TerminalBlock[]`. Add a new `useEffect` right after wherever `useTerminalBlocks(...)` is destructured (find the line assigning `const { blocks, ... } = useTerminalBlocks(...)`), reporting the latest block's running state whenever `blocks` changes:

```typescript
  useEffect(() => {
    const latest = blocks[blocks.length - 1];
    onRunningChange?.(latest?.status === "running");
  }, [blocks, onRunningChange]);
```

(If `onRunningChange` is an inline arrow function from the parent on every render — as `onAttention` already is, per this file's own comment about bridging it to a ref — this `useEffect` will re-run on every parent render too, which is harmless here since the effect body is cheap and idempotent; unlike `onAttention`, this callback isn't threaded into a `useCallback`-memoized OSC handler registration, so the ref-bridging trick this file uses for `onAttention` isn't needed for this one.)

- [ ] **Step 5: Type-check**

Run: `npx tsc -b`
Expected: Errors should now be limited to `TerminalApp.tsx` not yet passing these new props and `McpToolServerPage.tsx`/its test not yet handling `coordination_enabled` (Tasks 12-13 fix these). Confirm no NEW errors originate from `TerminalView.tsx` itself.

- [ ] **Step 6: Run the existing TerminalView test suite for regressions**

Run: `npm run test -- TerminalView`
Expected: All existing tests pass — this task is additive (new optional props, new effect), nothing existing should break.

- [ ] **Step 7: Commit**

```bash
git add src/components/TerminalView.tsx
git commit -m "feat(terminal): TerminalView can adopt an externally-created PTY session and reports running state"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- `tsc -b` output (confirm errors are only in the expected not-yet-updated files)
- `npm run test -- TerminalView` output
- Files changed
- Self-review findings

---

### Task 12: `TabBar` — `spawnedByAgent` field and visual marker

**Files:**
- Modify: `src/components/TabBar/index.tsx`
- Modify: `src/components/TabBar/index.css`

- [ ] **Step 1: Add the field to the `Tab` interface**

Find:

```typescript
  /** 非 active 的終端機分頁發生了值得注意的事：在側邊欄圖示上顯示一個彩色點。
   *  只存在記憶體，不進 localStorage——重開 app 後這些事件已經沒有意義。 */
  attention?: AttentionKind;
```

Add right after it:

```typescript
  /** 這個分頁是不是由 MCP 協調工具的 spawn_tab 開出來的——只影響圖示顯示，
   *  不影響任何行為。只存在記憶體，不進 localStorage。 */
  spawnedByAgent?: boolean;
```

- [ ] **Step 2: Extend `getTabIcon`**

Find:

```typescript
function getTabIcon(type: TabType, claudeBridge?: Tab["claudeBridge"]): React.ReactNode {
```

Read the rest of this function's body (a `switch`/`if` chain keyed on `type`, with the `claudeBridge === "explicit" ? <RobotIcon size={18} /> : <TerminalIcon size={18} />` branch for the `"terminal"` case) before editing, then change its signature to also accept `spawnedByAgent`:

```typescript
function getTabIcon(type: TabType, claudeBridge?: Tab["claudeBridge"], spawnedByAgent?: boolean): React.ReactNode {
```

Find the `"terminal"` case specifically:

```typescript
    case "terminal": return claudeBridge === "explicit" ? <RobotIcon size={18} /> : <TerminalIcon size={18} />;
```

Replace with:

```typescript
    case "terminal":
      if (spawnedByAgent) return <RobotIcon size={18} className="tab-icon--agent-spawned" />;
      return claudeBridge === "explicit" ? <RobotIcon size={18} /> : <TerminalIcon size={18} />;
```

Find the call site:

```typescript
              {getTabIcon(tab.type, tab.claudeBridge)}
```

Replace with:

```typescript
              {getTabIcon(tab.type, tab.claudeBridge, tab.spawnedByAgent)}
```

- [ ] **Step 3: Add the CSS class**

In `src/components/TabBar/index.css`, add:

```css
.tab-icon--agent-spawned {
  color: #4ade80;
}
```

(Reuses the same green already established as this codebase's "actively working / on" color — see `McpToolServerPage.css`'s `.mcp-tool-server-dot--on` from the earlier MCP tool server feature — rather than introducing a new color for the same underlying "agent-related" concept.)

- [ ] **Step 4: Type-check and run existing TabBar tests**

Run: `npx tsc -b && npm run test -- TabBar`
Expected: Type-check errors remain only in the not-yet-updated files (Task 13); existing `TabBar` tests all pass — this is an additive, optional field.

- [ ] **Step 5: Commit**

```bash
git add src/components/TabBar/index.tsx src/components/TabBar/index.css
git commit -m "feat(tabs): distinct icon color for agent-spawned coordination tabs"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Test/type-check results
- Files changed
- Self-review findings

---

### Task 13: `TerminalApp.tsx` — wire the event, focus heuristic, and new props

**Files:**
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: Read the relevant sections first**

Before editing, locate and read: the `handleAttention` callback (search `const handleAttention = useCallback`), the `useEffect` that sets up `onEnterpriseSkillInstalled`/`onTaskReceived`/etc. listeners and its cleanup (search `unlistenSkill`), the `<TerminalView` JSX render call (search `onAttention={(kind)`), and the top-level `import` block.

- [ ] **Step 2: Import the new IPC helpers**

Find:

```typescript
import { bridgeStatus } from "../ipc/bridge";
```

Add right after it:

```typescript
import { onCoordinationTabSpawned } from "../ipc/mcpToolServer";
```

- [ ] **Step 3: Add a ref tracking each tab's last-known running state**

Find the `notifyCooldownRef` declaration:

```typescript
  const notifyCooldownRef = useRef<Map<string, number>>(new Map());
```

Add right after it:

```typescript
  // Updated by each TerminalView's onRunningChange. Read (not written) by the
  // mcp-coordination-tab-spawned handler below to decide whether switching
  // focus to a newly agent-spawned tab would interrupt the user.
  const tabRunningRef = useRef<Map<string, boolean>>(new Map());
```

- [ ] **Step 4: Add the coordination-tab-spawned listener**

Find the `useEffect` that sets up the enterprise/skill listeners (containing `onEnterpriseSkillInstalled`), specifically its cleanup return statement:

```typescript
    return () => {
      unlistenTaskReceived?.();
      unlistenTaskReady?.();
      unlistenSkill?.();
    };
  }, [selectTab]);
```

Replace with (adds one more `let`, one more listener registration inside the same effect body — find where `unlistenSkill` is declared with `let` near the top of this same effect and add a sibling declaration there, then add the registration call, then extend the cleanup):

```typescript
    return () => {
      unlistenTaskReceived?.();
      unlistenTaskReady?.();
      unlistenSkill?.();
      unlistenCoordination?.();
    };
  }, [selectTab]);
```

Find where `let unlistenSkill: (() => void) | null = null;` (or its equivalent — match whatever the existing `let unlisten...` declarations for this effect look like) is declared near the top of the same effect body, and add a sibling declaration right after it:

```typescript
    let unlistenCoordination: (() => void) | null = null;
```

Find where `onEnterpriseSkillInstalled((payload) => { ... }).then((fn) => { unlistenSkill = fn; });` is called, and add this registration right after it, inside the same effect:

```typescript
    onCoordinationTabSpawned((payload) => {
      const newId = crypto.randomUUID();
      const activeTab = tabsRef.current.find((tb) => tb.id === activeIdRef.current);
      const activeIsBusy = activeTab?.type === "terminal" && tabRunningRef.current.get(activeTab.id) === true;
      setTabs((prev) => [...prev, {
        id: newId,
        title: payload.command ? `Agent: ${payload.command}` : t.terminal_tab,
        type: "terminal",
        ptySessionId: payload.session_id,
        spawnedByAgent: true,
      }]);
      if (!activeIsBusy) {
        selectTab(newId);
      }
    }).then((fn) => { unlistenCoordination = fn; });
```

This references `tabsRef` — check whether `TerminalApp.tsx` already keeps a ref mirror of the `tabs` state (search for `tabsRef`). If it does, use it as shown. If it does NOT exist yet, add one: find where `const [tabs, setTabs] = useState<Tab[]>(...)` is declared and add right after it:

```typescript
  const tabsRef = useRef(tabs);
  useEffect(() => { tabsRef.current = tabs; }, [tabs]);
```

(A ref mirror is necessary here, not `tabs` captured directly, for the same stale-closure reason `activeIdRef`/`windowFocusedRef` already exist in this file — the listener callback above is registered once when the effect runs and must read the *current* tabs list at event time, not whatever it was when the effect last ran.)

- [ ] **Step 5: Update `handleAttention` to also feed `tabRunningRef` is not needed** — `tabRunningRef` is populated by each `TerminalView`'s new `onRunningChange` prop (Task 11), not by `handleAttention`. Skip this step; it exists only to state explicitly that no change to `handleAttention` itself is needed, so you don't go looking for one.

- [ ] **Step 6: Pass the new props to `TerminalView`**

Find the `<TerminalView` render call's `onAttention` prop:

```typescript
                  onAttention={(kind) => handleAttention(tab.id, tab.title, kind)}
```

Add right after it:

```typescript
                  externalSessionId={tab.spawnedByAgent ? tab.ptySessionId : undefined}
                  onRunningChange={(isRunning) => { tabRunningRef.current.set(tab.id, isRunning); }}
```

- [ ] **Step 7: Type-check**

Run: `npx tsc -b`
Expected: Errors remaining should now be limited to `McpToolServerPage.tsx`/its test (Task 14). Confirm nothing new originates from `TerminalApp.tsx`.

- [ ] **Step 8: Run the existing TerminalApp test suite for regressions**

Run: `npm run test -- TerminalApp`
Expected: All pass.

- [ ] **Step 9: Commit**

```bash
git add src/components/TerminalApp.tsx
git commit -m "feat(terminal): render agent-spawned coordination tabs, switching focus only when the active tab is idle"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Whether `tabsRef` already existed or you added it
- `tsc -b` and test output
- Files changed
- Self-review findings

---

### Task 14: Settings page — `coordination_enabled` checkbox

**Files:**
- Modify: `src/components/Settings/McpToolServerPage.tsx`
- Modify: `src/components/Settings/McpToolServerPage.test.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Add the checkbox**

In `src/components/Settings/McpToolServerPage.tsx`, find:

```tsx
        <label className="mcp-tool-server-row">
          {t.mcp_tool_server_port}
          <input
            type="number"
            value={cfg.port}
            onChange={(e) => updateCfg({ ...cfg, port: Number(e.target.value) || 8318 })}
          />
        </label>
      </section>
```

Replace with:

```tsx
        <label className="mcp-tool-server-row">
          {t.mcp_tool_server_port}
          <input
            type="number"
            value={cfg.port}
            onChange={(e) => updateCfg({ ...cfg, port: Number(e.target.value) || 8318 })}
          />
        </label>

        <label className="mcp-tool-server-row">
          <input
            type="checkbox"
            checked={cfg.coordination_enabled}
            onChange={(e) => updateCfg({ ...cfg, coordination_enabled: e.target.checked })}
          />
          {t.mcp_tool_server_coordination_enable}
        </label>
        <p className="mcp-tool-server-desc">{t.mcp_tool_server_coordination_desc}</p>
      </section>
```

- [ ] **Step 2: Update the test fixtures**

In `src/components/Settings/McpToolServerPage.test.tsx`, find:

```typescript
  claude_bridge: { enabled: false, port: 8317, default_on_new_tab: false, opus: null, sonnet: null, haiku: null },
  mcp_tool_server: { enabled: false, port: 8318 },
```

Replace with:

```typescript
  claude_bridge: { enabled: false, port: 8317, default_on_new_tab: false, opus: null, sonnet: null, haiku: null },
  mcp_tool_server: { enabled: false, port: 8318, coordination_enabled: false },
```

Find both status fixtures:

```typescript
const STOPPED_STATUS: McpToolServerStatus = { running: false, port: null, token: null, error: null };
const RUNNING_STATUS: McpToolServerStatus = { running: true, port: 8318, token: "abc123", error: null };
```

These don't need changes — `McpToolServerStatus` has no `coordination_enabled` field (that's config, not status).

Also check `ClaudeBridgePage.test.tsx`'s `BASE_CONFIG` fixture (from the original MCP tool server feature) for the same `mcp_tool_server: { enabled: false, port: 8318 }` shape — update it the same way if found:

Run: `grep -rl "mcp_tool_server:" src/` to find every fixture, and add `coordination_enabled: false` to each one found beyond the two already shown above.

- [ ] **Step 3: Add a test for the new checkbox**

Add to `McpToolServerPage.test.tsx`:

```typescript
  it("enabling coordination tools calls mcpToolServerSetConfig with coordination_enabled: true", async () => {
    const user = userEvent.setup();
    render(<McpToolServerPage />);
    await waitFor(() => screen.getAllByRole("checkbox"));

    const checkboxes = screen.getAllByRole("checkbox");
    // Second checkbox is the coordination toggle (first is the server enable toggle).
    await user.click(checkboxes[1]);
    await user.click(screen.getByRole("button", { name: /save|儲存/i }));

    await waitFor(() => {
      expect(mcpToolServerSetConfig).toHaveBeenCalledWith(
        expect.objectContaining({ coordination_enabled: true }),
      );
    });
  });
```

- [ ] **Step 4: Add i18n keys**

In `src/lib/i18n.ts`, find the zh-TW `mcp_tool_server_copied: "已複製",` line (added by the original MCP tool server feature) and add right after it:

```typescript
    mcp_tool_server_coordination_enable: "啟用 agent 協調工具",
    mcp_tool_server_coordination_desc: "讓 agent 可以開新分頁、跟另一個 agent 互相協調。分頁裡執行的指令不受限制，風險較高，預設關閉。",
```

Find the English `mcp_tool_server_copied: "Copied",` line and add right after it:

```typescript
    mcp_tool_server_coordination_enable: "Enable agent coordination tools",
    mcp_tool_server_coordination_desc: "Lets an agent open new tabs and coordinate with another agent inside them. Commands run in those tabs are unrestricted — higher risk, disabled by default.",
```

- [ ] **Step 5: Verify**

Run: `npx tsc -b && npm run test -- McpToolServerPage ClaudeBridgePage`
Expected: Both pass, zero type errors anywhere in the project now.

- [ ] **Step 6: Commit**

```bash
git add src/components/Settings/McpToolServerPage.tsx src/components/Settings/McpToolServerPage.test.tsx src/lib/i18n.ts
git add -u  # picks up any other fixture files found in Step 2
git commit -m "feat(mcp-server): settings checkbox for the agent-coordination tools"
```

## Report Format

Report:
- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Which fixture files needed `coordination_enabled` added, beyond the two shown
- `tsc -b` / test output
- Files changed
- Self-review findings

---

### Task 15: Full verification pass

**Files:** none (verification only)

- [ ] **Step 1: Full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: All pass, including every test from Tasks 2, 4, 8, 9.

- [ ] **Step 2: Full frontend suite**

Run: `npm run lint && npx tsc -b && npm run test`
Expected: All pass. For `lint`, confirm any pre-existing unrelated lint debt (if the project has any at this point) is unchanged by this feature's diff, the same way earlier features on this codebase have verified it (via `git stash`/checkout comparison of the problem count) — don't just eyeball the total number.

- [ ] **Step 3: Manual end-to-end verification**

This needs a human at a real AITerm window with Claude Code CLI (or Codex CLI) installed — it can't be scripted.

1. Run `npm run tauri:dev`.
2. Settings → MCP Tool Server: enable the server AND the new "Enable agent coordination tools" checkbox, save.
3. Register with Claude Code (copy the `claude mcp add` command shown, run it in a terminal).
4. Open a terminal tab, start `claude`, and ask it to use `spawn_tab` to open a new tab running `claude` with an initial prompt (e.g. "spawn a new tab running claude and ask it to write a haiku, then wait for its reply and tell me what it said").
5. Confirm: a new tab genuinely appears in AITerm's tab bar with the distinct agent-spawned icon color; if the tab you're watching is idle, focus switches to the new tab — if you're actively watching a busy tab, it doesn't steal focus.
6. Confirm the coordinating agent's `wait_for_idle` call actually returns the second agent's real reply (not a timeout, not a hallucinated answer) — cross-check the shown output against what's genuinely visible in the spawned tab.
7. Toggle "Enable agent coordination tools" off, confirm a subsequent `spawn_tab` call from Claude Code now gets a clear "disabled" error instead of silently doing nothing or crashing.

- [ ] **Step 4: Report results to the user**

Summarize which of Steps 1-3 passed, and paste the exact output of any manual step that didn't behave as expected — do not report this task as done without having actually run the manual verification, or state clearly that it was skipped and why.
