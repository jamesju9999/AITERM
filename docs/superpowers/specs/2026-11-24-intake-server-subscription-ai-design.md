# 問題收集 Server：AI 支援訂閱登入與更完整的設定

日期：2026-11-24　狀態：待核可

## 目標

Server 的 AI（題目分類、群組「是不是新問題」）目前只能用 API 金鑰的供應商。要做到：

1. 支援**訂閱登入**：Claude（Anthropic OAuth）、ChatGPT（Codex）、Gemini（Antigravity）。
2. 設定畫面跟 AITerm 一樣豐富：模型清單下拉、**測試連線**、登入狀態與錯誤一目了然。
3. 全部從 AITerm Client（管理員）操作，不必碰 server 的命令列或瀏覽器。

使用者已確認可以用訂閱方式（條款風險由使用者承擔，UI 會標示）。

## 不做（這一版）

- Copilot、Claude CLI、ChatGPT 網頁版：依賴本機程式或瀏覽器工作階段，無頭 server 做不到。
- 多個供應商並存與自動切換：server 維持「一個作用中的供應商」。
- 訂閱帳號的用量／配額顯示。

## 關鍵限制（決定了設計）

**Anthropic 與 Codex 的續期憑證（refresh token）是一次性的**：每次續期，供應商發新的、讓舊的失效
（`ai/router.rs` 的 `store_refreshed_secrets` 已有註解）。如果把使用者電腦上那份登入直接複製給 server，
兩邊會互相搶，誰先續期，另一邊就登出。

→ **不複製，為 server 另外登入一次**，取得獨立的一組憑證。使用者本機的登入完全不受影響。
Google 的續期憑證不輪替，但為一致也走同一流程。

## 設計

### 流程

```
管理員在 AITerm 的 Server AI 卡片選「Claude 訂閱」→ 按「為 server 登入」
  → AITerm 在管理員自己的電腦開瀏覽器走一次 OAuth（與本機登入相同的流程）
  → 取得 token 後「不存進本機鑰匙圈」，直接用已驗證的 TLS 連線交給 server（PUT /v1/ai/oauth）
  → server 加密存起來，之後由 server 自己續期、自己呼叫供應商
```

- 各家登入方式沿用 AITerm 現有的：Anthropic 是貼上授權碼、Codex 是本機 1455 埠回呼、Google 是本機回呼。
  AITerm 的登入程式碼拆成「取得 token」與「存進本機」兩段，Server 登入只用前一段。
- token 只在 AITerm 記憶體中短暫存在，傳給 server 後清掉。

### Server 端

新增供應商 `anthropic_oauth`、`codex_oauth`、`antigravity_oauth`（與現有四個並列，仍是單一作用中）。

| 祕密（加密存放，不回傳） | 說明 |
|---|---|
| `ai_oauth_access` / `ai_oauth_refresh` / `ai_oauth_expires_at` | 存取憑證、續期憑證、到期時間 |
| `ai_oauth_account_id` | Codex 需要（從 id_token 取得） |
| `ai_oauth_project_id` | Antigravity 需要（登入時 onboarding 取得） |

- **續期**：到期前 5 分鐘（Google 15 分鐘）續期；用單一鎖（single-flight），不會兩個請求同時用同一個一次性續期憑證。
  新的續期憑證寫入失敗要明確回報（不能默默吞掉，否則下次就登不進去），寫入成功才用新的存取憑證。
- **失敗處理**：續期被拒（憑證失效）→ 狀態變「需要重新登入」，AI 視為未設定（行為與沒設 AI 時相同：不卡住收訊），
  `status` 與畫面都顯示原因。
- **一次性呼叫**（`AiComplete::complete(system, user)`，無工具、無串流需求）：
  - Anthropic：`/v1/messages`，帶 `anthropic-beta: claude-code-20250219,oauth-2025-04-20`，system 用區塊陣列並以
    Claude Code 識別句開頭（`CLAUDE_CODE_SENTINEL`）。**不帶 tools**，避開「訂閱帶 tools 被算額外用量」的問題。
  - Codex：`chatgpt.com` 的 Responses 端點，`instructions`＋`input`、`stream: true`、`store: false`，帶 `originator`、
    `chatgpt-account-id` 等標頭；把串流的文字片段接起來。
  - Antigravity：`cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse`，帶 `project` 與 Antigravity 的外層欄位；接起文字片段。
