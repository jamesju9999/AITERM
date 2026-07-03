# LoopStudio 強化階段 A 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修正 sub-agent tool-calling 歷史格式、以跨平台後端 `agent_exec` 取代 PTY sentinel、加入安全閘門（寫檔路徑限制＋危險指令暫停確認）、給 Verifier 唯讀工具。

**Architecture:** 前端 `useSubAgentLoop.ts` 重構出共用的 `runToolLoop`（正規 OpenAI tool-calling 歷史格式 + 工具執行器），sub-agent 與 Verifier 共用；指令執行改走新的 Rust `agent_exec` command（`tokio::process`，Unix `sh -c` / Windows `cmd /C`）；危險指令由規則式 `commandRisk.ts` 分類，經 `onConfirmNeeded` callback 上傳到 `useOrchestratorLoop` 的 `pendingConfirmation` state，由 LoopStudio UI 顯示允許/拒絕。

**Tech Stack:** React 19 hooks、Tauri 2 IPC、Rust tokio、Vitest、cargo test。

**Spec:** `docs/superpowers/specs/2026-07-03-loopstudio-hardening-phase-a-design.md`

---

## 檔案結構

| 檔案 | 動作 | 職責 |
|------|------|------|
| `src/lib/commandRisk.ts` | 新增 | 規則式指令危險分類器（純函式） |
| `src/lib/commandRisk.test.ts` | 新增 | 分類規則測試 |
| `src/lib/pathUtils.ts` | 新增 | 路徑 normalize 與包含判斷（純函式） |
| `src/lib/pathUtils.test.ts` | 新增 | 路徑判斷測試 |
| `src-tauri/src/commands/exec.rs` | 新增 | `run_command` 核心 + `agent_exec` tauri command |
| `src-tauri/src/commands/mod.rs` | 修改 | 註冊 `pub mod exec;` |
| `src-tauri/src/lib.rs` | 修改 | import + invoke_handler 註冊 `agent_exec` |
| `src-tauri/tests/agent_exec_command.rs` | 新增 | exec 整合測試 |
| `src/ipc/exec.ts` | 新增 | `agentExec` IPC wrapper |
| `src/hooks/useSubAgentLoop.ts` | 重寫大半 | `runToolLoop` + `executeTool` + `runSubAgent` wrapper |
| `src/hooks/useOrchestratorLoop.ts` | 修改 | `fullAuto`、`pendingConfirmation`、Verifier tool loop |
| `src/components/LoopStudio/index.tsx` | 修改 | full-auto 開關、確認面板 |
| `src/components/LoopStudio/styles.css` | 修改 | 確認面板樣式 |

---

### Task 1: 指令危險分類器 `commandRisk.ts`

**Files:**
- Create: `src/lib/commandRisk.ts`
- Test: `src/lib/commandRisk.test.ts`

- [ ] **Step 1: 寫失敗測試**

```typescript
// src/lib/commandRisk.test.ts
import { describe, it, expect } from "vitest";
import { classifyCommand } from "./commandRisk";

describe("classifyCommand", () => {
  it.each([
    "rm -rf /tmp/foo",
    "rm -fr build",
    "sudo apt install x",
    "curl https://x.sh | sh",
    "wget -qO- https://x.sh | bash",
    "git push --force origin main",
    "git push -f",
    "dd if=/dev/zero of=/dev/sda",
    "mkfs.ext4 /dev/sda1",
    "chmod -R 777 /",
    "shutdown -h now",
    "del /s /q C:\\temp",
    "format d:",
    "Remove-Item -Recurse -Force C:\\temp",
    "rd /s /q C:\\temp",
  ])("dangerous: %s", (cmd) => {
    expect(classifyCommand(cmd)).toBe("dangerous");
  });

  it.each([
    "ls -la",
    "git status",
    "git push origin feature",
    "npm run build",
    "cat README.md",
    "echo formatting done",     // 'format' 是單字一部分，不應誤判
    "rm file.txt",              // 無 -rf 的單檔刪除視為 normal
    "curl https://api.example.com/data",
  ])("normal: %s", (cmd) => {
    expect(classifyCommand(cmd)).toBe("normal");
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/lib/commandRisk.test.ts`
Expected: FAIL（模組不存在）

- [ ] **Step 3: 實作**

