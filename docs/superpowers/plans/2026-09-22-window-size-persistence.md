# Window Size/Position Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remember the AITerm window's size, position, and maximized state across restarts, on macOS, Windows, and Linux.

**Architecture:** Register the official `tauri-plugin-window-state` (v2) in the existing plugin chain in `src-tauri/src/lib.rs`. The plugin owns all the event-listening (Resized/Moved), in-memory caching, disk persistence (on `RunEvent::Exit`), and restore-on-launch logic — no app-specific Rust or frontend code is needed. See `docs/superpowers/specs/2026-09-22-window-size-persistence-design.md` for the full design and why `RunEvent::Exit` is safe with this app's existing window-close-confirm flow.

**Tech Stack:** Rust, Tauri 2, `tauri-plugin-window-state` crate.

---

### Task 1: Add the plugin dependency

**Files:**
- Modify: `src-tauri/Cargo.toml:33` (right after the `tauri-plugin-single-instance` line, alongside the other `tauri-plugin-*` deps)

- [ ] **Step 1: Add the dependency line**

In `src-tauri/Cargo.toml`, find this block:

```toml
tauri = { version = "2.10.3", features = [] }
tauri-plugin-log = "2"
tauri-plugin-dialog = "2"
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
tauri-plugin-single-instance = "2"
```

Change it to:

```toml
tauri = { version = "2.10.3", features = [] }
tauri-plugin-log = "2"
tauri-plugin-dialog = "2"
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
tauri-plugin-single-instance = "2"
tauri-plugin-window-state = "2"
```

- [ ] **Step 2: Verify the crate resolves**

Run: `cd src-tauri && cargo metadata --format-version 1 --no-deps > /dev/null && cargo update -p tauri-plugin-window-state --dry-run`

Expected: no error (confirms the version exists on crates.io and Cargo can resolve it). If this fails because the workspace hasn't fetched the index yet, run `cargo check -p aiterm-core` first (harmless, warms the registry cache) and retry.

- [ ] **Step 3: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore: add tauri-plugin-window-state dependency

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Register the plugin

**Files:**
- Modify: `src-tauri/src/lib.rs:242-246`

- [ ] **Step 1: Register it in the plugin chain**

In `src-tauri/src/lib.rs`, find this block (inside the function that builds `tauri::Builder`):

```rust
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
```

Change it to:

```rust
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        // 記住視窗大小/位置/是否最大化，下次啟動自動還原。存檔時機是
        // RunEvent::Exit（跟下面 mail::poller::stop_all 那個收尾用的是
        // 同一個事件），跟 v1.32.0 的關閉確認流程（quit.rs）天生相容：
        // 使用者在確認對話框按取消時 App 不會結束，Exit 不會觸發，狀態
        // 也就不會被提早存下去。見 docs/superpowers/specs/
        // 2026-09-22-window-size-persistence-design.md。
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
```

- [ ] **Step 2: Build check**

Run: `cd src-tauri && cargo check -p app`

