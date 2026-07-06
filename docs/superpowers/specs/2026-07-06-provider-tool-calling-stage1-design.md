# Provider Tool-Calling 序列化修正 — Stage 1：OpenAI + Ollama（設計文件）

日期：2026-07-06
狀態：待核可
範圍：三個 provider（OpenAI、Anthropic、Ollama）中，本文件只涵蓋 OpenAI 與 Ollama。
Anthropic 因訊息格式差異巨大（無 `tool` role、需 content-block 轉換、需角色交替合併），
另立 Stage 2 設計文件與實作計畫。

## 背景

`src-tauri/src/ai/` 下四個 provider 的 `generate_with_tools` 在序列化 outgoing messages 時，
只有 `compatible.rs`（OpenAI-compatible，Google AI/Gemini 走這條路徑，因為需要保留
`thought_signature`）正確轉發 `ChatMessage.tool_call_id` 與 `.tool_calls`。
`openai.rs`、`anthropic.rs`、`ollama.rs` 都只序列化 `{role, content}`，完全忽略這兩個欄位；
回應解析時也都寫死 `raw: None`（只有 `compatible.rs` 有填）。

後果：任何超過一輪 tool call 的多輪對話（LoopStudio 的 sub-agent/orchestrator/verifier
迴圈、或未來其他 multi-turn agent 流程）在使用 OpenAI 原生、Anthropic、Ollama 時，
第二輪起 assistant 的 tool_calls 與 tool 訊息的 tool_call_id 都會在送出時消失，
導致 provider 端出錯或模型無法理解對話歷史。

## 附帶修復：既有編譯錯誤（阻擋 `cargo test --lib` 與所有 provider 整合測試）

某次合併（`f9d8499`/`752f16a`）替 `ChatMessage` 加了 `tool_call_id`/`tool_calls` 欄位、
替 `AiError::RateLimit` 加了 `body` 欄位、把 `GenerateWithToolsResult::ToolCalls` 從
tuple variant 改成 struct variant，但沒同步更新既有測試，導致以下 12 處編譯失敗
（全部機械式修正，不涉及邏輯）：

| 檔案:行號 | 問題 |
|---|---|
| `src/ai/mod.rs:313` | `ChatMessage` 缺 `tool_call_id`/`tool_calls` |
| `tests/anthropic_client.rs:17` | 同上 |
| `tests/anthropic_client.rs:123` | `AiError::RateLimit { retry_after }` 缺 `body` |
| `tests/openai_client.rs:17` | `ChatMessage` 缺欄位 |
| `tests/openai_client.rs:157` | `ChatMessage` 缺欄位 |
| `tests/openai_client.rs:110` | `RateLimit` pattern 缺 `body` |
| `tests/ai_chat_command.rs:120` | `ChatMessage` 缺欄位 |
| `tests/ai_chat_command.rs:124` | `ChatMessage` 缺欄位 |
| `tests/compatible_client.rs:20` | `ChatMessage` 缺欄位 |
| `tests/ai_query_command.rs:77` | `ChatMessage` 缺欄位 |
| `tests/ollama_client.rs:17` | `ChatMessage` 缺欄位 |
| `tests/ollama_client.rs:160` | `ToolCalls(calls)` tuple pattern → struct pattern |

這些檔案本來就是本次要新增測試的地方，修復本身不算擴大範圍。

## Stage 1a：OpenAI（`src-tauri/src/ai/openai.rs`）

**`generate_with_tools` 的 outgoing message 建構**（目前只有 `{"role": m.role, "content": m.content}`）：
比照 `compatible.rs:136-145` 已驗證正確的邏輯——

```rust
let mut msg = serde_json::json!({"role": m.role, "content": m.content});
if let Some(id) = &m.tool_call_id {
    msg["tool_call_id"] = serde_json::Value::String(id.clone());
}
if let Some(tool_calls) = &m.tool_calls {
    msg["tool_calls"] = tool_calls.clone();
    msg["content"] = serde_json::Value::Null;
}
```

**回應解析**：目前 `return Ok(GenerateWithToolsResult::ToolCalls { calls, raw: None });` 改為
擷取 `raw = Some(choice["message"]["tool_calls"].clone())`（與 `raw_calls` 來源相同的
`serde_json::Value`，不需要 `compatible.rs` 那種「多 choice 挑選」的防禦邏輯——那是
「透過 OpenAI-compatible 代理商呼叫 Claude 擴充思考模式」的特例，原生 OpenAI 端點不會發生）。

其餘（health_check、`build_request_body`、`generate` 的串流路徑）不變 ——
`build_request_body` 只用於不帶工具的單輪查詢，`OpenAiMessage` 型別維持現狀。

## Stage 1b：Ollama（`src-tauri/src/ai/ollama.rs`）

Ollama 原生格式與 OpenAI 有兩個關鍵差異：`tool_calls[].function.arguments` 是
**JSON 物件**（OpenAI 是字串）；且沒有 `id` 概念（回應中不帶、送出時也不需要比對）。
系統統一契約（`ChatMessage.tool_calls`／`AiToolCall`）沿用 OpenAI 形狀
（`arguments` 為 JSON 字串），因此 Ollama provider 需要雙向轉換。

