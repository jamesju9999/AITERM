# 雲端模型使用量顯示設計規格

**日期**：2026-08-15
**狀態**：設計完成，待實作
**探勘測試**：`src-tauri/tests/usage_probe.rs`（`cargo test --test usage_probe -- --ignored --nocapture`）

## 目標

讓使用者在 AITerm 裡看得到「我的模型額度還剩多少、我用掉了多少」，具體回答兩個問題：

1. **我現在敢不敢開 agent loop？** → 訂閱型 provider 的上游配額剩餘
2. **我到底用掉多少／花了多少？** → AITerm 自身的 token 累計與成本估算

## 為什麼兩者都要

只做本地累計不夠：本地只算得到 AITerm 自己送出去的量，但同一份 Claude 訂閱還會被 Claude Code、Codex CLI 吃掉，所以**本地數字不等於剩餘額度**。

只做上游配額也不夠：API key 型 provider（OpenAI／Anthropic key／各家 compat）根本沒有「訂閱配額」這種東西，只有短期節流，本地累計才是使用者要的數字。

兩者互補，且**依 provider 類型顯示不同指標**，不硬湊成同一個數字。

---

## 探勘實證

以下全部以使用者真實憑證於 2026-08-15 實測取得，原始 dump 見探勘測試輸出。**這是本規格的地基**：三家都有穩定、單次 GET、可快取的結構化端點。

### A. Anthropic OAuth（`ProviderType::Anthropic` + `auth_method = "oauth"`）

`GET https://api.anthropic.com/api/oauth/usage` → **200**

```json
{
  "five_hour": { "utilization": 7.0, "resets_at": "2026-08-15T07:00:00.318695+00:00",
                 "limit_dollars": null, "used_dollars": null, "remaining_dollars": null },
  "seven_day": { "utilization": 4.0, "resets_at": "2026-08-19T16:00:00.318714+00:00", … },
  "limits": [
    { "kind": "session",     "group": "session", "percent": 7, "severity": "normal",
      "resets_at": "2026-08-15T07:00:00.318695+00:00", "is_active": true },
    { "kind": "weekly_all",  "group": "weekly",  "percent": 4, "severity": "normal",
      "resets_at": "2026-08-19T16:00:00.318714+00:00", "is_active": false }
  ],
  "extra_usage": { "is_enabled": false, "monthly_limit": 20000, "used_credits": 0.0,
                   "utilization": 0.0, "currency": "USD", "disabled_reason": "out_of_credits" },
  "spend": { "used": {…}, "limit": {…}, "percent": 0, "severity": "normal", "enabled": false }
}
```

必要 header：`Authorization: Bearer`、`anthropic-beta: claude-code-20250219,oauth-2025-04-20`、`anthropic-version: 2023-06-01`、`x-app: cli`。

`GET /api/oauth/profile` → 200，帶方案別（`has_claude_pro` / `has_claude_max`、`organization.rate_limit_tier`）。

另外**每次真實 `/v1/messages` 回應**都帶：

```
anthropic-ratelimit-unified-5h-utilization: 0.06
anthropic-ratelimit-unified-5h-reset: 1786777200
anthropic-ratelimit-unified-5h-status: allowed
anthropic-ratelimit-unified-7d-utilization: 0.03
anthropic-ratelimit-unified-status: allowed
anthropic-ratelimit-unified-representative-claim: five_hour
```

> ⚠️ **刻度不一致**：`/api/oauth/usage` 的 `utilization` 是**百分比**（`7.0` = 7%），header 的 `utilization` 是**小數**（`0.06` = 6%）。同名不同刻度，是最容易寫錯的地方。`limits[].percent` 也是百分比，可用來交叉驗證。

回 404 的候選端點（記錄下來避免重試）：`/api/oauth/claude_cli/usage`、`/v1/oauth/usage`、`/api/claude_cli/usage`、`/api/usage`。

### B. Codex（`ProviderType::Codex`）

`GET https://chatgpt.com/backend-api/codex/usage` → **200**

