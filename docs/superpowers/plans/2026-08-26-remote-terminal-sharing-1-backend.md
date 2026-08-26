# 遠端終端機共享 ①：後端傳輸地基 — 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓一個終端機分頁的原始 PTY 輸出能同時餵給多個遠端訂閱者、接受經授權的遠端輸入，並用加密且可人工核對身分的連線傳輸——全部在 Rust 後端完成，可用整合測試端到端驗證，不碰任何前端。

**Architecture:** `PtySession` 的輸出從單一 callback 改為「既有 callback ＋ `tokio::sync::broadcast` fan-out」，沒有訂閱者時只付一次原子讀取的代價。新增獨立的 `share` 模組：一個 axum server（第三個，與 `bridge`／`mcp_server` 並列但**綁區網介面**）、一個管理短碼與控制權的純邏輯狀態機、以及 TLS 自簽憑證＋RFC 5705 匯出金鑰導出的 4 位驗證碼（SAS）。

**Tech Stack:** Rust / axum 0.8（新開 `ws` feature）/ tokio broadcast / rustls 0.23 ＋ tokio-rustls 0.26（已在 Cargo.lock）/ rcgen（新增）/ tokio-tungstenite（新增，僅 dev-dependency，測試用的 ws 客戶端）

**Spec:** `docs/superpowers/specs/2026-08-26-remote-terminal-sharing-design.md`

**本計畫不含**（各自留給後續計畫）：任何前端 UI（計畫②）、mDNS 區網自動發現（計畫③）。計畫①結束時，連線只能用 `host:port` 手動指定，並由整合測試驅動。

---

## 檔案結構

| 檔案 | 責任 | 動作 |
|---|---|---|
| `src-tauri/src/pty/session.rs` | 消除 `spawn`/`spawn_with_id` 重複；ring buffer 常數化並加大；`get_recent_raw`；broadcast fan-out | 修改 |
| `src-tauri/src/pty/manager.rs` | 轉發 `get_recent_raw` 與 `subscribe` | 修改 |
| `src-tauri/src/share/mod.rs` | 模組宣告 ＋ `ShareServerState`（啟停生命週期，鏡像 `McpToolServerState`） | 新增 |
| `src-tauri/src/share/registry.rs` | 純邏輯狀態機：短碼、待審連線、觀看者、單一控制權 | 新增 |
| `src-tauri/src/share/protocol.rs` | ws 訊息型別（serde），前後端共用的線上格式 | 新增 |
| `src-tauri/src/share/tls.rs` | 自簽臨時憑證產生、rustls 設定、SAS 導出 | 新增 |
| `src-tauri/src/share/server.rs` | axum router ＋ ws handler（握手、審核、串流、輸入授權） | 新增 |
| `src-tauri/src/lib.rs` | 註冊 `share` 模組與 `ShareServerState` | 修改 |
| `src-tauri/Cargo.toml` | `axum` 開 `ws`；新增 `rcgen`、`rustls`、`tokio-rustls`；dev-dep 新增 `tokio-tungstenite` | 修改 |
| `src-tauri/tests/share_end_to_end.rs` | 整合測試：連線→審核→重播→串流→輸入授權→結束 | 新增 |

拆成五個小檔案而非一個大 `share.rs`：`registry.rs` 是不碰 I/O 的純邏輯（最需要密集單元測試）、`tls.rs` 是唯一碰密碼學的地方（審查時要能單獨看完）、`protocol.rs` 是計畫②前端要對照的線上格式契約。

---

## Task 1: 消除 `spawn` 與 `spawn_with_id` 的重複

**這是重構，不是新功能——不要為它捏造一個會紅的測試。** `PtySession::spawn`（`session.rs:274-422`）與 `spawn_with_id`（`:426-577`）目前是逐行相同的兩份拷貝，唯一差別是 id 的來源。後面 Task 2、Task 3 都要改 reader thread 的內容，不先合併就得改兩處、錯一處。

**Files:**
- Modify: `src-tauri/src/pty/session.rs:274-422`

- [ ] **Step 1: 先確認現在是綠的（重構的安全網）**

Run: `cd src-tauri && cargo test --lib pty::`
Expected: PASS。把通過的測試數量記下來，Step 4 要比對同一個數字。

- [ ] **Step 2: 確認兩份拷貝除了 id 之外真的完全相同**

```bash
cd src-tauri
sed -n '278,422p' src/pty/session.rs | grep -v 'let id = Uuid::new_v4' > /tmp/spawn_body.txt
sed -n '436,577p' src/pty/session.rs > /tmp/spawn_with_id_body.txt
diff /tmp/spawn_body.txt /tmp/spawn_with_id_body.txt
```

Expected: 只有空白／結尾大括號層級的差異。**若 diff 顯示任何實質邏輯差異，停下來回報**——代表兩份拷貝已經漂移，合併會改變行為，需要先確認哪一份才是對的。

- [ ] **Step 3: 把 `spawn` 改成委派**

把 `session.rs:274-422` 整個 `spawn` 函式（從 `pub fn spawn<F>` 到它的結尾 `}`）替換成：

```rust
    /// Spawn with a generated id. Thin wrapper over `spawn_with_id` — the two
    /// were byte-identical copies until this was collapsed, which meant every
    /// change to the reader thread had to be made twice.
    pub fn spawn<F>(shell: ShellSpec, size: PtySize, cwd: Option<PathBuf>, on_data: F) -> PtyResult<Self>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        Self::spawn_with_id(shell, size, Uuid::new_v4().to_string(), cwd, on_data)
    }
```

注意：參數從 `mut on_data` 改成 `on_data`（這裡不再自己呼叫它，只是傳下去）。

- [ ] **Step 4: 確認測試數量與結果都沒變**

Run: `cd src-tauri && cargo test --lib pty::`
Expected: PASS，且通過數量與 Step 1 完全相同。

- [ ] **Step 5: 確認 reader thread 的內容現在只剩一份**

Run: `cd src-tauri && grep -c 'pty reader error' src/pty/session.rs`
Expected: `1`（合併前是 `2`）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/pty/session.rs
git commit -m "refactor(pty): collapse PtySession::spawn into spawn_with_id

兩者原本是逐行相同的拷貝，改 reader thread 得改兩處。"
```

---

## Task 2: ring buffer 常數化、加大，並新增 `get_recent_raw`

ring buffer 目前上限是寫死在 reader thread 裡的 `const RING_CAP: usize = 8 * 1024`（`session.rs:353`）。8 KB 對 AI 取上下文夠用，但拿來重播終端機畫面只有二三十行，遠端連進去幾乎看不到歷史。同時需要一支**不做 ANSI strip** 的讀取方法——重播必須是原始位元組，strip 掉逃脫序列等於給對方一份沒有顏色、沒有游標位置的近似品。

**Files:**
- Modify: `src-tauri/src/pty/session.rs`
- Modify: `src-tauri/src/pty/manager.rs`
- Test: `src-tauri/src/pty/session.rs`（既有的 `#[cfg(test)] mod tests`）

- [ ] **Step 1: 寫會紅的測試**

加到 `session.rs` 既有的測試模組裡（跟 `bell_byte_in_output_increments_bell_count` 同一個 mod）：

```rust
    #[tokio::test]
    async fn get_recent_raw_keeps_ansi_escapes_that_get_recent_output_strips() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");

        // A red "SHAREME" followed by a reset. get_recent_output must not show
        // the escapes; get_recent_raw must.
        #[cfg(windows)]
        session.write(b"echo \x1b[31mSHAREME\x1b[0m\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"printf '\\033[31mSHAREME\\033[0m\\n'\n").unwrap();

        // Real PTY I/O on a background thread — poll rather than sleep a fixed
        // amount, same as the bell test above.
        let mut raw = None;
        for _ in 0..50 {
            if let Some(bytes) = session.get_recent_raw(64 * 1024) {
                // 不能只找裸的 "SHAREME"：PTY 會回顯你打進去的那行指令，而
                // 指令原文裡的 `\033` 是四個 ASCII 字元、不是真的 ESC byte，
                // 卻同樣含有 "SHAREME"。搜尋含真 ESC 的完整序列才能保證只
                // 匹配到 printf 真正執行後的著色輸出——否則這個測試大約每
                // 20 次會 flake 一次（已實測重現）。
                if bytes.windows(12).any(|w| w == b"\x1b[31mSHAREME") {
                    raw = Some(bytes);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let raw = raw.expect("expected SHAREME to show up in the raw ring buffer");

        assert!(
            raw.contains(&0x1b),
            "get_recent_raw must preserve ESC bytes; got {:?}",
            String::from_utf8_lossy(&raw)
        );
        let stripped = session.get_recent_output(64 * 1024).expect("stripped output");
        assert!(
            !stripped.contains('\u{1b}'),
            "get_recent_output must still strip ESC bytes; got {stripped:?}"
        );
    }

    #[test]
    fn output_ring_cap_is_large_enough_to_replay_a_screen() {
        // A single full redraw of an 80x24 screen with colour runs well past
        // 8 KB. A share viewer primed from a ring that small would see a
        // fragment of a screen, so this floor is a real requirement, not a
        // preference.
        assert!(
            OUTPUT_RING_CAP >= 128 * 1024,
            "OUTPUT_RING_CAP too small to prime a share viewer: {OUTPUT_RING_CAP}"
        );
    }
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --lib pty::session::tests::get_recent_raw_keeps_ansi_escapes_that_get_recent_output_strips`
Expected: **編譯失敗**——`no method named get_recent_raw`、`cannot find value OUTPUT_RING_CAP`。

- [ ] **Step 3: 把 RING_CAP 提到模組層級並加大**

在 `session.rs` 裡 `pub struct PtySession` 定義的正上方加入：

```rust
/// Raw PTY bytes retained per session. Serves two readers with different
/// appetites: AI context (which asks for a few KB) and screen sharing (which
/// replays this whole buffer to prime a newly connected viewer's terminal).
/// The share case sets the floor — a full 80x24 colour redraw runs well past
/// the 8 KB this used to be. At 256 KB, twenty open tabs cost ~5 MB.
const OUTPUT_RING_CAP: usize = 256 * 1024;
```

然後把 reader thread 裡的區塊（Task 1 合併後只剩一處）：

```rust
                            {
                                let mut ring = ring_for_thread.lock();
                                const RING_CAP: usize = 8 * 1024;
                                for &b in &chunk {
                                    if ring.len() >= RING_CAP { ring.pop_front(); }
                                    ring.push_back(b);
                                }
                            }
```

換成：

```rust
                            {
                                let mut ring = ring_for_thread.lock();
                                for &b in &chunk {
                                    if ring.len() >= OUTPUT_RING_CAP { ring.pop_front(); }
                                    ring.push_back(b);
                                }
                            }
```

- [ ] **Step 4: 實作 `get_recent_raw`**

加在 `session.rs` 既有的 `get_recent_output`（`:716`）正下方：

