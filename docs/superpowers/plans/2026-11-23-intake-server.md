# 問題收集 Server 版 ①② Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把問題收集的資料、規則與收訊抽成不依賴 Tauri 的 `aiterm-intake-core`，再做無頭 `aiterm-intake-server`（多專案、設定對等、成員 token 認證、HTTPS＋WebSocket），並讓 AITERM 能切到 Client 模式，大家看到同一份清單與狀態。

**Architecture:** 純邏輯模組搬進新 crate，`app` crate 用 `pub use` 轉出，既有 `crate::intake::store` 等路徑不變，因此搬移期間 AITERM 行為與測試不變。Server 是獨立二進位，用 axum＋rustls 提供 `/v1` API；Client 模式在 `commands.rs` 的每個指令前分流到 `RemoteClient`。

**Tech Stack:** Rust（axum 0.8、rustls 0.23、sqlx/SQLite、clap、reqwest）、aiterm-core 的 `share::{tls,auth}`（TLS 與 SAS 配對）、React 19（設定頁「連線」區塊）。

**Spec:** `docs/superpowers/specs/2026-11-23-intake-server-design.md`

## 與 spec 的計畫期偏差（查證後的決定）

| spec 原寫 | 查證結果 | 計畫決定 |
|---|---|---|
| ① 把 `ai/` 抽出供 server 重用 | `ai/` 無 Tauri，但依賴 `pty`、`chatgpt_web`、`usage`、`commands::provider`、`ConfigStore`，抽出要連帶搬半個 app | **不搬 `ai/`**。core 定義 `AiComplete` trait（一次性文字補全）；AITERM 用 `AiRouter` 實作；server 內建最小 client，支援 OpenAI、Anthropic API、OpenAI 相容、Ollama。使用者要的「server 自己設 AI 供應商與金鑰」不變；只是不重用 `ai/` 的程式。訂閱 OAuth 本來就不支援 |
| `IntakeHost::secret` | `SecretStore` 是系統鑰匙圈 | core 定義 `Secrets` trait；server 用檔案金鑰實作 |
| Email、Telegram 在 server | `mail/poller.rs`、`telegram/` 帶 Tauri | 先做 LINE＋網頁表單（spec 已定）；Email、Telegram 收訊放在最後兩個任務，IMAP 的 `mail/client.rs`、`parse.rs` 無 Tauri 才能重用，否則 Telegram／Email 標為「待補」並在 CHANGELOG 與 spec 寫明 |

## 檔案結構

**新增**
- `src-tauri/crates/aiterm-intake-core/` — `Cargo.toml`、`src/lib.rs`，及搬入的模組：`attach.rs filter.rs rules.rs config.rs store.rs queue.rs refs.rs dispatcher.rs line_source.rs line_store.rs line_text.rs mail_source.rs telegram_source.rs smtp.rs classify.rs web.rs`；新檔 `host.rs`（`Secrets`、`AiComplete`、`Notifier` trait）、`api.rs`（API 的請求／回應型別，Client 與 server 共用）。
- `src-tauri/crates/aiterm-intake-server/` — `src/main.rs`（clap）、`app.rs`（狀態）、`auth.rs`（成員 token、邀請、角色）、`pairing.rs`（TLS＋SAS）、`routes.rs`（`/v1`）、`events.rs`（WebSocket）、`channels.rs`（LINE／網頁表單收訊）、`ai.rs`（最小 AI client）、`secrets.rs`（檔案金鑰）、`audit.rs`；`tests/`（整合測試）。
- `src-tauri/src/intake/remote.rs` — `RemoteClient`（配對、token、`call`）與 `IntakeBackend` 狀態。
- `src/components/IntakeView/ConnectionCard.tsx`（+ test）— 「連線」區塊。

