# 遠端終端機 Ask AI（觀看端 Agent 迴圈）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓「遠端終端機」分頁（觀看端 `RemoteTerminalView`）能用**觀看端自己的 AI provider**，透過對話式輕量面板啟動 Agent 迴圈自動操作遠端主控端的終端機。

**Architecture:** 前端把 `TerminalView.tsx` 現有的 Agent 迴圈（`runAgentLoop` / `handleAiQuery` / `AgentLoopParams` / `normalizeAiError` / `PreviewState`）抽成共用模組 `src/lib/agentLoop.ts`，唯一介面變更是把寫死的 `invokeAiQuery` 換成注入的 `queryFn`。後端新增一個不吃 PTY session、改吃明確情境 `RemoteCtx` 的 Tauri 指令 `ai_query_ctx`，與現有 `ai_query` 共用抽出的 `run_single_command`。觀看端新增輕量面板 `RemoteTerminalView/AgentPanel.tsx`，引擎就是共用的 `runAgentLoop`，送指令與偵測每步完成走觀看端既有的 `useTerminalBlocks(connId)` + 分享連線。

**Tech Stack:** React 19 + TypeScript（Vitest / React Testing Library）、Rust + Tauri 2（wiremock / tokio）、xterm.js。

**Spec:** `docs/superpowers/specs/2026-08-28-remote-terminal-ask-ai-design.md`

**與 spec 的刻意出入：** spec 的「元件二」寫「`sessionId` 保留」。實際檢查程式碼後，`sessionId` 在 `runAgentLoop` / `handleAiQuery` 內**只**被拿去呼叫 `invokeAiQuery`，`onPhase` 與 `term.write` 狀態行都沒用到它。注入 `queryFn` 後 `sessionId` 變成純死欄位，因此本計畫直接把它從 `AgentLoopParams` 和 `handleAiQuery` 移除（4 個本機呼叫點改用閉包捕捉 session 的 `queryFn`）。

---

## 檔案結構

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src/lib/agentLoop.ts` | 建立 | 共用 Agent 迴圈：`PreviewState` / `INITIAL_PREVIEW` / `AgentLoopParams` / `runAgentLoop` / `handleAiQuery` / `shouldAutoExecute` / `normalizeAiError`。不依賴任何元件檔。 |
| `src/lib/agentLoop.test.ts` | 建立 | `runAgentLoop` 的單元測試：多步遞迴、`DONE`、步數上限、逾時、web sentinel、abort。 |
| `src/components/TerminalView.tsx` | 修改 | 刪掉搬走的定義，改 `import` 自 `agentLoop.ts`；4 個 `runAgentLoop({...})` 呼叫點補 `queryFn`。行為不變。 |
| `src-tauri/src/commands/ai.rs` | 修改 | 從 `ai_query` 抽出 `run_single_command`；新增 `RemoteCtx` struct 與 `ai_query_ctx` 指令。 |
| `src-tauri/src/ai/context.rs` | 修改 | 新增 `snapshot_from_remote_ctx(os, shell, cwd, recent_output)`。 |
| `src-tauri/src/lib.rs` | 修改 | `use` 與 `invoke_handler` 註冊 `ai_query_ctx`。 |
| `src-tauri/tests/ai_query_ctx.rs` | 建立 | wiremock 整合測試：`ai_query_ctx` 用傳入 `ctx` 組 prompt、回 `AiCommandReady`、`CommandGuard` 提級。 |
| `src/ipc/ai.ts` | 修改 | 新增 `RemoteCtx` 型別與 `invokeAiQueryCtx()`。 |
| `src/ipc/ai.test.ts` | 建立或修改 | `invokeAiQueryCtx` 的參數對應測試。 |
| `src/components/RemoteTerminalView/AgentPanel.tsx` | 建立 | 輕量對話式面板（呈現層）。 |
| `src/components/RemoteTerminalView/AgentPanel.css` | 建立 | 面板版面樣式（借用 AiPanel 的定位模式）。 |
| `src/components/RemoteTerminalView/AgentPanel.test.tsx` | 建立 | 面板元件測試。 |
| `src/components/RemoteTerminalView/index.tsx` | 修改 | 接上 mission state / `buildRemoteCtx` / `startAgentMission` / ai-stream 監聽 / Ask AI 鈕 / `handleWarpSubmit` / 唯讀停用 / 連線事件中止；渲染 `<AgentPanel>`；移除佔位邏輯。 |
| `src/components/RemoteTerminalView/index.test.tsx` | 修改 | 更新既有測試、加新測試。 |
| `src/lib/i18n.ts` | 修改 | 移除 `remote_terminal_ai_unsupported`；新增面板文案鍵（zh-TW + en）。 |
| `src/lib/i18n.remoteTerminal.test.ts` | 修改 | 更新對應斷言。 |

---

## Task 1: 抽出共用 `agentLoop.ts`（純重構 + `queryFn` 注入）

**Files:**
- Create: `src/lib/agentLoop.ts`
- Modify: `src/components/TerminalView.tsx`（`63-77`、`98-110`、`2119-2408` 之間的搬移；呼叫點 `~801`、`~946`、`~1482`、`~2019`）

回歸關卡＝既有 `TerminalView` 測試與 `tsc`。這一步不改迴圈邏輯，只搬家 + 把 `invokeAiQuery` 換成參數。

- [ ] **Step 1: 建立 `src/lib/agentLoop.ts`，貼入從 `TerminalView.tsx` 搬來的內容並改注入點**

把 `TerminalView.tsx` 這些定義原封不動搬進新檔（`PreviewState`、`INITIAL_PREVIEW`、`shouldAutoExecute`、`handleAiQuery`、`AgentLoopParams`、`runAgentLoop`、`normalizeAiError`），加上必要 import，並套用**兩處**修改：
1. `handleAiQuery` 的第 3 個參數 `sessionId: string` 改成 `queryFn: (query: string) => Promise<AiCommandReady>`；函式體內 `invokeAiQuery(query, sessionId, locale)` 改成 `queryFn(query)`。
2. `AgentLoopParams` 移除 `sessionId: string`，新增 `queryFn: (query: string) => Promise<AiCommandReady>`；`runAgentLoop` 內呼叫 `handleAiQuery(...)` 時，原本傳 `sessionId` 的位置改傳 `params.queryFn`。

```ts
// src/lib/agentLoop.ts
import type { Terminal } from "@xterm/xterm";
import type { Locale } from "./i18n";
import type { ExecutionMode } from "../ipc/config";
import {
  formatAiError,
  type AiCommandReady,
  type AiError,
  type RiskLevel,
} from "../ipc/ai";
import type { AgentPhase } from "../components/AgentStatusBar";
import type { AgentStepInfo } from "./agentStepReport";
import { webSearch, webFetch } from "../ipc/web";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

export interface PreviewState {
  loading: boolean;
  visible: boolean;
  command: string;
  explanation: string;
  riskLevel: RiskLevel;
}

export const INITIAL_PREVIEW: PreviewState = {
  loading: false,
  visible: false,
  command: "",
  explanation: "",
  riskLevel: "safe",
};

/** Decide whether to auto-execute based on execution mode and risk level. */
export function shouldAutoExecute(mode: ExecutionMode, risk: RiskLevel, agentActive = false): boolean {
  if (agentActive) {
    if (risk === "safe") return true;
    if (risk === "needs_confirm" && mode === "full-auto") return true;
    if (risk === "dangerous") return false;
  }
  if (mode === "always-confirm") return false;
  if (mode === "graded") return risk === "safe";
  if (mode === "full-auto") return risk === "safe" || risk === "needs_confirm";
  return false;
}

