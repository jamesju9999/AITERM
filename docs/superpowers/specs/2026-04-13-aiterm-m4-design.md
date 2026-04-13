# AITerm M4 — AI Panel + 多輪對話 設計文件

**日期**：2026-04-13
**里程碑**：M4
**前置**：M3（執行模式、串流、Provider Router、Ring Buffer）已完成

## 1. 目標與範圍

M4 為 AITerm 加入 **AI Panel**：一個可用 `Ctrl+I` 開關的右側面板，提供與 AI 的**多輪對話**能力。AI 回覆中可以嵌入 `<cmd>...</cmd>` 標籤，使用者一鍵即可把建議命令送到當前 terminal 執行。

對應 `docs/superpowers/specs/2026-04-10-aiterm-design.md` 第 11 節的 M4 定義：
> AI 面板 + 多輪對話 — Ctrl+I 面板、串流顯示、多輪對話、`<cmd>` 解析 | 多輪 AI 對話 + 一鍵執行建議命令

### 1.1 In scope

- `Ctrl+I` 開關右側覆蓋式 AI Panel
- 多輪對話（對話狀態綁定 terminal session，session 結束即消失）
- 串流顯示 AI 回覆（重用 M3 的 `ai-stream` event 機制）
- `<cmd>...</cmd>` 標籤解析與一鍵執行（單行直送；多行跳 confirm）
- 每 session 獨立對話歷史，上限 20 則訊息（user + assistant 合計）
- `🗑 New Chat` 按鈕清空當前 session 對話
- Panel 頂部顯示當前 provider badge，點擊開啟現有 ProviderPalette
- 錯誤處理：失敗時保留 user 訊息，提供 🔄 重試按鈕

### 1.2 Out of scope（M4 不做）

- 跨 session 共享的全域對話歷史（M4 綁 session）
- 持久化對話到檔案（重啟即消失）
- 主動取消正在進行的 backend streaming（留給 M5+，需要 CancellationToken）
- 重新注入 terminal 輸出到對話中（M3 的 ring buffer 已經提供 `recent_output` 作 context）
- 對話 export / 分享
- Chat 模式下的結構化 JSON 回覆（Chat 用自由文字 + `<cmd>` 標籤，非 JSON schema）
- 多行 `<cmd>` 的 shell parsing / 語法驗證

## 2. 架構概觀

```
┌─────────────────────────────────────────────────────────┐
│ TerminalView                                            │
│ ┌─────────────────────────────┐ ┌────────────────────┐  │
│ │ xterm.js (PTY)              │ │ AiPanel (new)      │  │
│ │                             │ │ ┌────────────────┐ │  │
│ │ (panelOpen=true 時不接收    │ │ │ Provider badge │ │  │
│ │  鍵盤輸入)                  │ │ │ 🗑 New Chat    │ │  │
│ │                             │ │ ├────────────────┤ │  │
│ │                             │ │ │ Messages list  │ │  │
│ │                             │ │ │ (stream + cmd) │ │  │
│ │                             │ │ ├────────────────┤ │  │
│ │                             │ │ │ Input + Send   │ │  │
│ └─────────────────────────────┘ └────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

**狀態所有權**：採 **Frontend-stateful** 架構，與現有 `ai_query` 的 stateless backend 設計一致。

- **前端**：`AiPanel` 元件的 React state 擁有 `messages: ChatMessage[]`，每次呼叫 `ai_chat` 時送完整歷史給後端
- **後端**：`ai_chat` Tauri command 完全無狀態，僅負責呼叫 provider、emit 串流 event、回傳完整 content
- **Session 綁定**：`AiPanel` 以 `key={sessionId}` 掛載，切 session 自動 unmount/remount，state 隨 React component 生命週期自然消亡

## 3. 後端設計

### 3.1 新 Tauri Command：`ai_chat`

位置：`src-tauri/src/commands/ai.rs`

```rust
#[tauri::command]
pub async fn ai_chat(
    messages: Vec<ChatMessage>,   // 前端送完整歷史（已截斷至 ≤20 則）
    session_id: String,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiChatReply, AiError>;

#[derive(Serialize)]
pub struct AiChatReply {
    pub content: String,         // 完整的 assistant 文字（含 <cmd> 標籤）
}
```

### 3.2 與 `ai_query` 的差異

| 面向 | `ai_query` (M3) | `ai_chat` (M4) |
|---|---|---|
| `QueryMode` | `SingleCommand` | `Chat` |
| System prompt | `build_single_command_prompt` | `build_chat_prompt`（新） |
| 回傳型別 | `AiCommandReady` | `AiChatReply` |
| Response parsing | `serde_json::from_str` 強制結構 | **無** parsing，直接回傳 `buf` |
| `max_tokens` | 512 | 1024 |

### 3.3 Chat System Prompt

新函式 `build_chat_prompt(&EnvSnapshot) -> String`：

```
You are an AI terminal assistant. The user is in an interactive terminal
session and you can see their OS, shell, cwd, and recent output.

Environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}
{recent_output_section}

