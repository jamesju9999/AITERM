# Agent 協調工具完成加速訊號 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 `send_input` 可以選用附加一段固定措辭的指示，讓被協調的 agent 完成任務時主動印出一個含 `tab_id` 的標記字串，使 `wait_for_idle` 能比純等 terminal bell 更快回報 idle，同時保留 bell 作為永遠可靠的 fallback。

**Architecture:** `PtySession` 的讀取迴圈比照既有 `bell_count` 的做法，額外用一個小的跨 chunk 尾巴緩衝掃描自己 `tab_id` 專屬的標記字串，累加成獨立的 `marker_count`。`CoordinationRegistry` 的 baseline 從單一 `u64` 擴充成 `(bell, marker)` 一對，`status_for` 判斷 idle 時任一訊號超過各自 baseline 就算數。`SendInputArgs` 新增選用的 `request_done_marker` 布林參數，開啟時 `send_input` 額外送出一次獨立的、`\r` 結尾的指示訊息。

**Tech Stack:** Rust, Tauri 2, `rmcp`（MCP tool server）, `portable-pty`, `tokio`

**Spec:** `docs/superpowers/specs/2026-08-21-coordination-done-marker-design.md`

---

## Task 1: 標記偵測的純函式（`pty/session.rs`）

**Files:**
- Modify: `src-tauri/src/pty/session.rs:39-41`（在 `home_dir()` 結束、`pub struct PtySession` 開始之間插入新函式）
- Test: `src-tauri/src/pty/session.rs`（`#[cfg(test)] mod tests` 區塊，檔案第 663 行起）

這個任務只加三個不需要真正 PTY 的純函式，用單元測試直接驗證邏輯，不牽涉 struct 欄位或讀取迴圈（那是 Task 2）。

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/pty/session.rs` 的 `mod tests` 區塊最後（`bell_byte_in_output_increments_bell_count` 之後、`shells_own_osc133_prompt_markers_do_not_count_as_bells` 之前皆可，或直接接在檔案最後一個測試後面）加入：

```rust
    #[test]
    fn done_marker_embeds_the_tab_id_between_fixed_delimiters() {
        assert_eq!(done_marker("abc-123"), "<<AITERM_DONE:abc-123>>");
    }

    #[test]
    fn contains_marker_true_when_marker_is_wholly_within_one_chunk() {
        let marker = b"<<AITERM_DONE:abc>>";
        assert!(contains_marker(b"", b"hello <<AITERM_DONE:abc>> world", marker));
    }

    #[test]
    fn contains_marker_false_when_marker_absent() {
        let marker = b"<<AITERM_DONE:abc>>";
        assert!(!contains_marker(b"", b"nothing to see here", marker));
    }

    #[test]
    fn contains_marker_finds_a_marker_split_across_the_tail_and_the_new_chunk() {
        // Regression coverage for the cross-chunk correctness gap the design
        // doc flags: bell detection is a single byte and can never straddle
        // a chunk boundary, but this marker is 20 bytes long here (52 in
        // production with a real UUID) and PTY reads are arbitrary-sized.
        let marker = b"<<AITERM_DONE:abc>>";
        let (first, second) = marker.split_at(marker.len() / 2);

        // Round 1: only the first half has arrived — not present yet.
        assert!(!contains_marker(b"", first, marker));
        let tail = marker_tail_after(first, marker.len());

        // Round 2: second half arrives — tail + this chunk together contain it.
        assert!(contains_marker(&tail, second, marker));
    }

    #[test]
    fn marker_tail_after_keeps_only_the_last_marker_len_minus_one_bytes() {
        let marker_len = 5;
        let chunk = b"abcdefgh";
        assert_eq!(marker_tail_after(chunk, marker_len), b"efgh".to_vec());
    }

    #[test]
    fn marker_tail_after_keeps_the_whole_chunk_when_shorter_than_the_window() {
        let marker_len = 20;
        let chunk = b"ab";
        assert_eq!(marker_tail_after(chunk, marker_len), b"ab".to_vec());
    }
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib pty::session::tests::done_marker_embeds_the_tab_id_between_fixed_delimiters`
Expected: 編譯失敗，`error[E0425]: cannot find function `done_marker` in this scope`（因為 `done_marker`/`contains_marker`/`marker_tail_after` 都還不存在）

- [ ] **Step 3: 寫最小實作**

在 `src-tauri/src/pty/session.rs` 第 39-41 行之間（`home_dir()` 的結尾 `}` 之後、`pub struct PtySession {` 之前）插入：

```rust
/// Prefix/suffix of the optional completion marker a cooperative agent can
/// print to let the MCP coordination tools' `wait_for_idle` return faster
/// than the mandatory bell fallback. See
/// `docs/superpowers/specs/2026-08-21-coordination-done-marker-design.md`.
const DONE_MARKER_PREFIX: &str = "<<AITERM_DONE:";
const DONE_MARKER_SUFFIX: &str = ">>";

/// Builds this session's own completion marker text. `tab_id` is always a
/// UUID (see `Uuid::new_v4()` at the call site), so the marker is a fixed
/// length in practice, but this function itself makes no assumption about
/// that — any `tab_id` string works.
pub fn done_marker(tab_id: &str) -> String {
    format!("{DONE_MARKER_PREFIX}{tab_id}{DONE_MARKER_SUFFIX}")
}

/// Scans `tail` immediately followed by `chunk` for `marker`. `tail` should
/// be the previous chunk's own trailing `marker.len() - 1` bytes (or empty
/// for the very first chunk), so a marker split across a chunk boundary is
/// still found. Only ever looks at these newly-arrived bytes — never
/// rescans older history — so a stale marker from a previous round can
/// never re-trigger a later scan.
fn contains_marker(tail: &[u8], chunk: &[u8], marker: &[u8]) -> bool {
    if marker.is_empty() {
        return false;
    }
    let mut combined = Vec::with_capacity(tail.len() + chunk.len());
    combined.extend_from_slice(tail);
    combined.extend_from_slice(chunk);
    combined.windows(marker.len()).any(|w| w == marker)
}

/// Computes the new tail to carry into the next `contains_marker` call: the
/// last `marker_len - 1` bytes of `chunk` (or all of `chunk` if it's
/// shorter than that window).
fn marker_tail_after(chunk: &[u8], marker_len: usize) -> Vec<u8> {
    let keep = marker_len.saturating_sub(1).min(chunk.len());
    chunk[chunk.len() - keep..].to_vec()
}

```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib pty::session::tests:: -- done_marker contains_marker marker_tail_after`
Expected: 6 個測試全部 `ok`（`done_marker_embeds_the_tab_id_between_fixed_delimiters`、`contains_marker_true_when_marker_is_wholly_within_one_chunk`、`contains_marker_false_when_marker_absent`、`contains_marker_finds_a_marker_split_across_the_tail_and_the_new_chunk`、`marker_tail_after_keeps_only_the_last_marker_len_minus_one_bytes`、`marker_tail_after_keeps_the_whole_chunk_when_shorter_than_the_window`）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/pty/session.rs
git commit -m "feat(pty): add pure helpers for the coordination done-marker"
```

---

## Task 2: 把標記偵測接進 `PtySession` 的讀取迴圈

**Files:**
- Modify: `src-tauri/src/pty/session.rs:41-72`（struct 欄位）
- Modify: `src-tauri/src/pty/session.rs:213-343`（`spawn()`）
- Modify: `src-tauri/src/pty/session.rs:345-478`（`spawn_with_id()`）
- Modify: `src-tauri/src/pty/session.rs:630-636`（新增 `marker_count()` getter，緊接在 `bell_count()` 之後）
- Test: `src-tauri/src/pty/session.rs`（`mod tests`）

- [ ] **Step 1: 寫失敗的測試（即時 PTY，比照既有 bell 測試）**

在 `mod tests` 區塊，`bell_byte_in_output_increments_bell_count` 測試之後加入：

```rust
    #[test]
    fn marker_count_starts_at_zero_for_a_fresh_session() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");
        assert_eq!(session.marker_count(), 0);
    }

    #[tokio::test]
    async fn marker_in_output_increments_marker_count() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            move |chunk| {
                let _ = tx.send(chunk);
            },
        )
        .expect("spawn pty");

        assert_eq!(session.marker_count(), 0);

        let marker = done_marker(&session.id);
        #[cfg(windows)]
        session.write(format!("echo {marker}\r\nexit\r\n").as_bytes()).unwrap();
        #[cfg(not(windows))]
        session.write(format!("printf '%s\\n' '{marker}'\n").as_bytes()).unwrap();

        let mut seen = false;
        for _ in 0..50 {
            if session.marker_count() > 0 {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            seen,
            "expected marker_count() to increment after the session's own marker was written and echoed"
        );

        let _ = rx.try_recv();
        drop(session);
    }

    #[tokio::test]
    async fn a_marker_for_a_different_tab_id_does_not_count() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            move |chunk| {
                let _ = tx.send(chunk);
            },
        )
        .expect("spawn pty");

        let other_marker = done_marker("not-this-session-id");
        #[cfg(windows)]
        session.write(format!("echo {other_marker}\r\nexit\r\n").as_bytes()).unwrap();
        #[cfg(not(windows))]
        session.write(format!("printf '%s\\n' '{other_marker}'\n").as_bytes()).unwrap();

        // Give the reader thread time to process it, then confirm it never counted.
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(
            session.marker_count(),
            0,
            "a marker addressed to a different tab_id must not count for this session"
        );

        let _ = rx.try_recv();
        drop(session);
    }
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib pty::session::tests::marker_count_starts_at_zero_for_a_fresh_session`
Expected: 編譯失敗，`error[E0599]: no method named `marker_count` found for struct `PtySession``

- [ ] **Step 3: 加 struct 欄位**

把 `src-tauri/src/pty/session.rs:41-72` 的 `pub struct PtySession { ... }` 從：

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
    /// Counts how many output chunks have contained at least one bell
    /// byte (`0x07`). Used by the MCP tool server's agent-coordination tools as an
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

改成（在 `bell_count` 欄位後加兩個新欄位）：

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
    /// Counts how many output chunks have contained at least one bell
    /// byte (`0x07`). Used by the MCP tool server's agent-coordination tools as an
    /// idle signal: both Claude Code and Codex CLI fall back to a plain
    /// terminal bell for "waiting for input" notifications when they can't
    /// detect a richer-notification-capable terminal (verified against both
    /// projects' source — see the design doc). A monotonic counter rather
    /// than a boolean so a caller can detect "a *new* bell happened since I
    /// last checked" by comparing against a remembered baseline, without this
    /// field needing any consuming/resetting behavior of its own.
    bell_count: Arc<AtomicU64>,
    /// Counts how many times this session's own completion marker
    /// (`done_marker(&self.id)`) has been observed in output — an optional,
    /// cooperative-agent-only signal that lets the MCP coordination tools'
    /// `wait_for_idle` return faster than the mandatory bell fallback. Same
    /// monotonic-counter reasoning as `bell_count`. See
    /// `docs/superpowers/specs/2026-08-21-coordination-done-marker-design.md`.
    marker_count: Arc<AtomicU64>,
    /// Tail of the previous read chunk (at most `marker.len() - 1` bytes),
    /// carried forward so a marker split across two chunk boundaries is
    /// still detected. Reset on every read to that read's own chunk tail.
    marker_tail: Arc<Mutex<Vec<u8>>>,
}
```

