# MCP Marketplace Browser 設計文件

## 目標

在 MCP 設定頁面中新增「市集」Tab，讓使用者可以搜尋 Smithery MCP Registry 上的 server，並一鍵安裝（自動執行 npx/pip + 新增設定）。

## 架構

```
Smithery Registry API (registry.smithery.ai)
        ↓ fetch (前端 JS，直接呼叫)
  McpMarketplaceTab.tsx   ← McpServersPage 第二個 Tab
        ↓ 點「安裝」
  install_mcp_package()   ← 新 Tauri 命令（Rust）
        ↓ tokio::process::Command
  mcp-install-log events  ← 逐行 stream 到前端 terminal 面板
        ↓ 安裝成功
  add_mcp_server()        ← 現有 IPC，自動新增設定
        ↓
  切回「已安裝」Tab
```

---

## Section 1：前端元件

### McpServersPage.tsx（修改）

頂部新增兩個 Tab 切換：

| Tab | 內容 |
|---|---|
| 已安裝 | 現有 server 清單（不改動） |
| 🌐 市集 | `<McpMarketplaceTab />` |

新增 `activeTab: "installed" | "marketplace"` state，預設為 `"installed"`。

安裝成功後自動切回 `"installed"`。

### McpMarketplaceTab.tsx（新檔）

**State：**
```typescript
query: string                    // 搜尋關鍵字
results: SmitheryServer[]        // 搜尋結果
isSearching: boolean             // 搜尋中 loading
installing: string | null        // 正在安裝的 qualifiedName
installLog: string[]             // terminal 面板的 log 行
installStatus: "idle" | "running" | "success" | "error"
```

**行為：**
- 搜尋輸入框，輸入後 debounce 500ms 呼叫 Smithery API
- 每筆結果顯示：名稱、描述、transport 類型 badge、「安裝」按鈕
- 若 server 無 `connections` 資料，顯示「複製指令」按鈕，不顯示「安裝」
- 點「安裝」→ 底部滑出 `McpInstallTerminal`，按鈕改為 `⟳ 安裝中...`
- 安裝成功 → 按鈕改為 `✓ 已安裝`，3 秒後切回已安裝 Tab
- 安裝失敗 → 按鈕改為 `✗ 失敗，重試`

### McpInstallTerminal.tsx（新檔）

底部可收合的 terminal 面板：

- 高度 160px，從底部 slide-in 動畫進入
- 顯示執行指令（灰色）+ 逐行 log 輸出（綠色/紅色）
- 自動 scroll 到最新一行
- 右上角有 `×` 關閉按鈕
- 成功：最後一行顯示 `✓ 安裝完成，已新增到已安裝清單`（綠色）
- 失敗：最後一行顯示錯誤訊息（紅色）+ 「重試」按鈕
- 失敗時不自動關閉

---

## Section 2：Smithery API 整合

### API 端點

**搜尋（前端 JS fetch 直接呼叫）：**
```
GET https://registry.smithery.ai/servers?q=<query>&pageSize=20

Response:
{
  "servers": [
    {
      "qualifiedName": "@modelcontextprotocol/server-filesystem",
      "displayName": "Filesystem",
      "description": "讀寫本地檔案系統",
      "homepage": "https://..."
    }
  ],
  "pagination": { "currentPage": 1, "pageSize": 20, "totalCount": 150 }
}
```

**取得 server 詳細資料（點安裝時呼叫）：**
```
GET https://registry.smithery.ai/servers/<qualifiedName>

Response:
{
  "qualifiedName": "...",
  "displayName": "...",
  "description": "...",
  "connections": [
    {
      "type": "stdio",
      "stdioFunction": {
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem"],
        "env": {}
      }
    }
  ]
}
```

### TypeScript 型別（新增至 src/ipc/mcp.ts 或 src/lib/smithery.ts）

