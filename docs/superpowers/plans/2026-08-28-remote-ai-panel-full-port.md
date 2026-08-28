# 遠端終端機完整移植 AiPanel 體驗 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把本機 `AiPanel` 的完整體驗（多輪對話、model 切換、串流、對話紀錄持久化、AI 總結、agent 迴圈）移植到觀看端遠端終端機分頁，取代輕量控制列 `AgentPanel`。

**Architecture:** 後端新增 `ai_chat_ctx`（吃 `RemoteCtx` 而非 PTY session，與 `ai_chat` 共用抽出的 `run_chat` 核心）。前端把 `AiPanel` 的純呈現層抽成共用 `ChatPanelShell`，`AiPanel` 與新的 `RemoteAiPanel` 各自 compose；`RemoteAiPanel` 配無 MCP 的 `useRemoteAiChat(connId)` hook + 移植自 `AiPanel` 的 `<cmd>` tag agent 迴圈。指令送出走觀看端既有 `useTerminalBlocks(connId).submitCommand`。孤兒 `ai_query_ctx` / `invokeAiQueryCtx` 移除；`agentLoop.ts` 保留給本機 `TerminalView`。

**Tech Stack:** React 19 + TypeScript（Vitest / RTL）、Rust + Tauri 2（tokio / wiremock 風格整合測試）、xterm.js。

**Spec:** `docs/superpowers/specs/2026-08-28-remote-ai-panel-full-port-design.md`

**分支:** 在 `feat/remote-ai-panel-port` 上實作（從 master `056d5e1` 或更新後的 master 切出）。

---

## 檔案結構

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/src/commands/ai.rs` | 修改 | 抽出 `run_chat`；`ai_chat` 改呼叫它；新增 `ai_chat_ctx`；移除 `ai_query_ctx` |
| `src-tauri/src/lib.rs` | 修改 | 註冊 `ai_chat_ctx`，移除 `ai_query_ctx` |
| `src-tauri/tests/ai_chat_ctx.rs` | 建立 | `ai_chat_ctx` / `run_chat` 整合測試 |
| `src-tauri/tests/ai_query_ctx.rs` | 刪除 | `ai_query_ctx` 已移除 |
| `src/ipc/ai.ts` | 修改 | 新增 `invokeAiChatCtx`；移除 `invokeAiQueryCtx`（`RemoteCtx` 型別保留） |
| `src/ipc/ai.test.ts` | 修改 | 移除 `invokeAiQueryCtx` 測試，加 `invokeAiChatCtx` 測試 |
| `src/types/chat.ts` | 建立 | `McpChatMessage` / `McpChatSession` 型別（三方共用） |
| `src/hooks/useMcpChat.ts` | 修改 | 型別改從 `../types/chat` re-export（不動邏輯） |
| `src/components/ChatPanel/styles.css` | 建立（git mv） | 從 `AiPanel/styles.css` 搬過來 |
| `src/components/AiPanel/styles.css` | 刪除（git mv 到上面） | — |
| `src/components/ChatPanel/ChatPanelShell.tsx` | 建立 | 從 `AiPanel` 抽出的純呈現殼層 |
| `src/components/ChatPanel/ChatPanelShell.test.tsx` | 建立 | 殼層元件測試 |
| `src/components/AiPanel/index.tsx` | 修改 | 變薄包裝：`useMcpChat` + 既有 `submitAgent` + MCP/附件 slot，compose `ChatPanelShell` |
| `src/hooks/useRemoteAiChat.ts` | 建立 | 無 MCP 的遠端對話 hook |
| `src/hooks/useRemoteAiChat.test.ts` | 建立 | hook 測試 |
| `src/components/RemoteTerminalView/RemoteAiPanel.tsx` | 建立 | 取代 AgentPanel；`useRemoteAiChat` + 移植的 agent 迴圈 + `ChatPanelShell` |
| `src/components/RemoteTerminalView/RemoteAiPanel.test.tsx` | 建立 | 元件測試 |
| `src/components/RemoteTerminalView/AgentPanel.tsx` | 刪除 | — |
| `src/components/RemoteTerminalView/AgentPanel.css` | 刪除 | — |
| `src/components/RemoteTerminalView/AgentPanel.test.tsx` | 刪除 | — |
| `src/components/RemoteTerminalView/index.tsx` | 修改 | 移除 agentLoop 接線，接上 `RemoteAiPanel`，`/agent`→submitAgent、`/ai`→send，provider 狀態 |
| `src/components/RemoteTerminalView/index.test.tsx` | 修改 | 更新測試 |
| `src/lib/i18n.ts` | 修改 | 清掉 `RemoteAiPanel` 用不到的 `remote_agent_*` 鍵 |
| `src/lib/i18n.remoteTerminal.test.ts` | 修改 | 對應更新 |

---

## Task 1: 後端 — 從 `ai_chat` 抽出 `run_chat`（純重構）

**Files:** Modify `src-tauri/src/commands/ai.rs`（`ai_chat` fn，目前約 383–553 行）

回歸關卡＝`cargo test --lib commands::ai` + 既有 `ai_chat` 整合測試。不改行為。

環境提醒：`cargo` 在 bash sandbox 下會卡（jobserver），跑 `cargo` 要 `dangerouslyDisableSandbox: true`。若 `externalBin` 缺檔先跑 `scripts/setup-uv-mac.sh`。

- [ ] **Step 1: 讀現況** — 讀 `src-tauri/src/commands/ai.rs` 的 `ai_chat`（383–553）與 `run_single_command`（約 231–318，當範本），確認要搬的是「provider resolve（by id / default）＋ `build_chat_prompt` ＋ 非 MCP 串流路徑（約 529–552）」。

- [ ] **Step 2: 新增私有 `run_chat`**，放在 `ai_chat` 之前：

```rust
/// `ai_chat` 與 `ai_chat_ctx` 共用的「非 MCP 串流」核心：resolve provider →
/// build_chat_prompt → provider.generate 串流 ai-stream(kind=Chat) → 回 AiChatReply。
/// 呼叫端負責先做訊息驗證（空 / 末角色）。`stream_id` 是 ai-stream 事件的
/// session_id 欄位（`ai_chat` 傳 PTY session id，`ai_chat_ctx` 傳 conn id）。
async fn run_chat(
    messages: Vec<ChatMessage>,
    snapshot: crate::ai::EnvSnapshot,
    provider_id: Option<String>,
    locale: Locale,
    router: &AiRouter,
    app: &AppHandle,
    stream_id: String,
) -> Result<AiChatReply, AiError> {
    let provider = match provider_id.as_deref() {
        Some(id) => router.resolve_by_id(id).await?,
        None => router.resolve().await?,
    };
    let prompt = build_chat_prompt(&snapshot, locale);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages,
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: None,
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        let _ = app.emit("ai-stream", AiStreamEvent {
            session_id: stream_id.clone(),
            kind: AiStreamKind::Chat,
            delta: chunk.delta.clone(),
            done: chunk.done,
            tokens: chunk.usage.map(|u| u.prompt + u.completion),
        });
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    Ok(AiChatReply {
        content: Some(buf),
        tool_calls: vec![],
        tool_calling_unsupported: false,
        tool_fallback_reason: None,
        raw_tool_calls: None,
    })
}
```

**驗證前先比對**：`run_chat` 的 provider-resolve / prompt / 串流三段必須跟 `ai_chat` 現有那三段**逐字等價**（欄位順序、`if chunk.done { break; }`、`join.await` 的 match arm、`AiChatReply` 全欄位）。若現有碼跟上面有出入，以**現有碼為準**。

- [ ] **Step 3: `ai_chat` 改呼叫 `run_chat`。** `ai_chat` 保留：空訊息 / 末角色驗證、`context::snapshot(&pty_manager, &session_id)`、整個 `if use_mcp && cfg.mcp_enabled { ... }` MCP 分支。MCP 分支 fall-through 之後（目前約 527 行「End MCP path」註解之後到函式結尾），把 529–552 那段換成：

```rust
    run_chat(messages, snapshot, provider_id, locale, &router, &app, session_id).await