- [ ] **Step 4: 在 `spawn()` 裡接線**

在 `src-tauri/src/pty/session.rs` 的 `spawn()` 方法裡，把：

```rust
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
                            // never the exact number of bells. Uses
                            // `contains_bare_bell` (not a naive `contains(&0x07)`)
                            // because our own OSC133 shell-integration markers are
                            // themselves BEL-terminated and must not be mistaken
                            // for a genuine agent notification bell.
                            if cd_parser::contains_bare_bell(&chunk) {
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

改成：

```rust
        let bell_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let bell_count_for_thread = Arc::clone(&bell_count);
        let marker_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let marker_count_for_thread = Arc::clone(&marker_count);
        let marker_tail: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let marker_tail_for_thread = Arc::clone(&marker_tail);
        let done_marker_bytes = done_marker(&id).into_bytes();

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
                            // never the exact number of bells. Uses
                            // `contains_bare_bell` (not a naive `contains(&0x07)`)
                            // because our own OSC133 shell-integration markers are
                            // themselves BEL-terminated and must not be mistaken
                            // for a genuine agent notification bell.
                            if cd_parser::contains_bare_bell(&chunk) {
                                bell_count_for_thread.fetch_add(1, Ordering::SeqCst);
                            }
                            // Optional completion-marker detection (see design doc
                            // 2026-08-21-coordination-done-marker-design.md). Uses a
                            // small carried-over tail so a marker split across two
                            // chunk boundaries is still found — unlike the single-byte
                            // bell above, this marker is multiple bytes long and PTY
                            // reads are arbitrary-sized.
                            {
                                let mut tail = marker_tail_for_thread.lock();
                                if contains_marker(&tail, &chunk, &done_marker_bytes) {
                                    marker_count_for_thread.fetch_add(1, Ordering::SeqCst);
                                }
                                *tail = marker_tail_after(&chunk, done_marker_bytes.len());
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
            marker_count,
            marker_tail,
        })
    }
```

- [ ] **Step 5: 在 `spawn_with_id()` 裡做完全一樣的接線**

`spawn_with_id()`（`src-tauri/src/pty/session.rs:345-478`）跟 `spawn()` 的讀取迴圈是逐字重複的一份（既有的程式碼重複，這次不重構，維持現狀，同步複製）。對 `spawn_with_id()` 套用跟 Step 4 完全相同的改法——`bell_count`/`bell_count_for_thread` 那兩行之後插入同樣的 `marker_count`/`marker_tail`/`done_marker_bytes` 宣告，讀取迴圈裡 `contains_bare_bell` 判斷式之後插入同樣的標記偵測區塊，`Ok(Self { ... })` 裡加上 `marker_count, marker_tail,`。

`spawn_with_id()` 跟 `spawn()` 的差異只在於 `id` 是參數傳入、不是 `Uuid::new_v4()` 產生——`done_marker(&id)` 這行不變，因為兩個函式的 `id` 變數都已經在讀取迴圈閉包建構之前確定。

- [ ] **Step 6: 加 `marker_count()` getter**

在 `src-tauri/src/pty/session.rs:630-636`，緊接在 `bell_count()` 方法後面加入：

```rust
    /// Monotonic count of times this session's own completion marker has
    /// been observed in output since it started. See the field doc comment
    /// on `marker_count` for why this is a counter, not a boolean.
    pub fn marker_count(&self) -> u64 {
        self.marker_count.load(Ordering::SeqCst)
    }
```

- [ ] **Step 7: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib pty::session::tests:: -- marker_count marker_in_output a_marker_for_a_different`
Expected: `marker_count_starts_at_zero_for_a_fresh_session`、`marker_in_output_increments_marker_count`、`a_marker_for_a_different_tab_id_does_not_count` 全部 `ok`。也順手確認沒有破壞既有 bell 測試：

