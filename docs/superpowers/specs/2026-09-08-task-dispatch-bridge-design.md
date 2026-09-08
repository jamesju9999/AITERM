# 工作看板派工可選 Claude Bridge / 帳號組合 — 設計

日期：2026-09-08
狀態：待使用者複審

## 問題

工作看板（Task Board）派工時，`RealDispatcher::dispatch`（`src-tauri/src/tasks/scheduler.rs:88`）呼叫 `dispatch::spawn_and_run`（`src-tauri/src/tasks/dispatch.rs:300`），後者建立分頁時對 `create_with_app` 的第 4 個參數 `bridge_env: Option<(u16, String)>` **寫死傳 `None`**（`dispatch.rs:311`）。

對照組：手動開新分頁走 `pty_create` command（`src-tauri/src/pty/commands.rs:25-46`），會依前端傳來的 `claude_bridge` 旗標查橋接 server 的 port/token，組出 `Some((port, token))` 再呼叫同一個 `create_with_app`。`create_with_app`（`src-tauri/src/pty/manager.rs:29-43`）是唯一真正呼叫 `crate::bridge::env::bridge_envs()` 注入 `ANTHROPIC_BASE_URL` 等環境變數的地方。

結果：**派工開出來的分頁一律直連 Anthropic**，無論 Settings 裡「新分頁預設啟用橋接」（`default_on_new_tab`）有沒有勾，這個全域開關對派工這條路徑完全不生效——是兩條互不相干的程式碼路徑。

使用者想要的：派工每一張卡片時，可以個別選擇「這張卡要不要走 Claude Bridge」，甚至指定要套用哪一組已存的[帳號組合](2026-09-08-bridge-account-profiles-design.md)（Profiles）。

## 範圍界定（brainstorming 已確認）

橋接 server 目前是**一份全域設定，服務所有連上的分頁**——無法讓分頁 A 走帳號 1、分頁 B 同時走帳號 2 而互不干擾。工作看板的卡片可以並行執行（`parallel_ok`），但**已明確不需要**「多張卡同時用不同帳號、彼此完全隔離」這件事。這排除了「幫每個分頁發獨立橋接 token、伺服器端依 token 查對應設定」這種大改動（那是另一個規模大得多的專案），把範圍收斂成：**派工前把選定的帳號組合套用成全域設定，再開分頁**。在這個確認過的前提下（不要求並行隔離），這個做法是安全、夠用的。

## 範圍

**含：**

- `TaskRow` 新增 `use_bridge: bool`（預設 `false`）與 `bridge_tiers: Option<String>`（JSON，NULL＝沿用當下設定）。
- `TaskEditorDialog.tsx` 新增「派工方式」下拉，選項：直連 Anthropic（預設）／走橋接（沿用目前設定）／走橋接：`<每個已存的帳號組合各一項>`。
- 派工時（`RealDispatcher::dispatch`），若 `use_bridge`，先把 `bridge_tiers`（若非 NULL）寫回 `ConfigStore` 的 `claude_bridge.{opus,sonnet,haiku}`，再以既有的 `bridge_env` 機制開分頁。
- 橋接 server 沒在跑時的降級行為，跟 `pty_create` 現有邏輯逐字一致（靜默不注入，不特別報錯）。

**不含：**

- 不做「多張並行卡各自帳號互相隔離」（已確認不需要，見上）。
- 不在 `TaskCard.tsx` 卡片本體加視覺標示（哪張卡用哪個帳號）——只在編輯對話框看得到。
- 不改 `Dispatcher` trait 介面；`scheduler.rs` 既有測試用的假 dispatcher 不受影響——橋接判斷全部包在 `RealDispatcher::dispatch` 內部實作裡。
- 不把「帳號組合」概念搬到後端（那是規格外的架構翻案，見下方「與 profiles 功能的關係」）。

## 與「帳號組合」功能的關係

[帳號組合（Profiles）](2026-09-08-bridge-account-profiles-design.md)是純前端概念，存在瀏覽器 `localStorage`（key `aiterm.bridgeProfiles`），後端完全不知道它存在。工作看板的排程器（`RealDispatcher::dispatch`）是純 Rust 背景迴圈，沒有、也不該有 webview 的 `localStorage` 存取權。

