# 遠端終端機共享 2A：Tauri commands 與事件推送 — 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓計畫①做好的共享後端能被前端驅動——開始／停止分享、收到連線請求時被推播、核准／拒絕／踢人，全部有型別化的 Tauri command 與 IPC wrapper。

**Architecture:** 比照既有的 `commands/mcp_server.rs` ＋ `src/ipc/mcpToolServer.ts`（同樣是「一個可啟停的本機 server」）。`ShareServerState` 進 Tauri managed state；`share/server.rs` 拿到 `AppHandle` 後在收到連線請求時發事件。**4 位驗證碼永遠不離開 Rust**——比對在 `share_approve` 裡做。

**Tech Stack:** Rust / Tauri 2 commands + events / TypeScript IPC wrappers

**Spec:** `docs/superpowers/specs/2026-08-26-remote-terminal-sharing-2-ui-design.md`

**本計畫不含**（各自留給後續）：任何 React 元件（2B）、mDNS（2C）。2A 結束時後端能被驅動，但畫面上還是按不到。

---

## 這個計畫最重要的一件事

spec 的不變式是「同意視窗絕不顯示主控端自己算出的 4 位數」——顯示了，使用者會照抄而不問對方，核對變成自欺。

**只靠 UI 不畫出來是靠自律的防線**：碼還是送到前端了，任何人日後加個 debug 顯示就破功。

所以這個計畫把它變成**結構上的保證**：

- `share_pending()` 回傳的 `PendingRequestView` **沒有 `sas` 欄位**——前端拿不到
- `share_approve(request_id, mode, typed_code)` 把使用者輸入的碼送進 Rust，在那裡跟 `PendingRequest::sas` 比對

前端**永遠沒有機會**顯示那個碼，因為它從來沒收到過。

---

## 檔案結構

| 檔案 | 責任 | 動作 |
|---|---|---|
| `src-tauri/src/commands/share.rs` | 全部 9 個 Tauri command | 新增 |
| `src-tauri/src/share/mod.rs` | `start_if_needed` 收 `AppHandle`；`ShareAppState` 帶著它 | 修改 |
| `src-tauri/src/share/server.rs` | 收到請求／觀看者變動時發事件 | 修改 |
| `src-tauri/src/lib.rs` | `.manage(Arc::new(ShareServerState::new()))` ＋ 註冊 commands | 修改 |
| `src/ipc/share.ts` | 型別化 IPC wrapper ＋ 事件訂閱 | 新增 |
| `src-tauri/tests/share_commands.rs` | command 層的整合測試 | 新增 |

---

## Task 1: `ShareServerState` 進 Tauri state，server 拿得到 AppHandle

事件要從 `share/server.rs` 的 ws handler 發出去（那裡才知道「有人要連進來了」），所以 `AppHandle` 必須一路傳進去。比照 `McpToolServerState::start` 收 `app_handle: Option<tauri::AppHandle>` 的既有作法。

`Option` 而不是必填：整合測試不起 Tauri app，傳 `None` 就好。

**Files:**
- Modify: `src-tauri/src/share/mod.rs`
- Modify: `src-tauri/src/share/server.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 先確認現在是綠的**

Run: `cd src-tauri && cargo test --lib share:: && cargo test --test share_end_to_end`
Expected: `39 passed` 與 `10 passed; 1 ignored`。記下這兩個數字。

- [ ] **Step 2: `ShareAppState` 加 `app` 欄位**

`src-tauri/src/share/server.rs`，把：

```rust
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
```

換成：

```rust
#[derive(Clone)]
pub struct ShareAppState {
    pub pty: Arc<PtyManager>,
    pub registry: Arc<ShareRegistry>,
    /// 用來把「有人要連進來」推播給前端。整合測試不起 Tauri app，所以是
    /// `Option`——`None` 時所有事件發送都是 no-op，其餘行為完全一樣。
    pub app: Option<tauri::AppHandle>,
}