```rust
    /// Return the last `max_bytes` bytes of terminal output exactly as the PTY
    /// produced them — ANSI escapes intact. Screen sharing replays this to
    /// prime a newly connected viewer; `get_recent_output`'s stripping would
    /// hand them a colourless, cursor-less approximation instead.
    ///
    /// Unlike `get_recent_output` this does not treat whitespace-only content
    /// as "nothing" — a screen that genuinely holds only blank lines is still
    /// the screen the viewer must be shown. `None` means nothing has been
    /// captured at all.
    pub fn get_recent_raw(&self, max_bytes: usize) -> Option<Vec<u8>> {
        let ring = self.output_ring.lock();
        if ring.is_empty() {
            return None;
        }
        let start = ring.len().saturating_sub(max_bytes);
        Some(ring.iter().skip(start).copied().collect())
    }
```

- [ ] **Step 5: 在 `PtyManager` 轉發**

加在 `manager.rs` 既有的 `get_recent_output`（`:104`）正下方：

```rust
    /// Raw (not ANSI-stripped) recent output for the given session. See
    /// `PtySession::get_recent_raw`.
    pub fn get_recent_raw(&self, id: &str, max_bytes: usize) -> Option<Vec<u8>> {
        self.sessions.lock().get(id).and_then(|s| s.get_recent_raw(max_bytes))
    }
```

- [ ] **Step 6: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --lib pty::`
Expected: PASS，含兩個新測試。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/pty/session.rs src-tauri/src/pty/manager.rs
git commit -m "feat(pty): add get_recent_raw and enlarge the output ring for share replay

8 KB 夠 AI 取上下文，但重播終端機畫面只有二三十行。"
```

---

## Task 3: PTY 輸出 broadcast fan-out

**Files:**
- Modify: `src-tauri/src/pty/session.rs`
- Modify: `src-tauri/src/pty/manager.rs`
- Test: `src-tauri/src/pty/session.rs`

- [ ] **Step 1: 寫會紅的測試**

```rust
    #[tokio::test]
    async fn every_subscriber_receives_the_same_output_and_on_data_still_fires() {
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

        // Two independent viewers, subscribed before anything is written.
        let mut a = session.subscribe();
        let mut b = session.subscribe();

        #[cfg(windows)]
        session.write(b"echo FANOUT\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"printf 'FANOUT\\n'\n").unwrap();

        // 這裡搜尋裸的 "FANOUT" 是**刻意的、也是正確的**，跟 Task 2 那個
        // 測試不同：PTY 會回顯你打進去的指令，所以匹配可能命中回顯而不是
        // printf 的輸出——但那無所謂，回顯同樣是走 PTY 輸出、同樣要經過
        // fan-out，正是這個測試要驗的東西。Task 2 那裡不能這樣做，是因為
        // 它斷言的是「位元組裡有真正的 ESC」，而回顯裡沒有。
        //
        // 換句話說：會不會被回顯提前命中，取決於你的斷言在乎什麼，不是
        // 一律要避開。
        async fn collect_until_marker(
            r: &mut tokio::sync::broadcast::Receiver<Vec<u8>>,
        ) -> Vec<u8> {
            let mut acc = Vec::new();
            for _ in 0..50 {
                match tokio::time::timeout(Duration::from_millis(200), r.recv()).await {
                    Ok(Ok(chunk)) => {
                        acc.extend_from_slice(&chunk);
                        if acc.windows(6).any(|w| w == b"FANOUT") {
                            return acc;
                        }
                    }
                    Ok(Err(_)) => break,
                    Err(_) => continue,
                }
            }
            acc
        }

        let got_a = collect_until_marker(&mut a).await;
        let got_b = collect_until_marker(&mut b).await;

        assert!(
            got_a.windows(6).any(|w| w == b"FANOUT"),
            "subscriber A missed the output"
        );
        assert!(
            got_b.windows(6).any(|w| w == b"FANOUT"),
            "subscriber B missed the output"
        );

        // 名字承諾的是「相同」，就要真的驗相同。兩個 receiver 訂閱的是同一個
        // sender，reader thread 每個 chunk 只 send 一次，所以兩邊收到的 chunk
        // 序列（順序、內容、邊界）完全一致；而 collect_until_marker 的停止
        // 條件是「累積內容是否含 FANOUT」——一個純內容函數，不受即時時序
        // 影響。因此兩邊會在收到相同數量的 chunk 後停止，這個斷言是確定性的。
        assert_eq!(
            got_a, got_b,
            "subscribers received different bytes for the same broadcast"
        );

        // The pre-existing on_data path must be untouched by fan-out.
        let mut via_callback = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            via_callback.extend_from_slice(&chunk);
        }
        assert!(
            via_callback.windows(6).any(|w| w == b"FANOUT"),
            "the original on_data callback stopped seeing output"
        );
    }

    #[test]
    fn subscriber_count_reflects_subscribe_and_drop() {
        // 注意這個測試**不**保證 reader thread 裡的 `receiver_count() > 0`
        // 守衛還在。那個守衛純粹是效能優化（省一次 chunk.clone()）——把它
        // 拿掉之後 `send()` 自己會對 0 receiver 安全地回 `Err` 而呼叫端用
        // `let _ =` 吞掉，功能完全不變，所以**沒有任何功能斷言抓得到它被
        // 移除**。要抓只能量 heap 配置次數，那是效能測試不是這裡的事。
        //
        // 這個測試守的是 subscribe()/Drop 的計數邏輯本身。名字要誠實反映
        // 它真正驗證的東西——掛一個假承諾的名字比沒有測試更糟。
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");

        assert_eq!(session.subscriber_count(), 0);
        let a = session.subscribe();
        assert_eq!(session.subscriber_count(), 1);
        let b = session.subscribe();
        assert_eq!(session.subscriber_count(), 2);
        drop(a);
        assert_eq!(session.subscriber_count(), 1);
        drop(b);
        assert_eq!(session.subscriber_count(), 0);
    }
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --lib pty::session::tests::every_subscriber_receives_the_same_output_and_on_data_still_fires`
Expected: **編譯失敗**——`no method named subscribe`、`no method named subscriber_count`。

- [ ] **Step 3: 在 `PtySession` 加 broadcast 欄位**

在 `session.rs` 模組層級、`OUTPUT_RING_CAP` 旁邊加入：

```rust
/// How many output chunks the fan-out channel buffers before the slowest
/// subscriber starts losing the oldest. Each slot holds one `Vec<u8>` of up
/// to 4096 bytes (the reader thread's read buffer size), and tokio allocates
/// every slot up front — so this is a worst case of roughly 1 MB per shared
/// tab, and it multiplies by the number of tabs being shared at once.
///
/// Deliberately not larger: a bigger buffer only delays `Lagged`, it cannot
/// prevent it, and the resynchronise-from-the-ring-buffer path has to exist
/// either way. A lagging viewer is never silently tolerated — it is told to
/// resync (see `share::server`, built in a later task of this plan), because
/// a terminal that misses bytes mid-escape-sequence renders wrong from then
/// on and never recovers on its own.
const OUTPUT_BROADCAST_CAP: usize = 256;
```

在 `pub struct PtySession { ... }` 的欄位清單末端（`marker_count` 之後）加入：

```rust
    /// Fan-out of raw output chunks to screen-share viewers. Independent of
    /// the `on_data` callback, which continues to serve the app's own
    /// terminal view. Subscribers appear only while a tab is being shared.
    output_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
```

- [ ] **Step 4: 在 `spawn_with_id` 建立 channel 並在 reader thread 送出**

在 `spawn_with_id` 裡、`let done_marker_bytes = ...` 那行下面加入：

```rust
        let (output_tx, _) =
            tokio::sync::broadcast::channel::<Vec<u8>>(OUTPUT_BROADCAST_CAP);
        let output_tx_for_thread = output_tx.clone();
```

在 reader thread 裡、`on_data(chunk);` 那行的**正上方**加入：

```rust
                            // Only pay for the clone when somebody is actually
                            // watching. With no viewers this costs one
                            // uncontended lock of the broadcast channel's
                            // internal tail mutex — cheap, but not literally
                            // free: `receiver_count()` takes that lock rather
                            // than doing an atomic load. Don't build a
                            // performance assumption on it being lock-free.
                            if output_tx_for_thread.receiver_count() > 0 {
                                let _ = output_tx_for_thread.send(chunk.clone());
                            }
```

在函式結尾的 `Ok(Self { ... })` 建構式裡，`marker_count,` 之後加入：

```rust
            output_tx,
```

- [ ] **Step 5: 加上 `subscribe` 與 `subscriber_count`**

加在 `session.rs` 的 `get_recent_raw` 正下方：

```rust
    /// Subscribe to this session's raw output. Every subscriber receives every
    /// chunk produced after it subscribed; history comes from
    /// `get_recent_raw`, not from this channel.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    /// How many share viewers are currently attached to this session.
    pub fn subscriber_count(&self) -> usize {
        self.output_tx.receiver_count()
    }
```

- [ ] **Step 6: 在 `PtyManager` 轉發**

加在 `manager.rs` 的 `get_recent_raw` 正下方：

```rust
    /// Subscribe to a session's raw output, or `None` if it doesn't exist.
    /// See `PtySession::subscribe`.
    pub fn subscribe(&self, id: &str) -> Option<tokio::sync::broadcast::Receiver<Vec<u8>>> {
        self.sessions.lock().get(id).map(|s| s.subscribe())
    }
```

- [ ] **Step 7: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --lib pty::`
Expected: PASS。

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/pty/session.rs src-tauri/src/pty/manager.rs
git commit -m "feat(pty): fan out raw output to screen-share subscribers

既有的 on_data 路徑完全不動；沒有訂閱者時只付一次 receiver_count 的代價。"
```

---

## Task 3b: 原子的「快照 ＋ 訂閱」

**這個 task 是 Task 3 的品質審查發現後補上的，取代了原計畫「接受重複、避免缺漏」的取捨。**

原本 Task 6 打算先 `subscribe()` 再 `get_recent_raw()`，理由是「重複比缺漏好」。但兩者用的是**兩把完全獨立的鎖**（`output_ring` 的 `Mutex` vs broadcast 內部的 tail lock），reader thread 也是先寫 ring、後 broadcast，中間沒有任何耦合。所以：

- 先 `subscribe()` 再取快照 → 中間到達的 chunk **同時**在快照與串流裡 → 重複位元組
- 先取快照再 `subscribe()` → 中間到達的 chunk **兩邊都沒有** → 缺漏，正是會截斷 ANSI 逃脫序列、畫面從此錯亂的那種災難

**兩害相權不是唯一出路——可以兩個都不要。** 讓 reader thread 在**同一把 ring 鎖裡**完成「寫 ring」與「broadcast 送出」，再提供一個持有那把鎖的 `subscribe_with_history`。這樣任何 chunk 要嘛在我們取得鎖之前就已寫入 ring（在快照裡），要嘛必須等我們放開鎖才能廣播（而那時我們已經訂閱了）。既不重複也不漏。

鎖順序在兩條路徑上一致（ring → broadcast tail），不會死鎖。代價只是 reader thread 多持有 ring 鎖跨一次**非阻塞**的 send。

**Files:**
- Modify: `src-tauri/src/pty/session.rs`
- Modify: `src-tauri/src/pty/manager.rs`
- Test: `src-tauri/src/pty/session.rs`

- [ ] **Step 1: 寫會紅的測試**

