# OpenAI 相容橋接伺服器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** AITerm 對外提供 OpenAI 相容 API（`/v1/chat/completions`、`/v1/models`），用使用者自訂的別名表橋接到任一已設定的供應商。

**Architecture:** 新增獨立的 axum server（`bridge/openai/`），與 Claude Code 橋接（8317）平行、互不影響。入口把 OpenAI 請求翻成 Anthropic 形狀的 `MessagesRequest`（現有所有上游 adapter 的輸入型別），經 `factory::build` 取得上游，事件收斂成 `UpstreamEvent` 後由新的 OpenAI 序列化器輸出。Anthropic 家族上游需要新增「Anthropic SSE → UpstreamEvent」解析器（現有路徑是 passthrough，不解析）。

**Tech Stack:** Rust（axum 0.8、tokio、reqwest、wiremock、tower oneshot 測試）、React 19 + Vitest。

**Spec:** `docs/superpowers/specs/2026-09-30-openai-compatible-server-design.md`

**與 spec 的差異（實作前調查的結果）：**
- 預設埠改 **8319**：8318 已是 MCP tool server 的預設埠（`config/types.rs:162`）。
- 翻譯的中間型別是現有 `MessagesRequest`（`bridge/anthropic/request.rs`），不另造新型別。
- ChatGPT Web 供應商沿用 `factory::kind_for` 的「刻意不支援」。

## File Structure

| 檔案 | 責任 |
|---|---|
| `src-tauri/src/config/types.rs`（改） | `OpenAiServerConfig`、`ModelAlias`、`AppConfig.openai_server` |
| `src-tauri/src/bridge/auth.rs`（改） | 加 `OPENAI_SERVER_TOKEN_KEY` 常數 |
| `src-tauri/src/bridge/openai_server/mod.rs`（新） | `OpenAiServerState`（啟停、綁定位址） |
| `src-tauri/src/bridge/openai_server/alias_map.rs`（新） | 別名 → `TierMapping` |
| `src-tauri/src/bridge/openai_server/request.rs`（新） | OpenAI 請求 JSON → `(Value, MessagesRequest)` |
| `src-tauri/src/bridge/openai_server/response.rs`（新） | `ChunkEncoder`、`CompletionAggregator`、`error_body` |
| `src-tauri/src/bridge/openai_server/anthropic_events.rs`（新） | Anthropic SSE → `UpstreamEvent` |
| `src-tauri/src/bridge/openai_server/server.rs`（新） | router、auth、handlers、CORS |
| `src-tauri/src/bridge/mod.rs`（改） | `pub mod openai_server;` |
| `src-tauri/src/commands/openai_server.rs`（新） | Tauri 指令 status/apply/set_config/regenerate_key |
| `src-tauri/src/commands/mod.rs`、`lib.rs`（改） | 註冊 state、指令、開機自啟 |
| `src-tauri/tests/bridge_openai_server.rs`（新） | wiremock 整合測試 |
| `src/ipc/openaiServer.ts`、`src/ipc/config.ts`（改/新） | IPC 型別 |
| `src/components/Settings/OpenAiServerPage.tsx`＋`.css`＋`.test.tsx`（新） | 設定頁 |
| `src/components/Settings/SettingsView.tsx`、`src/lib/i18n.ts`（改） | 分頁入口、字串（zhTW 與 en 兩邊） |

模組命名 `openai_server`（不是 `openai`）：避免與 `bridge::upstream::openai`（上游 client）混淆。

---

### Task 1: 設定型別

**Files:** Modify `src-tauri/src/config/types.rs`（`ClaudeBridgeConfig` 之後；`AppConfig` 加欄位；`Default` 加欄位；檔尾 tests）

- [ ] **Step 1: 寫失敗測試**（加在 types.rs 現有 tests 模組，緊鄰 `claude_bridge` 那兩個測試）

