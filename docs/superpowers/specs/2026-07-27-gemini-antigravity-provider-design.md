# 完成 Google AI 的 Gemini 訂閱（Antigravity）OAuth 認證

**日期**：2026-07-27
**狀態**：待審閱

## 背景與目標

AITerm 現有的 `ProviderType::GoogleAi` 目前只有 API Key 模式真正可用（走 `OpenAiCompatibleClient` 打公開的 `generativelanguage.googleapis.com/v1beta/openai`）。程式碼裡其實已經有一半 OAuth 骨架（`commands/provider.rs` 的 `google_oauth_login`/`google_oauth_logout`/`get_google_oauth_models`、`ai/router.rs` 的 `get_valid_google_oauth_token`/`do_google_oauth_refresh`、`ProviderType::GoogleAi` 分支裡的 `auth_method=="oauth"` 切換），但 `GOOGLE_OAUTH_CLIENT_ID`/`SECRET` 是刻意留空的字串，整條路徑從未真正接通，前端也完全沒有對應的登入 UI。

本次目標：**完成這個既有骨架**，讓使用者可以用自己的 Google 帳號（訂閱 Gemini Code Assist / Antigravity 相關方案）登入，透過 Google 內部的 **Antigravity / Cloud Code Assist API**（不是公開的 Generative Language API）使用 Gemini 對話。技術依據來自 `/Users/jamesju/Documents/CodeSample/OmniRoute-release-v3.8.49` 的唯讀研究（`src/lib/oauth/providers/antigravity.ts`、`open-sse/executors/antigravity.ts`、`open-sse/services/antigravityHeaders.ts` 等）。

**已知且經使用者確認接受的風險**：這條路徑打的是 Google 未公開的內部 API（`cloudcode-pa.googleapis.com/v1internal:*`），不是官方文件化的消費者流程。OmniRoute 自己的維運文件（`docs/security/STEALTH_GUIDE.md`）明確記載這是他們看過**最常見的 ToS 違規封帳原因**（多個 Google Ultra 帳號在啟用「自動花費 Google One AI credits 超額」選項後幾小時內被封）。本次 MVP **不做**那個會觸發封帳的 credits 超額功能，但底層挪用內部 API 這件事本身仍有 Google 單方面改版或封鎖帳號的風險，且已與使用者明確溝通並取得同意繼續。

## 範圍界定（重要決策，已與使用者確認）

| 決策點 | 選擇 |
|---|---|
| 實作範圍 | **MVP：能登入 + 能對話就好**。不做 quota/credits 顯示或自動花費、不做 thinking/reasoning 顯示、不做多帳號切換 UI、不做 tool-calling |
| Provider 型別 | **沿用既有 `ProviderType::GoogleAi`**，透過 `auth_method`（`api_key`/`oauth`）切換底層 client，不新增第 13 個 provider type |
| 指紋偽裝程度 | **完整仿真**（比照 OmniRoute）：固定 UA 假裝 `darwin/arm64`、動態抓官方 client 版本號、隱藏會洩漏「非原生 Node client」的 header |
| Client profile | 只做 `ide`，不做 `cli`（`agy` 獨立執行檔）那個變體 |
| 模型清單 | 先寫死已知的 Gemini 模型 id 當 fallback，並嘗試即時打 `fetchAvailableModels` |

### 明確排除（Non-goals）

- 不做 Google One AI credits 超額自動扣款（`ANTIGRAVITY_CREDITS=always`/`retry` 等機制）——這正是 OmniRoute 文件裡標記為「帳號封鎖熱點」的功能，本次完全不碰。
- 不解析/顯示 `x-goog-*` quota 或 5h/7d 用量 header。
- 不支援同一裝置切換多個 Google 帳號的 UI。
- 不支援 `cli`（`agy`）client profile，只做 `ide` profile 的 User-Agent/header 組合。
- 不包含同一個 Antigravity 端點下也能存取的 Claude-via-Vertex 模型（`claude-opus-4-6-thinking` 等）——那些是 Claude 品牌模型，不屬於「Gemini provider」範疇。
- 不做 `generateContent`（非串流）變體，一律走 `streamGenerateContent?alt=sse`（比照 OmniRoute，因為部分模型的非串流變體在 Cloud Code 這端會 400）。
- 不做 `onboardUser` 以外的 project 選擇 UI（例如使用者有多個 GCP project 時的手動選擇）——沿用 `loadCodeAssist` 自動回傳的第一個 `cloudaicompanionProject`。

## 認證資料流