```rust
    #[tokio::test]
    async fn subscribe_with_history_loses_no_bytes_and_duplicates_none() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");

        // 先產生一段歷史，確認它會出現在快照裡。
        #[cfg(windows)]
        session.write(b"echo HIST\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"printf 'HIST\\n'\n").unwrap();

        let mut history = None;
        for _ in 0..50 {
            if let Some(bytes) = session.get_recent_raw(64 * 1024) {
                if bytes.windows(4).any(|w| w == b"HIST") {
                    history = Some(bytes);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        history.expect("history should contain HIST before we subscribe");

        // 原子地取快照＋訂閱。
        let (snapshot, mut rx) = session.subscribe_with_history(64 * 1024);
        let snapshot = snapshot.expect("snapshot should not be empty");
        assert!(
            snapshot.windows(4).any(|w| w == b"HIST"),
            "the atomic snapshot lost the history that get_recent_raw could see"
        );

        // 訂閱之後才產生的東西只能來自串流，不能已經在快照裡。
        #[cfg(windows)]
        session.write(b"echo AFTER\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"printf 'AFTER\\n'\n").unwrap();

        assert!(
            !snapshot.windows(5).any(|w| w == b"AFTER"),
            "the snapshot somehow contains output produced after it was taken"
        );

        let mut streamed = Vec::new();
        for _ in 0..50 {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(chunk)) => {
                    streamed.extend_from_slice(&chunk);
                    if streamed.windows(5).any(|w| w == b"AFTER") {
                        break;
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }
        assert!(
            streamed.windows(5).any(|w| w == b"AFTER"),
            "output produced after subscribing never arrived on the stream"
        );
    }
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --lib pty::session::tests::subscribe_with_history_loses_no_bytes_and_duplicates_none`
Expected: **編譯失敗**——`no method named subscribe_with_history`。

- [ ] **Step 3: 把 broadcast 送出移進 ring 鎖範圍內**

reader thread 裡目前是兩個分開的區塊。把寫 ring 的區塊與 broadcast 的區塊**合併成一個**，讓 broadcast 在 ring 鎖仍持有時發生：

```rust
                            {
                                let mut ring = ring_for_thread.lock();
                                for &b in &chunk {
                                    if ring.len() >= OUTPUT_RING_CAP { ring.pop_front(); }
                                    ring.push_back(b);
                                }
                                // Broadcast while still holding the ring lock.
                                // This is what makes `subscribe_with_history`
                                // atomic: a chunk is either already in the
                                // snapshot a subscriber took, or it cannot be
                                // broadcast until that subscriber has both its
                                // snapshot and its receiver. Neither gap nor
                                // duplicate is possible.
                                //
                                // `send` never blocks — it overwrites the
                                // oldest slot and the slow reader gets
                                // `Lagged` later — so holding the ring lock
                                // across it cannot stall the reader thread.
                                if output_tx_for_thread.receiver_count() > 0 {
                                    let _ = output_tx_for_thread.send(chunk.clone());
                                }
                            }
```

並把原本位於 `on_data(chunk);` 上方的那個獨立 broadcast 區塊**整個刪掉**（現在已經移進上面了）。

- [ ] **Step 4: 加上 `subscribe_with_history`**

加在 `session.rs` 的 `subscribe` 正下方：

```rust
    /// Take a history snapshot and subscribe to future output as one atomic
    /// step. Screen sharing must use this rather than calling `get_recent_raw`
    /// and `subscribe` separately.
    ///
    /// Those two take different locks, and the reader thread writes the ring
    /// before it broadcasts — so calling them in either order leaves a window:
    /// subscribe-then-snapshot duplicates the bytes that land in between, and
    /// snapshot-then-subscribe loses them entirely. Losing bytes can truncate
    /// an ANSI escape sequence, and a terminal that renders a truncated escape
    /// stays wrong forever.
    ///
    /// Holding the ring lock across both closes the window: a chunk is either
    /// already in the snapshot, or the reader thread cannot broadcast it until
    /// this method has returned with its receiver in hand.
    pub fn subscribe_with_history(
        &self,
        max_bytes: usize,
    ) -> (Option<Vec<u8>>, tokio::sync::broadcast::Receiver<Vec<u8>>) {
        let ring = self.output_ring.lock();
        let history = if ring.is_empty() {
            None
        } else {
            let start = ring.len().saturating_sub(max_bytes);
            Some(ring.iter().skip(start).copied().collect())
        };
        let rx = self.output_tx.subscribe();
        drop(ring);
        (history, rx)
    }
```

- [ ] **Step 5: 在 `PtyManager` 轉發**

加在 `manager.rs` 的 `subscribe` 正下方：

```rust
    /// Atomic snapshot-plus-subscribe for a session. See
    /// `PtySession::subscribe_with_history` for why sharing must use this
    /// instead of `get_recent_raw` followed by `subscribe`.
    pub fn subscribe_with_history(
        &self,
        id: &str,
        max_bytes: usize,
    ) -> Option<(Option<Vec<u8>>, tokio::sync::broadcast::Receiver<Vec<u8>>)> {
        self.sessions.lock().get(id).map(|s| s.subscribe_with_history(max_bytes))
    }
```

- [ ] **Step 6: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --lib pty::`
Expected: PASS，比 Task 3 結束時多一個測試（118）。

連跑 10 次新測試確認不 flaky：

```bash
cd src-tauri
for i in $(seq 1 10); do
  cargo test --lib pty::session::tests::subscribe_with_history_loses_no_bytes_and_duplicates_none -- --exact 2>&1 | grep -E "^test result" || echo "RUN $i FAILED"
done
```

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/pty/session.rs src-tauri/src/pty/manager.rs
git commit -m "feat(pty): make snapshot-plus-subscribe atomic for share viewers

分開呼叫 get_recent_raw 與 subscribe 會依順序造成重複或缺漏；缺漏可能截斷
ANSI 逃脫序列讓畫面永久錯亂。把 broadcast 移進 ring 鎖範圍即可兩者皆免。"
```

---

## Task 4: `ShareRegistry` 狀態機（純邏輯，不碰網路）

短碼、待審連線、觀看者名單、單一控制權的規則全部集中在這裡，不依賴 axum、TLS 或 PTY。這樣控制權那條「同時只能一人」的不變式可以被密集測試，不需要起伺服器。

**Files:**
- Create: `src-tauri/src/share/registry.rs`
- Create: `src-tauri/src/share/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: 加入 `rand` 直接依賴**

`Cargo.toml` 的 `[dependencies]` 區塊加入（`rand 0.9` 已在 Cargo.lock 裡，這只是提為直接依賴）：

```toml
# 共享短碼的亂數來源。已透過其他相依進入 Cargo.lock，這裡只是提為直接依賴。
rand = "0.9"
```

- [ ] **Step 2: 建立模組骨架**

建立 `src-tauri/src/share/mod.rs`：

```rust
//! 遠端終端機共享：讓同區網的同事用一組 6 位短碼連進本機的某個終端機分頁，
//! 看畫面並（經同意後）接手控制。
//!
//! 與 `crate::bridge`（Claude Code 的 Anthropic API 相容層）和
//! `crate::mcp_server`（AITerm 對外的 MCP 工具 server）並列但**刻意分開**：
//! 那兩個綁 127.0.0.1，這一個綁區網介面。把路由混進 mcp_server 等於順手把
//! `execute_query`（允許任意 SQL）暴露到辦公室網路。
//!
//! 設計文件：`docs/superpowers/specs/2026-08-26-remote-terminal-sharing-design.md`

pub mod registry;
```

在 `src-tauri/src/lib.rs` 的模組宣告區（`pub mod enterprise;` 附近，維持字母序）加入：

```rust
pub mod share;
```

- [ ] **Step 3: 寫會紅的測試**

建立 `src-tauri/src/share/registry.rs`，先只寫測試模組：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn registry_with_one_share() -> (ShareRegistry, String) {
        let reg = ShareRegistry::new();
        let code = reg.start_share("tab-1".to_string());
        (reg, code)
    }

    #[test]
    fn a_share_code_is_six_digits() {
        let (_reg, code) = registry_with_one_share();
        assert_eq!(code.len(), 6, "code should be 6 chars, got {code:?}");
        assert!(
            code.chars().all(|c| c.is_ascii_digit()),
            "code should be all digits, got {code:?}"
        );
    }

    #[test]
    fn two_shares_never_collide_on_a_code() {
        let reg = ShareRegistry::new();
        let a = reg.start_share("tab-1".to_string());
        let b = reg.start_share("tab-2".to_string());
        assert_ne!(a, b);
    }

    #[test]
    fn an_unknown_code_resolves_to_nothing() {
        let (reg, code) = registry_with_one_share();
        // 短碼是亂數，所以不能寫死一個「一定不存在」的值——從真正的短碼推
        // 一個保證不同的出來。
        let bogus: String = code
            .chars()
            .map(|c| if c == '0' { '1' } else { '0' })
            .collect();
        assert_ne!(bogus, code, "test bug: bogus code must differ from the real one");
        assert_eq!(reg.tab_for_code(&bogus), None);
    }

    #[test]
    fn stopping_a_share_invalidates_its_code() {
        let (reg, code) = registry_with_one_share();
        assert_eq!(reg.tab_for_code(&code), Some("tab-1".to_string()));
        reg.stop_share("tab-1");
        assert_eq!(reg.tab_for_code(&code), None);
    }

    #[test]
    fn a_pending_request_is_not_yet_a_viewer() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).expect("code is live");
        assert_eq!(reg.viewers("tab-1").len(), 0);
        assert_eq!(reg.pending("tab-1").len(), 1);
        assert_eq!(reg.pending("tab-1")[0].request_id, req);
    }

    #[test]
    fn joining_with_a_dead_code_is_refused() {
        let (reg, code) = registry_with_one_share();
        reg.stop_share("tab-1");
        assert!(reg.request_join(&code, "Alice".to_string()).is_none());
    }

    #[test]
    fn approving_read_only_adds_a_viewer_without_control() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        let viewer = reg.approve(&req, AccessMode::ReadOnly).expect("approve");
        assert_eq!(reg.viewers("tab-1").len(), 1);
        assert_eq!(reg.pending("tab-1").len(), 0);
        assert!(!reg.may_send_input("tab-1", &viewer));
    }

    #[test]
    fn approving_with_control_lets_that_viewer_send_input() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        let viewer = reg.approve(&req, AccessMode::Control).expect("approve");
        assert!(reg.may_send_input("tab-1", &viewer));
    }

    #[test]
    fn denying_a_request_leaves_no_viewer_and_no_pending() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        reg.deny(&req);
        assert_eq!(reg.viewers("tab-1").len(), 0);
        assert_eq!(reg.pending("tab-1").len(), 0);
    }

    #[test]
    fn control_is_a_single_microphone() {
        let (reg, code) = registry_with_one_share();
        let r1 = reg.request_join(&code, "Alice".to_string()).unwrap();
        let alice = reg.approve(&r1, AccessMode::Control).unwrap();
        let r2 = reg.request_join(&code, "Bob".to_string()).unwrap();

        // Bob cannot be approved with control while Alice holds it.
        assert_eq!(reg.approve(&r2, AccessMode::Control), None);

        // Read-only is fine, and he still cannot type.
        let bob = reg.approve(&r2, AccessMode::ReadOnly).expect("read-only approve");
        assert!(reg.may_send_input("tab-1", &alice));
        assert!(!reg.may_send_input("tab-1", &bob));
    }

    #[test]
    fn control_must_be_revoked_before_it_can_be_granted_to_someone_else() {
        let (reg, code) = registry_with_one_share();
        let r1 = reg.request_join(&code, "Alice".to_string()).unwrap();
        let alice = reg.approve(&r1, AccessMode::Control).unwrap();
        let r2 = reg.request_join(&code, "Bob".to_string()).unwrap();
        let bob = reg.approve(&r2, AccessMode::ReadOnly).unwrap();

        assert!(!reg.grant_control("tab-1", &bob), "must refuse while Alice holds it");
        reg.revoke_control("tab-1");
        assert!(!reg.may_send_input("tab-1", &alice));
        assert!(reg.grant_control("tab-1", &bob));
        assert!(reg.may_send_input("tab-1", &bob));
    }

    #[test]
    fn removing_the_controller_releases_control() {
        let (reg, code) = registry_with_one_share();
        let r1 = reg.request_join(&code, "Alice".to_string()).unwrap();
        let alice = reg.approve(&r1, AccessMode::Control).unwrap();
        reg.remove_viewer("tab-1", &alice);
        assert_eq!(reg.viewers("tab-1").len(), 0);

        // Control is now free for the next viewer.
        let r2 = reg.request_join(&code, "Bob".to_string()).unwrap();
        let bob = reg.approve(&r2, AccessMode::Control).expect("control should be free");
        assert!(reg.may_send_input("tab-1", &bob));
    }

    #[test]
    fn stopping_a_share_drops_every_viewer() {
        let (reg, code) = registry_with_one_share();
        let r1 = reg.request_join(&code, "Alice".to_string()).unwrap();
        let alice = reg.approve(&r1, AccessMode::Control).unwrap();
        reg.stop_share("tab-1");
        assert_eq!(reg.viewers("tab-1").len(), 0);
        assert!(!reg.may_send_input("tab-1", &alice));
    }

    #[test]
    fn input_from_an_unknown_viewer_is_refused() {
        let (reg, _code) = registry_with_one_share();
        assert!(!reg.may_send_input("tab-1", "no-such-viewer"));
    }

    #[test]
    fn an_approved_request_can_be_mapped_back_to_its_viewer() {
        // ws handler 手上只有 request_id；approve 的回傳值是給主控端 UI 的。
        // 沒有這條對應，連線就不知道自己變成了哪一位觀看者。
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        let viewer = reg.approve(&req, AccessMode::ReadOnly).unwrap();
        assert_eq!(reg.viewer_for_request("tab-1", &req), Some(viewer));
    }

    #[test]
    fn a_denied_request_maps_to_no_viewer() {
        let (reg, code) = registry_with_one_share();
        let req = reg.request_join(&code, "Alice".to_string()).unwrap();
        reg.deny(&req);
        assert_eq!(reg.viewer_for_request("tab-1", &req), None);
    }

    #[test]
    fn a_share_is_live_only_while_it_has_a_code() {
        let reg = ShareRegistry::new();
        assert!(!reg.any_active());
        let _ = reg.start_share("tab-1".to_string());
        assert!(reg.any_active());
        reg.stop_share("tab-1");
        assert!(!reg.any_active());
    }
}
```