```rust
#[test]
fn openai_server_config_defaults_when_section_absent() {
    // 舊的 config.toml 沒有 [openai_server] 區塊，必須照常載入。
    let cfg: AppConfig = toml::from_str("").unwrap();
    assert!(!cfg.openai_server.enabled);
    assert_eq!(cfg.openai_server.port, 8319);
    assert!(!cfg.openai_server.allow_lan);
    assert!(cfg.openai_server.aliases.is_empty());
}

#[test]
fn openai_server_config_parses_aliases() {
    let cfg: AppConfig = toml::from_str(
        r#"
[openai_server]
enabled = true
port = 9100
allow_lan = true

[[openai_server.aliases]]
alias = "gpt-4o"
provider_id = "gemini"
model = "gemini-2.5-pro"
"#,
    )
    .unwrap();
    assert!(cfg.openai_server.allow_lan);
    assert_eq!(cfg.openai_server.aliases[0].alias, "gpt-4o");
    assert_eq!(cfg.openai_server.aliases[0].model, "gemini-2.5-pro");
}
```

- [ ] **Step 2:** `cd src-tauri && cargo test --lib openai_server_config` → FAIL（欄位不存在）。

- [ ] **Step 3: 實作**

```rust
pub fn default_openai_server_port() -> u16 { 8319 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelAlias {
    /// 對外名稱，客戶端送在 `model` 欄位。
    pub alias: String,
    pub provider_id: String,
    pub model: String,
}

/// 對外的 OpenAI 相容 server。獨立於 `ClaudeBridgeConfig`：不同協定、不同開關、不同埠。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_openai_server_port")]
    pub port: u16,
    /// false = 只綁 127.0.0.1；true = 綁 0.0.0.0（區網可連）。
    #[serde(default)]
    pub allow_lan: bool,
    #[serde(default)]
    pub aliases: Vec<ModelAlias>,
}

impl Default for OpenAiServerConfig {
    fn default() -> Self {
        Self { enabled: false, port: default_openai_server_port(), allow_lan: false, aliases: Vec::new() }
    }
}
```

`AppConfig` 加 `#[serde(default)] pub openai_server: OpenAiServerConfig,`；`Default for AppConfig`（約 `types.rs:324`）加 `openai_server: OpenAiServerConfig::default(),`。

- [ ] **Step 4:** 同指令 → PASS。`cargo check` 若有其他處以字面量建構 `AppConfig` 會報錯，逐一補欄位。
- [ ] **Step 5: Commit** `feat(openai-server): config types`

---

### Task 2: 別名解析

**Files:** Create `src-tauri/src/bridge/openai_server/mod.rs`（先只放 `pub mod alias_map;`）、`alias_map.rs`；Modify `bridge/mod.rs`（`pub mod openai_server;`）

介面：
```rust
pub fn resolve(cfg: &OpenAiServerConfig, model: &str) -> Option<TierMapping>
pub fn list(cfg: &OpenAiServerConfig) -> Vec<&ModelAlias>
```
`TierMapping { provider_id, model }` 沿用 `config::types`。

- [ ] **Step 1: 測試**（alias_map.rs 內）
  - `resolves_exact_alias`：別名 `gpt-4o` → 對應 provider/model。
  - `unknown_alias_is_none`。
  - `alias_match_is_case_sensitive`（`GPT-4o` 不等於 `gpt-4o`；OpenAI 模型名區分大小寫，不做模糊比對避免誤導到錯的供應商）。
  - `duplicate_alias_first_wins`（UI 會阻擋重複，但檔案手改時行為要確定）。
- [ ] **Step 2:** 跑測試 FAIL → **Step 3:** 實作（`cfg.aliases.iter().find(|a| a.alias == model).map(|a| TierMapping{..})`）→ **Step 4:** PASS → **Step 5:** commit `feat(openai-server): alias resolution`

---

### Task 3: OpenAI 請求 → MessagesRequest

**Files:** Create `openai_server/request.rs`