```json
{
  "plan_type": "free",
  "rate_limit": {
    "allowed": true, "limit_reached": false,
    "primary_window":   { "used_percent": 0, "limit_window_seconds": 2592000,
                          "reset_after_seconds": 2268666, "reset_at": 1789029443 },
    "secondary_window": null
  },
  "credits": { "has_credits": false, "unlimited": false, "balance": null },
  "spend_control": { "reached": false, "individual_limit": null },
  "rate_limit_reached_type": null
}
```

必要 header 同既有 `CodexClient::apply_headers`（`Authorization: Bearer`、`originator: codex_cli_rs`、`chatgpt-account-id`）。

真實 responses 請求的回應 header 亦帶同一組資訊：`x-codex-primary-used-percent`、`x-codex-primary-window-minutes`、`x-codex-primary-reset-at`、`x-codex-secondary-*`、`x-codex-plan-type`、`x-codex-active-limit`、`x-codex-credits-*`。

`/backend-api/codex/rate_limits` → 404。`api.openai.com/v1/usage` 以 OAuth token 呼叫 → 401（那是 API key 的端點，不通用）。

> **`secondary_window` 可能為 null**（本次實測即是）。UI 必須容忍只有一個窗。`plan_type` 也可能是 `free`，此時窗口是 30 天而非 5 小時 —— 不要把窗口長度寫死。

### C. GitHub Copilot（`ProviderType::GithubCopilot`）

`GET https://api.github.com/copilot_internal/user` → **200**（header：`Authorization: token <gh_token>`、`User-Agent`）

```json
{
  "access_type_sku": "yearly_subscriber_quota",
  "quota_reset_date_utc": "2026-09-01T00:00:00.000Z",
  "quota_snapshots": {
    "chat":        { "unlimited": true,  "percent_remaining": 100.0, … },
    "completions": { "unlimited": true,  "percent_remaining": 100.0, … },
    "premium_interactions": {
      "unlimited": false, "entitlement": 300, "remaining": 142,
      "percent_remaining": 47.5, "credits_used": 157, "overage_permitted": true }
  }
}
```

`copilot_internal/v2/token`（既有的 token 交換端點）也帶 `limited_user_quotas` / `sku`，但資訊較少；用 `/user`。

> Copilot 的語意是**剩餘次數**（142/300），不是使用百分比，而且 `chat`/`completions` 是無限、只有 `premium_interactions` 有限。這是三家裡唯一「剩餘」導向的。

### 沒有配額概念的 provider

`Ollama`、`OpenaiCompatible`（本地 vLLM 等）、以及所有 API key 型（`Openai`、`Anthropic` + api_key、`GoogleAi`、`Openrouter`、`Xai`、`Deepseek`、`Kimi`、`AnthropicCompatible`）。這些一律不查上游，只顯示本地累計。

`ChatgptWeb` 與 `Codex` 吃同一份訂閱，但走網頁後端；本規格**不**為它單獨查配額（見〈非目標〉）。

---

## 非目標

- **Admin API 帳單**（Anthropic `/v1/organizations/usage_report`、OpenAI `/v1/organizations/costs`）：需另一把 admin key 且要組織擁有者權限，多數使用者開不出來，維護成本不成比例。
- **每分鐘 rate-limit header 常駐顯示**：那是短期節流，平常都是滿的，純噪音。改為**撞到 429 時**在既有 `AiError::RateLimit` 的訊息裡帶上重置時間。
- **跨裝置同步用量**。
- **`ChatgptWeb` 的獨立配額查詢**：與 Codex 同一份訂閱，若使用者同時設了 Codex provider 就會重複顯示；第一版不做，日後若有需求再說。
- **Antigravity 配額**：使用者目前未設定此 provider，無憑證可探勘，本版不涵蓋。

---

## 資料模型

三家的配額語意不同（百分比 vs 剩餘次數、一個窗 vs 兩個窗、時間窗長度不一），不能塞進同一個扁平結構。正規化成共通形狀，各家 adapter 自己換算，**UI 只認這個形狀**：