```

（`messages` / `snapshot` / `provider_id` 是 `ai_chat` 現有的區域變數 / 參數；`snapshot` 目前在 410 行就 build 好了，維持。）

- [ ] **Step 4: 編譯 + 測試**

Run: `cd src-tauri && cargo test --lib commands::ai` （sandbox 關）
Expected: 全 PASS。

Run: `cd src-tauri && cargo test --test ai_chat_command 2>/dev/null || cargo test --test ai_chat 2>/dev/null || true`（先 `ls src-tauri/tests | grep -i chat` 找對檔名）
Expected: 既有 `ai_chat` 整合測試全 PASS。

Run: `cd src-tauri && cargo build`
Expected: exit 0。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "refactor(ai): 從 ai_chat 抽出 run_chat 供 ai_chat_ctx 共用"
```

---

## Task 2: 後端 — `ai_chat_ctx` 指令 + 移除 `ai_query_ctx`

**Files:** Modify `src-tauri/src/commands/ai.rs`、`src-tauri/src/lib.rs`；Create `src-tauri/tests/ai_chat_ctx.rs`；Delete `src-tauri/tests/ai_query_ctx.rs`

- [ ] **Step 1: 新增 `ai_chat_ctx`**（`src-tauri/src/commands/ai.rs`，放在 `ai_chat` 之後）：

```rust
/// `ai_chat` 的觀看端版本：不吃 PTY session_id，改吃明確的 `RemoteCtx`。
/// 無 MCP，所以不接受 `tool` 結尾的 history。`conn_id` 當 ai-stream 事件識別。
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

- [ ] **Step 2: 移除 `ai_query_ctx`** — 刪掉 `ai_query_ctx` 整個 fn（`src-tauri/src/commands/ai.rs`，上一里程碑加的，約在 `ai_query` 之後）。`RemoteCtx` struct、`context::snapshot_from_remote_ctx`、`run_single_command` **保留**（`ai_chat_ctx` / `ai_query` 各自要用）。

- [ ] **Step 3: `lib.rs`** — `use` 那行（約 35）把 `ai_query_ctx` 換成 `ai_chat_ctx`（結果：`ai::{agent_chat, ai_chat, ai_chat_ctx, ai_query}`）。`invoke_handler` 列表（約 303–305）把 `ai_query_ctx,` 那行換成 `ai_chat_ctx,`。

- [ ] **Step 4: `git rm src-tauri/tests/ai_query_ctx.rs`。** 它裡面對 `snapshot_from_remote_ctx` 的斷言若有價值，搬進新的 `ai_chat_ctx.rs` 或 `context.rs` 的 `#[cfg(test)] mod tests`（`snapshot_from_remote_ctx_handles_missing_fields` 這個測試要留著，二選一位置）。

- [ ] **Step 5: 建立 `src-tauri/tests/ai_chat_ctx.rs`**（照 `tests/ai_query_ctx.rs` 被刪前的 import 慣例——crate 名 `aiterm_lib`，`build_chat_prompt` 是 `pub`，`Locale::En` / `Locale::ZhTw`）：

```rust
//! ai_chat_ctx 的情境組裝：RemoteCtx 欄位進 build_chat_prompt，缺 shell/cwd degrade。
//! ai_chat_ctx 本身是 #[tauri::command]（要 AppHandle/State），無法在整合測試建構，
//! 所以測它的純建構塊：snapshot_from_remote_ctx + build_chat_prompt。

use aiterm_lib::ai::context::snapshot_from_remote_ctx;
use aiterm_lib::commands::ai::build_chat_prompt;
use aiterm_lib::ai::Locale;
use std::path::PathBuf;

#[test]
fn remote_ctx_fields_land_in_chat_prompt() {
    let snap = snapshot_from_remote_ctx(
        "linux", None, None,
        Some("user@host:~/proj$ ls\nCargo.toml  src/".into()),
    );
    let prompt = build_chat_prompt(&snap, Locale::En);
    assert!(prompt.contains("OS: linux"), "{prompt}");
    assert!(prompt.contains("~/proj$ ls"), "recent_output must reach the prompt: {prompt}");
    assert!(prompt.contains("Cwd: ."), "missing cwd degrades to \".\": {prompt}");
}

#[test]
fn known_fields_land_in_chat_prompt() {
    let snap = snapshot_from_remote_ctx("windows", Some("pwsh"), Some("C:\\src"), None);
    assert_eq!(snap.cwd, PathBuf::from("C:\\src"));
    let prompt = build_chat_prompt(&snap, Locale::ZhTw);
    assert!(prompt.contains("Shell: pwsh"));
    assert!(prompt.contains("C:\\src"));
}
```

> 若 `build_chat_prompt` 對 recent_output 的 section 標題文字跟 `build_single_command_prompt` 不同（讀 `ai.rs:146` 的 `build_chat_prompt`），把上面的斷言字串換成它實際輸出的（先跑一次 `cargo test --test ai_chat_ctx -- --nocapture` 看 panic 訊息裡的 prompt）。訊息驗證（空 / 末角色）在 `ai_chat_ctx` 的 `#[tauri::command]` 包裝裡，不好在整合測試打——可略，或在 `ai.rs` 的 `#[cfg(test)] mod tests` 加一個直接測那兩個 guard 的單元測試。

- [ ] **Step 6: 前端 `src/ipc/ai.ts`** — 移除 `invokeAiQueryCtx`（`RemoteCtx` interface 留著），在 `invokeAiChat` 之後加：

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

- [ ] **Step 7: `src/ipc/ai.test.ts`** — 移除 `describe("invokeAiQueryCtx")`，加：

```ts
describe("invokeAiChatCtx", () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue({ content: "hi", tool_calls: [], tool_calling_unsupported: false }));
  it("maps ctx to snake_case and passes connId/providerId/locale", async () => {
    await invokeAiChatCtx(
      [{ role: "user", content: "hi" }],
      { os: "linux", shell: null, cwd: null, recentOutput: "p$" },
      "conn-1", "prov-9", "en",
    );
    expect(invokeMock).toHaveBeenCalledWith("ai_chat_ctx", {
      messages: [{ role: "user", content: "hi" }],
      ctx: { os: "linux", shell: null, cwd: null, recent_output: "p$" },
      connId: "conn-1",
      providerId: "prov-9",
      locale: "en",
    });
  });
  it("defaults providerId to null", async () => {
    await invokeAiChatCtx([{ role: "user", content: "x" }], { os: "linux", shell: null, cwd: null, recentOutput: null }, "c", undefined, "zh-TW");
    expect(invokeMock.mock.calls[0][1].providerId).toBeNull();
  });
});
```

- [ ] **Step 8: 驗證**

Run: `cd src-tauri && cargo test --test ai_chat_ctx && cargo test --lib commands::ai && cargo build`（sandbox 關）
Expected: 全 PASS，build exit 0（確認 `invoke_handler` 註冊沒打錯）。

