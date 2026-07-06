# Provider Tool-Calling 序列化修正 Stage 2：Anthropic（設計文件）

日期：2026-07-09
狀態：待核可
範圍：只涵蓋 `src-tauri/src/ai/anthropic.rs` 的 `generate_with_tools`。
Stage 1（OpenAI + Ollama，PR #6）已完成但尚未 merge 到 master；本階段建立在
`feature/provider-tool-calling-stage1` 分支之上（因為 `tests/anthropic_client.rs`
的既有編譯修正在那個分支裡，master 上還沒有）。

## 背景

`anthropic.rs` 目前序列化 outgoing messages 時只做
`{"role": m.role, "content": m.content}`，完全忽略 `tool_call_id`/`tool_calls`；
回應解析時 `raw` 也寫死 `None`。與 Stage 1 的 OpenAI/Ollama 不同，這裡不是單純
「補欄位」就能解決——Anthropic 的 Messages API 架構本質不同：

- **沒有 `"tool"` role**。合法 role 只有 `"user"` 和 `"assistant"`。
- Assistant 的工具呼叫要放進**該則訊息自己的 `content` 陣列**，以
  `{"type":"tool_use","id":...,"name":...,"input":{...}}` block 表示。
- 工具執行結果要包成**下一則 user 訊息**的 `content` 陣列，以
  `{"type":"tool_result","tool_use_id":...,"content":...}` block 表示。
- **角色必須嚴格交替**（不能連續兩個 `user` 或連續兩個 `assistant`），否則 API 回 400。

系統內部的 `ChatMessage` 歷史是用「一則 assistant（`tool_calls` 陣列）+ N 則
`role:"tool"` 訊息（一個 tool call 一則）」表示一輪工具呼叫，這與 Anthropic 的
「一則 assistant + 一則 user（多個 tool_result block）」結構不同，需要轉換。

## 範圍確認：不做全面的角色合併

檢視系統實際產生 `ChatMessage` 歷史的地方（`useSubAgentLoop.ts` 的
`runToolLoop`、`useOrchestratorLoop.ts`），訊息序列固定是：

```
user（任務）
→ assistant（tool_calls）→ tool, tool, ...（同一輪的每個 tool call 各一則）
→ assistant（tool_calls 或最終文字）→ tool, tool, ...（或結束)
→ ...
```

**永遠不會出現**連續兩則 `user`、連續兩則 `assistant`，或 `tool` 訊息後直接接
一則新的 `user` 訊息（工具結果後一定輪到 assistant）。因此本階段**只處理「連續
`role:"tool"` 訊息合併成一則 `user` 訊息」這一種情況**，不建立通用的「任意相鄰
同角色合併」邏輯——那是在解決目前呼叫端不會產生的情境，違反 YAGNI。

## 修改內容

### 1. 回應解析（`generate_with_tools`，既有邏輯小改）

目前：
```rust
return Ok(GenerateWithToolsResult::ToolCalls { calls, raw: None });
```

改為：`raw` 保留**完整的 `content` 陣列**（不只 tool_use block），
因為 assistant 可能在呼叫工具的同時也輸出說明文字（text block + tool_use block
同時存在於同一個 `content` 陣列），保留完整陣列才不會在下一輪遺失這段文字：

```rust
return Ok(GenerateWithToolsResult::ToolCalls {
    calls,
    raw: Some(serde_json::Value::Array(content_blocks.clone())),
});
```

`calls` 的擷取邏輯（過濾出 `type == "tool_use"` 的 block）不變。

### 2. 請求建構：新函式 `build_anthropic_messages`

取代 `generate_with_tools` 目前的：
```rust
let messages: Vec<serde_json::Value> = req.messages.iter().map(|m| {
    serde_json::json!({ "role": m.role, "content": m.content })
}).collect();
```

新函式簽名：`fn build_anthropic_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value>`

單一 pass，逐一處理 `messages`：

- **`role == "tool"`**：不立即輸出，緩衝起來（收集連續出現的每一則）。
  一旦遇到非 `tool` 的訊息（或走到序列尾端），把緩衝的所有 `tool` 訊息一次
  flush 成**一則** `{"role":"user","content":[tool_result blocks...]}`，
  每個 tool_result block 為
  `{"type":"tool_result","tool_use_id": m.tool_call_id, "content": m.content}`。
- **`role == "assistant"` 且 `tool_calls` 為 `Some`**：輸出
  `{"role":"assistant","content": <content blocks>}`，其中 `<content blocks>`
  由 `to_anthropic_content_blocks(tool_calls_value)` 決定（見下方形狀偵測）。
- **其餘情況**（純文字 user/assistant，`tool_calls` 為 `None`）：原樣輸出
  `{"role": m.role, "content": m.content}`。

### 3. 形狀容錯：`to_anthropic_content_blocks`

`m.tool_calls` 這個欄位可能是兩種形狀之一：

- **Anthropic 原生形狀**（正常情況下的來源：上一輪 `raw` 被原樣塞回
  `ChatMessage.tool_calls`）——陣列每個元素已經是
  `{"type":"tool_use","id":...,"name":...,"input":{...}}`。
