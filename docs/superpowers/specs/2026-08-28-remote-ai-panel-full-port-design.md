# 遠端終端機完整移植 AiPanel 體驗 Design

**Goal:** 把本機終端機的 `AiPanel`（AI Studio 對話面板）完整體驗移植到**觀看端**（遠端終端機分頁），取代目前的輕量控制列 `AgentPanel`。觀看端使用者用**自己的 AI provider** 得到跟本機一樣的：多輪對話、model/provider 切換、串流回覆、對話紀錄持久化與瀏覽、AI 總結、agent 自動迴圈。

**里程碑：** 這是 [[project_remote_terminal_ask_ai]] 之後的獨立里程碑，另開分支 `feat/remote-ai-panel-port`。

## 背景

`feat/remote-terminal-ask-ai`（已併入 master，commit `07be712`）做完後，使用者實測覺得那個「AI 代理」控制列不夠——**沒有模型選擇、沒有 AI 對話紀錄、沒有 AI 總結**，跟本機終端機右側的 `AiPanel` 體驗不一樣。決定把 `AiPanel` 完整移植過來。

本機 `AiPanel`（`src/components/AiPanel/index.tsx`，793 行）提供：
- `useMcpChat(sessionId)` 多輪對話 + 串流 + MCP 工具呼叫 + localStorage session 持久化
- provider badge（點擊開 `ProviderPalette` 換預設 provider）+ `QuotaBadge`
- 自己一套 `<cmd>` tag 驅動的 agent 迴圈（`invokeAiChat` with `use_mcp=false`）
- 對話歷史側欄（載入 / 刪除）、放大縮小、寬度 resize、圖片附件、指令卡住偵測

觀看端沒有本機 PTY——透過分享協定跟主控端的 PTY 溝通。`AiPanel` 每個 `sessionId` 相依都要換成遠端等價物。

## brainstorm 定案的決策

1. **完整移植 `AiPanel` 的體驗**到遠端面板。
2. **不做 MCP**：遠端面板不提供 MCP 工具呼叫（工具跑的是觀看端本機，對遠端主控端沒意義）。面板上的 MCP 開關 / 工具數徽章整個拿掉。AI 要看遠端檔案就自己產生 `cat`/`ls` 指令。
3. **agent 引擎＝真的移植 `AiPanel` 自己那套 chat-agent 迴圈**（`<cmd>` tag、`invokeAiChat` 系、自然語言串流），不是重用 `agentLoop.ts`（那是單指令 JSON + OSC133，本機專用）。
4. **新面板完全取代舊的**：刪 `RemoteTerminalView/AgentPanel.tsx`。`/agent`、`/ai` 前綴改成把目標丟進新面板（`/agent` → agent 迴圈、`/ai` → 純對話「問，不執行」）。`agentLoop.ts` 留給本機 `TerminalView`（4 個呼叫點還在用）。`ai_query_ctx` 指令 + `invokeAiQueryCtx` + `tests/ai_query_ctx.rs` 變孤兒 → 一併移除。
5. **元件策略＝抽共用殼層 + 兩個薄包裝**：把純呈現層抽成 `ChatPanelShell`，`AiPanel` 與新的 `RemoteAiPanel` 各自 compose（提供自己的 chat hook + 執行/情境 handlers）。不 fork（避免 400+ 行漂移），不參數化 `AiPanel` 本體（避免動到實戰邏輯）。
6. **本版功能範圍**：自由對話模式（非 agent）、對話紀錄持久化 + 瀏覽/清除、AI 總結。**這版不做**：圖片附件、`QuotaBadge`、指令卡住偵測（`STUCK_IDLE_MS`）——留待後續。

## 沿用既有基礎（來自上一個里程碑）

`RemoteCtx` / `context::snapshot_from_remote_ctx` / `run_single_command`（`ai_query` 用）、`readRecentOutput` / `buildRemoteCtx`（`RemoteTerminalView`）、觀看端 `useTerminalBlocks(connId)` 的 block 追蹤與 `submitCommand(cmd, onComplete)` 契約、`ai-stream` 用 `connId` 分流、`ProviderPalette`、共享 `abortRef` + unmount 中止 effect。

