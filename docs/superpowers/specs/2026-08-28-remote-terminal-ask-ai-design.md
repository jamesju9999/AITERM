# 遠端終端機支援 Ask AI（Agent 迴圈）Design

**Goal:** 讓**觀看端**（`RemoteTerminalView`，即「遠端終端機」分頁）能用**觀看端自己的 AI provider**，透過 Agent 迴圈自動操作遠端主控端的終端機。使用情境：遠端主控端那台電腦的 AI 不夠強（或沒設定），改用觀看端這邊較強的 AI 來驅動遠端機器。

## 背景

`RemoteTerminalView` 目前有一顆「Ask AI」按鈕，純粹是視覺佔位（見 `2026-08-28-remote-terminal-toolbar-design.md`）：點下去只呼叫 `setAiUnsupported(true)` 顯示「AI 指令目前不支援於遠端分頁」提示，`handleWarpSubmit` 對 `/ai`、`/agent` 開頭直接擋掉。

本機終端機（`TerminalView.tsx`）的 `/agent` 走的是一套 callback-driven 的 Agent 迴圈（`runAgentLoop`，`TerminalView.tsx:2250`）：每步「問 AI → 自動送出指令 → 等 OSC 133;D 標記完成 → 取輸出與 exit code → 遞迴下一步」，直到 AI 回 `DONE`、到步數上限或逾時。情境（cwd / 近期輸出 / OS / shell）由 Rust 端 `context::snapshot(&pty_manager, &session_id)` 從**本機 PTY session** 收集。

觀看端沒有本機 PTY——它透過分享協定跟主控端的 PTY 溝通（`shareViewerSend` 送鍵盤輸入、`onShareViewerData` 收輸出）。但觀看端的 `useTerminalBlocks(connId, ...)` 已經完整運作：`WarpInput` 送出的指令會建立追蹤區塊，區塊完成偵測走的是主控端 byte stream 裡的 OSC 133;D（含 exit code），`submitCommand(cmd, onComplete)` 的 `onComplete` 會在 `finalizeBlock` 時帶著 `{ command, exitCode, rawOutput }` 觸發——這正是 `runAgentLoop` 需要的契約。

## 決策紀錄（brainstorm 過程中定案）

1. **AI 在觀看端跑**，用觀看端自己的 provider 設定與金鑰。不是主控端跑再回傳。理由＝主控端 AI 可能不夠強，重點就是借用觀看端的 AI。
2. **範圍＝Agent 自動迴圈**（不只單發 Ask AI）。`/ai` 前綴一併解鎖。
   ~~等於 `maxSteps = 1` 的迴圈。~~ **實作修正（2026-08-28，Task 6 review）**：`/ai`
   與 `/agent` 共用同一個步數上限（`max_agent_steps` 設定），跟本機
   `TerminalView.tsx` 對兩個前綴一視同仁的既有行為一致。`maxSteps = 1` 會讓
   `runAgentLoop` 在第一條指令跑完後撞上 `stepCount >= maxSteps` 而顯示一個
   誤導性的「已達最大迭代次數」失敗橫幅——所以不採用。
3. **cwd 不透過新協定訊息取得**。靠 (a) `recent_output` 通常含 shell 提示字元、AI 像人一樣讀，(b) AI 需要確定時自己產生 `pwd` 當一步。
4. **執行安全模型比照本機**：`CommandGuard` 判安全的自動跑、判危險的跳 `CommandPreview` 等觀看端使用者確認。
5. **實作方向＝把迴圈移植進觀看端，新增吃明確情境的 Rust 指令**，且前端 `runAgentLoop` 從 `TerminalView.tsx` **抽成共用模組**，本機與遠端共用同一份。不複製。
6. **Ask AI 進入點＝輕量對話式面板**（滑出式），引擎是共用的 `runAgentLoop`。不重用 `AiPanel` 元件本身（它綁死本機 `sessionId` 且走 `invokeAiChat` + MCP 的另一套迴圈）。

## 架構與資料流

觀看端打 `/agent <目標>`（或在面板輸入框送出目標）：

