# Claude Code 橋接 — 設計

日期：2026-08-07
狀態：待使用者複審

## 問題

使用者想在 AITerm 的終端機裡執行 Claude Code CLI，但讓它背後實際跑的是 AITerm 已經設定好的 AI 供應商（本地模型、Codex 訂閱、Gemini 訂閱、任何 OpenAI 相容端點），而不是 Anthropic 官方。

Claude Code 支援用環境變數改寫後端：

```
ANTHROPIC_BASE_URL=... ANTHROPIC_AUTH_TOKEN=... ANTHROPIC_DEFAULT_OPUS_MODEL=... claude
```

但它認得的只有 Anthropic Messages API（`POST /v1/messages`，SSE 串流）。AITerm 目前 12 種 `ProviderType` 裡只有 `anthropic` / `anthropic-compatible` 講這個協定，其餘都不通。而且 AITerm 目前**沒有任何本地 HTTP server**，`ANTHROPIC_BASE_URL` 在整個 repo 是零命中。

## 現況調查（設計的事實基礎）

以下都是實際讀過程式碼確認的，不是推測：

| 事實 | 位置 |
|---|---|
| `AiProvider` trait 的串流事件只有 `GenerateChunk { delta: String, done, usage }` —— 吐不出 tool_use、thinking、cache_control | `src-tauri/src/ai/mod.rs:115` |
| `compatible.rs` 的 `generate_with_tools` 硬寫 `"stream": false`，工具呼叫只有非串流路徑 | `src-tauri/src/ai/compatible.rs:153` |
| 串流用的 `OpenAiSseDelta` 只宣告 `content` 欄位，**streamed tool_calls delta 被靜默丟棄** | `src-tauri/src/ai/sse.rs:120` |
| `CodexClient` 沒有 `generate_with_tools`，掉回 trait 預設的 `Unsupported`，連 HTTP 都不發 | `src-tauri/src/ai/codex.rs:129` |
| Codex 的 SSE 只解析 `response.output_text.delta` / `response.completed` / `response.failed`，其餘全落進 `Other` 被丟棄 | `src-tauri/src/ai/codex.rs:251` |
| `AntigravityClient` 同樣沒有 `generate_with_tools`；`GeminiPart` 只宣告 `text`，`functionCall` / `thought` / `inlineData` 被 `filter_map` 丟棄 | `src-tauri/src/ai/antigravity.rs:268` |
| Antigravity 的 `maxOutputTokens` 硬寫成 16384，`req.max_tokens` 未透傳 | `src-tauri/src/ai/antigravity.rs:328` |
| Codex access token 過期前 300 秒續期、refresh token 會輪替必須回存 | `src-tauri/src/ai/router.rs:244`, `:278` |
| Antigravity 每個請求都必須帶 `project`，只能從登入時的 onboarding 取得，無法憑空產生 | `src-tauri/src/ai/antigravity.rs:320`, `src-tauri/src/commands/provider.rs:148` |
| Anthropic OAuth（`sk-ant-oat*`）要求第一個 system block 必須是 `CLAUDE_CODE_SENTINEL`，否則回**假的** `rate_limit_error` | `src-tauri/src/ai/anthropic.rs:342` |
| `ShellSpec.envs` 在 AppImage 環境修正**之後**套用，後寫者贏 | `src-tauri/src/pty/session.rs:325` |
| 前端既有的 `isClaudeCommand()` 只比對第一個 token 的 basename，刻意排除 `FOO=1 claude` | `src/lib/claudeCommand.ts:9` |

**結論**：這不是單純的「翻譯層」工作。Claude Code 是 100% 靠 tool_use 運作的客戶端（Read / Edit / Bash / Glob 每一步都是工具呼叫），而三個上游家族**沒有一個**目前具備串流工具呼叫的能力。要先補上游能力，才輪得到翻譯。

## 參考實作

`/Users/jamesju/Documents/CodeSample/OmniRoute-release-v3.8.49`（MIT，Copyright 2026 diegosouzapw）已完整實作同類功能，翻譯層約 15,274 LOC / 53 檔（`open-sse/translator/`）。本設計**不引用其程式碼**，只把它當作「哪些坑存在」的地圖。已確認並採納的三個關鍵教訓記在下文。

