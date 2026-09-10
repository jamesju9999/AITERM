# AITerm CLI Host Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 做出一個 headless 的 `aiterm-host` CLI 執行檔，讓桌面版 AITerm 用預共享金鑰連進去，並用觀看端自己的 AI 操作那台機器的 shell。

**Architecture:** 把 `src-tauri/src/` 裡不依賴 Tauri 的 `pty` 與 `share` 模組真搬移到新的 workspace 成員 `aiterm-core`；`share::server` 裡兩處 `AppHandle` 事件推播換成 `ShareEvents` trait。認證用預共享金鑰對 TLS exporter material 做 HMAC-SHA256 雙向互證，取代原本「人工唸 SAS 碼」的核准；協定不升版本，只加兩個可選附加欄位。CLI bin 只依賴 `aiterm-core`，零 GUI 依賴。

**Tech Stack:** Rust 2021、tokio、axum、rustls、portable-pty、hmac + sha2、clap 4。前端 React 19 + Vitest。

**Spec:** `docs/superpowers/specs/2026-09-10-aiterm-cli-host-design.md`

**這份計畫不含發布管道**（GitHub Releases／安裝腳本／musl／ghcr／Homebrew／npm）。那是獨立的第二份計畫，只依賴「本計畫產出一個編得出來的 bin」。

---

## 前置：本機環境

`src-tauri/binaries/` 是 gitignored 的，而 `tauri-build` 的 `build.rs` 在**編譯期**驗證每個 `externalBin` 存在於磁碟——沒有它連 `cargo check` 都會失敗。動工前先跑一次你平台的 setup 腳本：

```bash
# macOS
bash scripts/setup-uv-mac.sh
bash scripts/setup-db2-mac.sh
```

`aiterm-core` 與 `aiterm-host` 這兩個新 package 不受這個檢查影響（`build.rs` 綁在 `app` package 上），但只要你要編 `app`（Task 3 之後每一個任務都要）就需要。

---

## 檔案結構

### 新建

| 路徑 | 責任 |
|------|------|
| `src-tauri/crates/aiterm-core/Cargo.toml` | core package 定義，零 Tauri 依賴 |
| `src-tauri/crates/aiterm-core/src/lib.rs` | `pub mod pty; pub mod share;` |
| `src-tauri/crates/aiterm-core/src/pty/` | 從 `src-tauri/src/pty/` 搬來（不含 `commands.rs`） |
| `src-tauri/crates/aiterm-core/src/share/` | 從 `src-tauri/src/share/` 搬來（不含 `viewer_manager.rs`） |
| `src-tauri/crates/aiterm-core/src/share/auth.rs` | 金鑰、HMAC 證明的產生與驗證 |
| `src-tauri/crates/aiterm-core/src/share/events.rs` | `ShareEvents` trait 與 `SilentEvents` |
| `src-tauri/crates/aiterm-host/Cargo.toml` | CLI package 定義 |
| `src-tauri/crates/aiterm-host/src/main.rs` | CLI 進入點：參數解析、啟動、訊號處理 |
| `src-tauri/crates/aiterm-host/src/keyfile.rs` | 金鑰檔的讀取／產生／權限檢查 |
| `src-tauri/crates/aiterm-host/tests/cli_host.rs` | 端到端整合測試 |

### 修改

| 路徑 | 改什麼 |
|------|--------|
| `src-tauri/Cargo.toml` | 加 `[workspace]`、改依賴成 `aiterm-core` |
| `src-tauri/src/lib.rs` | `pub use aiterm_core::{pty, share}` 的轉接 |
| `src-tauri/src/pty/mod.rs` → `src-tauri/src/app_pty.rs` | 只留 `commands` 與 `create_with_app` |
| `src-tauri/src/share/mod.rs`（app 端殘留） | 只留 `viewer_manager` |
| `src-tauri/src/commands/share_viewer.rs` | `share_viewer_connect` 加 `key` 參數 |
| `src/ipc/shareViewer.ts` | 同上，前端型別 |
| `src/components/RemoteTerminalView/…` 的連線對話框 | 加「金鑰」欄位 |
| `src/lib/i18n.ts` | 新錯誤訊息字串 |

---

## Task 1: 驗證 serde 對未知欄位的行為（**gating**）

整份設計的相容性策略建立在「serde 忽略未知欄位」上。這跟 repo 已證明的「未知 *tag* 會報錯」是不同機制。**若這個任務的結論是「報錯」，立刻停下來回報使用者**——那代表要升 `PROTOCOL_VERSION` 到 3 並接受 GUI 之間的跨版本分享會斷，是範圍變更，不是實作者能自己決定的事。

這個任務在現有的 crate 佈局下就能做，不用等拆分。

**Files:**
- Modify: `src-tauri/src/share/protocol.rs`（只加測試）

- [ ] **Step 1: 寫測試**

加在 `src-tauri/src/share/protocol.rs` 的 `mod tests` 裡：

```rust
    #[test]
    fn an_unknown_field_on_join_is_ignored_rather_than_rejected() {
        // 整個「不升 PROTOCOL_VERSION、改用可選附加欄位」的相容性策略建立在
        // 這個行為上：舊版主控端收到新版觀看端多帶的 `auth` 欄位時，必須忽略
        // 它並照常走短碼流程，而不是硬性解析失敗變成無法解釋的斷線。
        //
        // 這跟 `an_unknown_server_message_fails_to_parse_rather_than_being_ignored`
        // 證明的是不同機制：那個講的是未知的 enum **tag**，這個講的是已知
        // variant 裡的未知**欄位**。兩者的 serde 預設行為不同，不能互相推論。
        let with_extra = r#"{"type":"join","protocol_version":2,"code":"384719","display_name":"Alice","auth":"deadbeef"}"#;
        let parsed: Result<ClientMessage, _> = serde_json::from_str(with_extra);
        let msg = parsed.expect(
            "serde rejected an unknown field on Join; the whole \
             additive-optional-field compatibility strategy in the CLI host \
             spec depends on it being ignored — STOP and report this",
        );
        assert_eq!(
            msg,
            ClientMessage::Join {
                protocol_version: 2,
                code: "384719".to_string(),
                display_name: "Alice".to_string(),
            }
        );
    }
```

- [ ] **Step 2: 跑測試**

```bash
cd src-tauri && cargo test --lib share::protocol::tests::an_unknown_field_on_join_is_ignored_rather_than_rejected
```

預期：**PASS**。

若是 FAIL：**停下來回報使用者**，不要繼續往下做。附上實際的錯誤訊息。

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/share/protocol.rs
git commit -m "test(share): pin down that serde ignores unknown fields on Join"
```

---

## Task 2: 建立 workspace 骨架

先讓兩個空 crate 存在且編得過，把建置設定的風險跟搬移的風險分開。

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/crates/aiterm-core/Cargo.toml`
- Create: `src-tauri/crates/aiterm-core/src/lib.rs`

- [ ] **Step 1: 建立 core crate**

`src-tauri/crates/aiterm-core/Cargo.toml`：

```toml
[package]
name = "aiterm-core"
version = "0.1.0"
edition = "2021"
rust-version = "1.88"

[dependencies]
```

`src-tauri/crates/aiterm-core/src/lib.rs`：

```rust
//! AITerm 的無 GUI 核心：PTY 生命週期與遠端終端機共享協定。
//!
//! **這個 crate 不依賴 Tauri，也不該依賴。** 它同時被 GUI（`app` crate）與
//! headless 的 `aiterm-host` CLI 使用，而後者要能在一台沒有任何 GUI 函式庫
//! 的伺服器上編譯並執行。任何 `tauri` 的 import 都會讓那件事失效。
```

- [ ] **Step 2: 把 app package 變成 workspace root**

在 `src-tauri/Cargo.toml` 的 `[package]` 區塊**之前**加入：

```toml
[workspace]
members = [".", "crates/aiterm-core"]
```

workspace root 刻意放在 `src-tauri/` 而不是 repo 根目錄：放 repo 根的話 `target/` 會整個搬家，而 worktree 是靠 symlink `src-tauri/target` 共用的（數十 GB 的重編成本），CI 路徑與 `tauri-build` 也都得跟著改。

- [ ] **Step 3: 驗證編得過**

```bash
cd src-tauri && cargo check -p aiterm-core && cargo check -p app
```

預期：兩個都成功。`app` 不該有任何新的警告。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/crates/aiterm-core/
git commit -m "build: introduce an aiterm-core workspace member"
```

---

## Task 3: 搬移 pty 的無 Tauri 模組

**這是純搬移。搬完之後既有測試必須一個都不用改就全綠——如果有測試需要改，那就不是搬移，停下來說明改了什麼。**

**Files:**
- Move: `src-tauri/src/pty/{session,shell,ansi,cd_parser,error,events}.rs` → `src-tauri/crates/aiterm-core/src/pty/`
- Modify: `src-tauri/crates/aiterm-core/Cargo.toml`
- Create: `src-tauri/crates/aiterm-core/src/pty/mod.rs`

- [ ] **Step 1: 搬檔**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
mkdir -p src-tauri/crates/aiterm-core/src/pty
git mv src-tauri/src/pty/session.rs   src-tauri/crates/aiterm-core/src/pty/session.rs
git mv src-tauri/src/pty/shell.rs     src-tauri/crates/aiterm-core/src/pty/shell.rs
git mv src-tauri/src/pty/ansi.rs      src-tauri/crates/aiterm-core/src/pty/ansi.rs
git mv src-tauri/src/pty/cd_parser.rs src-tauri/crates/aiterm-core/src/pty/cd_parser.rs
git mv src-tauri/src/pty/error.rs     src-tauri/crates/aiterm-core/src/pty/error.rs
git mv src-tauri/src/pty/events.rs    src-tauri/crates/aiterm-core/src/pty/events.rs
```

- [ ] **Step 2: 建立 core 的 pty/mod.rs**

`src-tauri/crates/aiterm-core/src/pty/mod.rs`：

```rust
pub mod ansi;
pub mod cd_parser;
pub mod error;
pub mod events;
pub mod session;
pub mod shell;

pub use error::{PtyError, PtyResult};
```

在 `src-tauri/crates/aiterm-core/src/lib.rs` 的註解之後加上：

```rust
pub mod pty;
```

- [ ] **Step 3: 補 core 的依賴**

`src-tauri/crates/aiterm-core/Cargo.toml` 的 `[dependencies]` 改成：

```toml
[dependencies]
portable-pty = "0.8"
uuid = { version = "1", features = ["v4", "serde"] }
anyhow = "1"
thiserror = "1"
parking_lot = "0.12"
tokio = { version = "1", features = ["sync", "rt-multi-thread", "macros", "process", "io-util", "net", "time"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = { version = "1.0", features = ["preserve_order"] }
log = "0.4"
dirs = "6"
```

- [ ] **Step 4: app crate 轉接**

`src-tauri/src/pty/mod.rs` 改成：

```rust
//! GUI 專屬的 PTY 接線。核心實作住在 `aiterm-core`——見那邊的 `pty` 模組。
//!
//! 這裡只留下依賴 Tauri 的東西：`#[tauri::command]` 進入點，以及把 PTY 輸出
//! 接到 Tauri 事件的 `manager`。

pub mod commands;
pub mod manager;

pub use aiterm_core::pty::{ansi, cd_parser, error, events, session, shell};
pub use aiterm_core::pty::{PtyError, PtyResult};
pub use manager::PtyManager;
```

`src-tauri/Cargo.toml` 的 `[dependencies]` 加：

```toml
aiterm-core = { path = "crates/aiterm-core" }
```

- [ ] **Step 5: 修 use 路徑**

`manager.rs` 目前用 `use super::error::…`、`use super::session::PtySession` 等相對路徑。因為 `pty/mod.rs` 有 `pub use`，這些 `super::` 路徑仍然解析得到，理論上不用改。實際跑編譯確認；若有解析不到的，改成 `use aiterm_core::pty::…` 的絕對路徑，**不要**改動搬過去的檔案內容。

- [ ] **Step 6: 驗證**

```bash
cd src-tauri && cargo test -p aiterm-core && cargo test -p app --lib
```

預期：`aiterm-core` 的測試（`session`／`shell`／`ansi`／`cd_parser` 裡的那些）全綠；`app` 的 lib 測試維持與搬移前相同的通過數。

- [ ] **Step 7: 確認搬移沒改行為**

```bash
git diff --cached -M --stat
```

預期：搬過去的六個檔案顯示為 `R100`（100% rename，內容零改動）。若不是 100%，看 diff 說明改了什麼——只有 `use` 路徑的調整是可接受的。

- [ ] **Step 8: Commit**

```bash
git add -A src-tauri/src/pty src-tauri/crates/aiterm-core src-tauri/Cargo.toml
git commit -m "refactor(pty): move the Tauri-free PTY modules into aiterm-core"
```

---

## Task 4: 搬移 PtyManager，把 create_with_app 留在 app

`PtyManager` 只有 `create_with_app`（`manager.rs:29`）依賴 Tauri；`create_with_callback`（:64）本來就沒有。`share::server` 需要 `PtyManager`，所以它必須進 core。

**Files:**
- Modify: `src-tauri/src/pty/manager.rs`
- Create: `src-tauri/crates/aiterm-core/src/pty/manager.rs`

- [ ] **Step 1: 寫失敗測試——core 的 manager 要能用指定 id 建 session**

`create_with_app` 需要「先產生 id、再用它 spawn」才能組出事件名稱。搬移後這個能力要由 core 提供。加到 `src-tauri/crates/aiterm-core/src/pty/manager.rs` 的 `mod tests`（檔案下一步才建，先把測試內容記在這裡）：

```rust
    #[test]
    fn create_with_callback_and_id_uses_the_id_it_was_given() {
        // GUI 端要先有 id 才能組出 `pty://data/{id}` 的事件名稱，再把發事件的
        // closure 傳進來。若這支自己另外產一個 id，事件會發到一個沒有人在聽的
        // 名字上，分頁永遠停在「initializing…」——而且不會有任何錯誤。
        let manager = PtyManager::new();
        let (tx, _rx) = mpsc::channel::<Vec<u8>>();
        let id = manager
            .create_with_callback_and_id(
                PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
                "chosen-id".to_string(),
                None,
                Vec::new(),
                Vec::new(),
                move |chunk| {
                    let _ = tx.send(chunk);
                },
            )
            .expect("create session");
        assert_eq!(id, "chosen-id");
        assert_eq!(manager.marker_count("chosen-id"), Some(0));
    }