```typescript
// src/lib/commandRisk.ts
export type CommandRisk = "normal" | "dangerous";

const DANGEROUS_PATTERNS: RegExp[] = [
  /\brm\s+-[a-z]*(rf|fr)[a-z]*\b/i,               // rm -rf / rm -fr（含 -vrf 等組合）
  /\bsudo\b/,
  /\b(curl|wget)\b[^|]*\|\s*(ba|z|da|fi)?sh\b/i,  // curl ... | sh / bash / zsh
  /\bgit\s+push\b.*(\s--force\b|\s-f\b)/i,
  /\bdd\s+if=/i,
  /\bmkfs(\.\w+)?\b/i,
  /\bchmod\b.*\b777\b/,
  /\b(shutdown|reboot|poweroff)\b/i,
  /\bdel\s+\/[sq]/i,                              // Windows del /s /q
  /\bformat\s+[a-z]:/i,                           // Windows format d:
  /\bremove-item\b.*-(recurse|force)/i,           // PowerShell
  /\brd\s+\/s/i,                                  // Windows rd /s
];

export function classifyCommand(command: string): CommandRisk {
  return DANGEROUS_PATTERNS.some((p) => p.test(command)) ? "dangerous" : "normal";
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/lib/commandRisk.test.ts`
Expected: PASS（全部案例）

- [ ] **Step 5: Commit**

```bash
git add src/lib/commandRisk.ts src/lib/commandRisk.test.ts
git commit -m "feat(loop-studio): add rule-based command risk classifier"
```

---

### Task 2: 路徑包含判斷 `pathUtils.ts`

**Files:**
- Create: `src/lib/pathUtils.ts`
- Test: `src/lib/pathUtils.test.ts`

- [ ] **Step 1: 寫失敗測試**

```typescript
// src/lib/pathUtils.test.ts
import { describe, it, expect } from "vitest";
import { isPathInside } from "./pathUtils";

describe("isPathInside", () => {
  it("accepts direct child", () => {
    expect(isPathInside("/home/user/proj/src/a.ts", "/home/user/proj")).toBe(true);
  });
  it("accepts root itself", () => {
    expect(isPathInside("/home/user/proj", "/home/user/proj")).toBe(true);
  });
  it("rejects sibling", () => {
    expect(isPathInside("/home/user/other/a.ts", "/home/user/proj")).toBe(false);
  });
  it("rejects prefix-only match", () => {
    expect(isPathInside("/home/user/proj2/a.ts", "/home/user/proj")).toBe(false);
  });
  it("resolves .. escape", () => {
    expect(isPathInside("/home/user/proj/../secrets.txt", "/home/user/proj")).toBe(false);
  });
  it("resolves .. staying inside", () => {
    expect(isPathInside("/home/user/proj/src/../a.ts", "/home/user/proj")).toBe(true);
  });
  it("handles windows backslash + case-insensitive drive paths", () => {
    expect(isPathInside("C:\\Proj\\src\\a.ts", "c:\\proj")).toBe(true);
    expect(isPathInside("C:\\Other\\a.ts", "C:\\Proj")).toBe(false);
  });
  it("trailing slash on root", () => {
    expect(isPathInside("/home/user/proj/a.ts", "/home/user/proj/")).toBe(true);
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/lib/pathUtils.test.ts`
Expected: FAIL（模組不存在）

- [ ] **Step 3: 實作**

```typescript
// src/lib/pathUtils.ts
/** Normalize separators to "/" and resolve "." / ".." segments (no filesystem access). */
export function normalizePath(p: string): string {
  const unified = p.replace(/\\/g, "/");
  const isAbsolute = unified.startsWith("/");
  const out: string[] = [];
  for (const part of unified.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") {
      if (out.length > 0 && out[out.length - 1] !== "..") out.pop();
      continue;
    }
    out.push(part);
  }
  return (isAbsolute ? "/" : "") + out.join("/");
}

/** True if `child` resolves to a location at or under `root`. Windows drive paths compare case-insensitively. */
export function isPathInside(child: string, root: string): boolean {
  let c = normalizePath(child);
  let r = normalizePath(root);
  // Windows drive-letter paths (e.g. "C:/...") are case-insensitive
  const isWindowsPath = /^[a-zA-Z]:\//.test(c) || /^[a-zA-Z]:\//.test(r);
  if (isWindowsPath) {
    c = c.toLowerCase();
    r = r.toLowerCase();
  }
  return c === r || c.startsWith(r.endsWith("/") ? r : r + "/");
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/lib/pathUtils.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/lib/pathUtils.ts src/lib/pathUtils.test.ts
git commit -m "feat(loop-studio): add path containment helper for write confinement"
```

---

### Task 3: Rust `agent_exec` command