**修改**
- `src-tauri/Cargo.toml` — workspace members、`app` 依賴 core。
- `src-tauri/src/intake/mod.rs` — 改為 `pub use aiterm_intake_core::{...}` 轉出。
- `src-tauri/src/intake/commands.rs` — 每個指令加 Remote 分流。
- `src-tauri/src/intake/runtime.rs` — Remote 模式不啟動收訊與 tick。
- `src/ipc/intake.ts`、`src/lib/i18n.ts`、`src/components/IntakeView/index.tsx`。
- `.github/workflows/` — `intake-server-v*` 發佈流程（最後一個任務）。

---

## Phase A — ① 抽出 core（AITERM 行為不變）

### Task 1: 建立 `aiterm-intake-core` 空 crate 並接進 workspace

**Files:** Create `src-tauri/crates/aiterm-intake-core/Cargo.toml`, `src/lib.rs`; Modify `src-tauri/Cargo.toml`.

- [ ] **Step 1:** 建 `Cargo.toml`：`[package] name="aiterm-intake-core" version="0.1.0" edition="2021" rust-version="1.88"`；dependencies 先放 `serde`、`serde_json`、`sqlx`（與 `app` 相同版本與 features，從 `src-tauri/Cargo.toml` 複製）、`tokio`、`chrono`、`uuid`、`log`、`base64`、`hmac`、`sha2`、`regex`、`reqwest`（`rustls-tls`、無 default features，與 app 一致）、`dirs`、`anyhow`。
- [ ] **Step 2:** `src/lib.rs`：`//! 問題收集核心：資料、規則與收訊，不依賴 Tauri。` 與 `pub mod host;`。`host.rs` 先放三個 trait：

```rust
use std::sync::Arc;

/// 祕密儲存（LINE 憑證、郵件密碼、AI 金鑰）。AITERM 用系統鑰匙圈，server 用檔案金鑰。
pub trait Secrets: Send + Sync {
    fn get(&self, id: &str) -> Option<String>;
    fn set(&self, id: &str, value: &str) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// 資料變動通知（AITERM：emit 事件；server：推播給 WebSocket）。
pub trait Notifier: Send + Sync {
    fn changed(&self);
}

/// 一次性文字補全，供題目分類與「是不是新問題」使用。
#[async_trait::async_trait]
pub trait AiComplete: Send + Sync {
    async fn complete(&self, prompt: &str) -> Result<String, String>;
}

pub type SharedSecrets = Arc<dyn Secrets>;
pub type SharedNotifier = Arc<dyn Notifier>;
pub type SharedAi = Arc<dyn AiComplete>;
```
  （加 `async-trait = "0.1"` 相依。）
- [ ] **Step 3:** `src-tauri/Cargo.toml` 的 `[workspace] members` 加 `"crates/aiterm-intake-core"`，`[dependencies]` 加 `aiterm-intake-core = { path = "crates/aiterm-intake-core" }`。
- [ ] **Step 4:** `cd src-tauri && cargo check --workspace` → 通過。
- [ ] **Step 5:** commit `feat(intake-core): 空的 core crate 與平台 trait`。

### Task 2: 搬無外部依賴的模組（每批搬完跑測試）

**Files:** Move（`git mv`）`src-tauri/src/intake/{attach,filter,rules,config,queue,refs,dispatcher,line_text,line_store,line_source,telegram_source,mail_source,smtp,store}.rs` → `crates/aiterm-intake-core/src/`；Modify `crates/aiterm-intake-core/src/lib.rs`、`src-tauri/src/intake/mod.rs`。