// ── 以下 handleAiQuery / normalizeAiError / AgentLoopParams / runAgentLoop
//    直接從 TerminalView.tsx 搬過來，套用本步驟說明的兩處注入修改。 ──

export function handleAiQuery(
  t: any,
  locale: Locale,
  queryFn: (query: string) => Promise<AiCommandReady>,   // 原本是 sessionId: string
  originalLine: string,
  query: string,
  term: Terminal,
  setPreview: (p: PreviewState) => void,
  setStreamText: React.Dispatch<React.SetStateAction<string>>,
  streamingRef: React.MutableRefObject<boolean>,
  executionModeRef: React.MutableRefObject<ExecutionMode>,
  writeRed: (msg: string) => void,
  submitCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void,
  onDone?: (explanation?: string) => void,
  agentActive = false,
  onCommandComplete?: (block: TerminalBlock) => void,
  onAiError?: (err: AiError) => void,
  onWebAction?: (type: "search" | "fetch", value: string) => void,
  onPhase?: (update: AgentPhase) => void,
  agentStep = 0,
  agentMaxSteps = 0,
) {
  void originalLine;
  setStreamText("");
  streamingRef.current = true;
  setPreview({ loading: true, visible: false, command: "", explanation: "", riskLevel: "safe" });

  queryFn(query)                                          // 原本是 invokeAiQuery(query, sessionId, locale)
    .then((resp) => {
      // ...函式其餘內容與 TerminalView.tsx 現狀完全一致，不改...
    })
    .catch((rawErr: unknown) => {
      // ...與現狀一致...
    });
}
```

`React` 型別：檔案頂端加 `import type React from "react";`（`setStreamText` / `MutableRefObject` 需要）。

`runAgentLoop` 內呼叫 `handleAiQuery` 的參數列，把原本第 3 個實參 `sessionId` 改成 `params.queryFn`：

```ts
  handleAiQuery(
    t,
    locale,
    params.queryFn,          // 原本是 sessionId
    "",
    query,
    term,
    // ...其餘不變...
  );