Run: `cd src-tauri && cargo test --lib pty::session::tests::`
Expected: 全部通過，包含既有的 `bell_count_starts_at_zero_for_a_fresh_session`、`bell_byte_in_output_increments_bell_count`、`shells_own_osc133_prompt_markers_do_not_count_as_bells`

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/pty/session.rs
git commit -m "feat(pty): detect the coordination done-marker across chunk boundaries"
```

---

## Task 3: `PtyManager::marker_count` 包裝方法

**Files:**
- Modify: `src-tauri/src/pty/manager.rs:108-112`（緊接在 `bell_count` 之後）
- Test: `src-tauri/src/pty/manager.rs`（`mod tests`）

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/pty/manager.rs` 的 `mod tests` 區塊最後加入：

```rust
    #[test]
    fn manager_marker_count_returns_none_for_missing() {
        let manager = PtyManager::new();
        assert!(manager.marker_count("no-such-id").is_none());
    }

    #[test]
    fn manager_marker_count_returns_zero_for_a_fresh_session() {
        let manager = PtyManager::new();
        let (tx, _rx) = mpsc::channel::<Vec<u8>>();
        let id = manager
            .create_with_callback(
                PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
                move |chunk| {
                    let _ = tx.send(chunk);
                },
            )
            .expect("create session");
        assert_eq!(manager.marker_count(&id), Some(0));
    }
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib pty::manager::tests::manager_marker_count_returns_none_for_missing`
Expected: 編譯失敗，`error[E0599]: no method named `marker_count` found for struct `PtyManager``

- [ ] **Step 3: 寫最小實作**

在 `src-tauri/src/pty/manager.rs:108-112`，緊接在 `bell_count` 方法後面加入：

```rust
    /// Marker-byte count for the given session, or `None` if the session
    /// doesn't exist. See `PtySession::marker_count` for what this counts.
    pub fn marker_count(&self, id: &str) -> Option<u64> {
        self.sessions.lock().get(id).map(|s| s.marker_count())
    }
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib pty::manager::tests::`
Expected: 全部通過，包含新增的 `manager_marker_count_returns_none_for_missing`、`manager_marker_count_returns_zero_for_a_fresh_session`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/pty/manager.rs
git commit -m "feat(pty): expose PtyManager::marker_count"
```

---

## Task 4: `CoordinationRegistry` 的 baseline 擴充成 (bell, marker) 一對

**Files:**
- Modify: `src-tauri/src/mcp_server/coordination_ops.rs:40-61`

這個任務只改 `CoordinationRegistry` 本身，呼叫端（`spawn_tab`/`send_input`/`status_for`）在 Task 5 一起改，因為改了呼叫端才能重新編譯過（`CoordinationRegistry` 目前沒有自己獨立的單元測試，都是透過 `send_input`/`get_tab_status` 間接測試——這個任務先讓型別對齊，Task 5 再補行為測試）。

- [ ] **Step 1: 修改 `CoordinationRegistry`**

把 `src-tauri/src/mcp_server/coordination_ops.rs:40-61` 的：

```rust
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
```

改成：

```rust
#[derive(Default)]
pub struct CoordinationRegistry {
    tabs: Mutex<HashMap<String, Baseline>>,
}

/// The bell/marker counts a tab's `PtySession` reported as of the last
/// `spawn_tab`/`send_input` call — the point every subsequent
/// `get_tab_status`/`wait_for_idle` compares fresh counts against. Paired
/// together (not two separate maps) because they're always read and
/// written for the same `tab_id` at the same moment.
#[derive(Clone, Copy, Default)]
struct Baseline {
    bell: u64,
    marker: u64,
}

impl CoordinationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_known(&self, tab_id: &str) -> bool {
        self.tabs.lock().contains_key(tab_id)
    }

    fn record_baseline(&self, tab_id: &str, bell: u64, marker: u64) {
        self.tabs.lock().insert(tab_id.to_string(), Baseline { bell, marker });
    }

    fn baseline(&self, tab_id: &str) -> Option<Baseline> {
        self.tabs.lock().get(tab_id).copied()
    }
}
```

- [ ] **Step 2: 確認編譯失敗（預期，因為呼叫端還沒改）**

Run: `cd src-tauri && cargo build --lib 2>&1 | head -40`
Expected: 編譯錯誤，`record_baseline` 呼叫端（`spawn_tab`/`send_input`，第 100 行與第 139 行）與 `baseline()` 呼叫端（`status_for`，第 150 行）的參數數量/型別對不上——這是預期中的失敗，Task 5 會修好。**這步不用改動任何東西，只是確認錯誤訊息符合預期後就往下走，不要 commit。**

---

## Task 5: `status_for`/`spawn_tab`/`send_input` 接上雙訊號，`signal` 欄位

**Files:**
- Modify: `src-tauri/src/mcp_server/coordination_ops.rs:63-74`（`TabStatus`/`WaitResult` struct）
- Modify: `src-tauri/src/mcp_server/coordination_ops.rs:84-194`（`spawn_tab`/`send_input`/`status_for`/`get_tab_status`/`wait_for_idle`）
- Modify: `src-tauri/src/mcp_server/coordination_ops.rs`（既有測試呼叫點）
- Test: `src-tauri/src/mcp_server/coordination_ops.rs`（`mod tests`）

- [ ] **Step 1: 改 `TabStatus`/`WaitResult`**

把：

```rust
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
```

改成：

```rust
#[derive(Serialize)]
struct TabStatus {
    idle: bool,
    recent_output: String,
    /// Which signal made `idle` true this time — `"bell"` or `"marker"`.
    /// `None` while still not idle. Purely informational (debugging/tests);
    /// callers that only care about `idle` can ignore it.
    signal: Option<&'static str>,
}

#[derive(Serialize)]
struct WaitResult {
    idle: bool,
    recent_output: String,
    timed_out: bool,
    signal: Option<&'static str>,
}
```

- [ ] **Step 2: 改 `spawn_tab`**

把：

```rust
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
```

改成：

```rust
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

    // Fresh session: bell_count()/marker_count() are 0 right now. Recording
    // that as the baseline means get_tab_status/wait_for_idle report "not
    // idle" until the first bell or marker — reasonable, since nothing has
    // run yet either way.
    registry.record_baseline(&tab_id, 0, 0);
