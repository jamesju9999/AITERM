# 終端機分頁標題改用指令歷史摘要 + 思考指示器不再蓋住輸入框

**日期**：2026-07-25
**狀態**：待審閱

## 背景

兩項使用者回報，都在終端機分頁：

1. **標題摘要來源改變**：稍早合併的功能（`2026-07-25-titlebar-ai-summary`）讓 title bar 顯示「分頁名稱 - AI 摘要」，但摘要是根據 `/ai` 對話內容生成的。使用者實測後發現，他要的其實不是「即時追蹤在做什麼」，而是**單純給每個終端機分頁一個穩定、好辨識的標題**（開多個終端機分頁時能分辨），而且應該根據**使用者實際打的 shell 指令**來生成，不是 AI 對話。純打指令（不呼叫 AI）也要有標題。

2. **思考指示器蓋住輸入框**：AI 生成指令/回應時（`preview.loading`），`StreamingIndicator`（「AI 生成指令中…」+串流文字）用 `position:absolute; bottom:0; z-index:10` 疊在 `WarpInput` 輸入框上面，導致重疊、串流文字溢出到輸入框下緣，視覺破碎。

## 範圍界定（已透過 mockup 與使用者確認）

- **Part 1 摘要來源**：**完全取代**——移除 AI 對話摘要，改用終端機指令歷史。不並存。
- **Part 1 生成時機**：每個終端機分頁**只生成一次**（第一個指令執行完成後，加短 debounce），之後固定不變。目的是穩定識別，不是即時更新——每個分頁一輩子只呼叫一次 AI。
- **Part 1 資料**：最近幾個指令的**指令文字 + cwd**，不含指令輸出（避免雜訊與過大 prompt）。
- **Part 2 版面**：思考期間讓 `StreamingIndicator` **取代** `WarpInput` 的位置（互斥渲染），沿用輸入框的圓角外框樣式；思考結束換回輸入框。

### 明確排除（Non-goals）

- 不並存兩種摘要來源。
- 不做摘要的即時／多次更新（刻意只生成一次）。
- 摘要不含指令輸出內容。
- 不持久化摘要（維持前一功能的決定，重啟後重新生成一次）。
- 只套用在終端機分頁（其餘分頁類型不受影響——這點沿用前一功能，`TerminalApp` 的組字邏輯已經有 `type === "terminal"` 的 gate，不需改）。
- Part 2 不改變思考指示器顯示的**內容**（「AI 生成指令中…」label + 串流 explanation + 游標），只改它的**定位/版面**。

## 設計

### Part 1：摘要改用指令歷史

#### 1a. 改寫 `src/lib/summarizeTab.ts`

現有 `summarizeConversation(messages: McpChatMessage[], ...)` 改成吃終端機指令 blocks。既有的 `TerminalBlock` 型別（`src/hooks/useTerminalBlocks.ts`）包含 `command`、`cwd`、`status`、`exitCode`、`rawOutput` 等。

新簽名：

```ts
export async function summarizeCommands(
  blocks: TerminalBlock[],
  sessionId: string,
  locale: Locale,
): Promise<string | null>
```

行為：
- 取 `blocks` 中 `command` 非空白的項目，最後 N 個（例如 `MAX_CONTEXT_COMMANDS = 10`）。
- 若一個都沒有 → 回傳 `null`（不呼叫 AI）。
- 組 prompt：把這些指令文字（每行一個）+ 最後一個 block 的 `cwd` 當上下文，要求 AI 用不超過 20 字（中文）／40 字元（英文）生成一句「這個終端機工作階段在做什麼」的精簡摘要，不加標點結尾、不加引號。
- 呼叫既有 `invokeAiChat([{ role: "user", content: prompt }], \`${sessionId}-summary\`, undefined, false, locale)`——重用預設供應商、獨立的 `-summary` session id、`useMcp=false`。
- 回傳 `reply.content?.trim() || null`；任何例外 catch 後回傳 `null`（靜默失敗，不影響 UI）。

移除舊的 `McpChatMessage`／`contentToDisplayString` import，改 import `TerminalBlock`。

#### 1b. 改寫 `src/lib/summarizeTab.test.ts`