```

`AgentLoopParams`：

```ts
export interface AgentLoopParams {
  t: any;
  goal: string;
  locale: Locale;
  queryFn: (query: string) => Promise<AiCommandReady>;   // 取代 sessionId: string
  term: Terminal;
  getSubmitCommand: () => (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  setPreview: (p: PreviewState) => void;
  setStreamText: React.Dispatch<React.SetStateAction<string>>;
  streamingRef: React.MutableRefObject<boolean>;
  executionModeRef: React.MutableRefObject<ExecutionMode>;
  writeRed: (msg: string) => void;
  abortRef: React.MutableRefObject<boolean>;
  stepCount: number;
  maxSteps: number;
  history: { command: string; exitCode: number; output: string }[];
  onComplete: (explanation?: string) => void;
  onFail: (msg: string) => void;
  onStepComplete?: (info: AgentStepInfo) => void;
  onPhase?: (update: AgentPhase) => void;
}

export function normalizeAiError(err: unknown): AiError {
  // ...從 TerminalView.tsx 原封搬過來...
}
```

- [ ] **Step 2: 從 `TerminalView.tsx` 刪掉搬走的定義，改成 import**

刪除 `TerminalView.tsx` 的：`interface PreviewState {...}`（63-69）、`const INITIAL_PREVIEW`（71-77）、`function shouldAutoExecute`（98-110）、`function handleAiQuery`（2119-2219）、`interface AgentLoopParams`（2226-2249）、`function runAgentLoop`（2250-2386）、`function normalizeAiError`（2392-2408）。

在 import 區（`./parseAiPrefix` 那行附近）加：

```ts
import {
  runAgentLoop,
  INITIAL_PREVIEW,
  type PreviewState,
  type AgentLoopParams,
} from "../lib/agentLoop";
```

移除 `TerminalView.tsx` 現在不再用到的 import：檢查 `invokeAiQuery`（若 `TerminalView.tsx` 其他地方沒用就從第 24 行的 `../ipc/ai` import 拿掉——用 `grep -n invokeAiQuery src/components/TerminalView.tsx` 確認）。`webSearch` / `webFetch`（第 37 行）同樣確認後移除。`formatAiError`、`AiError`、`RiskLevel` 若別處還有用就留著。

- [ ] **Step 3: 4 個 `runAgentLoop({...})` 呼叫點補 `queryFn`，移除 `sessionId`**

每個呼叫點把 `sessionId: <x>,` 那行換成：

```ts
    queryFn: (q) => invokeAiQuery(q, <x>, locale),
```

其中 `<x>` 依呼叫點分別為：
- `~801`（Telegram 遠端）：`sessionRef.current`
- `~946`（initialMission）：`session`
- `~1482`（終端機 Enter handler）：`session`
- `~2019`（WarpInput onSubmit）：`sessionId`

`TerminalView.tsx` 第 24 行的 `../ipc/ai` import 需要保留 / 加回 `invokeAiQuery`（Step 2 若移除了這裡要加回來——它現在只剩這 4 個閉包在用）。

- [ ] **Step 4: 型別檢查**

Run: `npx tsc -b`
Expected: PASS（0 錯誤）。若報 `TerminalView.tsx` 有未使用 import，回 Step 2 清乾淨。

- [ ] **Step 5: 跑既有前端測試（回歸關卡）**

Run: `npm run test -- src/components/TerminalView`
Expected: 既有 `TerminalView*.test.tsx` 全數 PASS，數量與改動前一致。

- [ ] **Step 6: Lint**

Run: `npm run lint`
Expected: PASS。`agentLoop.ts` 的 `t: any` 若觸發 `@typescript-eslint/no-explicit-any`，加 `// eslint-disable-next-line @typescript-eslint/no-explicit-any`（與 `AgentStatusBar` 等既有檔案同樣處理）。

- [ ] **Step 7: Commit**

```bash
git add src/lib/agentLoop.ts src/components/TerminalView.tsx
git commit -m "refactor(agent-loop): 抽出共用 agentLoop.ts，invokeAiQuery 改為注入的 queryFn"
```

---

## Task 2: 後端 — 從 `ai_query` 抽出 `run_single_command`（純重構）

**Files:**
- Modify: `src-tauri/src/commands/ai.rs:220-318`

回歸關卡＝`cargo test`（`ai_query` 既有整合測試）。

- [ ] **Step 1: 新增私有 `run_single_command`，把 `ai_query` 主體搬進去**

在 `ai.rs` 的 `ai_query` 之前加：

```rust
/// `ai_query` 與 `ai_query_ctx` 共用的核心：拿一個已組好的 `EnvSnapshot`，
/// 跑 provider、串流 `ai-stream` 事件、解析並用 `CommandGuard` 覆核，回傳
/// `AiCommandReady`。`stream_id` 是 `ai-stream` 事件的 `session_id` 欄位值
/// （`ai_query` 傳 PTY session id，`ai_query_ctx` 傳觀看連線 conn id）。
async fn run_single_command(
    snapshot: crate::ai::EnvSnapshot,
    query: String,
    locale: Locale,
    router: &AiRouter,
    app: &AppHandle,
    stream_id: String,
) -> Result<AiCommandReady, AiError> {
    let provider = router.resolve().await?;
    let prompt = build_single_command_prompt(&snapshot, locale);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::Value::String(query),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: snapshot,
        mode: QueryMode::SingleCommand,
        max_tokens: None,
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let schema = ai_single_command_schema();
    let join = tokio::spawn(async move { provider_for_spawn.generate_json(req, schema, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        let _ = app.emit("ai-stream", AiStreamEvent {
            session_id: stream_id.clone(),
            kind: AiStreamKind::Query,
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

    if buf.trim().is_empty() {
        return Err(AiError::ModelError {
            reason: "模型回傳空回應（HTTP 200 但 body 為空）。\
                     請確認 Provider 的 base_url 和 model 設定正確，\
                     以及目標模型是否支援 /chat/completions 端點。".into(),
            raw: String::new(),
        });
    }

    let cleaned = extract_json_from_response(&buf);
    let parsed: AiSingleCommand = serde_json::from_str(&cleaned).map_err(|e| AiError::ModelError {
        reason: e.to_string(),
        raw: buf.chars().take(300).collect(),
    })?;

    if parsed.command.trim().eq_ignore_ascii_case("DONE") {
        return Ok(AiCommandReady {
            command: parsed.command,
            explanation: parsed.explanation,
            risk_level: parsed.risk_level,
        });
    }

    let (guard_level, guard_reason) = CommandGuard::classify(&parsed.command);
    let final_risk_level = std::cmp::max(parsed.risk_level, guard_level);
    let final_explanation = if guard_level > parsed.risk_level && guard_reason.is_some() {
        format!("{} (系統安全攔截: {})", parsed.explanation, guard_reason.unwrap())
    } else {
        parsed.explanation
    };

    Ok(AiCommandReady {
        command: parsed.command,
        explanation: final_explanation,
        risk_level: final_risk_level,
    })
}
```

- [ ] **Step 2: `ai_query` 改成呼叫 `run_single_command`**

`ai_query` 函式體整段換成：

```rust
#[tauri::command]
pub async fn ai_query(
    query: String,
    session_id: String,
    locale: Locale,
    app: AppHandle,
    pty_manager: State<'_, std::sync::Arc<PtyManager>>,
    router: State<'_, AiRouter>,
) -> Result<AiCommandReady, AiError> {
    let snapshot = context::snapshot(&pty_manager, &session_id);
    run_single_command(snapshot, query, locale, &router, &app, session_id).await
}
```

- [ ] **Step 3: 編譯 + 跑後端測試（回歸關卡）**

Run: `cd src-tauri && cargo test ai_query`
Expected: 既有 `ai_query` 相關測試全 PASS。

（前提：已跑過 `scripts/setup-uv-{mac,linux,win}` 與 DB2 sidecar 的 setup，否則 `cargo test` 會因 `externalBin` 缺檔而失敗——見 `CLAUDE.md`。）

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "refactor(ai): 抽出 run_single_command 供 ai_query 與 ai_query_ctx 共用"
```

---

## Task 3: 後端 — `ai_query_ctx` 指令

**Files:**
- Modify: `src-tauri/src/ai/context.rs`（新增 `snapshot_from_remote_ctx`）
- Modify: `src-tauri/src/commands/ai.rs`（`RemoteCtx` + `ai_query_ctx`）
- Modify: `src-tauri/src/lib.rs`（`use` + `invoke_handler`）
- Create: `src-tauri/tests/ai_query_ctx.rs`

- [ ] **Step 1: `context.rs` 加 `snapshot_from_remote_ctx`**

在 `snapshot_from_parts` 之後加：

```rust
/// 從觀看端傳來的明確情境組 snapshot（沒有本機 PTY 可查）。
/// `shell` 未知時填空字串（prompt 的 `Shell:` 欄位就留白），`cwd` 未知時
/// 填 `.`（`build_single_command_prompt` 只是把它 `display()` 進提示詞，
/// 不會拿去存取檔案系統）。`dir_listing` 一律 None——無法列遠端目錄。
pub fn snapshot_from_remote_ctx(
    os: &str,
    shell: Option<&str>,
    cwd: Option<&str>,
    recent_output: Option<String>,
) -> EnvSnapshot {
    EnvSnapshot {
        os: os.to_string(),
        shell: shell.unwrap_or("").to_string(),
        cwd: cwd.map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")),
        recent_output,
        dir_listing: None,
    }
}
```

在 `context.rs` 的 `#[cfg(test)] mod tests` 加：

```rust
    #[test]
    fn snapshot_from_remote_ctx_handles_missing_fields() {
        let s = snapshot_from_remote_ctx("linux", None, None, None);
        assert_eq!(s.os, "linux");
        assert_eq!(s.shell, "");
        assert_eq!(s.cwd, PathBuf::from("."));
        assert!(s.recent_output.is_none());
        assert!(s.dir_listing.is_none());

        let s2 = snapshot_from_remote_ctx(
            "windows", Some("pwsh"), Some("C:\\src"), Some("PS C:\\src>".into()),
        );
        assert_eq!(s2.shell, "pwsh");
        assert_eq!(s2.cwd, PathBuf::from("C:\\src"));
        assert_eq!(s2.recent_output.as_deref(), Some("PS C:\\src>"));
    }
```

Run: `cd src-tauri && cargo test snapshot_from_remote_ctx`
Expected: PASS。

- [ ] **Step 2: `ai.rs` 新增 `RemoteCtx` 與 `ai_query_ctx`**

在 `AiCommandReady` 定義附近加 struct，並在 `ai_query` 之後加指令：

```rust
/// 觀看端（遠端終端機分頁）傳來的情境。沒有本機 PTY，全部欄位由觀看端
/// 從已知資訊組出：`os` 來自 granted 事件的 hostOs，`recent_output` 來自
/// 觀看端 xterm buffer 末段，`shell` / `cwd` 觀看端無從得知 → None。
#[derive(Debug, Deserialize)]
pub struct RemoteCtx {
    pub os: String,
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub recent_output: Option<String>,
}

/// `ai_query` 的觀看端版本：不吃 PTY session_id，改吃明確的 `RemoteCtx`。
/// `conn_id` 當作 `ai-stream` 事件的識別（觀看端面板監聽這個值）。
#[tauri::command]
pub async fn ai_query_ctx(
    query: String,
    ctx: RemoteCtx,
    locale: Locale,
    conn_id: String,
    app: AppHandle,
    router: State<'_, AiRouter>,
) -> Result<AiCommandReady, AiError> {
    let snapshot = context::snapshot_from_remote_ctx(
        &ctx.os,
        ctx.shell.as_deref(),
        ctx.cwd.as_deref(),
        ctx.recent_output,
    );
    run_single_command(snapshot, query, locale, &router, &app, conn_id).await
}
```

- [ ] **Step 3: `lib.rs` 註冊指令**

`lib.rs:35` 的 `ai::{agent_chat, ai_chat, ai_query}` 改成 `ai::{agent_chat, ai_chat, ai_query, ai_query_ctx}`。

`lib.rs:303` 附近 `ai_query,` 那行下面加 `ai_query_ctx,`。

- [ ] **Step 4: 建立 `src-tauri/tests/ai_query_ctx.rs`**

參照 `src-tauri/tests/` 既有 `ai_query` 測試的 wiremock 起手式（看 `rg -l "ai_query" src-tauri/tests` 找到範例檔，複製其 provider 設定 / router 建構樣板）。核心斷言：

```rust
// 目標：呼叫 ai_query_ctx 的核心路徑（run_single_command），驗證
//   1. 傳入的 ctx（os / cwd / recent_output）會出現在送往 provider 的 system prompt
//   2. provider 回的 JSON 會被解析成 AiCommandReady
//   3. CommandGuard 會把危險指令的 risk 往上提級
//
// 因為 ai_query_ctx 是 #[tauri::command]（需要 AppHandle / State），
// 這裡直接測底層：用 context::snapshot_from_remote_ctx + build_single_command_prompt
// 驗證 prompt 內容，再用一個 mock provider 跑 run_single_command 等價流程。
// 若 repo 既有 ai_query 測試是透過 tauri::test::mock_app 驅動整個指令，
// 就照同樣方式驅動 ai_query_ctx。

#[test]
fn remote_ctx_fields_land_in_prompt() {
    use aiterm_lib::ai::context::snapshot_from_remote_ctx;
    use aiterm_lib::commands::ai::build_single_command_prompt; // 若非 pub，改在 ai.rs 內寫 #[cfg(test)] 單元測試

    let snap = snapshot_from_remote_ctx(
        "linux", None, None, Some("user@host:~/proj$ ls\nCargo.toml  src/".into()),
    );
    let prompt = build_single_command_prompt(&snap, aiterm_lib::ai::Locale::EnUs);
    assert!(prompt.contains("OS: linux"));
    assert!(prompt.contains("~/proj$ ls"));       // recent_output 有進 prompt
    assert!(prompt.contains("Cwd: ."));           // cwd 缺省成 "."
}
```

> 注意：`build_single_command_prompt` 目前是 `pub fn`（`ai.rs:104`），可直接跨 crate 用。若整合測試連結不到（模組非 `pub`），改成在 `ai.rs` 的 `#[cfg(test)] mod tests` 內寫這段單元測試，斷言相同。`Locale` 的實際 enum 變體名以 `src-tauri/src/ai/mod.rs` 為準（用 `rg "enum Locale" -A4 src-tauri/src/ai`）。

- [ ] **Step 5: 編譯 + 測試**

Run: `cd src-tauri && cargo test ai_query`
Expected: `ai_query`、`ai_query_ctx`、`snapshot_from_remote_ctx`、`remote_ctx_fields_land_in_prompt` 全 PASS。

Run: `cd src-tauri && cargo build`
Expected: 0 錯誤（確認 `invoke_handler` 註冊語法正確）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/context.rs src-tauri/src/commands/ai.rs src-tauri/src/lib.rs src-tauri/tests/ai_query_ctx.rs
git commit -m "feat(ai): 新增 ai_query_ctx——吃明確 RemoteCtx 而非 PTY session 的單發查詢"
```

---

## Task 4: 前端 IPC — `invokeAiQueryCtx`

**Files:**
- Modify: `src/ipc/ai.ts`
- Create: `src/ipc/ai.test.ts`（若已存在則改為新增 describe 區塊）

- [ ] **Step 1: 寫失敗測試**

```ts
// src/ipc/ai.test.ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { invokeAiQueryCtx } from "./ai";

describe("invokeAiQueryCtx", () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue({ command: "ls", explanation: "", risk_level: "safe" }));