```

（函式其餘部分——`command` 的處理與事件送出——不變。）

- [ ] **Step 3: 改 `send_input`**

**前置修正（複審階段發現並補上）**：`send_input` 的指示文字需要分別引用 `DONE_MARKER_PREFIX`/`DONE_MARKER_SUFFIX`（Task 1 定義在 `src-tauri/src/pty/session.rs`，目前是 `const`，模組私有）。先把這兩個常數的可見性從 `const` 改成 `pub const`（只改可見性，數值與位置都不變，不影響 Task 1 已核准的任何行為或測試）：

```rust
pub const DONE_MARKER_PREFIX: &str = "<<AITERM_DONE:";
pub const DONE_MARKER_SUFFIX: &str = ">>";
```

把：

```rust
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
        .write(tab_id, format!("{text}\r").as_bytes())
        .map_err(|e| e.to_string())?;

    // Reset the baseline to the count *as of right now*, before any reply has
    // had a chance to arrive — so idle only flips true again once a *new*
    // bell (a reply to this specific input) is observed, not a stale one from
    // before this send.
    let current = pty_manager.bell_count(tab_id).unwrap_or(0);
    registry.record_baseline(tab_id, current);

    Ok(format!("sent to {tab_id}"))
}
```

改成：

```rust
pub(crate) fn send_input(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
    text: &str,
    request_done_marker: bool,
) -> Result<String, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    pty_manager
        .write(tab_id, format!("{text}\r").as_bytes())
        .map_err(|e| e.to_string())?;

    // Sent as a second, independently \r-terminated write (a separate
    // message) rather than appended via an embedded '\n' — this file's own
    // send_input_terminates_the_line_with_cr_not_lf regression test exists
    // precisely because raw-mode TUIs (Claude Code's own included) have no
    // guaranteed behavior for a bare LF byte, only for CR. See the design
    // doc's "指示文字" section.
    //
    // The instruction text below deliberately never writes the complete,
    // contiguous marker string (prefix + tab_id + suffix back-to-back) —
    // each piece is separated by other text. If it did, canonical-mode
    // terminal echo alone (no cooperating agent needed) would write that
    // same contiguous byte sequence right back into this session's output
    // stream as a pure side effect of writing this instruction, and
    // marker_count would increment immediately — a false "done" signal
    // before the target has done anything. Verified live against a real
    // PTY during implementation: the original (buggy) wording, which did
    // embed the marker contiguously, triggered marker_count=1 from echo
    // alone, with no agent involved.
    if request_done_marker {
        let instruction = format!(
            "（可選：完成後請在新的一行印出一個完成標記，格式為三段直接相連、中間不留任何字元：前綴 {} ，接著是你的識別碼 {} ，最後接上 {} 。這能讓協調端提早得知你已完成，不影響任何其他行為。）",
            crate::pty::session::DONE_MARKER_PREFIX,
            tab_id,
            crate::pty::session::DONE_MARKER_SUFFIX
        );
        pty_manager
            .write(tab_id, format!("{instruction}\r").as_bytes())
            .map_err(|e| e.to_string())?;
    }

    // Reset the baseline to the counts *as of right now*, after both of this
    // call's own writes — before any reply has had a chance to arrive — so
    // idle only flips true again once a *new* bell or marker (a reply to
    // this specific input) is observed, not a stale one from before this
    // send.
    let bell_current = pty_manager.bell_count(tab_id).unwrap_or(0);
    let marker_current = pty_manager.marker_count(tab_id).unwrap_or(0);
    registry.record_baseline(tab_id, bell_current, marker_current);

    Ok(format!("sent to {tab_id}"))
}
```

- [ ] **Step 4: 改 `status_for`**

把：

```rust
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
```

改成：

```rust
fn status_for(pty_manager: &PtyManager, registry: &CoordinationRegistry, tab_id: &str) -> Result<TabStatus, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    let baseline = registry.baseline(tab_id).unwrap_or_default();
    let bell_current = pty_manager
        .bell_count(tab_id)
        .ok_or_else(|| format!("tab_id '{tab_id}' is no longer running (it may have been closed)"))?;
    let marker_current = pty_manager.marker_count(tab_id).unwrap_or(0);
    let recent_output = pty_manager
        .get_recent_output(tab_id, RECENT_OUTPUT_BYTES)
        .unwrap_or_default();

    // Marker checked first: it's the optional, cooperative-agent signal this
    // feature adds, and when it fires it's always at least as fresh as bell.
    // Which one "wins" on a tie has no behavioral consequence — idle is idle
    // either way — this only affects the informational `signal` field.
    let marker_idle = marker_current > baseline.marker;
    let bell_idle = bell_current > baseline.bell;
    let signal = if marker_idle {
        Some("marker")
    } else if bell_idle {
        Some("bell")
    } else {
        None
    };

    Ok(TabStatus { idle: marker_idle || bell_idle, recent_output, signal })
}
```

- [ ] **Step 5: 改 `get_tab_status`（帶 `signal` 一起序列化，函式體不用動，`TabStatus` 已經有欄位了）**

`get_tab_status` 呼叫 `status_for` 後直接 `serde_json::to_string_pretty(&status)`，`signal` 欄位會自動包含在輸出裡，這個函式本身不需要改動。

- [ ] **Step 6: 改 `wait_for_idle`**

把：

```rust
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
```

改成：

```rust
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
            let result = WaitResult {
                idle: true,
                recent_output: status.recent_output,
                timed_out: false,
                signal: status.signal,
            };
            return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
        }
        if tokio::time::Instant::now() >= deadline {
            let result = WaitResult {
                idle: false,
                recent_output: status.recent_output,
                timed_out: true,
                signal: status.signal,
            };
            return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}
```

- [ ] **Step 7: 更新既有測試呼叫點（不然編譯不過）**

`mod tests` 裡三處直接呼叫 `send_input(...)` 的地方，都要在最後加一個 `false`（維持既有測試的行為：不開加速訊號）：

把：
```rust
        let err = send_input(&pty_manager, &registry, "not-a-real-tab", "hello").unwrap_err();
```
改成：
```rust
        let err = send_input(&pty_manager, &registry, "not-a-real-tab", "hello", false).unwrap_err();
```

把：
```rust
        let sent = send_input(&pty_manager, &registry, &tab_id, "echo hi").unwrap();
```
改成：
```rust
        let sent = send_input(&pty_manager, &registry, &tab_id, "echo hi", false).unwrap();
```

把（`send_input_terminates_the_line_with_cr_not_lf` 測試裡）：
```rust
        send_input(&pty_manager, &registry, &tab_id, "ab").unwrap();
```
改成：
```rust
        send_input(&pty_manager, &registry, &tab_id, "ab", false).unwrap();
```

- [ ] **Step 8: 確認編譯與既有測試都過**

Run: `cd src-tauri && cargo test --lib mcp_server::coordination_ops::`
Expected: 全部通過，包含既有的 `send_input_rejects_a_tab_id_not_in_the_registry`、`get_tab_status_rejects_a_tab_id_not_in_the_registry`、`send_input_and_get_tab_status_work_on_a_spawned_session`、`wait_for_idle_times_out_when_no_bell_arrives`、`wait_for_idle_returns_idle_once_a_bell_is_observed`、`wait_for_idle_rejects_a_tab_id_not_in_the_registry`、`send_input_terminates_the_line_with_cr_not_lf`

- [ ] **Step 9: 寫新行為的失敗測試**

**這步的第一個測試在複審階段被替換過**：原始版本斷言「回顯的輸出包含完整標記字串」，但那正是自我觸發那個 bug 的斷言方式（拿 bug 的症狀當正確行為在測）。換成同時驗證「指示文字確實送達（含 tab_id）」且「完整連續標記不會出現在回顯裡、`marker_count` 維持在 baseline」——這才是真正鎖住修正結果的測試。

在 `mod tests` 區塊最後加入：

```rust
    #[tokio::test]
    async fn send_input_with_request_done_marker_sends_the_instruction_without_self_triggering() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        send_input(&pty_manager, &registry, &tab_id, "echo hi", true).unwrap();

        // Wait for the echoed instruction to actually arrive (it mentions
        // this tab's own id, proving the instruction was sent).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
            if output.contains(&tab_id) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "expected the echoed input to mention the tab_id, got: {output}"
            );
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }

        // But the full, contiguous marker must NEVER appear anywhere in what
        // was echoed — if it did, the instruction itself would have
        // self-triggered a false completion signal via terminal echo alone,
        // with no agent involved. This is a regression test for a real bug
        // found during review: the original wording embedded the complete
        // contiguous marker in the instruction, and canonical-mode echo
        // alone incremented marker_count to 1 against a plain shell that
        // did nothing.
        let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
        let full_marker = crate::pty::session::done_marker(&tab_id);
        assert!(
            !output.contains(&full_marker),
            "the instruction text must never contain the complete contiguous marker — got: {output}"
        );
        assert_eq!(
            pty_manager.marker_count(&tab_id),
            Some(0),
            "writing the instruction alone must not increment marker_count — self-echo false positive"
        );
    }

    #[tokio::test]
    async fn wait_for_idle_returns_early_via_marker_signal_without_any_bell() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        let marker = crate::pty::session::done_marker(&tab_id);
        pty_manager.write(&tab_id, format!("printf '%s\\n' '{marker}'\n").as_bytes()).unwrap();

        let result_json = wait_for_idle(&pty_manager, &registry, &tab_id, Some(10)).await.unwrap();
        assert!(result_json.contains("\"idle\": true"), "{result_json}");
        assert!(result_json.contains("\"signal\": \"marker\""), "{result_json}");
    }