- [ ] **Step 4: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --lib share::registry`
Expected: **編譯失敗**——`cannot find type ShareRegistry`、`cannot find type AccessMode`。

- [ ] **Step 5: 實作 `ShareRegistry`**

把以下內容加到 `src-tauri/src/share/registry.rs` 的**最上方**（測試模組之前）：

```rust
//! 共享狀態機：短碼、待審連線、觀看者名單、單一控制權。
//!
//! 刻意不碰網路、TLS 或 PTY——控制權的不變式（同時只有一人）值得被密集
//! 測試，起伺服器才能測會讓那些測試又慢又脆。

use std::collections::HashMap;

use parking_lot::Mutex;
use rand::Rng;
use uuid::Uuid;

/// 觀看者被授予的存取層級。由主控端在同意視窗上當場選擇，不走「先唯讀再
/// 請求控制」的兩段式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    ReadOnly,
    Control,
}

/// 一筆等待主控端裁決的連線請求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub request_id: String,
    pub tab_id: String,
    /// 請求方自報的名字。**未經驗證**，僅供主控端辨識用；真正的身分保證來自
    /// SAS 人工核對（見 `share::tls`）。
    pub display_name: String,
}

/// 一位已獲准的觀看者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub viewer_id: String,
    pub display_name: String,
    pub mode: AccessMode,
    /// 產生這位觀看者的那筆 `request_id`。ws handler 手上只有 request_id
    /// （`approve` 的回傳值是給主控端 UI 的），靠這個欄位把兩者對起來。
    pub from_request: String,
}

struct SharedTab {
    code: String,
    viewers: Vec<Viewer>,
    /// 目前持有控制權的 `viewer_id`。一支麥克風：同時最多一人。
    controller: Option<String>,
}

#[derive(Default)]
pub struct ShareRegistry {
    /// tab_id → 該分頁的共享狀態。
    tabs: Mutex<HashMap<String, SharedTab>>,
    /// request_id → 待審請求。
    pending: Mutex<HashMap<String, PendingRequest>>,
}

impl ShareRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 開始分享一個分頁，回傳新產生的 6 位短碼。同一個分頁重複呼叫會換一組
    /// 新短碼（舊的立即失效）。
    pub fn start_share(&self, tab_id: String) -> String {
        let mut tabs = self.tabs.lock();
        let code = loop {
            let candidate = format!("{:06}", rand::rng().random_range(0..1_000_000u32));
            if !tabs.values().any(|t| t.code == candidate) {
                break candidate;
            }
        };
        tabs.insert(
            tab_id,
            SharedTab { code: code.clone(), viewers: Vec::new(), controller: None },
        );
        code
    }

    /// 停止分享：短碼作廢、觀看者全數移除、控制權釋放。
    pub fn stop_share(&self, tab_id: &str) {
        self.tabs.lock().remove(tab_id);
        self.pending.lock().retain(|_, p| p.tab_id != tab_id);
    }

    /// 是否還有任何分頁在分享中。共享 server 靠這個決定要不要關閉自己。
    pub fn any_active(&self) -> bool {
        !self.tabs.lock().is_empty()
    }

    /// 短碼對應到哪個分頁；短碼不存在或已作廢時回 `None`。
    pub fn tab_for_code(&self, code: &str) -> Option<String> {
        self.tabs
            .lock()
            .iter()
            .find(|(_, t)| t.code == code)
            .map(|(id, _)| id.clone())
    }

    /// 用短碼發起一筆待審請求。回傳 `request_id`，或在短碼無效時回 `None`。
    /// **這一步不會讓對方看到任何東西**——要等主控端 `approve`。
    pub fn request_join(&self, code: &str, display_name: String) -> Option<String> {
        let tab_id = self.tab_for_code(code)?;
        let request_id = Uuid::new_v4().to_string();
        self.pending.lock().insert(
            request_id.clone(),
            PendingRequest { request_id: request_id.clone(), tab_id, display_name },
        );
        Some(request_id)
    }

    /// 某分頁目前所有待審請求。
    pub fn pending(&self, tab_id: &str) -> Vec<PendingRequest> {
        self.pending.lock().values().filter(|p| p.tab_id == tab_id).cloned().collect()
    }

    /// 某分頁目前所有觀看者。
    pub fn viewers(&self, tab_id: &str) -> Vec<Viewer> {
        self.tabs.lock().get(tab_id).map(|t| t.viewers.clone()).unwrap_or_default()
    }

    /// 核准一筆待審請求，回傳新的 `viewer_id`。
    ///
    /// 以 `AccessMode::Control` 核准時，若控制權已被別人持有則**整筆核准失敗**
    /// 並回 `None`——不靜默降級成唯讀，那會讓主控端以為自己給出了控制權。
    pub fn approve(&self, request_id: &str, mode: AccessMode) -> Option<String> {
        let request = self.pending.lock().remove(request_id)?;
        let mut tabs = self.tabs.lock();
        let tab = match tabs.get_mut(&request.tab_id) {
            Some(t) => t,
            None => return None, // 分享在裁決前就被停掉了
        };
        if mode == AccessMode::Control && tab.controller.is_some() {
            // 放回待審，讓主控端能改用唯讀重新裁決。
            drop(tabs);
            self.pending.lock().insert(request_id.to_string(), request);
            return None;
        }
        let viewer_id = Uuid::new_v4().to_string();
        tab.viewers.push(Viewer {
            viewer_id: viewer_id.clone(),
            display_name: request.display_name,
            mode,
            from_request: request_id.to_string(),
        });
        if mode == AccessMode::Control {
            tab.controller = Some(viewer_id.clone());
        }
        Some(viewer_id)
    }

    /// 找出某筆已核准請求所產生的觀看者 id。ws handler 在等待裁決的迴圈裡靠
    /// 這支判斷「我被核准了嗎」——請求離開待審名單後，查得到觀看者就是核准，
    /// 查不到就是拒絕。
    pub fn viewer_for_request(&self, tab_id: &str, request_id: &str) -> Option<String> {
        self.tabs
            .lock()
            .get(tab_id)?
            .viewers
            .iter()
            .find(|v| v.from_request == request_id)
            .map(|v| v.viewer_id.clone())
    }

    /// 拒絕一筆待審請求。
    pub fn deny(&self, request_id: &str) {
        self.pending.lock().remove(request_id);
    }

    /// 把控制權交給一位既有的唯讀觀看者。控制權已被持有時回 `false`——必須先
    /// `revoke_control`，避免「轉移」變成無聲地把前一個人踢下台。
    pub fn grant_control(&self, tab_id: &str, viewer_id: &str) -> bool {
        let mut tabs = self.tabs.lock();
        let Some(tab) = tabs.get_mut(tab_id) else { return false };
        if tab.controller.is_some() {
            return false;
        }
        let Some(v) = tab.viewers.iter_mut().find(|v| v.viewer_id == viewer_id) else {
            return false;
        };
        v.mode = AccessMode::Control;
        tab.controller = Some(viewer_id.to_string());
        true
    }

    /// 收回目前的控制權（若有）。持有者降為唯讀，仍留在觀看者名單裡。
    pub fn revoke_control(&self, tab_id: &str) {
        let mut tabs = self.tabs.lock();
        let Some(tab) = tabs.get_mut(tab_id) else { return };
        if let Some(current) = tab.controller.take() {
            if let Some(v) = tab.viewers.iter_mut().find(|v| v.viewer_id == current) {
                v.mode = AccessMode::ReadOnly;
            }
        }
    }

    /// 移除一位觀看者。若他正持有控制權，控制權一併釋放。
    pub fn remove_viewer(&self, tab_id: &str, viewer_id: &str) {
        let mut tabs = self.tabs.lock();
        let Some(tab) = tabs.get_mut(tab_id) else { return };
        tab.viewers.retain(|v| v.viewer_id != viewer_id);
        if tab.controller.as_deref() == Some(viewer_id) {
            tab.controller = None;
        }
    }

    /// 這位觀看者送來的按鍵可不可以寫進 PTY。
    ///
    /// 這是**伺服器端**的授權檢查，不是 UI 提示。唯讀端理應根本不送輸入，但
    /// 那是對方程式的自律，不能當成安全邊界。
    pub fn may_send_input(&self, tab_id: &str, viewer_id: &str) -> bool {
        self.tabs
            .lock()
            .get(tab_id)
            .and_then(|t| t.controller.clone())
            .as_deref()
            == Some(viewer_id)
    }
}
```

- [ ] **Step 6: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --lib share::registry`
Expected: PASS，17 個測試全過。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/share/ src-tauri/src/lib.rs src-tauri/Cargo.toml
git commit -m "feat(share): add the share registry state machine