pub fn router(
    pty: Arc<PtyManager>,
    registry: Arc<ShareRegistry>,
    app: Option<tauri::AppHandle>,
) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/share", any(share_upgrade))
        .with_state(ShareAppState { pty, registry, app })
}
```

- [ ] **Step 3: `start_if_needed` 系列收 `AppHandle`**

`src-tauri/src/share/mod.rs`：

```rust
    pub async fn start_if_needed(
        &self,
        pty: Arc<PtyManager>,
        app: Option<tauri::AppHandle>,
    ) -> anyhow::Result<u16> {
        self.start_if_needed_on_port(pty, 0, app).await
    }

    /// 同 `start_if_needed`，但綁指定的 port。`0` 表示交給 OS 挑。
    ///
    /// 手動的區網連通性檢查需要固定 port：另一台機器必須**先知道**要連哪裡，
    /// 而浮動 port 逼人先把 server 跑起來才看得到位址。
    pub async fn start_if_needed_on_port(
        &self,
        pty: Arc<PtyManager>,
        port: u16,
        app: Option<tauri::AppHandle>,
    ) -> anyhow::Result<u16> {
        if let Some(p) = self.port() {
            return Ok(p);
        }
        ensure_crypto_provider();
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await?;
        let port = listener.local_addr()?.port();
        let app_router = server::router(pty, Arc::clone(&self.registry), app);
        let identity = tls::ShareIdentity::generate()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(serve_tls(listener, app_router, identity, rx));
        *self.running.lock() = Some(Running { port, shutdown: tx });
        Ok(port)
    }
```

（注意變數改名為 `app_router`，避免跟新參數 `app` 撞名。）

- [ ] **Step 4: 更新既有呼叫點**

`src-tauri/tests/share_end_to_end.rs` 有兩處呼叫要補 `None`：

```bash
cd src-tauri && grep -n "start_if_needed" tests/share_end_to_end.rs
```

每一處都在最後一個參數加 `None`。另外 `start_test_server` 裡的 `server::router(pty, registry)` 也要補成 `server::router(pty, registry, None)`。

- [ ] **Step 5: `lib.rs` 註冊 managed state**

在 `.manage(Arc::new(PtyManager::new()))` 那一串裡加一行（維持既有排列）：

```rust
        .manage(Arc::new(share::ShareServerState::new()))
```

**注意：只註冊，不啟動。** `lib.rs` 裡不該出現任何 `start_if_needed` 呼叫——沒人按分享就不該有監聽。這是計畫①的驗收條件之一。

- [ ] **Step 6: 確認測試數量沒變**

Run: `cd src-tauri && cargo test --lib share:: && cargo test --test share_end_to_end`
Expected: 跟 Step 1 完全相同（`39 passed`、`10 passed; 1 ignored`）。這是純接線，行為不該變。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/share/ src-tauri/src/lib.rs src-tauri/tests/share_end_to_end.rs
git commit -m "refactor(share): thread an AppHandle into the share server

事件要從 ws handler 發出去（那裡才知道有人要連進來），所以 AppHandle 得
一路傳進去。Option 是為了讓整合測試不必起 Tauri app。"
```

---

## Task 2: 收到連線請求時發事件

前端要能在對方輸入短碼的當下跳出同意視窗，所以需要推播而不是輪詢。

**事件 payload 刻意不含 `sas`**——見本計畫開頭。

**Files:**
- Modify: `src-tauri/src/share/server.rs`
- Modify: `src-tauri/src/share/protocol.rs`

- [ ] **Step 1: 定義事件 payload**

加到 `src-tauri/src/share/protocol.rs` 的結尾（`#[cfg(test)]` 之前）：

```rust
/// 「有人要連進來」推播給前端的內容。
///
/// **刻意沒有 `sas` 欄位。** 主控端的 4 位驗證碼永遠不離開 Rust——同意視窗
/// 要使用者輸入對方唸的碼，比對在 `share_approve` 裡做。若這裡帶了 sas，
/// 前端就有機會顯示它，而使用者會照抄而不問對方，核對變成自欺。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequestEvent {
    pub request_id: String,
    pub tab_id: String,
    /// 對方自報的名字，**未經驗證**。前端文案不能讓它看起來像身分保證。
    pub display_name: String,
}
```

- [ ] **Step 2: 寫會紅的測試**

加到 `src-tauri/src/share/protocol.rs` 的測試模組：

```rust
    #[test]
    fn the_pending_request_event_never_carries_a_sas() {
        // 這是結構性的保證，不是 UI 慣例：前端拿不到主控端的驗證碼，所以
        // 不可能顯示它。若哪天有人「順手」把 sas 加進這個結構，同意視窗就
        // 能照抄而不問對方，整個人工核對變成自欺。
        let ev = PendingRequestEvent {
            request_id: "r1".to_string(),
            tab_id: "t1".to_string(),
            display_name: "Alice".to_string(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"requestId\":\"r1\""), "got {json}");
        assert!(json.contains("\"displayName\":\"Alice\""), "got {json}");
        assert!(
            !json.contains("sas"),
            "the pending-request event must never carry a SAS; got {json}"
        );
    }
```

