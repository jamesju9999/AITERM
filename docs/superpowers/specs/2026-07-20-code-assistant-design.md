# Code Assistant — 設計規格

**日期：** 2026-07-20  
**功能：** 程式庫協助（Code Assistant）Tab  
**狀態：** 待實作

---

## 1. 功能目標

讓使用者指定一個本機專案目錄後，可透過地端或雲端 AI 對程式庫提問，涵蓋：
- 理解架構與模組關係
- 解釋特定程式碼
- 找 bug / 診斷問題
- 實作建議 / How-to

支援多輪對話，每輪的 AI 可主動讀取專案檔案後再回答。

---

## 2. 使用者流程

1. 使用者從 Tab Bar 開啟「程式庫」Tab
2. 首次進入顯示目錄選擇器（OS 原生 folder picker）
3. 選定專案根目錄後，進入對話介面
4. 使用者輸入問題 → AI 透過工具迭代讀取檔案 → 回答
5. 支援後續追問（對話歷史保留，file context 累積）
6. 「清除」重置對話；「更換目錄」選完新目錄後詢問「繼續對話 / 開新對話」

---

## 3. 上下文策略：Tool-use 迭代式（方案 B）

AI 擁有三個工具，可在回答前自行決定讀取哪些檔案：

| Tool | 簽名 | 用途 |
|------|------|------|
| `list_directory` | `(path: string)` | 列出目錄內容（相對專案根） |
| `read_file` | `(path: string)` | 讀取單一檔案內容 |
| `search_in_files` | `(query: string, file_pattern?: string)` | Grep-style 跨檔搜尋 |

**不支援 tool-use 的 provider**：自動 fallback 至兩段式（Phase 1 送目錄樹讓 AI 選檔 → Phase 2 組合內容回答），並顯示提示訊息。

---

## 4. 安全限制

### 工具層級
| 限制 | 值 |
|------|----|
| `list_directory` 單次最大回傳 | 200 entries |
| `read_file` 單檔上限 | 100 KB（超過截斷，並在內容末尾標注） |
| `search_in_files` 最大匹配片段 | 50 筆 |

### 自動排除（不列出、不讀取）
```
node_modules/  .git/  target/  dist/  build/
__pycache__/   .DS_Store  *.lock  *.bin
常見圖片格式：*.png *.jpg *.jpeg *.gif *.ico *.svg（大型）
```
若專案根存在 `.gitignore`，額外套用其規則。

### 迴圈保護
- **最大 tool call 輪數**：20 輪 / 使用者訊息
- **累積 token 估算上限**：200,000 tokens（Settings 可調整，對應地端模型 262k 上限留緩衝）
- 達到上限時，system prompt 注入：「已達讀取上限，請根據現有資訊直接回答，不要再呼叫工具。」

---

## 5. 架構

### 後端（Rust）

```
src-tauri/src/
  code_assistant/
    mod.rs          ← tool executor loop + fallback 邏輯
    tools.rs        ← list_directory / read_file / search_in_files 實作
    tree.rs         ← .gitignore 解析 + 排除清單過濾

  commands/
    code_assistant.rs  ← Tauri command: code_assistant_chat
```

**主要 Tauri command：**
```rust
#[tauri::command]
async fn code_assistant_chat(
    project_root: String,
    messages: Vec<ChatMessage>,
    provider_id: Option<String>,
    locale: Locale,
    app: AppHandle,
) -> Result<AiChatReply, AiError>
```

**執行邏輯（`mod.rs`）：**
1. 建立 system prompt（含專案根路徑、語言指令、工具使用說明）
2. 呼叫 `generate_with_tools`（現有 trait 方法）
3. 收到 `ToolCalls` → 執行對應工具 → 將結果加入 messages → 重複
4. 收到 `Text` → 回傳最終答案
5. `Unsupported` → fallback 兩段式

### 前端（React / TypeScript）

```
src/
  components/CodeAssistantView/
    index.tsx         ← 主元件（目錄選擇 + 對話介面）
    ToolCallCard.tsx  ← 工具呼叫進度卡片（可展開看回傳內容）
    styles.css

  ipc/
    codeAssistant.ts  ← invoke wrapper: code_assistant_chat
```

**重用現有元件：**
- `AiPanel/MessageBubble.tsx`
- `AiPanel/MessageList.tsx`
- `StreamingIndicator.tsx`

---

## 6. UI 佈局

```
┌─────────────────────────────────────────────────────┐
│  [📁 /Users/james/myproject]  [更換目錄]  [重新整理]  │
├─────────────────────────────────────────────────────┤
│                                                     │
│  🤖 AI：我先看看專案結構...                           │
│  ⚙️ [工具] list_directory("/")          ▼ 展開      │
│  ⚙️ [工具] read_file("src/main.rs")    ▼ 展開      │
│  🤖 AI：這個專案使用 Tauri 2...                      │
│                                                     │
│  👤 useAiChat 為什麼有 race condition？              │
│  ⚙️ [工具] read_file("src/hooks/useAiChat.ts") ▼   │
│  🤖 AI：問題在 mountedRef...                        │
│                                                     │
├─────────────────────────────────────────────────────┤
│  [輸入框]                          [傳送]  [清除]    │
└─────────────────────────────────────────────────────┘
```

**互動行為：**
- `ToolCallCard`：預設折疊，顯示工具名稱 + 路徑；點擊展開看完整回傳內容
- `更換目錄`：觸發 Tauri `open` dialog（folder 模式）選定新目錄後，詢問使用者「要繼續現有對話，還是開新對話？」（inline 確認列，不用 modal）
- `清除`：清空 messages，保留目錄選擇與 provider 設定
- 模型不支援 tool-use 時：頂部顯示黃色提示 banner，自動 fallback

---

## 7. 事件協議（Streaming）

後端在執行過程中透過 Tauri 事件通知前端，事件名稱：`code-assistant-event`

```typescript
type CodeAssistantEvent =
  | { kind: "tool_call";  tool: string; args: Record<string, string>; call_id: string }
  | { kind: "tool_result"; call_id: string; content: string; truncated: boolean }
  | { kind: "text_delta"; delta: string }
  | { kind: "done" }
  | { kind: "error"; error: AiError }
```

**流程：**
1. AI 發出 tool call → 後端 emit `tool_call` → 前端顯示 ToolCallCard（loading 狀態）
2. 工具執行完畢 → 後端 emit `tool_result` → 前端更新 ToolCallCard（完成狀態，可展開內容）
3. AI 輸出最終文字 → 後端逐 token emit `text_delta` → 前端 streaming 顯示
4. 全部結束 → emit `done`

前端透過 `session_id`（每次呼叫 `code_assistant_chat` 前由前端生成的 UUID）區分事件歸屬，避免多 Tab 混流。

---

## 8. Fallback 兩段式流程（provider 不支援 tool-use 時）

1. **Phase 1**：`list_directory("/")` 取得一到兩層目錄樹 → 連同使用者問題送給 AI → AI 回傳想讀哪些檔案（純文字 JSON 列表）
2. **Phase 2**：後端批次讀取這些檔案 → 組合成一份 context 文件 → 再次送 AI 回答原始問題
3. 每輪問題都重新執行兩段式（不累積 file context）

---

## 8. 不在此次範圍內

- 向量嵌入 / RAG（可未來迭代）
- 專案設定持久化（記住上次的目錄）— 可下一個版本加入
- 程式碼編輯能力（只讀，不寫入）
- 跨 session 的對話歷史儲存
