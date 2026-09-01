# Artifact 協定推廣（教模型 + 接上其餘聊天介面）Design Spec

## 背景

`2026-09-01-artifact-panel-design.md` 的第一個里程碑已經完成並合併（merge commit
`3cbd740`）：AI 只要輸出 ` ```artifact-html ` 或 ` ```artifact-chart ` fenced block，
`src/lib/markdown.tsx` 就會渲染成 `ArtifactBlockCard`、登記進 `ArtifactPanelContext`，
由側邊的 `ArtifactPanel` 顯示。

但那個里程碑刻意留了兩個缺口，這份 spec 就是補它們：

1. **模型不知道這個協定存在**——系統提示裡完全沒提，所以 AI 永遠不會主動輸出這種
   區塊，目前只能靠使用者手動貼。
2. **除了 AiPanel 之外的聊天介面都還沒接上**——使用者最初提出這整個需求時問的就是
   「在資料庫中要求顯示圖表時要如何顯示」，那正是 `DatabaseAiChat`；使用者的總體
   目標是「賦予 AITerm 在任何一個功能中要顯示文件或圖表時都有能力處理」。

## 現況調查

1. **後端 `build_chat_prompt`（`src-tauri/src/commands/ai.rs:160`）是共用的聊天系統
   提示**，經由 `run_chat`（`ai.rs:381`）與 MCP 分支（`ai.rs:461`）餵給 `ai_chat`
   （`ai.rs:416`）與 `ai_chat_ctx`（`ai.rs:567`）兩個 command。
2. **`ai_chat` 的呼叫端不只聊天介面**。實際盤點（`grep aiChat(|invokeAiChatCtx(`）：
   - 會渲染聊天 markdown、且已經或即將支援 artifact 的：`useMcpChat.ts:133`
     （AiPanel）、`useRemoteAiChat.ts:118` + `RemoteAiPanel.tsx:137`（遠端面板）、
     `DatabaseAiChat.tsx:295,351`、`CrossDbAiChat.tsx:270,303,382`。
   - **完全不渲染聊天 markdown 的一次性用途**：`ApiDocsView.tsx:250`、
     `DocConverterView.tsx:228`（已確認兩者都沒有 import `MarkdownText`）。教這兩個
     artifact 協定會讓 fence 直接變成它們產出文件裡的垃圾文字。
   → **這是「必須用旗標控管、不能無條件教」的決定性理由。**
3. **`RemoteAiPanel` 已經自動支援 artifact**：它跟 `AiPanel` 一樣使用
   `ChatPanelShell`（`RemoteAiPanel.tsx`），所以上個里程碑把 provider 包進
   `ChatPanelShell` 時，它就一起獲得了這個能力，這次不用另外處理。
4. **`DatabaseAiChat`/`CrossDbAiChat` 會自己注入一份前端組的系統提示**
   （`DatabaseAiChat.tsx:295` 的 `buildSystemPrompt()` → i18n 的
   `t.db_ai_system_prompt`；`CrossDbAiChat.tsx:270` → `t.cdb_ai_system_prompt`），
   疊在後端 `build_chat_prompt` 之上，最終請求會有兩則連續的 system 訊息。
   → 代表「教模型」有兩個可能的下手處，選後端那個可以只改一處、不用動 4 個 i18n
   函式（2 個介面 × 2 個語系）。
5. **`build_chat_prompt` 目前沒有任何「你可以渲染什麼」的段落**，只有 `<cmd>` 標籤
   規則。現成的先例在 `src-tauri/src/commands/design.rs` 的 `build_design_prompt`
   （`design.rs:140-235`），它明確教模型 mermaid 的渲染與語法注意事項——寫法照它。
6. **`DatabaseAiChat` 的版面**（`DatabaseAiChat.tsx:447`）是 flex row：可收合的
   240px 歷史抽屜（純布林切換，沒有 resizer）+ `flex:1` 的聊天欄，佔滿
   `DatabaseView` 的「ai」子分頁（`DatabaseView/index.tsx:150-152`）。沒有既有的
   分割或 resizer。`CrossDbAiChat` 結構平行但不是逐字複製（658 vs 756 行，helper
   命名幾乎全不同）。