- [ ] **Step 3: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --lib share::protocol::tests::the_pending_request_event_never_carries_a_sas`
Expected: **編譯失敗**——`cannot find type PendingRequestEvent`。

- [ ] **Step 4: 在 ws handler 發事件**

`src-tauri/src/share/server.rs`，在 `request_join` 成功、`tab_id` 也取到之後（也就是 `send_control(... AwaitingApproval ...)` 那段的**正上方**）加入：

```rust
    // 推播給前端，讓同意視窗跳出來。`None` 時（整合測試）是 no-op。
    if let Some(app) = &state.app {
        use tauri::Emitter;
        let _ = app.emit(
            "share://request-pending",
            super::protocol::PendingRequestEvent {
                request_id: request_id.clone(),
                tab_id: tab_id.clone(),
                display_name: display_name_for_event.clone(),
            },
        );
    }
```

`display_name` 在 `request_join` 時被 move 進去了，所以要在呼叫前先留一份。把：

```rust
    let Some(request_id) = state.registry.request_join(&code, display_name, sas.clone()) else {
```

改成：

```rust
    let display_name_for_event = display_name.clone();
    let Some(request_id) = state.registry.request_join(&code, display_name, sas.clone()) else {
```

- [ ] **Step 5: 觀看者離開時也發事件**

主控端的觀看者清單要能自己更新（對方關掉連線時，主控端沒做任何動作）。在 `handle_share` **最後那行** `state.registry.remove_viewer(&tab_id, &viewer_id);` 的正下方加入：

```rust
    // 觀看者清單變了，讓主控端的面板重新抓一次。這個事件不帶內容——前端
    // 收到就去 `share_viewers` 重讀，避免兩份資料對不上。
    if let Some(app) = &state.app {
        use tauri::Emitter;
        let _ = app.emit("share://viewers-changed", ());
    }
```

- [ ] **Step 6: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --lib share:: && cargo test --test share_end_to_end`
Expected: `40 passed`（39 + 1 個新測試）與 `10 passed; 1 ignored`。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/share/
git commit -m "feat(share): push pending requests and viewer changes to the frontend

事件 payload 刻意不含 sas——主控端的驗證碼永遠不離開 Rust，比對在
share_approve 裡做。前端拿不到就不可能顯示它。"
```

---

## Task 3: 開始／停止分享的 commands

**Files:**
- Create: `src-tauri/src/commands/share.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 建立 command 模組**

建立 `src-tauri/src/commands/share.rs`：

```rust
//! 遠端終端機共享的 Tauri commands。
//!
//! 形狀比照 `commands/mcp_server.rs`（同樣是一個可啟停的本機 server）。
//!
//! **一條貫穿整個模組的規則：4 位驗證碼不離開 Rust。** 回傳給前端的結構
//! 沒有 sas 欄位；同意時使用者輸入的碼送進來，比對在這裡做。理由見
//! `share_approve` 的說明。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::pty::manager::PtyManager;
use crate::share::registry::AccessMode;
use crate::share::ShareServerState;

/// 某個分頁的分享狀態，給分享面板顯示。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareStatus {
    /// 這個分頁正在分享嗎。
    pub sharing: bool,
    /// 6 位短碼。沒在分享時是 `None`。
    pub code: Option<String>,
    /// server 的 port。沒在分享時是 `None`。
    pub port: Option<u16>,
}

#[tauri::command]
pub async fn share_start(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
    pty: State<'_, Arc<PtyManager>>,
    app: AppHandle,
) -> Result<ShareStatus, String> {
    let port = server
        .start_if_needed(Arc::clone(&pty), Some(app))
        .await
        .map_err(|e| format!("啟動共享服務失敗：{e}"))?;
    let code = server.registry.start_share(tab_id);
    Ok(ShareStatus { sharing: true, code: Some(code), port: Some(port) })
}

#[tauri::command]
pub async fn share_stop(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<ShareStatus, String> {
    server.registry.stop_share(&tab_id);
    server.stop_if_idle();
    Ok(ShareStatus { sharing: false, code: None, port: None })
}

#[tauri::command]
pub async fn share_status(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<ShareStatus, String> {
    let code = server.registry.code_for_tab(&tab_id);
    let port = server.port();
    Ok(ShareStatus { sharing: code.is_some(), code, port })
}
```

- [ ] **Step 2: `ShareRegistry` 補一支 `code_for_tab`**

`share_status` 需要「這個分頁的短碼是什麼」，但 registry 目前只有反方向的 `tab_for_code`。加到 `src-tauri/src/share/registry.rs` 的 `tab_for_code` 正下方：

```rust
    /// 某分頁目前的短碼；沒在分享時回 `None`。`tab_for_code` 的反向查詢。
    pub fn code_for_tab(&self, tab_id: &str) -> Option<String> {
        self.tabs.lock().get(tab_id).map(|t| t.code.clone())
    }
```

並加測試到 `registry.rs` 的測試模組：

```rust
    #[test]
    fn code_for_tab_is_the_inverse_of_tab_for_code() {
        let (reg, code) = registry_with_one_share();
        assert_eq!(reg.code_for_tab("tab-1"), Some(code.clone()));
        assert_eq!(reg.tab_for_code(&code), Some("tab-1".to_string()));
        reg.stop_share("tab-1");
        assert_eq!(reg.code_for_tab("tab-1"), None);
    }
```

- [ ] **Step 3: 註冊模組與 commands**

`src-tauri/src/commands/mod.rs` 加一行（維持字母序）：

```rust
pub mod share;
```

`src-tauri/src/lib.rs` 的 `use` 區塊加入（比照既有 commands 的 import 樣式）：

```rust
    share::{share_start, share_status, share_stop},
```

並在 `tauri::generate_handler![...]` 清單裡加入這三個名字。

- [ ] **Step 4: 跑測試確認編譯與既有測試都過**

Run: `cd src-tauri && cargo test --lib share::`
Expected: `41 passed`（40 + `code_for_tab_is_the_inverse_of_tab_for_code`）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/ src-tauri/src/share/ src-tauri/src/lib.rs
git commit -m "feat(share): add start/stop/status commands"
```

---

## Task 4: 核准／拒絕——驗證碼比對在 Rust 端

**這是整個 2A 最重要的一個 task。**

**Files:**
- Modify: `src-tauri/src/commands/share.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/share_commands.rs`（新增）

- [ ] **Step 1: 寫會紅的測試**

建立 `src-tauri/tests/share_commands.rs`：

```rust
//! `commands/share.rs` 的核心不變式測試。
//!
//! 這裡不起 Tauri app——`#[tauri::command]` 的函式本體是普通的 async fn，
//! 但它們吃 `State<'_, T>`，測試裡拿不到。所以這些測試直接驗**被 command
//! 呼叫的那層邏輯**（`ShareRegistry` ＋ 比對函式），command 本身只是很薄的
//! 轉接。真正的端到端由 `share_end_to_end.rs` 涵蓋。

use aiterm_lib::share::registry::{AccessMode, ShareRegistry};

#[test]
fn approving_with_the_wrong_code_denies_the_request_outright() {
    // 攻擊者只有 1/10000 的一發機會。給重試等於送他一萬次——所以輸錯不是
    // 「再試一次」，是直接拒絕。
    let reg = ShareRegistry::new();
    let code = reg.start_share("tab-1".to_string());
    let req = reg
        .request_join(&code, "Alice".to_string(), "4917".to_string())
        .unwrap();

    let outcome = aiterm_lib::commands::share::decide(&reg, &req, AccessMode::Control, "1234");

    assert!(matches!(outcome, aiterm_lib::commands::share::Decision::CodeMismatch));
    assert_eq!(
        reg.pending("tab-1").len(),
        0,
        "a mismatched code must drop the request, not leave it retryable"
    );
    assert_eq!(reg.viewers("tab-1").len(), 0);
}

#[test]
fn approving_with_the_right_code_admits_the_viewer() {
    let reg = ShareRegistry::new();
    let code = reg.start_share("tab-1".to_string());
    let req = reg
        .request_join(&code, "Alice".to_string(), "4917".to_string())
        .unwrap();

    let outcome = aiterm_lib::commands::share::decide(&reg, &req, AccessMode::Control, "4917");

    match outcome {
        aiterm_lib::commands::share::Decision::Approved { viewer_id } => {
            assert!(reg.may_send_input("tab-1", &viewer_id));
        }
        other => panic!("expected Approved, got {other:?}"),
    }
    assert_eq!(reg.pending("tab-1").len(), 0);
}

#[test]
fn control_already_taken_is_reported_and_the_request_survives() {
    // 控制權被占用時整筆核准失敗，但請求要留著——主控端可以改用唯讀重新
    // 裁決。這是 registry 既有的語意，command 層不能把它吃掉。
    let reg = ShareRegistry::new();
    let code = reg.start_share("tab-1".to_string());
    let r1 = reg.request_join(&code, "Alice".to_string(), "1111".to_string()).unwrap();
    reg.approve(&r1, AccessMode::Control).unwrap();

    let r2 = reg.request_join(&code, "Bob".to_string(), "2222".to_string()).unwrap();
    let outcome = aiterm_lib::commands::share::decide(&reg, &r2, AccessMode::Control, "2222");

    assert!(matches!(outcome, aiterm_lib::commands::share::Decision::ControlTaken));
    assert_eq!(
        reg.pending("tab-1").len(),
        1,
        "the request must survive so the host can re-decide as read-only"
    );
}

#[test]
fn a_request_that_vanished_is_reported_not_panicked() {
    let reg = ShareRegistry::new();
    let code = reg.start_share("tab-1".to_string());
    let req = reg.request_join(&code, "Alice".to_string(), "4917".to_string()).unwrap();
    reg.deny(&req);

    let outcome = aiterm_lib::commands::share::decide(&reg, &req, AccessMode::ReadOnly, "4917");
    assert!(matches!(outcome, aiterm_lib::commands::share::Decision::RequestGone));
}
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --test share_commands`
Expected: **編譯失敗**——`could not find share in commands` 或 `cannot find function decide`。

- [ ] **Step 3: `ShareRegistry` 補一支「查某筆請求的 sas」**

比對要在 Rust 端做，所以 command 需要拿得到那筆請求存的 sas。加到 `registry.rs` 的 `pending` 正下方：

```rust
    /// 某筆待審請求存的 4 位驗證碼。
    ///
    /// **只給 `commands::share::decide` 用。** 這個值不會被序列化到任何送往
    /// 前端的結構裡——主控端必須跟對方口頭核對，而不是照抄畫面上的數字。
    pub fn sas_for_request(&self, request_id: &str) -> Option<String> {
        self.pending.lock().get(request_id).map(|p| p.sas.clone())
    }
```

- [ ] **Step 4: 實作 `decide`**

加到 `src-tauri/src/commands/share.rs`：

```rust
/// `share_approve` 的裁決結果。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Decision {
    /// 碼對了，觀看者已加入。
    ///
    /// 變體上這個 `rename_all` **不能省**：enum 容器層的 `rename_all` 只轉
    /// 變體名稱，不會轉變體裡的欄位。少了它，這裡會送出 `viewer_id` 而前端
    /// 型別寫的是 `viewerId`，兩邊在執行期才會對不上。
    #[serde(rename_all = "camelCase")]
    Approved { viewer_id: String },
    /// 輸入的碼跟這條連線的不符——請求已被拒絕，**不給重試**。
    CodeMismatch,
    /// 要控制權但已經有人持有。請求還在，可以改用唯讀重新裁決。
    ControlTaken,
    /// 請求已經不在了（對方斷線、或分享被停掉）。
    RequestGone,
}

/// 核准或拒絕一筆待審請求。
///
/// **驗證碼的比對在這裡做，不在前端。** 主控端的 4 位碼存在
/// `PendingRequest::sas`，從來不會被送到 JS 端——`PendingRequestView` 與
/// `PendingRequestEvent` 都沒有那個欄位。使用者必須跟對方口頭核對、把聽到
/// 的數字打進來，這裡才比對。
///
/// 若碼送到了前端，使用者會照抄畫面上的數字而不問對方，人工核對變成自欺，
/// 而整個防中間人保證的最後一哩就是那次口頭核對。
///
/// **碼不符時直接拒絕，不給重試**：攻擊者只有 1/10000 的一發機會，給重試
/// 等於送他一萬次。
///
/// 抽成自由函式（而不是寫在 `#[tauri::command]` 裡）是為了能在不起 Tauri
/// app 的情況下測試——command 本身只是很薄的轉接。
pub fn decide(
    registry: &crate::share::registry::ShareRegistry,
    request_id: &str,
    mode: AccessMode,
    typed_code: &str,
) -> Decision {
    let Some(expected) = registry.sas_for_request(request_id) else {
        return Decision::RequestGone;
    };
    if typed_code != expected {
        registry.deny(request_id);
        return Decision::CodeMismatch;
    }
    match registry.approve(request_id, mode) {
        Some(viewer_id) => Decision::Approved { viewer_id },
        // `approve` 回 None 有兩種可能：控制權被占用（請求被放回待審），
        // 或分享在裁決前被停掉（請求消失）。用請求還在不在來分辨。
        None => {
            if registry.sas_for_request(request_id).is_some() {
                Decision::ControlTaken
            } else {
                Decision::RequestGone
            }
        }
    }
}
```

- [ ] **Step 5: 加上 command 外殼**

同一個檔案：

```rust
/// 待審請求，給同意視窗顯示。
///
/// **刻意沒有 `sas` 欄位**——見 `decide` 的說明。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequestView {
    pub request_id: String,
    pub tab_id: String,
    /// 對方自報的名字，**未經驗證**。
    pub display_name: String,
}