改成測試新的 `summarizeCommands` 簽名，涵蓋：無指令 → `null` 且不呼叫 `invokeAiChat`；正常指令 → 用 `-summary` session id 呼叫並回傳 trim 後字串；`invokeAiChat` reject → `null`；空回覆 → `null`；只取最後 N 個指令（給超過 N 個時，最舊的不出現在 prompt）。沿用現有 mock `@tauri-apps/api/core` 的 `invoke` 的模式。

#### 1c. 觸發邏輯移到 `src/components/TerminalView.tsx`

- `TerminalView` 已經有 `blocks`（`TerminalBlock[]`，來自 `useTerminalBlocks`）與 `onSummaryUpdate` prop（目前傳給 `AiPanel`）。
- 移除傳給 `<AiPanel>` 的 `onSummaryUpdate={onSummaryUpdate}`（改由 `TerminalView` 自己用）。
- 新增一個 ref `summaryGeneratedRef = useRef(false)` 與一個 `useEffect`：

```tsx
  // Generate a one-time identifying tab title from the first executed
  // command(s). Debounced so rapid successive commands are captured
  // together; guarded so it only ever fires once per terminal session.
  const summaryGeneratedRef = useRef(false);
  useEffect(() => {
    if (summaryGeneratedRef.current) return;
    const hasFinalized = blocks.some((b) => b.status === "completed" || b.status === "failed");
    if (!hasFinalized) return;
    const timer = setTimeout(() => {
      summaryGeneratedRef.current = true;
      summarizeCommands(blocks, sessionId, locale)
        .then((summary) => {
          if (summary) onSummaryUpdate?.(summary);
        })
        .catch(() => {});
    }, 1500);
    return () => clearTimeout(timer);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [blocks, sessionId, locale, onSummaryUpdate]);
```

- 這個 effect 在 `blocks` 變化時重跑；`summaryGeneratedRef` 一旦為 true 就 no-op；debounce 清理讓連續指令重設計時器（打 3 個快指令 → 最後一個之後 1.5 秒才生成，`blocks` 此時已含 3 個）。因為 inactive 終端機分頁是用 `visibility:hidden` 隱藏而非 unmount（見 CLAUDE.md），`TerminalView` 每個分頁保持 mounted，ref 跨分頁切換存活，等同「每個終端機分頁只生成一次」。
- `dangerous` deps 註解：`blocks` 是每次 render 新陣列，但 `summaryGeneratedRef` guard + debounce 讓它不會重複觸發網路呼叫；沿用專案既有 effect 的 `eslint-disable-next-line react-hooks/exhaustive-deps` 慣例。

#### 1d. 移除 `AiPanel` 的摘要邏輯

`src/components/AiPanel/index.tsx`：
- 移除 `import { summarizeConversation } from "../../lib/summarizeTab";`。
- 移除 `AiPanelProps` 的 `onSummaryUpdate?: (summary: string) => void;` 與其在函式簽名的解構。
- 移除 `lastSummarizedCountRef` 與那個呼叫 `summarizeConversation` 的 `useEffect`。
- 移除「New Chat」`onClick` handler 裡的 `lastSummarizedCountRef.current = 0;`（回到只有 `chat.clear()` + `setHistoryOpen(false)`）。

#### 1e. 還原 `AiPanel.test.tsx`

把前一功能為了配合 AI 對話摘要而加的測試改動還原：
- 移除 `summaryResponseContent` 變數、mock `invoke` 裡的 `-summary` 分支、`realChatCalls()` helper。
- `aiChatCalls` 回到記錄 bare messages 陣列（`{ role, content }[][]`），或最簡單地還原成前一功能之前的形狀。
- 兩個 agent-mode 測試的斷言還原回 `expect(aiChatCalls.length).toBe(2)` / `aiChatCalls[1].map(...)`（因為不再有背景 `-summary` 呼叫）。
- 移除「calls onSummaryUpdate with the trimmed summary after a turn settles」這個新測試。

（實作時以「還原到 `AiPanel.test.tsx` 在 commit `8f33675` 的狀態」為準，即前一功能 Task 3 動它之前的版本——那時 8 個測試全過、無 `-summary` 概念。）