短碼、待審連線、觀看者、單一控制權；純邏輯不碰網路。"
```

---

## Task 5: ws 線上格式（`protocol.rs`）

先把線上格式釘死再寫 server——計畫②的前端要照這份契約實作，型別漂掉會兩邊對不上。

**Files:**
- Create: `src-tauri/src/share/protocol.rs`
- Modify: `src-tauri/src/share/mod.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: 打開 axum 的 `ws` feature**

`Cargo.toml` 把：

```toml
axum = "0.8"
```

改成：

```toml
# `ws` 不在 axum 的 default features 裡（default = form/http1/json/
# matched-path/original-uri/query/tokio/tower-log/tracing），遠端終端機共享
# 需要雙向串流，所以顯式打開。不引入新 crate。
axum = { version = "0.8", features = ["ws"] }
```

- [ ] **Step 2: 寫會紅的測試**

建立 `src-tauri/src/share/protocol.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_round_trips_through_json() {
        let msg = ClientMessage::Join {
            code: "384719".to_string(),
            display_name: "Alice".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"join\""), "got {json}");
        let back: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn granted_carries_the_access_mode_and_the_host_screen_size() {
        let msg = ServerMessage::Granted {
            mode: WireAccessMode::ReadOnly,
            cols: 120,
            rows: 40,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"granted\""), "got {json}");
        assert!(json.contains("\"mode\":\"read_only\""), "got {json}");
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn ended_carries_a_machine_readable_reason() {
        let msg = ServerMessage::Ended { reason: EndReason::HostStoppedSharing };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"reason\":\"host_stopped_sharing\""), "got {json}");
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn every_end_reason_survives_a_round_trip() {
        // 前端要靠 reason 決定顯示哪一句話，漏掉任何一個都會變成「未知錯誤」。
        for reason in [
            EndReason::Denied,
            EndReason::HostStoppedSharing,
            EndReason::SessionClosed,
            EndReason::KickedByHost,
            EndReason::InvalidCode,
        ] {
            let msg = ServerMessage::Ended { reason };
            let json = serde_json::to_string(&msg).unwrap();
            let back: ServerMessage = serde_json::from_str(&json).unwrap();
            assert_eq!(back, msg, "round trip failed for {reason:?}");
        }
    }
}
```

- [ ] **Step 3: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --lib share::protocol`
Expected: **編譯失敗**——`cannot find type ClientMessage` 等。

- [ ] **Step 4: 實作線上格式**

加到 `protocol.rs` 最上方：

```rust
//! 共享連線的線上格式。
//!
//! 分工：**文字 frame 走這裡的 JSON 控制訊息，二進位 frame 就是原始位元組**
//! ——server→viewer 的二進位是 PTY 輸出，viewer→server 的二進位是按鍵。
//! PTY 位元組不套 JSON/base64，因為那會讓每個 chunk 膨脹並多一次配置。
//!
//! 計畫②的前端要照這份契約實作，改動要同步兩邊。

use serde::{Deserialize, Serialize};

use super::registry::AccessMode;

/// `AccessMode` 的線上表示。刻意與內部型別分開：內部列舉改名不該悄悄變成
/// 破壞相容性的線上格式變更。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireAccessMode {
    ReadOnly,
    Control,
}

impl From<AccessMode> for WireAccessMode {
    fn from(m: AccessMode) -> Self {
        match m {
            AccessMode::ReadOnly => WireAccessMode::ReadOnly,
            AccessMode::Control => WireAccessMode::Control,
        }
    }
}

/// 連線結束的原因。前端靠這個決定顯示哪一句話，所以是機器可讀的列舉而不是
/// 自由文字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// 主控端按了拒絕。
    Denied,
    /// 主控端停止分享。
    HostStoppedSharing,
    /// 被分享的終端機自己結束了（shell 退出）。
    SessionClosed,
    /// 主控端單獨踢掉這位觀看者。
    KickedByHost,
    /// 短碼不存在或已作廢。
    InvalidCode,
}

/// 觀看端 → 主控端。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// 連上 ws 後的第一則訊息。在收到 `Granted` 之前，觀看端不會收到任何
    /// PTY 位元組。
    Join { code: String, display_name: String },
}

/// 主控端 → 觀看端。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// 請求已送達，正在等主控端裁決。觀看端此時顯示「等待對方同意」與自己
    /// 算出的 SAS。
    AwaitingApproval,
    /// 已獲准。`cols`/`rows` 是主控端的終端機尺寸——觀看端必須照這個建立
    /// xterm，不能用自己的視窗大小。緊接著會來一個二進位 frame 作為重播。
    Granted { mode: WireAccessMode, cols: u16, rows: u16 },
    /// 主控端 resize 了，觀看端重新 fit。
    Resize { cols: u16, rows: u16 },
    /// 控制權變動（被授予或被收回）。
    ControlChanged { mode: WireAccessMode },
    /// 觀看端落後太多，接下來的二進位 frame 是全量重播；收到這個要先清空
    /// 畫面再套用。漏掉的位元組可能截斷 ANSI 逃脫序列，帶著壞掉的畫面繼續
    /// 是不會自己好的。
    Resync,
    /// 連線結束。送出後 server 立即關閉這條 ws。
    Ended { reason: EndReason },
}
```

在 `share/mod.rs` 加入：

```rust
pub mod protocol;
```

- [ ] **Step 5: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --lib share::protocol`
Expected: PASS，4 個測試全過。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/share/ src-tauri/Cargo.toml
git commit -m "feat(share): define the share connection wire protocol

文字 frame 走 JSON 控制訊息，二進位 frame 是原始 PTY 位元組／按鍵。"
```

---

## Task 6: ws server（明文，綁 loopback）

先把串流與授權那條路徑做對並用整合測試釘住；TLS 在 Task 7 疊上去。**這個中間狀態只綁 `127.0.0.1`**，不會有任何未加密的區網監聽。

**Files:**
- Create: `src-tauri/src/share/server.rs`
- Modify: `src-tauri/src/share/mod.rs`
- Create: `src-tauri/tests/share_end_to_end.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: 加入測試用的 ws 客戶端**

`Cargo.toml` 的 `[dev-dependencies]` 加入：

```toml
# 整合測試用的 WebSocket 客戶端。僅 dev-dependency——正式程式碼只當 server。
tokio-tungstenite = "0.24"
futures-util = "0.3"
```

- [ ] **Step 2: 寫會紅的整合測試**

建立 `src-tauri/tests/share_end_to_end.rs`：

```rust
//! 共享連線的端到端測試：連線 → 待審 → 核准 → 重播 → 即時串流 →
//! 輸入授權 → 結束。
//!
//! 用真的 PTY（跑一個 shell）與真的 ws 連線，不是 mock——這條路徑的價值
//! 全在「真的接得起來」，用假的 PTY 測等於什麼都沒測。

use std::sync::Arc;
use std::time::Duration;

use aiterm_lib::pty::manager::PtyManager;
use aiterm_lib::share::registry::{AccessMode, ShareRegistry};
use aiterm_lib::share::protocol::{ClientMessage, EndReason, ServerMessage, WireAccessMode};
use futures_util::{SinkExt, StreamExt};
use portable_pty::PtySize;
use tokio_tungstenite::tungstenite::Message;

const SIZE: PtySize = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };

/// 起一個綁 127.0.0.1:0 的共享 server，回傳它的實際 port。
async fn start_test_server(
    pty: Arc<PtyManager>,
    registry: Arc<ShareRegistry>,
) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = aiterm_lib::share::server::router(pty, registry);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
}

/// 讀到下一則 JSON 控制訊息，跳過中間的二進位 frame。
async fn next_control<S>(ws: &mut S) -> ServerMessage
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    for _ in 0..200 {
        match tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                return serde_json::from_str(&t).expect("server sent malformed JSON")
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => panic!("ws error: {e}"),
            Ok(None) => panic!("ws closed while waiting for a control message"),
            Err(_) => continue,
        }
    }
    panic!("timed out waiting for a control message");
}

/// 累積二進位 frame 直到看到 `marker`，或逾時。
async fn collect_binary_until<S>(ws: &mut S, marker: &[u8]) -> Vec<u8>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut acc = Vec::new();
    for _ in 0..200 {
        match tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                acc.extend_from_slice(&b);
                if acc.windows(marker.len()).any(|w| w == marker) {
                    return acc;
                }
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => panic!("ws error: {e}"),
            Ok(None) => break,
            Err(_) => continue,
        }
    }
    acc
}

#[tokio::test]
async fn an_unknown_code_is_refused_without_reaching_the_host() {
    let pty = Arc::new(PtyManager::new());
    let registry = Arc::new(ShareRegistry::new());
    let port = start_test_server(pty, Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            code: "000000".to_string(),
            display_name: "Mallory".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Ended { reason: EndReason::InvalidCode }
    );
}

#[tokio::test]
async fn a_read_only_viewer_sees_output_but_cannot_type() {
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    // 分享前先在分頁裡留下一點歷史，之後要驗證它有被重播。
    #[cfg(windows)]
    pty.write(&tab_id, b"echo BEFORE\r\n").unwrap();
    #[cfg(not(windows))]
    pty.write(&tab_id, b"printf 'BEFORE\\n'\n").unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    assert_eq!(next_control(&mut ws).await, ServerMessage::AwaitingApproval);

    // 模擬主控端按下「只能看」。
    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    let viewer_id = registry.approve(&request_id, AccessMode::ReadOnly).expect("approve");

    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Granted { mode: WireAccessMode::ReadOnly, cols: 80, rows: 24 }
    );

    // 重播必須包含分享前就存在的歷史。
    let replayed = collect_binary_until(&mut ws, b"BEFORE").await;
    assert!(
        replayed.windows(6).any(|w| w == b"BEFORE"),
        "expected the pre-share history to be replayed"
    );

    // 唯讀端送輸入必須不會進到 PTY。
    ws.send(Message::Binary(b"echo HACKED\n".to_vec().into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let seen = pty.get_recent_output(&tab_id, 64 * 1024).unwrap_or_default();
    assert!(
        !seen.contains("HACKED"),
        "a read-only viewer's input reached the PTY: {seen}"
    );
    assert!(!registry.may_send_input(&tab_id, &viewer_id));
}

#[tokio::test]
async fn a_controlling_viewer_can_type_and_sees_the_result() {
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_control(&mut ws).await, ServerMessage::AwaitingApproval);

    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    registry.approve(&request_id, AccessMode::Control).expect("approve");
    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Granted { mode: WireAccessMode::Control, cols: 80, rows: 24 }
    );

    // 遠端打字，PTY 應該真的收到並回顯。
    #[cfg(windows)]
    ws.send(Message::Binary(b"echo REMOTE\r\n".to_vec().into())).await.unwrap();
    #[cfg(not(windows))]
    ws.send(Message::Binary(b"printf 'REMOTE\\n'\n".to_vec().into())).await.unwrap();

    let echoed = collect_binary_until(&mut ws, b"REMOTE").await;
    assert!(
        echoed.windows(6).any(|w| w == b"REMOTE"),
        "the controlling viewer's input never came back as output"
    );
}

#[tokio::test]
async fn stopping_the_share_ends_the_connection_with_a_reason() {
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_control(&mut ws).await, ServerMessage::AwaitingApproval);
    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    registry.approve(&request_id, AccessMode::ReadOnly).expect("approve");
    let _ = next_control(&mut ws).await; // Granted

    registry.stop_share(&tab_id);

    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Ended { reason: EndReason::HostStoppedSharing }
    );
}
```