#[tauri::command]
pub async fn share_pending(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<Vec<PendingRequestView>, String> {
    Ok(server
        .registry
        .pending(&tab_id)
        .into_iter()
        .map(|p| PendingRequestView {
            request_id: p.request_id,
            tab_id: p.tab_id,
            display_name: p.display_name,
        })
        .collect())
}

#[tauri::command]
pub async fn share_approve(
    request_id: String,
    mode: String,
    typed_code: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<Decision, String> {
    let mode = match mode.as_str() {
        "read_only" => AccessMode::ReadOnly,
        "control" => AccessMode::Control,
        other => return Err(format!("未知的存取模式：{other}")),
    };
    Ok(decide(&server.registry, &request_id, mode, &typed_code))
}

#[tauri::command]
pub async fn share_deny(
    request_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<(), String> {
    server.registry.deny(&request_id);
    Ok(())
}
```

- [ ] **Step 6: `lib.rs` 註冊三個新 command**

`use` 區塊那行改成：

```rust
    share::{share_approve, share_deny, share_pending, share_start, share_status, share_stop},
```

並在 `generate_handler!` 清單裡加入 `share_approve`、`share_deny`、`share_pending`。

- [ ] **Step 7: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --test share_commands && cargo test --lib share::`
Expected: `4 passed` 與 `41 passed`。

- [ ] **Step 8: 確認 sas 真的沒有出口**

```bash
cd src-tauri && grep -n "sas" src/commands/share.rs | grep -v "^.*///" | grep -v "typed_code\|sas_for_request\|expected"
```

Expected: 沒有輸出。也就是說 `share.rs` 裡除了「讀出來比對」之外，沒有任何地方把 sas 放進回傳結構。

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/commands/ src-tauri/src/share/ src-tauri/src/lib.rs src-tauri/tests/share_commands.rs
git commit -m "feat(share): approve/deny with the code compared in Rust

主控端的 4 位驗證碼永遠不離開 Rust：回傳給前端的結構沒有 sas 欄位，
同意時把使用者輸入的碼送進來比對。前端拿不到就不可能顯示它——這比
「UI 選擇不顯示」強得多。碼不符直接拒絕，不給重試。"
```

---

## Task 5: 觀看者清單、踢人、收回控制權

**Files:**
- Modify: `src-tauri/src/commands/share.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 加上三個 command**

加到 `src-tauri/src/commands/share.rs`：

```rust
/// 一位觀看者，給分享面板的清單顯示。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerView {
    pub viewer_id: String,
    /// 對方自報的名字，**未經驗證**。
    pub display_name: String,
    /// `"read_only"` 或 `"control"`。
    pub mode: String,
}

#[tauri::command]
pub async fn share_viewers(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<Vec<ViewerView>, String> {
    Ok(server
        .registry
        .viewers(&tab_id)
        .into_iter()
        .map(|v| ViewerView {
            viewer_id: v.viewer_id,
            display_name: v.display_name,
            mode: match v.mode {
                AccessMode::ReadOnly => "read_only".to_string(),
                AccessMode::Control => "control".to_string(),
            },
        })
        .collect())
}

#[tauri::command]
pub async fn share_kick(
    tab_id: String,
    viewer_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<(), String> {
    server.registry.remove_viewer(&tab_id, &viewer_id);
    Ok(())
}

#[tauri::command]
pub async fn share_revoke_control(
    tab_id: String,
    server: State<'_, Arc<ShareServerState>>,
) -> Result<(), String> {
    server.registry.revoke_control(&tab_id);
    Ok(())
}
```

- [ ] **Step 2: `lib.rs` 註冊**

`use` 區塊那行改成：

```rust
    share::{
        share_approve, share_deny, share_kick, share_pending, share_revoke_control, share_start,
        share_status, share_stop, share_viewers,
    },
```

並在 `generate_handler!` 加入 `share_kick`、`share_revoke_control`、`share_viewers`。

- [ ] **Step 3: 確認全部 9 個 command 都註冊了**

```bash
cd src-tauri && for c in share_start share_stop share_status share_pending share_approve share_deny share_viewers share_kick share_revoke_control; do
  printf "%-24s" "$c"; grep -c "\b$c\b" src/lib.rs
done
```

Expected: 每一行都是 `2`（`use` 一次、`generate_handler!` 一次）。任何一行是 `1` 就代表漏註冊，那個 command 前端呼叫時會在執行期才失敗。

- [ ] **Step 4: 編譯確認**

Run: `cd src-tauri && cargo test --lib share:: && cargo test --test share_commands`
Expected: `41 passed` 與 `4 passed`。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/ src-tauri/src/lib.rs
git commit -m "feat(share): viewer list, kick, and revoke-control commands"
```

---

## Task 6: 前端 IPC wrapper

**Files:**
- Create: `src/ipc/share.ts`
- Test: `src/ipc/share.test.ts`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/ipc/share.test.ts`：

```ts
import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { shareStart, shareApprove, sharePending, shareViewers } from "./share";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("share IPC", () => {
  it("passes the tab id when starting a share", async () => {
    invokeMock.mockResolvedValue({ sharing: true, code: "559207", port: 47823 });
    const s = await shareStart("tab-1");
    expect(invokeMock).toHaveBeenCalledWith("share_start", { tabId: "tab-1" });
    expect(s.code).toBe("559207");
  });

  it("sends the typed code to Rust rather than comparing here", async () => {
    // 比對必須在 Rust 端做——前端從來沒收到過主控端的驗證碼，所以連比對
    // 的材料都沒有。這個測試守著「前端只負責把使用者打的字送過去」。
    invokeMock.mockResolvedValue({ kind: "approved", viewerId: "v1" });
    await shareApprove("r1", "control", "4917");
    expect(invokeMock).toHaveBeenCalledWith("share_approve", {
      requestId: "r1",
      mode: "control",
      typedCode: "4917",
    });
  });

  it("never receives a sas field in pending requests", async () => {
    // 型別上就沒有這個欄位。這個測試是給未來的人看的：如果有人把 sas 加
    // 回後端的回傳結構，這裡會提醒他為什麼不該加。
    invokeMock.mockResolvedValue([
      { requestId: "r1", tabId: "t1", displayName: "Alice" },
    ]);
    const pending = await sharePending("t1");
    expect(Object.keys(pending[0])).toEqual(["requestId", "tabId", "displayName"]);
    expect(JSON.stringify(pending)).not.toContain("sas");
  });

  it("reads the viewer list for a tab", async () => {
    invokeMock.mockResolvedValue([
      { viewerId: "v1", displayName: "Alice", mode: "control" },
    ]);
    const viewers = await shareViewers("t1");
    expect(invokeMock).toHaveBeenCalledWith("share_viewers", { tabId: "t1" });
    expect(viewers[0].mode).toBe("control");
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `npx vitest run src/ipc/share.test.ts`
Expected: FAIL——`Failed to resolve import "./share"`。

- [ ] **Step 3: 實作 wrapper**

建立 `src/ipc/share.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ShareStatus {
  sharing: boolean;
  /** 6 位短碼。沒在分享時是 null。 */
  code: string | null;
  /** server 的 port。沒在分享時是 null。 */
  port: number | null;
}

/**
 * 一筆待審連線請求。
 *
 * **刻意沒有驗證碼欄位。** 主控端的 4 位碼永遠不離開 Rust——同意視窗要
 * 使用者輸入對方唸的碼，比對在 `share_approve` 裡做。前端拿不到那個值，
 * 所以不可能顯示它；使用者只能開口問對方。
 *
 * 若哪天有人想「順便把碼也傳過來顯示」：那會讓使用者照抄畫面上的數字而
 * 不問對方，人工核對變成自欺，而那次口頭核對正是整個防中間人保證的最後
 * 一哩。不要加。
 */
export interface PendingRequest {
  requestId: string;
  tabId: string;
  /** 對方自報的名字，**未經驗證**。文案不能讓它看起來像身分保證。 */
  displayName: string;
}

export interface Viewer {
  viewerId: string;
  /** 對方自報的名字，**未經驗證**。 */
  displayName: string;
  mode: "read_only" | "control";
}

export type Decision =
  | { kind: "approved"; viewerId: string }
  /** 輸入的碼不符——連線已被拒絕，不給重試。 */
  | { kind: "codeMismatch" }
  /** 控制權已被別人持有；請求還在，可以改用唯讀重新裁決。 */
  | { kind: "controlTaken" }
  /** 請求已經不在了（對方斷線或分享被停掉）。 */
  | { kind: "requestGone" };

export function shareStart(tabId: string): Promise<ShareStatus> {
  return invoke<ShareStatus>("share_start", { tabId });
}

export function shareStop(tabId: string): Promise<ShareStatus> {
  return invoke<ShareStatus>("share_stop", { tabId });
}

export function shareStatus(tabId: string): Promise<ShareStatus> {
  return invoke<ShareStatus>("share_status", { tabId });
}

export function sharePending(tabId: string): Promise<PendingRequest[]> {
  return invoke<PendingRequest[]>("share_pending", { tabId });
}

/** 把使用者輸入的碼送去 Rust 比對。前端不做比對——它沒有那個材料。 */
export function shareApprove(
  requestId: string,
  mode: "read_only" | "control",
  typedCode: string,
): Promise<Decision> {
  return invoke<Decision>("share_approve", { requestId, mode, typedCode });
}

export function shareDeny(requestId: string): Promise<void> {
  return invoke<void>("share_deny", { requestId });
}

export function shareViewers(tabId: string): Promise<Viewer[]> {
  return invoke<Viewer[]>("share_viewers", { tabId });
}

export function shareKick(tabId: string, viewerId: string): Promise<void> {
  return invoke<void>("share_kick", { tabId, viewerId });
}

export function shareRevokeControl(tabId: string): Promise<void> {
  return invoke<void>("share_revoke_control", { tabId });
}

/** 有人輸入短碼要連進來了——同意視窗該跳出來。 */
export function onSharePendingRequest(
  cb: (payload: PendingRequest) => void,
): Promise<UnlistenFn> {
  return listen<PendingRequest>("share://request-pending", (e) => cb(e.payload));
}

/**
 * 觀看者清單變了（有人連上或離開）。
 *
 * 刻意不帶內容——收到就去 `shareViewers` 重讀，避免推播的資料跟查詢的
 * 資料對不上。
 */
export function onShareViewersChanged(cb: () => void): Promise<UnlistenFn> {
  return listen<unknown>("share://viewers-changed", () => cb());
}
```

- [ ] **Step 4: 跑測試確認轉綠**

Run: `npx vitest run src/ipc/share.test.ts`
Expected: PASS，4 個測試全過。

- [ ] **Step 5: 型別檢查**

Run: `npx tsc -b`
Expected: 沒有輸出（成功）。

**注意**：用 `npx tsc -b`，**不要**用 `tsc --noEmit`——根 `tsconfig.json` 是 solution file（`"files": []`），`--noEmit` 什麼都不檢查而且永遠 exit 0。

- [ ] **Step 6: Commit**

```bash
git add src/ipc/share.ts src/ipc/share.test.ts
git commit -m "feat(share): typed IPC wrappers for the share commands

PendingRequest 型別上就沒有驗證碼欄位——前端從來收不到主控端的 4 位碼，
所以不可能顯示它。同意時只負責把使用者打的字送去 Rust 比對。"
```

---

## 完成標準

2A 做完時，以下全部成立：

- [ ] `cd src-tauri && cargo test` 全綠
- [ ] `npx vitest run src/ipc/share.test.ts` 全綠、`npx tsc -b` 通過
- [ ] `cargo clippy --lib --tests -- -D warnings 2>&1 | grep -E "src/share/|src/commands/share"` 沒有輸出

  注意：這個 repo 在計畫①開始前就有約 43 個既有 clippy 錯誤（`vcs`、`mcp`、`enterprise`、`pty/cd_parser` 等），**不是這個計畫造成的、也不要順手修**。同理 `cargo test --lib` 有一個既有的 flaky 測試（`pty::session::tests::every_subscriber_receives_the_same_output_and_on_data_still_fires`），偶發失敗重跑即可。
- [ ] 九個 command 都在 `lib.rs` 出現兩次（`use` ＋ `generate_handler!`）——見 Task 5 Step 3 的檢查指令
- [ ] `grep -n "sas" src-tauri/src/commands/share.rs` 只出現在比對邏輯與註解裡，**不出現在任何 `Serialize` 結構的欄位上**
- [ ] `lib.rs` 裡沒有任何 `start_if_needed` 呼叫——沒人按分享就不該有監聽

**尚未具備**：任何畫面（2B）、mDNS（2C）。2A 結束時後端能被前端驅動，但使用者按不到任何東西。