## 架構與資料流

```
觀看端 Ask AI 鈕 / WarpInput 打 /agent|/ai
  → RemoteAiPanel（新，取代 AgentPanel）
  → 兩種模式：
     /ai <問題> 或 chat 輸入送出 → send()：標準對話，不執行任何指令
     /agent <目標> 或 agent 模式送出 → submitAgent()：<cmd> tag 迴圈

  submitAgent 每輪：
    1. invokeAiChatCtx([system prompt, ...history], buildRemoteCtx(), connId, providerId?, locale)
       system prompt = build_chat_prompt（拿掉 listDirectory；cwd 未知，提示 AI 需要時自己 pwd）
    2. 後端 ai_chat_ctx：snapshot_from_remote_ctx → build_chat_prompt → provider.generate 串流
       ai-stream 事件 session_id = connId（面板 / useRemoteAiChat 監聽這個）
    3. 回覆進 chat 訊息；解析 <cmd>…</cmd>
    4. 有 <cmd> → submitCommand(cmd, onComplete)（觀看端既有 useTerminalBlocks(connId)）
       → 指令以 TerminalBlockCard 出現在主終端機區、遠端執行
       → onComplete(block) → 取 block.rawOutput 末 2000 字
       → 組「Command `…` finished (exit N). Output: …」user 訊息 → 遞迴下一輪
    5. 沒 <cmd> → 迴圈結束；最後那段 assistant 文字＝AI 總結
```

## 元件一：後端 `ai_chat_ctx`

位置 `src-tauri/src/commands/ai.rs`。

**抽出 `run_chat` 核心**（比照現有 `run_single_command`）：

```rust
/// ai_chat 與 ai_chat_ctx 共用的「非 MCP 串流」核心：resolve provider →
/// build_chat_prompt → provider.generate 串流 ai-stream(kind=Chat) → 回 AiChatReply。
/// 呼叫端負責先做訊息驗證（空 / 末角色）。
/// stream_id = ai-stream 事件的 session_id 欄位（ai_chat 傳 PTY session id，
/// ai_chat_ctx 傳觀看連線 conn id）。
async fn run_chat(
    messages: Vec<ChatMessage>,
    snapshot: crate::ai::EnvSnapshot,
    provider_id: Option<String>,
    locale: Locale,
    router: &AiRouter,
    app: &AppHandle,
    stream_id: String,
) -> Result<AiChatReply, AiError>
```

內容＝現在 `ai_chat` 的 provider resolve（by id / default）＋ `build_chat_prompt` ＋ 非 MCP 串流路徑（`ai.rs` 目前約 529–552 行那段）。`ai_chat` 保留它的空訊息 / 末角色驗證與整個 MCP 分支，fall-through 時改呼叫 `run_chat`。行為不變，回歸關卡＝既有 `ai_chat` 測試。

**新指令**：

```rust
#[tauri::command]
pub async fn ai_chat_ctx(
    messages: Vec<ChatMessage>,
    ctx: RemoteCtx,
    provider_id: Option<String>,
    conn_id: String,
    locale: Locale,
    app: AppHandle,
    router: State<'_, AiRouter>,
) -> Result<AiChatReply, AiError> {
    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }
    if messages.last().map(|m| m.role.as_str()) != Some("user") {
        return Err(AiError::InvalidInput { reason: "last message must be from user".into() });
    }
    let snapshot = context::snapshot_from_remote_ctx(
        &ctx.os, ctx.shell.as_deref(), ctx.cwd.as_deref(), ctx.recent_output,
    );
    run_chat(messages, snapshot, provider_id, locale, &router, &app, conn_id).await
}
```

（無 MCP，所以不接受 `tool` 結尾的 history。）