```

- [ ] **Step 10: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib mcp_server::coordination_ops::`
Expected: 全部通過，包含新增的 `send_input_with_request_done_marker_sends_the_instruction_without_self_triggering`、`wait_for_idle_returns_early_via_marker_signal_without_any_bell`

- [ ] **Step 11: Commit**

```bash
git add src-tauri/src/mcp_server/coordination_ops.rs
git commit -m "feat(mcp): dual bell/marker idle signal in agent coordination tools"
```

---

## Task 6: `SendInputArgs` 新增 `request_done_marker`，接上 `tools.rs`

**Files:**
- Modify: `src-tauri/src/mcp_server/tools.rs:105-111`（`SendInputArgs`）
- Modify: `src-tauri/src/mcp_server/tools.rs:253-262`（`send_input` 工具方法）
- Test: `src-tauri/tests/mcp_tool_server.rs`

- [ ] **Step 1: 寫失敗的整合測試**

**重要限制**：`test_router_with`（見檔案第 23-52 行）永遠把 `app: None` 傳給 `router(...)`（第 47 行 `None,`），代表這個 HTTP 層級的測試檔案裡 `spawn_tab` 永遠會走「沒有 AppHandle」的錯誤分支——這正是既有測試 `spawn_tab_without_an_app_handle_returns_a_clear_error` 在驗證的事。所以這裡沒辦法真的呼叫 `spawn_tab` 拿到一個可用的 `tab_id` 再測 `send_input`。`CoordinationRegistry::record_baseline` 也是模組私有（非 `pub`），這個外部整合測試檔案本來就呼叫不到，不能像 `coordination_ops.rs` 自己的單元測試那樣繞過 `spawn_tab` 直接塞一筆假資料。

因此這裡只驗證「新欄位能正確通過 MCP schema 反序列化，且不會破壞既有的 tab_id 檢查」——`send_input` 實際執行「兩次寫入」「baseline 更新」的行為已經在 Task 5 的 `coordination_ops.rs` 單元測試（`send_input_with_request_done_marker_sends_the_instruction_without_self_triggering`、`wait_for_idle_returns_early_via_marker_signal_without_any_bell`）裡驗證過了，不需要在這裡重複。

在 `src-tauri/tests/mcp_tool_server.rs` 最後加入（沿用檔案既有的 `test_router_with`/`call_tool` 輔助函式）：

```rust
#[tokio::test]
async fn send_input_accepts_request_done_marker_without_breaking_the_unknown_tab_rejection() {
    let (app, _pool) = test_router_with(true).await;
    let text = call_tool(
        app,
        "send_input",
        serde_json::json!({"tab_id": "nonexistent-tab-id", "text": "hi", "request_done_marker": true}),
    )
    .await;
    assert!(text.contains("was not created by spawn_tab"), "{text}");
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo build --lib 2>&1 | head -20`
Expected: 編譯錯誤——Task 5 已經把 `coordination_ops::send_input` 改成 5 個參數，但 `tools.rs` 裡的呼叫點（第 261 行附近）還是舊的 4 個參數，且 `SendInputArgs` 還沒有 `request_done_marker` 欄位可以解構

- [ ] **Step 3: 改 `SendInputArgs`**

把 `src-tauri/src/mcp_server/tools.rs:105-111` 的：

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendInputArgs {
    /// Tab id as returned by `spawn_tab`. Must be a tab this server spawned — never one the user opened by hand.
    pub tab_id: String,
    /// Text to send, as if typed into the tab followed by Enter.
    pub text: String,
}
```

改成：

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendInputArgs {
    /// Tab id as returned by `spawn_tab`. Must be a tab this server spawned — never one the user opened by hand.
    pub tab_id: String,
    /// Text to send, as if typed into the tab followed by Enter.
    pub text: String,
    /// Optional: if true, and the agent inside the target tab cooperates,
    /// wait_for_idle can return sooner than it otherwise would. Sends a
    /// short follow-up message asking the agent to print a specific marker
    /// when it's fully done. Not required — if the agent never prints it,
    /// this has no effect and idle detection falls back to the terminal
    /// bell exactly as if this were false. Defaults to false.
    #[serde(default)]
    pub request_done_marker: bool,
}
```

- [ ] **Step 4: 接上 `send_input` 工具方法**

把 `src-tauri/src/mcp_server/tools.rs:253-262` 的：

```rust
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
```

改成：

```rust
    #[tool(description = "Send text (as if typed, followed by Enter) to a tab previously created by spawn_tab. Cannot target a tab the user opened by hand. Disabled by default — must be enabled in Settings.")]
    async fn send_input(
        &self,
        Parameters(SendInputArgs { tab_id, text, request_done_marker }): Parameters<SendInputArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.require_coordination_enabled() {
            return to_call_result(Err(e));
        }
        to_call_result(coordination_ops::send_input(
            &self.pty_manager,
            &self.coordination_registry,
            &tab_id,
            &text,
            request_done_marker,
        ))
    }
```

- [ ] **Step 5: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test mcp_tool_server`
Expected: 全部通過，包含既有的 `coordination_tools_are_disabled_by_default`、`coordination_tool_rejects_an_unknown_tab_id`、`spawn_tab_without_an_app_handle_returns_a_clear_error`，以及新增的 `send_input_accepts_request_done_marker_without_breaking_the_unknown_tab_rejection`

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mcp_server/tools.rs src-tauri/tests/mcp_tool_server.rs
git commit -m "feat(mcp): expose request_done_marker on the send_input MCP tool"
```

---

## Task 7: 全套驗證與手動驗收

**Files:** 無新檔案——只跑既有的完整測試套件與型別檢查。

- [ ] **Step 1: 跑完整 Rust 測試套件**

Run: `cd src-tauri && cargo test`
Expected: 全部通過，沒有任何回歸（尤其是 `pty::session`、`pty::manager`、`mcp_server::coordination_ops`、`tests/mcp_tool_server` 這四個涵蓋到的模組）

- [ ] **Step 2: 跑 Rust lint**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
Expected: 無警告無錯誤

- [ ] **Step 3: 確認前端沒有受影響（這次改動完全在 Rust 後端，不動任何 `.ts`/`.tsx`）**

Run: `npx tsc -b`（在專案根目錄）
Expected: 無錯誤（本來就不該受影響，這步只是雙重確認沒有意外波及）

- [ ] **Step 4: 手動驗證（比照設計文件「測試」段落最後一項）**

1. 在 Settings → MCP Tool Server 頁面開啟「Agent coordination tools」
2. 用一個真的連上這個 MCP tool server 的 Claude Code session（協調端），呼叫 `spawn_tab({})` 開一個新分頁
3. 在新分頁裡手動打 `claude`，等它就緒
4. 協調端呼叫 `send_input({tab_id, text: "回答兩個字：測試", request_done_marker: true})`
5. 觀察新分頁：應該先看到 "測試" 的回應，然後看到協調端自動送出的中文指示訊息被當成第二則使用者訊息送出（Claude 應該會照做，印出 `<<AITERM_DONE:{tab_id}>>`）
6. 協調端呼叫 `wait_for_idle({tab_id})`，確認回傳的 `signal` 是 `"marker"` 而不是 `"bell"`，且比純等 bell 明顯更快浮現
7. 反向確認：同樣的流程但 `request_done_marker: false`（或整個不傳），確認行為跟這個功能開發前完全一樣——`signal` 應該是 `"bell"`

這步驟沒有自動化測試涵蓋（需要真的 Claude Code CLI 跟真的 MCP 連線），照設計文件的說明，這是唯一能證明整條協作流程真的通的驗證。

**這一步實測時真的抓到一個 bug**：第二段指示文字被目標端忽略，從未真正送出（見設計文件「第二段寫入被目標端忽略」段落）。修正見 Task 8。Task 8 完成後，Step 1-3 要重跑一次確認沒有回歸，Step 4 的手動驗證也要重新走一次確認指示文字這次真的被處理。