**注意**：測試用 `aiterm_lib::` 當 crate 名稱。實作前先確認 `src-tauri/Cargo.toml` 的 `[lib] name`，並照既有整合測試（例如 `tests/mcp_tool_server.rs` 的開頭幾行）用同一個名稱；不一致就改成正確的那個。

- [ ] **Step 3: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --test share_end_to_end`
Expected: **編譯失敗**——`share::server` 不存在、`router` 未定義。

- [ ] **Step 4: 實作 ws server**

建立 `src-tauri/src/share/server.rs`：

```rust
//! 共享連線的 axum router 與 ws handler。
//!
//! 每條 ws 連線對應一位觀看者。連線後的握手順序是固定的：
//! `Join` → `AwaitingApproval` → （主控端裁決）→ `Granted` ＋ 重播 ＋ 串流，
//! 或 `Ended`。在 `Granted` 之前，觀看端拿不到任何一個 PTY 位元組。

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::{any, get};
use axum::Router;
use tokio::sync::broadcast::error::RecvError;

use crate::pty::manager::PtyManager;

use super::protocol::{ClientMessage, EndReason, ServerMessage, WireAccessMode};
use super::registry::ShareRegistry;

/// 重播給新連線觀看者的位元組上限。等於 PTY ring buffer 的容量——重播的意義
/// 就是「把 ring 裡有的全部給他」。
const REPLAY_MAX_BYTES: usize = 256 * 1024;

/// 輪詢主控端裁決與分享是否被停掉的間隔。裁決由人操作，秒級足夠；用輪詢而
/// 不是再開一條通知管道，是因為狀態機刻意不依賴任何非同步執行期。
const DECISION_POLL: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub struct ShareAppState {
    pub pty: Arc<PtyManager>,
    pub registry: Arc<ShareRegistry>,
}

pub fn router(pty: Arc<PtyManager>, registry: Arc<ShareRegistry>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/share", any(share_upgrade))
        .with_state(ShareAppState { pty, registry })
}

async fn share_upgrade(ws: WebSocketUpgrade, State(state): State<ShareAppState>) -> Response {
    ws.on_upgrade(move |socket| handle_share(socket, state))
}

async fn send_control(ws: &mut WebSocket, msg: &ServerMessage) -> bool {
    let Ok(json) = serde_json::to_string(msg) else { return false };
    ws.send(Message::Text(json.into())).await.is_ok()
}

async fn end_with(ws: &mut WebSocket, reason: EndReason) {
    send_control(ws, &ServerMessage::Ended { reason }).await;
    let _ = ws.close().await;
}