**Files:**
- Create: `src-tauri/src/commands/exec.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`（`use commands::{...}` 區塊約 :18-71、`generate_handler!` 約 :221 起）
- Test: `src-tauri/tests/agent_exec_command.rs`

- [ ] **Step 1: 寫失敗測試**

```rust
// src-tauri/tests/agent_exec_command.rs
use aiterm_lib::commands::exec::run_command;

#[tokio::test]
async fn runs_simple_command_and_captures_stdout() {
    let r = run_command("echo hello_exec", None, 10_000).await.unwrap();
    assert!(r.stdout.contains("hello_exec"));
    assert_eq!(r.exit_code, Some(0));
    assert!(!r.timed_out);
}

#[tokio::test]
async fn reports_nonzero_exit_code() {
    let r = run_command("exit 3", None, 10_000).await.unwrap();
    assert_eq!(r.exit_code, Some(3));
    assert!(!r.timed_out);
}

#[tokio::test]
async fn captures_stderr() {
    #[cfg(windows)]
    let cmd = "echo oops 1>&2";
    #[cfg(not(windows))]
    let cmd = "echo oops >&2";
    let r = run_command(cmd, None, 10_000).await.unwrap();
    assert!(r.stderr.contains("oops"));
}

#[tokio::test]
async fn kills_on_timeout() {
    #[cfg(windows)]
    let cmd = "ping -n 30 127.0.0.1 >NUL";
    #[cfg(not(windows))]
    let cmd = "sleep 30";
    let start = std::time::Instant::now();
    let r = run_command(cmd, None, 500).await.unwrap();
    assert!(r.timed_out);
    assert!(start.elapsed().as_secs() < 5, "kill must not wait for the child");
}

#[tokio::test]
async fn respects_cwd() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let cmd = "cd";
    #[cfg(not(windows))]
    let cmd = "pwd";
    let r = run_command(cmd, Some(dir.path().to_str().unwrap()), 10_000).await.unwrap();
    // canonical 路徑在 macOS 可能有 /private 前綴，用資料夾名比對
    let dir_name = dir.path().file_name().unwrap().to_str().unwrap();
    assert!(r.stdout.contains(dir_name));
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test agent_exec_command`
Expected: 編譯錯誤（`commands::exec` 不存在）

- [ ] **Step 3: 實作 `exec.rs`**

```rust
// src-tauri/src/commands/exec.rs
use serde::Serialize;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const MAX_OUTPUT_CHARS: usize = 10_000;

#[derive(Debug, Serialize)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

/// Run a shell command with a hard timeout, capturing stdout/stderr.
/// Unix: `sh -c`, Windows: `cmd /C`. Output is truncated to MAX_OUTPUT_CHARS.
pub async fn run_command(
    command: &str,
    cwd: Option<&str>,
    timeout_ms: u64,
) -> Result<ExecResult, String> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let mut stdout_pipe = child.stdout.take().ok_or("failed to capture stdout")?;
    let mut stderr_pipe = child.stderr.take().ok_or("failed to capture stderr")?;

    // Read pipes concurrently so a killed child still yields partial output.
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf).await;
        buf
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf).await;
        buf
    });

    let (exit_code, timed_out) = match timeout(Duration::from_millis(timeout_ms), child.wait()).await {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => return Err(format!("wait failed: {e}")),
        Err(_) => {
            let _ = child.kill().await;
            (None, true)
        }
    };

    let stdout_buf = stdout_task.await.unwrap_or_default();
    let stderr_buf = stderr_task.await.unwrap_or_default();

    Ok(ExecResult {
        stdout: truncate_lossy(&stdout_buf),
        stderr: truncate_lossy(&stderr_buf),
        exit_code,
        timed_out,
    })
}

fn truncate_lossy(buf: &[u8]) -> String {
    let s = String::from_utf8_lossy(buf);
    if s.chars().count() > MAX_OUTPUT_CHARS {
        let mut t: String = s.chars().take(MAX_OUTPUT_CHARS).collect();
        t.push_str("\n[...truncated]");
        t
    } else {
        s.into_owned()
    }
}

#[tauri::command]
pub async fn agent_exec(
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<ExecResult, String> {
    run_command(&command, cwd.as_deref(), timeout_ms.unwrap_or(60_000)).await
}
```

- [ ] **Step 4: 註冊模組與 command**

`src-tauri/src/commands/mod.rs` 加入（依字母序放在 `pub mod enterprise;` 後）：

```rust
pub mod exec;
```