**`lib.rs`**：`use` 加 `ai_chat_ctx`，`invoke_handler` 加 `ai_chat_ctx`。移除 `ai_query_ctx`（`use` + handler list）。

**移除**：`ai_query_ctx` 指令本體、`src-tauri/tests/ai_query_ctx.rs`。`RemoteCtx` / `snapshot_from_remote_ctx` / `run_single_command` 保留；`snapshot_from_remote_ctx` 的既有單元測試保留（改放進 `ai_chat_ctx.rs` 或留在 `context.rs` 的 `mod tests`）。

**新測試** `src-tauri/tests/ai_chat_ctx.rs`：`build_chat_prompt` 吃 `snapshot_from_remote_ctx` 的產物、`RemoteCtx` 欄位進 prompt、缺 shell/cwd degrade；空訊息 / 末角色非 user 回 `InvalidInput`。

### 前端 IPC（`src/ipc/ai.ts`）

新增（比照 `invokeAiChat` + 上一里程碑 `invokeAiQueryCtx` 的 camel→snake 轉換）：

```ts
export function invokeAiChatCtx(
  messages: ChatMessage[],
  ctx: RemoteCtx,
  connId: string,
  providerId?: string,
  locale: Locale = "zh-TW",
): Promise<AiChatReply> {
  return invoke<AiChatReply>("ai_chat_ctx", {
    messages,
    ctx: { os: ctx.os, shell: ctx.shell, cwd: ctx.cwd, recent_output: ctx.recentOutput },
    connId,
    providerId: providerId ?? null,
    locale,
  });
}
```

移除 `invokeAiQueryCtx`（`RemoteCtx` 型別保留）。更新 `src/ipc/ai.test.ts`（移除 `invokeAiQueryCtx` 測試、加 `invokeAiChatCtx` 測試）。

## 元件二：`ChatPanelShell`（從 AiPanel 抽出的純呈現層）

新檔 `src/components/ChatPanel/ChatPanelShell.tsx` + `ChatPanel/styles.css`（把 `AiPanel/styles.css` 裡屬於殼層的規則搬過來，或 `AiPanel` 繼續 import 同一份、`ChatPanelShell` 也 import——傾向搬到共用檔）。

**抽出**：面板容器 + 左緣寬度 resize handle + header（`✨ AITerm AI Studio` 標題、provider badge、放大/縮小、對話歷史抽屜鈕、`🗑 New Chat`、`✕` 關閉）+ 歷史側欄（session 列表、載入、刪除）+ `MessageList` + 輸入區（textarea、送出鍵、agent/chat 模式切換、串流中禁用）。

**Props（注入點）**：

```ts
interface ChatPanelShellProps {
  isOpen: boolean;
  onClose: () => void;
  expanded: boolean;
  onToggleExpanded: () => void;

  messages: McpChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  thinkingLabel: string;

  onSend: (text: string) => void;
  agentMode: boolean;
  onToggleAgentMode: () => void;
  onSubmitAgent: (text: string) => void;
  agentRunning: boolean;
  onAbortAgent: () => void;

  providerName: string;
  onOpenProviderPalette: () => void;

  sessions: McpChatSession[];
  onLoadSession: (s: McpChatSession) => void;
  onNewChat: () => void;
  onDeleteSession: (id: string) => void;

  /** 額外輸入控制項插槽——AiPanel 放 MCP 開關 + 工具數；RemoteAiPanel 不傳。 */
  extraInputControls?: React.ReactNode;
  /** 額外 header 控制項插槽——附件按鈕等未來擴充。 */
  extraHeaderControls?: React.ReactNode;
  /** QuotaBadge 等；RemoteAiPanel 本版不傳。 */
  headerBadge?: React.ReactNode;
}
```