- [ ] **Step 1:** 先記基準：`cd src-tauri && cargo test --lib intake 2>&1 | grep "test result"` → 應為 `222 passed`（若有變動以實際為準，記下數字 N）。
- [ ] **Step 2:** 分三批 `git mv`：①`attach filter rules refs line_text`（無跨模組依賴）；②`config queue line_store dispatcher smtp`；③`store line_source telegram_source mail_source`。每批：移動檔案、在 core `lib.rs` 加 `pub mod x;`、把檔內 `crate::intake::foo` 改成 `crate::foo`、`app` 的 `mod.rs` 把 `pub mod x;` 換成 `pub use aiterm_intake_core::x;`。
- [ ] **Step 3:** 每批後 `cargo check -p aiterm-intake-core`，缺的相依補進 core 的 `Cargo.toml`（缺什麼補什麼，版本與 app 一致）。
- [ ] **Step 4:** 每批後跑 `cargo test -p aiterm-intake-core` 與 `cargo test --lib intake`；兩邊測試數加總必須等於 N，不得減少。
- [ ] **Step 5:** 若某個模組實際引用了 `crate::tasks`／`crate::pty` 等 core 沒有的東西，**不要硬搬**：記下來留在 `app`，並在 spec「開放問題 3」補一行。
- [ ] **Step 6:** commit 每批一次：`refactor(intake): 把 <模組> 搬進 aiterm-intake-core`。

### Task 3: `classify` 改用 `AiComplete`，`web.rs` 解耦

**Files:** Modify/move `classify.rs`、`web.rs`。

- [ ] **Step 1:** 讀 `classify.rs`：`classify`／`is_new_problem` 現在吃 `Arc<dyn AiProvider>`。拆成純函式（`prompt()`、`parse()`、`support_prompt()`、`parse_support()` 搬進 core）與 `async fn classify(ai: &dyn AiComplete, subject, body) -> Result<Kind, String>`、`is_new_problem(ai, messages)`。
- [ ] **Step 2:** 在 `app` 新增 `impl AiComplete for ProviderAdapter(Arc<dyn AiProvider>)`（放 `src-tauri/src/intake/ai_adapter.rs`），內部用 `classify.rs` 原本呼叫 provider 的方式（`ask()` helper）。`runtime.rs` 的 `classify_item`／`ai_is_new_problem` 改傳 adapter。
- [ ] **Step 3:** 既有 classify 測試（`parse`、`parse_support`）搬到 core 並通過；新增測試：假 `AiComplete` 回 `"question"`→`Kind::Question`；回錯誤→呼叫端拿到 `Err`。
- [ ] **Step 4:** `web.rs`（1044 行，6 處 `app.`）：把「路由＋表單處理」留在可搬部分，`start/restart/status` 等取 `app.try_state` 的外殼留在 `app`；檢查 `rules` 之外的依賴後搬進 core。若外殼與邏輯糾纏過深，先只搬 handler 與 `Router` 建構（`pub fn router(state: WebState) -> Router`），外殼留在 `app`。
- [ ] **Step 5:** 測試數不減；commit `refactor(intake): classify 改用 AiComplete，web 路由搬進 core`。

### Task 4: `line_server.rs` 解耦

- [ ] **Step 1:** 把 `LineState`、`handle()`、`push/push_to`、名稱查詢、`router()` 搬進 core（`line_channel.rs`），取祕密改用 `SharedSecrets`、通知改用 `SharedNotifier`；`LineServer`（啟停、`Arc<LineServer>`、`app.try_state`）外殼留在 `app`。
- [ ] **Step 2:** wiremock 與 HTTP 測試隨之搬進 core，通過；`app` 端的外殼測試通過。
- [ ] **Step 3:** commit `refactor(intake): LINE 收訊與推送搬進 core`。

### Task 5: 驗收 ①

- [ ] **Step 1:** `cargo tree -p aiterm-intake-core | grep -E "tauri|portable-pty"` → 無輸出。
- [ ] **Step 2:** `cargo test --workspace --no-fail-fast` 全綠（逐行看 `test result:`）；測試總數 ≥ 搬移前。
- [ ] **Step 3:** `npx tsc -b`、`npm run test` 通過（前端沒動，確認未破壞）。
- [ ] **Step 4:** commit 無；記錄結果。**不寫 CHANGELOG**（使用者無可見變化）。

---

## Phase B — ② Server

### Task 6: API 型別（`api.rs`）

**Files:** Create `crates/aiterm-intake-core/src/api.rs`；Test 同檔。