`src-tauri/src/lib.rs`：
1. 確認 `commands` 模組宣告為 `pub mod commands;`（integration test 需要 `aiterm_lib::commands::exec`）；若是 `mod commands;` 改為 pub。
2. 在 `use commands::{ ... }` 區塊加入 `exec::agent_exec,`。
3. 在 `generate_handler![` 清單的 `agent_chat,` 之後加入 `agent_exec,`。

- [ ] **Step 5: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test agent_exec_command`
Expected: 5 tests PASS（timeout 測試約需 0.5 秒）

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/exec.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/tests/agent_exec_command.rs
git commit -m "feat(loop-studio): add cross-platform agent_exec backend command"
```

---

### Task 4: 前端 IPC wrapper `exec.ts`

**Files:**
- Create: `src/ipc/exec.ts`

- [ ] **Step 1: 實作 wrapper**

```typescript
// src/ipc/exec.ts
import { invoke } from "@tauri-apps/api/core";

export interface ExecResult {
  stdout: string;
  stderr: string;
  exit_code: number | null;
  timed_out: boolean;
}

/** Run a shell command via the backend (sh -c / cmd /C). Default timeout 60s. */
export const agentExec = (
  command: string,
  cwd?: string,
  timeoutMs?: number,
): Promise<ExecResult> =>
  invoke<ExecResult>("agent_exec", {
    command,
    cwd: cwd ?? null,
    timeoutMs: timeoutMs ?? null,
  });
```

- [ ] **Step 2: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤

- [ ] **Step 3: Commit**

```bash
git add src/ipc/exec.ts
git commit -m "feat(loop-studio): add agentExec IPC wrapper"
```

---

### Task 5: 重構 `useSubAgentLoop.ts` — runToolLoop + 正規歷史格式 + 安全閘門

一次完成 A1（歷史格式）、A2 前端接線（exec 取代 PTY）、A3 工具端閘門（寫檔限制＋危險指令確認），因為三者都落在同一個工具執行迴圈裡，分開改會互相衝突。

**Files:**
- Modify: `src/hooks/useSubAgentLoop.ts`（整檔重構）

- [ ] **Step 1: 重寫 `useSubAgentLoop.ts`**