Run: `npm run test -- src/ipc/ai.test.ts && npx tsc -b`
Expected: PASS。

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/commands/ai.rs src-tauri/src/lib.rs src-tauri/tests/ai_chat_ctx.rs src/ipc/ai.ts src/ipc/ai.test.ts
git rm src-tauri/tests/ai_query_ctx.rs
git commit -m "feat(ai): 新增 ai_chat_ctx（吃 RemoteCtx 的多輪對話）；移除孤兒 ai_query_ctx"
```

---

## Task 3: 抽出共用型別 `src/types/chat.ts`

**Files:** Create `src/types/chat.ts`；Modify `src/hooks/useMcpChat.ts`

- [ ] **Step 1: 建立 `src/types/chat.ts`** — 把 `useMcpChat.ts` 的兩個 interface 原封搬過來：

```ts
import type { ContentPart } from "../ipc/ai";

export interface McpChatMessage {
  role: "user" | "assistant" | "tool_call" | "tool_result";
  content: string | ContentPart[];
  tool_name?: string;
  tool_call_id?: string;
  is_error?: boolean;
  is_loading?: boolean;
}

export interface McpChatSession {
  id: string;
  title: string;
  messages: McpChatMessage[];
  savedAt: number;
}
```

- [ ] **Step 2: `useMcpChat.ts`** — 刪掉本地的兩個 interface 定義，改成：

```ts
import type { McpChatMessage, McpChatSession } from "../types/chat";
export type { McpChatMessage, McpChatSession }; // re-export：既有 import { McpChatMessage } from "../hooks/useMcpChat" 不用改
```

（把 `import("../ipc/ai").ContentPart` 那個 inline 型別參照留在 `types/chat.ts` 用具名 import。）

- [ ] **Step 3: 驗證** — `npx tsc -b` → 0 errors；`grep -rn 'from "../hooks/useMcpChat"' src | grep -i "McpChat"` 確認既有 import 端（`AiPanel/MessageList.tsx` 等）仍解析得到。

- [ ] **Step 4: Commit**

```bash
git add src/types/chat.ts src/hooks/useMcpChat.ts
git commit -m "refactor(chat): 抽出 McpChatMessage/McpChatSession 到 types/chat.ts"
```

---

## Task 4: 抽出 `ChatPanelShell`（`AiPanel` 變薄包裝）

**Files:** `git mv src/components/AiPanel/styles.css src/components/ChatPanel/styles.css`；Create `src/components/ChatPanel/ChatPanelShell.tsx` + `ChatPanelShell.test.tsx`；Modify `src/components/AiPanel/index.tsx`（+ 所有 `import "./styles.css"` 的 sibling）

**回歸關卡：既有 `src/components/AiPanel/*.test.tsx` 全綠 + `npx tsc -b` + 手動開本機 AI 面板（附件、MCP、agent 模式、對話歷史）行為不變。** 這是本計畫風險最高的一步——只搬 JSX + 呈現 state，不改任何邏輯。

- [ ] **Step 1: CSS 搬家** — `git mv src/components/AiPanel/styles.css src/components/ChatPanel/styles.css`。`grep -rln '"./styles.css"\|AiPanel/styles.css' src/components/AiPanel` 找出所有 import 它的檔（`index.tsx`、`MessageList.tsx`、`MessageBubble.tsx`、`CmdTag.tsx`、`ModeHint.tsx` 等可能），把 import 路徑改成 `../ChatPanel/styles.css`。跑 `npm run test -- src/components/AiPanel` 確認沒壞。

- [ ] **Step 2: 建立 `src/components/ChatPanel/ChatPanelShell.tsx`。** 這個介面就是契約——把 `AiPanel/index.tsx` 的 render（約 498–792 行）搬過來，凡是「本機專屬 / 需要外部資料」的都改成讀 props：

```tsx
import { useState, useRef, useCallback, type KeyboardEvent, type ReactNode } from "react";
import type { AiError, ToolFallbackReason } from "../../ipc/ai";
import type { McpChatMessage, McpChatSession } from "../../types/chat";
import { MessageList } from "../AiPanel/MessageList";
import { ModeHint, type PanelMode } from "../AiPanel/ModeHint";
import { MaximizeIcon, MinimizeIcon, ZapIcon } from "../Icons";
import "./styles.css";

const MIN_WIDTH = 280;
const STORAGE_WIDTH_KEY = "aiterm-panel-width";

export interface ChatPanelShellProps {
  isOpen: boolean;
  onClose: () => void;

  // 對話資料
  messages: McpChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  thinkingLabel: string | null;
  error: AiError | string | null;
  onRetry: () => void;
  onExecuteCommand: (cmd: string) => void;

  // 送出 / 模式
  agentMode: boolean;
  onToggleAgentMode: () => void;
  onSend: (text: string) => void;        // 非 agent 模式送出
  onSubmitAgent: (text: string) => void; // agent 模式送出
  mode: PanelMode;                        // 給 ModeHint（由 wrapper 依 agentMode/mcp 算好）
  maxAgentSteps: number;
  mcpToolCount?: number;                  // ModeHint 用；remote 傳 0/省略

  // agent 執行狀態
  agentRunning: boolean;
  agentPhase: "thinking" | "running";
  agentStep: number;
  onAbortAgent: () => void;

  // provider
  providerName: string;
  onOpenProviderPalette: () => void;
  headerBadge?: ReactNode;               // QuotaBadge；remote 本版不傳

  // 對話歷史
  sessions: McpChatSession[];
  onLoadSession: (s: McpChatSession) => void;
  onNewChat: () => void;
  onDeleteSession: (id: string) => void;

  // 降級提示（本機 MCP 才有）
  toolFallbackReason?: ToolFallbackReason | null;

  // 插槽
  extraInputControls?: ReactNode;        // AiPanel 放 MCP 開關；RemoteAiPanel 不傳
  extraAboveInput?: ReactNode;           // AiPanel 放附件 pills + hidden file input + stuck prompt
  isWindows?: boolean;                   // AiPanel 傳 navigator 判斷；預設 false
  inputDisabled?: boolean;               // 唯讀等外部強制禁用
}

export function ChatPanelShell(props: ChatPanelShellProps) {
  const {
    isOpen, onClose, messages, streamBuf, isStreaming, thinkingLabel, error,
    onRetry, onExecuteCommand, agentMode, onToggleAgentMode, onSend, onSubmitAgent,
    mode, maxAgentSteps, mcpToolCount = 0, agentRunning, agentPhase, agentStep,
    onAbortAgent, providerName, onOpenProviderPalette, headerBadge, sessions,
    onLoadSession, onNewChat, onDeleteSession, toolFallbackReason,
    extraInputControls, extraAboveInput, isWindows = false, inputDisabled = false,
  } = props;

  const [input, setInput] = useState("");
  const [historyOpen, setHistoryOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [panelWidth, setPanelWidth] = useState(() => {
    try { const v = localStorage.getItem(STORAGE_WIDTH_KEY); if (v) return Math.max(MIN_WIDTH, parseInt(v, 10)); } catch { /* ignore */ }
    return 420;
  });
  const resizingRef = useRef(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const isDisabled = isStreaming || agentRunning || inputDisabled;

  const submit = useCallback(() => {
    const text = input.trim();
    if (!text || isDisabled) return;
    setInput("");
    if (agentMode) onSubmitAgent(text); else onSend(text);
  }, [input, isDisabled, agentMode, onSubmitAgent, onSend]);

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  // ...resize handle handlers (onResizePointerDown/Move/Up)：從 AiPanel/index.tsx 搬過來，
  //    寫回 localStorage(STORAGE_WIDTH_KEY)。

  // ── render ──
  // 把 AiPanel/index.tsx 498–792 的 JSX 貼進來，套用替換：
  //   chat.messages           → messages
  //   chat.streamBuf          → streamBuf
  //   chat.isStreaming        → isStreaming
  //   chat.error              → error
  //   chat.resend             → onRetry
  //   chat.sessions           → sessions
  //   chat.loadMessages(...)  → onLoadSession(s)（wrapper 內部再呼 loadMessages）
  //   chat.clear()            → onNewChat()
  //   chat.deleteSession(id)  → onDeleteSession(id)
  //   chat.toolFallbackReason → toolFallbackReason
  //   handleSubmit            → submit
  //   setAgentMode(m=>!m)     → onToggleAgentMode()
  //   submitAgent(text)       → （由 submit() 內部依 agentMode 分流）
  //   agentAbortRef/writePty  → onAbortAgent()（停止鈕只呼這個）
  //   IS_WINDOWS              → isWindows
  //   mcpToolCount            → mcpToolCount
  //   mode                    → mode
  //   附件 pills / hidden file input / stuck prompt → 不放這裡，改 {extraAboveInput}
  //   MCP toggle button        → 不放這裡，改 {extraInputControls}（放在 agent-toggle 旁）
  //   ModeHint 保留在殼層（generic），props: mode / maxAgentSteps / mcpToolCount
  //   agent-status 區塊保留在殼層，停止鈕 onClick={onAbortAgent}
  return (/* ... */);
}
```

> 實作者：讀 `AiPanel/index.tsx` 的 render，逐塊判斷「generic → 留殼層」還是「本機專屬 → slot」。generic：header（含 provider badge / expand / history / new chat / close）、resize handle、歷史側欄、`MessageList`、`ModeHint`、agent-status、輸入區的 textarea + send + agent-toggle。本機專屬（→ `extraAboveInput` / `extraInputControls`）：附件 pills、hidden `<input type=file>`、stuck prompt、MCP toggle、`toolFallbackReason` 那條 hint 可留殼層（接 optional prop）。

- [ ] **Step 3: `AiPanel/index.tsx` 改薄包裝。** 保留：`useMcpChat(sessionId)`、`submitAgent` / `runAgentLoop`（既有 agent 迴圈，不動）、附件 state + `processFiles` / `handlePaste` / `handleDrop` + attachment pills JSX、MCP state（`mcpEnabled` / `useMcp` / `mcpToolCount` / `getMcpTools`）+ MCP toggle JSX、stuck prompt state + JSX、`QuotaBadge`、`buildAgentSystemPrompt`、`agentMode` state + `STORAGE_AGENT_MODE_KEY` 持久化、`mode` 計算。render 變成：

```tsx
return (
  <ChatPanelShell
    isOpen={isOpen}
    onClose={onClose}
    messages={chat.messages}
    streamBuf={chat.streamBuf}
    isStreaming={chat.isStreaming || (agentRunning && agentPhase === "thinking")}
    thinkingLabel={/* 既有那段三元 */}
    error={chat.error}
    onRetry={chat.resend}
    onExecuteCommand={onExecuteCommand}
    agentMode={agentMode}
    onToggleAgentMode={() => setAgentMode((m) => !m)}
    onSend={(text) => void chat.send(text, mcpActive, undefined, attachments.length ? attachments : undefined)}
    onSubmitAgent={(text) => void submitAgent(text)}
    mode={mode}
    maxAgentSteps={maxAgentSteps}
    mcpToolCount={mcpToolCount}
    agentRunning={agentRunning}
    agentPhase={agentPhase}
    agentStep={agentStep}
    onAbortAgent={() => { agentAbortRef.current = true; writePty(sessionId, "\x03").catch(() => {}); }}
    providerName={providerName}
    onOpenProviderPalette={onOpenProviderPalette}
    headerBadge={quotaWindow ? <QuotaBadge window={quotaWindow} /> : undefined}
    sessions={chat.sessions}
    onLoadSession={(s) => chat.loadMessages(s.messages, s.id)}
    onNewChat={() => chat.clear()}
    onDeleteSession={(id) => chat.deleteSession(id)}
    toolFallbackReason={chat.toolFallbackReason}
    isWindows={IS_WINDOWS}
    extraInputControls={mcpEnabled ? (/* MCP toggle button JSX */) : null}
    extraAboveInput={<>
      {/* attachment pills JSX */}
      <input ref={fileInputRef} type="file" multiple style={{ display: "none" }} onChange={...} />
      {agentRunning && stuckPromptVisible && (/* stuck prompt JSX */)}
    </>}
  />
);
```

`onExecuteCommand` / `agentPhase` / `agentStep` / `agentRunning` / `mcpActive` 等都是 `AiPanel` 既有的變數，沿用。

- [ ] **Step 4: `ChatPanelShell.test.tsx`**：

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ChatPanelShell, type ChatPanelShellProps } from "./ChatPanelShell";

function base(over: Partial<ChatPanelShellProps> = {}): ChatPanelShellProps {
  return {
    isOpen: true, onClose: vi.fn(),
    messages: [], streamBuf: "", isStreaming: false, thinkingLabel: null, error: null,
    onRetry: vi.fn(), onExecuteCommand: vi.fn(),
    agentMode: false, onToggleAgentMode: vi.fn(), onSend: vi.fn(), onSubmitAgent: vi.fn(),
    mode: "chat" as never, maxAgentSteps: 5,
    agentRunning: false, agentPhase: "thinking", agentStep: 0, onAbortAgent: vi.fn(),
    providerName: "TestProv", onOpenProviderPalette: vi.fn(),
    sessions: [], onLoadSession: vi.fn(), onNewChat: vi.fn(), onDeleteSession: vi.fn(),
    ...over,
  };
}

describe("ChatPanelShell", () => {
  it("routes plain submit to onSend", async () => {
    const onSend = vi.fn();
    render(<ChatPanelShell {...base({ onSend })} />);
    await userEvent.type(screen.getByRole("textbox"), "hello");
    await userEvent.keyboard("{Enter}");
    expect(onSend).toHaveBeenCalledWith("hello");
  });
  it("routes submit to onSubmitAgent when agentMode", async () => {
    const onSubmitAgent = vi.fn();
    render(<ChatPanelShell {...base({ agentMode: true, onSubmitAgent })} />);
    await userEvent.type(screen.getByRole("textbox"), "do it{Enter}");
    expect(onSubmitAgent).toHaveBeenCalledWith("do it");
  });
  it("provider badge opens palette", async () => {
    const onOpenProviderPalette = vi.fn();
    render(<ChatPanelShell {...base({ onOpenProviderPalette })} />);
    await userEvent.click(screen.getByText("TestProv"));
    expect(onOpenProviderPalette).toHaveBeenCalled();
  });
  it("New Chat calls onNewChat", async () => {
    const onNewChat = vi.fn();
    render(<ChatPanelShell {...base({ onNewChat })} />);
    await userEvent.click(screen.getByTitle(/清空當前對話|New Chat/i));
    expect(onNewChat).toHaveBeenCalled();
  });
  it("shows the stop button and calls onAbortAgent while agentRunning", async () => {
    const onAbortAgent = vi.fn();
    render(<ChatPanelShell {...base({ agentRunning: true, onAbortAgent })} />);
    await userEvent.click(screen.getByTitle("停止"));
    expect(onAbortAgent).toHaveBeenCalled();
  });
  it("renders extraInputControls and extraAboveInput slots", () => {
    render(<ChatPanelShell {...base({
      extraInputControls: <button>MCP-SLOT</button>,
      extraAboveInput: <div>ABOVE-SLOT</div>,
    })} />);
    expect(screen.getByText("MCP-SLOT")).toBeInTheDocument();
    expect(screen.getByText("ABOVE-SLOT")).toBeInTheDocument();
  });
});
```