Rules:
1. Respond in Traditional Chinese (繁體中文).
2. When you want to suggest a runnable shell command, wrap it in
   <cmd>...</cmd> tags. The user can click the tag to execute it.
3. You may include multiple <cmd> tags in one reply if needed.
4. Each <cmd> must contain a command valid for {shell}. Prefer single-line
   commands; multi-line commands will ask the user for confirmation before
   executing.
5. Free-form explanation outside <cmd> tags is encouraged.
6. Never produce destructive operations against system roots unless the
   user explicitly asks; if you do, mark it clearly in prose.
```

**關鍵差異**：**不**教 AI 輸出 JSON，因為 Chat 的回覆是自由對話文字，`<cmd>` 解析放在前端（regex）。

### 3.4 串流事件擴充

重用 M3 既有的 `ai-stream` Tauri event，payload 新增 `kind` 欄位以區分 `/ai` 與 AI Panel：

```rust
#[derive(Serialize, Clone)]
pub struct AiStreamEvent {
    pub session_id: String,
    pub kind: AiStreamKind,      // 新增
    pub delta: String,
    pub done: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum AiStreamKind {
    Query,   // /ai command
    Chat,    // AiPanel
}
```

既有的 `ai_query` 要更新：emit event 時帶 `kind: AiStreamKind::Query`。`CommandPreview` 元件的 listener 要 filter `kind === "query"`。

### 3.5 `ai_chat` 實作骨架

```rust
#[tauri::command]
pub async fn ai_chat(
    messages: Vec<ChatMessage>,
    session_id: String,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiChatReply, AiError> {
    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = router.resolve()?;
    let prompt = build_chat_prompt(&snapshot);

    let req = GenerateRequest {
        system_prompt: prompt,
        messages,                      // 直接用前端送來的完整歷史
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: Some(1024),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        let _ = app.emit("ai-stream", AiStreamEvent {
            session_id: session_id.clone(),
            kind: AiStreamKind::Chat,
            delta: chunk.delta.clone(),
            done: chunk.done,
        });
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    Ok(AiChatReply { content: buf })
}
```

**注意**：無 JSON parsing、無 structured result 驗證，buf 直接當 `content` 回傳。

### 3.6 註冊 command

`src-tauri/src/lib.rs` 的 `generate_handler!` 加入 `ai_chat`。

## 4. 前端設計

### 4.1 檔案配置

```
src/
├── components/
│   ├── TerminalView.tsx          (existing, 修改)
│   ├── AiPanel/                   (新)
│   │   ├── index.tsx              ← 主元件
│   │   ├── MessageList.tsx        ← 渲染歷史訊息
│   │   ├── MessageBubble.tsx      ← 單則訊息（含 <cmd> 渲染）
│   │   ├── CmdTag.tsx             ← <cmd> 可點擊按鈕
│   │   └── styles.css             ← panel 樣式（右側覆蓋層）
│   └── CommandPreview.tsx         (existing, listener filter 加 kind=query)
├── hooks/
│   └── useAiChat.ts               (新) ← state + invoke + event listener
├── lib/
│   ├── chatHistory.ts             (新) ← truncateHistory 純函式
│   └── cmdParser.ts               (新) ← <cmd>...</cmd> regex 解析
└── ipc/
    └── ai.ts                      (existing, 加 aiChat helper)
```

### 4.2 `useAiChat` hook

```typescript
// src/hooks/useAiChat.ts
interface UseAiChatResult {
  messages: ChatMessage[];
  streamBuf: string;           // 當前串流中的 assistant chunk
  isStreaming: boolean;
  error: AiError | null;
  send: (userText: string) => Promise<void>;
  resend: () => Promise<void>; // 失敗後重試，不重複 append user
  clear: () => void;           // 🗑 New Chat
}

export function useAiChat(sessionId: string): UseAiChatResult {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streamBuf, setStreamBuf] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<AiError | null>(null);

  useEffect(() => {
    const unlisten = listen<AiStreamEvent>("ai-stream", (e) => {
      if (e.payload.kind !== "chat") return;
      if (e.payload.session_id !== sessionId) return;
      if (e.payload.done) return;
      setStreamBuf(prev => prev + e.payload.delta);
    });
    return () => { unlisten.then(f => f()); };
  }, [sessionId]);

  const invokeChat = async (msgs: ChatMessage[]) => {
    setStreamBuf("");
    setIsStreaming(true);
    setError(null);
    try {
      const reply = await aiChat(msgs, sessionId);
      setMessages([...msgs, { role: "assistant", content: reply.content }]);
    } catch (e) {
      setError(e as AiError);
      // 不 rollback msgs — user 訊息保留，UI 顯示錯誤氣泡
    } finally {
      setStreamBuf("");
      setIsStreaming(false);
    }
  };

  const send = async (userText: string) => {
    const userMsg: ChatMessage = { role: "user", content: userText };
    const next = truncateHistory([...messages, userMsg], 20);
    setMessages(next);
    await invokeChat(next);
  };

  const resend = async () => {
    if (messages.length === 0) return;
    if (messages[messages.length - 1].role !== "user") return;
    await invokeChat(messages);
  };

  const clear = () => { setMessages([]); setError(null); setStreamBuf(""); };

  return { messages, streamBuf, isStreaming, error, send, resend, clear };
}
```

### 4.3 `truncateHistory`

```typescript
// src/lib/chatHistory.ts
export function truncateHistory(msgs: ChatMessage[], limit: number): ChatMessage[] {
  if (limit <= 0) return [];
  if (msgs.length <= limit) return msgs;
  return msgs.slice(msgs.length - limit);
}
```

**規則**：`limit = 20` 是 **user + assistant 合計 20 則訊息**（約 10 輪對話）。保留**最後** N 則，最舊的被截斷。

### 4.4 `<cmd>` 解析

```typescript
// src/lib/cmdParser.ts
export interface CmdPart {
  type: "text" | "cmd";
  content: string;
  multiline?: boolean; // 僅 cmd 有意義
}

export function parseCmdTags(text: string): CmdPart[] {
  const parts: CmdPart[] = [];
  const regex = /<cmd>([\s\S]*?)<\/cmd>/g;
  let lastIdx = 0;
  let match: RegExpExecArray | null;
  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIdx) {
      parts.push({ type: "text", content: text.slice(lastIdx, match.index) });
    }
    const cmd = match[1].trim();
    parts.push({ type: "cmd", content: cmd, multiline: cmd.includes("\n") });
    lastIdx = match.index + match[0].length;
  }
  if (lastIdx < text.length) {
    parts.push({ type: "text", content: text.slice(lastIdx) });
  }
  return parts;
}
```

**邊界行為**：
- 未閉合 `<cmd>ls` → regex 不匹配 → 整段當純文字
- 多個 `<cmd>` → 全部抽出
- 嵌套 `<cmd>a<cmd>b</cmd></cmd>` → 非貪婪取內層 `<cmd>b</cmd>`
- React `{content}` 自動 escape，不會 XSS

### 4.5 `AiPanel` 主元件

```typescript
interface AiPanelProps {
  sessionId: string;
  isOpen: boolean;
  onClose: () => void;
  onExecuteCommand: (cmd: string) => void;
}
```

**Key 行為**：
- 以 `key={sessionId}` 由 `TerminalView` 掛載 → 切 session 自動 remount，state reset
- `isOpen=true` 時 autoFocus 到 textarea
- 監聽 `Escape` 鍵 → 呼叫 `onClose()`
- 結構：
  - 頂部 bar：Provider badge（點擊開 ProviderPalette）+ 🗑 New Chat 按鈕
  - 中間：`MessageList`（渲染 `messages`，串流中最後插一個 streaming bubble 顯示 `streamBuf`）
  - 底部：`textarea` + Send 按鈕（串流中 disabled；🗑 New Chat 也 disabled）
  - 錯誤狀態：最後一則 user 訊息後插入紅色錯誤氣泡 + 🔄 重試按鈕

### 4.6 `CmdTag` 元件

```typescript
interface CmdTagProps {
  command: string;
  multiline: boolean;
  onExec: (cmd: string) => void;
}