## 範圍

**含：**

- AITerm 內建一個綁 `127.0.0.1` 的 Anthropic Messages API 相容 server
- 三條上游路徑：Anthropic 家族轉發、OpenAI 家族翻譯、Codex、Antigravity
- 為 Codex / Antigravity / 串流 OpenAI **補上工具呼叫能力**（這是前置工作，非翻譯）
- 三層模型映射（opus / sonnet / haiku 各自指定 provider + 模型）
- 分頁層級的開關，開啟時把 `ANTHROPIC_*` 環境變數注入該分頁的 shell

**不含：**

- 改動現有 `AiProvider` trait 或現有 AI 面板 / Agent 模式的行為
- 讓區網其他機器使用（只綁 loopback）
- MITM 攔截 `api.anthropic.com`（OmniRoute 有，本設計不做）
- 寫入 `~/.claude/settings.json` 或 profiles（只用環境變數，不動使用者的 Claude 設定）
- 非串流的 `/v1/messages`（Claude Code 一律用串流；若收到 `stream:false` 回 400）

## 架構

### 為什麼另開模組而不是擴充現有 trait

Claude Code 需要的保真度（串流 tool_use、thinking 區塊、cache_control）AITerm 自己的 UI 完全用不到。把這些塞進共用的 `AiProvider` trait 會讓 7 支 client、Agent loop、chat hook 全進入爆炸半徑，而受益者只有一個消費者。

因此新增獨立的 `bridge/` 模組與自己的 trait，但**憑證解析與端點知識共用**：直接呼叫 `router.rs` 既有的 OAuth 函式，並把 `codex.rs` / `antigravity.rs` 裡的 URL 建構與 header 套用抽成共用自由函式，兩邊呼叫同一份。逆向來的端點知識（Codex 的 `backend-api/codex/responses`、UA 字串、`Openai-Beta` header、Antigravity 的 `v1internal` 與 `project` 欄位）**只能有一份**。

代價：短期內同一個上游有兩條程式路徑（AITerm 用的低保真、Claude Code 用的高保真）。這是刻意的，要在模組文件註記原因。

### 模組佈局

```
src-tauri/src/bridge/
  mod.rs              BridgeState（server handle、port、token）、啟動／停止
  server.rs           axum router
  auth.rs             bearer token 常數時間比對
  model_map.rs        Claude Code 的 model 字串 → (provider_id, model)
  anthropic/
    request.rs        傳入 MessagesRequest 的 deserializer
    response.rs       Anthropic SSE serializer（唯一一份）
  upstream/
    mod.rs            BridgeUpstream trait + UpstreamEvent
    openai.rs         M1
    anthropic.rs      M1
    codex.rs          M2
    antigravity.rs    M3
```

新增依賴：`axum`（`hyper` / `tower` / `tower-http` 已在 `Cargo.lock` 裡，透過 reqwest 與 tauri 傳遞進來；`tokio` 已啟用 `net` / `time` / `rt-multi-thread` / `macros` / `io-util`）。

### 端點

| 方法 | 路徑 | 說明 |
|---|---|---|
| POST | `/v1/messages` | 主要端點，SSE 串流 |
| POST | `/v1/messages/count_tokens` | Claude Code 用來做 context 管理，回粗估值 |
| GET | `/health` | 設定 UI 用來顯示 server 狀態 |

### 中立事件型別

整個設計的樞紐。三條翻譯路徑都收斂到這組事件，Anthropic SSE serializer 只寫一次；新增上游 = 寫一支 adapter，序列化端零改動。

```rust
enum UpstreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolUseStart { id: String, name: String },
    ToolInputDelta(String),   // partial JSON 片段
    ToolUseEnd,
    Done { stop_reason: StopReason, usage: Usage },
}
```

Anthropic 家族是特例：它本來就講同一個協定，解析再重組是純粹的損耗。所以 trait 回傳型別容許兩種形態：

```rust
enum UpstreamResponse {
    Passthrough(reqwest::Response),        // SSE 原樣 pipe
    Events(BoxStream<UpstreamEvent>),      // 需要翻譯
}
```