```
WarpInput / 面板輸入框 送出 "/agent 目標"
  → parseAgentPrefix 命中 → 不 submitCommand，改 startMission + 啟動 runAgentLoop
  → runAgentLoop（共用模組）第 N 步：
      1. 組 query = 目標 + 前 N-1 步歷史（沿用現有格式）
      2. queryFn(query) = invokeAiQueryCtx(query, buildRemoteCtx(), locale)   ← 注入
         buildRemoteCtx() = { os: hostPlatform, shell: null, cwd: null,
                              recent_output: <觀看端 xterm buffer 末 ~4000 字> }
      3. Rust ai_query_ctx：跳過 PTY 收集，用傳入的 ctx 組 EnvSnapshot 跑 provider
      4. 回 AiCommandReady { command, explanation, risk_level }
      5. shouldAutoExecute(觀看端 executionMode, risk, agentActive=true)?
         safe      → submitCommand(command, onBlockDone)
         dangerous → CommandPreview 等使用者確認 → submitCommand(command, onBlockDone)
         （submitCommand 的 write 走 shareViewerSend → 遠端 PTY 執行）
      6. 遠端輸出經 onShareViewerData 回來 → OSC 133;D → finalizeBlock → onBlockDone(block)
      7. 取 block.rawOutput（末 2000 字）+ block.exitCode → 加進歷史 → 遞迴第 N+1 步
  → AI 回 "DONE"（完成）/ 到 maxSteps（上限）/ 60s 未完成（逾時）→ 收尾
```

**不變的部分**：`runAgentLoop` 迴圈骨架、`DONE` 收尾、`AITERM_WEB_SEARCH:` / `AITERM_WEB_FETCH:` sentinel、60s 逐步逾時、`useAgentMission`、`CommandPreview`、`shouldAutoExecute`。

**改的部分**：只有 (a) 情境來源＝觀看端自組而非本機 PTY，(b) `submitCommand` 的 write 走分享連線（觀看端 `useTerminalBlocks(connId)` 已經是這樣接的，不需改）。

## 元件一：新 Rust 指令 `ai_query_ctx`

位置 `src-tauri/src/commands/ai.rs`，與 `ai_query`（同檔 221 行）並列。

```rust
#[derive(Debug, Deserialize)]
pub struct RemoteCtx {
    pub os: String,                     // "windows" | "linux" | "macos"（觀看端由 hostPlatform 對應）
    pub shell: Option<String>,          // 觀看端未知 → None
    pub cwd: Option<String>,            // 觀看端未知 → None
    pub recent_output: Option<String>,  // 觀看端 xterm buffer 末段
}

#[tauri::command]
pub async fn ai_query_ctx(
    query: String,
    ctx: RemoteCtx,
    locale: Locale,
    conn_id: String,                    // ai-stream 事件的識別碼（取代 session_id）
    app: AppHandle,
    router: State<'_, AiRouter>,
) -> Result<AiCommandReady, AiError>
```

- `EnvSnapshot` 由 `ctx` 直接組。`context.rs` 既有的 `snapshot_from_parts(os, shell, cwd)` 擴充成能吃 `Option<shell>` / `Option<cwd>` / `Option<recent_output>`（或新增 `snapshot_from_remote_ctx`），`dir_listing` 一律 `None`（觀看端無法列遠端目錄）。
- `ai_query`（現有）221–318 行裡「resolve provider → `build_single_command_prompt` → `generate_json` 串流 → 空回應 guard → `extract_json_from_response` → `serde_json::from_str::<AiSingleCommand>` → `DONE` 短路 → `CommandGuard::classify` → 取較高 risk → 組 `AiCommandReady`」這一整段抽成私有 `async fn run_single_command(snapshot: EnvSnapshot, query: String, locale: Locale, router: &AiRouter, app: &AppHandle, stream_id: String) -> Result<AiCommandReady, AiError>`，`ai_query` 與 `ai_query_ctx` 都呼叫它。**不複製這段邏輯。**
- `ai-stream` 事件的 `session_id` 欄位在 `ai_query_ctx` 路徑填 `conn_id`（觀看端面板監聽這個值）。
- 在 `src-tauri/src/lib.rs`（或註冊 handler 的地方）把 `ai_query_ctx` 加進 `invoke_handler`。

### 前端 IPC 包裝

`src/ipc/ai.ts` 新增：

```ts
export interface RemoteCtx {
  os: string;
  shell: string | null;
  cwd: string | null;
  recentOutput: string | null;
}

export function invokeAiQueryCtx(query: string, ctx: RemoteCtx, connId: string, locale: Locale) {
  return invoke<AiCommandReady>("ai_query_ctx", { query, ctx, connId, locale });
}
```

### 觀看端組 `ctx`

`RemoteTerminalView` 內 `buildRemoteCtx()`：