---

## Task 8：修正第二段寫入被目標端忽略（手動驗證階段發現）

**背景**：Task 7 Step 4 手動驗證時，真實 Claude Code CLI 上重現：`send_input` 背靠背送出的兩次 `\r` 寫入，第一段（任務文字）送出後目標端要花實際時間處理（實測 6 秒），第二段（指示文字）幾乎同時抵達，其內容被打進輸入框、但那個 `\r` 沒有觸發送出——指示文字永遠卡在輸入框，從未被目標端處理。詳細分析見 `docs/superpowers/specs/2026-08-21-coordination-done-marker-design.md` 的「第二段寫入被目標端忽略」段落。

**Files:**
- Modify: `src-tauri/src/mcp_server/coordination_ops.rs`（`send_input` 改成 `async fn`，新增 `wait_for_new_bell` 輔助函式）
- Modify: `src-tauri/src/mcp_server/tools.rs`（`send_input` 呼叫端補 `.await`；`SendInputArgs.request_done_marker` 與 `send_input` 工具描述文字更新，誠實告知會阻塞等待）
- Test: `src-tauri/src/mcp_server/coordination_ops.rs`（`mod tests`）

- [ ] **Step 1: 寫失敗的測試**

在 `coordination_ops.rs` 的 `mod tests` 區塊最後加入（`use super::*;` 已經在檔案最上面，這裡直接呼叫 module-private 的 `wait_for_new_bell` 沒問題）：

```rust
    #[tokio::test]
    async fn wait_for_new_bell_times_out_when_the_target_never_bells() {
        let pty_manager = PtyManager::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();

        let became_idle = wait_for_new_bell(&pty_manager, &tab_id, 0, Duration::from_millis(300)).await;
        assert!(!became_idle, "expected wait_for_new_bell to give up when no bell ever arrives");
    }

    #[tokio::test]
    async fn wait_for_new_bell_returns_true_once_a_new_bell_arrives() {
        let pty_manager = PtyManager::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();

        pty_manager.write(&tab_id, b"printf '\\007'\n").unwrap();

        let became_idle = wait_for_new_bell(&pty_manager, &tab_id, 0, Duration::from_secs(5)).await;
        assert!(became_idle, "expected wait_for_new_bell to observe the bell within the timeout");
    }
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib mcp_server::coordination_ops::tests::wait_for_new_bell_times_out_when_the_target_never_bells`
Expected: 編譯失敗，`error[E0425]: cannot find function `wait_for_new_bell` in this scope`

- [ ] **Step 3: 把 `send_input` 改成 `async fn`，新增 `wait_for_new_bell`**

把 `src-tauri/src/mcp_server/coordination_ops.rs` 裡的：

```rust
pub(crate) fn send_input(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
    text: &str,
    request_done_marker: bool,
) -> Result<String, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    pty_manager
        .write(tab_id, format!("{text}\r").as_bytes())
        .map_err(|e| e.to_string())?;

    // Sent as a second, independently \r-terminated write (a separate
    // message) rather than appended via an embedded '\n' — this file's own
    // send_input_terminates_the_line_with_cr_not_lf regression test exists
    // precisely because raw-mode TUIs (Claude Code's own included) have no
    // guaranteed behavior for a bare LF byte, only for CR. See the design
    // doc's "指示文字" section.
    //
    // The instruction text below deliberately never writes the complete,
    // contiguous marker string (prefix + tab_id + suffix back-to-back) —
    // each piece is separated by other text. If it did, canonical-mode
    // terminal echo alone (no cooperating agent needed) would write that
    // same contiguous byte sequence right back into this session's output
    // stream as a pure side effect of writing this instruction, and
    // marker_count would increment immediately — a false "done" signal
    // before the target has done anything. Verified live against a real
    // PTY during implementation: the original (buggy) wording, which did
    // embed the marker contiguously, triggered marker_count=1 from echo
    // alone, with no agent involved.
    if request_done_marker {
        let instruction = format!(
            "（可選：完成後請在新的一行印出一個完成標記，格式為三段直接相連、中間不留任何字元：前綴 {} ，接著是你的識別碼 {} ，最後接上 {} 。這能讓協調端提早得知你已完成，不影響任何其他行為。）",
            crate::pty::session::DONE_MARKER_PREFIX,
            tab_id,
            crate::pty::session::DONE_MARKER_SUFFIX
        );
        pty_manager
            .write(tab_id, format!("{instruction}\r").as_bytes())
            .map_err(|e| e.to_string())?;
    }

    // Reset the baseline to the counts *as of right now*, after both of this
    // call's own writes — before any reply has had a chance to arrive — so
    // idle only flips true again once a *new* bell or marker (a reply to
    // this specific input) is observed, not a stale one from before this
    // send.
    let bell_current = pty_manager.bell_count(tab_id).unwrap_or(0);
    let marker_current = pty_manager.marker_count(tab_id).unwrap_or(0);
    registry.record_baseline(tab_id, bell_current, marker_current);

    Ok(format!("sent to {tab_id}"))
}
```

改成：

```rust
pub(crate) async fn send_input(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
    text: &str,
    request_done_marker: bool,
) -> Result<String, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    pty_manager
        .write(tab_id, format!("{text}\r").as_bytes())
        .map_err(|e| e.to_string())?;

    // Sent as a second, independently \r-terminated write (a separate
    // message) rather than appended via an embedded '\n' — this file's own
    // send_input_terminates_the_line_with_cr_not_lf regression test exists
    // precisely because raw-mode TUIs (Claude Code's own included) have no
    // guaranteed behavior for a bare LF byte, only for CR. See the design
    // doc's "指示文字" section.
    //
    // The instruction text below deliberately never writes the complete,
    // contiguous marker string (prefix + tab_id + suffix back-to-back) —
    // each piece is separated by other text. If it did, canonical-mode
    // terminal echo alone (no cooperating agent needed) would write that
    // same contiguous byte sequence right back into this session's output
    // stream as a pure side effect of writing this instruction, and
    // marker_count would increment immediately — a false "done" signal
    // before the target has done anything. Verified live against a real
    // PTY during implementation: the original (buggy) wording, which did
    // embed the marker contiguously, triggered marker_count=1 from echo
    // alone, with no agent involved.
    //
    // The instruction is only sent once the target signals (via a fresh
    // bell) that it finished processing `text` and is idle again — sending
    // it immediately after the first write would race the target's own
    // processing time. Verified live against a real Claude Code CLI: the
    // instruction's characters got typed into the input box (proof the
    // write arrived) but its own \r never triggered submission, because the
    // target was still busy with `text` when it arrived. See the design
    // doc's "第二段寫入被目標端忽略" section.
    let mut instruction_sent = true;
    if request_done_marker {
        let bell_before = pty_manager.bell_count(tab_id).unwrap_or(0);
        let became_idle = wait_for_new_bell(
            pty_manager,
            tab_id,
            bell_before,
            Duration::from_secs(DEFAULT_WAIT_SECONDS),
        )
        .await;

        if became_idle {
            let instruction = format!(
                "（可選：完成後請在新的一行印出一個完成標記，格式為三段直接相連、中間不留任何字元：前綴 {} ，接著是你的識別碼 {} ，最後接上 {} 。這能讓協調端提早得知你已完成，不影響任何其他行為。）",
                crate::pty::session::DONE_MARKER_PREFIX,
                tab_id,
                crate::pty::session::DONE_MARKER_SUFFIX
            );
            pty_manager
                .write(tab_id, format!("{instruction}\r").as_bytes())
                .map_err(|e| e.to_string())?;
        } else {
            instruction_sent = false;
        }
    }

    // Reset the baseline to the counts *as of right now*, after both of this
    // call's own writes (or just the first, if the instruction was skipped
    // because the target never became idle) — before any reply has had a
    // chance to arrive — so idle only flips true again once a *new* bell or
    // marker (a reply to this specific input) is observed, not a stale one
    // from before this send.
    let bell_current = pty_manager.bell_count(tab_id).unwrap_or(0);
    let marker_current = pty_manager.marker_count(tab_id).unwrap_or(0);
    registry.record_baseline(tab_id, bell_current, marker_current);

    if request_done_marker && !instruction_sent {
        Ok(format!(
            "sent to {tab_id} (task only — target did not become idle within {DEFAULT_WAIT_SECONDS}s, so the completion-marker instruction was not sent)"
        ))
    } else {
        Ok(format!("sent to {tab_id}"))
    }
}

/// Polls `pty_manager`'s bell count for `tab_id`, returning `true` once it
/// exceeds `baseline` (a fresh bell — the target signaling it's idle again)
/// or `false` if `timeout` elapses first. Extracted as its own function
/// (rather than inlined in `send_input`) so tests can pass a short
/// `timeout` to exercise the "target never bells" path without waiting out
/// the real production timeout (`DEFAULT_WAIT_SECONDS` = 300s).
async fn wait_for_new_bell(pty_manager: &PtyManager, tab_id: &str, baseline: u64, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if pty_manager.bell_count(tab_id).unwrap_or(baseline) > baseline {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib mcp_server::coordination_ops::tests:: -- wait_for_new_bell`
Expected: `wait_for_new_bell_times_out_when_the_target_never_bells`、`wait_for_new_bell_returns_true_once_a_new_bell_arrives` 全部 `ok`（第一個測試只會等 300ms，不會等到 `DEFAULT_WAIT_SECONDS`）

