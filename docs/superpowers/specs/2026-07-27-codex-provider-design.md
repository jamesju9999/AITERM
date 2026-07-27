# 新增 AI Provider：Codex（ChatGPT 訂閱帳號認證）

**日期**：2026-07-27
**狀態**：待審閱

## 背景與目標

AITerm 目前支援 OpenAI、Anthropic（含 OAuth）、Ollama、通用 OpenAI-Compatible、GitHub Copilot、Google AI、OpenRouter、xAI、DeepSeek、Kimi、Anthropic-Compatible 共 11 種 provider 類型。本次目標是新增第 12 種：**Codex**——讓使用者用自己的 ChatGPT 訂閱帳號（Plus/Pro/Team，含 Codex 權限）登入，而不是輸入一般 OpenAI API Key。

技術依據：參考 `/Users/jamesju/Documents/CodeSample/OmniRoute-release-v3.8.49`（唯讀探索，未複製任何程式碼）整理出的 Codex/ChatGPT OAuth 與 API 呼叫細節，對照 AITerm 現有的三種 OAuth 先例（Anthropic PKCE 手動貼碼、Google 本機 HTTP server、GitHub Copilot device flow token 交換）重新設計。

## 範圍界定（重要決策，已與使用者確認）

| 決策點 | 選擇 |
|---|---|
| 實作範圍 | **MVP：能登入 + 能對話**。不做 reasoning-effort 選擇 UI、不解析/顯示 quota headers、不做同裝置多 ChatGPT 帳號切換 UI |
| 認證流程 | PKCE authorization code + **固定 port 1455 的本機 HTTP server**（比照 `google_oauth_login`，但 port 不能隨機——Codex 的 client_id 只註冊了 `http://localhost:1455/auth/callback` 這一個 redirect_uri） |
| API 呼叫方式 | 新寫 `src-tauri/src/ai/codex.rs`，直接實作 `AiProvider` trait（**不**透過 `OpenAiCompatibleClient`，因為 Responses API 的請求/回應格式跟 Chat Completions 完全不同） |
| 工具呼叫（tool calling） | MVP **不實作** `generate_with_tools`，沿用 trait 預設的 `Unsupported`——`/agent` 模式在 Codex provider 下會明確回報不支援，之後可另外評估 |
| 模型清單 | **動態抓取**（`GET https://chatgpt.com/backend-api/codex/models`），失敗時退回一組寫死的少量已知模型 id |
| Refresh token | 一次性輪替，refresh 請求**不帶 `scope`**，成功後**必定用新 refresh_token 覆蓋舊的** |

### 明確排除（Non-goals）

- 不做 reasoning-effort（`-high`/`-medium`/`-low` 後綴）選擇 UI，也不做 model slug 的 base+effort 拆分邏輯——MVP 直接把使用者選的模型字串原封不動放進 `model` 欄位送出（風險見「待驗證假設」）。
- 不解析 `x-codex-5h-usage` 等 quota headers，不在 UI 顯示用量。
- 不支援同一台裝置切換多個 ChatGPT 帳號的 UI（底層 `prompt=login` 參數已經避免帳號互踩，但沒有「帳號管理」畫面）。
- 不支援 `originator: openai_native`（「ChatGPT 原生登入」）這個變體，只做 `codex_cli_rs`。
- 不支援 `/responses/compact` 這個精簡端點變體。
- 不移植 OmniRoute 的 GitHub 型錄 fallback（`raw.githubusercontent.com/openai/codex/.../models.json`）——三層 fallback 簡化成兩層：即時 API → 寫死清單。

## 認證資料流

1. 使用者在設定頁按「使用 ChatGPT 帳號登入」。
2. 前端呼叫新的 `codex_oauth_login` command。
3. 後端產生 PKCE code_verifier/code_challenge + state，`TcpListener::bind("127.0.0.1:1455")`——bind 失敗（port 被占用）直接回傳清楚錯誤（例如「1455 port 被占用，請關閉相關程式後再試」），無 fallback。
4. 用預設瀏覽器開啟：
   ```
   https://auth.openai.com/oauth/authorize
     ?response_type=code&client_id=app_EMoamEEZ73f0CkXaXp7hrann
     &redirect_uri=http://localhost:1455/auth/callback
     &scope=openid+profile+email+offline_access
     &code_challenge=<S256>&code_challenge_method=S256
     &id_token_add_organizations=true&codex_cli_simplified_flow=true
     &originator=codex_cli_rs&prompt=login&state=<random>
   ```
   （`prompt=login` 是關鍵——強制重新登入避免 Auth0 session 沿用，讓同一 `client_id` 下不同 ChatGPT 帳號的 refresh token 不互相頂替。）