```rust
/// 單一配額窗的正規化快照。
#[derive(Debug, Clone, Serialize)]
pub struct QuotaWindow {
    /// 顯示用標籤，adapter 產生（"5 小時" / "7 天" / "本月 premium"）。
    pub label: String,
    /// 已使用百分比，一律 0.0–100.0。剩餘導向的來源由 adapter 換算。
    pub used_percent: f64,
    /// 重置時間（Unix 秒）。來源沒給就是 None。
    pub resets_at: Option<i64>,
    /// 嚴重度。**不是 used_percent 的函數** —— 上游若有明確的「已被擋住」
    /// 訊號，adapter 必須把該窗提成 Critical。上游給了 severity 字串就採用它，
    /// 都沒有才依 used_percent 推。
    pub severity: QuotaSeverity,
    /// 保留原始語意的補充說明（Copilot 的 "142 / 300 次"）。None 表示沒有更精確的說法。
    pub detail: Option<String>,
    /// 上游標記的代表窗（Anthropic 的 representative-claim、Codex 的
    /// primary_window、Copilot 的 premium_interactions）。
    /// **契約：一個 ProviderQuota 內至多一個為 true。**
    /// 只在多個窗嚴重度相同時用來 tie-break，不決定顯示哪個。
    pub is_primary: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSeverity { Normal, Warning, Critical }

/// 一個 provider 的完整配額快照。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderQuota {
    pub provider_id: String,
    /// 方案別，顯示用（"Claude Pro" / "free" / "yearly_subscriber_quota"）。
    pub plan: Option<String>,
    /// 可能有 0、1 或 2 個窗。空陣列代表查得到但沒有配額限制（Copilot 的無限方案）。
    pub windows: Vec<QuotaWindow>,
    /// 快照取得時間（Unix 秒），UI 用來顯示「幾分鐘前」。
    pub fetched_at: i64,
}
```

`severity` 的推導（來源沒給時）：`< 75%` = Normal、`75–90%` = Warning、`> 90%` = Critical。Anthropic 給了 `severity` 字串就直接採用。

### 本地累計

```rust
/// 一次 AI 請求的用量紀錄（寫入 usage.db）。
pub struct UsageRecord {
    pub provider_id: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub occurred_at: i64,
}
```

> 現有的 `ai::TokenUsage`（`src-tauri/src/ai/mod.rs:145`）只有 input/output。要擴充出 cache 欄位 —— Anthropic 的 `cache_creation_input_tokens` / `cache_read_input_tokens` 與 Codex 的 `input_tokens_details.{cache_write_tokens,cached_tokens}` 都已經在回應裡，只是目前被丟掉。**快取命中率是這份統計最有價值的一欄**（成本差一個數量級）。

---

## 後端架構

### 1. 配額查詢：`src-tauri/src/usage/quota.rs`

一個 trait + 三個 adapter，加一層快取：

```rust
#[async_trait]
pub trait QuotaSource: Send + Sync {
    async fn fetch(&self) -> Result<ProviderQuota, AiError>;
}
```

- `AnthropicOauthQuota` —— `GET /api/oauth/usage`，把 `five_hour` / `seven_day` 映成兩個 `QuotaWindow`；`representative-claim` 決定 `is_primary`；`utilization` 已是百分比，**直接用，不要再乘 100**。
- `CodexQuota` —— `GET /backend-api/codex/usage`，`primary_window` / `secondary_window`（後者可能 null）映成 1–2 個窗；`label` 由 `limit_window_seconds` 換算（`18000` → 「5 小時」、`2592000` → 「30 天」、`604800` → 「7 天」，其餘 fallback 成「N 小時／N 天」）。
- `CopilotQuota` —— `GET /copilot_internal/user`，只取 `unlimited == false` 的 snapshot（實測即 `premium_interactions`）；`used_percent = 100.0 - percent_remaining`；`detail = "142 / 300"`；`resets_at` 取 `quota_reset_date_utc`。

Adapter 由 provider 型別 + `auth_method` 決定，工廠函式放在 `quota.rs`，憑證一律沿用既有的 `get_valid_oauth_token` / `get_valid_codex_oauth_token` / `SecretStore`，**不自己重寫刷新邏輯**。

**快取**：`Mutex<HashMap<provider_id, (ProviderQuota, fetched_at)>>`，TTL **60 秒**。理由：配額變動以分鐘計，而 UI 每次開下拉選單就要顯示，不能每次都打網路。使用者手動重新整理可略過快取。

### 2. 本地累計：`src-tauri/src/usage/store.rs`

