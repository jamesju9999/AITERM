# MCP Tool Calling for AI Chat — Design Spec

## Goal

讓 AI Chat 能呼叫 MCP 工具，支援本地模型（Ollama）與雲端模型（Anthropic、OpenAI）。使用者可看到每次工具呼叫的可展開卡片（工具名稱、輸入、輸出）。

## Architecture

```
AiPanel
  └── useMcpChat (取代 useAiChat，加入 session history)
        ├── useMcp=true & 有工具 → agent loop (最多 10 次迭代)
        │     AI → tool_calls → executeMcpTool → 加入 history → 繼續
        └── useMcp=false → 正常對話（無工具）

Rust backend:
  OllamaClient::generate_with_tools()  ← 新增
  ai_chat: system prompt fallback       ← 新增（Unsupported provider 用）
```

## Tech Stack

- Frontend: React 19, TypeScript, Tauri 2 IPC
- Backend: Rust, tokio, reqwest, Ollama `/api/chat` with `tools` param

---

## 1. Rust: Ollama `generate_with_tools`

**File:** `src-tauri/src/ai/ollama.rs`

Ollama 的 `/api/chat` 支援 `tools` 參數，格式同 OpenAI function calling。需用 **非串流** 模式（Ollama tool calling 不支援 stream）。

### Request body

```json
{
  "model": "<model>",
  "stream": false,
  "messages": [...],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "<encoded_tool_name>",
        "description": "<description>",
        "parameters": <input_schema JSON>
      }
    }
  ]
}
```

### Response parsing

```json
{
  "message": {
    "role": "assistant",
    "content": "",
    "tool_calls": [
      {
        "function": {
          "name": "<tool_name>",
          "arguments": { ... }
        }
      }
    ]
  }
}
```

- 若 `message.tool_calls` 非空 → `GenerateWithToolsResult::ToolCalls(calls)`，每個 `AiToolCall` 的 `id` 用 `format!("call_{}", index)`
- 若無 tool_calls → `GenerateWithToolsResult::Text(message.content)`

新增型別：

```rust
#[derive(Serialize)]
struct OllamaToolRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    tools: Vec<OllamaTool>,
}

#[derive(Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    kind: String,        // "function"
    function: OllamaToolFunction,
}

#[derive(Serialize)]
struct OllamaToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct OllamaToolResponse {
    message: OllamaToolMessage,
}

#[derive(Deserialize)]
struct OllamaToolMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<OllamaToolCall>,
}

#[derive(Deserialize)]
struct OllamaToolCall {
    function: OllamaToolCallFunction,
}

#[derive(Deserialize)]
struct OllamaToolCallFunction {
    name: String,
    arguments: serde_json::Value,
}
```

---

## 2. Rust: System Prompt Fallback

**File:** `src-tauri/src/commands/ai.rs`

當 `generate_with_tools` 回傳 `GenerateWithToolsResult::Unsupported` 時，不再退回無工具模式，改用 system prompt 注入。

### System prompt 追加內容

```
You have access to the following tools. To call a tool, output ONLY a JSON block in this exact format (no other text):
<tool_call>{"name":"<tool_name>","arguments":{...}}</tool_call>

Available tools:
[
  { "name": "...", "description": "...", "parameters": {...} },
  ...
]

After receiving tool results, continue the conversation naturally.
```

### Response parsing

以 regex 抓取 `<tool_call>(...)</tool_call>` 區塊：
- 找到 → parse JSON → 回傳 `GenerateWithToolsResult::ToolCalls(calls)`，id 用 `"call_sp_0"` 等
- 找不到 → 回傳 `GenerateWithToolsResult::Text(full_response)`

此 fallback 在現有的 `use_mcp=true` 分支中，`generate_with_tools` 返回 `Unsupported` 時執行。對所有文字模型通用。

---

## 3. Frontend: `useMcpChat` 加入 Session History