介面：
```rust
pub struct Translated { pub request: MessagesRequest, pub raw: Value, pub stream: bool, pub include_usage: bool }
pub fn translate(body: &Value) -> Result<Translated, String>   // Err = 給客戶端看的 400 訊息
```
`raw` 是 Anthropic 形狀的 JSON（`Anthropic` 上游的 `send_raw` 吃它），`request` 是 `serde_json::from_value(raw.clone())` 的結果。

對應規則（每條一個測試，先寫紅燈）：

| OpenAI | Anthropic 形狀 |
|---|---|
| `messages[role=system\|developer]` | 合併成頂層 `system` 字串（多則以 `\n\n` 連接） |
| `role=user`，content 為字串 | `{"role":"user","content":"..."}` |
| content 為陣列：`{"type":"text"}` | `{"type":"text"}` |
| `{"type":"image_url","image_url":{"url":"data:image/png;base64,AAA"}}` | `{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAA"}}`；非 `data:` URL（http 連結）→ `Err("image_url 只支援 data: URL")` |
| `role=assistant` 含 `tool_calls[]` | content 陣列：文字區塊（有的話）＋每個 call 一個 `tool_use{id,name,input}`，`input` = `serde_json::from_str(arguments)`，解析失敗 → `{}` |
| `role=tool`（`tool_call_id`,`content`） | `role:"user"` 訊息，內含 `tool_result{tool_use_id,content}`；**連續的多個 tool 訊息合併進同一則 user 訊息**（Anthropic 要求 tool_result 緊接在 tool_use 之後且同一則） |
| `tools[].function{name,description,parameters}` | `tools[]{name,description,input_schema}` |
| `tool_choice`: `"auto"`／`"none"`／`"required"`／`{"type":"function","function":{"name":X}}` | `{"type":"auto"}`／不送 tools／`{"type":"any"}`／`{"type":"tool","name":X}` |
| `max_tokens` 或 `max_completion_tokens` | `max_tokens`；都沒給 → 4096（Anthropic 上游必填） |
| `temperature`、`stop`（字串或陣列） | `temperature`、`stop_sequences`（陣列化） |
| `stream`、`stream_options.include_usage` | 回傳於 `Translated` |
| `n` > 1 | `Err("n > 1 不支援")` |
| `messages` 缺少或空 | `Err("messages 不可為空")` |

- [ ] Step 1 寫上表全部測試 → Step 2 FAIL → Step 3 實作 → Step 4 PASS → Step 5 commit `feat(openai-server): translate chat.completions request`

---

### Task 4: Anthropic SSE → UpstreamEvent

**Files:** Create `openai_server/anthropic_events.rs`

介面：
```rust
pub struct AnthropicSseParser { /* 記錄目前開著的 block 型別 */ }
impl AnthropicSseParser {
    pub fn new() -> Self
    pub fn feed_line(&mut self, line: &str) -> Vec<UpstreamEvent>   // 只處理 `data:` 行
    pub fn finish(&mut self) -> Vec<UpstreamEvent>                   // 沒收到 message_stop 時補 Done
}
pub fn into_events(resp: reqwest::Response) -> impl Stream<Item = Result<UpstreamEvent, AiError>>
```
`into_events` 照 `upstream/openai/client.rs:76` 的 `unfold` 寫法（`find_line_end`／`separator_len` 共用）。

事件對應：
- `content_block_start` 的 `content_block.type == "tool_use"` → `ToolUseStart{id,name}`
- `content_block_delta`：`text_delta`→`TextDelta`；`thinking_delta`→`ThinkingDelta`；`input_json_delta.partial_json`→`ToolInputDelta`
- `content_block_stop` 且該 block 是 tool_use → `ToolUseEnd`
- `message_delta`：記 `stop_reason`（`end_turn`/`max_tokens`/`tool_use`/`stop_sequence`）與 `usage.output_tokens`；`message_start.message.usage.input_tokens` 記 input
- `message_stop` → `Done{stop_reason, usage}`
- `event: error`／`type:"error"` → 需要能表達為錯誤：`feed_line` 回傳型別改為 `Vec<Result<UpstreamEvent, AiError>>`（`AiError::ModelError{reason: message, raw}`）
- 未知事件、`ping`、壞掉的 JSON → 忽略