型別（Client、server 共用，欄位與 `commands.rs` 現有 DTO 對齊，從 `commands.rs` 的 `ConfigView`、`SenderDto`、`ProjectsView`、`RefreshResult` 搬入）：

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Hello { pub server_version: String, pub protocol: u32 }
pub const PROTOCOL: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role { Member, Admin }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Me { pub name: String, pub role: Role }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Event { pub r#type: String, pub seq: u64 }   // {"type":"changed","seq":N}
```
- [ ] **Step 1:** 寫測試：`Event` 序列化為 `{"type":"changed","seq":3}`；`Role` 序列化為 `"admin"`／`"member"`；`PROTOCOL == 1`。
- [ ] **Step 2:** 跑測試確認紅（型別未定義），實作，轉綠。
- [ ] **Step 3:** 把 `commands.rs` 的 DTO（`SenderDto` 等）搬進 `api.rs` 並由 `commands.rs` `use`；測試不減。
- [ ] **Step 4:** commit。

### Task 7: Server 骨架與資料夾（`init`／`run`）

**Files:** Create `crates/aiterm-intake-server/{Cargo.toml,src/main.rs,src/app.rs,src/secrets.rs}`；Modify workspace members。

- [ ] **Step 1 測試（`tests/init.rs`）：** 在 tempdir 跑 `init --data-dir <tmp>`，斷言 `intake.db`、`members.db`、`secrets.key`（Unix 權限 0600）、`tls/cert.pem`、`tls/key.pem` 存在；再跑一次 `init` 不覆蓋既有金鑰（內容不變）。
- [ ] **Step 2:** 實作 `FileSecrets`（`Secrets` trait）：`secrets.key` 存 32 byte 隨機金鑰，祕密以 AES-GCM 加密存 `secrets.db`（`aes-gcm` crate）；測試 set/get/delete 往返、金鑰錯誤時 get 回 `None`。
- [ ] **Step 3:** clap 子命令 `init`、`run`、`pair`、`members list|revoke`、`status`、`ai set`；`init` 呼叫 `aiterm_core::share::tls` 產生自簽憑證（看 `tls.rs` 的現有 API，照用）。
- [ ] **Step 4:** `run` 先只啟動 `/v1/hello`（回 `Hello`）與優雅結束（SIGTERM／Ctrl-C）；測試：起 server 取隨機埠，`GET /v1/hello` → 200 且 `protocol == 1`。
- [ ] **Step 5:** commit。

### Task 8: 成員、邀請與 token（`auth.rs`）

- [ ] **Step 1 測試：** 建邀請（15 分鐘、單次）；用邀請換 token 成功一次、第二次失敗；過期邀請失敗（注入時鐘）；token 只存雜湊（資料庫內容不含明文）；撤銷後驗證失敗；`verify(token)` 回 `Me{name,role}`；錯誤 token 與不存在 token 回應相同。
- [ ] **Step 2:** 實作：`members.db` 表 `members(id, name, role, token_hash, created_at, revoked_at)`、`invites(code_hash, name, role, expires_at, used_at)`；token 256-bit 隨機，`sha256` 雜湊，比對用常數時間。
- [ ] **Step 3:** axum middleware：除 `/v1/hello`、`/v1/pair/*` 外，必須帶 `Authorization: Bearer`，否則 401（不回原因）；`require_admin` extractor，member 呼叫得 403。測試覆蓋三種情況。
- [ ] **Step 4:** commit。

### Task 9: 配對（TLS＋SAS，`pairing.rs`）

- [ ] **Step 1:** 讀 `aiterm-core/src/share/{auth.rs,tls.rs}` 與 `server.rs` 的配對段，列出可重用的函式（commitment、SAS 計算、TLS exporter）；若 API 綁死「單一終端機分頁」，抽出最小的共用函式放回 `aiterm-core`（不改既有行為，既有測試必須通過）。
- [ ] **Step 2 測試：** 假 Client 與 server 完成配對 → 雙方 SAS 相同、server 端確認後拿到 token；假 Client 在中間換憑證 → SAS 不同、配對失敗（比照 `aiterm-host/tests/cli_host.rs` 的中間人測試）；邀請碼錯誤／過期 → 失敗；每來源 IP 連續失敗觸發 `backoff`。
- [ ] **Step 3:** 實作 `POST /v1/pair/start`、`/v1/pair/finish`；server 本機 CLI `pair`／`pair confirm`（互動顯示 4 位數並要求輸入 y）。
- [ ] **Step 4:** commit。

### Task 10: 專案與設定 API（設定對等）

- [ ] **Step 1 測試：** 往返測試——`Config` 的每個欄位經 `PUT /v1/projects/{pid}/config` 再 `GET` 不丟失（用 `serde_json::to_value(Config::default())` 的所有鍵，逐一改成非預設值後往返比對；新增欄位自動納入）；member 呼叫 `PUT` 得 403；`GET` 回應不含任何祕密（對 LINE token 設為 `"SECRET-XYZ"` 後，整份回應字串不含它）；`PUT /v1/secrets/{id}` 後 `GET /v1/status` 顯示「已設定」。
- [ ] **Step 2:** 實作 `GET/POST/DELETE /v1/projects`、`GET/PUT /v1/projects/{pid}/config`、`GET/PUT /v1/global-config`、`GET/PUT /v1/projects/{pid}/senders`（含標籤）、`GET /v1/projects/{pid}/dropped`、LINE 未知來源三個端點、`POST /v1/projects/{pid}/check-folders`（在 server 上跑 `refs::check`）。處理函式直接呼叫 core 現有函式（`config::load/save`、`store::*`），與 `commands.rs` 同一份邏輯；把 `commands.rs` 中非 Tauri 的商業邏輯抽成 core 內的函式，兩邊共用（例如 `set_config` 的「鏡像 line_port／清理 reference_folders」步驟）。
- [ ] **Step 3:** 專案模型：`projects(id, name, work_dir)` 表放 `intake.db`；`work_dir` 可空。
- [ ] **Step 4:** commit。

### Task 11: 清單、動作與附件 API

- [ ] **Step 1 測試：** 清單依 `status`／`source`／`q` 篩選；`approve`／`reject`／`status`／`update_draft` 改變狀態並讓 WebSocket 收到 `changed`；附件下載需 token 且阻擋 `..` 路徑穿越；`reply` 呼叫 wiremock 的 LINE push API 並標記已送出、內容經 `filter::redact` 與 `line_text::to_plain`。
- [ ] **Step 2:** 實作對應端點（路徑見 spec「API」）。送出回覆的邏輯（`runtime::send_reply` 的 LINE、網頁表單／Email 分支）抽成 core 的 `reply::send(pool, secrets, item_id, transport)`，AITERM 與 server 共用；先實作 LINE 與 SMTP（`smtp.rs` 已在 core）。
- [ ] **Step 3:** 稽核紀錄：`audit(ts, member, action, item_id)` 表；每個動作寫一筆；測試斷言。
- [ ] **Step 4:** 預留欄位：`intake_items` 加可空欄位 `claimed_by`、`claimed_at`、`claim_expires_at`（schema 遷移用 `ALTER TABLE … ADD COLUMN`，已存在則略過；測試：舊資料庫升級後欄位存在、既有資料不變）。
- [ ] **Step 5:** `POST …/items/{id}/claim`、`/release`、`/result` 一律回 `501` 與 `{"error":"not implemented"}`；每個端點一則測試。
- [ ] **Step 6:** commit。

### Task 12: WebSocket 推播（`events.rs`）

- [ ] **Step 1 測試：** 連線需 token（無 token 被拒）；`Notifier::changed()` 後所有連線收到遞增 `seq`；撤銷成員後該成員連線立即被關閉；斷線重連後帶 `?since=<seq>`，seq 不同則收到一則 `changed`。
- [ ] **Step 2:** 實作 `BroadcastNotifier`（`tokio::sync::broadcast`＋`AtomicU64`）；`/v1/events` 升級 WebSocket。
- [ ] **Step 3:** commit。

### Task 13: Server 收訊（LINE、網頁表單）與 AI

- [ ] **Step 1:** `channels.rs`：依專案設定啟動 LINE webhook（沿用 core 的 `line_channel::router`，綁定位址與埠來自設定）與網頁表單（core 的 `web::router`，獨立埠）；設定變更時熱重啟（比照 `LineServer::restart`）。
- [ ] **Step 2 測試（整合）：** 用 wiremock 與簽章過的 webhook 請求打 server 的 LINE 埠 → `GET /v1/projects/{pid}/items` 看得到題目、WebSocket 收到 `changed`；簽章錯誤得 401；網頁表單送出後出現題目。
- [ ] **Step 3:** `ai.rs`：`ServerAi`（`AiComplete`）依全域設定 `ai.provider`（openai／anthropic／openai_compat／ollama）、`ai.model`、`ai.base_url` 與祕密 `ai.api_key` 呼叫一次補全；未設定時 `complete` 回 `Err("尚未設定 AI")`，呼叫端走既有預設值。測試用 wiremock 驗證四種供應商的請求形狀與回應解析、未設定的行為、`/v1/status` 顯示「尚未設定 AI」。
- [ ] **Step 4:** 收訊後的「歸屬／開題／分類」流程（本機在 `runtime::tick`）：抽出不依賴終端機的部分（`line_triage`、`classify_item` 的分類）進 core `pipeline.rs`，server 每分鐘 tick 一次；**不派工**（spec ②）。測試：群組安靜期後開題（假 `AiComplete`）；AI 未設定時走預設。
- [ ] **Step 5:** commit。

### Task 14: AITERM `RemoteClient` 與分流

**Files:** Create `src-tauri/src/intake/remote.rs`；Modify `commands.rs`、`runtime.rs`、`lib.rs`（註冊新指令）。

- [ ] **Step 1 測試（對真的 test server，沿用 Task 7 的測試 helper）：** 配對成功後 token 存進（假）鑰匙圈；`list`／`approve` 走 Remote 回與 Local 同形狀；憑證指紋改變 → 明確的 `FingerprintChanged` 錯誤，不可靜默接受；401 → 清掉 token 並回 `Revoked`；離線 → `Offline`，不 panic；版本（`protocol`）不符 → `Incompatible`。
- [ ] **Step 2:** `RemoteClient { base, fingerprint: Option<String>, token }`、`async fn call<T>(method, path, body)`；憑證驗證：釘住指紋時用自訂 rustls verifier，否則系統根憑證。
- [ ] **Step 3:** 新 Tauri 指令 `intake_remote_pair(addr, code)`（回 SAS 供畫面顯示並等待確認）、`intake_remote_status`、`intake_remote_disconnect`；連線狀態存 `IntakeBackend`（`Local` | `Remote`）。
- [ ] **Step 4:** `commands.rs` 的每個指令開頭：`if let Some(rc) = remote(&app) { return rc.<對應呼叫> }`。表格式地逐一加，並為每個指令寫一則「Remote 模式下不碰本機資料庫」的測試（本機 pool 的列數不變）。
- [ ] **Step 5:** `runtime::install`：Remote 模式不啟動收訊與 tick；切換時動態啟停。測試：Remote 模式下 tick 不被呼叫（雙收防護）。
- [ ] **Step 6:** 事件：Remote 連線的 WebSocket 收到 `changed` → `app.emit(CHANGED_EVENT)`，前端無需改。斷線退避重連（`aiterm-core` `backoff`）。
- [ ] **Step 7:** commit。

### Task 15: 前端「連線」區塊與 Remote 模式 UI

- [ ] **Step 1 測試（`ConnectionCard.test.tsx`）：** 未連線時顯示位址與邀請碼輸入；格式錯誤當場提示且不呼叫 IPC；配對中顯示 4 位驗證碼；已連線顯示位址、名稱、角色與「中斷連線」；各種錯誤（過期、指紋改變、被移除、版本不符）顯示對應文字；離線顯示「已離線」並停用動作按鈕。
- [ ] **Step 2:** `src/ipc/intake.ts` 加 `remotePair/remoteStatus/remoteDisconnect`；`i18n.ts` 加 zh-TW 與 en 字串（兩邊都要，語系漂移有測試）；`ConnectionCard.tsx` 放進 `IntakeView` 設定頁最上方。
- [ ] **Step 3:** Remote 模式下：管理員專屬設定對 member 唯讀並提示「這是 server 的設定」；資料夾選擇器停用，改手動輸入並顯示「server 上的路徑」；專案清單來自 server。對應測試。
- [ ] **Step 4:** `npx tsc -b`、`npm run lint`、`npm run test` 通過；commit。

### Task 16: 端到端整合、發佈與文件

- [ ] **Step 1:** `crates/aiterm-intake-server/tests/e2e.rs`：真起 server（隨機埠、tempdir）、配對兩個成員、A 回覆 → B 的 WebSocket 收到 `changed` 並看到已回覆；撤銷 B → B 的請求 401 且 WebSocket 被關。
- [ ] **Step 2:** 設定往返完整性、member／admin 閘門、祕密不外洩三組測試已在前面任務；此處補「全部端點都經過 auth middleware」的測試：列舉 router 的所有路徑，無 token 一律 401（`/v1/hello`、`/v1/pair/*` 例外並以白名單明列）。
- [ ] **Step 3:** `.github/workflows/intake-server.yml`：tag `intake-server-v*` 觸發，先 `needs` 測試，再建 Linux x86_64 與 macOS arm64 二進位並建立 release；完成後跑 `scripts/check_workflows.py`。**不打 tag**。
- [ ] **Step 4:** `docs/intake-server.md`：部署（systemd 範例）、`init`／`pair`／`members`、反向代理與公開網路憑證、備份 `~/.aiterm-intake/`。
- [ ] **Step 5:** `cargo test --workspace --no-fail-fast`（逐行看 `test result:`）、`npx tsc -b`、`npm run lint`、`npm run test` 全綠。
- [ ] **Step 6:** 寫 CHANGELOG（只寫實機看過的行為；沒實機驗收的明說）；commit。

### Task 17（最後）: Email 與 Telegram 收訊

- [ ] **Step 1:** 讀 `mail/client.rs`、`mail/parse.rs`、`telegram/mod.rs`，判斷哪些無 Tauri 可搬或重用。
- [ ] **Step 2:** 可行者做進 `channels.rs`，各附 wiremock／假 IMAP 測試；不可行者在 spec 與 CHANGELOG 明列為「server 暫不支援」。
- [ ] **Step 3:** commit。

## 自我檢查（對照 spec）

| spec 要求 | 任務 |
|---|---|
| core crate、`cargo tree` 無 Tauri | 1–5 |
| `IntakeHost`（祕密／通知／AI） | 1、3、7、13（以三個 trait 實作） |
| server 指令 `init/run/pair/members/status` | 7、8、9；`ai set` 7、13 |
| API 與 `commands.rs` 一一對應、設定對等、多專案 | 10、11 |
| WebSocket `changed`＋`seq` | 12 |
| 配對、指紋釘住、角色、稽核、撤銷即斷線 | 8、9、11、12、16 |
| Server 的 AI | 13 |
| Client 連線流程、錯誤訊息、雙收防護、離線 | 14、15 |
| 版本相容 | 14（`Incompatible`） |
| 預留 claim 欄位與 501 端點 | 11（Step 5） |
| 發佈 `intake-server-v*`、文件 | 16 |
| Email／Telegram | 17 |
