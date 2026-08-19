# AITerm MCP Tool Server — 設計

日期：2026-08-19
狀態：待使用者複審

## 問題

使用者想讓 Claude Code CLI（或其他 MCP client）直接呼叫 AITerm 已經設定好的能力——目前鎖定 DB 連線查詢與知識庫語意搜尋——而不必在 Claude Code 那邊另外設定一個獨立的 DB MCP server、重複填一次連線資訊。

## 現況調查（設計的事實基礎）

以下都是實際讀過程式碼確認的，不是推測：

| 事實 | 位置 |
|---|---|
| AITerm 目前只有 **MCP client** 角色：`McpManager` 連到使用者設定的外部 MCP server（stdio/http），把工具轉給 AITerm 自己的 AI chat 用 | `src-tauri/src/mcp/mod.rs` |
| 現有 MCP client 的 `HttpTransport` 是單純「POST 一個 JSON-RPC 請求、等一個 JSON 回應」，不是完整的 Streamable HTTP（無 SSE、無 `Mcp-Session-Id`） | `src-tauri/src/mcp/transport.rs:135` |
| AITerm 已經有一個綁 `127.0.0.1` 的 axum server 先例（Claude Bridge，Anthropic Messages API 相容），含 server 生命週期管理（`BridgeState::start/stop`，oneshot shutdown）、bearer token 認證、設定頁開關 | `src-tauri/src/bridge/mod.rs`, `src-tauri/src/bridge/auth.rs`, `src/components/Settings/ClaudeBridgePage.tsx` |
| `DbManager` 已有乾淨、非 session 綁定的操作：`is_connected` / `execute` / `list_tables` / `list_schemas` / `get_table_schema`，`execute` 在連線未建立時回 `Err("not_connected")`，不會自動連線 | `src-tauri/src/db/manager.rs` |
| `db_connect` command 的連線邏輯（讀 Keychain 密碼 → `build_adapter` → `manager.insert`）目前只在這個 Tauri command 裡，沒有抽成獨立函式 | `src-tauri/src/commands/db.rs:317` |
| 知識庫是多 notebook 結構，不是單一集合：`list_notebooks` 回傳多筆 | `src-tauri/src/db/knowledge_base.rs:160` |
| `knowledge_base/tools.rs` 已有 `tool_definitions()` / `dispatch_tool()`，但簽章是 `dispatch_tool(pool, notebook_id, embedder, name, args)`——`notebook_id` 與 `embedder` 來自呼叫端目前的 chat session，形狀對不上一個無狀態的外部 MCP 呼叫（每次呼叫要能指定任意 notebook） | `src-tauri/src/knowledge_base/tools.rs:8`, `:49` |
| `code_assistant/mod.rs` 也有自己一份 `tool_definitions()`/`dispatch_tool()`，簽章又不同（綁 `root_path`/`session_id`）——三個模組的「工具」形狀互不相容，沒有共用介面可以直接複用 | `src-tauri/src/code_assistant/mod.rs:69`, `:216` |
| `rmcp = "3.1.3"`（官方 MCP Rust SDK）在 crates.io 上可用，跟現有 `axum = "0.8"` / `tokio` 版本相容 | crates.io 查詢確認 |
| 知識庫搜尋依賴一個 `Embedder`，其實作取決於使用者是否設定了 embedding 供應商 | `src-tauri/src/knowledge_base/embedding.rs`, `chat.rs` 建構方式 |

**結論**：這是新增一個「AITerm 當 MCP server」的角色，跟現有的「AITerm 當 MCP client」（`mcp/`）與「AITerm 當 Anthropic API 相容上游」（`bridge/`）是三個獨立關注點，不應該共用開關/連接埠，但可以共用 server 生命週期管理與認證的既有模式。

## 範圍

**含：**

- 新模組 `src-tauri/src/mcp_server/`：用 `rmcp` crate 的 Streamable HTTP server transport，掛一個新的 axum Router，綁 `127.0.0.1:<可設定埠>`
- 獨立的啟用開關與連接埠（不跟 Claude Bridge 共用）
- DB 工具群組：`list_connections` / `list_schemas` / `list_tables` / `get_table_schema` / `execute_query`（任意 SQL，不限唯讀）
- 知識庫工具群組：`list_notebooks` / `search_documents` / `read_document`
- `execute_query` 呼叫時，若目標連線尚未連線，自動走一次等同 `db_connect` 的邏輯（讀 Keychain 密碼建 adapter）再執行
- bearer token 認證（獨立於 Claude Bridge 的 token），設定頁提供可複製的 `claude mcp add` 指令
- 查詢結果截斷（筆數/位元組上限），避免超大結果灌爆 context
- 新設定頁區塊（比照 `ClaudeBridgePage.tsx` 樣式）

**不含：**

- VCS/git 狀態、終端機指令區塊歷史等其他能力（這次先聚焦 DB + 知識庫；架構上用同一個 server 加新工具群組不難，但這次不做）
- 唯讀限制或執行前的核准 UI（使用者已明確選擇開放任意 SQL、風險自行承擔）
- 讓區網其他機器使用（只綁 loopback，跟 Claude Bridge 一致）
- stdio transport（只做 HTTP；AITerm 是常駐 GUI app，不適合被 Claude Code 當子行程 spawn）
- 修改 `knowledge_base/tools.rs` 或 `code_assistant/mod.rs` 既有的 dispatch 邏輯（新工具群組另外實作，不共用那兩個模組的 `dispatch_tool`）

## 架構

### 為什麼另開模組，且用 rmcp 而非手刻協定