- [ ] 測試：一段完整的 text 回應、一段 tool_use（含分片 JSON）、thinking、`error` 事件、沒有 `message_stop` 的截斷串流（`finish()` 補 Done）、壞 JSON 行不終止。用**真實形狀的 fixture**（照 Anthropic 文件的事件序列手寫，並在 Task 9 用 wiremock 再驗一次）。
- [ ] 流程同前：紅 → 實作 → 綠 → commit `feat(openai-server): parse Anthropic SSE into upstream events`

---

### Task 5: OpenAI 回應序列化

**Files:** Create `openai_server/response.rs`

介面：
```rust
pub struct ChunkEncoder { /* id, model, created, include_usage, tool_index, started */ }
impl ChunkEncoder {
    pub fn new(id: String, model: String, include_usage: bool) -> Self
    pub fn push(&mut self, ev: UpstreamEvent) -> Vec<String>   // 每個元素是完整的 "data: {...}\n\n"
    pub fn finish(&mut self) -> Vec<String>                    // 未收到 Done 時補 finish_reason chunk；最後一定是 "data: [DONE]\n\n"（只送一次）
}
pub struct CompletionAggregator { .. }
impl CompletionAggregator { pub fn new(id: String, model: String) -> Self; pub fn push(&mut self, ev: UpstreamEvent); pub fn finish(self) -> Value }
pub fn error_body(kind: &str, code: Option<&str>, message: &str) -> Value   // {"error":{"message","type","param":null,"code"}}
pub fn error_chunk(kind: &str, message: &str) -> String                     // 串流中途失敗：data: {"error":{...}}\n\n
pub fn finish_reason(s: StopReason) -> &'static str   // EndTurn/StopSequence→"stop"、MaxTokens→"length"、ToolUse→"tool_calls"
```

規則：
- 第一個 chunk 帶 `delta:{"role":"assistant","content":""}`；之後文字為 `delta:{"content":...}`。
- `ThinkingDelta` **丟棄**（OpenAI 標準沒有對應欄位；塞進 content 會污染輸出）。
- 每個 `ToolUseStart` 遞增 `tool_calls[].index`，第一片帶 `id`、`type:"function"`、`function:{name, arguments:""}`；`ToolInputDelta` 帶 `function:{arguments: 片段}`。
- `Done` → 一個帶 `finish_reason` 的空 delta chunk；`include_usage` 為真時再送一個 `choices:[]`、`usage:{prompt_tokens,completion_tokens,total_tokens}` 的 chunk；然後 `[DONE]`。
- 聚合器：文字串接進 `message.content`；tool_calls 累積成 `[{id,type:"function",function:{name,arguments}}]`，只有工具呼叫時 `content: null`；`usage` 一律帶。
- `object`：串流 `chat.completion.chunk`、非串流 `chat.completion`；`created` 用 unix 秒。

- [ ] 測試：純文字串流的完整幀序列（斷言首幀 role、`[DONE]` 只出現一次且在最後）、兩個連續工具呼叫（index 0/1、arguments 分片可拼回合法 JSON）、`include_usage` 開／關差異、`finish_reason` 三種對映、Thinking 不外洩、`finish()` 在沒收到 Done 時補收尾、聚合器工具呼叫與文字混合。
- [ ] 流程：紅 → 綠 → commit `feat(openai-server): chat.completion serializers`

---

### Task 6: Server（router／auth／handlers）

**Files:** Create `openai_server/server.rs`

```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub secrets: Arc<SecretStore>,
    pub token: Arc<String>,
    pub tool_meta: Arc<ToolMetaCache>,
}
pub fn router(state: AppState, allow_lan: bool) -> Router
```