- `os`：`hostPlatform === "windows" ? "windows" : "linux"`（`hostPlatform` 已由 `onShareViewerGranted` 的 `hostOs` 設定；目前只分 windows / other，other 一律當 linux。未來若 granted 事件帶更細的 OS 再放寬）。
- `shell`：`null`。
- `cwd`：`null`。
- `recentOutput`：讀 `termRef.current.buffer.active`，從 `baseY` 往回逐行 `translateToString(true)` 串接，取末約 4000 字（對齊本機 `get_recent_output` 的 4096）。term 尚未建立時 → `null`。

## 元件二：抽共用的 `agentLoop.ts`

新檔 `src/lib/agentLoop.ts`。把 `TerminalView.tsx` 現有這幾段整段搬過去並 export：

- `handleAiQuery`（2119）
- `runAgentLoop`（2250）
- `interface AgentLoopParams`（2226）
- `normalizeAiError`（2392）
- 相關的小常數 / 型別（`INITIAL_PREVIEW`、`PreviewState` 等若只有這些函式在用就一起搬；共用的留在原處 import）。

**唯一的介面變更**：`handleAiQuery` 目前寫死 `invokeAiQuery(query, sessionId, locale)`。改成注入：

```ts
interface AgentLoopParams {
  // ...現有欄位全部保留...
  queryFn: (query: string) => Promise<AiCommandReady>;  // 取代內部寫死的 invokeAiQuery
  // sessionId 保留：term.write 的狀態行、onPhase 仍用它當識別；只是不再拿去呼叫 IPC
}
```

- `handleAiQuery` 內 `invokeAiQuery(query, sessionId, locale)` → `params.queryFn(query)`（`handleAiQuery` 也多收一個 `queryFn` 參數，由 `runAgentLoop` 傳入）。
- 本機 `TerminalView.tsx`：四個 `runAgentLoop({...})` 呼叫點（801、946、1482、2019 附近）補 `queryFn: (q) => invokeAiQuery(q, session, locale)`。行為完全不變。
- `TerminalView.tsx` 改成 `import { runAgentLoop } from "../lib/agentLoop"`，刪掉搬走的那幾段。

**風險控管**（專案記憶有多次 refactor 弄壞 agent / 分頁邏輯的紀錄）：

- 這是純搬移 + 一個參數注入，**不改迴圈邏輯本身**。
- 搬完立刻跑既有 `npm run test`（`TerminalView` 的 agent 相關測試是回歸關卡）＋ `npx tsc -b`，全綠才繼續。
- 預期 `TerminalView.tsx` 的 diff 形狀：刪一大段、加一行 import、四處加 `queryFn`。任何超出這個形狀的改動都要停下來檢視。

## 元件三：觀看端輕量面板

### 新檔 `src/components/RemoteTerminalView/AgentPanel.tsx`

滑出式面板，沿用 `AiPanel` 既有的 CSS 定位 / 寬度變數（不重新設計一套）。

**Props**：

```ts
interface Props {
  mission: AgentMission | null;      // 來自 useAgentMission
  phase: AgentPhase | null;          // runAgentLoop 的 onPhase 最新值
  streamText: string;                // setStreamText 的內容
  preview: PreviewState;             // 危險指令等待確認時
  onSubmitGoal: (goal: string) => void;
  onStop: () => void;
  onConfirmPreview: () => void;      // CommandPreview 的「執行」
  onCancelPreview: () => void;
  onClose: () => void;
  disabled: boolean;                 // 唯讀 / 未連線
  t: Translations;
}
```

**版面**：

- 頂：標題（新翻譯鍵 `remote_agent_panel_title`「AI 代理」）＋關閉鈕。
- 中：對話式任務紀錄，資料來源 `mission.history`（`{ command, exitCode, output }[]`）：
  - 每步一張卡：`▶ {command}`、折疊的 `{output}`（已是末 2000 字）、exit code 徽章（`0` 綠 / 非 0 紅）。
  - 目前步驟：依 `phase.phase` 顯示 `asking` / `running` / `web` 的狀態行 ＋ `streamText` 串流文字。
  - 結束訊息：`DONE` → `remote_agent_done`「✅ 任務完成」；`onFail` → `remote_agent_failed`「⚠ {reason}」；到步數上限沿用本機 `term_agent_max_steps`。
- 底：
  - 任務**非**進行中：輸入框（送出＝ `onSubmitGoal`）。
  - 任務進行中：換成「■ 停止」鈕（`onStop`）。
  - `preview.visible` 時：在底部顯示 `CommandPreview`（複用既有元件），接 `onConfirmPreview` / `onCancelPreview`。

### `RemoteTerminalView/index.tsx` 變更

**移除**：