function CmdTag({ command, multiline, onExec }: CmdTagProps) {
  const handleClick = () => {
    if (multiline) {
      if (!window.confirm(`確定執行多行命令？\n\n${command}`)) return;
    }
    onExec(command);
  };
  return (
    <button className="cmd-tag" onClick={handleClick}>
      <code>{command}</code>
      <span className="play">▶</span>
    </button>
  );
}
```

**規則**：
- **單行命令**：點擊直接 `onExec(command)`，無 confirm
- **多行命令**：點擊先 `window.confirm()` 顯示完整命令，確認後才執行

### 4.7 `TerminalView` 改動

```typescript
const [panelOpen, setPanelOpen] = useState(false);

useKeyboardShortcut("Ctrl+I", () => setPanelOpen(o => !o));

return (
  <div className="terminal-view">
    <Xterm ... disabled={panelOpen} />
    <AiPanel
      key={sessionId}
      sessionId={sessionId}
      isOpen={panelOpen}
      onClose={() => setPanelOpen(false)}
      onExecuteCommand={(cmd) => ptyWrite(sessionId, cmd + "\n")}
    />
  </div>
);
```

**`disabled={panelOpen}`**：xterm 鍵盤 handler 在 panel 打開時要 detach。如果 xterm.js API 無法動態 detach，fallback 為在 xterm 上罩一層透明 div 吃掉事件。實作時決定。

## 5. 資料流（一輪完整對話）

```
1. 使用者在 AiPanel textarea 打字 → 按 Enter 或 Send 按鈕
2. AiPanel.handleSend():
   a. 呼叫 useAiChat.send(userText)
