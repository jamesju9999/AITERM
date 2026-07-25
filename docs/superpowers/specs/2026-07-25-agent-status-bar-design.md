# Agent 狀態列：把 agent 協調狀態移出終端機

**日期**：2026-07-25
**狀態**：待審閱

## 背景

Agent（`/agent`）自動多步執行時，會把生命週期的協調狀態用 `term.write` 寫進 xterm 緩衝區：

- `[Agent: 思考下一步... (N/M)]`（`TerminalView.tsx` 約 1614）
- `→ asking AI...`（`handleAiQuery` 約 1450–1451，含回覆時的 `\x1b[1A\x1b[2K` 清除）
- `[Agent Mission Completed] 🎉` / `⚠ Agent stopped: ...`（3 個 `onComplete`/`onFail` 站點：約 460–473、921–932、1360–1369）
- 網路動作標籤 `🔍 搜尋:` / `📄 取得:`（約 1669–1670）

這些文字寫進 xterm 後，落在**固定高度**的 live-frame（`.aiterm-live-frame`，高度 = `liveRows × cellHeightPx`，`overflow: clip`）內，造成使用者實測回報的兩個問題：

1. **[1] 裁切**：`→ asking AI...` / 思考行等落在 live-frame 最底列，被下緣裁掉半行。
2. **[2] 留白**：為了讓完成訊息不被裁而把 `liveRows` 撐到 `MAX_LIVE_ROWS`（先前補丁），導致任務結束後到下個指令前，live-frame 出現一大片空白。

根因是**用終端機字元流承載 UI 狀態**：xterm 的內容受 live-frame 尺寸與游標定位牽制，天生不適合承載會變動、需要美觀排版的狀態訊息。

## 目標

把 agent 的**生命週期協調狀態**改用專屬 React 元件呈現（輸入框上方的細長狀態列），同時根治 [1] 與 [2]。實際指令與其輸出照舊經 PTY 進終端機 blocks。

## 範圍界定（已與使用者確認）

- **內容邊界**：只把「狀態」移到專屬 UI。**實際執行的指令與其輸出仍留在終端機 blocks**（它們是真正的 shell 內容，要進 scrollback）。
- **呈現位置**：輸入框上方的細長狀態列（沿用現有 `StreamingIndicator` 所在區塊的視覺風格）。
- **完成/失敗關閉行為**：`done`/`failed` 狀態**持續顯示**，直到使用者送出下一個指令（自動關閉）或點 ✕ 手動關閉。**不做定時自動淡出**（避免使用者錯過）。

### 明確排除（Non-goals）

- 不把指令執行過程收進獨立面板（終端機仍是指令與輸出的唯一去處）。
- 不改單次 `/ai`（非 agent）流程：維持現有 `StreamingIndicator` 取代 `WarpInput` 的互斥渲染（見 `2026-07-25-terminal-summary-and-thinking-indicator-design` Part 2）。
- 不處理「block 內指令重複顯示一次（`▶` echo + shell echo）」——既有行為，另案。
- 不動每個指令的危險警告 / 錯誤提示 / 密碼等待提示（非 agent 生命週期狀態；密碼等待仍需在終端機提示，因為使用者要在終端機輸入）。

## 設計

### 1. 新元件 `src/components/AgentStatusBar.tsx`（+ `.css`）

細長狀態列，依 `phase` 呈現：

| phase | 顯示內容 |
|-------|----------|
| `asking` | `◐ 詢問 AI 中…` · 目標 · `步驟 N/M` |
| `running` | `▶ 執行中: <command>` · `步驟 N/M` |
| `web` | `🔍 搜尋: <query>` 或 `📄 取得: <url>` |
| `done` | `✅ 完成（N 步）` · ✕ 可關閉 |
| `failed` | `⚠ 已停止: <原因>` · ✕ 可關閉 |

Props（草案）：

```ts
interface AgentStatusBarProps {
  phase: "asking" | "running" | "web" | "done" | "failed";
  goal: string;
  step: number;        // 1-based 目前步驟
  maxSteps: number;
  detail?: string;     // running 的 command / web 的 query / failed 的原因
  webKind?: "search" | "fetch";
  onDismiss: () => void;
}
```

