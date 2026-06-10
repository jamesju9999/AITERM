# MCP Client 支援設計文件

**日期：** 2026-06-10  
**目標：** 讓 AITerm 作為 MCP Client，連接外部 MCP Servers，讓 AI 在所有對話情境中可以呼叫 MCP 工具。

---

## 1. 整體架構

MCP 支援分為 5 個子系統，可逐步交付：

```
設定 UI (React)
  └─ MCP Servers 設定頁 + Claude Desktop import + 全域開關
       ↓ IPC
Rust src-tauri/src/mcp/ 模組
  ├─ McpManager — 管理 server 連線 & 工具探索
  ├─ stdio transport — spawn subprocess, JSON-RPC over stdin/stdout
  └─ http/sse transport — HTTP 請求到遠端 server
       ↓
AI provider 整合 (src-tauri/src/ai/router.rs)
  └─ ai_chat 呼叫時附帶 tool definitions → 解析 AI 回傳的 tool_calls
       ↓ IPC
Frontend 工具呼叫協調 (src/hooks/useMcpChat.ts)
  └─ 偵測 tool_call → execute_mcp_tool → 送回結果 → 繼續對話
       ↓
各 AI Chat UI 元件
  └─ 每個 chat 的 MCP toggle button（per-chat 開關）
```

---

## 2. Config 儲存

在現有 `AppConfig` 新增兩個欄位：

```rust
pub struct AppConfig {
    // ... 現有欄位不變 ...
    pub mcp_enabled: bool,                    // 全域 MCP 開關，預設 true
    pub mcp_servers: Vec<McpServerConfig>,    // MCP server 清單
}

pub struct McpServerConfig {
    pub id: String,                           // 唯一識別符
    pub name: String,                         // 顯示名稱
    pub enabled: bool,                        // 是否啟用
    pub transport: McpTransport,              // Stdio | Http | Sse
    // stdio 專用
    pub command: Option<String>,              // 如 "npx"
    pub args: Vec<String>,                    // 如 ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
    pub env: HashMap<String, String>,         // 額外環境變數
    // http/sse 專用
    pub url: Option<String>,                  // 如 "http://localhost:3000/sse"
}

pub enum McpTransport {
    Stdio,
    Http,
    Sse,
}
```

儲存位置與現有 config 相同（`config.toml`）。

---

## 3. Rust MCP 模組

新增 `src-tauri/src/mcp/` 模組：

```
src-tauri/src/mcp/
  mod.rs        — McpManager 公開介面
  transport.rs  — stdio / http / sse transport 實作
  protocol.rs   — JSON-RPC 2.0 訊息結構、MCP initialize/tools/list/call
  types.rs      — McpTool, McpToolResult, McpServerStatus 等型別
```

### McpManager

```rust
pub struct McpManager {
    connections: HashMap<String, McpConnection>,  // server_id → 連線
}

impl McpManager {
    pub async fn connect_all(&mut self, servers: &[McpServerConfig]);
    pub async fn get_all_tools(&self) -> Vec<McpToolInfo>;
    pub async fn call_tool(&self, server_id: &str, tool_name: &str, args: Value) -> Result<McpToolResult, McpError>;
    pub async fn get_server_status(&self, server_id: &str) -> McpServerStatus;
    pub async fn reconnect(&mut self, server_id: &str);
}

pub struct McpToolInfo {
    pub server_id: String,
    pub server_name: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,    // JSON Schema，供 AI provider 用
}

pub enum McpServerStatus {
    Connecting,
    Connected { tool_count: usize },
    Error(String),
    Disabled,
}
```

### MCP Protocol（手寫 JSON-RPC，不加新 crate）

只需支援三個 method：

```
initialize      → 取得 server 能力（protocolVersion, capabilities）
tools/list      → 取得工具清單（name, description, inputSchema）
tools/call      → 執行工具（name, arguments → content[]）
```

### Transport

- **stdio**：用 `tokio::process::Command` spawn subprocess，以 `BufReader`/`BufWriter` 做換行分隔的 JSON-RPC 通訊
- **http/sse**：用現有 `reqwest`，POST 到 `{url}/messages`，GET SSE 從 `{url}/sse` 接收事件