`BridgeUpstream` trait 只有一個方法：吃已解析的 `MessagesRequest` 與已解析的憑證，回傳 `UpstreamResponse`。

### 請求生命週期

1. Claude Code `POST /v1/messages`，帶 `Authorization: Bearer <token>`
2. `auth.rs` 對 keyring 裡的橋接 token 做常數時間比對；不符回 401
3. 解析 body → `MessagesRequest`；`stream != true` 回 400
4. `model_map` 判層級 → 查出 `(provider_id, model)`；查不到回 400 且訊息指向設定頁
5. **每個請求重新向 router 解析憑證**，不快取 client
6. 選對應 adapter → 翻譯 → 發出 → 取得 `UpstreamResponse`
7. `anthropic/response.rs` 序列化成 Anthropic SSE 回傳（Passthrough 則直接 pipe）

第 5 點是硬性要求，不是效能取捨：Codex 的 access token 300 秒就要續期（`router.rs:244`），Antigravity 每請求都要帶 `project`（`antigravity.rs:320`）。快取 client 會在長時間對話中途拿到過期 token。

### 層級判定用哨兵字串

我們注入的是 `ANTHROPIC_DEFAULT_OPUS_MODEL=aiterm:opus`（三層同理）。Claude Code 會把這個字串原樣放進請求的 `model` 欄位，server 直接認。

理由：比猜 `claude-opus-4-...` 這種會隨 Claude Code 版本改變的型號名穩定得多。fallback 才去比對 `opus` / `sonnet` / `haiku` 子字串（處理使用者手動覆寫環境變數的情況）。

## 上游路徑

### `provider_type` → adapter 對照

`google-ai` 依 `auth_method` 分流，這點沿用 `router.rs:463` 既有的判斷邏輯：

| `provider_type` | adapter | 里程碑 |
|---|---|---|
| `anthropic`, `anthropic-compatible` | `upstream/anthropic.rs`（Passthrough） | M1 |
| `openai`, `openai-compatible`, `ollama`, `openrouter`, `deepseek`, `kimi`, `xai`, `github-copilot` | `upstream/openai.rs` | M1 |
| `google-ai` 且 `auth_method != "oauth"` | `upstream/openai.rs` | M1 |
| `codex` | `upstream/codex.rs` | M2 |
| `google-ai` 且 `auth_method == "oauth"` | `upstream/antigravity.rs` | M3 |

**尚未支援的 provider**：設定頁的三層映射下拉要把當前里程碑還沒支援的 provider 標示為停用並註明原因；若使用者的既有設定指向這類 provider（例如 M1 階段指向 Codex），server 回 400 並在訊息裡說明該 provider 尚未支援，不要靜默 fallback 到別的 provider。

### Anthropic 家族轉發（M1，低難度）

適用 `provider_type` = `anthropic` / `anthropic-compatible`。

原樣轉發 body，只動三處：

1. 換 `model` 為映射出的模型名
2. 換 auth header —— 重用 `anthropic.rs:49` 的 `auth_request` 邏輯（API key 模式設 `x-api-key`；OAuth 模式設 `Authorization: Bearer` + `anthropic-beta: claude-code-20250219,oauth-2025-04-20` + `x-app: cli`）
3. SSE 直接 pipe

**陷阱**：OAuth 模式下第一個 system block 必須是 `CLAUDE_CODE_SENTINEL`（`anthropic.rs:342`），否則上游回假的 `rate_limit_error`。Claude Code 自己送的 system prompt 第一句正好就是那句話，多半天然滿足 —— 但仍要檢查、缺了就補上。

### OpenAI 家族翻譯（M1，主戰場）

適用 `openai` / `openai-compatible` / `ollama` / `openrouter` / `deepseek` / `kimi` / `xai` / `google-ai`（API key 模式）/ `github-copilot`，共 9 種。

**請求翻譯（Anthropic → OpenAI chat.completions）**