- 主題感知（light/dark，`@media prefers-color-scheme` + `data-theme` 兩向覆蓋，比照專案既有樣式）。
- i18n：新增字串到 `src/lib/i18n.ts`（en / zh-TW），沿用既有 `term_agent_*` 命名慣例。

### 2. TerminalView 狀態與渲染

- 新增 `agentPhase` state：`{ phase, step, maxSteps, detail?, webKind? } | null`。
- `goal` 沿用現有 `agentMission.goal`。
- 渲染優先序（輸入框區塊）：
  1. `agentPhase !== null` → 顯示 `<AgentStatusBar>`（**WarpInput 仍在下方保留**，可打字/中止）。
  2. 否則 `preview.loading`（單次 `/ai`）→ 維持現有 `StreamingIndicator`。
  3. 否則 → `WarpInput`。
- `onDismiss` 與「送出下一個指令」都把 `agentPhase` 設回 `null`。

### 3. 資料流

`runAgentLoop` / `handleAiQuery` 目前呼叫 `term.write` 寫狀態的位置，改成呼叫新的 `onPhase(update)` callback（與現有 `onStepComplete` 相同的 params threading 方式），由 TerminalView 更新 `agentPhase`。步驟數用它們既有的 `stepCount` / `maxSteps` 參數：

- 進入 `runAgentLoop`（`stepCount > 0`）→ `onPhase({ phase: "asking", step: stepCount+1, maxSteps })`。
- 自動執行指令（`shouldAutoExecute` 分支）→ `onPhase({ phase: "running", detail: command, ... })`。
- 網路動作 `onWebAction` → `onPhase({ phase: "web", detail: value, webKind: type, ... })`。
- `onComplete` → `onPhase({ phase: "done", step: 最終步數, ... })`。
- `onFail` → `onPhase({ phase: "failed", detail: msg, ... })`。

### 4. 從終端機移除的 `term.write`

移除：`[Agent: 思考下一步 (N/M)]`、`→ asking AI...`（含其 `\x1b[1A\x1b[2K` 清除序列）、`[Agent Mission Completed] 🎉`、`⚠ Agent stopped`、`🔍/📄` 網路標籤。

保留：`▶ command`（標記 agent 下達的指令、屬終端機內容）、真實指令輸出、完成後送 `\r` 給 shell 取得乾淨提示字元。

### 5. 還原先前兩個補丁（已被本設計取代）

先前為了緩解 [1]/[2] 加的兩個修改要一併還原，避免死碼與 [2] 留白：

- `handleAiQuery` 的 `clearThinkingLine` 參數與其兩處 `\x1b[1A\x1b[2K`（DONE / error 路徑）→ 移除（不再寫思考行，無殘留可清）。
- mission 結束時 `setLiveRows(MAX_LIVE_ROWS)` 的 effect（`prevMissionActiveRef` 那段）→ 移除（正是造成 [2] 留白的元凶）。

### 6. 錯誤處理

- 逾時（約 1659）、超過步數上限（`term_agent_max_steps`）→ 走 `onFail` → `failed` phase 顯示原因。
- AI 請求失敗 → 現有 `onAiError` → `onFail` → `failed` phase。
- 密碼等待提示維持現狀（終端機內），非本次範圍。

## 測試

- `src/components/AgentStatusBar.test.tsx`：RTL 驗證五個 phase 的渲染（含 done/failed 的 ✕ 關閉、步驟顯示、detail 文字）。
- 既有前端測試（286）維持綠燈；`tsc --noEmit`、`eslint` 乾淨（不新增錯誤）。
- 實機 `tauri:dev` 手動驗證：
  1. 跑 `/agent` 任務，終端機**不再**出現 `思考下一步` / `asking AI` / `Mission Completed` 等狀態文字。
  2. 執行期間 live-frame **無裁切**、任務結束後**無留白**。
  3. 完成列正確顯示，送出下一個指令或點 ✕ 後關閉；輸入/回顯正常無錯位。

## 檔案異動摘要

- 新增：`src/components/AgentStatusBar.tsx`、`AgentStatusBar.css`、`AgentStatusBar.test.tsx`
- 修改：`src/components/TerminalView.tsx`（新增 `agentPhase` state + 渲染 + `onPhase` threading + 移除狀態 `term.write` + 還原兩補丁）、`src/lib/i18n.ts`（新增字串）