### 新增 Tauri Commands（`src-tauri/src/commands/mcp.rs`）

```rust
list_mcp_servers()     → Vec<McpServerInfo>           // 含連線狀態
add_mcp_server(input)  → Result<(), String>
update_mcp_server(input) → Result<(), String>
remove_mcp_server(id)  → Result<(), String>
get_mcp_tools()        → Vec<McpToolInfo>             // 所有已連線 server 的工具
execute_mcp_tool(server_id, tool_name, args) → Result<McpToolResult, String>
import_claude_desktop_mcp() → Result<Vec<McpServerConfig>, String>  // 讀取並回傳候選清單，不自動寫入
set_mcp_enabled(enabled: bool) → Result<(), String>   // 全域開關
```

---

## 4. AI Provider 整合

### ai_chat 擴充

`src-tauri/src/commands/ai.rs` 的 `ai_chat` 新增：

1. 若 `mcp_enabled == true` 且請求中 `use_mcp == true`，呼叫 `McpManager::get_all_tools()`
2. 將工具清單轉換為各 provider 的 native 格式：
   - **OpenAI / GitHub Copilot / OpenAI-Compatible**：`tools: [{ type: "function", function: { name, description, parameters } }]`
   - **Anthropic**：`tools: [{ name, description, input_schema }]`
   - **Ollama**：嘗試 OpenAI 格式（部分模型支援），失敗時在 `AiChatReply` 中回傳 `tool_calling_unsupported: true`
   - **Google AI**：`tools: [{ functionDeclarations: [...] }]`
3. 解析 AI 回傳的 tool call 並在 `AiChatReply` 中回傳

### 擴充 AiChatReply

```rust
pub struct AiChatReply {
    pub content: Option<String>,                 // 最終文字回覆（有 tool_calls 時為 None）
    pub tool_calls: Vec<AiToolCall>,             // AI 要執行的工具（可能多個）
    pub tool_calling_unsupported: bool,          // provider 不支援時為 true
}

pub struct AiToolCall {
    pub id: String,           // tool call ID（provider 回傳，回傳結果時需附上）
    pub server_id: String,    // AITerm 標記（從 tool name prefix 解析）
    pub tool_name: String,
    pub args: Value,
}
```

Tool name 編碼方式：`{server_id}__{tool_name}`（雙底線分隔），在送給 AI 前編碼，收到 tool call 後解碼。

---

## 5. Frontend Tool Calling Loop

### 新增 `src/hooks/useMcpChat.ts`

包裝 `useAiChat`，加入 tool calling 迴圈：

```
useMcpChat(options: { useMcp: boolean })

loop:
  1. 呼叫 ai_chat(messages, session_id, { use_mcp: useMcp })
  2. 收到 tool_calling_unsupported → 顯示 warning，繼續（退化為無工具）
  3. 收到 tool_calls（可能多個）：
     a. 在對話中插入工具執行狀態訊息（顯示 loading）
     b. 逐一呼叫 execute_mcp_tool(server_id, tool_name, args)
     c. 更新工具執行狀態訊息（顯示結果/錯誤）
     d. 將工具結果加入 messages（role: "tool"）
  4. 若有 tool_calls → 回到步驟 1
  5. 若有 content → 顯示最終回覆，結束迴圈
  6. 最多執行 max_tool_iterations = 10 輪（防止無限迴圈）
```

### 工具執行 UI 區塊

對話中每次工具呼叫顯示為可收合的訊息 bubble：

```
⚙ read_file          [展開 ▾]
  › 參數：{ "path": "/Users/..." }
  › 結果：（預設收合，可展開）
  ✓ 完成  /  ✗ 失敗：錯誤訊息
```

### 各 AI Chat 元件改動

`AiPanel`、`DatabaseAiChat`、`CrossDbAiChat`、`DesignView` 改用 `useMcpChat` 取代直接呼叫 `useAiChat`，並新增 MCP toggle button。

