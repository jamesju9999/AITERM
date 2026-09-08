# Claude Bridge 帳號組合（Profiles）— 設計

日期：2026-09-08
狀態：待使用者複審

## 問題

使用者有兩個（或以上）不同的 Claude 訂閱帳號，透過 [Claude Code 橋接](2026-08-07-claude-code-bridge-design.md) 讓 AITerm 終端機裡跑的 `claude` CLI 打到 AITerm 已設定好的 provider。目前切換帳號的做法是到 Settings → Claude Bridge 頁面，把 opus / sonnet / haiku 三個 tier 的下拉選單**逐一**改成另一組 provider，再按 Save。額度用完時，這個「三個下拉選單各改一次」的操作很煩，也容易漏改其中一個 tier。

已確認的事實基礎（讀過程式碼，非推測）：

- 橋接 server 是**每個 HTTP 請求都重新讀 `ConfigStore`**，憑證與 provider 映射不快取（`src-tauri/src/bridge/factory.rs` 開頭註解：「憑證每個請求重新解析，不快取」）。`ConfigStore` 是 process 內共享的 `RwLock<AppConfig>`（`src-tauri/src/config/mod.rs:17-19`），`bridge_set_config` 寫入後立即對所有後續請求生效。
- 因此「改設定」和「重啟 `claude` / 重新登入」完全無關：只要換掉 tier → provider 的映射並存檔，已經在跑的 `claude` session 下一句話就會走新的映射，不需要使用者做任何事。
- 目前 `ClaudeBridgeConfig`（`src/ipc/bridge.ts:26-33`）只有單一組 `{ opus, sonnet, haiku }`，UI（`ClaudeBridgePage.tsx`）沒有「預存多組、一鍵套用」的概念。

## 範圍

**含：**

- 前端新增「帳號組合（Profiles）」概念：每個 profile 是一份 `{ opus, sonnet, haiku }` tier 映射的具名快照。
- 在 Claude Bridge 設定頁新增一個區塊，可以：新增（把目前表格存成新組合）、套用（一鍵切換，立即生效）、更新（把目前表格蓋寫回某個既有組合）、重新命名、刪除。
- 目前表格的三個 tier 若與某個 profile 完全相符，標示該 profile 為「使用中」。
- Profiles 存在瀏覽器 `localStorage`，不經過 Rust 後端。

**不含：**

- 不新增/修改任何 Tauri command 或 `AppConfig` schema（後端完全不動，沿用既有 `bridge_set_config`）。
- 不做 Settings 頁面以外的快速切換入口（終端機分頁列、選單列、鍵盤快捷鍵等）——之後想做可以在這個資料模型上加。
- 不做額度用盡的自動偵測/自動切換，純手動觸發。
- 不做跨裝置同步、匯入匯出。
- 不特別處理「profile 裡引用的 provider 之後被刪除」——套用後下拉選單顯示空白是既有行為（目前表格本來就不驗證 provider_id 是否存在），這次不新增額外防護。

## 資料模型

純前端，新增型別（放在 `ClaudeBridgePage.tsx` 或同目錄一個小檔案，視實作時檔案大小決定）：

```ts
interface BridgeProfile {
  id: string;      // crypto.randomUUID()
  name: string;     // 使用者自訂名稱，如「個人帳號」「公司帳號」
  opus: TierMapping | null;
  sonnet: TierMapping | null;
  haiku: TierMapping | null;
}
```

存放於 `localStorage` key `aiterm.bridgeProfiles`，序列化為 JSON 陣列。讀寫都要包 `try/catch`（localStorage 在部分環境可能拋錯或回傳損毀資料），失敗時視為空陣列，不讓整頁掛掉。

## UI 設計

新區塊「帳號組合」（`bridge_section_profiles`），位置在既有「Tier 對應」表格**上方**。兩者關係：表格＝目前正在套用的設定；組合＝存起來的預設值，套用時整份覆蓋表格再存檔。

- 有 profile 時，逐列顯示：
  - 名稱
  - 「使用中」標示（僅當目前 `cfg.opus / sonnet / haiku` 三者都與該 profile 逐一相等時顯示；用簡單的 tier-by-tier 相等比較，不需要深比對函式庫）
  - 四個動作：**套用** / **更新** / **重新命名** / **刪除**
- 沒有 profile 時顯示空狀態提示（`bridge_profile_empty`），引導使用者「先在下面設定好一組 tier，再存成組合」。
- 頂部「另存目前設定為新組合」按鈕（`bridge_profile_new`），輸入名稱後把目前 `cfg.opus/sonnet/haiku` 存成新 profile 並持久化。

### 各動作行為

- **套用**：組出 `{ ...cfg, opus: profile.opus, sonnet: profile.sonnet, haiku: profile.haiku }`，`setCfg` 更新畫面，並直接把這個新物件（不是等 state 更新後的 `cfg`，避免 stale closure）傳給既有 `bridgeSetConfig` IPC 立即存檔套用。沿用現有 `saving` / `saved` / `status?.error` 顯示邏輯，不新增錯誤處理路徑。
- **更新**：把目前 `cfg.opus/sonnet/haiku` 蓋寫回這個 profile（只改 localStorage 裡的資料，不呼叫 `bridgeSetConfig`——這只是編輯已存的組合，不是套用）。這是「編輯」的實作方式：使用者先套用某組合、在下面表格手動調整、按更新存回去，不用另外做一個獨立的每-tier 編輯表單。
- **重新命名**：更新 `name` 欄位（用簡單的文字輸入，跟 `manualCommand` 那類既有互動風格一致，不用彈跳視窗套件）。
- **刪除**：從陣列移除，附一次確認（避免手滑；沿用瀏覽器原生 `confirm()` 或現有專案裡類似刪除操作的確認模式，實作時比照專案既有寫法）。

## 後端

不動。所有動作都是組出一份新的 `ClaudeBridgeConfig` 物件後呼叫既有的 `bridgeSetConfig(value)` → `bridge_set_config` command。橋接 server 每請求重讀設定的行為已經驗證過，套用後同一個已在執行的 `claude` process 下一句話就會走新帳號，不需重啟、不需 `/login`。

## i18n

在 `src/lib/i18n.ts` 補上 en / zh-TW 兩份字串，比照現有 `bridge_*` 命名風格，至少包含：
`bridge_section_profiles`、`bridge_section_profiles_desc`、`bridge_profile_new`、`bridge_profile_new_placeholder`、`bridge_profile_apply`、`bridge_profile_update`、`bridge_profile_rename`、`bridge_profile_delete`、`bridge_profile_delete_confirm`、`bridge_profile_active`、`bridge_profile_empty`。

## 測試

延伸既有 `ClaudeBridgePage.test.tsx`：

- 新增 profile：填名稱、存檔後 localStorage 與畫面列表都要有這筆。
- 套用 profile：斷言呼叫 `bridgeSetConfig` 時傳入的 payload 三個 tier 都等於該 profile 的值（尤其驗證不是套用到 stale 的 `cfg`）。
- 更新 profile：只改 localStorage，**不**呼叫 `bridgeSetConfig`。
- 重新命名／刪除：畫面與 localStorage 同步變化。
- 「使用中」判斷：三個 tier 全符合才標示；任一 tier 不同就不標示。
- localStorage 讀取失敗（丟例外或回傳非法 JSON）時，頁面仍能正常渲染、profile 列表視為空陣列。