最初評估過手刻最小 Streamable HTTP（跟 `bridge/server.rs` 手刻 SSE 的風格一致，不加新依賴），但使用者傾向導入官方 `rmcp` crate，原因是協定正確性更有保障，且已確認未來要繼續擴充知識庫等更多工具群組——手刻只解一次的協定子集，換來的是往後每加一種能力就要重新面對 Streamable HTTP 規格細節（`Mcp-Session-Id`、批次請求、SSE 升級條件等）的風險。採用 `rmcp` 換取的是一次性導入成本，換掉的是往後持續的協定維護風險。

### 模組佈局

```
src-tauri/src/mcp_server/
  mod.rs              McpServerState（server handle、port、token，比照 bridge::BridgeState 的 start/stop）
  server.rs           axum router，掛 rmcp 的 Streamable HTTP service
  db_tools.rs          DB 工具群組：tool 定義 + dispatch，直接操作 DbManager/ConfigStore/SecretStore/Db2SidecarState
  kb_tools.rs           知識庫工具群組：tool 定義 + dispatch，直接操作 SqlitePool + Embedder，接受任意 notebook_id
```

跟既有兩個相關模組的關係：

| 模組 | 角色 | 開關/埠 |
|---|---|---|
| `mcp/` | AITerm 當 MCP **client**，連外部 server，工具給 AITerm 自己的 AI chat 用 | 無獨立開關（隨每個 server 設定啟用） |
| `bridge/` | AITerm 當 Anthropic Messages API 相容**上游**，給 Claude Code CLI 選 AI 供應商用 | 獨立開關 + 埠（既有） |
| `mcp_server/`（本設計） | AITerm 當 MCP **server**，曝露 DB/知識庫能力給任意 MCP client | 獨立開關 + 埠（新增） |

### 工具清單

**DB 群組**（對應 `DbManager` 既有操作，新增一個「未連線時自動連線」的包裝）：

| 工具 | 參數 | 說明 |
|---|---|---|
| `list_connections` | 無 | 回傳 id/name/db_type/database（不含密碼） |
| `list_schemas` | `connection_id` | |
| `list_tables` | `connection_id`, `schema` | |
| `get_table_schema` | `connection_id`, `schema`, `table` | |
| `execute_query` | `connection_id`, `sql` | 任意 SQL；未連線時自動連線 |

**知識庫群組**（新實作，不沿用 `knowledge_base/tools.rs` 的 session 綁定簽章）：

| 工具 | 參數 | 說明 |
|---|---|---|
| `list_notebooks` | 無 | |
| `search_documents` | `notebook_id`, `query`, `top_k`（預設 8，上限 20） | 語意搜尋，回傳文字片段 + 來源路徑 + 相似度 |
| `read_document` | `notebook_id`, `path` | 讀取單一文件的完整轉換內容，超過大小上限時截斷 |

### 認證

比照 `bridge/auth.rs`：獨立 bearer token，存在 SecretStore 的新 key（例如 `MCP_TOOL_SERVER_TOKEN_KEY`），常數時間比對。設定頁顯示：

```
claude mcp add --transport http aiterm-tools http://127.0.0.1:<port>/mcp \
  --header "Authorization: Bearer <token>"
```

使用者複製貼到終端機執行一次即完成註冊（Claude Code CLI 會存進 `~/.claude.json` 或專案的 `.mcp.json`，之後自動連線，不需要每個 session 重新註冊）。

### 生命週期與設定 UI

- `McpServerState`：跟 `BridgeState` 同款 start/stop + oneshot shutdown 模式，`start()` 埠被占用時回錯誤（不自動換埠，理由同 Claude Bridge：埠若漂移，已註冊的 MCP client 設定會指向死位址）
- 新增 `config/types.rs` 的 `McpToolServerConfig { enabled: bool, port: u16 }`，`#[serde(default)]` 保舊設定檔相容
- 新增設定頁區塊（比照 `ClaudeBridgePage.tsx`）：Enable 開關、Port 欄位、Token 顯示 + 重新產生按鈕、複製 `claude mcp add` 指令按鈕、連線狀態顯示

### 錯誤處理

- DB 連線失敗（密碼錯誤、DB2 sidecar 未建置）→ 工具回應 `isError: true` + 人類可讀錯誤訊息，不讓 server 中斷其他呼叫
- 知識庫搜尋時若目前沒有可用的 embedding 供應商設定 → 同樣回 `isError: true`，不丟 500
- `execute_query` 與 `read_document` 的結果比照 `knowledge_base/tools.rs` 現有的 `safe_truncate` 模式做位元組上限截斷（UTF-8 安全邊界），截斷時在回應文字裡註明「已截斷」，避免超大結果吃光 Claude 的 context 視窗
- MCP 初始化握手（`initialize` / `notifications/initialized`）與工具 schema 驗證交給 `rmcp` 處理，不手刻

## 測試

- Rust 單元測試：DB 工具的自動連線包裝（未連線 → 自動連線成功執行；密碼錯誤 → 回 `isError: true` 而非 panic/500）；知識庫工具在多 notebook 情境下的 `notebook_id` 過濾正確性；截斷邏輯的 UTF-8 邊界安全性（沿用 `knowledge_base/tools.rs` 已有的 `safe_truncate` 測試模式）
- 用 `rmcp` 官方提供的 client（或現有 `src-tauri/src/mcp/` 的 client 邏輯）寫一支整合測試，實際打這個新 server 的 `/mcp` endpoint 跑一次 `initialize` → `tools/list` → `tools/call`，驗證協定往返正確，而不是只測內部函式
- 手動驗證：真的用 `claude mcp add --transport http` 註冊，在一個裝了 Claude Code CLI 的終端機分頁裡問一句需要查 DB/知識庫的問題，確認工具真的被呼叫且結果正確顯示
