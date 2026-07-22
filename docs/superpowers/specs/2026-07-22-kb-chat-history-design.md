# Knowledge Base 對話歷史記錄 Implementation Design

**Goal:** 讓知識庫（Knowledge Base）的問答對話持久化保存，同一筆記本下可有多組獨立對話記錄，使用者可切換、刪除、匯出成 md 檔案。

**Architecture:** 沿用專案既有的 session 持久化慣例（`design.rs`/`DesignView` 的 session 清單模式）與匯出慣例（`CodeAssistantView` 的 md 匯出邏輯），新增兩張表到既有的 `knowledge_base.db`，後端在串流完成時直接寫入，前端新增一個可調寬度的右側對話記錄側邊欄。

**Tech Stack:** Rust（sqlx SQLite）、Tauri command、React + TypeScript。

---

## 1. 需求範圍

- 同一筆記本下可有**多組獨立對話記錄**（非單一持續對話）。
- 對話記錄標題：自動取該對話**第一則使用者訊息**的前段文字，並顯示時間戳。
- 使用者可**刪除**對話記錄（沿用筆記本刪除的既有風格：純 × 按鈕，無確認彈窗）。
- 每組對話記錄可**匯出成 md 檔案**（沿用 `CodeAssistantView.handleExport` 的既有格式與存檔對話框）。
- 對話記錄清單顯示在**右側邊欄**（如設計討論中的截圖所示位置），且**寬度可拖曳調整**。

## 2. 資料儲存

新增兩張表到既有共用的 `knowledge_base.db`（沿用 `notebook_id` 外鍵慣例，與 `notebooks`/`documents`/`chunks` 同一個 DB 檔）：

```sql
CREATE TABLE IF NOT EXISTS kb_chat_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    notebook_id TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (notebook_id) REFERENCES notebooks(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS kb_chat_messages (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL, -- 'user' | 'assistant'
    content TEXT NOT NULL,
    tool_calls_json TEXT, -- JSON array of {tool, args, result}；user 訊息為 NULL
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (session_id) REFERENCES kb_chat_sessions(id) ON DELETE CASCADE
);
```

`tool_calls_json` 記錄該則回答用到的工具呼叫（含結果），用途：
1. 重新載入舊對話時，能還原來源引用（source citation chips），跟即時對話時的行為一致。
2. 匯出 md 時能附上工具呼叫細節（沿用 `CodeAssistantView` 匯出格式）。

Schema 建立集中放在 `KnowledgeBaseDb::init()`（`db/knowledge_base.rs`，與既有 notebooks/documents/chunks schema 同一處），CRUD 函式放在新檔案 `db/kb_chat_sessions.rs`（比照 `db/design.rs`、`db/loop_sessions.rs` 每個 session 體系各自一個檔案的既有慣例）：

- `create_chat_session(pool, notebook_id, title) -> id`
- `list_chat_sessions(pool, notebook_id) -> Vec<ChatSessionSummary>`（id, title, updated_at，依 updated_at DESC 排序）
- `load_chat_session_messages(pool, session_id) -> Vec<ChatMessageRow>`
- `delete_chat_session(pool, session_id)`（交易內先刪 messages 再刪 session，比照 `delete_notebook` 的非原子刪除修復模式）
- `create_chat_message(pool, session_id, role, content, tool_calls_json: Option<&str>)`（同時更新 session 的 `updated_at`）

## 3. 後端流程

比照 `design_chat` 既有模式：**後端在串流完成當下直接寫入 DB**，不透過前端額外呼叫 save 指令。