- [ ] **Step 5: 驗證**

Run: `npm run test -- src/components/AiPanel src/components/ChatPanel`
Expected: 既有 `AiPanel*.test.tsx` 全綠（同數量）、`ChatPanelShell.test.tsx` 全綠。

Run: `npx tsc -b && npm run lint`
Expected: PASS / 無新違規。

- [ ] **Step 6: Commit**

```bash
git add src/components/ChatPanel src/components/AiPanel
git commit -m "refactor(ai-panel): 抽出 ChatPanelShell 純呈現殼層，AiPanel 變薄包裝"
```

---

## Task 5: `useRemoteAiChat` hook

**Files:** Create `src/hooks/useRemoteAiChat.ts` + `useRemoteAiChat.test.ts`

- [ ] **Step 1: 建立 `src/hooks/useRemoteAiChat.ts`。** 拿 `useMcpChat.ts` 當底，做這些變更：
- `sendMessage` 內 `while (iterations < MAX_TOOL_ITERATIONS)` 的工具迴圈**整段拿掉**，換成單次呼叫：
  ```ts
  const reply = await invokeAiChatCtx(history, buildCtx(), connId, providerId, locale);
  if (!mountedRef.current) return;
  setMessages(prev => [...prev, { role: "assistant", content: reply.content ?? streamBufRef.current }]);
  ```