SQLite，`{data_dir}/AITERM/usage.db`，比照 `src-tauri/src/db/loop_sessions.rs:36-55` 的既有寫法（`SqliteConnectOptions::new().create_if_missing(true)`，失敗才 fallback 到 in-memory）。

```sql
CREATE TABLE IF NOT EXISTS usage_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id  TEXT NOT NULL,
    model        TEXT NOT NULL,
    input_tokens        INTEGER NOT NULL DEFAULT 0,
    output_tokens       INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens   INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens  INTEGER NOT NULL DEFAULT 0,
    occurred_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_usage_time ON usage_records (occurred_at);
CREATE INDEX IF NOT EXISTS idx_usage_provider ON usage_records (provider_id, occurred_at);
```

保留 90 天，啟動時清掉更舊的。

### 3. 記帳的接點：包在 `AiRouter` 外層

`GenerateChunk` 的消費端散在至少 6 個檔案（`commands/ai.rs` 一支就有 7 處 `rx.recv()` 迴圈，另有 `commands/vcs.rs`、`commands/design.rs`、`knowledge_base/chat.rs`、`mail/classify.rs`、`code_assistant/mod.rs`）。逐一改是保證會漏的做法。

改採**裝飾器**：`AiRouter::resolve_by_id` 回傳 `Arc<dyn AiProvider>`，把它包一層 `MeteredProvider`，內部自建中繼 channel，轉發每個 chunk 給原本的 `tx`，同時記下最後一個帶 `usage` 的 chunk，在串流結束時寫進 `usage.db`。

```
呼叫端 ──tx──▶ MeteredProvider ──tx'──▶ 真正的 provider
                     └─ 攔截 usage ─▶ usage.db
```

`generate` 與 `generate_with_tools` 都要包。`resolve()` 內部就是呼叫 `resolve_by_id`，唯一要另外處理的是 `resolve()` 裡的 `OPENAI_API_KEY` 環境變數 fallback 路徑（`router.rs:432-441`）。

**一個接點覆蓋全部呼叫端，沒有任何既有檔案需要改動。** 這是選這個做法的主因。

### 4. 成本估算

單價表寫死在 `src-tauri/src/usage/pricing.rs`，`HashMap<&str, ModelPrice>`（input / output / cache_read / cache_write，單位 USD per 1M tokens），比對 model 字串前綴。查不到單價就**不顯示金額**，不要猜。

訂閱型 provider（Anthropic OAuth / Codex / Copilot）**不顯示金額**，只顯示「等值 API 花費」並明確標註那不是帳單 —— 顯示成訂閱制的花費會誤導。

---

## IPC

```rust
/// 查單一 provider 的配額。force=true 略過 60 秒快取。
#[tauri::command]
async fn usage_quota(provider_id: String, force: bool) -> Result<Option<ProviderQuota>, AiError>;

/// 查全部已設定 provider 的配額（並行，個別失敗不影響其他）。
#[tauri::command]
async fn usage_quota_all(force: bool) -> Vec<QuotaResult>;

/// 本地累計，依 provider + model 分組。
#[tauri::command]
async fn usage_summary(range: UsageRange) -> Result<UsageSummary, String>;
```

`usage_quota` 回 `Option`：`None` 表示這個 provider **沒有配額概念**（Ollama、API key 型），與「查詢失敗」是不同的狀態，UI 要能分辨。

`usage_quota_all` 回 `Vec<QuotaResult>`（每個元素是 `Ok(ProviderQuota)` 或帶錯誤訊息的變體）而非 `Result<Vec<_>>` —— 一個 provider 的 token 過期不該讓整批查詢失敗。

`UsageRange`：`Today | Days7 | Days30`。

> **Tauri IPC 的錯誤型別**：`AiError` 是既有的 discriminated union，前端已能處理；沿用它，不要用 `String(e)` 把結構化錯誤壓成字串。

---

## 前端 UI

### 第一層：`ModelPickerButton` —— 常駐徽章

配額顯示放進既有的 `src/components/ModelPickerButton.tsx`，理由是這個元件已被 8 個檔案共用（TerminalView、LoopStudio 的 index 與 AgentRoster、VcsView、KnowledgeBaseView、CrossDbAiChat、DocConverterView、ApiDocs 的 ExtractionSettings）。改一處，終端、LoopStudio、VCS、知識庫等視圖就全部常駐可見，不必新增任何全域狀態列，也不會跟既有 `AgentStatusBar` 搶版面。