路由：`GET /health`（**免驗證**，回 `ok`，給狀態燈用）、`GET /v1/models`、`POST /v1/chat/completions`、`OPTIONS *`（CORS preflight）。用 `DefaultBodyLimit::max(16 * 1024 * 1024)`。

行為：
1. **auth**：只認 `Authorization: Bearer`（OpenAI 慣例；不吃 `x-api-key`）。重用 `auth::extract_token(authorization, None)`＋`auth::token_matches`。失敗 → 401 `error_body("invalid_request_error", Some("invalid_api_key"), ...)`。
2. `/v1/models`：`{"object":"list","data":[{"id":alias,"object":"model","created":0,"owned_by":"aiterm"}]}`。
3. `/v1/chat/completions`：JSON 解析失敗 → 400；`request::translate` 的 Err → 400 `invalid_request_error`；`alias_map::resolve` 為 None → 404 `error_body("invalid_request_error", Some("model_not_found"), "The model `X` does not exist")`。
4. **共用的上游開啟函式** `open_events(state, mapping, translated) -> Result<BoxStream<'static, Result<UpstreamEvent, AiError>>, AiError>`：
   - `factory::build(...)` 取得 `Upstream`。
   - `Upstream::Anthropic(a)`：`a.send_raw(&raw_with_stream_true, model, &ClientHeaders::default())` → `UpstreamResponse::Passthrough(resp)` → `anthropic_events::into_events(resp)`。**一律對上游要求 `stream:true`**（`raw["stream"] = true`），非串流客戶端由聚合器收斂，與現有 `messages_non_streaming` 對 OpenAI 上游的作法一致。
   - 其餘（OpenAi／Codex／Antigravity／ChatgptWeb）：`BridgeUpstream::send(&request, model)` → `Events(s)`；`Passthrough` 視為內部錯誤。
5. 串流：`text/event-stream` 回應；用 `mpsc` + `unfold` 模式（照 `bridge/stream.rs`），但**不需要 ping**（OpenAI 客戶端不需要，且 `: ping` 註解行對部分 SDK 是雜訊）。上游中途錯誤 → `error_chunk` 然後 `[DONE]`。
6. 非串流：聚合後回 JSON。
7. 錯誤對映 `AiError` → HTTP：沿用 `bridge::server::status_for`（需改為 `pub`，已是 `pub fn`）；type：`AuthFailed`→`authentication_error`、`RateLimit`→`rate_limit_exceeded`（code `rate_limit_exceeded`）、`NotConfigured|InvalidInput`→`invalid_request_error`、其餘 `api_error`；訊息文字沿用 `bridge::server::error_text`（把它改成 `pub(crate)`，避免重寫 Codex usage_limit 的可讀化邏輯）。
8. **CORS**：`allow_lan == false` 時**不加任何 CORS 標頭**（本機非瀏覽器客戶端不需要；瀏覽器跨站頁面藉此無法讀取回應，防 DNS rebinding／惡意網頁打本機）。`allow_lan == true` 時對 `OPTIONS` 回 204 並加 `access-control-allow-origin: *`、`allow-headers: authorization, content-type`、`allow-methods: GET, POST, OPTIONS`（Bearer key 本身就是憑證，不靠 cookie，`*` 不會擴大攻擊面）。
9. 每個請求 `log::info!("openai-server 請求 model={} → provider={} model={} stream={}", ...)`（沿用 bridge 的排錯慣例）。

- [ ] 測試放 Task 9 的整合測試檔（需要 wiremock），此 task 內只放純函式單元測試：`error_kind_for` 對映、`models_body` 形狀。
- [ ] 流程：紅 → 綠 → commit `feat(openai-server): router and handlers`

---

### Task 7: 生命週期與 Tauri 指令

**Files:** Modify `openai_server/mod.rs`；Create `commands/openai_server.rs`；Modify `commands/mod.rs`、`lib.rs`、`bridge/auth.rs`