7. **`CodeAssistantView` 與 `KnowledgeBaseView` 各自走獨立的後端指令與系統提示**，
   不經過 `ai_chat`：
   - `CodeAssistantView`（`ca-view` 根容器，`index.tsx:257`）→ `useCodeAssistant`
     → `invokeCodeAssistantChat`（`useCodeAssistant.ts:155`）→ 後端
     `src-tauri/src/code_assistant/mod.rs` 的 `build_system_prompt`（`mod.rs:146`）。
   - `KnowledgeBaseView`（`kb-main` 根容器，`index.tsx:261`，已有自己的歷史欄
     resizer `kb-chat-history-resizer`，`index.tsx:426`）→ `useKnowledgeBaseChat`
     → `invokeKbChat`（`useKnowledgeBaseChat.ts:224`）→ 後端
     `src-tauri/src/knowledge_base/chat.rs` 的 `build_system_prompt`（`chat.rs:94`）。
   → **這兩個 prompt builder 各自只有一個消費端**，所以那兩處的教學可以無條件加，
   不需要 `build_chat_prompt` 那種旗標。
8. **`ChatPanelShell` 的分割版型目前是寫在元件內的**（provider、resizer、兩欄
   佈局）。這段邏輯有一個很不直覺的坑：resizer 必須用 `setPointerCapture`，因為
   右欄是 iframe，游標一進去 `mousemove` 就變成 iframe 自己文件的事件、父視窗收不
   到（上個里程碑實機才發現，見 commit `544d935`）。

## 設計

### A. 教模型：一段共用文字，接在三個 prompt builder 上

協定說明文字抽成一個共用的 Rust 函式（例如
`src-tauri/src/ai/artifact_prompt.rs` 的 `artifact_protocol_section(locale)`），由下面三個
prompt builder 引用，避免同一份說明散在各處各自漂移：

| Prompt builder | 觸及的介面 | 怎麼加 |
|---|---|---|
| `commands/ai.rs` 的 `build_chat_prompt` | AiPanel、遠端面板、兩個資料庫聊天、**以及兩個非聊天的一次性用途** | **要旗標**（見下） |
| `code_assistant/mod.rs` 的 `build_system_prompt` | 只有 `CodeAssistantView` | 無條件加 |
| `knowledge_base/chat.rs` 的 `build_system_prompt` | 只有 `KnowledgeBaseView` | 無條件加 |

#### A1. `build_chat_prompt` 要用旗標控管

`ai_chat` 與 `ai_chat_ctx` 新增參數 `supports_artifacts: bool`（預設 `false`，不傳的
呼叫端行為完全不變），往下傳給 `build_chat_prompt(snapshot, locale, supports_artifacts)`。
為 `true` 時在提示尾端附加一段說明；為 `false` 時完全不附加。

**傳 `true` 的呼叫端**（都會渲染聊天 markdown、且都有 `ArtifactPanelProvider`）：
`useMcpChat.ts`、`useRemoteAiChat.ts`、`DatabaseAiChat.tsx` 的兩個呼叫、
`CrossDbAiChat.tsx` 的三個呼叫。

**維持 `false`**：`ApiDocsView.tsx`、`DocConverterView.tsx`。

> 為什麼要旗標而不是無條件加：見「現況調查」第 2 點。「誰能顯示」與「誰被教」必須
> 永遠一致，否則模型會輸出一個那個介面根本渲染不出來的東西。

附加的內容大意（實際文字在實作計畫裡定稿）：

- 何時該用：內容本身是一份值得獨立閱讀的文件（報告、摘要、表格化的整理），或是一組
  適合看圖的數據。短答案、一兩句話、單一指令**不要**用。
- `artifact-html`：fence 內是完整 HTML，會在獨立的沙盒 iframe 裡渲染（可以用
  `<style>` 和 `<script>`，但拿不到外部環境）。第一行建議放 `<title>` 當標題。
- `artifact-chart`：fence 內是 JSON，schema 為
  `{"type":"bar"|"line"|"pie","title":string,"data":object[],"xKey":string,"series":[{"key":string,"label":string}]}`。
- 一則訊息最多一個 artifact（現行 context 只保留最後一個）。

### B. 接上其餘四個介面：先抽共用元件

**B1. 從 `ChatPanelShell` 抽出 `<ArtifactSplit>`**
（新檔 `src/components/ArtifactPanel/ArtifactSplit.tsx`）：接收 `children`（聊天欄
內容），內部負責「有 artifact 時裂成兩欄 + resizer + `<ArtifactPanel />`」。
`ChatPanelShell` 改用它，行為完全不變（既有測試就是驗收標準）。

不直接把那段邏輯複製到各個介面，理由是現況調查第 8 點：那段拖拉邏輯含有一個只有
實機才會發現的 iframe/pointer-capture 陷阱，複製五份幾乎保證後四份會重蹈覆轍。

**B2. 四個介面各自包 provider、套 `ArtifactSplit`**：
`DatabaseAiChat`、`CrossDbAiChat`、`CodeAssistantView`、`KnowledgeBaseView`，
做法一致——用 `ArtifactPanelProvider` 包住自己的 return，把原本的聊天欄內容交給
`<ArtifactSplit>`。`KnowledgeBaseView` 已經有自己的歷史欄 resizer，新的 artifact
分割是它右側的另一條，兩者互不干擾。