**`AiPanel/index.tsx` 變薄包裝**：`useMcpChat(sessionId)` + 既有 `submitAgent` + 附件 state + MCP 開關（傳進 `extraInputControls`）+ `QuotaBadge`（`headerBadge`）+ 圖片附件按鈕（`extraHeaderControls` 或輸入區）。**邏輯不變**，回歸關卡＝既有 `AiPanel*.test.tsx` 全綠 + 手動驗證（附件、MCP、agent 模式、歷史）。

## 元件三：`useRemoteAiChat`（無 MCP 的遠端對話 hook）

新檔 `src/hooks/useRemoteAiChat.ts`（~150 行，職責對齊 `useMcpChat` 的非 MCP 部分）：

```ts
export function useRemoteAiChat(connId: string, buildCtx: () => RemoteCtx) {
  // messages / addMessage / clear / loadMessages
  // send(text): invokeAiChatCtx([...history, {role:"user",content:text}], buildCtx(), connId, providerId, locale)
  //   串流：listen("ai-stream")，kind==="chat" && session_id===connId → streamBuf += delta；done → addMessage(assistant)
  // isStreaming / streamBuf
  // sessions 持久化：localStorage key "aiterm-remote-chat-sessions"（跟本機 "aiterm-mcp-chat-sessions" 分開）
  //   自動存檔 / loadAllSessions / deleteSession
  // mountedRef guard 防 setState-after-unmount（比照 useAiChat）
  // ai-stream 監聽在 cleanup 解除
}
```

型別：把 `useMcpChat` 的 `McpChatMessage` / `McpChatSession` 搬到新檔 `src/types/chat.ts`，`useMcpChat` / `useRemoteAiChat` / `ChatPanelShell` 三方都從這裡 import（`useMcpChat.ts` 改成 re-export 以免動到既有 import 端）。`providerId` / `locale` 從 context / config 取，跟 `AiPanel` 一致。

## 元件四：`RemoteAiPanel`（取代 AgentPanel）

新檔 `src/components/RemoteTerminalView/RemoteAiPanel.tsx`。用 `forwardRef` + `useImperativeHandle` 曝 `{ submitAgent(text), send(text), abort() }` 給 `RemoteTerminalView`。

**Props**：`connId`、`buildRemoteCtx`、`submitCommand`（`useTerminalBlocks(connId)` 的）、`isControl: boolean`、`maxSteps: number`、`sharedAbortRef`、`isOpen`、`onClose`、`onOpenProviderPalette`。

**內容**：
- `const chat = useRemoteAiChat(connId, buildRemoteCtx)`
- 遠端版 agent 迴圈（移植 `AiPanel` 的 `runAgentLoop` / `submitAgent`）：
  - `invokeAiChat(msgs, sessionId, …)` → `invokeAiChatCtx(msgs, buildRemoteCtx(), connId, providerId, locale)`
  - `buildAgentSystemPrompt`：拿掉 `getSessionCwd` / `listDirectory`；系統提示詞說明「你在遠端終端機、看不到 cwd 與目錄列表，需要時先跑 `pwd` / `ls`」、「絕不做破壞性操作」
  - `onExecuteCommand(cmd, cb)` → `submitCommand(cmd, cb)`
  - 結果讀取：直接 `block.rawOutput.slice(-2000)`（不呼叫 `getPtyRecentOutput`）
  - `agentAbortRef`：跟 `sharedAbortRef` 綁（`RemoteTerminalView` 的 unmount / 連線事件會設它）
  - 每步逾時（60s，移植 `agentLoop.ts` 的概念）→ 對話加 assistant 訊息 `remote_agent_no_shell_integration` 意思、停迴圈
  - `sendRemoteResponse`（Telegram）不接
- render `<ChatPanelShell messages={chat.messages} … onSend={chat.send} onSubmitAgent={submitAgent} agentRunning={agentRunning} onAbortAgent={abort} extraInputControls={undefined} headerBadge={undefined} isOpen={isOpen} onClose={onClose} … />`
- `isControl === false` → 輸入區 disabled、`submitAgent` / `send` 早退

## 元件五：`RemoteTerminalView/index.tsx` 改接線