| Anthropic | OpenAI |
|---|---|
| `system`（字串或 block 陣列） | `messages[0]`，role `system` |
| content block `text` | text |
| content block `image`（base64 source） | `image_url` 的 `data:` URI |
| content block `tool_use` | assistant 訊息的 `tool_calls` |
| content block `tool_result` | role `tool` 訊息 + `tool_call_id` |
| `tools` | `{"type":"function","function":{name,description,parameters}}` |
| `tool_choice` | `tool_choice` |
| `max_tokens` / `stop_sequences` / `temperature` | 同名透傳 |
| `thinking.budget_tokens` | `reasoning_effort`（粗略三段映射） |
| `cache_control` | **丟棄**（只在 Anthropic 路徑保留） |

兩個非顯而易見的：

- **`tool_result` 裡夾圖片時要把圖片提到後續的 user turn** —— OpenAI 的 `tool` 訊息不能帶圖片內容。
- 連續同 role 的訊息不合併，OpenAI 端可接受。

**回應翻譯（OpenAI SSE → `UpstreamEvent`）**

核心是**新寫 index-keyed 串流 tool_calls 累積器**（`sse.rs:120` 目前完全沒有這塊）。處理 `delta.tool_calls[]` 的 `{index, id, function.name, function.arguments}`：

- 以 `index` 為 key 累積
- 工具名稱到齊才發 `ToolUseStart`（因為 Anthropic 的 `content_block_start` 就要帶 `name`，而 OpenAI 是 name 先到、arguments 分片後到）
- `function.arguments` 的每個片段直接轉成 `ToolInputDelta`（不需要等 JSON 完整，Anthropic 的 `input_json_delta` 本來就是 partial JSON）

其餘：`finish_reason` → `stop_reason`（`stop`→`end_turn`、`length`→`max_tokens`、`tool_calls`→`tool_use`）；`reasoning_content` / `reasoning` 欄位 → `ThinkingDelta`（DeepSeek 與部分相容 server 會吐）。

### Codex（M2，先補上游能力）

**前置工作（新功能，不是翻譯）**：

- Responses API 的 body 加 `tools`。注意其格式是**扁平的** `{"type":"function","name","description","parameters"}`，跟 chat.completions 的巢狀 `function` 物件不同。
- 新增 SSE 事件解析（目前 `codex.rs:251` 全部落進 `Other` 被丟棄）：

| 事件 | → |
|---|---|
| `response.output_item.added`（type=function_call） | `ToolUseStart` |
| `response.function_call_arguments.delta` | `ToolInputDelta` |
| `response.function_call_arguments.done` | `ToolUseEnd` |
| `response.reasoning_summary_text.delta` | `ThinkingDelta` |

- 工具結果回送要放 `{"type":"function_call_output","call_id","output"}` 到 `input` 陣列
- 圖片用 `input_image` content part
- `instructions` 欄位是後端必填（`codex.rs:81` 的註解），一定要送東西

**待驗證假設**：上表的事件名稱是從 Responses API 公開規格推導的，而 `chatgpt.com/backend-api/codex/responses` 是逆向的私有端點，行為未必一致。

驗證方式：在 `src-tauri/tests/` 寫一支 `#[ignore]` 的整合測試，用真實 Codex 憑證打一次帶 `tools` 的請求，把原始 SSE 全部 dump 到檔案。**先看到 dump 才動手寫 adapter。**

### Antigravity（M3，先補上游能力）

**前置工作**：

- `request` 加 `tools: [{functionDeclarations: [...]}]`
- **`GeminiPart` 重構**：從只有 `text` 的 struct 改成 enum，涵蓋 `text` / `functionCall` / `thought` / `inlineData`（`antigravity.rs:268`）
- 硬寫的 `maxOutputTokens: 16384`（`antigravity.rs:328`）改成透傳 `max_tokens`
- **Gemini 的 functionCall 沒有 id**，但 Anthropic 的 `tool_use.id` 是必填 → 要自己合成穩定 id（例如 `call_{index}_{name}` 的雜湊），並在收到對應的 tool_result 時對回去
- role 映射要顯式處理 `tool`（目前 `antigravity.rs:302` 是靠「其餘皆 user」的 fallback 誤打誤撞映對）

**待驗證假設**：`cloudcode-pa.googleapis.com/v1internal:streamGenerateContent` 是否接受 `tools`，未經證實。驗證方式同 M2。