**收合狀態（常駐）**：按鈕上一律顯示目前選中 provider 的代表窗徽章 —— `5h 7%`、`premium 142/300`。這是使用者「隨時知道還剩多少」的主要入口，**不是只有超標才出現**。

- 代表窗的選擇：**取最嚴重的那個窗**。同嚴重度時才用上游標記的代表窗（Anthropic 的 `representative-claim`、Codex 的 `primary_window`、Copilot 的 `premium_interactions`，即 `is_primary`），再同則取第一個。
  > 一開始的設計是直接用上游的代表窗，但那會在「5h 窗剛重置 0%、7d 窗已 96%」時顯示綠色 0%，與這個功能的目的正好相反。收合狀態只有一格，那一格必須是最該讓人停手的數字。
- **severity 不是 used_percent 的函數**：上游若有明確的「已被擋住」訊號（Codex 的 `limit_reached` / `allowed`、Anthropic 的 `spend.severity`），adapter 必須把該窗提成 critical —— 花費上限觸發時 `used_percent` 可能還是 0。
- 依 `severity` 上色：normal 用低調的次要文字色（不搶注意力）、warning 琥珀、critical 紅。**平常靜、超標才跳**，靠顏色而非有無來分級。
- 沒有配額概念的 provider（Ollama、API key 型）顯示今日 token 數，例如 `12.4k`。
- 查詢失敗或尚未載入顯示灰色「—」，**絕不擋住按鈕的原有功能**。

**展開狀態（下拉選單）**：每個 provider 列右側顯示各自的徽章，讓使用者能在切換前比較。多窗的 provider（Anthropic 的 5h + 7d）在這裡兩個都顯示，收合狀態則只顯示代表窗。

- 查詢失敗顯示灰色「—」加 tooltip 說明原因，**不擋住選單、不跳錯誤**。

**抓取時機**（因常駐顯示而必要）：

- 掛載時查一次目前選中的 provider（只查一個，不是全部）。
- 之後每 **5 分鐘**背景輪詢一次，同樣只查選中的那個。搭配 60 秒快取，先後掛載的視圖會共用同一份快照。
  > 快取沒有做 in-flight 請求合併，所以若多個視圖在快取為空時**同一瞬間**掛載，仍會各自打一次上游。實務上影響很小（唯讀 GET、幾個重複請求遠低於各家速率上限），故不為此加合併機制。
- 切換 provider 時立即查新選中的那個。
- **下拉展開時**才觸發 `usage_quota_all`（查全部），這是唯一會一次打三個端點的時機。
- 視窗失去焦點時跳過輪詢；**回到前景時立即補查一次** —— 使用者離開再回來的那一刻，正是他最想知道「現在還剩多少」的時候，讓他盯著 5 分鐘前的舊數字違背這個功能的目的。後端 60 秒快取讓這次補查很便宜。

> 輪詢成本很低：三個都是單次 GET、幾百 ms，而常駐狀態每 5 分鐘只查一個。但這確實是「常駐顯示」相對「展開才查」多付的代價，實作時不要把它擴大成「每 5 分鐘查全部」。

### 第二層：Settings 新增「用量」頁

`src/components/Settings/UsagePage.tsx`，比照既有 Settings 分頁結構：

1. **配額區** —— 每個訂閱型 provider 一張卡：方案別、各窗的進度條 + 重置倒數、`detail` 原始語意、最後更新時間 + 手動重新整理鈕。
2. **本地累計區** —— 區間切換（今天／7 天／30 天），依 provider → model 展開的表格：次數、input、output、cache read、cache write、估算成本。
3. **快取命中率**單獨一欄（`cache_read / (input + cache_read)`）。

### 第三層：agent mission 即時用量

`useAgentMission.ts` 的多步迴圈是最會爆量的地方，一次 mission 常比一整天手動問答還多。在 `AgentStatusBar` 既有的狀態列加一段「本次 X tokens／第 N 步」。純本地累加，不查網路，零風險。

### i18n