解法：**在編輯卡片、使用者選定帳號組合的那一刻**（前端，有 `localStorage` 存取權），把該 profile 的 `{opus, sonnet, haiku}` 三個 tier 值**解析成具體數值**，存進 `TaskRow.bridge_tiers`（不存 profile 的 id 或名字）。派工當下 Rust 端讀到的只是「這張卡要套用的三個 tier 數值」，完全不需要知道「profile」這個概念——後端從頭到尾不碰 profiles，這個功能不需要為此新增任何後端儲存。

副作用（刻意接受）：卡片存的是**當時的快照**。之後如果使用者在 Settings 把那個 profile 改名、改內容、甚至刪除，已經存進卡片裡的快照不會跟著變。這是刻意的設計，不是遺漏——快照語意最簡單、最好推理（「這張卡建立/編輯當下選了什麼，派工就套用什麼」），不需要在派工當下再去解一次可能已經不存在的 profile 參照。

## 資料模型

`src-tauri/src/tasks/store.rs`：`TaskRow` 新增

```rust
pub use_bridge: bool,
pub bridge_tiers: Option<String>,
```

`bridge_tiers` 的 JSON 形狀（存的是 `ClaudeBridgeConfig` 的 tier 子集）：

```json
{"opus": {"provider_id": "...", "model": "..."} | null,
 "sonnet": {"provider_id": "...", "model": "..."} | null,
 "haiku": {"provider_id": "...", "model": "..."} | null}
```

`src-tauri/src/tasks/mod.rs::init_schema`：`CREATE TABLE IF NOT EXISTS tasks` 直接加這兩欄（給全新安裝），另外照 `interactive`/`ai_summary`/`session_id` 現有寫法各補一行 `ALTER TABLE tasks ADD COLUMN ...`（給既有資料庫；欄位已存在時會失敗，刻意吞掉錯誤，跟現有幾行相同寫法）：

```sql
ALTER TABLE tasks ADD COLUMN use_bridge INTEGER NOT NULL DEFAULT 0
ALTER TABLE tasks ADD COLUMN bridge_tiers TEXT
```

`store::create_task` 與 `store::update_task_fields` 的呼叫端（`commands/tasks.rs` 的 `tasks_create`/`tasks_update`）比照 `parallel_ok`/`interactive` 的模式：新增 `set_use_bridge`/`set_bridge_tiers`（或合併成一個 `set_bridge_config(pool, id, use_bridge, bridge_tiers)`）——**隨時可改，不像 title/body 只能在 `planning` 狀態改**（`edit_allowed` 的限制不套用在這兩個欄位上，跟 `parallel_ok`/`interactive` 目前的待遇一致）。

## 派工邏輯

`RealDispatcher`（`scheduler.rs:77-84`）新增兩個欄位：

```rust
pub bridge: Arc<crate::bridge::BridgeState>,
pub secrets: Arc<crate::secret::SecretStore>,
```

在 `scheduler.rs::spawn`（唯一的生產環境建構點，`scheduler.rs:433`）比照 `config`/`pty` 的既有寫法，從 `app.state()` 取得後放進 `RealDispatcher { ... }`。

`RealDispatcher::dispatch`（`scheduler.rs:88`）在呼叫 `dispatch::spawn_and_run` **之前**新增：

```rust
if task.use_bridge {
    if let Some(json) = &task.bridge_tiers {
        if let Ok(snap) = serde_json::from_str::<BridgeTierSnapshot>(json) {
            self.config.update(|c| {
                c.claude_bridge.opus = snap.opus;
                c.claude_bridge.sonnet = snap.sonnet;
                c.claude_bridge.haiku = snap.haiku;
            });
        }
        // 解析失敗（理論上不會發生，資料是自己存的 JSON）就跳過覆寫，
        // 沿用當下設定──跟 bridge_tiers 是 NULL 時的行為一致，不讓派工失敗。
    }
}
let bridge_env = match (task.use_bridge, self.bridge.port()) {
    (true, Some(port)) => self.secrets
        .get(crate::bridge::auth::BRIDGE_TOKEN_KEY)
        .ok()
        .flatten()
        .map(|t| (port, t)),
    _ => None,
};
```