> per-tab 隔離：provider 包在各自元件內部（不是全域），跟 `ChatPanelShell` 同一個
> 道理——不同連線/分頁的 artifact 不該互相看見。

## 明確排除（Non-goals）

- **`DesignView`**：使用者明確決定不接。它已經有自己的右側 Markdown 預覽面板
  （`SpecPreview` 的四個分頁），再疊一個 artifact 面板是重複而非加值。
- **不改 4 個 i18n 系統提示函式**：教學統一放後端 `build_chat_prompt`，見現況調查
  第 4 點。
- **不做多 artifact 並存/切換**：context 仍然只保留最後一個。
- **不改 `ApiDocsView`/`DocConverterView` 的行為**（它們維持 `supports_artifacts`
  預設的 `false`）。

## 對既有程式碼的影響

- `src-tauri/src/commands/ai.rs`：`build_chat_prompt` 新增第三個參數與條件式段落；
  `ai_chat`、`ai_chat_ctx` 新增 `supports_artifacts` 參數並往下傳。
- `src/ipc/ai.ts`：`aiChat`、`invokeAiChatCtx` 包裝函式新增對應的選用參數。
- `src/hooks/useMcpChat.ts`、`src/hooks/useRemoteAiChat.ts`：呼叫時傳 `true`。
- 新增 `src/components/ArtifactPanel/ArtifactSplit.tsx` 與
  `ArtifactSplit.css`：分割相關的四條規則（`--split`、`chat-column`、
  `artifact-resizer`、`--resizing`）從 `ChatPanel/styles.css` **搬過來**，
  不留副本，class 名沿用不改（既有測試用它們斷言）。
- `src/components/ChatPanel/ChatPanelShell.tsx`：改用 `ArtifactSplit`（純重構）。
- `src/components/DatabaseView/DatabaseAiChat.tsx`、
  `src/components/CrossDbView/CrossDbAiChat.tsx`：包 provider、套 `ArtifactSplit`、
  呼叫時傳 `true`。
- `src/components/CodeAssistantView/index.tsx`、
  `src/components/KnowledgeBaseView/index.tsx`：包 provider、套 `ArtifactSplit`
  （它們的後端提示無條件教，前端不需要傳旗標）。
- 新增 `src-tauri/src/ai/artifact_prompt.rs`（共用的協定說明文字）。
- `src-tauri/src/code_assistant/mod.rs`、`src-tauri/src/knowledge_base/chat.rs`：
  各自的 `build_system_prompt` 引用共用文字。
- 不動：`DesignView`、`SpecPreview`、`ApiDocsView`、`DocConverterView`、i18n。

## 測試策略

### Rust 單元測試（`src-tauri`）

1. `build_chat_prompt` 在 `supports_artifacts=true` 時包含 `artifact-html` 與
   `artifact-chart` 兩個關鍵字；為 `false` 時兩者都不出現。
2. 為 `false` 時的輸出與這次改動之前逐字相同（用既有的幾個 `chat_prompt_*` 測試
   守住，補參數後不該有任何斷言需要修改）。

### 前端單元測試

3. `ArtifactSplit`：沒有 artifact 時只渲染 children、不出現 resizer；有 artifact 時
   出現 `.aiterm-artifact-panel` 與 resizer。
4. `ChatPanelShell` 既有的三個 artifact 版型測試必須原封不動繼續通過——這就是 B1
   重構「行為不變」的驗收標準。
5. 四個新接的介面各一組：訊息含 ` ```artifact-html ` 時渲染出 artifact 面板；沒有
   時不出現（`DatabaseAiChat`、`CrossDbAiChat`、`CodeAssistantView`、
   `KnowledgeBaseView`）。`KnowledgeBaseView` 另外確認新的 artifact 分割不會影響它
   既有的歷史欄 resizer。
6. `aiChat`/`invokeAiChatCtx` 的 IPC 包裝：確認 `supports_artifacts` 有正確帶進
   invoke 參數（沿用 `src/ipc/ai.test.ts` 既有的 invoke mock 寫法）。

### 需要真機驗證（無法自動化）

7. 在資料庫分頁實際問「用圖表顯示 xxx」，在知識庫分頁實際問「整理成一份報告」，
   確認模型真的會輸出 `artifact-chart`/`artifact-html`、
   圖畫得出來、且資料綁定正確——這取決於使用者所選 provider/model 的能力，程式碼
   只負責把協定教給它並把面板準備好。
8. 確認 `ApiDocsView`/`DocConverterView` 的產出裡不會冒出 artifact fence。