Expected: compiles cleanly, no errors. (Per `CLAUDE.md`, `uv`/DB2 binaries must already be set up from a prior `tauri:dev` or setup-script run for this to succeed — if it fails with a missing resource-path error unrelated to this change, that's the pre-existing `externalBin` gotcha documented there, not this task.)

- [ ] **Step 3: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src-tauri/src/lib.rs
git commit -m "feat(window): remember size/position/maximized state across restarts

Registers tauri-plugin-window-state. No app-specific logic needed —
the plugin owns event listening, disk persistence on RunEvent::Exit,
and restore-on-launch. Falls back to tauri.conf.json's 800x600 when
there's no saved state yet (fresh install).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: Real-device verification

There is no app-specific logic to unit test here — the behavior lives entirely inside the third-party plugin. Verification is manual, on a real build, per the spec's test section. Do this on **macOS at minimum**; Windows and Linux ideally too, but note explicitly in the final report which platforms were actually checked and which weren't (per this project's changelog convention — see `feedback_changelog_verify_ui_claims` in memory: never claim a platform behavior that wasn't actually observed).

**Files:**
- None (manual verification only)

- [ ] **Step 1: Build and launch**

Run: `npm run tauri:dev` (or `env -u APPLE_SIGNING_IDENTITY npm run tauri:dev` on macOS per this project's dev-verification convention)

- [ ] **Step 2: Resize/move/maximize, then quit and relaunch**

1. Resize the window to a distinctly different size than 800×600 (e.g. drag to roughly 1200×800), and move it to a different screen position.
2. Quit the app fully (red button on macOS, or the window close button; on macOS also separately test Cmd+Q since it's the custom-menu-item quit path called out in the spec).
3. Relaunch (`npm run tauri:dev` again, or reopen the built app).
4. Confirm: the window opens at the same size and position as step 1, not the 800×600 default.

- [ ] **Step 3: Maximize, quit, relaunch**

1. Maximize the window.
2. Quit fully.
3. Relaunch.
4. Confirm: the window opens maximized.

- [ ] **Step 4: Fresh-install fallback**

1. Find and delete the plugin's saved-state file (on macOS this is under `~/Library/Application Support/com.aiterm.app/` — locate the exact filename by checking what the plugin created after step 2; it's typically `.window-state.json` in the app's config dir).
2. Relaunch.
3. Confirm: window opens at the `tauri.conf.json` default (800×600), same as before this change — no crash, no blank/off-screen window.

- [ ] **Step 5: Close-confirm interaction**

1. Start a long-running command in a terminal tab (so the window-close-confirm dialog from v1.32.0 will trigger).
2. Resize the window to a new distinct size.
3. Try to close the window; when the "work in progress" confirm dialog appears, click **Cancel (繼續執行)**.
4. Confirm the app is still running and the window is still open (i.e. cancel actually cancelled).
5. Now actually close it (click close again, choose "關閉並中止" / confirm), then relaunch.
6. Confirm: the size from step 2 was saved (proves the cancel didn't corrupt state, and the eventual real exit did save it).

- [ ] **Step 6: Record verification status**

Add a short note to the spec file (`docs/superpowers/specs/2026-09-22-window-size-persistence-design.md`) under a new `## 驗證狀態` section stating exactly which of steps 2–5 were actually run and on which platform(s), following this project's established convention of only writing changelog/spec claims for behavior actually observed (see `feedback_changelog_verify_ui_claims` in memory). Then commit:

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add -f docs/superpowers/specs/2026-09-22-window-size-persistence-design.md
git commit -m "docs: record real-device verification status for window state persistence

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: Changelog entry

**Files:**
- Modify: `CHANGELOG.md` (top of file, per this project's convention documented in the file's own header)

- [ ] **Step 1: Add the entry**

Read the current top of `CHANGELOG.md` first to get the exact current top-of-file version heading (the plan can't hardcode it — it changes as versions ship). Insert a new `## v<next-version>` section above it, written for end users, in the same style as the existing entries (see e.g. the `v1.33.1`/`v1.33.2` entries for tone and the "注意事項" convention for caveats about what wasn't verified). Content should cover:

- What changed: the window now remembers its size, position, and whether it was maximized, and restores that the next time AITerm opens.
- Caveat: only list the platforms actually confirmed in Task 3 as tested; for any platform not tested, say so plainly (matching this project's established convention — do not claim untested platform behavior).

- [ ] **Step 2: Ask the user which version number to use**

This project's convention (see memory: `feedback_no_auto_tag`) is to never push a version tag without asking first. Before finalizing the changelog heading, ask the user whether this ships as its own patch/minor version or gets bundled with other pending work, and what number to use.

- [ ] **Step 3: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add CHANGELOG.md
git commit -m "docs: changelog for v<version> (remember window size/position across restarts)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

Do **not** create or push a git tag as part of this task — ask the user first, per this project's established convention.