1. 使用者在設定頁把 Google AI 供應商的驗證方式切到「Google 帳號登入」，按登入。
2. 後端沿用既有 `google_oauth_login` 的本機 loopback server 寫法（動態挑 port，不像 Codex 需要固定 1455——Antigravity 的 client 是 Google 的 installed-app 型別，允許任意 `127.0.0.1` port），開瀏覽器導向：
   ```
   https://accounts.google.com/o/oauth2/v2/auth
     ?response_type=code&client_id=<antigravity client_id>
     &redirect_uri=http://localhost:<port>/oauth2callback
     &scope=https://www.googleapis.com/auth/cloud-platform
            https://www.googleapis.com/auth/userinfo.email
            https://www.googleapis.com/auth/userinfo.profile
            https://www.googleapis.com/auth/cclog
            https://www.googleapis.com/auth/experimentsandconfigs
     &code_challenge=<S256>&code_challenge_method=S256&state=<random>
     &access_type=offline&prompt=consent
   ```
   **注意：scope 不含 `openid`**——OmniRoute 的註解明講，帶 `openid`（配合 PKCE）會讓 Google 導向一個會卡住的 `firstparty/nativeapp` 同意畫面，這是實測修正過的地雷，必須原樣保留這個「不含 openid」的決定。
3. 使用者完成登入，callback 回本機 server，驗證 `state`，用 code 換 token（**form-urlencoded**，且**要帶 `client_secret`**——Google 的 installed-app client 型別即使是「公開」client 也會配一組 secret，這不是需要保密的機密，是 Google OAuth 這個 client 型別的標準做法）。
4. **Onboarding（Codex 沒有這一步，是 Gemini/Antigravity 特有的）**：
   - 用拿到的 access_token 打 `POST https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist`，body `{"metadata":{"ideType":"ANTIGRAVITY"}}`。
   - 若回應的 `cloudaicompanionProject` 有值 → 直接使用，登入流程視為完成。
   - 若沒有值（全新 Google 帳號，還沒有 Cloud Code project）→ 打 `POST .../v1internal:onboardUser`（body 含 tier id + metadata），輪詢最多 10 次、每次間隔 5 秒等待 Google 佈建完成，然後重打一次 `loadCodeAssist` 取得剛佈建好的 project id。整段登入流程因此可能耗時到 ~50 秒，UI 需要顯示對應的等待訊息，不能讓使用者以為卡住了。
5. 存進 keychain：`{provider_id}`（access_token）、`{provider_id}:oauth_refresh`、`{provider_id}:oauth_expires_at`、新增 `{provider_id}:project_id`。
6. `config.update`：設定該 provider `auth_method = Some("oauth")`（與 Anthropic/Codex 共用同一套語意）。
7. 之後每次對話，`AntigravityClient` 都要在 request body 帶上 `project`；若讀不到 `project_id`（理論上不該發生，但防禦性處理）就先嘗試重打一次 `loadCodeAssist` 補救，還是沒有就回明確錯誤，不送出必定失敗的請求。
8. Refresh：Google 的 refresh token **不輪替**（跟 Codex 不同），可以用比較長的提前刷新緩衝（沿用 15 分鐘，比照 OmniRoute 對 Google 系 provider 的設定，比其他 provider 的 5 分鐘更寬鬆），refresh 成功後只需要更新 access_token 與過期時間，不需要覆蓋 refresh_token。

## 後端設計（Rust，`src-tauri/src/`）

### 1. `src-tauri/src/ai/antigravity.rs`（新檔，比照 `codex.rs` 等級）

```rust
pub struct AntigravityClient {
    access_token: String,
    project_id: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}
```

- **常數**：`ANTIGRAVITY_RUNTIME_BASE_URL = "https://cloudcode-pa.googleapis.com"`（MVP 只打穩定 host，不做 `daily-cloudcode-pa.googleapis.com` canary 的 fallback）、`ANTIGRAVITY_FALLBACK_IDE_VERSION`（寫死版本號，動態抓取失敗時使用）。
- **`generate()`**：
  - 把 `GenerateRequest.messages` 轉成 `contents: [{role: <"user"|"model">, parts: [{text}]}]`（`ChatMessage.role == "assistant"` 對應到 Gemini 的 `"model"`；其餘角色原樣，MVP 不特別處理 `"tool"` 角色，因為不支援 tool-calling）。
  - `system_prompt` 對應到 `systemInstruction: {parts:[{text}]}`。
  - `generationConfig` 強制 `topK: 40, topP: 1.0, maxOutputTokens: 16384`（比照 OmniRoute 的硬性上限，避免送出會被上游拒絕的值）。
  - 外層 envelope：`{project: self.project_id, requestId: "agent/<epoch-ms>/<8-hex>", request: {...}, model: self.model, userAgent: "antigravity", requestType: "agent"}`。
  - Headers：`Authorization: Bearer`、`Content-Type: application/json`、`Accept: text/event-stream`、`User-Agent`（見下方指紋章節）。
  - Endpoint 固定 `POST {base_url}/v1internal:streamGenerateContent?alt=sse`。
  - 新寫一個 Gemini 原生 SSE 消費函式：解析 `data: {"candidates":[{"content":{"parts":[{"text":...}]},"finishReason":...}],"usageMetadata":{...}}`，沒有明確 finish 事件時用串流結束（HTTP body 關閉）當作 done 的訊號（比照 `consume_codex_sse`/`consume_anthropic_sse` 的「串流結束就送 done」保底邏輯）。