- [ ] `auth.rs` 加：`pub const OPENAI_SERVER_TOKEN_KEY: &str = "openai-server:token";`
- [ ] `OpenAiServerState`（結構同 `BridgeState`）：`start(config, secrets, token, port, allow_lan)`；綁定位址 `if allow_lan { [0,0,0,0] } else { [127,0,0,1] }`；埠被占用回明確錯誤（訊息含位址與埠）。`port()`、`stop()`、並多一個 `lan()` 取得目前是否綁區網（供 UI 顯示）。
- [ ] 指令（模式照 `commands/bridge.rs`）：`openai_server_status`、`openai_server_apply`、`openai_server_set_config(value: OpenAiServerConfig)`、`openai_server_regenerate_key`（產生新 token 寫入 keychain；若 server 在跑則重啟套用）。`OpenAiServerStatus { running, port, lan, token, error }`（`rename_all = "camelCase"`）。
- [ ] `set_config` 驗證：別名不可空白、不可重複、埠 1024–65535，違反回 `Err(String)`（前端顯示）。
- [ ] `lib.rs`：`.manage(Arc::new(bridge::openai_server::OpenAiServerState::new()))`；`invoke_handler` 註冊四個指令；`setup` 內照 `lib.rs:328-356` 加開機自啟區塊（失敗只記 log）。
- [ ] 單元測試：`set_config` 驗證邏輯抽成純函式 `validate(&OpenAiServerConfig) -> Result<(), String>`，測重複別名、空別名、埠範圍。
- [ ] `cargo check`、commit `feat(openai-server): lifecycle and tauri commands`

---

### Task 8: 整合測試（wiremock）

**Files:** Create `src-tauri/tests/bridge_openai_server.rs`（harness 仿 `tests/bridge_server.rs`：`tempdir` + `ConfigStore::new_at` + tower `oneshot`）

測試用 config 寫入一個指向 wiremock 的 `openai-compatible` provider 與一個 `anthropic-compatible` provider，再設定別名。逐條先紅：

- [ ] 401：無 key、錯 key。`/health` 免 key。
- [ ] `/v1/models` 只列別名。
- [ ] 未登錄 model → 404 且 `code == "model_not_found"`。
- [ ] OpenAI 上游：**非串流**文字往返（上游回 SSE，回給客戶端單一 `chat.completion`，`choices[0].message.content` 正確）。
- [ ] OpenAI 上游：**串流**文字往返（斷言 `data: [DONE]` 結尾、首幀 role）。
- [ ] OpenAI 上游：**串流工具呼叫**往返（上游回 `tool_calls` 分片；客戶端拼出的 arguments 是合法 JSON、`finish_reason == "tool_calls"`）。
- [ ] 工具結果回合：客戶端送 `assistant.tool_calls` + `role:tool`，斷言 wiremock 收到的上游 body 含 `tool_result` 對應的訊息（驗證請求翻譯的雙向一致）。
- [ ] Anthropic 上游：串流文字＋工具呼叫（wiremock 回 Anthropic 形狀 SSE）→ 客戶端拿到 OpenAI chunk；並斷言上游收到的 body `stream == true`。
- [ ] 上游 429 `usage_limit_reached` body → 客戶端 429，訊息含 `plan`（沿用 `error_text`）。
- [ ] 上游 401 → 客戶端 401 `authentication_error`。
- [ ] `allow_lan=false` 時回應**沒有** `access-control-allow-origin`；`allow_lan=true` 時 `OPTIONS` 回 204 且有該標頭。
- [ ] 每個測試都要先確認會紅（暫時把實作弄壞或先不寫實作跑一次），符合 repo 的 red-light 規則。
- [ ] 跑法：`cd src-tauri && cargo test --workspace --no-fail-fast`，逐行看 `test result:`。
- [ ] commit `test(openai-server): wiremock integration suite`

---

### Task 9: 前端 IPC、設定頁、i18n