**型別調整**：現有 `OllamaResponseToolCall`/`OllamaResponseFunction`（目前只 `Deserialize`，
`arguments: serde_json::Value`）加上 `Serialize` derive，同一組型別雙向共用
（形狀本來就相同，不需要另外定義送出用的型別）。

**`OllamaMessage`** 加一個可選欄位：

```rust
#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaResponseToolCall>>,
}
```

**`build_messages` 送出方向轉換**：對每個 `ChatMessage`，若 `m.tool_calls` 為 `Some`
（系統統一格式：`[{id, type:"function", function:{name, arguments: "<JSON 字串>"}}]`），
逐一將 `function.arguments` 字串解析回 `serde_json::Value`，轉成
`Vec<OllamaResponseToolCall>`（丟棄 `id`/`type`，Ollama 不需要），並將該訊息的
`content` 設為空字串（比照 Ollama 自己回應時的慣例：
`{"role":"assistant","content":"","tool_calls":[...]}`）。
`role: "tool"` 的訊息不需特殊處理，`tool_call_id` 直接忽略（Ollama 不用 id 比對），
`content`/`role` 原樣傳遞。

注意：`OllamaMessage` 加了新欄位後，`build_messages` 內既有的兩個建構點
（system prompt 訊息、迴圈內每則訊息）都必須同步補上 `tool_calls` 欄位
（system prompt 固定 `None`；迴圈內依上述邏輯決定），否則無法編譯。
`build_request_body`（純文字單輪查詢，不含工具）共用同一個 `build_messages`，
不需要另外處理。

**回應解析（收回方向）**：目前 `Ok(GenerateWithToolsResult::ToolCalls { calls, raw: None })`
改為額外合成一個 OpenAI 形狀的 `raw`，與 `calls`（`AiToolCall`，`id: format!("call_{}", i)`）
的合成邏輯一致：

```rust
let raw_tool_calls: Vec<serde_json::Value> = data.message.tool_calls.iter().enumerate()
    .map(|(i, tc)| serde_json::json!({
        "id": format!("call_{}", i),
        "type": "function",
        "function": {
            "name": tc.function.name,
            "arguments": serde_json::to_string(&tc.function.arguments).unwrap_or_default(),
        }
    }))
    .collect();
let raw = Some(serde_json::Value::Array(raw_tool_calls));
```

這讓 `raw` 與送出方向的轉換邏輯自洽：下一輪送出時，`build_messages` 預期收到的正是
這種 OpenAI 形狀（`arguments` 為字串），再轉換回 Ollama 原生格式。

## 錯誤處理

- 兩個 provider 的轉換邏輯都是**盡力而為**：`function.arguments` 字串若解析失敗
  （理論上不會發生，因為字串本來就是我們自己序列化出來的），退回空物件
  `serde_json::json!({})`，不 panic、不中斷請求。
- 其餘既有錯誤處理路徑（401/429/5xx/網路錯誤/health_check）完全不變。

## 測試

- **compile-break 修正**：12 處機械修正，跑一次 `cargo build --tests` 確認全部檔案編譯通過。
- **OpenAI**（新增至 `tests/openai_client.rs`）：wiremock 測試驗證——
  1. 帶 `tool_call_id`/`tool_calls` 的 `ChatMessage` 送出後，實際 HTTP request body
     裡的對應訊息確實含有這兩個欄位（且 tool_calls 存在時 content 為 null）。
  2. Mock 回應帶 `tool_calls`，確認 `GenerateWithToolsResult::ToolCalls.raw` 不是 `None`，
     且內容與回應中的 `message.tool_calls` 一致。
- **Ollama**（新增至 `tests/ollama_client.rs`）：
  1. 送出方向：`ChatMessage.tool_calls`（OpenAI 形狀，字串 arguments）經過
     `build_messages` 後，HTTP request body 裡的 `tool_calls[].function.arguments`
     是物件而非字串。
  2. 收回方向：既有的 `generate_with_tools_returns_tool_calls` 測試更新為
     struct-pattern 並額外斷言 `raw` 是 `Some`、格式為 OpenAI 形狀
     （`raw[0]["function"]["arguments"]` 是字串、可被 `serde_json::from_str` 解析回
     `{"query":"WWDC 2026"}`）。
  3. 往返一致性測試：模擬「上一輪 raw 被原樣塞回 `ChatMessage.tool_calls`」的情境，
     驗證 `build_messages` 能正確把它轉回 Ollama 原生格式送出。
- 跑 `cargo test`（不只 `--lib`）確認整個 Rust 測試套件（含既有 PTY/exec 測試）全綠燈。

## 不做的事（本階段）

- Anthropic 的訊息格式轉換（Stage 2，另立文件）。
- 不改變 `compatible.rs`、`copilot.rs`、`router.rs`。
- 不改變前端 `src/ipc/ai.ts`／`ChatMessage` 型別契約（前端已經是正確的 OpenAI 形狀，
  問題完全在 Rust 後端序列化）。
- 不新增/修改 `AiToolCall`、`GenerateWithToolsResult` 的結構本身。