**移除**：`useAgentMission` / `runAgentLoop` / `INITIAL_PREVIEW` / `PreviewState` / `invokeAiQueryCtx` / `AgentPanel` import；`agentMission` / `agentPhase` / `streamText` / `preview` / `streamingRef` / `executionModeRef` / `agentMissionRef` / `missionRunningRef` state/ref；`startAgentMission` / `stopAgentMission` / 危險指令 `onConfirmPreview` 段；render 的 `<AgentPanel>`；`ai-stream` 監聽（搬進 `useRemoteAiChat`）。

**保留**：`readRecentOutput`（模組層）、`buildRemoteCtx`、`abortRef` + unmount 中止 effect（改傳給 `RemoteAiPanel`）、`maxAgentStepsRef` + `getConfig` effect、`submitCommandRef`。

**新增**：
- `const [aiPanelOpen, setAiPanelOpen] = useState(false)`、`const remoteAiPanelRef = useRef<RemoteAiPanelHandle>(null)`
- Ask AI 鈕 `onClick` → `setAiPanelOpen(true)`（唯讀時 `disabled` 不變）
- `handleWarpSubmit(cmd)`：
  - agent 迴圈進行中送出任何東西 → `remoteAiPanelRef.current?.abort()` + `return`
  - `parseAgentPrefix` 命中 → `setAiPanelOpen(true)` + `remoteAiPanelRef.current?.submitAgent(goal)`
  - `parseAiPrefix` 命中 → `setAiPanelOpen(true)` + `remoteAiPanelRef.current?.send(query)`
  - 否則 `submitCommand(cmd)`
- 連線事件（`onShareViewerResync` / `onShareViewerControlChanged` → 非 control / `onShareViewerEnded`）：`abortRef.current = true`（unmount effect 用的同一顆）+ `remoteAiPanelRef.current?.abort()`
- render `<RemoteAiPanel ref={remoteAiPanelRef} isOpen={aiPanelOpen} connId={connId} buildRemoteCtx={buildRemoteCtx} submitCommand={submitCommandRef.current /* 用 getter */} isControl={phase.kind === "live" && phase.mode === "control"} maxSteps={maxAgentStepsRef.current} sharedAbortRef={abortRef} onClose={() => setAiPanelOpen(false)} onOpenProviderPalette={…} />`

**刪檔**：`src/components/RemoteTerminalView/AgentPanel.tsx` / `AgentPanel.css` / `AgentPanel.test.tsx`。

**i18n**：`remote_agent_*` 鍵——`RemoteAiPanel` / `ChatPanelShell` 用得到的留（例如逾時訊息 `remote_agent_no_shell_integration`、`remote_agent_needs_control`、`remote_agent_aborted_*`），用不到的（`remote_agent_panel_title` / `remote_agent_stop` / `remote_agent_goal_placeholder` 等控制列專用）刪。`ChatPanelShell` 沿用 `AiPanel` 既有的寫死中文字串，不在這次擴大 i18n 化。

## 邊角案例與風險