- `aiUnsupported` state 與 `{aiUnsupported && <div ...>}` 區塊。
- `handleWarpSubmit` 裡 `parseAiPrefix(cmd) !== null || parseAgentPrefix(cmd) !== null` → `setAiUnsupported(true)` 的擋法。
- i18n 的 `remote_terminal_ai_unsupported` 鍵一併從 `src/lib/i18n.ts` 的兩個語系刪掉，並更新 `src/lib/i18n.remoteTerminal.test.ts` 對應斷言。

**新增 state / ref**：

- `agentPanelOpen: boolean`。
- `useAgentMission()` → `mission, startMission, appendHistory, stopMission, clearMission`。
- `abortRef = useRef(false)`。
- `executionModeRef`：讀觀看端既有的 execution mode 設定（跟本機同一個 localStorage / context 來源；`RemoteTerminalView` 目前沒讀，這次接上）。
- `streamingRef = useRef(false)`、`[streamText, setStreamText]`。
- `[preview, setPreview]`（`PreviewState`，初值 `INITIAL_PREVIEW`）。
- `[agentPhase, setAgentPhase]`。

**行為接線**：

- Ask AI 鈕 `onClick` → `setAgentPanelOpen(true)`（不再 `setAiUnsupported(true)`）。唯讀時 `disabled` + tooltip `remote_agent_needs_control`「需要控制權才能使用 AI 代理」。
- `handleWarpSubmit(cmd)`：
  - `parseAgentPrefix(cmd)` 命中 → `startAgentMission(goal, maxSteps=max_agent_steps)`。
  - `parseAiPrefix(cmd)` 命中 → `startAgentMission(goal, maxSteps=max_agent_steps)`（見決策 2 的實作修正，跟 `/agent` 同步數上限）。
  - 否則 → 現有的 `submitCommand(cmd)`。