- **`health_check()`**：打一次 `fetchAvailableModels`（見下方 provider.rs 章節的邏輯）驗證 token 有效。
- **`generate_with_tools()`**：不覆寫，沿用 trait 預設 `Unsupported`（MVP 不支援 tool-calling）。

**指紋偽裝（`apply_headers`/UA 組裝）：**
- User-Agent 固定格式：`antigravity/ide/<version> darwin/arm64`——**不管使用者實際上是 macOS/Windows/Linux，一律回報 darwin/arm64**（比照 OmniRoute 的既有做法：上游對 Mac build 的處理比較寬鬆）。
- `<version>` 來源：先嘗試打 Antigravity IDE 的 auto-updater feed 拿最新版本號（含快取），失敗則退回寫死的版本常數。MVP 用「進程內快取 + 一次性嘗試」即可，不需要 OmniRoute 那種 6 小時 TTL 的精細快取（先簡化，若日後版本號真的常變動再加強）。
- Headers 送出前不刻意加入 `sec-ch-ua-*`/`x-stainless-*` 之類的東西（Rust 的 `reqwest` 預設就不會加這些瀏覽器/SDK 專屬 header，這點跟 OmniRoute 用 Node.js 的處境不同——OmniRoute 要「清掉」是因為它跑在可能被其他 SDK header 污染的環境，AITerm 用 `reqwest` 直接發送則從一開始就沒有這個污染源，不需要額外的清除邏輯，但仍要注意不要自己手滑加了 Tauri/reqwest 預設之外的多餘 header）。

### 2. `src-tauri/src/commands/provider.rs`

- 把 `GOOGLE_OAUTH_CLIENT_ID`/`GOOGLE_OAUTH_CLIENT_SECRET` 的空字串換成真正的 Antigravity client_id/secret（`1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com` +對應 secret；secret 值需要在實作時從真實 Antigravity/`agy` client 流量或公開文件重新確認，不從 OmniRoute 的 XOR-masked 常數硬解，避免對可能屬於受保護憑證的資料做不必要的處理）。
- `GOOGLE_OAUTH_SCOPE` 常數改為：`"https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs"`（**移除 `openid`**）。
- `google_oauth_login`：在既有的「換 token → 存 keychain」流程中間，插入 onboarding 步驟（新函式 `perform_antigravity_onboarding(access_token) -> Result<String, String>`，內部處理 `loadCodeAssist` + 條件式 `onboardUser` 輪詢），把回傳的 project id 存進 `{provider_id}:project_id`。
- `get_google_oauth_models`：改打 `GET https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels`（帶同一組 headers），解析失敗或清單為空就回退一組寫死的已知 Gemini 模型 id（例如 `gemini-2.5-pro`、`gemini-2.5-flash`、`gemini-2.5-flash-lite` 這幾個現行公開型號名稱對應的 Antigravity 端點 id，實作時需要驗證這幾個 id 在該端點下是否真的可用——這正是待驗證假設之一）。

### 3. `src-tauri/src/ai/router.rs`

- `get_valid_google_oauth_token` 簽名改為回傳 `(String, String)`（access_token, project_id），project_id 從 `{id}:project_id` 讀取；缺失時嘗試呼叫 provider.rs 的 onboarding 函式重新取得一次。
- `do_google_oauth_refresh`：簡化——成功後只更新 `access_token`/`oauth_expires_at`，**不覆蓋 `oauth_refresh`**（Google 的 refresh token 不輪替，覆蓋反而多此一舉且與事實不符）。
- `ProviderType::GoogleAi` 分支：`is_oauth` 為真時，改為 `Arc::new(crate::ai::antigravity::AntigravityClient::new(token, project_id, provider_cfg.model.clone()))`；api-key 分支完全不動。

## 前端設計（`src/`）