```

- [ ] **Step 2: 搬檔並改造**

```bash
git mv src-tauri/src/pty/manager.rs src-tauri/crates/aiterm-core/src/pty/manager.rs
```

在 core 的 `manager.rs`：刪掉 `use tauri::{AppHandle, Emitter};` 與 `use base64::…`，刪掉整個 `create_with_app`，改成加入這支：

```rust
    /// 用指定的 id 與環境變數調整 spawn 一個 session，輸出丟給 `on_data`。
    ///
    /// **id 由呼叫端給**：GUI 端要先有 id 才能組出 `pty://data/{id}` 的事件
    /// 名稱，再把發事件的 closure 傳進來。
    ///
    /// `envs` / `env_removals` 讓呼叫端注入自己的環境變數（GUI 用它接
    /// Claude Code 橋接）——core 不知道也不該知道那是什麼。
    pub fn create_with_callback_and_id<F>(
        &self,
        size: PtySize,
        id: String,
        cwd: Option<PathBuf>,
        envs: Vec<(String, String)>,
        env_removals: Vec<String>,
        on_data: F,
    ) -> PtyResult<String>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        let mut shell: ShellSpec = default_shell().ok_or(PtyError::NoShellAvailable)?;
        shell.envs.extend(envs);
        shell.env_removals.extend(env_removals);
        let session = PtySession::spawn_with_id(shell, size, id.clone(), cwd, on_data)?;
        self.sessions.lock().insert(id.clone(), Arc::new(session));
        Ok(id)
    }
```

把 Step 1 的測試加進 `mod tests`。

在 `src-tauri/crates/aiterm-core/src/pty/mod.rs` 加 `pub mod manager;` 與 `pub use manager::PtyManager;`。

- [ ] **Step 3: app 端重建 create_with_app**

`src-tauri/src/pty/manager.rs`（新檔，取代搬走的那個）：

```rust
//! GUI 專屬：把一個 PTY session 的輸出接到 Tauri 事件上。
//!
//! session 的管理本身住在 `aiterm_core::pty::manager`。這裡只有「輸出去哪裡」
//! 這一件事——而那正是唯一依賴 Tauri 的部分。

use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use portable_pty::PtySize;
use tauri::{AppHandle, Emitter};

use aiterm_core::pty::error::PtyResult;
use aiterm_core::pty::events::{data_event_name, PtyDataPayload};
use aiterm_core::pty::PtyManager;

/// Spawn 一個 session 並把輸出接到 `pty://data/{id}` 事件。
///
/// `bridge_env` 非 None 時，把 Claude Code 橋接的環境變數注入這個分頁。
/// 環境變數只能在 spawn 的瞬間決定，所以事後無法對已開的分頁切換。
pub fn create_with_app(
    manager: &PtyManager,
    app: AppHandle,
    size: PtySize,
    cwd: Option<PathBuf>,
    bridge_env: Option<(u16, String)>,
) -> PtyResult<String> {
    let (envs, env_removals) = match bridge_env {
        Some((port, token)) => (
            crate::bridge::env::bridge_envs(port, &token),
            crate::bridge::env::ENV_TO_REMOVE.iter().map(|s| s.to_string()).collect(),
        ),
        None => (Vec::new(), Vec::new()),
    };

    let id = uuid::Uuid::new_v4().to_string();
    let event_name = data_event_name(&id);

    manager.create_with_callback_and_id(size, id, cwd, envs, env_removals, move |chunk| {
        let payload = PtyDataPayload { base64: BASE64.encode(&chunk) };
        if let Err(e) = app.emit(&event_name, payload) {
            eprintln!("emit {event_name} failed: {e}");
        }
    })
}
```

注意 `bridge_envs` 目前回傳的型別要能 `extend` 進 `Vec<(String, String)>`；若它回傳的是別的形狀（例如 iterator），在這裡 `.collect()` 成 `Vec<(String, String)>`。

`src-tauri/src/pty/mod.rs` 的 `pub use manager::PtyManager;` 改成：

```rust
pub use aiterm_core::pty::PtyManager;
pub use manager::create_with_app;
```

- [ ] **Step 4: 改呼叫端**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM && grep -rn "create_with_app" src-tauri/src
```

每一處 `manager.create_with_app(app, size, cwd, bridge)` 改成
`crate::pty::create_with_app(&manager, app, size, cwd, bridge)`。

- [ ] **Step 5: 驗證**

```bash
cd src-tauri && cargo test -p aiterm-core pty::manager && cargo test -p app --lib
```

預期：新測試 PASS，其餘維持原本的通過數。

- [ ] **Step 6: 手動冒煙（必要——這條路徑沒有自動化測試涵蓋）**

```bash
npm run tauri:dev
```

開一個新分頁，確認提示字元出現、能打指令。這驗證的是 `pty://data/{id}` 事件名稱沒有在改造中錯位——那個失效模式是「分頁永遠停在 initializing…、零錯誤訊息」，任何自動化測試都抓不到。

- [ ] **Step 7: Commit**

```bash
git add -A src-tauri/src/pty src-tauri/crates/aiterm-core
git commit -m "refactor(pty): move PtyManager into aiterm-core, keep the Tauri wiring in app"
```

---

## Task 5: 搬移 share 的無 Tauri 模組

`registry.rs`、`tls.rs`、`mdns.rs`、`protocol.rs` 對 Tauri 的參照數是 0，純搬移。

**Files:**
- Move: `src-tauri/src/share/{protocol,registry,tls,mdns}.rs` → `src-tauri/crates/aiterm-core/src/share/`
- Create: `src-tauri/crates/aiterm-core/src/share/mod.rs`（暫時只有模組宣告）

- [ ] **Step 1: 搬檔**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
mkdir -p src-tauri/crates/aiterm-core/src/share
git mv src-tauri/src/share/protocol.rs src-tauri/crates/aiterm-core/src/share/protocol.rs
git mv src-tauri/src/share/registry.rs src-tauri/crates/aiterm-core/src/share/registry.rs
git mv src-tauri/src/share/tls.rs      src-tauri/crates/aiterm-core/src/share/tls.rs
git mv src-tauri/src/share/mdns.rs     src-tauri/crates/aiterm-core/src/share/mdns.rs
```

- [ ] **Step 2: core 的 share/mod.rs**

```rust
//! 遠端終端機共享：協定、短碼註冊表、TLS 身分與 SAS、mDNS 廣播。
//!
//! server 端（`server`）與觀看端（`viewer`）都住在這裡；把事件推播給 GUI
//! 的那一層留在 `app` crate 的 `share::viewer_manager`。

pub mod mdns;
pub mod protocol;
pub mod registry;
pub mod tls;

/// rustls 0.23 要求行程層級的預設加密供應者。裝一次就好；重複呼叫會回
/// `Err`，直接忽略——那代表別人已經裝過了，不是錯誤。
pub fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
```

注意 `ensure_crypto_provider` 原本是 `pub(crate)`（`src-tauri/src/share/mod.rs:34`），跨 crate 之後必須是 `pub`。

`src-tauri/crates/aiterm-core/src/lib.rs` 加 `pub mod share;`。

- [ ] **Step 3: 補依賴**

`src-tauri/crates/aiterm-core/Cargo.toml` 的 `[dependencies]` 加上（版本與 `src-tauri/Cargo.toml` 現有的完全一致，不要自己挑新版）：

```toml
rand = "0.9"
base64 = "0.22"
sha2 = "0.10"
rcgen = "0.13"
rustls = "0.23"
tokio-rustls = "0.26"
mdns-sd = "0.11"
```

先跑 `grep -n 'rustls\|tokio-rustls\|mdns-sd' src-tauri/Cargo.toml` 抄實際版本號，上面的數字是預期值不是保證值。

- [ ] **Step 4: app 端轉接**

`src-tauri/src/share/mod.rs` 的模組宣告改成：

```rust
pub mod server;
pub mod viewer;
pub mod viewer_manager;

pub use aiterm_core::share::{ensure_crypto_provider, mdns, protocol, registry, tls};
```

（`server`／`viewer` 還在 app 裡，下一個任務才搬。）

- [ ] **Step 5: 驗證**

```bash
cd src-tauri && cargo test -p aiterm-core share && cargo test -p app --lib && cargo test -p app --test '*'
```

預期：搬過去的測試全綠，整合測試（`src-tauri/tests/`）也全綠。**這裡一定要跑整合測試**——`--lib` 不編譯 `tests/`，而 share 的整合測試正是碰這些型別最多的地方。

- [ ] **Step 6: Commit**

```bash
git add -A src-tauri/src/share src-tauri/crates/aiterm-core src-tauri/Cargo.toml
git commit -m "refactor(share): move protocol/registry/tls/mdns into aiterm-core"
```

---

## Task 6: ShareEvents trait

`share::server` 用 `Option<AppHandle>` 做兩件事：推播「有人要連進來」（`server.rs:192-202`）與「觀看者名單變了」（`server.rs:418-419`）。換成 trait 才能搬進 core。

**Files:**
- Create: `src-tauri/crates/aiterm-core/src/share/events.rs`
- Modify: `src-tauri/src/share/server.rs`

- [ ] **Step 1: 寫失敗測試**

`src-tauri/crates/aiterm-core/src/share/events.rs`：

```rust
//! 把「有事情發生了」告訴上層的介面。
//!
//! server 本身不知道上層是 GUI（要發 Tauri 事件）還是無人值守的 CLI
//! （沒有任何 UI 要更新）。原本這裡是 `Option<tauri::AppHandle>`，那讓
//! server 綁死在 Tauri 上，headless 的 CLI host 因此不可能重用它。

use super::protocol::PendingRequestEvent;

pub trait ShareEvents: Send + Sync + 'static {
    /// 有人送出連線請求，正在等裁決。
    fn pending_request(&self, ev: &PendingRequestEvent);
    /// 觀看者名單或其存取層級變動了。
    fn viewers_changed(&self);
}

/// 什麼都不做的實作，給無人值守的 CLI host 用。
///
/// 刻意寫成一個有名字的型別而不是讓 server 收 `Option<impl ShareEvents>`：
/// 「CLI 模式下這些事件去哪了」應該在程式碼裡看得見，而不是靠一個 None。
pub struct SilentEvents;