  it("passes query, ctx (snake_case recent_output), connId, locale to the ai_query_ctx command", async () => {
    await invokeAiQueryCtx(
      "list files",
      { os: "linux", shell: null, cwd: null, recentOutput: "prompt$" },
      "conn-123",
      "en",
    );
    expect(invokeMock).toHaveBeenCalledWith("ai_query_ctx", {
      query: "list files",
      ctx: { os: "linux", shell: null, cwd: null, recent_output: "prompt$" },
      connId: "conn-123",
      locale: "en",
    });
  });
});
```

Run: `npm run test -- src/ipc/ai.test.ts`
Expected: FAIL（`invokeAiQueryCtx` is not exported）。

- [ ] **Step 2: 實作**

在 `src/ipc/ai.ts` 的 `invokeAiQuery` 之後加：

```ts
export interface RemoteCtx {
  os: string;
  shell: string | null;
  cwd: string | null;
  recentOutput: string | null;
}

export function invokeAiQueryCtx(
  query: string,
  ctx: RemoteCtx,
  connId: string,
  locale: Locale = "zh-TW",
): Promise<AiCommandReady> {
  return invoke<AiCommandReady>("ai_query_ctx", {
    query,
    ctx: {
      os: ctx.os,
      shell: ctx.shell,
      cwd: ctx.cwd,
      recent_output: ctx.recentOutput,
    },
    connId,
    locale,
  });
}
```

Run: `npm run test -- src/ipc/ai.test.ts`
Expected: PASS。

- [ ] **Step 3: 型別檢查**

Run: `npx tsc -b`
Expected: PASS。

- [ ] **Step 4: Commit**

```bash
git add src/ipc/ai.ts src/ipc/ai.test.ts
git commit -m "feat(ipc): invokeAiQueryCtx 包裝 ai_query_ctx"
```

---

## Task 5: `AgentPanel.tsx` 輕量面板（呈現層）

**Files:**
- Create: `src/components/RemoteTerminalView/AgentPanel.tsx`
- Create: `src/components/RemoteTerminalView/AgentPanel.css`
- Create: `src/components/RemoteTerminalView/AgentPanel.test.tsx`

面板是純呈現：所有狀態由 `RemoteTerminalView` 傳入，所有動作用 callback 往上拋。引擎邏輯不在這裡。

- [ ] **Step 1: 寫失敗測試**

```tsx
// src/components/RemoteTerminalView/AgentPanel.test.tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AgentPanel } from "./AgentPanel";
import { INITIAL_PREVIEW } from "../../lib/agentLoop";
import { translations } from "../../lib/i18n";

const t = translations["en"];
const baseProps = {
  mission: null,
  phase: null,
  streamText: "",
  preview: INITIAL_PREVIEW,
  onSubmitGoal: vi.fn(),
  onStop: vi.fn(),
  onConfirmPreview: vi.fn(),
  onCancelPreview: vi.fn(),
  onClose: vi.fn(),
  disabled: false,
  t,
};

describe("AgentPanel", () => {
  it("submits a typed goal via onSubmitGoal", async () => {
    const onSubmitGoal = vi.fn();
    render(<AgentPanel {...baseProps} onSubmitGoal={onSubmitGoal} />);
    await userEvent.type(screen.getByRole("textbox"), "find big files");
    await userEvent.keyboard("{Enter}");
    expect(onSubmitGoal).toHaveBeenCalledWith("find big files");
  });

  it("renders one card per mission history step with exit-code badge", () => {
    render(
      <AgentPanel
        {...baseProps}
        mission={{
          goal: "g", active: true, stepCount: 2, maxSteps: 5, tokensUsed: 0,
          history: [
            { command: "pwd", exitCode: 0, output: "/home/u" },
            { command: "false", exitCode: 1, output: "" },
          ],
        }}
      />
    );
    expect(screen.getByText("pwd")).toBeInTheDocument();
    expect(screen.getByText("false")).toBeInTheDocument();
    expect(screen.getByText("exit 1")).toBeInTheDocument();
  });

  it("shows a Stop button while the mission is active and calls onStop", async () => {
    const onStop = vi.fn();
    render(
      <AgentPanel
        {...baseProps}
        onStop={onStop}
        mission={{ goal: "g", active: true, stepCount: 0, maxSteps: 5, tokensUsed: 0, history: [] }}
      />
    );
    await userEvent.click(screen.getByRole("button", { name: t.remote_agent_stop }));
    expect(onStop).toHaveBeenCalled();
  });

  it("renders CommandPreview when preview.visible and wires confirm/cancel", async () => {
    const onConfirmPreview = vi.fn();
    render(
      <AgentPanel
        {...baseProps}
        onConfirmPreview={onConfirmPreview}
        preview={{ loading: false, visible: true, command: "rm -rf x", explanation: "danger", riskLevel: "dangerous" }}
      />
    );
    expect(screen.getByText("rm -rf x")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /Execute Anyway/i }));
    expect(onConfirmPreview).toHaveBeenCalled();
  });

  it("shows the failed reason when phase is failed", () => {
    render(<AgentPanel {...baseProps} phase={{ phase: "failed", reason: "lost control" }} />);
    expect(screen.getByText(/lost control/)).toBeInTheDocument();
  });
});
```

Run: `npm run test -- src/components/RemoteTerminalView/AgentPanel.test.tsx`
Expected: FAIL（模組不存在）。

- [ ] **Step 2: 實作 `AgentPanel.tsx`**

```tsx
// src/components/RemoteTerminalView/AgentPanel.tsx
import { useState } from "react";
import type { AgentMission } from "../../hooks/useAgentMission";
import type { AgentPhase } from "../AgentStatusBar";
import type { PreviewState } from "../../lib/agentLoop";
import { CommandPreview } from "../CommandPreview";
import type { Translations } from "../../lib/i18n";
import "./AgentPanel.css";