- `ai-stream` 監聽的 `event.payload.session_id !== sessionId` → `!== connId`；`event.payload.kind !== "chat"` 也要濾掉（`ai_chat_ctx` 只發 `kind: "chat"`）。
- localStorage key：`const SESSIONS_STORAGE_KEY = "aiterm-remote-chat-sessions";`
- 移除 `executeMcpTool` / `AiToolCall` / `toolFallbackReason` / `buildContentParts` / attachments 相關（`useRemoteAiChat` 不吃附件）。`send(text)` 簽名簡化成 `send(text: string)`。
- 參數：`useRemoteAiChat(connId: string, buildCtx: () => RemoteCtx)`。`providerId` / `locale` 從 `useLocale()` + `getConfig()`（或由呼叫端傳入 `providerId`——傾向讓 `RemoteAiPanel` 傳 `providerId`，hook 收第三參數 `providerId?: string`）。
- 保留：`messages` / `streamBuf` / `isStreaming`(=`isLoading`) / `error` / `sessions` / `addMessage` / `clear` / `loadMessages` / `deleteSession` / `resend` / auto-save effect / `mountedRef` / `currentSessionIdRef` / `formatSessionTitle`。
- 回傳物件：`{ messages, isStreaming, streamBuf, error, send, addMessage, clear, loadMessages, deleteSession, resend, sessions }`。

- [ ] **Step 2: `useRemoteAiChat.test.ts`**：

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

const invokeChatCtx = vi.fn();
vi.mock("../ipc/ai", async (imp) => ({ ...(await imp<any>()), invokeAiChatCtx: (...a: unknown[]) => invokeChatCtx(...a) }));

const listeners: Array<(e: unknown) => void> = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_n: string, cb: (e: unknown) => void) => { listeners.push(cb); return Promise.resolve(() => {}); },
}));
vi.mock("../contexts/LocaleContext", () => ({ useLocale: () => ({ t: { chat_empty: "empty" }, locale: "en" }) }));

import { useRemoteAiChat } from "./useRemoteAiChat";
const CTX = { os: "linux", shell: null, cwd: null, recentOutput: null };

beforeEach(() => { invokeChatCtx.mockReset().mockResolvedValue({ content: "answer", tool_calls: [], tool_calling_unsupported: false }); listeners.length = 0; localStorage.clear(); });

describe("useRemoteAiChat", () => {
  it("send() calls invokeAiChatCtx with connId and appends the assistant reply", async () => {
    const { result } = renderHook(() => useRemoteAiChat("conn-1", () => CTX));
    await act(async () => { await result.current.send("hi"); });
    expect(invokeChatCtx).toHaveBeenCalledWith(
      expect.arrayContaining([{ role: "user", content: "hi" }]), CTX, "conn-1", undefined, "en",
    );
    expect(result.current.messages.map(m => m.content)).toContain("answer");
  });

  it("accumulates ai-stream deltas for matching connId/kind into streamBuf", async () => {
    const { result } = renderHook(() => useRemoteAiChat("conn-1", () => CTX));
    await waitFor(() => expect(listeners.length).toBeGreaterThan(0));
    act(() => listeners[0]({ payload: { session_id: "conn-1", kind: "chat", delta: "par", done: false } }));
    act(() => listeners[0]({ payload: { session_id: "conn-1", kind: "chat", delta: "tial", done: false } }));
    expect(result.current.streamBuf).toBe("partial");
    // wrong id / kind ignored
    act(() => listeners[0]({ payload: { session_id: "other", kind: "chat", delta: "X", done: false } }));
    act(() => listeners[0]({ payload: { session_id: "conn-1", kind: "query", delta: "Y", done: false } }));
    expect(result.current.streamBuf).toBe("partial");
  });

  it("persists a session to aiterm-remote-chat-sessions", async () => {
    const { result } = renderHook(() => useRemoteAiChat("conn-1", () => CTX));
    await act(async () => { await result.current.send("remember me"); });
    const raw = localStorage.getItem("aiterm-remote-chat-sessions");
    expect(raw).toContain("remember me");
  });

  it("clear() empties messages", async () => {
    const { result } = renderHook(() => useRemoteAiChat("conn-1", () => CTX));
    await act(async () => { await result.current.send("x"); });
    act(() => result.current.clear());
    expect(result.current.messages).toEqual([]);
  });
});
```

- [ ] **Step 3: 驗證** — `npm run test -- src/hooks/useRemoteAiChat && npx tsc -b`。

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useRemoteAiChat.ts src/hooks/useRemoteAiChat.test.ts
git commit -m "feat(remote-terminal): useRemoteAiChat——無 MCP 的觀看端多輪對話 hook"
```

---

## Task 6: `RemoteAiPanel` 元件

**Files:** Create `src/components/RemoteTerminalView/RemoteAiPanel.tsx` + `RemoteAiPanel.test.tsx`

- [ ] **Step 1: 建立 `RemoteAiPanel.tsx`。** `forwardRef` + `useImperativeHandle` 曝控制方法給 `RemoteTerminalView`：