```typescript
export interface SmitheryServer {
  qualifiedName: string;
  displayName: string;
  description: string;
  homepage?: string;
}

export interface SmitheryServerDetail {
  qualifiedName: string;
  displayName: string;
  description: string;
  connections: SmitheryConnection[];
}

export interface SmitheryConnection {
  type: "stdio" | "http" | "sse";
  stdioFunction?: {
    command: string;
    args: string[];
    env: Record<string, string>;
  };
  url?: string;
}

export async function searchSmithery(query: string): Promise<SmitheryServer[]>
export async function getSmitheryServer(qualifiedName: string): Promise<SmitheryServerDetail>
```

### Tauri CSP 設定

在 `src-tauri/tauri.conf.json` 的 `security.csp` 中新增：

```json
"connect-src": ["https://registry.smithery.ai"]
```

### 新 Tauri 命令：`install_mcp_package`

```rust
// src-tauri/src/commands/mcp.rs 新增
#[tauri::command]
pub async fn install_mcp_package(
    app: AppHandle,
    command: String,      // 例如 "npx"
    args: Vec<String>,    // 例如 ["-y", "@mcp/server-filesystem"]
    session_id: String,   // 用於識別 stream 事件
) -> Result<(), String>
```

**執行流程：**
1. 用 `tokio::process::Command` 執行 `command args`
2. stdout/stderr 逐行 emit `mcp-install-log` 事件：
   ```rust
   // 事件 payload
   { session_id: String, line: String, is_error: bool, done: bool }
   ```
3. 執行成功（exit code 0）→ emit `done: true, is_error: false`
4. 執行失敗 → emit `done: true, is_error: true`，並在 line 中包含錯誤訊息
5. 逾時（60 秒）→ kill process，emit 逾時錯誤

---

## Section 3：安裝 UX 細節與錯誤處理

### 按鈕狀態機

```
[安裝] → [⟳ 安裝中...（disabled）] → [✓ 已安裝（disabled）]
                                    ↘ [✗ 失敗，重試]
```

### 錯誤情境處理

| 情境 | 處理方式 |
|---|---|
| Node.js 未安裝（`npx` not found） | terminal 顯示「請先安裝 Node.js：https://nodejs.org」 |
| Python 未安裝（`pip` not found） | terminal 顯示「請先安裝 Python：https://python.org」 |
| Smithery API 網路錯誤 | 搜尋框下方顯示紅色提示「無法連線到市集，請檢查網路」|
| Smithery API 無 connections 資料 | 顯示「複製指令」按鈕，不顯示「安裝」 |
| 安裝逾時（> 60 秒）| terminal 顯示「安裝逾時，請手動執行指令」，終止 process |
| 安裝 exit code ≠ 0 | terminal 顯示完整 stderr，按鈕改為「重試」 |

### 安裝成功後的自動設定

從 Smithery `connections[0]` 自動建立 `McpServerInput`：

- **stdio**：`command = stdioFunction.command`，`args = stdioFunction.args.join("\n")`，`env = Object.entries(env).map(([k,v]) => \`${k}=${v}\`).join("\n")`
- **http/sse**：`url = connection.url`

呼叫現有 `add_mcp_server(input)` 新增，不需要使用者再填表單。

---

## 檔案異動清單

| 檔案 | 動作 |
|---|---|
| `src/components/Settings/McpServersPage.tsx` | 修改：加 Tab 切換 |
| `src/components/Settings/McpMarketplaceTab.tsx` | 新增 |
| `src/components/Settings/McpInstallTerminal.tsx` | 新增 |
| `src/lib/smithery.ts` | 新增：Smithery API fetch 函式 + 型別 |
| `src-tauri/src/commands/mcp.rs` | 修改：新增 `install_mcp_package` 命令 |
| `src-tauri/src/lib.rs` | 修改：註冊新命令 |
| `src-tauri/tauri.conf.json` | 修改：CSP 允許 registry.smithery.ai |

## 不在此次範圍內

- MCP server 的版本更新（update）
- 分頁載入更多結果（只做第一頁 20 筆）
- 使用者評分 / 排序功能
- 離線快取搜尋結果