Run: `cd src-tauri && cargo build --lib 2>&1 | head -60`
Expected: 編譯失敗——`send_input` 現在是 `async fn`，但既有呼叫端（`coordination_ops.rs` 自己的測試、`tools.rs`）都還是同步呼叫，缺少 `.await`。這是預期中的失敗，Step 5-6 會修好。

- [ ] **Step 5: 更新 `coordination_ops.rs` 既有測試呼叫點，補上 `.await`**

`mod tests` 裡所有直接呼叫 `send_input(...)` 的地方（全部已經是 `#[tokio::test] async fn`，不需要改測試函式本身的 async 性質，只需要在呼叫後面補 `.await`）：

把：
```rust
        let err = send_input(&pty_manager, &registry, "not-a-real-tab", "hello", false).unwrap_err();
```
改成：
```rust
        let err = send_input(&pty_manager, &registry, "not-a-real-tab", "hello", false).await.unwrap_err();
```

把：
```rust
        let sent = send_input(&pty_manager, &registry, &tab_id, "echo hi", false).unwrap();
```
改成：
```rust
        let sent = send_input(&pty_manager, &registry, &tab_id, "echo hi", false).await.unwrap();
```

把（`send_input_terminates_the_line_with_cr_not_lf` 測試裡）：
```rust
        send_input(&pty_manager, &registry, &tab_id, "ab", false).unwrap();
```
改成：
```rust
        send_input(&pty_manager, &registry, &tab_id, "ab", false).await.unwrap();
```

- [ ] **Step 6: 重寫 `send_input_with_request_done_marker_sends_the_instruction_without_self_triggering` 測試**

**這個既有測試（Task 5 留下的）在新行為下會逾時**：它對一個空 shell 送 `request_done_marker: true`，空 shell 執行 `echo hi` 不會觸發任何 bell，新行為下 `send_input` 會等到 `DEFAULT_WAIT_SECONDS`（300 秒）才放棄——測試會直接卡住 5 分鐘，不能就這樣留著。修法：在背景另開一個 task，短暫延遲後手動送一個 bell 位元組序列進同一個 PTY，模擬「目標端剛處理完任務文字、回到閒置」，讓 `send_input` 內部的等待可以很快解除。

把：

```rust
    #[tokio::test]
    async fn send_input_with_request_done_marker_sends_the_instruction_without_self_triggering() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        send_input(&pty_manager, &registry, &tab_id, "echo hi", true).unwrap();

        // Wait for the echoed instruction to actually arrive (it mentions
        // this tab's own id, proving the instruction was sent).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
            if output.contains(&tab_id) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "expected the echoed input to mention the tab_id, got: {output}"
            );
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }

        // But the full, contiguous marker must NEVER appear anywhere in what
        // was echoed — if it did, the instruction itself would have
        // self-triggered a false completion signal via terminal echo alone,
        // with no agent involved. This is a regression test for a real bug
        // found during review: the original wording embedded the complete
        // contiguous marker in the instruction, and canonical-mode echo
        // alone incremented marker_count to 1 against a plain shell that
        // did nothing.
        let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
        let full_marker = crate::pty::session::done_marker(&tab_id);
        assert!(
            !output.contains(&full_marker),
            "the instruction text must never contain the complete contiguous marker — got: {output}"
        );
        assert_eq!(
            pty_manager.marker_count(&tab_id),
            Some(0),
            "writing the instruction alone must not increment marker_count — self-echo false positive"
        );
    }
```

改成：

```rust
    #[tokio::test]
    async fn send_input_with_request_done_marker_sends_the_instruction_once_the_target_bells() {
        let pty_manager = Arc::new(PtyManager::new());
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        // Simulate a cooperating target: shortly after send_input's first
        // write lands, ring a bell — as if the target just finished
        // processing the task text and is idle again, waiting for new
        // input. Only once this bell is observed should send_input proceed
        // to write the instruction (see wait_for_new_bell).
        let pty_manager_for_bell = Arc::clone(&pty_manager);
        let tab_id_for_bell = tab_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            pty_manager_for_bell.write(&tab_id_for_bell, b"printf '\\007'\n").unwrap();
        });

        send_input(&pty_manager, &registry, &tab_id, "echo hi", true).await.unwrap();

        // Wait for the echoed instruction to actually arrive (it mentions
        // this tab's own id, proving the instruction was sent).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
            if output.contains(&tab_id) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "expected the echoed input to mention the tab_id, got: {output}"
            );
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }

        // But the full, contiguous marker must NEVER appear anywhere in what
        // was echoed — if it did, the instruction itself would have
        // self-triggered a false completion signal via terminal echo alone,
        // with no agent involved. This is a regression test for a real bug
        // found during review: the original wording embedded the complete
        // contiguous marker in the instruction, and canonical-mode echo
        // alone incremented marker_count to 1 against a plain shell that
        // did nothing.
        let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
        let full_marker = crate::pty::session::done_marker(&tab_id);
        assert!(
            !output.contains(&full_marker),
            "the instruction text must never contain the complete contiguous marker — got: {output}"
        );
    }

    /// Same scenario as the test above, but no background bell is ever
    /// injected. This would take DEFAULT_WAIT_SECONDS (300s) to time out
    /// for real, which is far too slow to run on every `cargo test` — so
    /// this is `#[ignore]`d (run manually/in a slow-test lane when
    /// touching this code path, e.g. `cargo test --lib -- --ignored
    /// send_input_with_request_done_marker_skips_the_instruction_when_the_target_never_bells`).
    /// `wait_for_new_bell`'s own give-up behavior is already covered fast
    /// by `wait_for_new_bell_times_out_when_the_target_never_bells` above;
    /// this test additionally confirms `send_input`'s own wiring and
    /// returned message wording end-to-end.
    #[ignore]
    #[tokio::test]
    async fn send_input_with_request_done_marker_skips_the_instruction_when_the_target_never_bells() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        let sent = send_input(&pty_manager, &registry, &tab_id, "echo hi", true).await.unwrap();
        assert!(
            sent.contains("task only") && sent.contains("was not sent"),
            "expected the return message to note the instruction was skipped, got: {sent}"
        );
    }
