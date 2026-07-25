# Title Bar AI 摘要（類似 Warp 的分頁摘要）

**日期**：2026-07-25
**狀態**：待審閱

## 背景與目標

使用者希望 App 自繪的 title bar（`src/components/TitleBar/`，目前永遠顯示固定文字「AITerm」）能改成顯示「目前分頁名稱 - AI 摘要」，仿照 Warp 在其分頁清單中，用 AI 自動把該工作階段的對話內容摘要成一句短標題的做法（例如「評估OAuth驗證方式於AITERM的可行性」）。

現況調查：
- `<TitleBar title={...} />` 已支援動態 `title` prop，但 `TerminalApp.tsx` 呼叫時完全沒傳，永遠落回預設值「AITerm」。
- `Tab` 介面（`src/components/TabBar/index.tsx`）已有 `title` 欄位；新分頁建立時，`title` 會被設成該分頁類型的本地化名稱（如「終端機」），使用者可透過既有的 `onRename` 機制自行改名。
- 每個終端機分頁的 `/ai` 對話狀態由 `useMcpChat(sessionId)` 管理（**不是** `useAiChat` — 該 hook 目前完全沒有任何呼叫端，是死碼，不要誤用），對話內容存在該 hook 回傳的 `messages: McpChatMessage[]` 裡，每個分頁透過各自的 `sessionId` 互相獨立。
- `AiPanel/index.tsx` 內部已經有 `agentRunning`（agent 自動模式執行狀態）與 `chat.isStreaming`（單次回應串流狀態）兩個狀態可用。
- `TerminalApp.tsx` 已有一個結構幾乎相同的既有模式可以直接照抄：`agentProgress` 欄位透過 `onAgentProgress` callback 從 `TerminalView` 往上回傳給 `TerminalApp`，再寫回 `tabs` 狀態（`TerminalApp.tsx:357-360`）。本次的 `aiSummary` 走同一種 prop-callback 鏈路，只是多穿一層（`AiPanel` → `TerminalView` → `TerminalApp`，因為摘要相關的 `agentRunning`/`isStreaming` 狀態實際存在 `AiPanel` 裡，比 `agentProgress` 的來源更深一層）。
- `invokeAiChat(messages, sessionId, providerId, useMcp, locale)`（`src/ipc/ai.ts`）是既有的一次性 AI 查詢 IPC 函式，重用它即可完成摘要生成，**不需要新增任何後端 Rust 程式碼**。

## 範圍界定（已透過視覺化 mockup 與使用者確認）

### 明確排除（Non-goals）

- 不同步 OS 原生視窗標題（macOS Dock / Cmd+Tab 預覽、Windows 工作列）——只更新 App 自繪的 `.aiterm-titlebar__title` 文字，不呼叫 Tauri 的 `window.setTitle()`。
- 不新增使用者手動編輯摘要的介面（如需與既有分頁 rename 整合，留待未來另外討論）。
- 不持久化摘要——`aiSummary` 只存在記憶體中的 `tabs` state，不寫進 `SESSION_TABS_KEY` 的 localStorage。App 重啟或分頁清單重新載入後，摘要消失，等下一次 AI 回應完成後才會重新生成。
- 只套用在 `type === "terminal"` 的分頁。其餘 9 種分頁類型（database、design、cross-db、vcs、doc-converter、api-docs、loop-studio、code-assistant、knowledge-base）維持現狀，title bar 只顯示分頁名稱本身。
- 不做固定的低成本模型 —— 摘要呼叫使用該分頁當前設定的預設 AI 供應商，跟正式回答同一顆模型。

## 設計

### 1. 觸發時機

摘要生成的**唯一觸發點**：`AiPanel` 內某次 AI 回應完全結束時（`!chat.isStreaming && !agentRunning` 同時成立的那個時間點），且 `chat.messages` 中至少有一則 assistant 回覆。

- **Agent 自動模式的多步驟不會逐步觸發**——`agentRunning` 在整個任務執行期間維持 `true`，只有任務全部跑完、回到 `false` 時才觸發一次摘要，避免一次 agent mission 產生多次額外的 LLM 呼叫。
- 用 `useEffect` 監看 `[chat.messages.length, chat.isStreaming, agentRunning]`，在依賴變化後判斷是否符合觸發條件（訊息數增加、且兩個旗標都是 false）再呼叫摘要函式，避免每次 render 都重複觸發。

### 2. 摘要生成方式

新增一個小函式（放在 `AiPanel/index.tsx` 內，或抽成 `src/lib/summarizeTab.ts` 均可，實作時依現有檔案風格決定），簽名大致為：

```ts
async function summarizeConversation(
  messages: McpChatMessage[],
  sessionId: string,
  providerId: string | undefined,
  locale: Locale,
): Promise<string | null>
```