impl ShareEvents for SilentEvents {
    fn pending_request(&self, _ev: &PendingRequestEvent) {}
    fn viewers_changed(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Default)]
    struct Counting {
        pending: AtomicUsize,
        viewers: AtomicUsize,
    }

    impl ShareEvents for Counting {
        fn pending_request(&self, _ev: &PendingRequestEvent) {
            self.pending.fetch_add(1, Ordering::SeqCst);
        }
        fn viewers_changed(&self) {
            self.viewers.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn a_share_events_impl_can_be_held_as_a_trait_object() {
        // server 會把它存成 `Arc<dyn ShareEvents>`。trait 若不是 object-safe
        // （例如哪天有人加了泛型方法），這裡會編譯失敗——那比在 server 那個
        // 大檔案裡發現要好。
        let counting = Arc::new(Counting::default());
        let as_dyn: Arc<dyn ShareEvents> = counting.clone();
        as_dyn.pending_request(&PendingRequestEvent {
            request_id: "r1".to_string(),
            tab_id: "t1".to_string(),
            display_name: "Alice".to_string(),
        });
        as_dyn.viewers_changed();
        assert_eq!(counting.pending.load(Ordering::SeqCst), 1);
        assert_eq!(counting.viewers.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn silent_events_does_nothing_and_does_not_panic() {
        let silent: Arc<dyn ShareEvents> = Arc::new(SilentEvents);
        silent.pending_request(&PendingRequestEvent {
            request_id: "r1".to_string(),
            tab_id: "t1".to_string(),
            display_name: "Alice".to_string(),
        });
        silent.viewers_changed();
    }
}
```

在 `src-tauri/crates/aiterm-core/src/share/mod.rs` 加 `pub mod events;`。

- [ ] **Step 2: 跑測試確認會紅**

```bash
cd src-tauri && cargo test -p aiterm-core share::events
```

預期：編譯失敗（`events` 模組還沒被宣告 / `PendingRequestEvent` 路徑不對）。修到 PASS。

- [ ] **Step 3: 改 server 用 trait**

`src-tauri/src/share/server.rs`：

`ShareAppState` 的 `app` 欄位改成：

```rust
    /// 用來把「有人要連進來」推播給上層。整合測試與 headless 的 CLI host
    /// 傳 `SilentEvents`——所有事件發送都是 no-op，其餘行為完全一樣。
    pub events: Arc<dyn ShareEvents>,
```

`router` 的簽章改成：

```rust
pub fn router(
    pty: Arc<PtyManager>,
    registry: Arc<ShareRegistry>,
    events: Arc<dyn ShareEvents>,
) -> Router {
```

`server.rs:192-202` 那段改成：

```rust
    // 推播給上層，讓同意視窗跳出來。`SilentEvents` 時是 no-op。
    state.events.pending_request(&PendingRequestEvent {
        request_id: request_id.clone(),
        tab_id: tab_id.clone(),
        display_name: display_name_for_event.clone(),
    });
```

`server.rs:418-419` 那段改成：

```rust
    state.events.viewers_changed();
```

- [ ] **Step 4: app 端實作**

新增 `src-tauri/src/share/tauri_events.rs`：

```rust
//! `ShareEvents` 的 GUI 實作：把事件轉成 Tauri 事件送給前端。

use aiterm_core::share::events::ShareEvents;
use aiterm_core::share::protocol::PendingRequestEvent;
use tauri::{AppHandle, Emitter};

pub struct TauriShareEvents {
    pub app: AppHandle,
}

impl ShareEvents for TauriShareEvents {
    fn pending_request(&self, ev: &PendingRequestEvent) {
        let _ = self.app.emit("share://request-pending", ev.clone());
    }

    fn viewers_changed(&self) {
        let _ = self.app.emit("share://viewers-changed", ());
    }
}
```

`PendingRequestEvent` 目前只 derive `Debug, Clone, Serialize`——`emit` 需要 `Serialize`，已經有了，`Clone` 也有，不用改。

`src-tauri/src/share/mod.rs` 加 `pub mod tauri_events;`，並把 `start_if_needed` / `start_if_needed_on_port` 的 `app: Option<tauri::AppHandle>` 參數改成 `events: Arc<dyn ShareEvents>`，往下傳給 `server::router`。

- [ ] **Step 5: 改呼叫端**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM && grep -rn "start_if_needed\|server::router" src-tauri/src src-tauri/tests
```

GUI 呼叫端傳 `Arc::new(TauriShareEvents { app })`；整合測試裡原本傳 `None` 的改傳 `Arc::new(SilentEvents)`。

- [ ] **Step 6: 驗證**

```bash
cd src-tauri && cargo test -p aiterm-core && cargo test -p app --lib && cargo test -p app --test '*'
```

- [ ] **Step 7: Commit**

```bash
git add -A src-tauri/src/share src-tauri/crates/aiterm-core src-tauri/tests
git commit -m "refactor(share): replace the AppHandle event sink with a ShareEvents trait"
```

---

## Task 7: 搬移 server 與 viewer，位址參數化

`server.rs` 現在已經沒有 Tauri 依賴了，可以搬。順手把寫死的綁定位址提升成參數。

**Files:**
- Move: `src-tauri/src/share/{server,viewer}.rs` → `src-tauri/crates/aiterm-core/src/share/`
- Modify: `src-tauri/crates/aiterm-core/src/share/mod.rs`

- [ ] **Step 1: 寫失敗測試——綁 127.0.0.1 就不該從別的介面連得到**

加到 `src-tauri/crates/aiterm-core/src/share/mod.rs` 的 `mod tests`：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn binding_to_loopback_does_not_listen_on_all_interfaces() {
        // `--bind 127.0.0.1` 的意義就是「不要暴露在網路上」。若位址參數被忽略、
        // 實際還是綁 0.0.0.0，這個承諾就是假的，而使用者不會有任何跡象——
        // 從 loopback 連得上，看起來一切正常。
        let state = ShareServerState::new();
        let pty = Arc::new(crate::pty::PtyManager::new());
        let port = state
            .start_if_needed_on(pty, std::net::Ipv4Addr::LOCALHOST, 0, Arc::new(events::SilentEvents))
            .await
            .expect("server starts");

        // loopback 連得上
        assert!(
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await.is_ok(),
            "loopback should be reachable"
        );

        // 但實際綁定的位址必須是 loopback，不是萬用位址
        assert_eq!(state.bound_addr().map(|a| a.ip()), Some(std::net::Ipv4Addr::LOCALHOST.into()));
    }
}
```

- [ ] **Step 2: 搬檔**

```bash
git mv src-tauri/src/share/server.rs src-tauri/crates/aiterm-core/src/share/server.rs
git mv src-tauri/src/share/viewer.rs src-tauri/crates/aiterm-core/src/share/viewer.rs
```

把 `src-tauri/src/share/mod.rs` 裡 `ShareServerState`／`Running`／`serve_tls`／`ensure_crypto_provider` 這整套搬進 `src-tauri/crates/aiterm-core/src/share/mod.rs`（`ensure_crypto_provider` 在 Task 5 已經在那裡了，不要留兩份）。app 端的 `src-tauri/src/share/mod.rs` 縮成：

```rust
//! GUI 專屬的共享接線。協定與 server 住在 `aiterm-core`。

pub mod tauri_events;
pub mod viewer_manager;

pub use aiterm_core::share::{
    ensure_crypto_provider, events, mdns, protocol, registry, server, tls, viewer,
    ShareServerState,
};
```

- [ ] **Step 3: 位址參數化**

core 的 `share/mod.rs`：

```rust
    /// 啟動 server（若尚未啟動），綁在指定的位址與 port。
    ///
    /// `port` 為 `0` 表示交給 OS 挑。位址之所以是參數而不是寫死 `0.0.0.0`：
    /// headless 的 CLI host 常常跑在只有一張網卡該被暴露的機器上，而
    /// `--bind 127.0.0.1` 配 SSH tunnel 是那台機器上最保守的用法。
    pub async fn start_if_needed_on(
        &self,
        pty: Arc<PtyManager>,
        addr: std::net::Ipv4Addr,
        port: u16,
        events: Arc<dyn ShareEvents>,
    ) -> anyhow::Result<u16> {
        if let Some(p) = self.port() {
            return Ok(p);
        }
        ensure_crypto_provider();
        let listener = tokio::net::TcpListener::bind(SocketAddr::from((addr, port))).await?;
        let bound = listener.local_addr()?;
        let app_router = server::router(pty, Arc::clone(&self.registry), events);
        let identity = tls::ShareIdentity::generate()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(serve_tls(listener, app_router, identity, rx));
        let mdns = match mdns::MdnsAdvertiser::start() {
            Ok(a) => Some(a),
            Err(e) => {
                log::warn!("mDNS daemon 啟動失敗，這次分享不會被自動發現：{e}");
                None
            }
        };
        *self.running.lock() = Some(Running { port: bound.port(), bound, shutdown: tx, mdns });
        Ok(bound.port())
    }

    /// 實際綁定到的位址，沒在跑時回 `None`。
    pub fn bound_addr(&self) -> Option<SocketAddr> {
        self.running.lock().as_ref().map(|r| r.bound)
    }
```

`Running` 加一個 `bound: SocketAddr` 欄位。既有的兩支保留為包裝：

```rust
    pub async fn start_if_needed(
        &self,
        pty: Arc<PtyManager>,
        events: Arc<dyn ShareEvents>,
    ) -> anyhow::Result<u16> {
        self.start_if_needed_on(pty, std::net::Ipv4Addr::UNSPECIFIED, 0, events).await
    }

    pub async fn start_if_needed_on_port(
        &self,
        pty: Arc<PtyManager>,
        port: u16,
        events: Arc<dyn ShareEvents>,
    ) -> anyhow::Result<u16> {
        self.start_if_needed_on(pty, std::net::Ipv4Addr::UNSPECIFIED, port, events).await
    }
```

**mDNS 這裡先維持原樣**（永遠嘗試啟動），CLI 的 `--advertise` 開關在 Task 12 處理——CLI host 根本不會呼叫 `mdns_register`，沒註冊就不會廣播任何服務，daemon 本身起不起來無關緊要。

- [ ] **Step 4: 跑測試**

```bash
cd src-tauri && cargo test -p aiterm-core share::tests::binding_to_loopback_does_not_listen_on_all_interfaces
```

預期：PASS。

- [ ] **Step 4b: 把 `OUTPUT_RING_CAP` 收回 `pub(crate)`**

Task 3 為了讓還留在 app crate 的 `share/server.rs:34` 讀得到
`crate::pty::session::OUTPUT_RING_CAP`，把它從 `pub(crate)` 放寬成 `pub`。
`server.rs` 這一步搬進 core 之後，那個理由就消失了。

```bash
cd src-tauri && grep -rn "OUTPUT_RING_CAP" src/ crates/
```

確認 app crate 底下**沒有**任何命中之後，把
`crates/aiterm-core/src/pty/session.rs:113` 改回：

```rust
pub(crate) const OUTPUT_RING_CAP: usize = 256 * 1024;
```

這種「為了過渡期而放寬、之後沒人收回去」的可見度是會永久留下的——沒有任何
測試會因為它太寬而變紅，所以不在這裡收，就再也不會收了。

- [ ] **Step 5: 全面驗證**

```bash
cd src-tauri && cargo test -p aiterm-core && cargo test -p app --lib && cargo test -p app --test '*' && cargo clippy -p app -p aiterm-core -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add -A src-tauri/src/share src-tauri/crates/aiterm-core src-tauri/tests
git commit -m "refactor(share): move server/viewer into aiterm-core and parameterize the bind address"
```

---

## Task 8: share::auth — HMAC 證明

**Files:**
- Create: `src-tauri/crates/aiterm-core/src/share/auth.rs`
- Modify: `src-tauri/crates/aiterm-core/src/share/tls.rs`
- Modify: `src-tauri/crates/aiterm-core/Cargo.toml`

- [ ] **Step 1: 加依賴**

`src-tauri/crates/aiterm-core/Cargo.toml`：

```toml
hmac = "0.12"
```

（`sha2` 在 Task 5 已經加了。`hmac` 的 `Mac::verify_slice` 本身就是 constant-time，不需要另外引入 `subtle`。）

- [ ] **Step 2: tls.rs 加 auth 專用的 exporter label**

在 `SAS_EXPORTER_LABEL` 旁邊加：

```rust
/// CLI host 金鑰互證專用的 exporter label。
///
/// **刻意不重用 `SAS_EXPORTER_LABEL`。** 同一份秘密同時餵給兩個不同用途的
/// 建構是跨協定攻擊的標準溫床：SAS 那份會以 4 位數的形式呈現在人眼前，
/// 這份則直接決定要不要放行一條連線，兩者的暴露程度完全不同。
pub const AUTH_EXPORTER_LABEL: &[u8] = b"EXPERIMENTAL aiterm cli-host auth v1";
```

把 `exporter_material` 重構成吃 label 的版本，並保留原簽章當包裝：

```rust
pub fn exporter_material_with_label<Data>(
    conn: &rustls::ConnectionCommon<Data>,
    label: &[u8],
) -> anyhow::Result<[u8; SAS_MATERIAL_LEN]> {
    conn.export_keying_material([0u8; SAS_MATERIAL_LEN], label, None)
        .map_err(|e| anyhow::anyhow!("export_keying_material: {e}"))
}

pub fn exporter_material<Data>(
    conn: &rustls::ConnectionCommon<Data>,
) -> anyhow::Result<[u8; SAS_MATERIAL_LEN]> {
    exporter_material_with_label(conn, SAS_EXPORTER_LABEL)
}
```

（實際的泛型參數與 where 子句照抄現有 `exporter_material` 的簽章，上面是形狀示意。）

- [ ] **Step 3: 寫失敗測試**

`src-tauri/crates/aiterm-core/src/share/auth.rs`：

```rust
//! CLI host 的預共享金鑰互證。
//!
//! GUI 主控端的身分保證來自「觀看端唸出 4 位 SAS、主控端的人核對」——那需要
//! 一個人在旁邊。headless 的 CLI host 沒有這個人，所以改用一組長期金鑰。
//!
//! **金鑰不上線。** 兩端各自對「自己那條 TLS 連線的 exporter material」做
//! HMAC，只把結果送出去。中間人終止 TLS 之後手上是兩條不同的連線、兩份不同
//! 的 exporter，所以它既算不出正確的證明，原封轉發也不成立。
//!
//! **互證是必要的，不是加分。** 只驗觀看端的話，中間人雖然偽造不出觀看端的
//! 證明，但它可以乾脆自己扮演主控端：直接回 `Granted`、餵假畫面、收走使用者
//! 打的每一個鍵。觀看端不驗憑證（見 `share::viewer` 的 `SasIsTheOnlyIdentityCheck`），
//! 所以「對面真的握有金鑰」必須由主控端那份證明提供。

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// 金鑰長度。32 bytes = 256 bit，暴力搜尋不成立。
pub const KEY_LEN: usize = 32;

/// 觀看端證明的 domain separator。
const VIEWER_LABEL: &[u8] = b"aiterm-viewer-v1";
/// 主控端證明的 domain separator。
///
/// **兩個方向必須用不同的 label。** 相同的話，中間人可以把主控端送來的證明
/// 原封當成觀看端的證明送回去（反射攻擊），不需要知道金鑰就能通過。
const HOST_LABEL: &[u8] = b"aiterm-host-v1";

fn proof(key: &[u8], label: &[u8], exporter: &[u8]) -> String {
    let mut mac = <Hmac<Sha256>>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(label);
    mac.update(exporter);
    super::tls::hex_of(&mac.finalize().into_bytes())
}

fn verify(key: &[u8], label: &[u8], exporter: &[u8], candidate: &str) -> bool {
    let Some(bytes) = super::tls::decode_hex(candidate) else { return false };
    let mut mac = <Hmac<Sha256>>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(label);
    mac.update(exporter);
    // `verify_slice` 是 constant-time 的，不要換成 `==`。
    mac.verify_slice(&bytes).is_ok()
}

/// 觀看端送給主控端的證明。
pub fn viewer_proof(key: &[u8], exporter: &[u8]) -> String {
    proof(key, VIEWER_LABEL, exporter)
}

/// 主控端驗證觀看端的證明。
pub fn verify_viewer_proof(key: &[u8], exporter: &[u8], candidate: &str) -> bool {
    verify(key, VIEWER_LABEL, exporter, candidate)
}

/// 主控端送給觀看端的證明。
pub fn host_proof(key: &[u8], exporter: &[u8]) -> String {
    proof(key, HOST_LABEL, exporter)
}

/// 觀看端驗證主控端的證明。
pub fn verify_host_proof(key: &[u8], exporter: &[u8], candidate: &str) -> bool {
    verify(key, HOST_LABEL, exporter, candidate)
}

/// 產生一組新金鑰。
pub fn generate_key() -> [u8; KEY_LEN] {
    use rand::RngCore;
    let mut key = [0u8; KEY_LEN];
    rand::rng().fill_bytes(&mut key);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
    const OTHER_KEY: &[u8] = b"fedcba9876543210fedcba9876543210";
    const EXPORTER: &[u8] = b"exporter-material-of-connection-1";
    const OTHER_EXPORTER: &[u8] = b"exporter-material-of-connection-2";

    #[test]
    fn a_correct_viewer_proof_verifies() {
        let p = viewer_proof(KEY, EXPORTER);
        assert!(verify_viewer_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_correct_host_proof_verifies() {
        let p = host_proof(KEY, EXPORTER);
        assert!(verify_host_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn the_wrong_key_is_rejected() {
        let p = viewer_proof(OTHER_KEY, EXPORTER);
        assert!(!verify_viewer_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_proof_from_another_connection_is_rejected() {
        // 這是整個機制的核心：中間人終止 TLS 之後，它跟觀看端那條連線的
        // exporter 跟它跟主控端那條連線的 exporter 不同，所以原封轉發不成立。
        // 這條測試若壞了，防中間人保證整個歸零而不會有任何其他徵兆。
        let p = viewer_proof(KEY, OTHER_EXPORTER);
        assert!(!verify_viewer_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_host_proof_cannot_be_replayed_as_a_viewer_proof() {
        // 反射攻擊：把主控端送來的證明原封當成觀看端的證明送回去。
        // 兩個方向用相同的 label 就會通過——那不需要知道金鑰。
        let p = host_proof(KEY, EXPORTER);
        assert!(!verify_viewer_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_viewer_proof_cannot_be_replayed_as_a_host_proof() {
        let p = viewer_proof(KEY, EXPORTER);
        assert!(!verify_host_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_malformed_proof_is_rejected_rather_than_panicking() {
        assert!(!verify_viewer_proof(KEY, EXPORTER, "not-hex"));
        assert!(!verify_viewer_proof(KEY, EXPORTER, ""));
        assert!(!verify_viewer_proof(KEY, EXPORTER, "ab"));
    }

    #[test]
    fn two_generated_keys_differ() {
        assert_ne!(generate_key(), generate_key());
    }
}
```

在 `src-tauri/crates/aiterm-core/src/share/mod.rs` 加 `pub mod auth;`。

- [ ] **Step 4: 跑測試確認會紅，再修到綠**

```bash
cd src-tauri && cargo test -p aiterm-core share::auth
```

先確認在 `auth.rs` 還沒建立時是編譯失敗；建好之後全部 PASS（8 條）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/aiterm-core/src/share/auth.rs src-tauri/crates/aiterm-core/src/share/mod.rs src-tauri/crates/aiterm-core/src/share/tls.rs src-tauri/crates/aiterm-core/Cargo.toml
git commit -m "feat(share): add pre-shared-key mutual proofs bound to the TLS exporter"
```

---

## Task 9: 協定加上可選的 auth 欄位

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/share/protocol.rs`

- [ ] **Step 1: 寫失敗測試**

加到 `protocol.rs` 的 `mod tests`：

```rust
    #[test]
    fn a_join_without_auth_serializes_exactly_as_before() {
        // 短碼模式送出的 JSON 必須跟加這個欄位之前逐位元組相同，否則舊版
        // 主控端收到的東西就變了——而它們早就裝在別人機器上。
        let msg = ClientMessage::Join {
            protocol_version: PROTOCOL_VERSION,
            code: "384719".to_string(),
            display_name: "Alice".to_string(),
            auth: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("auth"), "auth must be omitted when None; got {json}");
    }

    #[test]
    fn a_join_with_auth_round_trips() {
        let msg = ClientMessage::Join {
            protocol_version: PROTOCOL_VERSION,
            code: String::new(),
            display_name: "Alice".to_string(),
            auth: Some("ab".repeat(32)),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"auth\":"), "got {json}");
        let back: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn a_v2_join_without_the_auth_field_still_parses() {
        // 舊版觀看端送來的東西。`#[serde(default)]` 讓它落在 None。
        let old = r#"{"type":"join","protocol_version":2,"code":"384719","display_name":"Alice"}"#;
        let back: ClientMessage = serde_json::from_str(old).unwrap();
        assert_eq!(
            back,
            ClientMessage::Join {
                protocol_version: 2,
                code: "384719".to_string(),
                display_name: "Alice".to_string(),
                auth: None,
            }
        );
    }

    #[test]
    fn granted_omits_host_auth_when_absent() {
        let msg = ServerMessage::Granted {
            mode: WireAccessMode::Control,
            cols: 120,
            rows: 40,
            host_os: "linux".to_string(),
            host_auth: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("host_auth"), "got {json}");
    }

    #[test]
    fn granted_round_trips_with_host_auth() {
        let msg = ServerMessage::Granted {
            mode: WireAccessMode::Control,
            cols: 120,
            rows: 40,
            host_os: "linux".to_string(),
            host_auth: Some("cd".repeat(32)),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn the_protocol_version_stays_at_two() {
        // 升版本會讓 v1.24 的使用者連不上 v1.25 的同事——`server.rs` 的檢查
        // 是嚴格相等，而原始碼註解明講版本落差在區網分享裡是常態。CLI host
        // 的認證刻意設計成可選的附加欄位就是為了不動這個數字。
        assert_eq!(PROTOCOL_VERSION, 2);
    }
```

- [ ] **Step 2: 跑測試確認會紅**

```bash
cd src-tauri && cargo test -p aiterm-core share::protocol
```

預期：編譯失敗（`Join` 沒有 `auth` 欄位、`Granted` 沒有 `host_auth`）。

- [ ] **Step 3: 改協定**

`ClientMessage::Join` 改成：

```rust
    Join {
        protocol_version: u32,
        code: String,
        display_name: String,
        /// CLI host 的預共享金鑰證明（見 `share::auth`）。
        ///
        /// **可選的附加欄位，`PROTOCOL_VERSION` 刻意不動。** 升版本會讓
        /// `server.rs` 的嚴格相等檢查把所有跨版本的 GUI 分享一起擋掉，而
        /// 那在區網裡是常態情境。舊版主控端收到這個欄位會忽略它，照常走
        /// 短碼流程。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<String>,
    },
```

`ServerMessage::Granted` 加：

```rust
        /// CLI host 對觀看端的證明（見 `share::auth`）。
        ///
        /// 觀看端在金鑰模式下**必須驗過這個才渲染畫面、才送出按鍵**。沒有它
        /// 的話中間人可以直接扮演主控端：它偽造不出觀看端的證明，但它不需要
        /// ——它可以自己回 `Granted`、餵假畫面、收走每一個按鍵。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_auth: Option<String>,
```

- [ ] **Step 4: 修所有建構點**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM && grep -rn "ClientMessage::Join\|ServerMessage::Granted" src-tauri/src src-tauri/crates src-tauri/tests
```

每一處補上 `auth: None` / `host_auth: None`（真正要帶值的地方在 Task 10、11 處理）。

- [ ] **Step 5: 跑測試**

```bash
cd src-tauri && cargo test -p aiterm-core && cargo test -p app --lib && cargo test -p app --test '*'
```

預期：Step 1 的六條新測試全 PASS，既有測試維持原本的通過數。

- [ ] **Step 6: Commit**

```bash
git add -A src-tauri/crates/aiterm-core src-tauri/src src-tauri/tests
git commit -m "feat(share): add optional auth fields to Join and Granted without bumping the version"
```

---

## Task 10: server 端接上金鑰認證

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/share/server.rs`
- Test: `src-tauri/crates/aiterm-core/src/share/server.rs`（`mod tests`）

- [ ] **Step 1: 定義 HostAuth**

在 `server.rs` 的 `ShareAppState` 上方加：

```rust
/// CLI host 模式的認證設定。`ShareAppState::auth` 是 `None` 時走既有的短碼 +
/// 人工 SAS 核對，一行行為都不變。
pub struct HostAuth {
    /// 預共享金鑰。
    pub key: Vec<u8>,
    /// 這台 CLI host 的合成短碼——由 `ShareRegistry::start_share` 產生，
    /// **不印給使用者、不由觀看端提供**。存在的理由純粹是 registry 以短碼
    /// 為索引；認證通過後拿它去 `request_join`，registry 的觀看者與控制權
    /// 邏輯就完全不用改。
    pub code: String,
    /// 認證通過後自動核准的存取層級（`--read-only` 決定）。
    pub mode: AccessMode,
}
```

`ShareAppState` 加欄位：

```rust
    pub auth: Option<Arc<HostAuth>>,
```

`router` 多一個參數 `auth: Option<Arc<HostAuth>>`。

- [ ] **Step 2: 寫失敗測試**

`server.rs` 的 `mod tests`（若還沒有就新建）：

```rust
#[cfg(test)]
mod auth_tests {
    use super::*;
    use crate::share::auth;

    fn host_auth(key: &[u8], code: &str) -> Arc<HostAuth> {
        Arc::new(HostAuth {
            key: key.to_vec(),
            code: code.to_string(),
            mode: AccessMode::Control,
        })
    }

    #[test]
    fn a_valid_proof_resolves_to_the_hosts_own_code() {
        let key = auth::generate_key();
        let exporter = [7u8; tls::SAS_MATERIAL_LEN];
        let ha = host_auth(&key, "999999");
        let proof = auth::viewer_proof(&key, &exporter);

        let decision = decide_join(Some(&ha), Some(proof.as_str()), "", &exporter);
        // 觀看端送的 code 是空字串，但實際使用的必須是 host 自己的短碼。
        assert_eq!(decision, JoinDecision::AutoApprove { code: "999999".to_string(), mode: AccessMode::Control });
    }

    #[test]
    fn a_missing_proof_is_rejected_in_key_mode() {
        // 舊版觀看端（或短碼模式的觀看端）連上 CLI host 的情況。
        let key = auth::generate_key();
        let exporter = [7u8; tls::SAS_MATERIAL_LEN];
        let ha = host_auth(&key, "999999");
        assert_eq!(decide_join(Some(&ha), None, "384719", &exporter), JoinDecision::Reject);
    }

    #[test]
    fn a_wrong_proof_is_rejected_in_key_mode() {
        let key = auth::generate_key();
        let other = auth::generate_key();
        let exporter = [7u8; tls::SAS_MATERIAL_LEN];
        let ha = host_auth(&key, "999999");
        let proof = auth::viewer_proof(&other, &exporter);
        assert_eq!(decide_join(Some(&ha), Some(proof.as_str()), "", &exporter), JoinDecision::Reject);
    }

    #[test]
    fn without_host_auth_a_join_always_takes_the_code_path() {
        // GUI 主控端：即使觀看端多送了 auth 欄位也一律忽略，走短碼 + SAS。
        let exporter = [7u8; tls::SAS_MATERIAL_LEN];
        assert_eq!(
            decide_join(None, Some("deadbeef"), "384719", &exporter),
            JoinDecision::UseCode { code: "384719".to_string() }
        );
        assert_eq!(
            decide_join(None, None, "384719", &exporter),
            JoinDecision::UseCode { code: "384719".to_string() }
        );
    }
}
```

- [ ] **Step 3: 跑測試確認會紅**

```bash
cd src-tauri && cargo test -p aiterm-core share::server::auth_tests
```

預期：編譯失敗（`decide_join` / `JoinDecision` / `HostAuth` 不存在）。

- [ ] **Step 4: 實作 decide_join**

`server.rs`：

```rust
/// 一則 `Join` 該怎麼處理。
///
/// 抽成自由函式（而不是寫在 `handle_share` 裡）是為了能在不起 TLS、不起
/// server 的情況下測試每一條分支——這裡是整個 CLI host 唯一改變核准語意的
/// 地方，值得被單獨釘住。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinDecision {
    /// 走既有的短碼 + 人工核准流程。
    UseCode { code: String },
    /// 金鑰驗過了：用這個短碼加入，並立刻以這個層級自動核准。
    AutoApprove { code: String, mode: AccessMode },
    /// 拒絕。
    Reject,
}

pub fn decide_join(
    host_auth: Option<&HostAuth>,
    proof: Option<&str>,
    client_code: &str,
    exporter: &[u8],
) -> JoinDecision {
    let Some(ha) = host_auth else {
        // GUI 主控端：一律走短碼。觀看端多送的 auth 欄位直接忽略。
        return JoinDecision::UseCode { code: client_code.to_string() };
    };
    match proof {
        Some(p) if crate::share::auth::verify_viewer_proof(&ha.key, exporter, p) => {
            JoinDecision::AutoApprove { code: ha.code.clone(), mode: ha.mode }
        }
        _ => JoinDecision::Reject,
    }
}
```

- [ ] **Step 5: 接進 handle_share**

`handle_share` 裡解構 Join 的那段（`server.rs:127`）加上 `auth`：

```rust
            Ok(ClientMessage::Join { protocol_version, code, display_name, auth }) => {
                (protocol_version, code, display_name, auth)
            }
```

在版本檢查之後、SAS 承諾流程之前，插入：

```rust
    // CLI host 模式：金鑰驗過就自動核准，不需要人在旁邊唸碼。SAS 承諾流程
    // 照樣走完（訊息序列刻意一個字都不變），只是觀看端不會顯示那 4 位數。
    let decision = decide_join(state.auth.as_deref(), auth.as_deref(), &code, &exporter);
    let (effective_code, auto_approve) = match decision {
        JoinDecision::UseCode { code } => (code, None),
        JoinDecision::AutoApprove { code, mode } => (code, Some(mode)),
        // 刻意用既有的 `Denied` 而不是新增 EndReason 變體：新變體會讓舊版
        // 觀看端在 `serde_json::from_str` 硬性失敗，變成無法解釋的斷線。
        JoinDecision::Reject => return end_with(&mut ws, EndReason::Denied).await,
    };
```

接著把後面所有用 `&code` 的地方改用 `&effective_code`（`request_join`、兩處 `tab_for_code`、等待迴圈裡的 `tab_for_code`）。

在 `state.events.pending_request(...)` 之後、`AwaitingApproval` 之前插入：

```rust
    // 金鑰模式：立刻核准，等待迴圈下一輪就會看到 viewer 已建立。
    if let Some(mode) = auto_approve {
        state.registry.approve(&request_id, mode);
    }
```

`Granted` 的建構（`server.rs:285-288`）改成：

```rust
    if !send_control(
        &mut ws,
        &ServerMessage::Granted {
            mode,
            cols,
            rows,
            host_os: std::env::consts::OS.to_string(),
            host_auth: state
                .auth
                .as_ref()
                .map(|a| crate::share::auth::host_proof(&a.key, &exporter)),
        },
    )
    .await
```

- [ ] **Step 6: 跑測試**

```bash
cd src-tauri && cargo test -p aiterm-core && cargo test -p app --lib && cargo test -p app --test '*'
```

- [ ] **Step 7: Commit**

```bash
git add -A src-tauri/crates/aiterm-core src-tauri/src src-tauri/tests
git commit -m "feat(share): auto-approve key-authenticated joins on the server"
```

---

## Task 10b: 認證失敗退避

CLI host 的埠會被丟在公開網路上。256-bit 金鑰爆破不現實，但每一次失敗的嘗試都要走完 TLS 握手、產生一組 nonce、寫一行 log——不設限的話那本身就是一條放大管道。

**Files:**
- Create: `src-tauri/crates/aiterm-core/src/share/backoff.rs`
- Modify: `src-tauri/crates/aiterm-core/src/share/server.rs`

- [ ] **Step 1: 寫失敗測試**

`src-tauri/crates/aiterm-core/src/share/backoff.rs`：

```rust
//! 認證失敗的來源退避。
//!
//! 不是為了防爆破（256-bit 金鑰不需要），是為了擋 log flooding 與握手階段的
//! 資源耗用——CLI host 的埠常常直接暴露在網路上。

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// 失敗幾次之後開始延遲。前幾次不罰，因為使用者貼錯一次金鑰是常態。
const FREE_ATTEMPTS: u32 = 3;
/// 每次延遲的基數；實際延遲是 `BASE * 2^(failures - FREE_ATTEMPTS)`，上限 `MAX`。
const BASE: Duration = Duration::from_millis(500);
const MAX: Duration = Duration::from_secs(30);
/// 多久沒有新的失敗就把紀錄清掉。
const FORGET_AFTER: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct AuthBackoff {
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl AuthBackoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// 這個來源現在該被延遲多久。
    pub fn delay_for(&self, ip: IpAddr, now: Instant) -> Duration {
        let mut map = self.failures.lock();
        map.retain(|_, (_, last)| now.duration_since(*last) < FORGET_AFTER);
        let Some((count, _)) = map.get(&ip) else { return Duration::ZERO };
        if *count <= FREE_ATTEMPTS {
            return Duration::ZERO;
        }
        let exp = (*count - FREE_ATTEMPTS).min(16);
        BASE.saturating_mul(1u32 << exp).min(MAX)
    }

    pub fn record_failure(&self, ip: IpAddr, now: Instant) {
        let mut map = self.failures.lock();
        let entry = map.entry(ip).or_insert((0, now));
        entry.0 += 1;
        entry.1 = now;
    }

    /// 認證成功就清掉這個來源的紀錄。
    pub fn record_success(&self, ip: IpAddr) {
        self.failures.lock().remove(&ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, n])
    }

    #[test]
    fn a_fresh_source_is_not_delayed() {
        let b = AuthBackoff::new();
        assert_eq!(b.delay_for(ip(1), Instant::now()), Duration::ZERO);
    }

    #[test]
    fn the_first_few_failures_are_free() {
        // 貼錯一次金鑰是常態，不該讓使用者等。
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..FREE_ATTEMPTS {
            b.record_failure(ip(1), now);
        }
        assert_eq!(b.delay_for(ip(1), now), Duration::ZERO);
    }

    #[test]
    fn the_delay_grows_after_the_free_attempts() {
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..(FREE_ATTEMPTS + 1) {
            b.record_failure(ip(1), now);
        }
        let first = b.delay_for(ip(1), now);
        assert!(first > Duration::ZERO, "got {first:?}");

        b.record_failure(ip(1), now);
        let second = b.delay_for(ip(1), now);
        assert!(second > first, "delay must grow: {first:?} -> {second:?}");
    }

    #[test]
    fn the_delay_is_capped() {
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..100 {
            b.record_failure(ip(1), now);
        }
        assert_eq!(b.delay_for(ip(1), now), MAX);
    }

    #[test]
    fn one_source_does_not_delay_another() {
        // 沒有這條的話，任何人都能用一台機器狂試金鑰，把合法使用者一起鎖住。
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..50 {
            b.record_failure(ip(1), now);
        }
        assert_eq!(b.delay_for(ip(2), now), Duration::ZERO);
    }

    #[test]
    fn a_success_clears_the_record() {
        let b = AuthBackoff::new();
        let now = Instant::now();
        for _ in 0..50 {
            b.record_failure(ip(1), now);
        }
        b.record_success(ip(1));
        assert_eq!(b.delay_for(ip(1), now), Duration::ZERO);
    }

    #[test]
    fn old_records_are_forgotten() {
        let b = AuthBackoff::new();
        let start = Instant::now();
        for _ in 0..50 {
            b.record_failure(ip(1), start);
        }
        let much_later = start + FORGET_AFTER + Duration::from_secs(1);
        assert_eq!(b.delay_for(ip(1), much_later), Duration::ZERO);
    }
}
```

在 `share/mod.rs` 加 `pub mod backoff;`。

- [ ] **Step 2: 跑測試**

```bash
cd src-tauri && cargo test -p aiterm-core share::backoff
```

預期：七條全 PASS。

- [ ] **Step 3: 接進 server**

`ShareAppState` 加 `pub backoff: Arc<backoff::AuthBackoff>`（`router` 自己建一個，不用外面傳）。

要拿到來源 IP：`share_upgrade` 加一個 `axum::extract::ConnectInfo<SocketAddr>` 的 extractor。**這需要 `serve_tls` 的 accept 迴圈把 peer 位址放進 request extension**——目前它已經在放 `ConnectionExporter`（`share/mod.rs:216`），照同樣的方式再放一個。

`handle_share` 裡 `JoinDecision::Reject` 那個分支改成：

```rust
        JoinDecision::Reject => {
            let delay = state.backoff.delay_for(peer_ip, std::time::Instant::now());
            state.backoff.record_failure(peer_ip, std::time::Instant::now());
            log::warn!("認證失敗：來源 {peer_ip}，自報名稱 {display_name:?}");
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            return end_with(&mut ws, EndReason::Denied).await;
        }
```

`JoinDecision::AutoApprove` 的分支加一行 `state.backoff.record_success(peer_ip);`。

延遲**放在回覆之前**而不是接受連線之前：那讓攻擊者的每一次嘗試都必須佔著一條連線等完，才是真正壓低嘗試速率的作法。

- [ ] **Step 4: 驗證**

```bash
cd src-tauri && cargo test -p aiterm-core && cargo test -p app --lib && cargo test -p app --test '*'
```

- [ ] **Step 5: Commit**

```bash
git add -A src-tauri/crates/aiterm-core src-tauri/src
git commit -m "feat(share): back off repeated auth failures per source address"
```

---

## Task 11: 觀看端帶上證明並驗主控端

**Files:**
- Modify: `src-tauri/crates/aiterm-core/src/share/viewer.rs`

- [ ] **Step 1: 寫失敗測試**

加到 `viewer.rs` 的 `mod tests`（若沒有就新建）：

```rust
#[cfg(test)]
mod host_auth_tests {
    use super::*;
    use crate::share::auth;

    #[test]
    fn in_key_mode_a_missing_host_proof_is_refused() {
        // 中間人扮演主控端最省事的作法就是根本不送這個欄位。若這裡放行，
        // 它就能餵假畫面並收走每一個按鍵，而使用者看不出任何異狀。
        let key = auth::generate_key();
        let exporter = [3u8; crate::share::tls::SAS_MATERIAL_LEN];
        assert!(!host_proof_acceptable(Some(&key), &exporter, None));
    }

    #[test]
    fn in_key_mode_a_wrong_host_proof_is_refused() {
        let key = auth::generate_key();
        let other = auth::generate_key();
        let exporter = [3u8; crate::share::tls::SAS_MATERIAL_LEN];
        let p = auth::host_proof(&other, &exporter);
        assert!(!host_proof_acceptable(Some(&key), &exporter, Some(&p)));
    }

    #[test]
    fn in_key_mode_a_correct_host_proof_is_accepted() {
        let key = auth::generate_key();
        let exporter = [3u8; crate::share::tls::SAS_MATERIAL_LEN];
        let p = auth::host_proof(&key, &exporter);
        assert!(host_proof_acceptable(Some(&key), &exporter, Some(&p)));
    }

    #[test]
    fn in_code_mode_the_absence_of_a_host_proof_is_fine() {
        // 短碼模式：GUI 主控端不會送這個欄位，身分保證來自人工 SAS 核對。
        let exporter = [3u8; crate::share::tls::SAS_MATERIAL_LEN];
        assert!(host_proof_acceptable(None, &exporter, None));
    }

    #[test]
    fn in_code_mode_an_unexpected_host_proof_is_ignored() {
        let exporter = [3u8; crate::share::tls::SAS_MATERIAL_LEN];
        assert!(host_proof_acceptable(None, &exporter, Some("deadbeef")));
    }
}
```

- [ ] **Step 2: 跑測試確認會紅**

```bash
cd src-tauri && cargo test -p aiterm-core share::viewer::host_auth_tests
```

預期：編譯失敗（`host_proof_acceptable` 不存在）。

- [ ] **Step 3: 實作**

`viewer.rs`：

```rust
/// 主控端送來的證明可不可以接受。
///
/// `key` 是 `None`（短碼模式）時永遠回 `true`——身分保證來自人工 SAS 核對，
/// 這個欄位在那個模式下本來就不會出現。
pub fn host_proof_acceptable(key: Option<&[u8]>, exporter: &[u8], candidate: Option<&str>) -> bool {
    let Some(key) = key else { return true };
    match candidate {
        Some(p) => crate::share::auth::verify_host_proof(key, exporter, p),
        None => false,
    }
}
```

`ViewerHandshake` 加兩個欄位：

```rust
    /// 這條連線的 exporter material，留給串流階段驗主控端的證明用。
    pub exporter: [u8; tls::SAS_MATERIAL_LEN],
    /// 金鑰模式的金鑰；短碼模式是 `None`。
    pub key: Option<Vec<u8>>,
```

`connect_and_handshake` 加一個參數 `key: Option<&[u8]>`，並在送 Join 時帶上證明：

```rust
    send_json(
        &mut ws,
        &ClientMessage::Join {
            protocol_version: PROTOCOL_VERSION,
            // 金鑰模式沒有短碼——身分完全由金鑰決定。送空字串的副作用是好的：
            // 拿金鑰模式的連線去指一台 GUI 主控端，會在 `tab_for_code("")`
            // 乾淨地失敗成 `InvalidCode`。
            code: if key.is_some() { String::new() } else { code.to_string() },
            display_name: display_name.to_string(),
            auth: key.map(|k| crate::share::auth::viewer_proof(k, &exporter)),
        },
    )
    .await?;
```

回傳處：

```rust
    Ok(ViewerHandshake { sas, ws, exporter, key: key.map(|k| k.to_vec()) })
```

`run_viewer_stream` 加兩個參數 `key: Option<Vec<u8>>` 與 `exporter: [u8; tls::SAS_MATERIAL_LEN]`，處理 `Granted` 的分支改成：

```rust
                        ServerMessage::Granted { mode, cols, rows, host_os, host_auth } => {
                            if !host_proof_acceptable(key.as_deref(), &exporter, host_auth.as_deref()) {
                                // 不送 Granted，也不再讀任何 Data——中間人扮演
                                // 主控端時，這裡是唯一擋得住的地方。
                                let _ = events.send(ViewerEvent::Ended {
                                    reason: "host_auth_failed".to_string(),
                                });
                                break;
                            }
                            let _ = events.send(ViewerEvent::Granted {
                                mode: wire_mode_str(mode),
                                cols,
                                rows,
                                host_os,
                            });
                        }
```

注意 `Resize` 那個分支也建構 `ViewerEvent::Granted`，但它來自 `ServerMessage::Resize`，沒有 `host_auth` 欄位，不受影響。

- [ ] **Step 4: 跑測試**

```bash
cd src-tauri && cargo test -p aiterm-core share::viewer
```

預期：五條新測試 PASS。

- [ ] **Step 5: 改呼叫端**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM && grep -rn "connect_and_handshake\|run_viewer_stream" src-tauri/src src-tauri/crates src-tauri/tests
```

`viewer_manager.rs` 的 `connect` 加 `key: Option<String>` 參數（hex 字串），解成 bytes 後往下傳；`run_viewer_stream` 的呼叫補上 `handshake.key` 與 `handshake.exporter`。

- [ ] **Step 6: 全面驗證**

```bash
cd src-tauri && cargo test -p aiterm-core && cargo test -p app --lib && cargo test -p app --test '*'
```

- [ ] **Step 7: Commit**

```bash
git add -A src-tauri/crates/aiterm-core src-tauri/src src-tauri/tests
git commit -m "feat(share): make the viewer prove and verify the pre-shared key"
```

---

## Task 12: 金鑰檔

**Files:**
- Create: `src-tauri/crates/aiterm-host/Cargo.toml`
- Create: `src-tauri/crates/aiterm-host/src/keyfile.rs`
- Create: `src-tauri/crates/aiterm-host/src/main.rs`（先只放 `mod keyfile;`）
- Modify: `src-tauri/Cargo.toml`（workspace members）

- [ ] **Step 1: 建 crate**

`src-tauri/crates/aiterm-host/Cargo.toml`：

```toml
[package]
name = "aiterm-host"
version = "0.1.0"
edition = "2021"
rust-version = "1.88"

[[bin]]
name = "aiterm-host"
path = "src/main.rs"

[dependencies]
aiterm-core = { path = "../aiterm-core" }
anyhow = "1"
clap = { version = "4", features = ["derive"] }
dirs = "6"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal", "time"] }
portable-pty = "0.8"
log = "0.4"
env_logger = "0.11"

[dev-dependencies]
tempfile = "3"
```

`src-tauri/Cargo.toml` 的 members 改成：

```toml
[workspace]
members = [".", "crates/aiterm-core", "crates/aiterm-host"]
```

`src-tauri/crates/aiterm-host/src/main.rs` 先放：

```rust
mod keyfile;

fn main() {}
```

- [ ] **Step 2: 寫失敗測試**

`src-tauri/crates/aiterm-host/src/keyfile.rs`：

```rust
//! 金鑰檔的讀取、產生與權限檢查。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use aiterm_core::share::auth::KEY_LEN;

/// 預設的金鑰檔位置。
pub fn default_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("找不到設定檔目錄")?;
    Ok(dir.join("aiterm-host").join("key"))
}

/// 讀出金鑰；檔案不存在就產生一組新的。
///
/// 回傳 `(金鑰, 是否為這次新產生的)`——呼叫端用第二個值決定要不要在啟動訊息
/// 裡特別提醒使用者「這是新金鑰，記得複製到 GUI」。
pub fn load_or_create(path: &Path) -> Result<(Vec<u8>, bool)> {
    if path.exists() {
        check_permissions(path)?;
        let hex = std::fs::read_to_string(path)
            .with_context(|| format!("讀不到金鑰檔 {}", path.display()))?;
        let key = aiterm_core::share::tls::decode_hex(hex.trim())
            .with_context(|| format!("金鑰檔 {} 的內容不是合法的 hex", path.display()))?;
        if key.len() != KEY_LEN {
            bail!("金鑰長度是 {} bytes，應該是 {KEY_LEN}：{}", key.len(), path.display());
        }
        return Ok((key, false));
    }

    let key = aiterm_core::share::auth::generate_key();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("建不出目錄 {}", parent.display()))?;
    }
    write_private(path, &aiterm_core::share::tls::hex_of(&key))
        .with_context(|| format!("寫不進金鑰檔 {}", path.display()))?;
    Ok((key.to_vec(), true))
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    // 先建成 0600 再寫，不要先寫完再 chmod——那之間有一個任何人都讀得到的窗口。
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

/// Unix 上檢查權限；比 0600 寬就拒絕，比照 ssh。
///
/// Windows 不做這個檢查——ACL 的語意跟 mode bits 不同，硬套會得到一個既
/// 擋不住真正的問題、又會誤擋正常設定的檢查。改為在啟動訊息裡明確指出
/// 金鑰檔的位置。
#[cfg(unix)]
pub fn check_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!(
            "金鑰檔 {} 的權限是 {mode:o}，其他人讀得到。請執行：chmod 600 {}",
            path.display(),
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn check_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creating_a_key_produces_one_of_the_right_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("key");
        let (key, created) = load_or_create(&path).unwrap();
        assert!(created);
        assert_eq!(key.len(), KEY_LEN);
        assert!(path.exists());
    }

    #[test]
    fn reading_an_existing_key_returns_the_same_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        let (first, created_first) = load_or_create(&path).unwrap();
        let (second, created_second) = load_or_create(&path).unwrap();
        assert!(created_first);
        assert!(!created_second);
        assert_eq!(first, second, "重啟後金鑰必須不變，否則 GUI 存的連線會失效");
    }

    #[test]
    fn a_malformed_key_file_is_an_error_rather_than_a_silent_zero_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "not hex at all").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(load_or_create(&path).is_err());
    }

    #[test]
    fn a_key_of_the_wrong_length_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "abcd").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(load_or_create(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_world_readable_key_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        let (key, _) = load_or_create(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_or_create(&path).unwrap_err();
        assert!(
            err.to_string().contains("chmod 600"),
            "錯誤訊息要直接給出修法，got: {err}"
        );
        // 不是因為讀不出來才失敗——金鑰本身是好的。
        assert_eq!(key.len(), KEY_LEN);
    }

    #[cfg(unix)]
    #[test]
    fn a_newly_created_key_file_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        load_or_create(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "產生金鑰時就要是 0600，不能事後補 chmod");
    }
}
```

- [ ] **Step 3: 跑測試**

```bash
cd src-tauri && cargo test -p aiterm-host
```

預期：七條（Unix 上）全 PASS。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/crates/aiterm-host/
git commit -m "feat(host): add the CLI host key file with ssh-style permission checks"
```

---

## Task 13: CLI 進入點

**Files:**
- Modify: `src-tauri/crates/aiterm-host/src/main.rs`

- [ ] **Step 1: 寫失敗測試——參數解析**

在 `main.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn the_default_port_is_fixed_not_random() {
        // GUI 裡存的連線記著一個 port。隨機 port（GUI 主控端的作法）會讓那筆
        // 連線在每次重啟後失效。
        let args = Args::parse_from(["aiterm-host"]);
        assert_eq!(args.port, 8022);
    }

    #[test]
    fn mdns_is_off_by_default() {
        // 跟 GUI 相反，且是刻意的：伺服器情境用不到自動發現，而廣播等於在
        // 辦公室網路上宣告「這裡有一個 shell」。
        let args = Args::parse_from(["aiterm-host"]);
        assert!(!args.advertise);
    }

    #[test]
    fn control_is_the_default_access_level() {
        let args = Args::parse_from(["aiterm-host"]);
        assert!(!args.read_only);
        assert_eq!(args.access_mode(), AccessMode::Control);
    }

    #[test]
    fn read_only_flips_the_access_level() {
        let args = Args::parse_from(["aiterm-host", "--read-only"]);
        assert_eq!(args.access_mode(), AccessMode::ReadOnly);
    }

    #[test]
    fn the_default_bind_is_all_interfaces() {
        let args = Args::parse_from(["aiterm-host"]);
        assert_eq!(args.bind, std::net::Ipv4Addr::UNSPECIFIED);
    }

    #[test]
    fn bind_accepts_loopback() {
        let args = Args::parse_from(["aiterm-host", "--bind", "127.0.0.1"]);
        assert_eq!(args.bind, std::net::Ipv4Addr::LOCALHOST);
    }
}
```

- [ ] **Step 2: 跑測試確認會紅**

```bash
cd src-tauri && cargo test -p aiterm-host tests::
```

預期：編譯失敗（`Args` 不存在）。

- [ ] **Step 3: 實作**

`src-tauri/crates/aiterm-host/src/main.rs`：

```rust
//! AITerm CLI Host：把這台機器的一個 shell 開放給桌面版 AITerm 連進來，
//! 讓觀看端用它自己的 AI 操作。
//!
//! 這個執行檔**不需要任何 AI 設定或 API key**——AI 跑在觀看端。

mod keyfile;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use portable_pty::PtySize;

use aiterm_core::pty::PtyManager;
use aiterm_core::share::events::SilentEvents;
use aiterm_core::share::registry::AccessMode;
use aiterm_core::share::server::HostAuth;
use aiterm_core::share::ShareServerState;

#[derive(Parser, Debug)]
#[command(name = "aiterm-host", about = "把一個 shell 開放給 AITerm 遠端連線與 AI 操作")]
struct Args {
    /// 監聽埠。刻意是固定的預設值而不是隨機——GUI 裡存的連線記著它。
    #[arg(long, default_value_t = 8022)]
    port: u16,

    /// 綁定位址。`127.0.0.1` 配 SSH tunnel 是最保守的用法。
    #[arg(long, default_value_t = Ipv4Addr::UNSPECIFIED)]
    bind: Ipv4Addr,

    /// 金鑰檔位置。預設 `<設定檔目錄>/aiterm-host/key`。
    #[arg(long)]
    key_file: Option<PathBuf>,

    /// 要跑的 shell。預設沿用 AITerm 既有的偵測。
    #[arg(long)]
    shell: Option<PathBuf>,

    /// shell 的起始工作目錄。
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// 連進來的人只能看，不能打字。
    #[arg(long)]
    read_only: bool,

    /// 開啟 mDNS 廣播。**預設關閉**，跟 GUI 相反：伺服器情境用不到自動發現，
    /// 而廣播等於在網路上宣告「這裡有一個 shell」。
    #[arg(long)]
    advertise: bool,

    /// 只印出連線資訊就退出，不開 shell。
    #[arg(long)]
    print_connection: bool,
}

impl Args {
    fn access_mode(&self) -> AccessMode {
        if self.read_only { AccessMode::ReadOnly } else { AccessMode::Control }
    }

    fn key_path(&self) -> Result<PathBuf> {
        match &self.key_file {
            Some(p) => Ok(p.clone()),
            None => keyfile::default_path(),
        }
    }
}

/// 金鑰的來源。環境變數優先於檔案——容器情境常常只能給環境變數，而在那種
/// 情況下檔案往往是不存在或唯讀的。
fn resolve_key(args: &Args) -> Result<(Vec<u8>, String)> {
    if let Ok(hex) = std::env::var("AITERM_HOST_KEY") {
        let key = aiterm_core::share::tls::decode_hex(hex.trim())
            .context("AITERM_HOST_KEY 不是合法的 hex")?;
        if key.len() != aiterm_core::share::auth::KEY_LEN {
            anyhow::bail!(
                "AITERM_HOST_KEY 是 {} bytes，應該是 {}",
                key.len(),
                aiterm_core::share::auth::KEY_LEN
            );
        }
        if args.key_file.is_some() {
            eprintln!("警告：同時給了 --key-file 與 AITERM_HOST_KEY，採用環境變數。");
        }
        return Ok((key, "AITERM_HOST_KEY".to_string()));
    }
    let path = args.key_path()?;
    let (key, created) = keyfile::load_or_create(&path)?;
    if created {
        eprintln!("已產生新金鑰：{}", path.display());
    }
    Ok((key, path.display().to_string()))
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    let (key, key_source) = resolve_key(&args)?;
    let key_hex = aiterm_core::share::tls::hex_of(&key);

    if args.print_connection {
        print_connection(&args, &key_hex, &key_source);
        return Ok(());
    }

    let pty = Arc::new(PtyManager::new());
    let session_id = "cli".to_string();
    let (cols, rows) = (120u16, 40u16);

    // 輸出丟掉：CLI host 自己不畫任何東西，觀看端是透過 `subscribe_with_history`
    // 拿畫面的，那條路徑不經過這個 callback。
    pty.create_with_callback_and_id(
        PtySize { rows, cols, pixel_width: 0, pixel_height: 0 },
        session_id.clone(),
        args.cwd.clone(),
        shell_override_envs(&args),
        Vec::new(),
        |_chunk| {},
    )
    .context("開不出 shell")?;

    let server = ShareServerState::new();
    // registry 以短碼為索引，所以還是要走一次 start_share。它回傳的 6 位短碼
    // 在金鑰模式下不印、不用——身分完全由金鑰決定。
    let code = server.registry.start_share(session_id.clone());

    let auth = Arc::new(HostAuth {
        key: key.clone(),
        code: code.clone(),
        mode: args.access_mode(),
    });

    let port = server
        .start_if_needed_on_with_auth(
            pty.clone(),
            args.bind,
            args.port,
            Arc::new(SilentEvents),
            Some(auth),
        )
        .await
        .with_context(|| format!("綁不上 {}:{}", args.bind, args.port))?;

    if args.advertise {
        server.mdns_register(&session_id, &code);
    }

    print_connection(&args, &key_hex, &key_source);
    println!("監聽中：{}:{port}", args.bind);

    wait_for_shutdown(&pty, &session_id).await;
    Ok(())
}

/// `--shell` 的實作：`pty::shell` 的偵測是看 `SHELL`／`COMSPEC` 這些環境
/// 變數的，所以覆寫的方式就是覆寫那個變數，而不是另外開一條 spawn 路徑。
fn shell_override_envs(args: &Args) -> Vec<(String, String)> {
    match &args.shell {
        #[cfg(unix)]
        Some(p) => vec![("SHELL".to_string(), p.display().to_string())],
        #[cfg(windows)]
        Some(p) => vec![("COMSPEC".to_string(), p.display().to_string())],
        None => Vec::new(),
    }
}

fn print_connection(args: &Args, key_hex: &str, key_source: &str) {
    println!("AITerm CLI Host");
    println!("  位址：{}", args.bind);
    println!("  埠　：{}", args.port);
    println!("  金鑰：{key_hex}");
    println!("  來源：{key_source}");
    println!("  存取：{}", if args.read_only { "唯讀" } else { "可控制" });
    println!();
    println!("在 AITerm 的「連線到遠端終端機」裡填入上面的位址、埠與金鑰。");
    println!("跨網段時位址請填這台機器對觀看端可達的位址（Tailscale / VPN / SSH tunnel）。");
}
```

`wait_for_shutdown` 見下一個任務。

- [ ] **Step 4: server 端接受 auth**

`aiterm-core` 的 `share/mod.rs` 加一支帶 auth 的啟動函式，既有的兩支轉呼叫它並傳 `None`：

```rust
    pub async fn start_if_needed_on_with_auth(
        &self,
        pty: Arc<PtyManager>,
        addr: std::net::Ipv4Addr,
        port: u16,
        events: Arc<dyn ShareEvents>,
        auth: Option<Arc<server::HostAuth>>,
    ) -> anyhow::Result<u16> {
        if let Some(p) = self.port() {
            return Ok(p);
        }
        ensure_crypto_provider();
        let listener = tokio::net::TcpListener::bind(SocketAddr::from((addr, port))).await?;
        let bound = listener.local_addr()?;
        let app_router = server::router(pty, Arc::clone(&self.registry), events, auth);
        let identity = tls::ShareIdentity::generate()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(serve_tls(listener, app_router, identity, rx));
        let mdns = match mdns::MdnsAdvertiser::start() {
            Ok(a) => Some(a),
            Err(e) => {
                log::warn!("mDNS daemon 啟動失敗，這次分享不會被自動發現：{e}");
                None
            }
        };
        *self.running.lock() = Some(Running { port: bound.port(), bound, shutdown: tx, mdns });
        Ok(bound.port())
    }
```

把 Task 7 寫的 `start_if_needed_on` 縮成轉呼叫這一支：

```rust
    pub async fn start_if_needed_on(
        &self,
        pty: Arc<PtyManager>,
        addr: std::net::Ipv4Addr,
        port: u16,
        events: Arc<dyn ShareEvents>,
    ) -> anyhow::Result<u16> {
        self.start_if_needed_on_with_auth(pty, addr, port, events, None).await
    }
```

只留一份啟動流程。兩份幾乎相同的版本必然會漂移——而漂移的那一份會是沒有測試涵蓋的 GUI 路徑。

- [ ] **Step 5: 跑測試**

```bash
cd src-tauri && cargo test -p aiterm-host && cargo test -p aiterm-core
```

- [ ] **Step 6: 手動冒煙**

```bash
cd src-tauri && cargo run -p aiterm-host -- --print-connection
```

預期：印出位址、埠 8022、64 個 hex 字元的金鑰，然後退出。再跑一次，金鑰**必須相同**。

- [ ] **Step 7: Commit**

```bash
git add -A src-tauri/crates
git commit -m "feat(host): add the aiterm-host CLI entry point"
```

---

## Task 14: 生命週期與訊號處理

**Files:**
- Modify: `src-tauri/crates/aiterm-host/src/main.rs`

- [ ] **Step 1: 寫失敗測試——shell 結束要能被偵測到**

`aiterm-core` 的 `PtyManager` 需要一支「這個 session 還活著嗎」。加到
`src-tauri/crates/aiterm-core/src/pty/manager.rs` 的 `mod tests`：

```rust
    #[test]
    fn a_session_reports_alive_until_it_is_closed() {
        // CLI host 靠這個判斷「shell 退出了，該收工了」。沒有它就只能輪詢
        // 輸出有沒有停——而一個閒置的 shell 跟一個結束的 shell 一樣安靜。
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
        assert_eq!(manager.is_alive(&id), Some(true));
        manager.close(&id).expect("close");
        assert_eq!(manager.is_alive(&id), None);
    }
```

- [ ] **Step 2: 跑測試確認會紅**

```bash
cd src-tauri && cargo test -p aiterm-core pty::manager::tests::a_session_reports_alive_until_it_is_closed
```

預期：編譯失敗（`is_alive` 不存在）。

- [ ] **Step 3: 實作 is_alive**

`PtyManager` 加：

```rust
    /// 這個 session 的紀錄還在不在（`None` = 已經被 `close` 掉或從未存在）。
    ///
    /// **這不是「子行程還活著嗎」。** shell 真正結束的訊號在 `subscribe` 的
    /// broadcast channel 被關閉——reader thread 在 PTY EOF 時結束、sender 被
    /// drop，接收端就拿到 `RecvError::Closed`。CLI host 等的是那個，不是這支
    /// （見 `wait_for_shutdown`）。這支的用途只有「這個 id 我還記著嗎」。
    pub fn is_alive(&self, id: &str) -> Option<bool> {
        self.sessions.lock().get(id).map(|_| true)
    }
```

- [ ] **Step 4: 實作 wait_for_shutdown**

`src-tauri/crates/aiterm-host/src/main.rs`：

```rust
/// 等到該收工為止：shell 自己結束，或收到終止訊號。
///
/// shell 結束的訊號取自 PTY 的 broadcast channel 被關閉——reader thread 在
/// PTY EOF 時結束、sender 被 drop，接收端就會拿到 `Closed`。這比輪詢輸出
/// 可靠：一個閒置的 shell 跟一個結束的 shell 一樣安靜。
async fn wait_for_shutdown(pty: &PtyManager, session_id: &str) {
    let mut rx = match pty.subscribe(session_id) {
        Some(rx) => rx,
        None => return,
    };

    let shell_ended = async {
        loop {
            match rx.recv().await {
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    tokio::select! {
        _ = shell_ended => {
            println!("shell 結束，收工。");
        }
        _ = terminate_signal() => {
            println!("收到終止訊號，關閉連線並收工。");
        }
    }

    // 收掉 PTY。觀看端會因為 `subscribe_with_history` 的 channel 關閉而收到
    // `SessionClosed`，這是既有行為，不需要另外送訊息。
    let _ = pty.close(session_id);
}

#[cfg(unix)]
async fn terminate_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(windows)]
async fn terminate_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
```

- [ ] **Step 5: 手動冒煙**

```bash
cd src-tauri && cargo run -p aiterm-host
```

在另一個終端機：

```bash
kill -TERM $(pgrep -f 'aiterm-host')
```

預期：主行程印出「收到終止訊號」並在一秒內退出，不留殭屍行程（用 `pgrep -f aiterm-host` 確認沒有殘留）。

- [ ] **Step 6: Commit**

```bash
git add -A src-tauri/crates
git commit -m "feat(host): shut down cleanly on shell exit and on SIGTERM/SIGINT"
```

---

## Task 15: 端到端整合測試

這是唯一同時驗證「金鑰認證真的能連上」與「遠端 AI agent 迴圈的硬性前提（`OSC 133;D`）在 CLI host 上成立」的測試。

**Files:**
- Create: `src-tauri/crates/aiterm-host/tests/cli_host.rs`

- [ ] **Step 1: 寫測試**

```rust
//! 端到端：起一個真的 CLI host server，用 `aiterm-core` 的觀看端客戶端連進去。
//!
//! 刻意不 spawn `aiterm-host` 執行檔本身——那會把「參數解析」跟「協定」綁在
//! 同一條測試上，失敗時分不清是哪一邊。參數解析在 `main.rs` 的單元測試裡
//! 已經釘住了。

use std::sync::Arc;
use std::time::Duration;

use aiterm_core::pty::PtyManager;
use aiterm_core::share::events::SilentEvents;
use aiterm_core::share::registry::AccessMode;
use aiterm_core::share::server::HostAuth;
use aiterm_core::share::viewer::{connect_and_handshake, run_viewer_stream, ViewerEvent};
use aiterm_core::share::{auth, ShareServerState};
use portable_pty::PtySize;

struct Harness {
    server: ShareServerState,
    port: u16,
    key: Vec<u8>,
    _pty: Arc<PtyManager>,
}

async fn start_host(mode: AccessMode) -> Harness {
    let pty = Arc::new(PtyManager::new());
    pty.create_with_callback_and_id(
        PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 },
        "cli".to_string(),
        None,
        Vec::new(),
        Vec::new(),
        |_| {},
    )
    .expect("shell spawns");

    let server = ShareServerState::new();
    let code = server.registry.start_share("cli".to_string());
    let key = auth::generate_key().to_vec();
    let host_auth = Arc::new(HostAuth { key: key.clone(), code, mode });

    let port = server
        .start_if_needed_on_with_auth(
            pty.clone(),
            std::net::Ipv4Addr::LOCALHOST,
            0,
            Arc::new(SilentEvents),
            Some(host_auth),
        )
        .await
        .expect("server starts");

    Harness { server, port, key, _pty: pty }
}

/// 連上並收事件，直到拿到 `Granted` 或 `Ended`。
async fn connect(
    port: u16,
    key: Option<&[u8]>,
) -> (
    tokio::sync::mpsc::UnboundedReceiver<ViewerEvent>,
    tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
) {
    let hs = connect_and_handshake("127.0.0.1", port, "", "test-viewer", key)
        .await
        .expect("handshake");
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (keys_tx, keys_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(run_viewer_stream(hs.ws, events_tx, keys_rx, hs.key, hs.exporter));
    (events_rx, keys_tx)
}

#[tokio::test]
async fn a_viewer_with_the_right_key_is_granted_without_any_human_approval() {
    let h = start_host(AccessMode::Control).await;
    let (mut events, _keys) = connect(h.port, Some(&h.key)).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("no event within 5s — the auto-approval path never fired")
        .expect("channel open");

    match ev {
        ViewerEvent::Granted { mode, .. } => assert_eq!(mode, "control"),
        other => panic!("expected Granted, got {other:?}"),
    }
    drop(h);
}

#[tokio::test]
async fn a_viewer_with_the_wrong_key_is_denied() {
    let h = start_host(AccessMode::Control).await;
    let wrong = auth::generate_key().to_vec();
    let (mut events, _keys) = connect(h.port, Some(&wrong)).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("no event within 5s")
        .expect("channel open");

    match ev {
        ViewerEvent::Ended { reason } => assert_eq!(reason, "denied"),
        other => panic!("expected Ended, got {other:?}"),
    }
}

#[tokio::test]
async fn a_viewer_with_no_key_is_denied() {
    // 舊版觀看端、或短碼模式的觀看端，指到一台 CLI host 的情況。
    let h = start_host(AccessMode::Control).await;
    let (mut events, _keys) = connect(h.port, None).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("no event within 5s")
        .expect("channel open");

    assert!(
        matches!(ev, ViewerEvent::Ended { .. }),
        "a keyless viewer must not get in; got {ev:?}"
    );
}

#[tokio::test]
async fn read_only_mode_is_reported_to_the_viewer() {
    let h = start_host(AccessMode::ReadOnly).await;
    let (mut events, _keys) = connect(h.port, Some(&h.key)).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("no event within 5s")
        .expect("channel open");

    match ev {
        ViewerEvent::Granted { mode, .. } => assert_eq!(mode, "read_only"),
        other => panic!("expected Granted, got {other:?}"),
    }
}

#[tokio::test]
async fn a_command_run_through_the_viewer_produces_an_osc_133_d_marker() {
    // 這是遠端 AI agent 迴圈的硬性前提：觀看端靠 `OSC 133;D`（含 exit code）
    // 判斷「這一步跑完了」。沒有它，每一步都要等 60 秒逾時，整個功能形同壞掉
    // ——而那不會有任何錯誤訊息。
    let h = start_host(AccessMode::Control).await;
    let (mut events, keys) = connect(h.port, Some(&h.key)).await;

    // 先等 Granted
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("no Granted within 5s")
            .expect("channel open");
        if matches!(ev, ViewerEvent::Granted { .. }) {
            break;
        }
    }

    // 等 shell 的第一個提示字元安定下來再送指令
    tokio::time::sleep(Duration::from_millis(1500)).await;
    keys.send(b"echo aiterm-marker-probe\n".to_vec()).expect("send keys");

    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!remaining.is_zero(), "no OSC 133;D within 15s; saw: {:?}", String::from_utf8_lossy(&seen));
        let Ok(Some(ev)) = tokio::time::timeout(remaining, events.recv()).await else {
            panic!("event stream ended before an OSC 133;D marker; saw: {:?}", String::from_utf8_lossy(&seen));
        };
        if let ViewerEvent::Data(bytes) = ev {
            seen.extend_from_slice(&bytes);
            // `\x1b]133;D` 後面接 `;<exit code>`。只找前綴就夠——exit code
            // 的有無在 `pty::cd_parser` 已經有自己的測試。
            if seen.windows(7).any(|w| w == b"\x1b]133;D") {
                return;
            }
        }
    }
}
```

- [ ] **Step 2: 跑測試**

```bash
cd src-tauri && cargo test -p aiterm-host --test cli_host
```

預期：五條全 PASS。

若 `a_command_run_through_the_viewer_produces_an_osc_133_d_marker` 失敗，**不要放寬斷言**——那代表 CLI host 起的 shell 沒有拿到 shell integration，遠端 AI 功能在 CLI host 上是壞的。先查 `pty::shell` 的注入在這條路徑上有沒有被繞過。

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/aiterm-host/tests/
git commit -m "test(host): end-to-end key auth and OSC 133 marker coverage"
```

---

## Task 16: GUI 觀看端加金鑰欄位

**Files:**
- Modify: `src-tauri/src/commands/share_viewer.rs`
- Modify: `src-tauri/src/share/viewer_manager.rs`
- Modify: `src/ipc/shareViewer.ts`
- Modify: `src/ipc/shareViewer.test.ts`
- Modify: `src/components/ConnectDialog/index.tsx`
- Modify: `src/components/ConnectDialog/index.test.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 寫失敗的前端測試**

`src/ipc/shareViewer.test.ts` 加：

```ts
  it("passes the key through to the backend when given", async () => {
    invokeMock.mockResolvedValue({ connId: "c1", sas: "" });
    await shareViewerConnect({
      host: "10.0.0.5",
      port: 8022,
      code: "",
      displayName: "James",
      key: "ab".repeat(32),
    });
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_connect", {
      host: "10.0.0.5",
      port: 8022,
      code: "",
      displayName: "James",
      key: "ab".repeat(32),
    });
  });

  it("omits the key for code-mode connections", async () => {
    // 短碼模式必須送 undefined 而不是空字串——空字串在後端會被當成「有金鑰
    // 但是空的」，握手直接失敗，而且錯誤訊息會指向金鑰不符，完全誤導。
    invokeMock.mockResolvedValue({ connId: "c1", sas: "1234" });
    await shareViewerConnect({
      host: "10.0.0.5",
      port: 8022,
      code: "384719",
      displayName: "James",
    });
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_connect", {
      host: "10.0.0.5",
      port: 8022,
      code: "384719",
      displayName: "James",
      key: undefined,
    });
  });
```

- [ ] **Step 2: 跑測試確認會紅**

```bash
npm run test -- src/ipc/shareViewer.test.ts
```

預期：FAIL（`shareViewerConnect` 還沒有 `key` 參數）。

- [ ] **Step 3: 改前端 IPC**

`src/ipc/shareViewer.ts`：

```ts
export interface ShareViewerConnectArgs {
  host: string;
  port: number;
  code: string;
  displayName: string;
  /** CLI host 的預共享金鑰（hex）。短碼模式留空。 */
  key?: string;
}

export function shareViewerConnect(args: ShareViewerConnectArgs): Promise<ViewerConnected> {
  return invoke<ViewerConnected>("share_viewer_connect", {
    host: args.host,
    port: args.port,
    code: args.code,
    displayName: args.displayName,
    key: args.key,
  });
}
```

既有呼叫端（`grep -rn "shareViewerConnect" src/`）改成傳物件。

- [ ] **Step 4: 改後端指令**

`src-tauri/src/commands/share_viewer.rs` 的 `share_viewer_connect` 加 `key: Option<String>` 參數，往下傳給 `ViewerManager::connect`。

`viewer_manager.rs` 的 `connect` 加同名參數，把 hex 解成 bytes：

```rust
        let key_bytes = match key.as_deref() {
            Some(hex) => Some(
                aiterm_core::share::tls::decode_hex(hex.trim())
                    .ok_or_else(|| anyhow::anyhow!("金鑰不是合法的 hex"))?,
            ),
            None => None,
        };
        let handshake =
            connect_and_handshake(&host, port, &code, &display_name, key_bytes.as_deref()).await?;
```

- [ ] **Step 5: UI 加欄位**

`src/components/ConnectDialog/index.tsx` 已經有 `code` / `name` / `manualOpen` / `address` 四個 `useState`（:27-30）。在旁邊加第五個：

```tsx
  const [key, setKey] = useState("");
```

欄位放在「手動位址」那一區裡（`manualOpen` 展開的區塊，:117 附近），因為金鑰模式一定是手動填位址的——mDNS 發現走的是短碼：

```tsx
            <label className="aiterm-connect__label" htmlFor="aiterm-connect-key">
              {t.connect_key_label}
            </label>
            <input
              id="aiterm-connect-key"
              className="aiterm-connect__input"
              value={key}
              onChange={(e) => setKey(e.target.value)}
              autoComplete="off"
              spellCheck={false}
            />
            <p className="aiterm-connect__hint">{t.connect_key_hint}</p>
```

送出時：

```tsx
    // 空字串要送 undefined，不能送 ""。後端把 Some("") 當成「有金鑰但是空的」，
    // 握手會失敗，而錯誤訊息會指向金鑰不符——對一個根本沒填金鑰的使用者來說
    // 完全誤導。
    key: key.trim() === "" ? undefined : key.trim(),
```

**金鑰的持久化**：存進既有的 `SecretStore`（`secret_set` / `secret_get` 指令），key 名用 `remote_host_key:<host>:<port>`。**不要用 localStorage**。

- [ ] **Step 5b: i18n**

`src/lib/i18n.ts` 加三筆（en 與 zh-TW 都要）。前兩筆是 `ConnectDialog` 的欄位文案：

```ts
  connect_key_label: { en: "Key (optional)", "zh-TW": "金鑰（選填）" },
  connect_key_hint: {
    en: "For CLI hosts. Leave blank to use a 6-digit code.",
    "zh-TW": "連 CLI host 用。留空則使用 6 位短碼。",
  },
```

第三筆的鍵名**不能自己取**。`src/components/RemoteTerminalView/index.tsx:801` 的 `endReasonText(t, reason)` 是用 `` `remote_terminal_ended_${reason}` `` 組出 i18n 鍵去查的，所以 `reason: "host_auth_failed"` 對應的鍵名固定是：

```ts
  remote_terminal_ended_host_auth_failed: {
    en: "The host's key does not match. Someone may be intercepting this connection.",
    "zh-TW": "主機金鑰不符，可能有人在中間攔截這條連線。",
  },
```

`endReasonText` 對認不得的 reason 有 fallback（:793 的註解說 spec 要求不能出現「未知錯誤」），所以漏掉這一筆不會爆炸——它會**安靜地退化成一句通用訊息**。這正是這一步唯一的失效模式，也是為什麼下面要有一條測試釘住它。

- [ ] **Step 5c: 釘住 reason 對照**

`src/components/RemoteTerminalView/index.test.tsx` 加：

```tsx
  it("shows the host-key mismatch message rather than a generic fallback", async () => {
    // endReasonText 對認不得的 reason 有 fallback，所以漏掉 i18n 那一筆不會
    // 讓任何測試變紅——只會讓使用者在一個真正該警覺的情境下看到一句通用訊息。
    // 這條測試就是那個缺口的唯一防線。
    const { t } = renderRemoteTerminal();
    emitEnded("host_auth_failed");
    expect(await screen.findByText(t.remote_terminal_ended_host_auth_failed)).toBeInTheDocument();
  });
```

（`renderRemoteTerminal` / `emitEnded` 用該檔案裡既有的 helper；若名字不同就照抄既有測試的掛載方式。）

- [ ] **Step 6: 驗證**

```bash
npm run test -- src/ipc/shareViewer.test.ts src/components/ConnectDialog src/components/RemoteTerminalView
npx tsc -b
npm run lint
cd src-tauri && cargo test -p app --lib && cargo test -p app --test '*'
```

`npx tsc -b`，**不是 `tsc --noEmit`**——根目錄的 `tsconfig.json` 是 solution file（`"files": []`），`--noEmit` 什麼都不檢查而且永遠回 0。

- [ ] **Step 7: Commit**

```bash
git add src/ src-tauri/src/commands/share_viewer.rs src-tauri/src/share/viewer_manager.rs
git commit -m "feat(remote): let the viewer connect to a CLI host with a pre-shared key"
```

---

## Task 17: 真機驗收

自動化測試涵蓋不到「兩個真的 app 對接」。這一步必須由使用者實際操作。

- [ ] **Step 1: 全套自動化驗證**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
npx tsc -b
npm run test
npm run lint
cd src-tauri && cargo test && cargo clippy --workspace -- -D warnings
```

**`cargo test` 不加 `--lib`**——`--lib` 不編譯 `tests/` 底下的整合測試，而這次改動最集中的地方正是那裡。

- [ ] **Step 2: 起 CLI host**

```bash
cd src-tauri && cargo run -p aiterm-host -- --bind 127.0.0.1
```

記下印出的埠與金鑰。

- [ ] **Step 3: 從 GUI 連上**

```bash
npm run tauri:dev
```

開「連線到遠端終端機」，填 `127.0.0.1`、埠、金鑰，短碼留空。預期：**不需要任何同意視窗、不需要唸碼**，直接看到 CLI host 那個 shell 的畫面。

- [ ] **Step 4: 驗 AI 迴圈**

在遠端終端機分頁開 `RemoteAiPanel`，下一個多步驟任務（例如「列出這個資料夾裡最大的三個檔案」）。預期：每一步都在合理時間內完成並進入下一步。**若每一步都卡約 60 秒才前進，就是 `OSC 133;D` 沒有生效**——那代表 Task 15 的標記測試有漏洞，回去查，不要當成效能問題。

- [ ] **Step 5: 驗錯誤路徑**

- 金鑰填錯一個字元 → 預期看到「主機金鑰不符」而不是逾時或無聲失敗。
- 金鑰留空、短碼也留空 → 預期看到「主控端拒絕」。
- CLI host 那邊按 Ctrl+C → 預期 GUI 立刻顯示連線結束，且 `pgrep -f aiterm-host` 沒有殘留。

- [ ] **Step 6: 驗重啟後連線仍有效**

重啟 CLI host（同樣的參數），GUI 用**同一筆**存好的連線再連一次。預期：直接連上，不需要重填金鑰。這是「金鑰就是身分」這個決策的驗收點。

- [ ] **Step 7: Commit**

```bash
git commit --allow-empty -m "chore: verify the CLI host end to end on a real machine"
```

---

## 完成後

用 `superpowers:finishing-a-development-branch` 決定怎麼收尾。接著是第二份計畫：發布管道（Releases + 安裝腳本 + musl、ghcr.io、Homebrew tap、npm）。