- `ProviderForm.tsx`：`google-ai` 類型比照 `anthropic` 現有的「API Key / OAuth」分頁 UI，新增 `googleAuthMethod` state（`"api_key" | "oauth"`）+ 對應的 tab 按鈕；OAuth 分頁內容比照剛做完的 Codex 登入/登出按鈕（`googleOAuthLogin`/`googleOAuthLogout`，這兩個 IPC 函式已經存在，只是從未被呼叫）。
- OAuth 模式下隱藏 base_url 欄位（固定端點，跟 Codex 一致的處理）；api-key 模式維持現況（顯示 base_url，預設 `generativelanguage.googleapis.com/v1beta/openai`）。
- 模型欄位：OAuth 模式下改呼叫 `getGoogleOAuthModels`（已存在的 IPC 函式）取代目前 api-key 模式用的 `getGoogleAiModelsByProvider`。
- 登入按鈕需要顯示「登入中…（可能需要最多 50 秒，正在準備您的 Google Cloud 專案）」這類等待提示，因應 onboarding 輪詢可能耗時的情況。
- `handleSave` 的 `auth_method` 邏輯需要仿照 Codex 那次修的方式，比照 `anthropic` 一樣把 `google-ai` 也納入「OAuth 登入後存檔要保留 `auth_method:"oauth"`」的分支，避免重蹈覆轍。

## 錯誤處理

- Onboarding 逾時（`onboardUser` 輪詢 10 次仍未 `done`）→ 回傳清楚的逾時錯誤，附上「請稍後在 Google Cloud Console 確認是否已啟用 Gemini Code Assist」之類的指引文字。
- `loadCodeAssist`/`onboardUser` 任何一步 HTTP 失敗 → 整個登入流程失敗並回報，不把使用者留在「token 存了但沒有 project id」的半殘狀態（如果 onboarding 失敗，不儲存 token，要求使用者重試）。
- 對話請求時 `project_id` 缺失 → 嘗試重新 `loadCodeAssist` 一次；仍失敗則回傳結構化的「缺少 Google Cloud 專案 id，請重新登入」錯誤，不送出必定 422 的請求。
- Refresh 失敗 → 沿用既有寬鬆策略（記警告、繼續用舊 token），跟 Anthropic/Codex 一致。
- Models 端點失敗/回應為空 → 退回寫死清單，不阻擋儲存設定。

## 測試計畫

- Rust：`antigravity.rs` 針對 request body 組裝（`contents`/`systemInstruction`/`generationConfig` 強制值、envelope 的 `project`/`userAgent`/`requestId` 格式）、header 組裝（UA 格式、`darwin/arm64` 固定值）寫 unit test；SSE 解析用 wiremock 整合測試（`tests/antigravity_client.rs`，比照 `codex_client.rs`）。
- Rust：`router.rs` 補 `google_ai_oauth_provider_without_token_is_not_configured` 測試。
- Rust：onboarding 邏輯（`loadCodeAssist`/`onboardUser` 輪詢）用 wiremock 模擬「一次成功」「需要輪詢兩次才 done」「逾時」三種情境。
- 前端：暫無自動化測試涵蓋 `ProviderForm.tsx` 的 OAuth 分頁邏輯，僅手動驗證。
- **無法自動化的部分**：真正的 OAuth 登入、onboarding、對話，都需要使用者本人用真實 Google 帳號手動跑一次驗證。

## 待驗證假設（實作階段需確認，不阻擋設計核准）

1. **Antigravity client_secret 的正確值**——OmniRoute 原始碼裡是 XOR-masked 過的，設計上刻意不從那裡硬解出來；實作時需要透過其他管道（例如攔截真實 Antigravity client 的 OAuth 流量、或查證是否有其他公開來源）取得正確的 secret 值，或先用空字串測試 Google 是否真的要求這個欄位（部分 installed-app client 即使技術上配了 secret，实际換 token 時仍可能不強制檢查）。
2. **`fetchAvailableModels` 這個端點在只有基本 scope（不含 openid）的 token 下能不能正常回應**，以及實際回傳的模型 id 命名（本次設計文件裡列的 `gemini-2.5-pro` 等 id 是「目前已知公開型號名稱」的猜測，OmniRoute 自己的目錄也註明「這是會一直變動、需要人工重新校正的清單」，且他們自己的目錄也有「官方偷改 model id 導致要做 fallback chain」的先例）。
3. **`onboardUser` 的實際等待時間**在真實新帳號上到底多長，10 次 × 5 秒（50 秒總計）的輪詢上限是否足夠——若不夠，需要調整輪詢次數或間隔。
4. **不做 `daily-cloudcode-pa.googleapis.com` canary host fallback 這件事**是否會讓某些帳號/區域的請求穩定性變差——OmniRoute 兩個 host 都打是有理由的（canary 優先），MVP 先只打穩定 host，觀察真實使用後再決定要不要補上。