行為：
- 從 `messages` 取**最後 10 則**訊息中的 user/assistant 內容（略過 `tool_call`/`tool_result` 雜訊），組成純文字。這個數字純粹是「讓摘要有足夠上下文、又不會把一整個長對話都送去摘要」的務實取捨，不是精確調校過的值——實作時如果發現摘要品質不理想，調整這個數字不影響其他設計決策。
- 呼叫 `invokeAiChat([{ role: "user", content: <摘要指令 + 對話內容> }], `${sessionId}-summary`, providerId, false, locale)`——`useMcp` 固定傳 `false`（摘要不需要工具存取），`sessionId` 用獨立字串（`${原sessionId}-summary`）避免跟主聊天畫面共用同一個 `ai-stream` 事件通道互相干擾。
- 摘要指令依語系而定，中文版例如：「請根據以下對話，用不超過 20 個字生成一句精簡的中文摘要，描述使用者目前在做什麼。不要標點符號結尾、不要加引號、只輸出摘要本身。」英文版同理，字數上限可放寬到 40 字元。
- 回傳 `reply.content?.trim() || null`。呼叫過程中任何例外都在函式內 catch 並回傳 `null`（見「錯誤處理」）。

### 3. 資料流與元件改動

比照既有 `agentProgress`/`onAgentProgress` 模式：

- **`Tab` 介面**（`src/components/TabBar/index.tsx`）新增 `aiSummary?: string`。
- **`AiPanelProps`**（`src/components/AiPanel/index.tsx`）新增 `onSummaryUpdate?: (summary: string) => void`。在符合觸發條件的 `useEffect` 中呼叫 `summarizeConversation(...)`，成功時呼叫 `onSummaryUpdate?.(summary)`。
- **`TerminalViewProps`**（`src/components/TerminalView.tsx`）新增 `onSummaryUpdate?: (summary: string) => void`，原樣透傳給內部渲染的 `<AiPanel onSummaryUpdate={onSummaryUpdate} ... />`。
- **`TerminalApp.tsx`**：在渲染 `<TerminalView>` 的地方（跟現有 `onAgentProgress={(done, total) => setTabs(prev => prev.map(...))}` 相鄰）新增：
  ```tsx
  onSummaryUpdate={(summary) => {
    setTabs((prev) =>
      prev.map((t) => t.id === tab.id ? { ...t, aiSummary: summary } : t)
    );
  }}
  ```
- **Title bar 文字組合**：在 `TerminalApp.tsx` 渲染 `<TitleBar />`之前，計算：
  ```tsx
  const activeTab = tabs.find((t) => t.id === activeId);
  const titleBarText = activeTab
    ? (activeTab.type === "terminal" && activeTab.aiSummary
        ? `${activeTab.title} - ${activeTab.aiSummary}`
        : activeTab.title)
    : "AITerm";
  ```
  然後 `<TitleBar title={titleBarText} />`。

  注意：這裡直接重用 `activeTab.title`（分頁自己的顯示名稱），而不是重新從 `type` 推導一份本地化標籤。好處是使用者若透過既有的 rename 功能把分頁改名（例如改成「Deploy Server」），title bar 會自動顯示「Deploy Server - 摘要」而不是寫死的類型名稱「終端機 - 摘要」，行為更直覺，且不需要額外程式碼。

### 4. 顯示格式（已透過 mockup 確認）

- 格式固定為 `${分頁標題} - ${摘要}`，沒有摘要時只顯示分頁標題本身（不加減號）。
- 摘要過長時的視覺截斷完全依賴 `TitleBar/index.css` 既有的 `white-space: nowrap; overflow: hidden; text-overflow: ellipsis;`，不需要新增任何截斷邏輯。
- 切換分頁時，title bar 立即反映新的 active tab 的既有 `aiSummary`（若尚未生成則顯示分頁標題本身），**不會**觸發重新生成。

## 錯誤處理

- 摘要 IPC 呼叫失敗（網路錯誤、rate limit、供應商未設定等）：`summarizeConversation` 內 catch 例外，回傳 `null`；呼叫端收到 `null` 時不呼叫 `onSummaryUpdate`，title bar 維持原本文字（不顯示錯誤訊息，不重試）。
- 摘要呼叫本身不阻塞或延遲任何使用者互動——是在主要 AI 回應顯示完成、`isStreaming` 已變回 `false` 之後才另外背景觸發的獨立非同步呼叫，使用者可以立即繼續輸入下一則訊息，不需要等摘要跑完。

## 測試計畫

- 不新增測試涵蓋 title bar 文字組合邏輯本身的 UI 快照（這塊很難用現有測試框架有效驗證且價值有限）。
- 為 `summarizeConversation`（或最終定案的等效函式）寫幾個純函式單元測試，涵蓋：組合出的訊息陣列格式正確、`invokeAiChat` 呼叫失敗時回傳 `null` 而非拋出例外、成功時正確 trim 並回傳字串。使用現有的 `vi.mock` 模式（比照 `useAiChat.test.ts`/`useMcpChat.test.ts` 既有寫法）mock 掉 `invokeAiChat`。
- 手動驗證：開一個新終端機分頁 → 用 `/ai` 問一個問題 → 等回應完成 → 確認 title bar 更新成「終端機 - 摘要」；切換到另一個尚未用過 AI 的分頁 → 確認 title bar 顯示回該分頁標題本身，沒有摘要殘留；切回原分頁 → 確認摘要還在（同一次 App session 內）；重啟 App → 確認摘要消失，分頁標題恢復原樣。