其中 `BridgeTierSnapshot { opus: Option<TierMapping>, sonnet: Option<TierMapping>, haiku: Option<TierMapping> }` 是新增的最小結構（複用既有的 `crate::config::types::TierMapping`），只在這個模組內部使用。

`dispatch::spawn_and_run`（`dispatch.rs:300`）簽名新增 `bridge_env: Option<(u16, String)>` 參數，原本寫死的 `create_with_app(app.clone(), size, Some(...), None)` 改成把這個參數原樣傳入第 4 格。

降級行為完全比照 `pty_create` 現有的既有邏輯與註解（「server 沒在跑就不注入 —— 注入指向死埠的位址比不注入更難除錯」）：`bridge.port()` 是 `None`（server 沒在跑）或拿不到 token 時，`bridge_env` 就是 `None`，這張卡的分頁靜默降級成直連 Anthropic，不新增任何錯誤訊息或 `error_message` 欄位寫入——維持跟手動開分頁一致的既有產品決策。

## 前端 UI

`src/components/Settings/bridgeProfiles.ts` 的 `loadBridgeProfiles`/`tiersEqual`/`BridgeProfile` 型別直接從 `TaskEditorDialog.tsx` 匯入（純函式模組，無 React/Settings 相依，跨資料夾匯入在這個專案是常見做法，不需要搬檔案）。

新增下拉（建立、編輯共用同一份邏輯）：

```
派工方式
├ 直連 Anthropic（預設）           → use_bridge=false, bridge_tiers=null
├ 走橋接（沿用目前設定）           → use_bridge=true,  bridge_tiers=null
├ 走橋接：<profile.name>（逐一列出）→ use_bridge=true,  bridge_tiers=JSON.stringify({opus,sonnet,haiku})
```

`loadBridgeProfiles()` 回傳空陣列時，下拉只有前兩個選項，優雅降級。

**編輯既有卡片時的顯示邏輯**：若 `task.use_bridge` 為 false → 顯示「直連 Anthropic」；若為 true 且 `bridge_tiers` 是 `null` → 顯示「走橋接（沿用目前設定）」；若為 true 且 `bridge_tiers` 非 null → `JSON.parse` 後用 `tiersEqual` 逐一比對現存的 profiles，找到唯一匹配就顯示該 profile 名字，找不到匹配（profile 後來被改名/刪除/內容變了）就顯示通用選項「走橋接（此卡自訂組合）」——不強迫使用者重選、不清空既有設定。

## i18n

`src/lib/i18n.ts` 新增（en / zh-TW 各一份）：`task_bridge_label`（「派工方式」）、`task_bridge_direct`（「直連 Anthropic（預設）」）、`task_bridge_current`（「走橋接（沿用目前設定）」）、`task_bridge_custom`（「走橋接（此卡自訂組合）」）、`task_bridge_profile_option: (name: string) => string`（「走橋接：${name}」）。

## 測試

- **Rust**：`dispatch::spawn_and_run` 新參數的既有呼叫端（`scheduler.rs` 裡的呼叫、`mcp_server/coordination_ops.rs:120` 那個第三方呼叫者）都要顯式傳 `None`，維持現有行為不變——這兩處都不是「使用者透過工作看板派工」的路徑，不該意外被牽動。補一個 `RealDispatcher::dispatch` 的整合測試：`use_bridge=true` + 有 `bridge_tiers` 時，斷言 `self.config.get().claude_bridge` 的三個 tier 值確實被覆寫成快照值；`bridge_tiers=null` 時斷言設定沒被動到；`use_bridge=false` 時斷言 `bridge_env` 傳的是 `None`（可以在 `bridge.port()` 回傳 `Some` 的情況下斷言，證明是 `use_bridge` 本身擋下來的，不是因為 server 沒在跑）。
- **前端**：`TaskEditorDialog.test.tsx` 新增：下拉選單依 `loadBridgeProfiles()` 動態列出選項；選某個 profile 存檔時，`createTask`/`updateTask` 收到的 `bridge_tiers` 是該 profile 的 JSON 序列化值；重新打開一張 `bridge_tiers` 剛好匹配某個 profile 的卡片，下拉正確預選該 profile；`bridge_tiers` 內容跟任何現存 profile 都對不上時，顯示「此卡自訂組合」而不是報錯或清空。