**Files:** Create `src/ipc/openaiServer.ts`、`src/components/Settings/OpenAiServerPage.tsx`、`OpenAiServerPage.css`、`OpenAiServerPage.test.tsx`；Modify `src/ipc/config.ts`（`AppConfig.openai_server: OpenAiServerConfig`）、`SettingsView.tsx`（tab `"openaiServer"`）、`src/lib/i18n.ts`（zhTW 與 enRaw **兩邊都補**）

- [ ] `ipc/openaiServer.ts`：`OpenAiServerStatus`（camelCase）、`ModelAlias`、`OpenAiServerConfig`（snake_case，與 Rust serde 一致）、`openaiServerStatus/Apply/SetConfig/RegenerateKey`。
- [ ] 頁面（版型照 `McpToolServerPage.tsx`）：狀態燈＋埠；啟用開關；埠輸入；「允許區網連線」核取方塊，勾選時顯示紅色風險警告；API key 顯示（預設遮蔽、可顯示／複製／重新產生）；別名表（每列：對外名稱輸入、供應商下拉、模型輸入；新增／刪除列；重複別名即時標紅並禁用儲存）；範例區塊：`base_url = http://<host>:<port>/v1`、一段 curl。供應商清單取自 `getConfig().providers`。
- [ ] `<host>`：`allow_lan` 為假顯示 `127.0.0.1`，為真顯示 `<本機區網 IP>` 佔位文字（不要猜 IP）。
- [ ] 測試（Vitest）：載入既有設定；勾選啟用並儲存呼叫 `openaiServerSetConfig` 帶 `enabled:true`；勾區網出現警告；重複別名禁用儲存；重新產生 key 呼叫 IPC 並更新顯示；後端回 `error` 時顯示。
- [ ] i18n：字串 key 前綴 `openai_server_`；跑既有的 i18n 語系一致性測試（**比對 `localeSources`，不是合併後物件**，見記憶「i18n 語系漂移」）。
- [ ] `npx tsc -b`、`npm run lint`、`npm run test`。
- [ ] commit `feat(openai-server): settings page`

---

### Task 10: 驗證與收尾

- [ ] `cd src-tauri && cargo test --workspace --no-fail-fast`：**逐行**讀 `test result:`，不能只看最後一行。已知的環境性失敗（aiterm-core pty openpty 競爭、MailView flaky）要明確標註為既有問題，不能混為通過。
- [ ] `npx tsc -b`、`npm run lint`、`npm run test`。
- [ ] **實機端到端**（能做的部分）：用測試 config 起 `cargo test` 中的真 `TcpListener`（`OpenAiServerState::start` 綁 127.0.0.1 隨機埠）＋ `curl` 打 `/v1/models` 與一次串流呼叫（上游用本機 wiremock 或使用者已設定的供應商）。真供應商（Codex／Gemini／Copilot）需要使用者的憑證與 GUI，我無法代為驗證，交付時**明講未實機驗證**，並給使用者 curl 範例。
- [ ] 不寫 CHANGELOG、不打 tag（記憶：tag 前要先問過使用者；CHANGELOG 屬發版動作）。
- [ ] 更新記憶：新增 project 記憶記錄本功能狀態與埠衝突的發現。

## Self-Review

- **Spec 覆蓋**：入口／別名／網路與安全（Task 6、7）／翻譯要點（Task 3–5）／前端（Task 9）／測試（Task 8、10）皆有對應 task。
- **待定項**：spec 的「預設埠 8318 是否衝突」已解決 → 8319。
- **型別一致**：`Translated`、`AppState`、`OpenAiServerConfig`、`ModelAlias`、`OpenAiServerStatus` 在各 task 用同一名稱；`TierMapping` 沿用現有型別。
- **已知取捨**：Anthropic 上游一律以串流呼叫再聚合，非串流客戶端多一次聚合但共用單一解析路徑；ThinkingDelta 不輸出；`image_url` 只支援 `data:` URL。