- 面板輸入框送出 → 同一個 `startAgentMission(goal, maxSteps=max_agent_steps)`，並確保 `agentPanelOpen`。
- `startAgentMission(goal, maxSteps)`：
  ```
  if (任務進行中) return;             // 一次一個任務
  abortRef.current = false;
  setAgentPanelOpen(true);
  startMission(goal, maxSteps);
  runAgentLoop({
    t, goal, locale, sessionId: connId, term: termRef.current,
    getSubmitCommand: () => submitCommand,
    setPreview, setStreamText, streamingRef, executionModeRef,
    writeRed: (m) => termRef.current?.write(`\r\n\x1b[31m${m}\x1b[0m\r\n`),
    abortRef, stepCount: 0, maxSteps, history: [],
    queryFn: (q) => invokeAiQueryCtx(q, buildRemoteCtx(), connId, locale),
    onPhase: setAgentPhase,
    onComplete: () => { setAgentPhase({ phase: "done", ... }); stopMission(); },
    onFail: (msg) => { setAgentPhase({ phase: "failed", reason: msg }); stopMission(); },
    onStepComplete: (info) => appendHistory(info.command, info.exitCode, info.output),
  });
  ```
  （`onStepComplete` 用來把每步餵進 `useAgentMission.history` 給面板畫；本機那邊是拿去轉發 Telegram，這裡改用途。）
- `onStop`：`abortRef.current = true; stopMission(); setPreview(INITIAL_PREVIEW);`。
- 面板 `onClose`：若任務進行中 → 等同 `onStop`；否則單純 `setAgentPanelOpen(false)`。

**唯讀 / 連線狀態聯動**（見下節邊角案例）：`phase.mode` 變 `read_only`、`onShareViewerResync`、`onShareViewerEnded` 時，若 `mission?.active` → `abortRef.current = true` + `onFail(對應訊息)`。

## 邊角案例與風險

| 情境 | 處理 |
|---|---|
| 主控端 shell 沒有 OSC 133;D shell integration | 迴圈偵測不到每步完成 → 走 60s 逐步逾時 → `onFail`。面板顯示 `remote_agent_no_shell_integration`「遠端 shell 未啟用指令追蹤，無法自動接續」。硬性前提，不做 heuristic 猜完成。 |
| 任務進行中收到 `onShareViewerResync` | 現有邏輯 `clearAllBlocks()`，等待中的 `onBlockDone` 永不觸發。主動 `abortRef = true` + `onFail`「連線重新同步，任務中止」，不等逾時。 |
| 主控端中途收回控制權（`onShareViewerControlChanged` → `read_only`） | `abortRef = true` + `onFail`「已失去控制權」。面板輸入框停用。 |
| 連線結束（`onShareViewerEnded`）任務還在跑 | 同上，`abortRef` + `onFail`。 |
| 使用者在任務進行中於 `WarpInput` 打字送出 | 比照本機：中斷任務（`abortRef = true` + `stopMission`），該行**不**送到遠端（避免與 agent 指令交錯）。 |
| 危險指令等 `CommandPreview` 確認時使用者關面板 | 關面板＝中止任務（`abortRef` + `stopMission` + `setPreview(INITIAL_PREVIEW)`）。 |
| StrictMode 雙掛載 | 面板 state 在 `RemoteTerminalView` 層，與現有 `disconnectTimerRef` 的 StrictMode 處理同層；`runAgentLoop` 是 callback-driven 不吃 effect，無新增陷阱。 |
| 觀看端沒設 AI provider | `invokeAiQueryCtx` 回 `AiError::not_configured` → `onAiError` → `onFail`，面板顯示既有 `term_setup_hint_provider`。 |
| `recent_output` 讀不到（term 未建立） | `ctx.recentOutput = null`，AI 少一點情境，不阻斷。 |
| 同時多個遠端分頁各自跑任務 | 每個 `RemoteTerminalView` 有自己的 `useAgentMission` / `abortRef` / `connId`；`ai-stream` 用 `connId` 分流，互不干擾。 |

## 測試

### Rust（`src-tauri/tests/`，wiremock）

- `ai_query_ctx` 用傳入的 `ctx`（os / cwd / recent_output）組出 prompt，送到 mock provider，回 `AiCommandReady`。
- `ctx.cwd = None` / `recent_output = None` 時 prompt 不崩、不含空欄位雜訊。
- `CommandGuard` 仍把危險指令 risk 提級（沿用 `ai_query` 既有測試模式）。
- 抽出 `run_single_command` 共用後，既有 `ai_query` 測試全綠。

### 前端（Vitest + RTL）

- **回歸關卡**：`agentLoop.ts` 抽出後，`TerminalView` 既有 agent 測試全綠、`npx tsc -b` 過。
- `agentLoop` 單元測試：注入假 `queryFn` + 假 `submitCommand`，驗證多步遞迴、`DONE` 收尾、步數上限、60s 逾時（`vi.useFakeTimers`）、web sentinel 攔截、`abortRef` 生效。
- `AgentPanel.test.tsx`：
  - 送出目標 → 呼叫 `onSubmitGoal` → `mission.history` 多步時渲染多張卡片 + exit code 徽章顏色。
  - 進行中顯示「停止」鈕，點了觸發 `onStop`。
  - `phase` 為 `done` / `failed` 各自的結束文案。
  - `preview.visible` 時底部出現 `CommandPreview`。
- `RemoteTerminalView/index.test.tsx` 更新：
  - Ask AI 鈕在 `mode === "control"` 點了開面板；`read_only` 時 `disabled`。
  - `/agent xxx` 從 `WarpInput` 送出 → 啟動 mission、**不**呼叫 `submitCommand` 送原字串。
  - `/ai xxx` → 啟動 mission（步數上限同 `/agent`，見決策 2 的實作修正）。
  - 移除舊的「Ask AI 顯示 `aiUnsupported` 且不觸發 IPC」測試（行為已改）。
  - resync / control-revoked / ended 事件在任務進行中 → mission 被中止（`onFail` 文案）。

### 手動驗證（記憶：refactor 後必須手動驗證）

- 真的連一台遠端、`/agent` 跑一個多步任務（例：「找出佔用最多空間的三個目錄」），確認：指令逐步出現在遠端、輸出回流、面板卡片遞增、`DONE` 收尾。
- 中途按「停止」→ 迴圈確實停、不再送指令。
- 遠端 shell 關掉 OSC 133（或連一個沒 shell integration 的）→ 確認走逾時且面板訊息正確。
- 唯讀連線 → Ask AI 鈕 disabled。

## 明確不做（YAGNI）

- 不做主控端跑 AI、也不加「拉主控端 `context::snapshot`」的協定訊息。
- 不做 MCP 工具在觀看端（沒有本機 shell，無意義）。
- 不重用 `AiPanel` 元件；不動 `AiPanel` 的本機行為。
- 不做觀看端 AI 的多輪 chat（只有 Agent 迴圈 + `/ai` 單發）。
- 不處理 `hostPlatform` 更細的 OS 分類（等 granted 事件本身帶更多資訊再說）。
- 不做主控端對「觀看端 AI 正在驅動我的機器」的額外提示 / 否決 UI——主控端本來就看得到每條指令並可隨時收回控制權。

## 驗證指令

```bash
npm run test
npm run lint
npx tsc -b
cd src-tauri && cargo test
```