#### 1f. `TerminalApp.tsx`、`TerminalView` 的 `onSummaryUpdate` prop、`Tab.aiSummary`

**不改**。`TerminalApp` 的 `onSummaryUpdate` handler（寫入 `tab.aiSummary`）與 title bar 組字（`type === "terminal" && aiSummary ? "title - summary" : title`）維持原樣；`TerminalView` 保留接收 `onSummaryUpdate` prop（改由自己呼叫）；`Tab.aiSummary` 欄位維持。

### Part 2：思考指示器不再蓋住輸入框

#### 2a. `TerminalView.tsx` 渲染改互斥

把目前的：

```tsx
      {!isAlternateBuffer && (
        <WarpInput ... />
      )}
      {preview.loading && (
        <StreamingIndicator visible text={streamText} />
      )}
```

改成：思考中（`preview.loading`）顯示 `StreamingIndicator` 取代 `WarpInput`；否則顯示 `WarpInput`。兩者都仍受 `!isAlternateBuffer` gate：

```tsx
      {!isAlternateBuffer && (
        preview.loading
          ? <StreamingIndicator visible text={streamText} />
          : <WarpInput ... />
      )}
```

（`WarpInput` 的完整 props/handlers 維持不變，只是被包進三元判斷。）

#### 2b. `StreamingIndicator.css` 改成正常流、沿用輸入框外框

把 `.aiterm-streaming` 從絕對定位改成正常流元素，並沿用 `WarpInput` 的圓角外框視覺（同樣的 margin、圓角、綠色邊框、深底），讓它看起來就是輸入框進入思考狀態：

```css
.aiterm-streaming {
  margin: 10px 14px;
  padding: 8px 14px;
  background-color: #161616;
  border: 1.5px solid #2f9e7f;
  border-radius: 24px;
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 12px;
}
```

移除 `position: absolute; bottom: 0; left: 0; right: 0; z-index: 10;` 與原本的 `border-top`／半透明底色。`.aiterm-streaming__label`、`.aiterm-streaming__text`、`.aiterm-streaming__cursor` 的內部樣式維持（label 小字、text 有 `max-height` 內捲、游標閃爍），只是外層容器變成輸入框樣式。

#### 2c. `StreamingIndicator.tsx`

元件邏輯不改（`extractPartialExplanation`、`visible` gate、內容渲染都維持）。只有 CSS 與外層在 `TerminalView` 的定位方式改變。

## 錯誤處理

- 摘要生成失敗（網路、rate limit、供應商未設定、空回覆）：`summarizeCommands` 內 catch → `null`；`TerminalView` 收到 `null` 不呼叫 `onSummaryUpdate`，title bar 維持只顯示分頁名稱。因為 `summaryGeneratedRef` 在嘗試時已設為 true，**失敗後不會重試**（符合「只生成一次」的取捨——若首次失敗，該分頁這輪就沒有摘要，維持分頁名稱，直到 App 重啟）。
- Part 2 純視覺，無新錯誤路徑。

## 測試計畫

- `summarizeTab.test.ts`：改寫為 `summarizeCommands` 的單元測試（見 1b），mock `invoke`。
- `AiPanel.test.tsx`：還原到不含 `-summary` 概念的狀態（見 1e），確認 8 個測試回到全過。
- 不為 `TerminalView` 的觸發 effect 或 Part 2 的版面改動新增自動化測試（`TerminalView` 無現成測試 harness，且 Part 2 是純 CSS/版面，價值有限）——比照專案既有慣例。
- 手動驗證：
  - 開新終端機分頁 → 打一個指令（如 `ls`）→ 等執行完 → 約 1.5 秒後 title bar 從「終端機」變「終端機 - 摘要」。
  - 同分頁再打更多指令 → title **不再變**（只生成一次）。
  - 開第二個終端機分頁 → 各自獨立生成、互不影響。
  - 用 `/ai` 觸發生成指令 → 思考期間輸入框位置顯示「AI 生成指令中…」的圓角框（不再有疊圖），思考結束換回可輸入的輸入框。
  - 重啟 App → 摘要消失，分頁恢復只顯示名稱，下次打指令再生成一次。