```tsx
import { forwardRef, useCallback, useImperativeHandle, useRef, useState } from "react";
import type { RemoteCtx } from "../../ipc/ai";
import { invokeAiChatCtx, formatAiError, type AiError, type ChatMessage as AiChatMessage } from "../../ipc/ai";
import type { TerminalBlock } from "../../hooks/useTerminalBlocks";
import { useRemoteAiChat } from "../../hooks/useRemoteAiChat";
import { ChatPanelShell } from "../ChatPanel/ChatPanelShell";
import { useLocale } from "../../contexts/LocaleContext";
import { languageDirective } from "../../lib/i18n";

export interface RemoteAiPanelHandle {
  submitAgent: (goal: string) => void;
  send: (text: string) => void;
  abort: () => void;
}

interface Props {
  connId: string;
  buildRemoteCtx: () => RemoteCtx;
  submitCommand: (cmd: string, onComplete?: (b: TerminalBlock) => void) => void;
  isControl: boolean;
  maxSteps: number;
  providerName: string;
  providerId?: string;
  onOpenProviderPalette: () => void;
  isOpen: boolean;
  onClose: () => void;
  /** RemoteTerminalView 的共享 abortRef（unmount / 連線事件會設 true）。 */
  sharedAbortRef: React.MutableRefObject<boolean>;
}

const AGENT_STEP_TIMEOUT_MS = 60_000;

export const RemoteAiPanel = forwardRef<RemoteAiPanelHandle, Props>(function RemoteAiPanel(props, ref) {
  const { connId, buildRemoteCtx, submitCommand, isControl, maxSteps, providerName, providerId, onOpenProviderPalette, isOpen, onClose, sharedAbortRef } = props;
  const { t, locale } = useLocale();
  const chat = useRemoteAiChat(connId, buildRemoteCtx, providerId);
  const [agentMode, setAgentMode] = useState(true); // 遠端預設 agent 模式
  const [agentRunning, setAgentRunning] = useState(false);
  const [agentPhase, setAgentPhase] = useState<"thinking" | "running">("thinking");
  const [agentStep, setAgentStep] = useState(0);
  const abortRef = useRef(false);
  const loopRef = useRef<(history: { role: "user" | "assistant"; content: string }[], sys: string, step: number) => Promise<void>>(async () => {});

  const buildSystemPrompt = useCallback(() => (
    `You are an AI operating a REMOTE terminal over a shared connection. ` +
    `You cannot see the working directory or a directory listing — if you need them, ` +
    `emit <cmd>pwd</cmd> or <cmd>ls</cmd> as your first step.\n` +
    `To run a shell command wrap it in <cmd>...</cmd> (one command). ` +
    `After each command you receive its output; keep going until the goal is met, then give a final ` +
    `explanation with NO more <cmd> tags. Never run destructive or irreversible operations (e.g. rm -rf /). ` +
    `Write all explanations in ${languageDirective(locale)}.`
  ), [locale]);

  const runLoop = useCallback(async (history: { role: "user" | "assistant"; content: string }[], sys: string, step: number): Promise<void> => {
    if (abortRef.current || sharedAbortRef.current) { setAgentRunning(false); return; }
    if (step >= maxSteps) {
      chat.addMessage({ role: "assistant", content: t.term_agent_max_steps(maxSteps) });
      setAgentRunning(false);
      return;
    }
    setAgentStep(step + 1);
    setAgentPhase("thinking");

    let reply: string;
    try {
      const msgs: AiChatMessage[] = [{ role: "system", content: sys }, ...history];
      const r = await invokeAiChatCtx(msgs, buildRemoteCtx(), connId, providerId, locale);
      reply = r.content ?? "";
    } catch (e) {
      const isAiErr = e != null && typeof e === "object" && "kind" in (e as object);
      chat.addMessage({ role: "assistant", content: `⚠ ${isAiErr ? formatAiError(e as AiError) : String(e)}` });
      setAgentRunning(false);
      return;
    }
    if (abortRef.current || sharedAbortRef.current) { setAgentRunning(false); return; }
    chat.addMessage({ role: "assistant", content: reply });

    const m = reply.match(/<cmd>([\s\S]*?)<\/cmd>/i);
    if (!m) { setAgentRunning(false); return; } // 收尾——最後這段 assistant 文字＝總結

    const cmd = m[1].trim();
    setAgentPhase("running");
    await new Promise<void>((resolve) => {
      let done = false;
      const to = setTimeout(() => {
        if (done) return;
        done = true;
        chat.addMessage({ role: "assistant", content: t.remote_agent_no_shell_integration });
        setAgentRunning(false);
        resolve();
      }, AGENT_STEP_TIMEOUT_MS);
      submitCommand(cmd, (block) => {
        if (done) return;
        done = true;
        clearTimeout(to);
        if (abortRef.current || sharedAbortRef.current) { setAgentRunning(false); resolve(); return; }
        const out = (block.rawOutput ?? "").slice(-2000);
        const next = [
          ...history,
          { role: "assistant" as const, content: reply },
          { role: "user" as const, content: `Command \`${cmd}\` finished (exit code ${block.exitCode ?? 0}).\nOutput:\n\`\`\`\n${out}\n\`\`\`\n\nContinue. If the goal is met, give your final explanation with no more <cmd> tags.` },
        ];
        resolve();
        void loopRef.current(next, sys, step + 1);
      });
    });
  }, [maxSteps, t, chat, connId, providerId, locale, buildRemoteCtx, submitCommand, sharedAbortRef]);
  loopRef.current = runLoop;

  const submitAgent = useCallback((goal: string) => {
    if (!isControl || agentRunning) return;
    abortRef.current = false;
    setAgentRunning(true);
    setAgentStep(0);
    chat.addMessage({ role: "user", content: goal });
    const prior = chat.messages
      .filter((x): x is typeof x & { role: "user" | "assistant" } => x.role === "user" || x.role === "assistant")
      .map((x) => ({ role: x.role, content: typeof x.content === "string" ? x.content : "" }));
    void runLoop([...prior, { role: "user", content: goal }], buildSystemPrompt(), 0);
  }, [isControl, agentRunning, chat, runLoop, buildSystemPrompt]);

  const send = useCallback((text: string) => {
    if (!isControl) return;
    void chat.send(text);
  }, [isControl, chat]);

  const abort = useCallback(() => { abortRef.current = true; setAgentRunning(false); }, []);

  useImperativeHandle(ref, () => ({ submitAgent, send, abort }), [submitAgent, send, abort]);

  return (
    <ChatPanelShell
      isOpen={isOpen}
      onClose={onClose}
      messages={chat.messages}
      streamBuf={chat.streamBuf}
      isStreaming={chat.isStreaming || (agentRunning && agentPhase === "thinking")}
      thinkingLabel={agentRunning ? (agentPhase === "thinking" ? t.ai_agent_thinking : t.ai_agent_executing) : (chat.isStreaming ? t.ai_thinking : null)}
      error={chat.error}
      onRetry={chat.resend}
      onExecuteCommand={(cmd) => submitCommand(cmd)}
      agentMode={agentMode}
      onToggleAgentMode={() => setAgentMode((x) => !x)}
      onSend={send}
      onSubmitAgent={submitAgent}
      mode={agentMode ? ("agent" as never) : ("chat" as never)}
      maxAgentSteps={maxSteps}
      agentRunning={agentRunning}
      agentPhase={agentPhase}
      agentStep={agentStep}
      onAbortAgent={abort}
      providerName={providerName}
      onOpenProviderPalette={onOpenProviderPalette}
      sessions={chat.sessions}
      onLoadSession={(s) => chat.loadMessages(s.messages, s.id)}
      onNewChat={() => chat.clear()}
      onDeleteSession={(id) => chat.deleteSession(id)}
      inputDisabled={!isControl}
    />
  );
});
```

> `mode` 給 `ModeHint` 的型別是 `PanelMode`（`AiPanel/ModeHint`）——用它實際的字面值（讀 `ModeHint.tsx`），別硬塞 `as never`；上面是占位。`t.remote_agent_no_shell_integration` / `t.ai_agent_*` / `t.ai_thinking` / `t.term_agent_max_steps` 都是既有鍵。

- [ ] **Step 2: `RemoteAiPanel.test.tsx`**：mock `../../hooks/useRemoteAiChat`（回一個可控的假 chat 物件，`addMessage` 記錄呼叫）、`../../ipc/ai` 的 `invokeAiChatCtx`。測：

```tsx
// 1. submitAgent → invokeAiChatCtx 被呼叫；回覆含 <cmd> → 傳入的 submitCommand 被呼叫該指令
// 2. submitCommand 的 onComplete(block) → 下一輪 invokeAiChatCtx 的 messages 帶到 block.rawOutput
// 3. 回覆不含 <cmd> → agentRunning 收掉、最後 assistant 訊息就是那段文字（總結）
// 4. abort() → 迴圈停：不再有新的 invokeAiChatCtx / submitCommand
// 5. isControl=false → submitAgent / send 早退（invokeAiChatCtx 沒被呼叫）
// 6. 每步逾時：submitCommand 的 onComplete 一直不觸發 → advanceTimersByTime(60000) → 對話出現 no_shell_integration 訊息、agentRunning 收掉
```

用 `vi.useFakeTimers()` 測第 6 條。假 `submitCommand` 用一個「手動觸發 onComplete」的實作（存起回呼，測試自己叫）。

- [ ] **Step 3: 驗證** — `npm run test -- src/components/RemoteTerminalView/RemoteAiPanel && npx tsc -b && npm run lint`。

- [ ] **Step 4: Commit**

```bash
git add src/components/RemoteTerminalView/RemoteAiPanel.tsx src/components/RemoteTerminalView/RemoteAiPanel.test.tsx
git commit -m "feat(remote-terminal): RemoteAiPanel——移植 AiPanel 的 chat-agent 迴圈到觀看端"
```

---

## Task 7: 接線 `RemoteTerminalView` + 移除 AgentPanel

**Files:** Modify `src/components/RemoteTerminalView/index.tsx` + `index.test.tsx`；Delete `AgentPanel.tsx` / `.css` / `.test.tsx`；Modify `src/lib/i18n.ts` + `i18n.remoteTerminal.test.ts`

- [ ] **Step 1: 移除 agentLoop 那套。** 依 `grep -n` 結果刪：
  - import：`useAgentMission`、`runAgentLoop`/`INITIAL_PREVIEW`/`PreviewState`（from `../../lib/agentLoop`）、`invokeAiQueryCtx`（`RemoteCtx` / `AiStreamEvent` 若 `RemoteAiPanel` 不需要就也刪；`RemoteCtx` `buildRemoteCtx` 的回傳型別要用，留）、`AgentPanel`
  - state/ref：`agentMission`/`startMission`/`stopMission`、`agentPhase`、`streamText`、`preview`、`streamingRef`、`executionModeRef`、`agentMissionRef`、`missionRunningRef`
  - fn：`startAgentMission`、`stopAgentMission`
  - render：`<AgentPanel .../>` 整塊
  - 既有的 `ai-stream` `listen` 監聽（現在 `useRemoteAiChat` 內部自己聽）
  - **保留**：`readRecentOutput`（模組層）、`buildRemoteCtx`（改掉回傳——它現在給 `RemoteAiPanel` 用，簽名不變）、`abortRef` + `useEffect(() => () => { abortRef.current = true }, [])`、`maxAgentStepsRef` + `getConfig` effect、`submitCommandRef`

- [ ] **Step 2: 加 provider 狀態**（比照 `TerminalView.tsx:226-231, 2040`）：

```ts
import { listProviders } from "../../ipc/provider";
import { ProviderPalette } from "../ProviderPalette";
// ...
const [activeProvider, setActiveProvider] = useState("");
const [activeProviderId, setActiveProviderId] = useState("");
const [paletteOpen, setPaletteOpen] = useState(false);
useEffect(() => {
  listProviders().then((list) => {
    const active = list.find((p) => p.is_default) ?? list[0];
    setActiveProvider(active?.display_name ?? "");
    setActiveProviderId(active?.id ?? "");
  }).catch(() => {});
}, []);
```

- [ ] **Step 3: 加 `RemoteAiPanel` 接線**：

```ts
import { RemoteAiPanel, type RemoteAiPanelHandle } from "./RemoteAiPanel";
// ...
const [aiPanelOpen, setAiPanelOpen] = useState(false);
const remoteAiPanelRef = useRef<RemoteAiPanelHandle>(null);
```

`handleWarpSubmit` 改寫：
```ts
const handleWarpSubmit = useCallback((cmd: string) => {
  const agentGoal = parseAgentPrefix(cmd);
  const aiGoal = parseAiPrefix(cmd);
  if (agentGoal !== null) { setAiPanelOpen(true); remoteAiPanelRef.current?.submitAgent(agentGoal); return; }
  if (aiGoal !== null)   { setAiPanelOpen(true); remoteAiPanelRef.current?.send(aiGoal); return; }
  submitCommand(cmd);
}, [submitCommand]);
```

Ask AI 鈕 `onClick` → `setAiPanelOpen(true)`（`disabled` 判斷不變）。

連線事件：在 `onShareViewerResync` / `onShareViewerControlChanged`（`mode !== "control"`）/ `onShareViewerEnded` 的 handler 內，把原本設 `agentMission` 相關的那幾行換成：
```ts
abortRef.current = true;
remoteAiPanelRef.current?.abort();
```
（`abortRef` 還是那顆 unmount effect 用的；`RemoteAiPanel` 收 `sharedAbortRef={abortRef}` 會看到。）

render（`WarpInput` 之後）：
```tsx
<RemoteAiPanel
  ref={remoteAiPanelRef}
  isOpen={aiPanelOpen}
  onClose={() => setAiPanelOpen(false)}
  connId={connId}
  buildRemoteCtx={buildRemoteCtx}
  submitCommand={(c, cb) => submitCommandRef.current(c, cb)}
  isControl={phase.kind === "live" && phase.mode === "control"}
  maxSteps={maxAgentStepsRef.current}
  providerName={activeProvider}
  providerId={activeProviderId}
  onOpenProviderPalette={() => setPaletteOpen(true)}
  sharedAbortRef={abortRef}