- **OpenAI 形狀**（前端在 `raw` 缺失時的 fallback 重建路徑，
  見 `useSubAgentLoop.ts` 的 `runToolLoop`）——陣列每個元素是
  `{"id":...,"type":"function","function":{"name":...,"arguments":"<JSON 字串>"}}`。

**形狀偵測不能只看第一個元素的 `type`**：因為「保留完整 content 陣列」的決定
（見上一節），`raw` 可能是 `[text block, tool_use block]`，第一個元素會是
`"text"` 而非 `"tool_use"`，若只檢查第一個元素會誤判成 OpenAI 形狀。
正確判斷依據：**OpenAI 形狀的元素一定有 `"function"` 鍵**（`{id, type:"function",
function:{name, arguments}}`），Anthropic 原生 content block（不論 `text` 或
`tool_use`）都**沒有** `"function"` 鍵。因此改為檢查陣列中是否**任一**元素含
`"function"` 鍵：

- 若陣列中任一元素有 `"function"` 鍵 → 視為 OpenAI 形狀，逐一轉換：
  純文字元素（沒有 `"function"` 鍵，理論上 OpenAI 形狀不會混文字，但仍需防呆）
  略過；帶 `"function"` 的元素轉成
  `{"type":"tool_use", "id": el["id"], "name": el["function"]["name"],
  "input": <解析 el["function"]["arguments"] 字串得到的物件，解析失敗則 {}>}`。
- 否則 → 整個陣列直接當作 content blocks 使用（clone），無論裡面是
  `text`、`tool_use` 或兩者混合。

這與 Stage 1 一路延續的 fail-safe 風格一致——不假設 `raw` 一定被正確填入，
兩種來源都要能正確轉換成 Anthropic 要的格式。

## 錯誤處理

- `tool_call_id` 理論上一定存在（系統內部呼叫端保證），若真的缺失，
  `tool_use_id` 欄位使用空字串而非 panic——與 Stage 1 一致的「盡力而為、
  不中斷請求」原則。
- OpenAI 形狀轉換時，`arguments` 字串解析失敗（理論上不會發生）退回空物件
  `{}`，比照 Stage 1 openai.rs/ollama.rs 既有的容錯模式。
- 其餘既有錯誤處理路徑（401/529/其他 5xx/網路錯誤/health_check）完全不變。
- `generate()`（純文字串流路徑）與 `build_request_body`/`AnthropicMessage`
  完全不受影響——本階段只改 `generate_with_tools`。

## 測試

新增至 `tests/anthropic_client.rs`（wiremock，比照現有測試風格）：

1. **單一 tool call round-trip**：送出一則帶 Anthropic 原生形狀 `tool_calls`
   的 assistant 訊息 + 一則對應的 `tool` 訊息，驗證實際送出的 HTTP body 中：
   assistant 訊息的 `content` 是含 `tool_use` block 的陣列；下一則訊息是
   `role:"user"`，`content` 含一個 `tool_result` block，`tool_use_id` 對應正確。
2. **多個平行 tool call 合併**：一則 assistant 帶 2 個 tool_use 的 `tool_calls`，
   後面接續 2 則 `tool` 角色訊息（模擬平行呼叫的結果）。驗證送出的 request body
   裡這 2 則 `tool` 訊息被合併成**恰好一則** `user` 訊息，`content` 陣列有 2 個
   `tool_result` block，且訊息總數沒有多出來的 user 訊息。
3. **OpenAI 形狀相容性**：`tool_calls` 給 OpenAI 形狀（`function.arguments` 為
   JSON 字串），驗證轉換後送出的 request body 正確產生 `tool_use` block
   （`input` 是解析後的物件，不是字串）。
4. **`raw` 含文字 + tool_use 混合**：mock 回應的 `content` 陣列同時有一個
   `text` block 和一個 `tool_use` block，驗證 `GenerateWithToolsResult::ToolCalls.raw`
   等於完整的 `content` 陣列（兩個 block 都在），而不只是 tool_use 部分。
5. **純文字訊息不受影響**：既有的 `request_body_puts_system_at_top_level`
   測試（純文字，無 tool_calls）需確認仍然通過，且不需要更動任何邏輯路徑。
6. **混合 content（text + tool_use）正確 round-trip**：`ChatMessage.tool_calls`
   直接給 `[{"type":"text","text":"..."},{"type":"tool_use","id":...,"name":...,"input":{...}}]`
   （模擬「上一輪 raw 被原樣塞回」且該 raw 混有文字與工具呼叫的情況），驗證
   `to_anthropic_content_blocks` 正確判斷為 Anthropic 原生形狀（不會因為第一個
   元素是 `text` 而誤判成 OpenAI 形狀），輸出的 `content` 陣列兩個 block 都在、
   內容不變。這是自我審查時抓到的形狀偵測邊界案例，必須要有測試鎖住。

## 不做的事（本階段）

- 不建立通用的「任意相鄰同角色合併」邏輯（範圍確認章節已說明原因）。
- 不修改 `generate()`／純文字串流路徑／`build_request_body`／`AnthropicMessage`。
- 不修改 Ollama Task 3 品質審查已記錄的既有 fast-follow 項目（id 碰撞、
  arguments 解析失敗降級）——那是獨立待辦。
- 不處理 `thought_signature`（Gemini 專屬機制，走 `compatible.rs`，與
  Anthropic 無關）。