所有新字串進 `src/lib/i18n.ts`，en / zh-TW 兩份。

---

## 錯誤處理

| 情況 | 行為 |
|---|---|
| OAuth token 過期 | 沿用既有刷新邏輯；刷新失敗回 `AiError::AuthFailed`，UI 顯示「需重新登入」 |
| 端點回 404／結構改變 | 記 log、回錯誤、UI 顯示「—」。**絕不因為配額查不到就擋住 AI 功能** |
| 網路逾時 | 5 秒 timeout，回快取中的舊值並標「N 分鐘前」 |
| `usage.db` 開不起來 | 比照 `loop_sessions.rs` fallback 到 in-memory，記帳失效但不影響 AI |
| 上游沒回 `usage` | 不寫紀錄。**不要用估算值補**，那會讓統計失去可信度 |

配額查詢與記帳都是**旁路**：任何失敗都不得影響 AI 主流程。

---

## 測試策略

**Rust 單元測試**（不觸網路）：

- 三個 adapter 的**回應解析**：把探勘 dump 的真實 JSON 當 fixture，斷言映射成正確的 `QuotaWindow`。**尤其要有一個測試釘死 Anthropic 的 `utilization: 7.0` → `used_percent: 7.0`**（不是 700，也不是 0.07）—— 這是最容易寫錯的地方。
- `secondary_window: null` 時 Codex 只產生一個窗。
- Copilot 的 `unlimited: true` snapshot 被略過、`percent_remaining: 47.5` → `used_percent: 52.5`。
- severity 推導的邊界（75、90）。
- 快取 TTL：60 秒內第二次呼叫不觸發 fetch；`force=true` 會。
- `usage.db` 的寫入與區間彙總。

**wiremock 整合測試**：adapter 對假伺服器，涵蓋 200／401／404／逾時。

**前端 Vitest**：

- `ModelPickerButton` 掛載時只查**選中的那一個** provider（呼叫 `usage_quota`），**不是** `usage_quota_all`。
- 下拉展開時才呼叫 `usage_quota_all`。
- 收合狀態在 severity 為 normal 時**仍然顯示**徽章（這是 A 案的核心，最容易在改動中被退回成「只有超標才顯示」）。
- 切換 provider 會立即重查新選中者。
- 視窗失焦時停止輪詢。
- 配額查詢失敗時選單仍可正常選取 provider，按鈕仍可點擊。
- `UsagePage` 的區間切換與空狀態。

> **fixture 一律取自探勘 dump 的真實回應，不要自己編。** 自編 fixture 會把錯誤的假設固化成「通過的測試」。

---

## 風險

- **逆向端點**：三個都是無文件端點，上游隨時可能改。緩解：解析失敗一律降級成「—」，永不影響 AI 主流程；解析程式碼集中在三個 adapter，改動面小。
- **`api/oauth/usage` 的 header 依賴**：需要 `anthropic-beta: claude-code-20250219,oauth-2025-04-20` 與 `x-app: cli`。這組 header 已在既有 `anthropic.rs:52-54` 使用中，共用同一份常數，不要重複定義 —— 重複常數是 bug 溫床。
- **單價表會過期**：查不到單價就不顯示金額，不猜。
- **`plan_type: free` 的窗口是 30 天**：不要把「5 小時窗」寫死進任何 label 或邏輯，一律由 `limit_window_seconds` 推導。

---

## 已產生的改動（本次探勘）

- 新增 `src-tauri/tests/usage_probe.rs`（標 `#[ignore]`，不進常規 CI）
- `src-tauri/src/ai/router.rs:122` 的 `get_valid_oauth_token` 由 `pub(crate)` 放寬為 `pub`，比照既有 `get_valid_codex_oauth_token` 的先例（`router.rs:325`），純可見度調整、行為不變

---

## 實作順序建議

1. **本地累計**（`usage.db` + `MeteredProvider` 裝飾器 + `TokenUsage` 補 cache 欄位）—— 零外部依賴，先落地
2. **配額 adapter 三家** + 快取 + IPC —— 有探勘 fixture 可先寫測試
3. **`ModelPickerButton` 徽章**
4. **Settings 用量頁**
5. **agent mission 即時用量**

1 與 2 互不依賴，可並行。