```typescript
// src/hooks/useSubAgentLoop.ts
import { agentChat, type AgentToolDefinition, type ChatMessage } from "../ipc/ai";
import { readFile, writeTextFile, listDirectory, getSessionCwd } from "../ipc/fs";
import { agentExec } from "../ipc/exec";
import { classifyCommand } from "../lib/commandRisk";
import { isPathInside } from "../lib/pathUtils";

export function serializeError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  try { return JSON.stringify(e); } catch { return String(e); }
}

export interface AgentDefinition {
  name: string;
  providerId: string;
  roleDescription: string;
  tools: AgentToolName[];
}

export type AgentToolName = "read_file" | "write_file" | "list_directory" | "execute_command";

export interface SubAgentAction {
  tool: string;
  input: string;
  output: string;
  isError: boolean;
}

export interface SubAgentResult {
  answer: string;
  actions: SubAgentAction[];
  isError: boolean;
}

/** Context shared by all tool executions in one loop run. */
export interface ToolExecutionContext {
  sessionId: string;
  /** projectDir ?? session CWD — write_file confinement root and execute_command cwd. Null disables both. */
  effectiveRoot: string | null;
  /** Called before running a dangerous command. Resolve false to deny. Absent = deny dangerous commands. */
  onConfirmNeeded?: (command: string) => Promise<boolean>;
}

const TOOL_DEFS: Record<AgentToolName, AgentToolDefinition> = {
  read_file: {
    name: "read_file",
    description: "Read the contents of a text file at the given absolute path.",
    input_schema: {
      type: "object",
      properties: { path: { type: "string", description: "Absolute file path to read" } },
      required: ["path"],
    },
  },
  write_file: {
    name: "write_file",
    description: "Write or overwrite a text file at the given absolute path.",
    input_schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "Absolute file path to write" },
        content: { type: "string", description: "Content to write" },
      },
      required: ["path", "content"],
    },
  },
  list_directory: {
    name: "list_directory",
    description: "List entries in a directory. Pass an empty string to list the current working directory.",
    input_schema: {
      type: "object",
      properties: { path: { type: "string", description: "Directory path, or empty string for CWD" } },
      required: ["path"],
    },
  },
  execute_command: {
    name: "execute_command",
    description: "Execute a shell command and return its output.",
    input_schema: {
      type: "object",
      properties: { command: { type: "string", description: "Shell command to execute" } },
      required: ["command"],
    },
  },
};

async function executeTool(
  name: AgentToolName,
  args: Record<string, string>,
  ctx: ToolExecutionContext,
): Promise<{ result: string; isError: boolean }> {
  try {
    switch (name) {
      case "read_file": {
        const { content, truncated } = await readFile(args.path);
        return { result: truncated ? `${content}\n[...truncated]` : content, isError: false };
      }
      case "write_file": {
        if (ctx.effectiveRoot && !isPathInside(args.path, ctx.effectiveRoot)) {
          return {
            result: `錯誤：不允許寫入專案目錄（${ctx.effectiveRoot}）以外的路徑：${args.path}。請改用專案目錄內的路徑。`,
            isError: true,
          };
        }
        await writeTextFile(args.path, args.content);
        return { result: `Successfully wrote ${args.path}`, isError: false };
      }
      case "list_directory": {
        const entries = await listDirectory(ctx.sessionId, args.path ?? "");
        return {
          result: entries.map(e => (e.is_dir ? `${e.name}/` : e.name)).join("\n") || "(empty)",
          isError: false,
        };
      }
      case "execute_command": {
        if (classifyCommand(args.command) === "dangerous") {
          if (!ctx.onConfirmNeeded) {
            return { result: "錯誤：此指令被判定為危險指令，此執行環境不允許危險指令。", isError: true };
          }
          const approved = await ctx.onConfirmNeeded(args.command);
          if (!approved) {
            return { result: "使用者拒絕執行此指令，請改用其他方式完成任務。", isError: true };
          }
        }
        const r = await agentExec(args.command, ctx.effectiveRoot ?? undefined);
        const combined = [r.stdout, r.stderr ? `[stderr]\n${r.stderr}` : ""].filter(Boolean).join("\n");
        if (r.timed_out) {
          return { result: `[timeout after 60s — process killed]\n${combined}`, isError: true };
        }
        if (r.exit_code !== 0) {
          return { result: `[exit code ${r.exit_code}]\n${combined}`, isError: true };
        }
        return { result: combined || "(no output)", isError: false };
      }
    }
  } catch (e) {
    return { result: `Error: ${serializeError(e)}`, isError: true };
  }
}

/**
 * Core tool-calling loop shared by sub-agents and the Verifier.
 * Mutates `history` in place using proper OpenAI tool-calling format:
 * one assistant message carrying tool_calls, then one `tool` message per call.
 */
export async function runToolLoop(
  providerId: string,
  history: ChatMessage[],
  enabledTools: AgentToolName[],
  ctx: ToolExecutionContext,
  maxIterations: number,
  onAction?: (action: SubAgentAction) => void,
): Promise<SubAgentResult> {
  const actions: SubAgentAction[] = [];
  const toolDefs = enabledTools.map(t => TOOL_DEFS[t]);
  const limit = maxIterations === 0 ? Infinity : maxIterations;

  for (let i = 0; i < limit; i++) {
    const reply = await agentChat(providerId, history, toolDefs, ctx.sessionId);

    if (reply.tool_calls.length === 0) {
      return { answer: reply.content ?? "", actions, isError: false };
    }

    // One assistant message with ALL tool_calls. Prefer raw_tool_calls
    // (preserves Gemini thought_signature); reconstruct otherwise.
    const assistantToolCalls =
      reply.raw_tool_calls ??
      reply.tool_calls.map((tc, idx) => ({
        id: tc.id || `call_${Date.now()}_${idx}`,
        type: "function" as const,
        function: { name: tc.tool_name, arguments: JSON.stringify(tc.args) },
      }));
    history.push({ role: "assistant", content: null, tool_calls: assistantToolCalls });

    for (let idx = 0; idx < reply.tool_calls.length; idx++) {
      const tc = reply.tool_calls[idx];
      const callId = assistantToolCalls[idx]?.id ?? tc.id ?? `call_${idx}`;

      let result: string;
      let isError: boolean;
      if (!enabledTools.includes(tc.tool_name as AgentToolName)) {
        result = `Tool '${tc.tool_name}' is not enabled for this agent`;
        isError = true;
      } else {
        ({ result, isError } = await executeTool(
          tc.tool_name as AgentToolName,
          tc.args as Record<string, string>,
          ctx,
        ));
      }

      const action: SubAgentAction = {
        tool: tc.tool_name,
        input: JSON.stringify(tc.args),
        output: result,
        isError,
      };
      actions.push(action);
      onAction?.(action);

      history.push({ role: "tool", content: result, tool_call_id: callId });
    }
  }

  const limitMsg = `Sub-agent reached max tool iterations (${maxIterations}) without completing the task.`;
  return { answer: limitMsg, actions, isError: true };
}

export interface RunSubAgentOptions {
  onAction?: (action: SubAgentAction) => void;
  onConfirmNeeded?: (command: string) => Promise<boolean>;
  maxInnerIterations?: number;
  sharedContext?: string;
  projectDir?: string;
}

export async function runSubAgent(
  sessionId: string,
  agent: AgentDefinition,
  task: string,
  options: RunSubAgentOptions = {},
): Promise<SubAgentResult> {
  const { onAction, onConfirmNeeded, maxInnerIterations = 30, sharedContext = "", projectDir } = options;

  const cwd = projectDir
    || await getSessionCwd(sessionId).catch(() => null)
    || null;

  const contextSection = sharedContext
    ? `\n## 先前迭代的累積 Context\n以下是整個任務到目前為止已完成與尚未完成的紀錄，請避免重複已完成的工作：\n${sharedContext}\n`
    : "";

  const systemPrompt = `${agent.roleDescription}

Current working directory: ${cwd ?? "(unknown)"}
All file operations and shell commands MUST be performed under this directory. Use absolute paths when writing files.
${contextSection}
## 指示
使用可用的工具完成指派的任務。完成後，用繁體中文回報：
1. 你做了什麼（具體行動）
2. 結果是什麼（具體產出或發現）
3. 是否有任何問題或未完成的部分

不要重複 Context 中已標記為完成的工作。`;

  const history: ChatMessage[] = [
    { role: "system", content: systemPrompt },
    { role: "user", content: task },
  ];

  return runToolLoop(
    agent.providerId,
    history,
    agent.tools,
    { sessionId, effectiveRoot: cwd, onConfirmNeeded },
    maxInnerIterations,
    onAction,
  );
}
```

注意：
- `writePty`、`getPtyRecentOutput` import 與 `executeCommandInPty` 已整個移除。
- tool result 不再加 `Tool result for ${name}:` 前綴——`tool` role + `tool_call_id` 已明確對應。
- `cwd` 為 null 時（拿不到 session CWD）寫檔限制與 exec cwd 都停用，維持可用性。

- [ ] **Step 2: 型別檢查（預期 useOrchestratorLoop 呼叫端錯誤）**

Run: `npx tsc --noEmit`
Expected: `useOrchestratorLoop.ts` 中 `runSubAgent(...)` 呼叫簽名錯誤（positional args）。這是預期的，Task 6 修。

- [ ] **Step 3: Commit（與 Task 6 一起 commit，此步跳過）**

---

### Task 6: `useOrchestratorLoop.ts` — fullAuto、pendingConfirmation、Verifier tool loop

**Files:**
- Modify: `src/hooks/useOrchestratorLoop.ts`

- [ ] **Step 1: 更新 imports 與 LoopConfig**

```typescript
// 檔頭 import 改為：
import { useState, useCallback, useRef } from "react";
import { agentChat, type ChatMessage, type AgentToolDefinition } from "../ipc/ai";
import {
  runSubAgent, runToolLoop, serializeError,
  type AgentDefinition, type SubAgentAction,
} from "./useSubAgentLoop";
import { loopSessionSave, loopSessionLoad, parseLoopSessionData } from "../ipc/loopSession";
```

`LoopConfig` 加欄位（放在 `projectDir?: string;` 後）：

```typescript
  /** true = 跳過危險指令確認，全自動執行 */
  fullAuto?: boolean;