/>
{paletteOpen && (
  <ProviderPalette
    onClose={() => setPaletteOpen(false)}
    onSelect={() => { setPaletteOpen(false); /* re-fetch providers */ listProviders().then(/* 同上 */); }}
  />
)}
```
（`ProviderPalette` 的實際 props 讀 `src/components/ProviderPalette.tsx` / `TerminalView` 呼叫處對齊。）

- [ ] **Step 4: 刪檔** — `git rm src/components/RemoteTerminalView/AgentPanel.tsx src/components/RemoteTerminalView/AgentPanel.css src/components/RemoteTerminalView/AgentPanel.test.tsx`。

- [ ] **Step 5: i18n 清理** — `grep -rn "remote_agent_" src/components/RemoteTerminalView src/components/ChatPanel` 看 `RemoteAiPanel` / `ChatPanelShell` 實際還用哪些 `remote_agent_*` 鍵。用不到的（`remote_agent_panel_title` / `remote_agent_stop` / `remote_agent_goal_placeholder` / `remote_agent_output` 若還在 / `remote_agent_done` 之類控制列專用）從 `src/lib/i18n.ts` 的 `zhTW` 與 `enRaw` 兩塊刪掉。**保留**：`remote_agent_no_shell_integration`、`remote_agent_needs_control`（若 Ask AI 鈕 tooltip 還用）、`remote_agent_aborted_*`（連線事件訊息若 `RemoteAiPanel.abort` 有用到就留，沒用到就刪）。更新 `src/lib/i18n.remoteTerminal.test.ts`（多半是泛前綴掃描，移除鍵後自然綠——先跑一次）。

- [ ] **Step 6: 更新 `src/components/RemoteTerminalView/index.test.tsx`** — 沿用既有 `handlers` / `captureHandler` / render helper 樣板。
  - `vi.mock("./RemoteAiPanel")`：回一個假元件，並讓測試能拿到最後一次 render 的 props（或 forwardRef 的 handle）——例如 mock 成把 `ref.current = { submitAgent: submitAgentSpy, send: sendSpy, abort: abortSpy }` 的實作。
  - `vi.mock("../../ipc/provider")`：`listProviders` 回固定陣列。
  - 移除舊的 AgentPanel 相關測試。
  - 新增：
    1. Ask AI 鈕 control 模式點了 → `aiPanelOpen` 為真（假 `RemoteAiPanel` 收到 `isOpen`）；read_only 時 `disabled`。
    2. `/agent tidy up` 從 WarpInput 送出 → `submitAgentSpy("tidy up")` 被呼叫、`sendMock`（shareViewerSend）**沒**收到 `/agent tidy up`。
    3. `/ai what is this` → `sendSpy("what is this")`。
    4. 觸發 `control` handler `read_only` / `ended` handler → `abortSpy` 被呼叫。

- [ ] **Step 7: 驗證**

Run: `npm run test -- src/components/RemoteTerminalView src/components/ChatPanel src/components/AiPanel src/hooks/useRemoteAiChat src/lib/i18n`
Expected: 全 PASS。

Run: `npx tsc -b && npm run lint`
Expected: PASS / 無新違規。

- [ ] **Step 8: Commit**

```bash
git add src/components/RemoteTerminalView src/lib/i18n.ts src/lib/i18n.remoteTerminal.test.ts
git rm src/components/RemoteTerminalView/AgentPanel.tsx src/components/RemoteTerminalView/AgentPanel.css src/components/RemoteTerminalView/AgentPanel.test.tsx
git commit -m "feat(remote-terminal): 觀看端改用 RemoteAiPanel，移除 AgentPanel 與 agentLoop 接線"
```

---

## Task 8: 完整驗證與手動冒煙

- [ ] **Step 1: 前端全套** — `npm run test`（Task 1 前 1079，本計畫新增測試後應更多，全綠）
- [ ] **Step 2: 型別 + lint** — `npx tsc -b && npm run lint`（無新違規）
- [ ] **Step 3: 後端** — `cd src-tauri && cargo test`（sandbox 關；全綠）
- [ ] **Step 4: build** — `npm run build`
- [ ] **Step 5: 手動冒煙**（`npm run tauri:dev`）：
  - **本機 AI 面板迴歸**：開本機終端機 → Ctrl+I → 送訊息（串流回覆）、切 agent 模式跑一個任務、貼一張圖（附件 pill 出現）、開 MCP toggle、開對話歷史載入/刪除、放大縮小、拖寬度。全部行為跟改動前一樣（[[feedback_aipanel_attachment_regression]]）。
  - **遠端**：連一台遠端（control 模式）→ Ask AI 開 `RemoteAiPanel`：
    - `/agent 找出最大的三個檔案` → 對話串流、`<cmd>` 逐條出現在主終端機區並執行、輸出回流、多步後給總結
    - `/ai 這個目錄是做什麼的` → 純問答、不執行指令
    - 切 provider（badge → palette）→ 下一次查詢用新 provider
    - 開對話歷史 → 看得到剛剛的 session，載入、刪除
    - agent 進行中 → 主控端收回控制權 → 面板任務中止
    - 唯讀連線 → Ask AI 鈕 disabled
- [ ] **Step 6: 收尾 commit**（手動驗證有微調才需要）

---

## Self-Review

**1. Spec coverage**

| Spec 段落 | 對應 Task |
|---|---|
| 決策 1（完整移植 AiPanel 體驗） | Task 4（殼層）+ Task 6（RemoteAiPanel）|
| 決策 2（不做 MCP）| Task 5（`useRemoteAiChat` 拿掉工具迴圈）、Task 6（不傳 `extraInputControls`）|
| 決策 3（移植 chat-agent 迴圈，非 agentLoop.ts）| Task 6 Step 1 的 `runLoop`（移植自 `AiPanel` 的 `runAgentLoop`）|
| 決策 4（新面板取代舊；`/agent`→agent、`/ai`→chat；孤兒 `ai_query_ctx` 移除）| Task 2（移除）、Task 7 Step 3（前綴分流）、Task 7 Step 4（刪 AgentPanel）|
| 決策 5（抽殼層 + 兩薄包裝）| Task 4 |
| 決策 6（範圍：自由對話 / 歷史持久化 / 總結；不做附件·配額·卡住偵測）| Task 5（`send`）、Task 5 Step 1（`aiterm-remote-chat-sessions`）、Task 6（無 `<cmd>` 收尾＝總結）；Task 6 不接附件 / QuotaBadge / stuck |
| 元件一（`run_chat` + `ai_chat_ctx` + IPC）| Task 1、Task 2 |
| 元件二（`ChatPanelShell`）| Task 4 |
| 元件三（`useRemoteAiChat`）| Task 5 |
| 元件四（`RemoteAiPanel`）| Task 6 |
| 元件五（`RemoteTerminalView` 接線）| Task 7 |
| 邊角：OSC133 缺失 → 逾時訊息 | Task 6 Step 1 的 `AGENT_STEP_TIMEOUT_MS` + `remote_agent_no_shell_integration` |
| 邊角：resync / 收回控制權 / 連線結束中止 | Task 7 Step 3（`abortRef` + `abort()`）|
| 邊角：unmount | Task 7 保留 `abortRef` unmount effect；`useRemoteAiChat` `mountedRef` + cleanup |
| 邊角：唯讀 | Task 6（`isControl` guard + `inputDisabled`）、Task 7（Ask AI 鈕 disabled）|
| 邊角：未設 provider | Task 6 Step 1 catch → `formatAiError` 訊息 |
| 邊角：session key 分開 | Task 5 Step 1 |
| 邊角：`ChatPanelShell` 抽出的本機迴歸 | Task 4 回歸關卡 + Task 8 Step 5 手動 |
| 測試（Rust / 前端 / 手動）| 各 Task 的測試步驟 + Task 8 |
| i18n 清理 | Task 7 Step 5 |

**2. Placeholder scan**
- Task 4 Step 2 的 `return (/* ... */)` 與註解式替換清單——這是「把 `AiPanel/index.tsx` 498–792 的 JSX 搬過來並套替換」的搬移指示，不是要工作者發明 JSX。`ChatPanelShellProps` 介面是完整契約。可接受。
- Task 6 的 `mode={... as never}` 占位——Step 1 的 note 已明確要求「讀 `ModeHint.tsx` 用實際 `PanelMode` 字面值」。
- Task 7 Step 3 的 `ProviderPalette` props「讀呼叫處對齊」——`TerminalView.tsx:2040` 是現成範本。

**3. Type consistency**
- `run_chat(messages, snapshot, provider_id, locale, router, app, stream_id)`：Task 1 定義、Task 1 Step 3（`ai_chat` 呼叫）、Task 2 Step 1（`ai_chat_ctx` 呼叫）三處一致。
- `invokeAiChatCtx(messages, ctx, connId, providerId?, locale?)`：Task 2 Step 6 定義、Task 5（hook）、Task 6（agent 迴圈）呼叫一致。camelCase `recentOutput` → snake `recent_output` 在 IPC 包裝內轉。
- `RemoteAiPanelHandle = { submitAgent, send, abort }`：Task 6 定義、Task 7 Step 3 消費一致。
- `McpChatMessage` / `McpChatSession`：Task 3 移到 `types/chat.ts`，Task 4 / 5 / 6 都從那裡（或 `useMcpChat` re-export）import。
- `useRemoteAiChat` 回傳 `{ messages, isStreaming, streamBuf, error, send, addMessage, clear, loadMessages, deleteSession, resend, sessions }`：Task 5 定義、Task 6 消費一致（`RemoteAiPanel` 用 `chat.addMessage` / `chat.send` / `chat.loadMessages` / `chat.clear` / `chat.deleteSession` / `chat.resend` / `chat.sessions` / `chat.messages` / `chat.streamBuf` / `chat.isStreaming` / `chat.error`）。
- `ChatPanelShellProps`：Task 4 定義、Task 4 Step 3（`AiPanel` 傳）、Task 6（`RemoteAiPanel` 傳）——兩個消費端都要傳齊必填欄位；選填（`headerBadge` / `extraInputControls` / `extraAboveInput` / `toolFallbackReason` / `mcpToolCount` / `isWindows` / `inputDisabled`）RemoteAiPanel 多半不傳。