| 情境 | 處理 |
|---|---|
| 主控端 shell 沒有 OSC 133;D | agent 迴圈每步 60s 逾時 → 對話加 assistant 訊息「遠端 shell 未啟用指令追蹤，無法自動接續」+ 停迴圈。（`STUCK_IDLE_MS` 本版不做。） |
| 任務進行中 resync / 收回控制權 / 連線結束 | `RemoteTerminalView` 呼叫 `remoteAiPanelRef.current.abort()`；面板對話加系統訊息說明原因，`agentRunning` 收掉。 |
| 分頁關閉 / unmount | 共享 `abortRef` 的 unmount effect 設 `true`；`useRemoteAiChat` cleanup 解除 `ai-stream` 監聽；串流中的 `invokeAiChatCtx` 靠 `mountedRef` guard。 |
| 唯讀連線 | Ask AI 鈕 `disabled`；面板已開則輸入區 disabled、`submitAgent`/`send` 早退。 |
| 觀看端未設 provider | `invokeAiChatCtx` 回 `AiError` → 對話顯示 `formatAiError` + setup hint（比照 `AiPanel` catch）。 |
| `recent_output` 讀不到 | `buildRemoteCtx` 回 `recentOutput: null`，prompt 少一段，不阻斷。 |
| 對話 session localStorage 混淆 | 分開 key `aiterm-remote-chat-sessions`。 |
| 危險指令 | chat 模式無 risk 分級；比照本機 `AiPanel`：靠系統提示詞 + `abortRef`，不加 `CommandGuard`。使用者看得到每條 `<cmd>` 出現在主終端機區、可隨時停 / 收回控制權。 |
| `ChatPanelShell` 抽出後本機 `AiPanel` 迴歸 | 抽出＝搬 JSX + 呈現 state，其餘注入，不改邏輯；`AiPanel*.test.tsx` 全綠 + 手動驗證（[[feedback_aipanel_attachment_regression]]：附件功能必須驗證存活）。 |

## 測試

### Rust（`src-tauri/tests/`）
- `ai_chat_ctx.rs`（新）：`RemoteCtx` 欄位進 `build_chat_prompt`、缺 shell/cwd degrade、空訊息 / 末角色非 user → `InvalidInput`。
- `ai_chat` 既有測試全綠（`run_chat` 抽出後）。
- 移除 `ai_query_ctx.rs`。

### 前端（Vitest + RTL）
- `ChatPanelShell.test.tsx`（新）：訊息列渲染、送出 → `onSend` / agent 模式 → `onSubmitAgent`、模式切換、歷史抽屜開啟/載入/刪除、串流指示、`extraInputControls` 插槽 render。
- `AiPanel*.test.tsx` 既有全綠（回歸關卡）；必要時小幅更新選擇器。
- `useRemoteAiChat.test.ts`（新）：mock `invokeAiChatCtx` + `listen`；`send()` 呼叫 IPC、`ai-stream`（`kind:"chat"` && `session_id===connId`）delta 進 `streamBuf`、done → assistant 訊息；wrong id / `kind:"query"` 忽略；session 存進 `aiterm-remote-chat-sessions`；`clear` / `deleteSession`。
- `RemoteAiPanel.test.tsx`（新）：`submitAgent` → `invokeAiChatCtx`；回覆含 `<cmd>` → 呼叫傳入的 `submitCommand`；`block` 完成 → 下一輪帶輸出；無 `<cmd>` → 收尾（最後 assistant 訊息＝總結）；`abort()` 停迴圈；唯讀早退；逾時訊息。
- `RemoteTerminalView/index.test.tsx` 更新：`AgentPanel` → `RemoteAiPanel`；Ask AI 開面板、唯讀 disabled；`/agent` → `submitAgent`、`/ai` → `send`、都不把原字串送 `submitCommand`；resync/control-lost/ended → `abort` 被呼叫。移除 `AgentPanel.test.tsx`。

### 手動
- 本機 AI 面板（附件、MCP、agent 模式、對話歷史）行為不變。
- 遠端連線後 Ask AI 開新面板：`/agent` 多步任務跑完有總結、`/ai` 純問答、切 provider、開對話歷史載入/刪除、重連時任務中止、唯讀時停用。

## 明確不做（YAGNI）

- 圖片附件、`QuotaBadge`、指令卡住偵測（`STUCK_IDLE_MS`）在遠端面板——留待後續。
- 遠端 MCP 橋接。
- per-conversation provider override（provider badge 沿用 `AiPanel`：開 `ProviderPalette` 換全域預設）。
- `hostPlatform` 更細的 OS 分類（macOS host 仍標 `linux`，見上一里程碑；`buildRemoteCtx` 已有註解）。
- 不動本機 `AiPanel` 的邏輯，只抽殼層。

## 驗證指令

```bash
npm run test
npm run lint
npx tsc -b
cd src-tauri && cargo test
```