```

- [ ] **Step 2: hook state 加 pendingConfirmation**

`UseOrchestratorLoopResult` 介面加：

```typescript
export interface PendingConfirmation {
  agentName: string;
  command: string;
  resolve: (approved: boolean) => void;
}
```

並在 `UseOrchestratorLoopResult` 增加 `pendingConfirmation: PendingConfirmation | null;`。

hook 內（`abortRef` 宣告後）加：

```typescript
  const [pendingConfirmation, setPendingConfirmation] = useState<PendingConfirmation | null>(null);
  const pendingResolveRef = useRef<((approved: boolean) => void) | null>(null);

  const requestConfirmation = useCallback((agentName: string, command: string): Promise<boolean> => {
    return new Promise<boolean>((resolve) => {
      const wrapped = (approved: boolean) => {
        setPendingConfirmation(null);
        pendingResolveRef.current = null;
        resolve(approved);
      };
      pendingResolveRef.current = wrapped;
      setPendingConfirmation({ agentName, command, resolve: wrapped });
    });
  }, []);
```

`stop()` 改為同時打斷等待中的確認（視為拒絕）：

```typescript
  const stop = useCallback(() => {
    abortRef.current = true;
    pendingResolveRef.current?.(false);
  }, []);
```

回傳值加上 `pendingConfirmation`：

```typescript
  return { trace, isRunning, iteration, start, stop, resume, pendingConfirmation };