`run_chat`（`knowledge_base/chat.rs`）新增參數 `chat_session_id: String`。在迴圈最上層新增 `current_round_text: String`（每輪重置，累積該輪 `TextDelta`），並在函式頂層新增 `persisted_tool_calls: Vec<PersistedToolCall>`（跨輪累積，於每個 `ToolResult` 發送點同步 push；重複呼叫被跳過的情況不記錄，避免雜訊）。

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedToolCall {
    tool: String,
    args: serde_json::Value,
    result: String,
}
```

在原本觸發 `KbChatEvent::Done` 之前（run_chat 唯一的成功結束點——無論是 XML 工具呼叫收斂、原生 tool-calling 收斂、或 force_answer 逼答，都會流經同一個 `Text(text)` 且 `xml_calls` 為空的分支），呼叫新的持久化函式：

```rust
let _ = save_chat_turn(&pool, &chat_session_id, &last_user_text, &current_round_text, &persisted_tool_calls).await;
```

`run_fallback` 比照處理：新增 `chat_session_id` 參數，累積 `answer_buf`，在既有的 `Done` 事件前寫入 user + assistant 兩則訊息。因為 `run_fallback` 現有邏輯本來就不會發送 `ToolCall`/`ToolResult` 事件（fallback 模式沒有來源引用 UI），持久化時 `tool_calls_json` 存 `None`，維持與現有行為一致，不額外新增 fallback 的來源引用能力（不在本次範圍內）。

`last_user_text` 的取得方式與 `run_fallback` 現有的 `messages.iter().rev().find(...)` 邏輯相同，`run_chat` 也在函式頂層做一次同樣的擷取。

## 4. Tauri 指令

新增至 `commands/knowledge_base.rs`：

- `kb_create_chat_session(notebook_id: String, title: String) -> Result<String, AiError>`
- `kb_list_chat_sessions(notebook_id: String) -> Result<Vec<ChatSessionSummary>, AiError>`
- `kb_load_chat_session(session_id: String) -> Result<Vec<ChatMessageRow>, AiError>`
- `kb_delete_chat_session(session_id: String) -> Result<(), AiError>`

`kb_chat` 簽章新增必填參數 `chat_session_id: String`（原本用來做事件路由的 `session_id` 保持不變、語意不變，兩者是不同概念，不可混用）。

## 5. 前端行為

### 5.1 對話建立時機（延遲建立）

- 使用者點側邊欄「+ 新對話」：只清空目前畫面（`messages = []`）、`activeChatSessionId` 設為 `null`。**不會**立刻呼叫後端建立 session。
- 使用者送出第一則訊息時，若 `activeChatSessionId === null`，`send()` 內先呼叫 `kb_create_chat_session(notebookId, title)`（`title` = 該訊息前段文字，例如取前 30 字），拿到 id 存入 `activeChatSessionId`，接著才呼叫 `kb_chat(..., chatSessionId)`。
- 同一組對話後續的每一次 `send()` 都重複使用同一個 `activeChatSessionId`。

### 5.2 筆記本切換

切換筆記本時（既有的 `useEffect(() => {...}, [notebookId])`）：清空 `messages`、`activeChatSessionId` 設回 `null`、重新呼叫 `kb_list_chat_sessions(newNotebookId)` 刷新側邊欄清單。**不自動接續**上次對話（與 DesignView 的「自動載入未結束 session」不同——每組 KB 對話記錄是獨立、離散的項目，由使用者自行從清單點選）。

### 5.3 右側對話記錄側邊欄

新增元件（放在 `KnowledgeBaseView/` 內），比照 `DesignView` 的 `design-resizer` 拖曳邏輯（`isResizing` state + `mousemove`/`mouseup` 監聽 + `Math.max/min` 限制寬度），但方向相反（掛在容器右側，寬度 = `containerRect.right - e.clientX`），寬度限制在 220–480px 之間，並用 `localStorage` 記住使用者調整過的寬度（比照 `CodeAssistantView` 的 `loadSavedRoot`/`saveRoot` 存取模式）。

側邊欄內容：
- 頂部「+ 新對話」按鈕
- 對話記錄清單（依 `updated_at` DESC 排序），每筆顯示：標題（單行截斷）、時間戳、刪除（×）與匯出（↓）兩個小按鈕
- 清單為空時顯示提示文字（比照既有 `kb_select_notebook_hint` 等空狀態文案風格）
- 點擊清單項目（非按鈕區域）：呼叫 `kb_load_chat_session(id)` 取回訊息，還原成 `KbMessage[]`（`tool_calls_json` 還原成 `toolCalls`），設定 `activeChatSessionId = id`

### 5.4 刪除

點擊 × 呼叫 `kb_delete_chat_session(id)`，成功後重新整理清單；若刪除的正是目前開啟中的對話，畫面清空並回到 `activeChatSessionId = null` 的新對話狀態。無確認彈窗（沿用筆記本刪除的既有風格）。

### 5.5 匯出

沿用 `CodeAssistantView.handleExport` 既有寫法：`@tauri-apps/plugin-dialog` 的 `save()` 選存檔位置（預設檔名用對話標題）、組出 md 內容（標題、筆記本名稱、逐則問答、工具呼叫列表）、`writeTextFile` 寫入。可作用於目前開啟中的對話，或側邊欄清單項目上的匯出按鈕（後者需先 `kb_load_chat_session` 取得該對話完整內容才能匯出，若尚未是目前開啟中的對話）。

## 6. 型別異動摘要

- `useKnowledgeBaseChat.ts`：新增 `activeChatSessionId` state；`send()` 內新增延遲建立 session 的邏輯；`clear()` 語意調整為「新對話」按鈕的行為。
- `ipc/knowledgeBase.ts`：新增 `ChatSessionSummary`、`ChatMessageRow` 型別與對應的 4 個新 invoke 包裝函式；`invokeKbChat` 新增 `chatSessionId` 參數。
- `KnowledgeBaseView/index.tsx`：新增右側對話記錄側邊欄元件與拖曳調整寬度邏輯。