```

- [ ] **Step 7: 加上 `Arc` import**

Step 6 的新測試用到 `Arc::clone`——確認檔案最上面的 `use` 區塊有 `use std::sync::Arc;`（如果沒有就加上去；`PtyManager` 本身在 `pty/manager.rs` 裡已經用 `Arc` 包裝 session，這裡是測試自己另外包一層 `Arc<PtyManager>` 給背景 task 用，跟 `PtyManager` 內部的 `Arc` 無關）。

- [ ] **Step 8: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib mcp_server::coordination_ops::`
Expected: 全部通過（`#[ignore]` 的那個測試預設不會跑，這是預期行為），包含新增的 `wait_for_new_bell_times_out_when_the_target_never_bells`、`wait_for_new_bell_returns_true_once_a_new_bell_arrives`、`send_input_with_request_done_marker_sends_the_instruction_once_the_target_bells`，以及所有既有測試（含補上 `.await` 的那幾個）

Run: `cd src-tauri && cargo test --lib mcp_server::coordination_ops:: -- --ignored send_input_with_request_done_marker_skips_the_instruction_when_the_target_never_bells`
Expected: 通過（會真的等 300 秒——這步驟很慢，只需要跑過一次確認邏輯正確即可，不需要每次都跑）

- [ ] **Step 9: 更新 `tools.rs` 呼叫端補 `.await`**

把 `src-tauri/src/mcp_server/tools.rs` 裡的：

```rust
        to_call_result(coordination_ops::send_input(
            &self.pty_manager,
            &self.coordination_registry,
            &tab_id,
            &text,
            request_done_marker,
        ))
```

改成：

```rust
        to_call_result(coordination_ops::send_input(
            &self.pty_manager,
            &self.coordination_registry,
            &tab_id,
            &text,
            request_done_marker,
        ).await)
```

- [ ] **Step 10: 更新 `SendInputArgs.request_done_marker` 與 `send_input` 工具描述文字，誠實告知會阻塞**

把 `src-tauri/src/mcp_server/tools.rs` 裡的：

```rust
    /// Optional: if true, and the agent inside the target tab cooperates,
    /// wait_for_idle can return sooner than it otherwise would. Sends a
    /// short follow-up message asking the agent to print a specific marker
    /// when it's fully done. Not required — if the agent never prints it,
    /// this has no effect and idle detection falls back to the terminal
    /// bell exactly as if this were false. Defaults to false.
    #[serde(default)]
    pub request_done_marker: bool,
```

改成：

```rust
    /// Optional: if true, and the agent inside the target tab cooperates,
    /// wait_for_idle can return sooner than it otherwise would. Once the
    /// target signals (via a bell) that it has finished processing `text`,
    /// sends a short follow-up message asking it to print a specific
    /// marker when fully done. Not required — if the target never prints
    /// it, this has no effect and idle detection falls back to the
    /// terminal bell exactly as if this were false.
    ///
    /// Important: when true, this call blocks (up to 300 seconds) until the
    /// target becomes idle from `text` before returning — it is NOT instant
    /// like a plain send_input call. If the target never bells within that
    /// window, the follow-up message is simply not sent (noted in this
    /// call's own response) rather than erroring. Defaults to false.
    #[serde(default)]
    pub request_done_marker: bool,
```

把：

```rust
    #[tool(description = "Send text (as if typed, followed by Enter) to a tab previously created by spawn_tab. Cannot target a tab the user opened by hand. Disabled by default — must be enabled in Settings.")]
    async fn send_input(
```

改成：

```rust
    #[tool(description = "Send text (as if typed, followed by Enter) to a tab previously created by spawn_tab. Cannot target a tab the user opened by hand. If request_done_marker is true, this call blocks (up to 300s) waiting for the target to finish processing the text before sending a completion-marker request — it is not instant in that case. Disabled by default — must be enabled in Settings.")]
    async fn send_input(
```

- [ ] **Step 11: 執行測試確認通過**

Run: `cd src-tauri && cargo build --lib`
Expected: 編譯成功，無新警告

Run: `cd src-tauri && cargo test --test mcp_tool_server`
Expected: 全部通過，包含既有的 `send_input_accepts_request_done_marker_without_breaking_the_unknown_tab_rejection`

Run: `cd src-tauri && cargo test --lib`
Expected: 全部通過，沒有回歸

- [ ] **Step 12: Commit**

```bash
git add src-tauri/src/mcp_server/coordination_ops.rs src-tauri/src/mcp_server/tools.rs
git commit -m "fix(mcp): wait for target to idle before sending done-marker instruction"
```

- [ ] **Step 13: 重跑 Task 7 的自動化驗證**

Run: `cd src-tauri && cargo test`
Expected: 全部通過，沒有回歸

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings 2>&1 | grep -E "coordination_ops\.rs|tools\.rs"`
Expected: 沒有新增的警告（既有的 `tool_router`/`ptr_arg` 既有警告不算）

Run: `npx tsc -b`（在專案根目錄）
Expected: 無錯誤

- [ ] **Step 14: 重新手動驗證**

比照 Task 7 Step 4 的流程再走一次，這次額外確認：協調端送出的指示文字**真的被目標端處理**，目標端印出完成標記，`wait_for_idle` 回報 `signal: "marker"`。

---

## Self-Review 摘要（寫計畫時做過，記錄在此供執行者參考）

- **Spec 涵蓋**：設計文件的每個「含」項目都對應到 Task 1-6 的具體步驟；「不含」項目（新 MCP 工具、改變 bell 行為、`spawn()`/`spawn_with_id()` 重構、分頁清理）都沒有出現在任何 Task 裡。
- **兩處在寫計畫過程中修正的設計缺口**（已回頭更新 spec 並各自 commit）：`request_done_marker` 從「`spawn_tab`+`send_input`都加」收斂成「只加在 `send_input`」（`spawn_tab` 的 `command` 沒有可回報完成的任務，且會有寫入競態風險）；指示文字從「用 `\n` 接成一次寫入」改成「兩次獨立 `\r` 結尾的寫入」（避免對 raw-mode TUI 送出未經驗證的 LF 位元組，這個檔案自己既有的 `send_input_terminates_the_line_with_cr_not_lf` 測試就是在防同一類問題）。
- **型別一致性**：`CoordinationRegistry::record_baseline`/`baseline` 的簽名變化（Task 4）跟 Task 5 所有呼叫端（`spawn_tab`/`send_input`/`status_for`）用的都是同一組 `(bell, marker)` 語意；`done_marker`/`contains_marker`/`marker_tail_after`（Task 1）在 Task 2 的讀取迴圈、Task 5 的 `send_input`、Task 6 的整合測試裡都是同一個函式簽名，沒有重新定義過。
- **執行階段發現並修正的設計缺陷（非寫計畫時發現，第一輪 code-quality 複審後才浮現）**：Task 5 的指示文字第一版直接把完整、連續的標記字串寫進指示文字本身。真實 PTY 上驗證過：canonical mode 的終端機本地 echo 會把寫進去的位元組原樣送回輸出流，讀取迴圈掃到的正是同一條資料流——只要指示文字包含完整連續標記，光是送出指示、什麼都還沒發生，`marker_count` 就會被觸發，跟目標端有沒有真的完成任何事無關。這不是實作誤差，是設計文件「指示文字」段落本身的結構性缺陷，已回頭修正 spec（拆成三段描述，中間插入其他文字，指示文字本身不會連續出現完整標記）並更新這份計畫的 Task 5 Step 3／Step 9（含新增 `DONE_MARKER_PREFIX`/`DONE_MARKER_SUFFIX` 可見性改成 `pub`、原本斷言「回顯含完整標記」的測試換成「回顯含 tab_id 但不含完整連續標記、且 `marker_count` 維持 0」）。