---

## 6. MCP Toggle（開關）

### 全域開關

位於 Settings → MCP Servers 頁面頂部的總開關。關閉時：
- 所有 chat 的 MCP toggle button 隱藏
- `ai_chat` 不附帶工具定義

### Per-chat 開關

每個 AI chat UI 的 toolbar 加入 toggle button：

- MCP 全域開啟 + 有已連線 server → 按鈕可點擊，顯示 `⚙ MCP (N)` 其中 N 為工具總數
- MCP 全域開啟 + 無 server 或全部斷線 → 按鈕 disabled，tooltip 提示「請先在設定中新增 MCP Server」
- MCP 全域關閉 → 按鈕隱藏

Per-chat 開關狀態存 component local state（`useState`），預設開啟，不持久化。

---

## 7. 新增 IPC（`src/ipc/mcp.ts`）

```typescript
export interface McpServerInput {
  id?: string;
  name: string;
  enabled: boolean;
  transport: "stdio" | "http" | "sse";
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
}

export interface McpServerInfo extends McpServerInput {
  id: string;
  status: "connecting" | "connected" | "error" | "disabled";
  tool_count?: number;
  error_message?: string;
}

export interface McpToolInfo {
  server_id: string;
  server_name: string;
  name: string;
  description: string;
}

export async function listMcpServers(): Promise<McpServerInfo[]>
export async function addMcpServer(input: McpServerInput): Promise<void>
export async function updateMcpServer(input: McpServerInput): Promise<void>
export async function removeMcpServer(id: string): Promise<void>
export async function getMcpTools(): Promise<McpToolInfo[]>
export async function executeMcpTool(serverId: string, toolName: string, args: unknown): Promise<unknown>
export async function importClaudeDesktopMcp(): Promise<McpServerInput[]>
export async function setMcpEnabled(enabled: boolean): Promise<void>
```

---

## 8. 錯誤處理

| 情境 | 處理方式 |
|------|---------|
| MCP server 連線失敗 | 狀態顯示為 Error，其他 server 不受影響 |
| 工具執行失敗 | tool 結果訊息顯示錯誤，AI 收到 error content，繼續對話 |
| Provider 不支援 tool calling | 顯示 warning banner，退化為無工具對話 |
| Tool calling 迴圈超過 10 輪 | 停止並顯示警告 |
| Claude Desktop config 不存在 | Import 按鈕顯示「找不到 Claude Desktop 設定」 |

---

## 9. 檔案異動清單

**新增：**
- `src-tauri/src/mcp/mod.rs`
- `src-tauri/src/mcp/transport.rs`
- `src-tauri/src/mcp/protocol.rs`
- `src-tauri/src/mcp/types.rs`
- `src-tauri/src/commands/mcp.rs`
- `src/ipc/mcp.ts`
- `src/hooks/useMcpChat.ts`
- `src/components/Settings/McpServersPage.tsx`
- `src/components/Settings/McpServerForm.tsx`
- `src/components/Settings/McpServersPage.css`

**修改：**
- `src-tauri/src/config/types.rs` — 新增 `mcp_enabled`, `mcp_servers`
- `src-tauri/src/lib.rs` — 註冊 mcp module 和 commands
- `src-tauri/src/commands/ai.rs` — `ai_chat` 注入工具定義、解析 tool_calls
- `src-tauri/src/ai/mod.rs` — 擴充 `AiChatReply`、`ChatMessage` 新增 tool role
- `src-tauri/src/ai/router.rs` — 各 provider 的 tool calling 格式轉換
- `src/components/Settings/SettingsView.tsx` — 新增 MCP Servers 分頁
- `src/components/AiPanel/index.tsx` — 改用 useMcpChat + toggle button
- `src/components/DatabaseView/DatabaseAiChat.tsx` — 同上
- `src/components/CrossDbView/CrossDbAiChat.tsx` — 同上
- `src/components/DesignView/DesignView.tsx` — 同上
- `src/lib/i18n.ts` — 新增 MCP 相關 i18n 字串