interface Props {
  mission: AgentMission | null;
  phase: AgentPhase | null;
  streamText: string;
  preview: PreviewState;
  onSubmitGoal: (goal: string) => void;
  onStop: () => void;
  onConfirmPreview: () => void;
  onCancelPreview: () => void;
  onClose: () => void;
  disabled: boolean;
  t: Translations;
}

export function AgentPanel({
  mission, phase, streamText, preview,
  onSubmitGoal, onStop, onConfirmPreview, onCancelPreview, onClose, disabled, t,
}: Props) {
  const [draft, setDraft] = useState("");
  const active = mission?.active ?? false;

  const submit = () => {
    const goal = draft.trim();
    if (!goal || disabled || active) return;
    setDraft("");
    onSubmitGoal(goal);
  };

  return (
    <div className="aiterm-remote-agent-panel" role="dialog" aria-label={t.remote_agent_panel_title}>
      <div className="aiterm-remote-agent-panel__header">
        <span className="aiterm-remote-agent-panel__title">{t.remote_agent_panel_title}</span>
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onClose}>
          ✕
        </button>
      </div>

      <div className="aiterm-remote-agent-panel__body">
        {mission?.history.map((h, i) => (
          <div className="aiterm-remote-agent-panel__step" key={i}>
            <div className="aiterm-remote-agent-panel__cmd">
              <span>▶ {h.command}</span>
              <span
                className={
                  "aiterm-remote-agent-panel__exit" +
                  (h.exitCode === 0 ? " is-ok" : " is-fail")
                }
              >
                {h.exitCode === 0 ? "exit 0" : `exit ${h.exitCode}`}
              </span>
            </div>
            {h.output && (
              <details className="aiterm-remote-agent-panel__out">
                <summary>{t.remote_agent_output}</summary>
                <pre>{h.output}</pre>
              </details>
            )}
          </div>
        ))}

        {phase?.phase === "asking" && (
          <div className="aiterm-remote-agent-panel__status">◐ {t.term_agent_status_asking}</div>
        )}
        {phase?.phase === "running" && (
          <div className="aiterm-remote-agent-panel__status">
            ▶ {t.term_agent_status_running(phase.command)}
          </div>
        )}
        {phase?.phase === "web" && (
          <div className="aiterm-remote-agent-panel__status">
            {phase.webKind === "search"
              ? t.term_agent_status_web_search(phase.query)
              : t.term_agent_status_web_fetch(phase.query)}
          </div>
        )}
        {streamText && phase?.phase === "asking" && (
          <pre className="aiterm-remote-agent-panel__stream">{streamText}</pre>
        )}

        {phase?.phase === "done" && (
          <div className="aiterm-remote-agent-panel__done">✅ {t.remote_agent_done}</div>
        )}
        {phase?.phase === "failed" && (
          <div className="aiterm-remote-agent-panel__failed">⚠ {phase.reason}</div>
        )}
      </div>

      {preview.visible && (
        <CommandPreview
          command={preview.command}
          explanation={preview.explanation}
          riskLevel={preview.riskLevel}
          onConfirm={onConfirmPreview}
          onCancel={onCancelPreview}
        />
      )}

      <div className="aiterm-remote-agent-panel__footer">
        {active ? (
          <button className="aiterm-btn aiterm-btn--danger" onClick={onStop}>
            ■ {t.remote_agent_stop}
          </button>
        ) : (
          <input
            className="aiterm-input"
            type="text"
            value={draft}
            disabled={disabled}
            placeholder={disabled ? t.remote_agent_needs_control : t.remote_agent_goal_placeholder}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                submit();
              }
            }}
          />
        )}
      </div>
    </div>
  );
}
```

> `Translations` 型別由 `src/lib/i18n.ts` export（`RemoteTerminalView/index.tsx` 已經 `import type { Translations }`）。若 `aiterm-btn--danger` / `aiterm-input` class 在專案樣式中不存在，用 `rg "aiterm-btn--danger|aiterm-input" src/**/*.css` 確認；不存在就在 `AgentPanel.css` 補等價樣式。

- [ ] **Step 3: 實作 `AgentPanel.css`**

```css
/* src/components/RemoteTerminalView/AgentPanel.css
   定位模式借用 AiPanel/styles.css：絕對定位、貼右、整欄、flex 直排。 */