5. 使用者用自己的 ChatGPT 訂閱帳號完成登入。
6. 瀏覽器導回 `localhost:1455/auth/callback?code=...&state=...`；本機 server（比照 `google_oauth_login` 的 `tokio::net::TcpListener` + 手動解析 HTTP request 寫法，逾時 2 分鐘）收下，回一頁「登入成功，可關閉視窗」HTML，關閉連線。
7. 驗證 `state` 相符後，POST `https://auth.openai.com/oauth/token`（**form-urlencoded**，非 JSON）：
   ```
   grant_type=authorization_code&client_id=app_EMoamEEZ73f0CkXaXp7hrann
   &code=<code>&redirect_uri=http://localhost:1455/auth/callback
   &code_verifier=<verifier>
   ```
8. 解析回傳的 `id_token`（JWT，只需 base64url decode payload 部分，不驗簽——因為整段 token exchange 本身走 HTTPS 且來自官方 token endpoint，信任層級等同其他既有 OAuth 流程對 access_token 的處理），取 `https://api.openai.com/auth.chatgpt_account_id` claim 當作 `chatgpt-account-id`。
9. 存進 keychain：
   - `{provider_id}` → access_token
   - `{provider_id}:oauth_refresh` → refresh_token
   - `{provider_id}:oauth_expires_at` → 過期時間戳
   - `{provider_id}:oauth_account_id`（新欄位）→ chatgpt-account-id
10. `config.update`：設定該 provider `auth_method = Some("oauth")`。

## 後端設計（Rust，`src-tauri/src/`）

### 1. `config/types.rs`

`ProviderType` enum 新增 `Codex` variant，沿用 `#[serde(rename_all = "kebab-case")]` 自動序列化為 `"codex"`。`ProviderConfig` 不需新增欄位。

### 2. `src-tauri/src/ai/codex.rs`（新檔，比照 `anthropic.rs` 等級）

```rust
pub struct CodexClient {
    access_token: String,
    model: String,
    chatgpt_account_id: Option<String>,
    client: reqwest::Client,
}
```

- **常數**：`CODEX_RESPONSES_URL = "https://chatgpt.com/backend-api/codex/responses"`、`CODEX_CLIENT_VERSION = "0.144.1"`、User-Agent 字串常數。
- **`generate()`**：
  - 把 `GenerateRequest.messages` 轉成 `input: [{ "type": "message", "role": ..., "content": [{ "type": "input_text", "text": ... }] }]`。
  - `instructions` 直接設為 `req.system_prompt`（AITerm 本來就會產生完整 system prompt，滿足 Codex「`instructions` 必填」的限制，不需要 OmniRoute 那套 placeholder 邏輯）。
  - 強制 `stream: true`、`store: false`。
  - Headers：`Authorization: Bearer <access_token>`、`originator: codex_cli_rs`、`User-Agent: codex-cli/0.144.1 (...)`、`Version`、`Openai-Beta: responses=experimental`、`X-Codex-Beta-Features: responses_websockets`，有 `chatgpt_account_id` 就加 `chatgpt-account-id`。
  - 新寫一個 Responses-API 專用的 SSE 消費函式（`response.output_text.delta` → 累積 delta 文字；`response.completed`/`response.failed` → 結束），跟現有 `sse::consume_openai_sse`（Chat Completions 格式）分開，不共用 payload 解析邏輯，但可共用最底層的「用空行切 SSE frame」邏輯。
  - HTTP 錯誤（非 2xx）：比照現有 `map_http_error` 模式轉成 `AiError`。
- **`health_check()`**：打一次 models 端點（見下方 `get_codex_oauth_models` 的邏輯）驗證 token 有效。
- **`generate_with_tools()`**：不覆寫，沿用 trait 預設 `Unsupported`。

### 3. `commands/provider.rs`

新增常數：
```rust
const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_OAUTH_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CODEX_REDIRECT_PORT: u16 = 1455;
```

新增指令：
- `codex_oauth_login`（比照 `google_oauth_login` 整體結構：固定 port 而非隨機 port；token exchange 用 `.form(&params)` 而非 `.json()`；額外解析 `id_token` 取 `chatgpt_account_id`）。
- `codex_oauth_logout`（清除四把 keychain 條目，重設 `auth_method`）。
- `get_codex_oauth_models`：`GET https://chatgpt.com/backend-api/codex/models?client_version=0.144.1`，帶跟 `generate()` 相同的 headers。回應形狀容錯解析（`{models:[...]}` / `{data:[...]}` / 裸陣列 / 物件 map 四種都要接住），每筆項目的 id 依 `slug` → `id` → `model` 依序 fallback，取不到值的項目略過，最後排序回傳；請求失敗或清單為空則回傳一組寫死的少量已知模型 id 當保底。
- 共用小 helper：JWT payload 解碼（base64url decode 中間段 + `serde_json::Value` 解析出指定 claim 路徑）。

### 4. `ai/router.rs`