```

- [ ] **Step 3: 更新 runSubAgent 呼叫（改 options object + 確認 callback）**

`start` 內找到 `const result = await runSubAgent(...)`（原 :440-448），改為：

```typescript
              const confirmFn = config.fullAuto
                ? async () => true
                : (command: string) => requestConfirmation(args.agent_name, command);
              const result = await runSubAgent(
                config.sessionId,
                targetAgent,
                args.task,
                {
                  onAction: (action) => addTraceBuffered({ kind: "sub_agent_action", agentName: args.agent_name, text: action.tool, actions: [action], iteration: iter }),
                  onConfirmNeeded: confirmFn,
                  maxInnerIterations: config.maxInnerIterations,
                  sharedContext,
                  projectDir: config.projectDir,
                },
              );
```

並把 `start` 的 useCallback 依賴陣列改為 `[addTrace, requestConfirmation]`。

- [ ] **Step 4: Verifier 改用 runToolLoop（唯讀工具）**

`buildVerifierSystemPrompt` 的 `## Instructions` 段前加一段：

```typescript
  return `You are a Verifier AI. Your job is to objectively evaluate progress toward a goal.

## Stopping Condition
${stoppingCondition}

## Available Sub-Agents (ONLY use these names in your suggestion)
${agentList}

## Verification Tools
You have read-only tools (read_file, list_directory). Before concluding, you may
use them to inspect the actual files and verify the Orchestrator's claims.

## Instructions
Analyze the Orchestrator's report and respond ONLY with a JSON object in this exact format:
...（其餘不變）`;
```

`start` 內把原本的（原 :502-507）：

```typescript
        const verifierReply = await agentChat(
          config.verifier.providerId,
          verifierMessages,
          [],
          config.sessionId,
        );

        const verifierResult = parseVerifierResult(verifierReply.content ?? "");
```

改為：

```typescript
        const verifierRun = await runToolLoop(
          config.verifier.providerId,
          verifierMessages,
          ["read_file", "list_directory"],
          { sessionId: config.sessionId, effectiveRoot: config.projectDir ?? null },
          8,
          (action) => addTraceBuffered({ kind: "sub_agent_action", agentName: config.verifier.name, text: action.tool, actions: [action], iteration: iter }),
        );

        const verifierResult = parseVerifierResult(verifierRun.answer);
```

若 `AgentToolDefinition` import 變成未使用，將其自 import 移除。

- [ ] **Step 5: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤（Task 5 遺留的呼叫端錯誤在此消失）

- [ ] **Step 6: 跑既有前端測試確認無迴歸**

Run: `npm run test`
Expected: 全部 PASS（含 Task 1、2 的新測試）

- [ ] **Step 7: Commit**

```bash
git add src/hooks/useSubAgentLoop.ts src/hooks/useOrchestratorLoop.ts
git commit -m "feat(loop-studio): proper tool-calling history, backend exec, safety gates, verifier read-only tools"
```

---

### Task 7: UI — full-auto 開關與危險指令確認面板

**Files:**
- Modify: `src/components/LoopStudio/index.tsx`
- Modify: `src/components/LoopStudio/styles.css`

- [ ] **Step 1: RosterState 加 fullAuto**

`index.tsx` 的 `RosterState` 介面加 `fullAuto: boolean;`，`loadRoster()` 預設值物件與 `handleNewProject` 的 defaults 都加 `fullAuto: false,`。
（舊 localStorage 資料缺此欄位時為 `undefined`，falsy，等同關閉——不需遷移。）

- [ ] **Step 2: handleStart 傳入 fullAuto**

`handleStart` 的 `config: LoopConfig` 物件加一行：

```typescript
      fullAuto: roster.fullAuto ?? false,
```

- [ ] **Step 3: 限制設定區加開關**

在「限制設定」`ls-limits-grid` 之後（`</div>` 結束 grid 後、section 內）加：

```tsx
          <label className="ls-field ls-fullauto-row">
            <input
              type="checkbox"
              checked={roster.fullAuto ?? false}
              onChange={e => updateRoster({ fullAuto: e.target.checked })}
              disabled={loop.isRunning}
            />
            <span className="ls-field-label">
              Full-auto 模式
              <span className="ls-hint-inline">（跳過危險指令確認，僅在信任目標時開啟）</span>
            </span>
          </label>
```