**File:** `src/hooks/useMcpChat.ts`

目前 `useMcpChat` 沒有 session history（useAiChat 有）。補齊以下介面，讓 AiPanel 可完整替換：

```typescript
// 新增欄位
sessions: ChatSession[]
loadMessages(messages: McpChatMessage[]): void
deleteSession(id: string): void
addMessage(msg: McpChatMessage): void   // agent loop 用
resend(): void                          // 重試最後一則
error: string | null
isStreaming: boolean                    // alias for isLoading
```

Session history 邏輯直接從 `useAiChat` 移植（localStorage key 改為 `aiterm-mcp-chat-sessions`）。

`McpChatMessage` 本身不變，歷史儲存時 `tool_call` / `tool_result` 型別照存。

---

## 4. Frontend: AiPanel 切換到 `useMcpChat`

**File:** `src/components/AiPanel/index.tsx`

- `const chat = useAiChat(sessionId)` → `const chat = useMcpChat(sessionId)`
- `chat.send(text, useMcp && mcpEnabled && mcpToolCount > 0)` → `chat.sendMessage(text, useMcp && mcpEnabled && mcpToolCount > 0)`
- `chat.isStreaming` → `chat.isLoading`（或在 hook 中加 alias）
- `chat.messages` 型別從 `AiChatMessage[]` 改為 `McpChatMessage[]`
- 保留 agentMode / agentRunning / submitAgent 邏輯不變（agent loop 與 MCP tool calling 是獨立功能）

---

## 5. Frontend: MessageList 工具呼叫卡片

**Files:** `src/components/AiPanel/MessageList.tsx`, `src/components/AiPanel/styles.css`

### 新訊息型別渲染

**`tool_call`（工具呼叫中）：**

```
┌─────────────────────────────────────────────┐
│ ⚙ brave_web_search              [loading…]   │
│ ▼ 輸入                                        │
│   { "query": "WWDC 2026 announcements" }     │
└─────────────────────────────────────────────┘
```

**`tool_result`（工具回傳後，合併到上面的卡片）：**

```
┌─────────────────────────────────────────────┐
│ ⚙ brave_web_search              ✓ 完成       │
│ ▼ 輸入                                        │
│   { "query": "WWDC 2026 announcements" }     │
│ ▼ 輸出                                        │
│   Apple announced...                         │
└─────────────────────────────────────────────┘
```

- 同一個 `tool_call_id` 的 call + result 合併顯示
- 預設收合（只顯示工具名 + 狀態）
- 點擊展開 input/output
- `is_error=true` → 卡片邊框紅色，狀態顯示 ✗

### 工具名稱顯示

encoded name 格式 `server_id__tool_name`，顯示時只取 `__` 後段：
```typescript
const displayName = name.includes("__") ? name.split("__").slice(1).join("__") : name;
```

---

## 6. 不需要改動的部分

| 模組 | 狀態 |
|------|------|
| Anthropic `generate_with_tools` | ✅ 已實作 |
| OpenAI `generate_with_tools` | ✅ 已實作 |
| `execute_mcp_tool` Tauri 命令 | ✅ 已有 |
| `ai_chat` 主要流程 | ✅ 只補 fallback 分支 |
| MCP toggle UI | ✅ 已有 |

---

## Error Handling

- 工具執行失敗（`is_error=true`）：顯示錯誤卡片，繼續 loop（把 error 內容作為 tool result 回傳 AI）
- 達到 10 次迭代上限：顯示警告訊息，停止 loop
- Ollama tool calling 失敗（HTTP error）：回傳 `AiError::Network`，前端顯示錯誤

## Testing

- `ollama.rs` 新增 unit test：mock HTTP response，驗證 tool_calls 正確 parse
- `useMcpChat` 已有 agent loop 邏輯，不需額外測試結構變更
- MessageList tool card：vitest + RTL 驗證 tool_call / tool_result 渲染