### `/v1/messages/count_tokens`

M1 回粗估值（字元數 ÷ 4），不引入 tokenizer 依賴。理由：非 OpenAI 模型的分詞本來就不同，引入 tiktoken 也只是另一種估算。若實測發現 Claude Code 對誤差敏感再升級。

## 從 OmniRoute 採納的三個教訓

1. **開場要送真的 `event: ping` frame。** Claude Code 在等上游第一個 byte 期間遇到 SSE 靜默會斷線，而 SSE comment（`: ping`）無效。OmniRoute 的 `open-sse/utils/earlyStreamKeepalive.ts:63` 對此有明確註解。標為**待驗證假設**，但實作成本低，M1 先做。驗證方式：手動用一個刻意慢的上游（例如本地大模型冷啟動）觀察 Claude Code 是否斷線。
2. **tool_use 的 `content_block_start` 必須延後發送**，等工具名稱確定。OmniRoute 的 `response/openai-to-claude.ts:289-330` 同樣這麼做。
3. **`ANTHROPIC_BASE_URL` 不能帶 `/v1` 後綴**（`docs/guides/CLAUDE-CODE-CONFIGURATION.md:37` 明確標註）。

## 設定

### Schema

掛在 `AppConfig` 上，`#[serde(default)]` 確保向後相容（`src-tauri/src/config/types.rs`）：

```rust
pub struct ClaudeBridgeConfig {
    pub enabled: bool,
    pub port: u16,                        // 預設 8317
    pub default_on_new_tab: bool,
    pub opus:   Option<TierMapping>,
    pub sonnet: Option<TierMapping>,
    pub haiku:  Option<TierMapping>,
}

pub struct TierMapping {
    pub provider_id: String,
    pub model: String,
}
```

三層是全域單一組映射，不做多組 profile。分頁層級只決定「這個分頁要不要接上去」。

### Token

存 keyring，key `claude-bridge:token`（沿用 `SecretStore` 的既有慣例）。首次啟用時產生 32 bytes 隨機值並回存。

### Port

預設 8317。**被占用時啟動失敗，不自動漂移** —— 環境變數只能在分頁 spawn 的瞬間決定，port 漂移會讓已開分頁指向死位址。UI 顯示錯誤，讓使用者明確改 port。

### server 生命週期

常駐。`enabled` 為 true 時於 app 啟動時拉起，關閉開關時停止。綁 `127.0.0.1`，不提供區網存取。

## 環境變數注入

### 路徑

`pty_create`（`src-tauri/src/pty/commands.rs:26`）多帶一個 bool 參數 → `PtyManager::create_with_app`（`src-tauri/src/pty/manager.rs:26`）→ 取得 `default_shell()` 的 `ShellSpec` 後 push 進 `envs`。

`envs` 是在 AppImage 環境修正**之後**套用（`src-tauri/src/pty/session.rs:325`），後寫者贏，不會被覆蓋。

### 注入內容

```
ANTHROPIC_BASE_URL=http://127.0.0.1:8317
ANTHROPIC_AUTH_TOKEN=<keyring token>
ANTHROPIC_DEFAULT_OPUS_MODEL=aiterm:opus
ANTHROPIC_DEFAULT_SONNET_MODEL=aiterm:sonnet
ANTHROPIC_DEFAULT_HAIKU_MODEL=aiterm:haiku
API_TIMEOUT_MS=3000000
CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1
```

另外 `env_remove("ANTHROPIC_API_KEY")`：使用者環境若本來就有這把金鑰，會是個極難查的干擾源（症狀是「明明設了橋接卻打到真的 Anthropic」）。啟用橋接的分頁一律清掉。

### 跨平台

全部是純環境變數，三個平台的 spawn 路徑（`shell.rs:82` / `:102` / `:129`）都會套用 `ShellSpec.envs`，行為一致。server 綁 loopback 也無平台差異。

## UI

### 設定頁

新增「Claude Code 橋接」區塊：

- 啟用開關（控制 server 常駐）
- Port 輸入
- 三層映射：各一個 provider 下拉 + 模型輸入（切換 provider 時預設帶入該 `ProviderConfig.model`）
- server 狀態顯示（running / port 被占用 / 尚未設定映射）
- 「複製手動命令」按鈕，產生完整的 `ANTHROPIC_BASE_URL=... claude` 命令列，給想在 AITerm 之外使用的人