- 三家的線上格式、用戶端識別字串與常數**以 AITerm 現有實作為準**（已用真實帳號驗證過）。

### 共用程式碼與防止分歧

新增小型 crate `aiterm-ai-oauth`（只含常數、token 交換／續期、一次性呼叫的請求組裝與回應解析，不依賴 tauri／pty）。
Server 與 AITerm 的「為 server 登入」都用它。**AITerm 既有的 AI 路由與登入不改動**（避免回歸），
只加一組測試，斷言新 crate 的常數與 `commands/provider.rs`、`ai/*.rs` 裡的一致；日後有人改了一邊會立刻紅燈。

### API（`/v1`，皆管理員）

- `PUT /v1/ai/oauth`：`{provider, access, refresh, expires_in, account_id?, project_id?}`，存祕密並設為作用中。
- `DELETE /v1/ai/oauth`：登出（刪除祕密）。
- `GET /v1/ai`：擴充回傳 `auth`（`key`｜`oauth`）、`login`（`ok`｜`expired`｜`needs_login`｜`none`）、帳號提示（如信箱）、最近錯誤。
- `GET /v1/ai/models`：該供應商可用模型（金鑰型呼叫供應商；訂閱型用內建清單，與 AITerm 相同）。
- `POST /v1/ai/test`：送一個極短的測試請求，回傳成功與否、耗時、錯誤訊息。

### AITerm 畫面（連到 server 時的「Server AI」卡片）

- 供應商下拉多三個：Claude 訂閱、ChatGPT 訂閱、Gemini 訂閱。
- 訂閱型：顯示「為 server 登入」「登出」、登入狀態（已登入：帳號／已過期／需要重新登入）；金鑰欄位隱藏。
- 模型改成下拉（可手動輸入），清單來自 server。
- 「測試連線」按鈕：顯示通過／失敗、耗時與錯誤原因。
- 說明文字標示：訂閱帳號由整個團隊的請求共用，請自行確認供應商條款。

## 安全

- token 只存在 server 的加密祕密庫，不回傳、不寫進 log；API 只有管理員可呼叫，傳輸走已驗證的 TLS。
- 稽核紀錄：誰、何時、登入或登出了哪個供應商（不記 token）。
- 登出會刪除 server 上的憑證（但供應商端的授權要使用者自行到帳號頁面撤銷）。

## 實作分期（各自可獨立發佈）

1. **P1 設定更完整（不碰訂閱）**：模型下拉、`/v1/ai/models`、測試連線、狀態顯示。對現有四個金鑰型供應商就有用。
2. **P2 Anthropic 訂閱**：新 crate 骨架、登入拆段、續期（含一次性續期憑證鎖）、一次性呼叫、畫面。
3. **P3 Codex 訂閱**。
4. **P4 Antigravity 訂閱**。

每一期都要用**真實帳號**做一次端到端驗證（登入、呼叫、等到續期後再呼叫）才算完成；沒驗證的不寫進更新說明。

## 風險

- 三家訂閱協定都不是公開文件，會隨供應商改版而失效；常數與標頭以現有實作為準，失效時要同時修兩邊。
- 續期與併發：一次性續期憑證若寫入失敗會使整組登入作廢，需要明確的錯誤與「重新登入」路徑。
- 供應商可能對「同一個訂閱從伺服器來源呼叫」限流或封鎖；這是使用者已知並承擔的條款風險。
- 同時在兩個地方登入同一個帳號通常沒問題（各自獨立憑證），但各家上限我沒有查證，列為實測項目。

## 待使用者確認

1. 三家都要，還是先做 Anthropic（P2）就好？建議：P1 → P2 → 視使用情況再做 P3、P4。
2. 作用中供應商維持單一（建議），還是需要多個並備援？