- [ ] **Step 4: 右欄加確認面板**

`index.tsx` 右欄 `<div className="ls-right">` 內、`<ExecutionTrace ...>` 之前加：

```tsx
        {loop.pendingConfirmation && (
          <div className="ls-confirm-panel">
            <div className="ls-confirm-title">
              ⚠ {loop.pendingConfirmation.agentName} 請求執行危險指令
            </div>
            <pre className="ls-confirm-command">{loop.pendingConfirmation.command}</pre>
            <div className="ls-confirm-actions">
              <button
                type="button"
                className="ls-confirm-deny"
                onClick={() => loop.pendingConfirmation?.resolve(false)}
              >
                拒絕
              </button>
              <button
                type="button"
                className="ls-confirm-allow"
                onClick={() => loop.pendingConfirmation?.resolve(true)}
              >
                允許執行
              </button>
            </div>
          </div>
        )}
```

- [ ] **Step 5: styles.css 加樣式**

附加到 `styles.css` 末尾：

```css
.ls-fullauto-row {
  flex-direction: row;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}

.ls-confirm-panel {
  border: 1px solid #b58900;
  border-radius: 6px;
  padding: 12px;
  margin-bottom: 8px;
  background: color-mix(in srgb, #b58900 12%, transparent);
}

.ls-confirm-title {
  font-weight: 600;
  color: #b58900;
  margin-bottom: 8px;
}

.ls-confirm-command {
  background: var(--bg-secondary, #1a1a1a);
  border-radius: 4px;
  padding: 8px;
  font-size: 12px;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-all;
  margin: 0 0 10px 0;
}

.ls-confirm-actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
}

.ls-confirm-deny,
.ls-confirm-allow {
  padding: 5px 14px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 13px;
  border: 1px solid var(--border, #333);
  background: transparent;
  color: var(--text, #ddd);
}

.ls-confirm-allow {
  background: #b58900;
  border-color: #b58900;
  color: #1a1a1a;
  font-weight: 600;
}
```

- [ ] **Step 6: 型別檢查與 lint**

Run: `npx tsc --noEmit && npm run lint`
Expected: 無錯誤

- [ ] **Step 7: Commit**

```bash
git add src/components/LoopStudio/index.tsx src/components/LoopStudio/styles.css
git commit -m "feat(loop-studio): full-auto toggle and dangerous-command confirmation panel"
```

---

### Task 8: 全面驗證

- [ ] **Step 1: 前端測試 + 型別 + lint**

Run: `npm run test && npx tsc --noEmit && npm run lint`
Expected: 全部 PASS / 無錯誤

- [ ] **Step 2: Rust 測試**

Run: `cd src-tauri && cargo test`
Expected: 全部 PASS（含新的 agent_exec_command）

- [ ] **Step 3: 手動測試（`npm run tauri:dev`）**

1. 開 Loop Studio tab，設 1 個 sub-agent（工具全開）＋ Verifier，projectDir 指向一個測試資料夾。
2. 目標：「在專案目錄建立 hello.txt，內容為 hello world」→ 確認 loop 完成、檔案存在、trace 顯示 agent 動作、終端機**沒有**出現指令。
3. 目標改為需要跑指令的任務（如「列出目錄下所有 .md 檔」）→ 確認 execute_command 走後端、輸出正確。
4. 讓 agent 嘗試危險指令（目標：「刪除整個 /tmp/testdir，用 rm -rf」）→ 確認確認面板出現；按「拒絕」→ agent 收到拒絕訊息並繼續；重跑按「允許」→ 執行。
5. 開啟 Full-auto 重跑步驟 4 → 不出現確認面板。
6. 確認面板顯示時按「停止」→ loop 停止，無 hang。
7. 目標寫檔到 projectDir 以外（如 /tmp/outside.txt）→ agent 收到路徑拒絕錯誤。

- [ ] **Step 4: 最終 commit（若手動測試觸發小修正）**

```bash
git add -A src/ src-tauri/src/
git commit -m "fix(loop-studio): phase A manual test fixes"
```

---

## Self-Review 紀錄

- Spec 覆蓋：A1→Task 5、A2→Task 3+4+5、A3→Task 1+2+5+6+7、A4→Task 6。錯誤處理與測試段落皆有對應步驟。
- 型別一致性：`runToolLoop` / `ToolExecutionContext` / `RunSubAgentOptions` / `PendingConfirmation` 簽名在 Task 5、6、7 間一致。
- 已知取捨：Task 5 與 6 之間 tsc 短暫紅燈（呼叫端簽名變更），兩者合併為一個 commit。