### 分頁

- 新分頁按鈕的下拉多一項「新增 Claude Code 分頁」
- 設定頁的「新分頁預設啟用橋接」開關
- 分頁標題顯示一個小標記，讓使用者知道這個分頁接著橋

不做「對已開分頁切換」—— 環境變數只能在 spawn 時決定，事後切換是騙人的。

## 錯誤處理

- 上游錯誤映射成 Anthropic 錯誤格式 `{"type":"error","error":{"type","message"}}`，讓 Claude Code 能正常顯示，而不是吐一坨原始 JSON
- 憑證解析失敗（OAuth 過期且刷不動）→ 401，訊息指出是哪個 provider
- 層級未映射 → 400，訊息指向設定頁
- `stream != true` → 400
- server 端一律寫進 AITerm 既有的 log 機制。Claude Code 那端只看得到最終錯誤字串，沒有 server log 會非常難除錯。

## 測試

| 層級 | 內容 |
|---|---|
| Rust 單元 | 請求翻譯（各種 content block 組合：純文字、圖片、tool_use、tool_result、tool_result 夾圖片）；SSE 序列化（給定 `UpstreamEvent` 序列，斷言輸出的 frame 序列）；**串流 tool_calls 累積器的分片邊界**（name 分兩片到、arguments 分五片到、多工具交錯 index） |
| Rust 整合 | wiremock 假上游，跑完整 `/v1/messages` 請求，斷言回傳的 SSE 序列；auth 失敗、層級未映射、port 占用 |
| Rust 探勘（`#[ignore]`） | M2 / M3 開工前，用真憑證 dump 原始 SSE |
| 前端 Vitest | 設定 UI 的三層映射互動；`pty_create` 帶出的 IPC 參數 |
| 手動驗收 | 在 AITerm 終端機打 `claude`，跑完一次**讀檔 → 改檔 → 跑測試**的完整 tool 循環。這是唯一能證明真的可用的測試 |

## 里程碑

| | 內容 | 驗收 |
|---|---|---|
| **M1** | axum server、auth、model_map、count_tokens、Anthropic 家族轉發、OpenAI 家族翻譯（含新寫串流 tool_calls 累積器）、config schema、env 注入、設定 UI、分頁 UI | 選一個 OpenAI 相容 provider（本地模型即可），在 AITerm 終端機跑完一次完整 tool 循環 |
| **M2** | Codex 路徑：補 Responses API 的 tools 與 function_call SSE 事件解析，接上 adapter | 同上，provider 換成 Codex |
| **M3** | Antigravity 路徑：補 functionDeclarations、重構 `GeminiPart`、合成 tool id，接上 adapter | 同上，provider 換成 Antigravity |

M2 與 M3 各自以「探勘測試 dump 原始 SSE」為第一步。若 dump 顯示端點不接受 `tools`，該里程碑就地停止並回報，不繼續寫 adapter。

**三個里程碑各自寫一份實作計畫**，不合成一份。M1 完成即可獨立合併使用；M2 / M3 是純增量，不改動 M1 已驗收的行為。

## 待驗證假設彙總

實作時必須先驗證，不可當成既定事實：

1. Claude Code 遇到 SSE 靜默會斷線，需要真的 `event: ping` frame。→ 用刻意慢的上游手動觀察。
2. Codex 私有端點接受 `tools`，且吐 `response.function_call_arguments.*` 系列事件。→ `#[ignore]` 探勘測試 dump 原始 SSE。
3. Antigravity `v1internal` 接受 `tools` 並吐 `functionCall` part。→ 同上。
4. Claude Code 對 `count_tokens` 的粗估誤差不敏感。→ M1 手動驗收時觀察是否出現異常的 context 壓縮行為。
5. Claude Code 在 `ANTHROPIC_AUTH_TOKEN` 與 `ANTHROPIC_API_KEY` 並存時優先用前者。→ 本設計以「直接清掉 `ANTHROPIC_API_KEY`」迴避，不依賴此假設。