3. useAiChat.send():
   a. next = truncateHistory([...messages, newUserMsg], 20)
   b. setMessages(next)
   c. invokeChat(next)
4. invokeChat():
   a. setStreamBuf(""); setIsStreaming(true); setError(null)
   b. invoke("ai_chat", { messages: next, sessionId })
5. 後端 ai_chat:
   a. 取 PtyManager 的 recent_output ring buffer
   b. 組 chat system prompt
   c. router.resolve() 取 provider
   d. 開 channel, spawn generate task
   e. 每個 chunk emit ai-stream event (kind=chat) 並累積進 buf
   f. 完成後回傳 AiChatReply { content: buf }
6. 前端 ai-stream listener (kind=chat, session match):
   - 每個 delta → setStreamBuf(prev + delta)
   - done=true → ignore (等 invoke 結果)
7. invoke Promise resolve:
   a. setMessages([...next, { role: "assistant", content: reply.content }])
   b. setStreamBuf("")
   c. setIsStreaming(false)
8. MessageBubble 渲染 assistant 訊息:
   - parseCmdTags(content) → 交錯渲染 text span / CmdTag button
9. 使用者點 CmdTag → onExecuteCommand(cmd) → ptyWrite(session, cmd + "\n")
10. PTY 執行命令，輸出進 xterm，也進 ring buffer
11. 下一輪對話自動帶上更新後的 recent_output（透過 M3 context snapshot）
```

## 6. 錯誤處理

| 錯誤 | 行為 |
|---|---|
| `NotConfigured` | 訊息列紅色系統訊息 + 提示點 provider badge 開 palette |
| `Network` | user 訊息保留，顯示錯誤氣泡「⚠ 網路錯誤：{message}」+ 🔄 重試按鈕 |
| `AuthFailed` | 同上，訊息提示「請檢查 API Key」 |
| `RateLimit` | 同上，顯示「⏳ 速率限制，請 {retry_after}s 後重試」 |
| `ModelError` | Chat 模式不做 JSON parsing → 理論上不發生；若後端改動後發生，當一般 error 處理 |

**重試語意**：`resend()` **不**重新 append user 訊息，直接拿現有 `messages`（最後一則就是上次失敗的 user）再 invoke 一次 `ai_chat`。使用者不用重打字。

**Provider 切換中**：正在跑的 `ai_chat` 用舊 provider 跑完，下一則用新 provider。M4 不中斷串流。

**串流中 UI 鎖定**：`isStreaming === true` 時 textarea、Send button、**🗑 New Chat button 全部 disabled**。原因：若串流中允許 clear，`invokeChat` 的 closure 捕獲的 `msgs` 會在 resolve 時覆寫清空後的狀態，產生 race condition。統一鎖定是最乾淨的作法。

**關閉 panel 時串流仍進行中**：Panel 只是 CSS 隱藏，`useAiChat` hook 還活著，串流繼續完成，重新打開即看到完整結果。

**切 session 時串流仍進行中**：`AiPanel` component unmount → `useAiChat` cleanup → listener 斷開 → 後端 `ai_chat` 繼續跑完（浪費 token 但不崩）。M4 不做主動取消，留給 M5+。

## 7. 測試策略

### 7.1 後端

**單元測試**（`src-tauri/src/commands/ai.rs` 的 `mod tests`）：
1. `build_chat_prompt_contains_environment_fields`
2. `build_chat_prompt_includes_recent_output_when_present`
3. `build_chat_prompt_instructs_cmd_tag_format`
4. `build_chat_prompt_omits_json_schema`

**Integration 測試**（新檔 `src-tauri/tests/ai_chat_integration.rs`，仿 `ai_query.rs`）：
1. `chat_happy_path_streams_and_returns_content` — mock provider，驗證完整 content（含 `<cmd>`）
2. `chat_sends_full_message_history` — 送多則歷史，驗證後端 request body 含完整陣列
3. `chat_emits_stream_events_with_kind_chat` — 驗證 event payload `kind === "chat"`
4. `chat_returns_raw_content_without_json_parsing` — mock 回非 JSON 文字，驗證**不**回 `ModelError`
5. `chat_propagates_network_error` — mock 500，驗證 `AiError::Network`

### 7.2 前端

**純函式測試**：
- `src/lib/chatHistory.test.ts`：`truncateHistory` 邊界（空、≤limit、>limit、limit=0）
- `src/lib/cmdParser.test.ts`：純文字、單 cmd、多 cmd、未閉合、多行、嵌套

**Hook 測試**（`src/hooks/useAiChat.test.ts`，用 `@testing-library/react`）：
1. `send` 先 append user → 再 append assistant
2. 失敗時 user 保留，`error` 設定
3. `resend` 不重複 append，直接重送
4. `clear` 清空 messages + error + streamBuf
5. Stream event 更新 `streamBuf`

**元件測試**：
- `src/components/AiPanel/CmdTag.test.tsx`：單行點擊無 confirm；多行點擊跳 confirm；confirm 取消不執行
- `src/components/AiPanel/MessageBubble.test.tsx`：user/assistant/error 三種 bubble 渲染
- `src/components/AiPanel/index.test.tsx`：isOpen、Escape、autofocus、disabled、clear、key 重置

### 7.3 手動測試（Golden Path）

1. `Ctrl+I` → panel 從右側滑出，輸入框 autofocus
2. 打「請列出目錄」→ 送出 → 即時串流 → assistant 訊息含 `<cmd>ls</cmd>` 按鈕
3. 點 `ls` → 直接執行，terminal 顯示輸出
4. 第二輪「現在顯示詳細資訊」→ AI 回覆 `<cmd>ls -la</cmd>`（多輪 context 有效）
5. 🗑 New Chat → 清空 → 重新對話
6. 切 tab 到另一個 session → 打開 panel → 空歷史（每 session 獨立）
7. 回原 tab → 打開 panel → 原歷史還在
8. Esc 關 panel → 鍵盤回到 terminal，打字正常
9. 暫停 Ollama → 送訊息 → 紅色錯誤氣泡 + 重試按鈕
10. 串流中切 provider → 當次不中斷，下次用新 provider

### 7.4 不寫自動化測試的部分

- Tauri event listener 跨 process 行為（靠 Rust integration test + 手動）
- xterm.js 鍵盤 detach 的瀏覽器行為（靠手動）
- Panel 動畫 / CSS layout（靠手動）

## 8. 開放問題與未來工作

- **主動取消串流**：M5+ 用 `CancellationToken` 讓使用者中途停止 AI 生成
- **對話持久化**：M5+ 可選擇把對話存 SQLite 或 JSON，重啟後 restore
- **跨 session 全域歷史**：設計層級變動大，M4 不做
- **`<cmd>` 內的 shell parsing**：M4 對多行命令只跳 confirm，不做語法檢查；未來可整合 `shellcheck` 類工具
- **Panel resize / 拖曳寬度**：M4 固定寬度，M5+ 再看

## 9. Milestone 檢核清單

M4 完成條件：
- [ ] `Ctrl+I` 開關 AI Panel，切換行為正確
- [ ] 多輪對話上下文有效（AI 能理解「上一個」「再來一次」）
- [ ] 串流即時顯示（不等完整結果才出現）
- [ ] `<cmd>` 標籤解析正確（單行、多行、多個、邊界 case）
- [ ] 單行 `<cmd>` 一鍵直送 PTY，多行跳 confirm
- [ ] 對話狀態綁 session，切 tab 隔離，session 關閉即消失
- [ ] 🗑 New Chat 清空當前對話
- [ ] Provider badge 顯示 + 點擊開 palette
- [ ] 錯誤時保留 user 訊息 + 提供 🔄 重試
- [ ] 上限 20 則訊息自動截斷
- [ ] 所有後端 / 前端單元測試通過
- [ ] 手動 golden path 10 項全通