async fn handle_share(mut ws: WebSocket, state: ShareAppState) {
    // 1. 第一則訊息必須是 Join。
    let join = match ws.recv().await {
        Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientMessage>(&t) {
            Ok(ClientMessage::Join { code, display_name }) => (code, display_name),
            Err(_) => return end_with(&mut ws, EndReason::InvalidCode).await,
        },
        _ => return end_with(&mut ws, EndReason::InvalidCode).await,
    };
    let (code, display_name) = join;

    // 2. 短碼換待審請求。短碼無效就到此為止——主控端不會看到任何東西，所以
    //    亂猜短碼連「打擾對方」都做不到。
    let Some(request_id) = state.registry.request_join(&code, display_name) else {
        return end_with(&mut ws, EndReason::InvalidCode).await;
    };
    let Some(tab_id) = state.registry.tab_for_code(&code) else {
        return end_with(&mut ws, EndReason::InvalidCode).await;
    };

    if !send_control(&mut ws, &ServerMessage::AwaitingApproval).await {
        state.registry.deny(&request_id);
        return;
    }

    // 3. 等主控端裁決。刻意不設自動拒絕的逾時——使用者可能只是走開了，自動
    //    拒絕會讓他回來時毫無線索（見 spec 的錯誤處理）。觀看端要放棄的話
    //    自己關掉連線即可，那會讓下面的迴圈偵測到 ws 已死。
    let viewer_id = loop {
        // 分享在裁決前被停掉。
        if state.registry.tab_for_code(&code).is_none() {
            state.registry.deny(&request_id);
            return end_with(&mut ws, EndReason::HostStoppedSharing).await;
        }
        // 已經不在待審名單裡：不是被核准（下面查得到 viewer）就是被拒絕。
        let still_pending = state
            .registry
            .pending(&tab_id)
            .iter()
            .any(|p| p.request_id == request_id);
        if !still_pending {
            break match state.registry.viewer_for_request(&tab_id, &request_id) {
                Some(id) => id,
                None => return end_with(&mut ws, EndReason::Denied).await,
            };
        }
        tokio::time::sleep(DECISION_POLL).await;
    };

    // 4. 已獲准：先送尺寸與模式，再送重播，最後接即時串流。
    let mode: WireAccessMode = state
        .registry
        .viewers(&tab_id)
        .into_iter()
        .find(|v| v.viewer_id == viewer_id)
        .map(|v| v.mode.into())
        .unwrap_or(WireAccessMode::ReadOnly);
    let (cols, rows) = state.pty.size(&tab_id).unwrap_or((80, 24));

    if !send_control(&mut ws, &ServerMessage::Granted { mode, cols, rows }).await {
        state.registry.remove_viewer(&tab_id, &viewer_id);
        return;
    }

    // **一定要用 `subscribe_with_history`，不要分開呼叫 `subscribe` 與
    // `get_recent_raw`。** 那兩支用的是不同的鎖，不論哪個順序都會留下窗口：
    // 先訂閱會讓中間的 chunk 重複，先取快照會讓它整段消失——而消失可能截斷
    // 一段 ANSI 逃脫序列，畫面從此錯亂且不會自己好。見 Task 3b。
    let Some((history, mut rx)) =
        state.pty.subscribe_with_history(&tab_id, REPLAY_MAX_BYTES)
    else {
        state.registry.remove_viewer(&tab_id, &viewer_id);
        return end_with(&mut ws, EndReason::SessionClosed).await;
    };

    if let Some(history) = history {
        if ws.send(Message::Binary(history.into())).await.is_err() {
            state.registry.remove_viewer(&tab_id, &viewer_id);
            return;
        }
    }

    // 5. 串流迴圈：PTY 輸出下行、按鍵上行、分享狀態監看，三者併行。
    let mut share_watch = tokio::time::interval(DECISION_POLL);
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Ok(chunk) => {
                    if ws.send(Message::Binary(chunk.into())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    // 漏掉的位元組可能截斷 ANSI 逃脫序列——不能當沒事發生。
                    // 叫觀看端清空畫面，重新給他全量重播。
                    //
                    // 重新同步同樣要用 `subscribe_with_history`：這裡的窗口
                    // 跟首次連線時一模一樣，用 `get_recent_raw` 補快照而讓
                    // 舊的 rx 繼續收，中間的 chunk 一樣會重複或消失。取得
                    // 新的 receiver 後直接換掉舊的。
                    if !send_control(&mut ws, &ServerMessage::Resync).await {
                        break;
                    }
                    let Some((history, fresh_rx)) =
                        state.pty.subscribe_with_history(&tab_id, REPLAY_MAX_BYTES)
                    else {
                        end_with(&mut ws, EndReason::SessionClosed).await;
                        break;
                    };
                    rx = fresh_rx;
                    if let Some(history) = history {
                        if ws.send(Message::Binary(history.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Err(RecvError::Closed) => {
                    end_with(&mut ws, EndReason::SessionClosed).await;
                    break;
                }
            },
            incoming = ws.recv() => match incoming {
                Some(Ok(Message::Binary(keys))) => {
                    // 伺服器端授權檢查。唯讀端理應根本不送，但那是對方程式的
                    // 自律，不是安全邊界。
                    if state.registry.may_send_input(&tab_id, &viewer_id) {
                        let _ = state.pty.write(&tab_id, &keys);
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break,
            },
            _ = share_watch.tick() => {
                if state.registry.tab_for_code(&code).is_none() {
                    end_with(&mut ws, EndReason::HostStoppedSharing).await;
                    break;
                }
                let still_a_viewer = state
                    .registry
                    .viewers(&tab_id)
                    .iter()
                    .any(|v| v.viewer_id == viewer_id);
                if !still_a_viewer {
                    end_with(&mut ws, EndReason::KickedByHost).await;
                    break;
                }
            }
        }
    }

    state.registry.remove_viewer(&tab_id, &viewer_id);
}
```

- [ ] **Step 5: 補上 `PtyManager::size`**

server 需要主控端的終端機尺寸來告訴觀看端，`PtyManager` 目前沒有這支。先在 `session.rs` 加：

```rust
    /// Current terminal size, as last set by `resize` (or as spawned).
    /// Screen sharing sends this to viewers so they build their own terminal
    /// at the host's dimensions rather than their own window's.
    pub fn size(&self) -> (u16, u16) {
        let s = *self.size.lock();
        (s.cols, s.rows)
    }
```

這需要 `PtySession` 記住尺寸——目前沒有存。在結構加欄位 `size: Mutex<PtySize>`，`spawn_with_id` 建構時填入傳進來的 `size`，並在既有的 `resize`（`session.rs:743`）成功後更新它：

```rust
    pub fn resize(&self, size: PtySize) -> PtyResult<()> {
        let master = self.master.lock();
        master
            .resize(size)
            .map_err(|e| PtyError::Internal(format!("resize: {e}")))?;
        *self.size.lock() = size;
        Ok(())
    }
```

（上面的 `resize` 主體請照 `session.rs:743` 現有內容為準，只加最後那行 `*self.size.lock() = size;`。）

`manager.rs` 轉發：

```rust
    /// Current terminal size (cols, rows) for the given session.
    pub fn size(&self, id: &str) -> Option<(u16, u16)> {
        self.sessions.lock().get(id).map(|s| s.size())
    }
```

單元測試（加到 `session.rs` 測試模組）：

```rust
    #[test]
    fn size_reports_what_the_session_was_spawned_with_and_tracks_resize() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");
        assert_eq!(session.size(), (80, 24));

        session
            .resize(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
            .expect("resize");
        assert_eq!(session.size(), (120, 40));
    }
```

- [ ] **Step 6: 在 `share/mod.rs` 宣告 server 模組**

```rust
pub mod server;
```

- [ ] **Step 7: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --lib share:: && cargo test --lib pty:: && cargo test --test share_end_to_end`
Expected: 全部 PASS。

若 `a_read_only_viewer_sees_output_but_cannot_type` 偶發失敗，先確認不是 sleep 時間不夠——真實 PTY 有啟動延遲。**不要用加長 sleep 來掩蓋真正的失敗**：先把 `pty.get_recent_output` 印出來看 shell 到底吐了什麼。

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/share/ src-tauri/src/pty/ src-tauri/tests/share_end_to_end.rs src-tauri/Cargo.toml
git commit -m "feat(share): add the WebSocket share server with server-side input authorisation

握手、待審、重播、即時串流、Lagged 重新同步；唯讀端的輸入在伺服器端被丟棄。"
```

---

## Task 7: TLS 自簽憑證 ＋ 4 位 SAS

明文那條路已經被測試釘住，現在把它包進 TLS，並導出雙方各自可算的 4 位驗證碼。中間人必須維持兩條獨立的 TLS 連線，兩邊導出的值必然不同，對不上就是有人在中間。

**Files:**
- Create: `src-tauri/src/share/tls.rs`
- Modify: `src-tauri/src/share/mod.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: 加入依賴**

`Cargo.toml` 的 `[dependencies]`：

```toml
# 遠端終端機共享的傳輸加密。rustls / tokio-rustls 已經透過 reqwest 的
# rustls-tls 進入 Cargo.lock，這裡只是提為直接依賴；rcgen 是新的，用來產生
# 每次分享的臨時自簽憑證。
#
# `default-features = false` + `ring` 不可省略：rustls 0.23 與 tokio-rustls
# 0.26 的預設供應者都是 aws_lc_rs，那會拉進 aws-lc-sys（C／組語建置，Windows
# 要 nasm+cmake）。本專案的樹裡目前只有 ring（reqwest 帶進來的），照預設寫
# 會憑空多一個 C 相依並很可能弄壞 Windows CI。
rustls = { version = "0.23", default-features = false, features = ["ring", "std", "tls12", "logging"] }
tokio-rustls = { version = "0.26", default-features = false, features = ["ring", "tls12", "logging"] }
rcgen = "0.13"
```

驗證這一步沒有意外拉進 aws-lc：

```bash
cd src-tauri && cargo tree -i aws-lc-sys 2>&1 | head -5
```

Expected: `error: package ID specification ... did not match any packages`（也就是根本沒這個套件）。若它出現在依賴樹裡，回頭檢查上面的 feature 設定。

**另外**：rustls 0.23 需要一個行程層級的預設加密供應者，否則 `ClientConfig::builder()` / `ServerConfig::builder()` 會 panic。在 `share/mod.rs` 加入並在 `start_if_needed` 最前面呼叫：

```rust
/// rustls 0.23 要求行程層級的預設加密供應者。裝一次就好；重複呼叫會回
/// `Err`，直接忽略——那代表別人已經裝過了，不是錯誤。
fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
```

- [ ] **Step 2: 寫會紅的測試**

建立 `src-tauri/src/share/tls.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_cert_has_a_private_key_and_a_der_chain() {
        let ident = ShareIdentity::generate().expect("generate");
        assert!(!ident.cert_der.is_empty());
        assert!(!ident.key_der.secret_der().is_empty());
    }

    #[test]
    fn two_identities_are_different() {
        // 每次分享產生一組新的臨時憑證——重用會讓不同場次的連線可以被關聯。
        let a = ShareIdentity::generate().expect("a");
        let b = ShareIdentity::generate().expect("b");
        assert_ne!(a.cert_der, b.cert_der);
    }

    #[test]
    fn the_same_exporter_bytes_give_the_same_four_digit_code() {
        let material = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let a = sas_from_exporter(&material);
        let b = sas_from_exporter(&material);
        assert_eq!(a, b);
        assert_eq!(a.len(), 4, "SAS should be 4 chars, got {a:?}");
        assert!(a.chars().all(|c| c.is_ascii_digit()), "got {a:?}");
    }

    #[test]
    fn different_exporter_bytes_give_a_different_code() {
        // 這是整個防中間人設計的支點：中間人的兩條 TLS 連線導出不同的
        // material，所以兩邊畫面上的數字對不起來。
        let a = sas_from_exporter(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        let b = sas_from_exporter(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x89]);
        assert_ne!(a, b);
    }

    #[test]
    fn the_sas_label_is_pinned() {
        // 標籤是連線雙方必須一致的常數。改動它會讓新舊版本算出不同的 SAS，
        // 使用者會看到「對不上」而以為被攻擊——所以固定住並讓改動很明顯。
        assert_eq!(SAS_EXPORTER_LABEL, b"aiterm share sas v1");
    }
}
```

- [ ] **Step 3: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --lib share::tls`
Expected: **編譯失敗**——`cannot find type ShareIdentity` 等。

- [ ] **Step 4: 實作**

加到 `tls.rs` 最上方：

```rust
//! 共享連線的傳輸安全。
//!
//! 兩件事：
//! 1. **加密**——每次分享產生一組臨時自簽憑證，連線走 TLS 1.3。
//! 2. **防冒充**——自簽憑證本身擋不住中間人（觀看端沒有任何先驗資訊可判斷
//!    該信任哪張憑證），所以身分保證來自 **SAS 人工核對**：雙方各自從 TLS
//!    連線導出（RFC 5705）同一份金鑰 material，算成 4 位數顯示在畫面上，由
//!    使用者口頭核對。中間人必須維持兩條獨立的 TLS 連線，導出的 material 必
//!    然不同，數字就對不起來。
//!
//! 為什麼不用 PAKE（SPAKE2）：那樣可以省掉人工核對，但 `spake2` crate 目前
//! 只有 pre-release 版（0.5.0-pre.0），不適合放在安全關鍵路徑上。rustls 已
//! 經在依賴樹裡。見設計文件的「安全契約」。

use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// RFC 5705 匯出用的標籤。**連線雙方必須完全一致**——不一致會算出不同的
/// SAS，使用者會看到「對不上」而誤以為遭到攻擊。版本號在字串裡，未來要改
/// 演算法時連同標籤一起換。
pub const SAS_EXPORTER_LABEL: &[u8] = b"aiterm share sas v1";

/// 從匯出的金鑰 material 取幾個位元組來算 SAS。
const SAS_MATERIAL_LEN: usize = 32;

/// 一次分享用的臨時 TLS 身分。分享停止就丟棄——重用會讓不同場次的連線可以
/// 被外部關聯起來。
pub struct ShareIdentity {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
}

impl ShareIdentity {
    /// 產生一組新的自簽憑證。CN 沒有意義（觀看端不驗證它，身分來自 SAS），
    /// 但仍填一個可辨識的值方便除錯。
    pub fn generate() -> anyhow::Result<Self> {
        let cert = rcgen::generate_simple_self_signed(vec!["aiterm-share".to_string()])?;
        Ok(Self {
            cert_der: CertificateDer::from(cert.cert.der().to_vec()),
            key_der: PrivateKeyDer::try_from(cert.signing_key.serialize_der())
                .map_err(|e| anyhow::anyhow!("serialise share key: {e}"))?,
        })
    }
}

/// 把 TLS 匯出的金鑰 material 折成 4 位數字。
///
/// 4 位數（1/10000）不是用來抵擋離線暴力破解的——它只需要讓「即時的中間人
/// 攻擊」在人工核對這一關被抓到，而攻擊者沒有重試的機會：核對失敗使用者就
/// 不會按同意。
pub fn sas_from_exporter(material: &[u8]) -> String {
    // 折疊整段 material，讓任何一個位元組的差異都會影響結果。
    let mut acc: u32 = 0;
    for (i, b) in material.iter().enumerate() {
        acc = acc
            .wrapping_mul(31)
            .wrapping_add((*b as u32).wrapping_add(i as u32));
    }
    format!("{:04}", acc % 10_000)
}

/// 從一條已完成握手的 TLS 連線導出這一端的 SAS。
///
/// `conn` 是 `rustls::ServerConnection` 或 `rustls::ClientConnection`——兩者
/// 都 deref 到 `ConnectionCommon`，`export_keying_material` 就在上面
/// （rustls 0.23 `src/conn.rs:460`）。握手完成前呼叫會失敗，所以呼叫端必須
/// 在握手之後才叫。
pub fn sas_for_connection(
    export: impl FnOnce([u8; SAS_MATERIAL_LEN], &[u8], Option<&[u8]>) -> Result<[u8; SAS_MATERIAL_LEN], rustls::Error>,
) -> anyhow::Result<String> {
    let material = export([0u8; SAS_MATERIAL_LEN], SAS_EXPORTER_LABEL, None)
        .map_err(|e| anyhow::anyhow!("export keying material: {e}"))?;
    Ok(sas_from_exporter(&material))
}
```

- [ ] **Step 5: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --lib share::tls`
Expected: PASS，5 個測試全過。

若 `rcgen 0.13` 的 API 名稱與上面不符（`cert.signing_key` 在不同小版本曾叫 `key_pair`），以 `cargo doc --open -p rcgen` 或 `~/.cargo/registry/src/.../rcgen-0.13.*/src/lib.rs` 的實際簽名為準修正，**不要憑記憶硬套**。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/share/tls.rs src-tauri/src/share/mod.rs src-tauri/Cargo.toml
git commit -m "feat(share): add ephemeral TLS identity and SAS derivation

自簽臨時憑證負責加密，4 位 SAS 人工核對負責防中間人。"
```

---

## Task 8: 把 TLS 接上 server，並綁區網介面

**Files:**
- Modify: `src-tauri/src/share/server.rs`
- Modify: `src-tauri/src/share/mod.rs`
- Modify: `src-tauri/tests/share_end_to_end.rs`

- [ ] **Step 1: 寫會紅的測試**

加到 `tests/share_end_to_end.rs`：

```rust
/// 測試用的客戶端憑證驗證器：接受任何憑證。
///
/// 這在這裡**不是偷懶**——正式的觀看端也必須這樣做。自簽憑證沒有任何憑證鏈
/// 可以驗，身分保證完全來自 SAS 人工核對。把這件事寫成一個有名字的型別，是
/// 為了讓它在程式碼審查時顯眼，而不是藏在一個 `danger` 呼叫裡。
#[derive(Debug)]
struct SasIsTheOnlyIdentityCheck;

impl rustls::client::danger::ServerCertVerifier for SasIsTheOnlyIdentityCheck {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[tokio::test]
async fn both_ends_of_a_real_tls_connection_derive_the_same_sas() {
    // 這是防中間人設計的支點，必須用真的 TLS 握手驗證，不能只測折疊函式：
    // 中間人的兩條連線導出不同的 material，兩邊數字對不起來——但那個保證
    // 只有在「雙方各自從自己的連線導出」時才成立。
    let _ = rustls::crypto::ring::default_provider().install_default();

    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let state = aiterm_lib::share::ShareServerState::new();
    let code = state.registry.start_share(tab_id.clone());
    let port = state
        .start_if_needed(Arc::clone(&pty))
        .await
        .expect("start share server");

    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("tcp connect");

    let mut cfg = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SasIsTheOnlyIdentityCheck))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];

    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let server_name = rustls::pki_types::ServerName::try_from("aiterm-share").unwrap();
    let tls = connector.connect(server_name, tcp).await.expect("tls handshake");

    // 握手完成後、把 stream 交給 tungstenite 之前，先算自己這端的 SAS。
    let client_sas = {
        let (_io, conn) = tls.get_ref();
        aiterm_lib::share::tls::sas_for_connection(|buf, label, ctx| {
            conn.export_keying_material(buf, label, ctx)
        })
        .expect("client SAS")
    };

    let (mut ws, _) = tokio_tungstenite::client_async("ws://aiterm-share/share", tls)
        .await
        .expect("ws handshake");

    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    // server 把它自己那端導出的 SAS 放進 AwaitingApproval。兩端必須一致。
    match next_control(&mut ws).await {
        ServerMessage::AwaitingApproval { sas } => {
            assert_eq!(
                sas, client_sas,
                "both ends of the same TLS connection must derive the same SAS"
            );
            assert_eq!(sas.len(), 4);
        }
        other => panic!("expected AwaitingApproval, got {other:?}"),
    }
}
```

**注意**：這個測試示範的客戶端 TLS 設定（自訂驗證器 ＋ 先算 SAS 再交給 tungstenite）就是計畫②觀看端要用的那一套。計畫②實作觀看端時，把這段抽成 `share` 裡的正式函式，不要在前端重寫一次。

- [ ] **Step 2: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --test share_end_to_end both_ends_of_a_real_tls_connection_derive_the_same_sas`
Expected: **編譯失敗**——`ShareServerState` 尚未定義、`ServerMessage::AwaitingApproval` 還沒有 `sas` 欄位。

- [ ] **Step 3: 實作 TLS 接受迴圈與客戶端**

在 `share/mod.rs` 加入 `ShareServerState`（鏡像 `mcp_server::McpToolServerState`，但綁的是**所有介面**而非 loopback）以及測試輔助函式。完整內容：

```rust
pub mod protocol;
pub mod registry;
pub mod server;
pub mod tls;

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::pty::manager::PtyManager;
use registry::ShareRegistry;

/// Server 生命週期。鏡像 `mcp_server::McpToolServerState`，但有兩個關鍵差異：
/// 綁的是 `0.0.0.0`（區網可達）而不是 `127.0.0.1`，而且**只在有分頁正在分享
/// 時存在**——最後一個分享停止就關閉，不留常駐監聽。
pub struct ShareServerState {
    running: Mutex<Option<Running>>,
    pub registry: Arc<ShareRegistry>,
}

struct Running {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl Default for ShareServerState {
    fn default() -> Self {
        Self { running: Mutex::new(None), registry: Arc::new(ShareRegistry::new()) }
    }
}

impl ShareServerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.port)
    }

    /// 啟動 server（若尚未啟動）。綁 `0.0.0.0:0` 讓 OS 挑一個空閒 port——與
    /// bridge/mcp_server 不同，這裡沒有外部設定檔記著位址，所以浮動 port 不
    /// 會讓任何東西指向死地址。
    pub async fn start_if_needed(&self, pty: Arc<PtyManager>) -> anyhow::Result<u16> {
        if let Some(p) = self.port() {
            return Ok(p);
        }
        ensure_crypto_provider();
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await?;
        let port = listener.local_addr()?.port();
        let app = server::router(pty, Arc::clone(&self.registry));
        let identity = tls::ShareIdentity::generate()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        // 自己的 accept 迴圈而不是 axum::serve——見下方「TLS 的接線」。
        tokio::spawn(serve_tls(listener, app, identity, rx));
        *self.running.lock() = Some(Running { port, shutdown: tx });
        Ok(port)
    }

    /// 沒有分頁在分享時關閉 server。呼叫端在每次 `stop_share` 之後叫這支。
    pub fn stop_if_idle(&self) {
        if self.registry.any_active() {
            return;
        }
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
        }
    }
}
```

**TLS 的接線**：`axum::serve` 不直接吃 TLS，而且我們需要**每條連線各自的** TLS 狀態才能導出 SAS，所以要自己寫 accept 迴圈。先在 `Cargo.toml` 的 `[dependencies]` 補上（三者都已在 Cargo.lock，只是提為直接依賴）：

```toml
# 共享 server 的 TLS accept 迴圈：axum::serve 不吃 TLS，而且導出 SAS 需要
# 每條連線各自的 rustls 狀態，所以自己接。
hyper = { version = "1", features = ["server", "http1"] }
hyper-util = { version = "0.1", features = ["server", "server-auto", "tokio"] }
tower-service = "0.3"
```

在 `share/mod.rs` 加上（`start_if_needed` 上面已經呼叫它了）：

```rust
use axum::Router;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HyperBuilder;
use tokio_rustls::TlsAcceptor;
use tower_service::Service;

/// 一條 TLS 連線導出的 SAS，透過 request extension 交給 ws handler。
/// 每條連線一組——這正是它能當身分保證的原因。
#[derive(Clone, Debug)]
pub struct ConnectionSas(pub String);

/// TLS accept 迴圈。每條連線握手完成後先導出 SAS，塞進 request extension，
/// 再交給 axum router。
///
/// 用 `serve_connection_with_upgrades`（不是 `serve_connection`）——WebSocket
/// 是 HTTP upgrade，用錯那支的話升級請求會被當成普通請求處理，ws 永遠接不起來。
async fn serve_tls(
    listener: tokio::net::TcpListener,
    app: Router,
    identity: tls::ShareIdentity,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let server_config = match rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![identity.cert_der.clone()], identity.key_der.clone_key())
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("共享 server TLS 設定失敗：{e}");
            return;
        }
    };
    let acceptor = TlsAcceptor::from(Arc::new(server_config));

    loop {
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((s, _peer)) => s,
                Err(e) => {
                    log::error!("共享 server accept 失敗：{e}");
                    continue;
                }
            },
            _ = &mut shutdown => break,
        };

        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let Ok(tls_stream) = acceptor.accept(stream).await else {
                // 握手失敗（對方不講 TLS、或憑證被拒）——安靜放掉這條連線。
                return;
            };

            // 握手已完成，可以導出金鑰 material 了。握手前呼叫會失敗。
            let sas = {
                let (_io, conn) = tls_stream.get_ref();
                tls::sas_for_connection(|buf, label, ctx| {
                    conn.export_keying_material(buf, label, ctx)
                })
                .unwrap_or_default()
            };

            let io = TokioIo::new(tls_stream);
            let svc = hyper::service::service_fn(move |mut req: hyper::Request<Incoming>| {
                req.extensions_mut().insert(ConnectionSas(sas.clone()));
                let mut app = app.clone();
                async move { app.call(req).await }
            });

            let _ = HyperBuilder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(io, svc)
                .await;
        });
    }
}
```

在 `server.rs` 的 `share_upgrade` 取出 SAS 並傳給 handler：

```rust
async fn share_upgrade(
    ws: WebSocketUpgrade,
    axum::Extension(sas): axum::Extension<crate::share::ConnectionSas>,
    State(state): State<ShareAppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_share(socket, state, sas.0))
}
```

`handle_share` 的簽名加上 `sas: String`，並把送出 `AwaitingApproval` 那行改成：

```rust
    if !send_control(&mut ws, &ServerMessage::AwaitingApproval { sas: sas.clone() }).await {
```

**這一步會弄壞 Task 6 的三個明文整合測試**：它們的 `start_test_server` 直接用 `axum::serve` 起 router，沒有經過 TLS accept 迴圈，所以沒有 `ConnectionSas` extension，`share_upgrade` 會回 500。

修法是讓測試 helper 自己注入一個假的，**不要**把 `share_upgrade` 的 extractor 改成 `Option`——那等於讓正式路徑在 TLS 沒接上時無聲降級成沒有 SAS，把身分保證變成裝飾。改 `tests/share_end_to_end.rs` 的 helper：

```rust
async fn start_test_server(
    pty: Arc<PtyManager>,
    registry: Arc<ShareRegistry>,
) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // 這條路徑刻意不走 TLS——這三個測試驗的是握手/授權/串流邏輯，TLS 由
    // both_ends_of_a_real_tls_connection_compute_the_same_sas 單獨驗。SAS
    // extension 在正式路徑由 TLS accept 迴圈注入，這裡手動補一個假的。
    let app = aiterm_lib::share::server::router(pty, registry)
        .layer(axum::Extension(aiterm_lib::share::ConnectionSas("0000".to_string())));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
}
```

這一步的 `ServerMessage::AwaitingApproval` 因此要加一個欄位：

```rust
    AwaitingApproval { sas: String },
```

並同步更新 `protocol.rs` 的測試與 Task 6 那三個整合測試裡的 `assert_eq!(..., ServerMessage::AwaitingApproval)`，改成用 pattern match 只檢查變體：

```rust
    assert!(matches!(next_control(&mut ws).await, ServerMessage::AwaitingApproval { .. }));
```

觀看端的 SAS 由它自己那條 `ClientConnection` 算出，不從線上接收——**若 SAS 是伺服器送過來的，中間人只要原封轉發就能讓兩邊一致，整個保證就沒了**。線上那個 `sas` 欄位只給主控端 UI 顯示用。

- [ ] **Step 4: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --test share_end_to_end && cargo test --lib share::`
Expected: 全部 PASS。

- [ ] **Step 5: 確認沒有意外的常駐監聽**

Run: `cd src-tauri && cargo test --lib share::` 之後，人工確認 `ShareServerState::default()` 不會啟動任何東西：

```bash
cd src-tauri && grep -n "start_if_needed" src/share/mod.rs src/lib.rs
```

Expected: `lib.rs` 裡**沒有**任何呼叫——server 只該由「使用者按下分享」觸發（那是計畫②的事）。若 `lib.rs` 出現呼叫，那是錯的。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/share/ src-tauri/tests/share_end_to_end.rs
git commit -m "feat(share): serve the share socket over TLS and bind the LAN interface

SAS 由雙方各自從自己的 TLS 連線導出，不從線上傳遞。"
```

---

## 完成標準

計畫①做完時，以下全部成立：

- [ ] `cd src-tauri && cargo test` 全綠
- [ ] `cd src-tauri && cargo clippy -- -D warnings` 無警告
- [ ] `npm run lint` 與 `npx tsc -b` 仍綠（本計畫不動前端，這是防迴歸）
- [ ] `grep -c 'pty reader error' src/pty/session.rs` 回 `1`（reader thread 沒有再次分裂成兩份）
- [ ] `lib.rs` 裡沒有任何 `start_if_needed` 呼叫——沒人按分享就沒有監聽
- [ ] 整合測試涵蓋：短碼無效被拒、唯讀端看得到但打不進去、控制端打得進去、停止分享送出正確的結束原因、TLS 兩端算出相同 SAS

**尚未具備**（依序由後續計畫補上）：任何 UI（計畫②）、區網自動發現（計畫③）。計畫①結束時這套東西只能由測試驅動，使用者按不到。