.aiterm-remote-agent-panel {
  position: absolute;
  top: 0;
  right: 0;
  height: 100%;
  width: 420px;
  max-width: 90vw;
  display: flex;
  flex-direction: column;
  z-index: 50;
  background: var(--panel-bg, #1a1a1a);
  border-left: 1px solid var(--border-color, #2a2a2a);
}
.aiterm-remote-agent-panel__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color, #2a2a2a);
}
.aiterm-remote-agent-panel__title {
  font-weight: 700;
  background: var(--accent-gradient);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}
.aiterm-remote-agent-panel__body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.aiterm-remote-agent-panel__cmd {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 12px;
}
.aiterm-remote-agent-panel__exit.is-ok { color: #10b981; }
.aiterm-remote-agent-panel__exit.is-fail { color: #f87171; }
.aiterm-remote-agent-panel__out pre,
.aiterm-remote-agent-panel__stream {
  white-space: pre-wrap;
  word-break: break-all;
  font-size: 11px;
  max-height: 200px;
  overflow-y: auto;
  margin: 4px 0 0;
}
.aiterm-remote-agent-panel__failed { color: #f87171; }
.aiterm-remote-agent-panel__done { color: #10b981; }
.aiterm-remote-agent-panel__footer {
  padding: 8px 12px;
  border-top: 1px solid var(--border-color, #2a2a2a);
}
.aiterm-remote-agent-panel__footer .aiterm-input { width: 100%; }
```

- [ ] **Step 4: 加 i18n 鍵（先加，測試才有得比對）**

在 `src/lib/i18n.ts` 的 zh-TW 區塊（`remote_terminal_*` 附近）加：

```ts
    remote_agent_panel_title: "AI 代理",
    remote_agent_goal_placeholder: "輸入要 AI 代理完成的目標…",
    remote_agent_needs_control: "需要控制權才能使用 AI 代理",
    remote_agent_stop: "停止",
    remote_agent_output: "輸出",
    remote_agent_done: "任務完成",
    remote_agent_no_shell_integration: "遠端 shell 未啟用指令追蹤，無法自動接續。",
    remote_agent_aborted_resync: "連線重新同步，任務中止。",
    remote_agent_aborted_control_lost: "已失去控制權，任務中止。",
    remote_agent_aborted_ended: "連線已結束，任務中止。",
```

在 en 區塊對應位置加：

```ts
    remote_agent_panel_title: "AI Agent",
    remote_agent_goal_placeholder: "Describe the goal for the AI agent…",
    remote_agent_needs_control: "Control permission is required to use the AI agent",
    remote_agent_stop: "Stop",
    remote_agent_output: "Output",
    remote_agent_done: "Mission complete",
    remote_agent_no_shell_integration: "The remote shell has no command tracking; cannot continue automatically.",
    remote_agent_aborted_resync: "Connection resynced; mission aborted.",
    remote_agent_aborted_control_lost: "Control permission lost; mission aborted.",
    remote_agent_aborted_ended: "Connection ended; mission aborted.",
```

- [ ] **Step 5: 跑面板測試**

Run: `npm run test -- src/components/RemoteTerminalView/AgentPanel.test.tsx`
Expected: 全 PASS。

- [ ] **Step 6: 型別檢查 + lint**

Run: `npx tsc -b && npm run lint`
Expected: PASS。

- [ ] **Step 7: Commit**

```bash
git add src/components/RemoteTerminalView/AgentPanel.tsx src/components/RemoteTerminalView/AgentPanel.css src/components/RemoteTerminalView/AgentPanel.test.tsx src/lib/i18n.ts
git commit -m "feat(remote-terminal): AgentPanel 輕量對話式面板（呈現層）"
```

---

## Task 6: 接線 `RemoteTerminalView/index.tsx`

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Modify: `src/components/RemoteTerminalView/index.test.tsx`
- Modify: `src/lib/i18n.ts`（移除 `remote_terminal_ai_unsupported`）
- Modify: `src/lib/i18n.remoteTerminal.test.ts`

- [ ] **Step 1: 移除佔位邏輯**

在 `RemoteTerminalView/index.tsx`：
- 刪 `const [aiUnsupported, setAiUnsupported] = useState(false);`（`155` 附近）。
- 刪 render 裡的 `{aiUnsupported && (<div className="aiterm-remote-terminal__ai-unsupported">{t.remote_terminal_ai_unsupported}</div>)}`（`555-557`）。
- `handleWarpSubmit`（`194-204`）暫時保留結構，Step 4 會改寫。

- [ ] **Step 2: 加入 mission 狀態、config、refs**

在既有 import 補：

```ts
import { useAgentMission } from "../../hooks/useAgentMission";
import { runAgentLoop, INITIAL_PREVIEW, type PreviewState } from "../../lib/agentLoop";
import { invokeAiQueryCtx, type RemoteCtx, type AiStreamEvent } from "../../ipc/ai";
import { getConfig, type ExecutionMode } from "../../ipc/config";
import { listen } from "@tauri-apps/api/event";
import type { AgentPhase } from "../AgentStatusBar";
import { reportAgentStep } from "../../lib/agentStepReport";
import { AgentPanel } from "./AgentPanel";
```

在元件內（既有 state 附近）加：

```ts
  const { agentMission, startMission, appendHistory, stopMission } = useAgentMission();
  const [agentPanelOpen, setAgentPanelOpen] = useState(false);
  const [agentPhase, setAgentPhase] = useState<AgentPhase | null>(null);
  const [streamText, setStreamText] = useState("");
  const [preview, setPreview] = useState<PreviewState>(INITIAL_PREVIEW);
  const streamingRef = useRef(false);
  const abortRef = useRef(false);
  const executionModeRef = useRef<ExecutionMode>("graded");
  const maxAgentStepsRef = useRef(5);

  useEffect(() => {
    getConfig()
      .then((cfg) => {
        executionModeRef.current = cfg.execution_mode;
        maxAgentStepsRef.current = cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5);
      })
      .catch(() => {});
  }, []);
```

`submitCommand` 已由既有 `useTerminalBlocks(connId, ...)` 解構出來（`105` 附近的 `const { blocks, isAlternateBuffer, submitCommand, appendOutput, clearAllBlocks } = useTerminalBlocks(...)`）。加一顆橋接 ref 給迴圈用最新值：

```ts
  const submitCommandRef = useRef(submitCommand);
  submitCommandRef.current = submitCommand;
```

- [ ] **Step 3: `buildRemoteCtx` 與 `readRecentOutput`**

在檔案內（元件外的純函式區，`endReasonText` 附近）加：

```ts
/** 讀 xterm buffer 末段當 AI 情境；term 未建立回 null。 */
export function readRecentOutput(term: Terminal | null, maxChars = 4000): string | null {
  if (!term) return null;
  const buf = term.buffer.active;
  const bottom = buf.baseY + buf.cursorY;
  const lines: string[] = [];
  let chars = 0;
  for (let i = bottom; i >= 0 && chars < maxChars; i--) {
    const s = buf.getLine(i)?.translateToString(true) ?? "";
    lines.push(s);
    chars += s.length + 1;
  }
  lines.reverse();
  const joined = lines.join("\n").trimEnd();
  return joined.length > 0 ? joined : null;
}
```

在元件內：

```ts
  const buildRemoteCtx = useCallback((): RemoteCtx => ({
    os: hostPlatform === "windows" ? "windows" : "linux",
    shell: null,
    cwd: null,
    recentOutput: readRecentOutput(termRef.current),
  }), [hostPlatform]);
```

- [ ] **Step 4: `startAgentMission` 與改寫 `handleWarpSubmit`**

```ts
  const startAgentMission = useCallback((goal: string, maxSteps: number) => {
    if (agentMission?.active) return;            // 一次一個任務
    if (!(phaseRef.current.kind === "live" && phaseRef.current.mode === "control")) return;
    abortRef.current = false;
    setAgentPhase(null);
    setPreview(INITIAL_PREVIEW);
    setAgentPanelOpen(true);
    startMission(goal, maxSteps);
    runAgentLoop({
      t,
      goal,
      locale,
      queryFn: (q) => invokeAiQueryCtx(q, buildRemoteCtx(), connId, locale),
      term: termRef.current!,
      getSubmitCommand: () => submitCommandRef.current,
      setPreview,
      setStreamText,
      streamingRef,
      executionModeRef,
      writeRed: (m) => termRef.current?.write(`\r\n\x1b[31m${m}\x1b[0m\r\n`),
      abortRef,
      stepCount: 0,
      maxSteps,
      history: [],
      onPhase: setAgentPhase,
      onComplete: () => { setAgentPhase({ phase: "done", steps: agentMission?.stepCount ?? 0 }); stopMission(); },
      onFail: (msg) => {
        // runAgentLoop 逾時走 onFail(term_agent_timeout_fail)——在遠端這幾乎
        // 都是「主控端 shell 沒有 OSC 133;D」造成的，換成更貼切的訊息。
        const reason = msg === t.term_agent_timeout_fail ? t.remote_agent_no_shell_integration : msg;
        setAgentPhase({ phase: "failed", reason });
        stopMission();
      },
      onStepComplete: (info) => {
        appendHistory(info.command, info.exitCode, info.output);
        reportAgentStep(info, {});   // 觀看端沒有 Telegram / 首頁進度回報，傳空 callbacks
      },
    });
  }, [agentMission, t, locale, connId, buildRemoteCtx, startMission, stopMission, appendHistory]);

  const stopAgentMission = useCallback(() => {
    abortRef.current = true;
    stopMission();
    setPreview(INITIAL_PREVIEW);
  }, [stopMission]);
```

> `locale` 由 `useLocale()` 取得（`RemoteTerminalView` 目前只解構 `t`，改成 `const { t, locale } = useLocale();`——確認 `LocaleContext` 有 export `locale`，`TerminalView` 是這樣用的）。
> `reportAgentStep` 接受 `{ sendRemoteResponse?, onAgentProgress? }`，兩者皆可選，傳 `{}` 是合法 no-op（見 `agentStepReport.ts`）。這行其實可省略——保留只是為了跟本機呼叫形狀一致；若 lint 抱怨未使用 import 就整行連 import 一起拿掉。

改寫 `handleWarpSubmit`：

```ts
  const handleWarpSubmit = useCallback(
    (cmd: string) => {
      // 任務進行中在輸入框送出任何東西 = 中斷任務（比照本機終端機按 Enter
      // 中斷 agent）。中斷後這一行不繼續處理，避免跟 agent 指令交錯送到遠端。
      if (agentMission?.active) {
        stopAgentMission();
        return;
      }
      const agentGoal = parseAgentPrefix(cmd);
      const aiGoal = parseAiPrefix(cmd);
      if (agentGoal !== null) {
        startAgentMission(agentGoal, maxAgentStepsRef.current);
        return;
      }
      if (aiGoal !== null) {
        startAgentMission(aiGoal, 1);
        return;
      }
      submitCommand(cmd);
    },
    [submitCommand, startAgentMission, stopAgentMission, agentMission],
  );
```

- [ ] **Step 5: Ask AI 鈕改行為 + 唯讀停用**

`RemoteTerminalView/index.tsx` 的 Ask AI `<button>`（`433-444`）：

```tsx
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            title={
              phase.kind === "live" && phase.mode === "control"
                ? t.term_ai_helper_tooltip
                : t.remote_agent_needs_control
            }
            disabled={!(phase.kind === "live" && phase.mode === "control")}
            onClick={(e) => {
              e.stopPropagation();
              setAgentPanelOpen(true);
            }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <SparklesIcon size={14} />
            <span>Ask AI</span>
          </button>
```

- [ ] **Step 6: ai-stream 監聽（餵串流文字）**

在既有 `useEffect([connId])`（訂閱 `share-viewer://*` 的那個，`206-292`）內，`track(...)` 之外、`return` 之前加一個獨立監聽（或另開一個 `useEffect([connId])`）：

```ts
    const streamUn = listen<AiStreamEvent>("ai-stream", (event) => {
      if (event.payload.kind !== "query") return;
      if (event.payload.session_id !== connId) return;
      if (!event.payload.done) setStreamText((s) => s + event.payload.delta);
    });
    streamUn.then((un) => { if (disposed) un(); else unlisteners.push(un); });
```

（`disposed` / `unlisteners` 是該 effect 既有的區域變數，沿用。）

- [ ] **Step 7: 連線事件中止任務**

在既有的三個 handler 內補中止：

```ts
    track(
      onShareViewerResync(connId, () => {
        termRef.current?.clear();
        clearAllBlocksRef.current();
        if (agentMissionRef.current?.active) {
          abortRef.current = true;
          setAgentPhase({ phase: "failed", reason: t.remote_agent_aborted_resync });
          stopMission();
        }
      }),
    );

    track(
      onShareViewerControlChanged(connId, (mode) => {
        setPhase({ kind: "live", mode });
        if (mode !== "control" && agentMissionRef.current?.active) {
          abortRef.current = true;
          setAgentPhase({ phase: "failed", reason: t.remote_agent_aborted_control_lost });
          stopMission();
        }
      }),
    );

    track(onShareViewerEnded(connId, (reason) => {
      setPhase({ kind: "ended", reason });
      if (agentMissionRef.current?.active) {
        abortRef.current = true;
        setAgentPhase({ phase: "failed", reason: t.remote_agent_aborted_ended });
        stopMission();
      }
    }));
```

新增橋接 ref（因為這些 handler 只註冊一次，需讀最新 mission）：

```ts
  const agentMissionRef = useRef(agentMission);
  agentMissionRef.current = agentMission;
```

> 這個 effect 的依賴陣列目前是 `[connId]`。新增讀取 `t` / `stopMission` 是透過穩定參照（`t` 每次 render 變但只在 callback 內讀當下值可接受；若要嚴謹就也橋接一顆 `tRef`）。與本 repo 既有 `phaseRef` / `blocksRef` 橋接手法一致，見檔案內註解。

- [ ] **Step 8: 渲染面板**

在 return 的最外層 `<div className="aiterm-remote-terminal">` 內末尾（`WarpInput` 之後）加：

```tsx
      {agentPanelOpen && (
        <AgentPanel
          mission={agentMission}
          phase={agentPhase}
          streamText={streamText}
          preview={preview}
          onSubmitGoal={(goal) => startAgentMission(goal, maxAgentStepsRef.current)}
          onStop={stopAgentMission}
          onConfirmPreview={() => {
            const cmd = preview.command;
            setPreview(INITIAL_PREVIEW);
            submitCommandRef.current(cmd);
          }}
          onCancelPreview={() => { setPreview(INITIAL_PREVIEW); stopAgentMission(); }}
          onClose={() => {
            if (agentMission?.active) stopAgentMission();
            setAgentPanelOpen(false);
          }}
          disabled={!(phase.kind === "live" && phase.mode === "control")}
          t={t}
        />
      )}
```

> `onConfirmPreview` 直接送指令但**不**接 `onBlockDone`——迴圈在 `handleAiQuery` 的自動執行分支才會掛 `onCommandComplete`。危險指令走的是 preview 分支，`handleAiQuery` 那次呼叫**不會**遞迴。這代表：使用者確認危險指令後，該步的輸出不會被迴圈接續。**這是本迭代的已知限制**——spec 的安全模型是「危險指令要人確認」，確認後即停在該步，由使用者接手或重新下 `/agent` 目標。在 `AgentPanel` 的 preview 區塊上方顯示一行提示（用既有 `t.term_danger_warning` 或新鍵），Task 7 手動驗證時確認行為可接受。

- [ ] **Step 9: 更新 `src/lib/i18n.ts` — 移除 `remote_terminal_ai_unsupported`**

從 zh-TW（`807`）與 en（`2165`）刪掉 `remote_terminal_ai_unsupported` 這行。

- [ ] **Step 10: 更新 `src/lib/i18n.remoteTerminal.test.ts`**

用 `rg "remote_terminal_ai_unsupported" src/lib/i18n.remoteTerminal.test.ts` 找到斷言並刪除／改寫。若該測試是「每個 remote_terminal_* 鍵 zh 與 en 都存在」的泛掃描，移除鍵後自然通過，不需改測試——先跑一次看。

- [ ] **Step 11: 更新 `src/components/RemoteTerminalView/index.test.tsx`**

- 移除「點 Ask AI 顯示 `aiUnsupported` 且不觸發 IPC」的測試（行為已改）。
- 新增測試（沿用檔案既有的 `handlers` / `captureHandler` / mock 樣板；用 `granted` handler 把連線推進 `live` + `control`）：

```tsx
it("Ask AI 按鈕在 control 模式可點、開啟面板", async () => {
  renderRemote();                        // 檔案既有的 render helper
  act(() => handlers["granted:conn-1"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never));
  await userEvent.click(screen.getByRole("button", { name: /Ask AI/i }));
  expect(screen.getByRole("dialog", { name: /AI (代理|Agent)/i })).toBeInTheDocument();
});

it("唯讀模式 Ask AI 按鈕 disabled", () => {
  renderRemote();
  act(() => handlers["granted:conn-1"]({ mode: "read_only", cols: 80, rows: 24, hostOs: "linux" } as never));
  expect(screen.getByRole("button", { name: /Ask AI/i })).toBeDisabled();
});

it("/agent 前綴啟動 mission，不把原字串送給 submitCommand", async () => {
  const { sendMock } = renderRemote();   // sendMock 是檔案頂端既有的 shareViewerSend spy
  act(() => handlers["granted:conn-1"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never));
  // 透過 WarpInput 送出（元件已把 handleWarpSubmit 接給 WarpInput）
  await userEvent.type(screen.getByPlaceholderText(/./), "/agent tidy up{Enter}");
  expect(sendMock).not.toHaveBeenCalledWith("conn-1", "/agent tidy up");
});
```

> `invokeAiQueryCtx` 在這個測試檔要 mock 掉（加到既有的 `vi.mock("../../ipc/...")` 或新增一段 `vi.mock("../../ipc/ai", ...)`），回一個永不 resolve 或回固定 `AiCommandReady` 的 promise，避免真的打 IPC。`@tauri-apps/api/event` 的 `listen` 也要 mock（回 `Promise.resolve(() => {})`）。

- [ ] **Step 12: 跑全部相關測試**

Run: `npm run test -- src/components/RemoteTerminalView src/lib/i18n.remoteTerminal`
Expected: 全 PASS。

Run: `npx tsc -b && npm run lint`
Expected: PASS。

- [ ] **Step 13: Commit**

```bash
git add src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx src/lib/i18n.ts src/lib/i18n.remoteTerminal.test.ts
git commit -m "feat(remote-terminal): 觀看端 Ask AI 接上 Agent 迴圈與 AgentPanel"
```

---

## Task 7: 完整驗證與手動冒煙

**Files:** 無（純驗證）

- [ ] **Step 1: 前端全套**

Run: `npm run test`
Expected: 全 PASS（特別注意 `TerminalView*`、`RemoteTerminalView*`、`agentLoop`、`AgentPanel`、`ipc/ai`、`i18n*`）。

- [ ] **Step 2: 型別 + lint**

Run: `npx tsc -b && npm run lint`
Expected: PASS。

- [ ] **Step 3: 後端全套**

Run: `cd src-tauri && cargo test`
Expected: 全 PASS。

- [ ] **Step 4: 建置**

Run: `npm run build`
Expected: 成功（`tsc` + `vite`）。

- [ ] **Step 5: 手動冒煙（記憶：refactor 後必須手動驗證）**

`npm run tauri:dev`，一台當主控端分享終端機、另一個分頁／實例當觀看端連上（`mode: control`）：

1. 觀看端 `/agent 找出目前目錄下最大的三個檔案` → 面板開啟、指令逐步出現在遠端並執行、輸出回流、`mission.history` 卡片遞增、最後 `DONE`（✅ 任務完成）。
2. 任務進行中按「■ 停止」→ 迴圈立即停、不再送新指令。
3. 任務進行中在 `WarpInput` 打一行普通指令送出 → 任務中止，該行**不**送到遠端。
4. 主控端在觀看端任務進行中收回控制權 → 面板顯示「已失去控制權，任務中止」、輸入框停用。
5. 危險指令（例如目標誘導出 `rm -rf`）→ 面板底部出現 `CommandPreview`，按 Esc 取消 → 任務中止、不執行。
6. 觀看端未設 AI provider → 面板顯示未設定提示，不 crash。
7. 唯讀連線 → 工具列 Ask AI 鈕 disabled、tooltip 正確。

- [ ] **Step 6: 收尾 commit（如手動驗證需要微調）**

```bash
git add -A
git commit -m "fix(remote-terminal): 手動冒煙後的調整"
```

（無調整則跳過。）

---

## Self-Review

**1. Spec coverage**

| Spec 段落 | 對應 Task |
|---|---|
| 決策 1（觀看端跑 AI） | Task 3（`ai_query_ctx` 用觀看端 ctx）、Task 6（`queryFn` 走 `invokeAiQueryCtx`） |
| 決策 2（Agent 迴圈 + `/ai`=maxSteps 1） | Task 6 Step 4（`handleWarpSubmit`） |
| 決策 3（cwd 靠 recent_output / `pwd`） | Task 3（`cwd: None`）、Task 6 Step 3（`buildRemoteCtx` 不填 cwd） |
| 決策 4（安全模型比照本機） | Task 1 保留 `shouldAutoExecute`；Task 6 Step 8 危險指令走 `CommandPreview` |
| 決策 5（抽共用 + 新 Rust 指令，不複製） | Task 1、Task 2、Task 3 |
| 決策 6（輕量面板，不重用 AiPanel） | Task 5 |
| 元件一（`ai_query_ctx` + `RemoteCtx` + `snapshot_from_*` + 註冊 + IPC 包裝） | Task 3、Task 4 |
| 元件一（`recent_output` 讀 xterm buffer 末 ~4000 字） | Task 6 Step 3（`readRecentOutput`） |
| 元件二（`agentLoop.ts` 抽出 + `queryFn` 注入 + 4 呼叫點 + 風險控管） | Task 1 |
| 元件三（`AgentPanel` Props / 版面 / `index.tsx` 變更 / 移除佔位 / 唯讀停用） | Task 5、Task 6 |
| 邊角：OSC 133 缺失 → 逾時 | `runAgentLoop` 既有 60s 逾時（Task 1 原封保留）；文案 `remote_agent_no_shell_integration`（Task 5 Step 4）＋ Task 7 Step 5.? 手動驗證。**補強**：逾時訊息目前用 `term_agent_timeout`，面板未特別顯示 shell integration 提示——見下方 gap。 |
| 邊角：resync / 收回控制權 / 連線結束中止 | Task 6 Step 7 |
| 邊角：任務中 WarpInput 打字中斷 | 見下方 gap |
| 邊角：關面板＝中止 | Task 6 Step 8（`onClose`） |
| 邊角：未設 provider | `runAgentLoop` → `onAiError` → `onFail`（Task 1 保留）；Task 7 Step 5.6 手動驗證 |
| 測試（Rust / 前端 / 手動） | Task 3 Step 4、各 Task 的測試步驟、Task 7 |
| i18n 移除 `remote_terminal_ai_unsupported` | Task 6 Step 9-10 |

**發現的 gap，已在計畫內補上：**

- **任務進行中在 `WarpInput` 打普通指令要中斷任務**：`handleWarpSubmit`（Task 6 Step 4）目前非前綴就直接 `submitCommand(cmd)`，沒有中斷。**修正**：`handleWarpSubmit` 開頭加 `if (agentMission?.active) { stopAgentMission(); return; }`（在解析前綴之前），並把 `agentMission` 加進 `useCallback` 依賴。Task 7 Step 5.3 是它的驗證。
- **逾時時面板要顯示 shell integration 提示**：`runAgentLoop` 逾時走 `onFail(t.term_agent_timeout_fail)`。**修正**：Task 6 Step 4 的 `onFail` 改成 `onFail: (msg) => { const reason = msg === t.term_agent_timeout_fail ? t.remote_agent_no_shell_integration : msg; setAgentPhase({ phase: "failed", reason }); stopMission(); }`。

**2. Placeholder scan**：Task 1 Step 1 對 `handleAiQuery` 的 `.then/.catch` 主體寫「與現狀一致，不改」——這是**搬移**指示（原碼在 `TerminalView.tsx:2147-2218`，逐字搬），不是要工作者自行發明邏輯，可接受。其餘步驟均有完整程式碼。

**3. Type consistency**：
- `RemoteCtx`：Rust `snake_case`（`recent_output`），前端 TS `camelCase`（`recentOutput`），`invokeAiQueryCtx` 內做轉換——Task 3 / Task 4 一致。
- `queryFn: (query: string) => Promise<AiCommandReady>`：Task 1（`AgentLoopParams` / `handleAiQuery`）、Task 6（`startAgentMission` 傳入）簽名一致。
- `AgentPhase`：沿用 `../AgentStatusBar` export，Task 5 / Task 6 一致（`done` 帶 `steps`、`failed` 帶 `reason`）。
- `run_single_command(snapshot, query, locale, &router, &app, stream_id)`：Task 2 定義、Task 2/Task 3 兩個呼叫點參數順序一致。
- `snapshot_from_remote_ctx(os, shell: Option, cwd: Option, recent_output: Option<String>)`：Task 3 Step 1 定義、Step 2 呼叫一致。
- `useAgentMission` 回傳 `agentMission`（非 `mission`）——Task 6 解構時用 `agentMission`，傳給 `AgentPanel` 的 prop 名是 `mission`（Task 5 Props 定義），一致。