新增 `ProviderType::Codex` match arm：
- `get_valid_codex_oauth_token()`（新函式，結構比照既有 `get_valid_oauth_token`/`get_valid_google_oauth_token`：5 分鐘緩衝提前 refresh）。
- Refresh 請求 body **不含 `scope`** 欄位；成功回應的 `refresh_token` 一律覆蓋舊值存回 keychain（現有 Anthropic/Google 分支本來就有這行為，這裡沿用同樣寫法，只是明確標註「Codex 這裡不可省略，否則下次 refresh 會用到已作廢的 token」）。
- Refresh 失敗時的行為沿用現有寬鬆策略（記警告、繼續用舊 token），不額外做「可恢復 vs 不可恢復」的錯誤分類——MVP 範圍內簡化，實際請求若真的認證失敗會透過既有 `AiError::AuthFailed` 路徑回報。
- 讀出 keychain 裡的 `{id}:oauth_account_id`，建構 `CodexClient::new(token, model, chatgpt_account_id)`。

### 5. `lib.rs`

把 `codex_oauth_login`/`codex_oauth_logout`/`get_codex_oauth_models` 加進 import 與 `invoke_handler!` 清單。

## 前端設計（`src/`）

- `src/ipc/config.ts`：`ProviderType` union 新增 `"codex"`。
- `src/ipc/provider.ts`：`PROVIDER_TYPE_LABELS` 新增 `Codex (ChatGPT)`；新增 3 個 IPC 包裝函式（`codexOAuthLogin`/`codexOAuthLogout`/`getCodexOAuthModels`）。
- `src/components/Settings/ProviderForm.tsx`：`PROVIDER_TYPES` 加入 `codex`；不顯示 base_url 欄位（固定端點）；比照 Anthropic OAuth 現有 UI 放一顆「使用 ChatGPT 帳號登入」按鈕 + 登入狀態顯示（已登入的帳號/登出按鈕）；模型欄位用 `<input list>` + `<datalist>`（比照 Google AI/OpenRouter 等既有動態抓取 provider 的模式）。
- i18n（`src/lib/i18n.ts`）補中英文字串：provider 顯示名稱、登入/登出按鈕文字、登入中/登入失敗提示。

## 錯誤處理

- 1455 port 被占用：`codex_oauth_login` 直接回傳明確錯誤，不重試、不換 port。
- OAuth 逾時（2 分鐘未完成登入）：比照 `google_oauth_login` 現有訊息格式。
- `state` 不符：視為過期或被竄改，要求重新開始登入流程。
- Token exchange 失敗：回傳 HTTP 狀態碼 + body 內容（比照現有 Anthropic/Google 分支）。
- `chatgpt_account_id` claim 缺失：非致命，該次登入仍完成，只是後續請求不帶 `chatgpt-account-id` header（best-effort）。
- Refresh 失敗：沿用現有寬鬆策略（見上方 router.rs 段落）。
- Models 端點回應形狀不符預期／請求失敗：退回寫死的保底清單，不阻擋使用者儲存設定。

## 測試計畫

- Rust：`codex.rs` 針對 request body 組裝寫 unit test（`system_prompt` → `instructions`、messages → `input` 陣列形狀、`stream:true`、`store:false` 皆被強制設定）；header 組裝測試（`originator`、User-Agent、`chatgpt-account-id` 存在時才出現）——比照 `anthropic.rs` 既有的 sentinel-header 測試風格。
- Rust：`router.rs` 新增 `codex_provider_without_oauth_token_is_not_configured` 測試，比照既有各 provider 的 `*_without_api_key_is_not_configured` 風格。
- 前端：暫無自動化測試涵蓋 `ProviderForm.tsx` 的 OAuth 按鈕流程，本次不新增測試框架，僅手動驗證。
- **無法自動化的部分**：真正的 OAuth 登入（需要一組真實 ChatGPT 訂閱帳號）與端對端對話驗證，需要使用者本人手動跑一次確認。

## 待驗證假設（實作階段需確認，不阻擋設計核准）

1. **模型 slug 格式**：即時 `models` 端點回傳的 `slug`（例如可能是 `gpt-5.6-sol-high` 這種「base+effort」組合格式，也可能是純 base model）到底能不能直接放進 `/responses` 請求的 `model` 欄位送出——OmniRoute 的內部型錄用組合格式是給自己 UI 用的，送出前會拆成 `model` + `reasoning.effort` 兩個欄位；MVP 不做拆分邏輯，直接原樣送出。實作時需要用真實帳號呼叫一次 models 端點看實際回應內容，並用回傳的第一個 slug 直接發一次 `/responses` 請求驗證是否可行；如果不行，最小修正是解析後綴、拆成 `model`+`reasoning.effort` 兩欄位（沿用 OmniRoute 的正規表達式模式），不影響本次其餘設計。
2. **`chatgpt-account-id` 是否為所有帳號類型（含純 Personal、無 Team 組織）都會出現的 claim**——若某些帳號完全沒有這個 claim，代表對應請求會永遠缺這個 header；根據 OmniRoute 的程式碼推斷這是常態存在的欄位，但沒有 100% 確認，實作時用真實帳號登入一次即可確認。
