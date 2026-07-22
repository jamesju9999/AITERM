# 終端機區塊渲染架構重構 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把終端機區塊（block）從「疊加在 xterm 畫布上、靠文字比對猜測位置」的 overlay，改為「headless 解析 PTY 輸出 → 獨立 DOM 區塊清單」的架構，同時加上 Warp 風格的常駐標頭列（路徑／git branch／diff 統計／耗時／exit code）。

**Architecture:** 可見的即時 xterm 只保留「目前 prompt／執行中指令」的即時輸出；區塊完成時（OSC 133 `D`）把累積的原始 bytes 丟給一個不掛載 DOM 的 headless xterm 實例解析成結構化逐行資料，清空可見 xterm，並把該資料渲染成獨立的 `TerminalBlockCard`，疊加在一個永久保留的可捲動區塊清單裡。

**Tech Stack:** React 19、`@xterm/xterm` 5.5（含 headless 解析）、Tauri 2（Rust 後端 `git` subprocess）、Vitest + React Testing Library、`cargo test` + `tempfile`。

**Spec:** `docs/superpowers/specs/2026-07-22-terminal-block-rearchitecture-design.md`

> **執行中修正（Task 5 code review 發現）：** 這個專案的 `tsconfig.json` 是 solution-style（`"files": []` + `references`），導致 `npx tsc --noEmit` 對 `src/` 完全不做型別檢查、永遠 exit 0——本計畫先前所有 Task 的「型別檢查」步驟因此都是假通過。**從現在起，本計畫所有型別檢查步驟一律改用 `npx tsc -b`**（等同 `npm run build` 實際執行的檢查）。這個問題也讓 Task 5 移除 `TerminalBlock` 舊欄位（`output`/`startLine`/`endLine`/`startMarker`/`endMarker`）時，遺漏了兩個計畫原本沒列到、但同樣讀取這些欄位的隱藏消費者：`src/components/AiPanel/index.tsx`（已直接修正，`block.output` → `block.rawOutput`）與 `TerminalView.tsx` 裡另外兩處（見 Task 7 新增的 Step 0）。

---

## Task 1: Rust — `GitBlockInfo` 型別與 `quick_block_info` 方法

**Files:**
- Modify: `src-tauri/src/vcs/types.rs`
- Modify: `src-tauri/src/vcs/git.rs`
- Test: `src-tauri/src/vcs/git.rs`（inline `#[cfg(test)]` module）

- [ ] **Step 1: 在 `types.rs` 新增 `GitBlockInfo` struct**

先讀 `src-tauri/src/vcs/types.rs` 確認現有 struct 的 derive 慣例（`BranchEntry` 等），在檔案末尾新增：

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitBlockInfo {
    pub branch: String,
    pub insertions: u32,
    pub deletions: u32,
}
```

- [ ] **Step 2: 在 `git.rs` 加入 import**

在 `src-tauri/src/vcs/git.rs` 第 8-10 行的 import 區塊，把 `types` import 改為：

```rust
use super::types::{
    BlameEntry, BranchEntry, CommitEntry, GitBlockInfo, IssueEntry, PrEntry, VcsResult,
    WorkflowRun,
};
```

- [ ] **Step 3: 寫失敗的測試**

在 `src-tauri/src/vcs/git.rs` 檔案末尾新增測試模組：

```rust
#[cfg(test)]
mod block_info_tests {
    use super::*;
    use std::fs;
    use std::process::Command as StdCommand;

    fn init_repo(dir: &std::path::Path) {
        StdCommand::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.email", "test@test.com"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.name", "Test"]).current_dir(dir).status().unwrap();
    }

    #[tokio::test]
    async fn returns_branch_and_diff_stats_for_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let file_path = dir.path().join("a.txt");
        fs::write(&file_path, "line1\nline2\nline3\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir.path()).status().unwrap();

        // Uncommitted change: +2 insertions, -1 deletion
        fs::write(&file_path, "line1\nline2b\nline3\nline4\nline5\n").unwrap();

        let client = GitClient::new(dir.path().to_string_lossy().to_string(), None);
        let info = client.quick_block_info().await.expect("expected Some for git repo");

        assert!(info.branch == "master" || info.branch == "main");
        assert_eq!(info.insertions, 3);
        assert_eq!(info.deletions, 1);
    }

    #[tokio::test]
    async fn returns_none_for_non_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        let client = GitClient::new(dir.path().to_string_lossy().to_string(), None);
        assert!(client.quick_block_info().await.is_none());
    }
}
```

- [ ] **Step 4: 執行測試確認失敗**

Run: `cd src-tauri && cargo test block_info_tests -- --nocapture`
Expected: FAIL，錯誤訊息為 `no method named 'quick_block_info' found`

- [ ] **Step 5: 實作 `quick_block_info`**

在 `src-tauri/src/vcs/git.rs` 的 `impl GitClient` 區塊內（`fn git(&self, ...)` 私有方法附近，約 352 行前）新增：

```rust
pub async fn quick_block_info(&self) -> Option<GitBlockInfo> {
    let branch_out = self
        .git(&["rev-parse".to_string(), "--abbrev-ref".to_string(), "HEAD".to_string()])
        .ok()?;
    let branch = branch_out.trim().to_string();
    if branch.is_empty() {
        return None;
    }

    let shortstat = self
        .git(&["diff".to_string(), "--shortstat".to_string()])
        .unwrap_or_default();
    let (insertions, deletions) = Self::parse_shortstat(&shortstat);

    Some(GitBlockInfo { branch, insertions, deletions })
}

fn parse_shortstat(s: &str) -> (u32, u32) {
    let mut insertions = 0u32;
    let mut deletions = 0u32;
    for part in s.trim().split(',') {
        let part = part.trim();
        if let Some(num_str) = part.split_whitespace().next() {
            if let Ok(n) = num_str.parse::<u32>() {
                if part.contains("insertion") {
                    insertions = n;
                } else if part.contains("deletion") {
                    deletions = n;
                }
            }
        }
    }
    (insertions, deletions)
}
```

- [ ] **Step 6: 執行測試確認通過**

Run: `cd src-tauri && cargo test block_info_tests -- --nocapture`
Expected: PASS（2 個測試）

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/vcs/types.rs src-tauri/src/vcs/git.rs
git commit -m "feat(vcs): add GitClient::quick_block_info for terminal block headers"
```

---

## Task 2: Rust — 註冊 `vcs_get_block_info` Tauri command

**Files:**
- Modify: `src-tauri/src/commands/vcs.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 在 `commands/vcs.rs` 新增 command**

讀取 `src-tauri/src/commands/vcs.rs` 確認 `GitClient`/`GitBlockInfo` 的 import 路徑，在檔案末尾（`vcs_detect_repo` 或 `vcs_query` 附近）新增：

```rust
#[tauri::command]
pub async fn vcs_get_block_info(cwd: String) -> Option<crate::vcs::types::GitBlockInfo> {
    let client = crate::vcs::git::GitClient::new(cwd, None);
    client.quick_block_info().await
}
```

- [ ] **Step 2: 在 `lib.rs` 註冊 command**

Modify `src-tauri/src/lib.rs:68-71`（import 區塊），加入 `vcs_get_block_info`：

```rust
    vcs::{
        pick_folder, vcs_add_connection, vcs_agent_step, vcs_detect_repo, vcs_get_block_info,
        vcs_list_connections, vcs_query, vcs_remove_connection, vcs_test_connection,
        vcs_update_connection,
    },
```

Modify `src-tauri/src/lib.rs:344-352`（`generate_handler!` 清單），在 `vcs_query,` 後加入：

```rust
            vcs_query,
            vcs_get_block_info,
            vcs_agent_step,
```

- [ ] **Step 3: 確認編譯通過**

Run: `cd src-tauri && cargo build 2>&1 | tail -30`
Expected: 編譯成功，無錯誤

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/vcs.rs src-tauri/src/lib.rs
git commit -m "feat(vcs): register vcs_get_block_info tauri command"
```

---

## Task 3: 前端 IPC wrapper

**Files:**
- Modify: `src/ipc/vcs.ts`

- [ ] **Step 1: 新增型別與 wrapper 函式**

在 `src/ipc/vcs.ts` 的型別區塊（`VcsConnectionInfo` 附近）新增：

```ts
export interface GitBlockInfo {
  branch: string;
  insertions: number;
  deletions: number;
}
```

在檔案末尾（緊接 `vcs_detect_repo`/`vcs_query` wrapper 附近）新增：

```ts
export function getGitBlockInfo(cwd: string): Promise<GitBlockInfo | null> {
  return invoke("vcs_get_block_info", { cwd });
}
```

- [ ] **Step 2: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤

- [ ] **Step 3: Commit**

```bash
git add src/ipc/vcs.ts
git commit -m "feat(ipc): add getGitBlockInfo wrapper for vcs_get_block_info"
```

---

## Task 4: `ansiBlockParser` — headless ANSI 解析工具

**Files:**
- Create: `src/lib/ansiBlockParser.ts`
- Test: `src/lib/ansiBlockParser.test.ts`

> 註：spec 草稿中原命名為 `useHeadlessAnsiParser.ts`（hook 形式），但這段邏輯不持有 React state，改成一般 async 工具函式更單純，放進 `src/lib/`（與 `commandRisk.ts`、`cmdParser.ts` 等既有工具同層）。

- [ ] **Step 1: 寫失敗的測試**

Create `src/lib/ansiBlockParser.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { parseAnsiToRenderedLines } from "./ansiBlockParser";

describe("parseAnsiToRenderedLines", () => {
  it("splits plain multi-line text into one RenderedLine per line", async () => {
    const lines = await parseAnsiToRenderedLines("hello\r\nworld\r\n", 80);
    expect(lines.length).toBeGreaterThanOrEqual(2);
    expect(lines[0].spans.map((s) => s.text).join("")).toBe("hello");
    expect(lines[1].spans.map((s) => s.text).join("")).toBe("world");
  });

  it("captures ANSI foreground color as a styled span", async () => {
    const lines = await parseAnsiToRenderedLines("\x1b[32mgreen\x1b[0m plain\r\n", 80);
    const spans = lines[0].spans;
    const greenSpan = spans.find((s) => s.text === "green");
    expect(greenSpan?.fg).toBe("#0dbc79");
    const plainSpan = spans.find((s) => s.text.includes("plain"));
    expect(plainSpan?.fg).toBeUndefined();
  });

  it("captures bold attribute", async () => {
    const lines = await parseAnsiToRenderedLines("\x1b[1mbold text\x1b[0m\r\n", 80);
    const boldSpan = lines[0].spans.find((s) => s.text === "bold text");
    expect(boldSpan?.bold).toBe(true);
  });

  it("trims trailing unstyled blank content from each line", async () => {
    const lines = await parseAnsiToRenderedLines("hi\r\n", 80);
    const totalText = lines[0].spans.map((s) => s.text).join("");
    expect(totalText).toBe("hi");
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npm run test -- src/lib/ansiBlockParser.test.ts`
Expected: FAIL，`Cannot find module './ansiBlockParser'`

- [ ] **Step 3: 實作 `ansiBlockParser.ts`**

Create `src/lib/ansiBlockParser.ts`:

```ts
import { Terminal } from "@xterm/xterm";

export interface RenderedSpan {
  text: string;
  fg?: string;
  bg?: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
}

export interface RenderedLine {
  spans: RenderedSpan[];
}

const ANSI_PALETTE = [
  "#000000", "#cd3131", "#0dbc79", "#e5e510",
  "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
  "#666666", "#f14c4c", "#23d18b", "#f5f543",
  "#3b8eea", "#d670d6", "#29b8db", "#e5e5e5",
];

function paletteColor(value: number): string {
  if (value < 16) return ANSI_PALETTE[value];
  // 256-color extended palette: fall back to a grayscale-ish approximation
  // rather than pulling in a full xterm-256 table for a rarely-used range.
  const gray = Math.min(255, value * 3).toString(16).padStart(2, "0");
  return `#${gray}${gray}${gray}`;
}

function cellColor(isRGB: boolean, isPalette: boolean, isDefault: boolean, value: number): string | undefined {
  if (isDefault) return undefined;
  if (isRGB) return `#${value.toString(16).padStart(6, "0")}`;
  if (isPalette) return paletteColor(value);
  return undefined;
}

function spansEqual(a: RenderedSpan, b: Omit<RenderedSpan, "text">): boolean {
  return a.fg === b.fg && a.bg === b.bg && a.bold === b.bold && a.italic === b.italic && a.underline === b.underline;
}

/**
 * Parses raw ANSI byte output (as captured from a PTY) into structured,
 * styled lines by replaying it through a headless (unmounted) xterm.js
 * instance — reusing xterm's own battle-tested ANSI/VT parser instead of
 * hand-rolling one.
 */
export async function parseAnsiToRenderedLines(raw: string, cols: number, rows = 30): Promise<RenderedLine[]> {
  const term = new Terminal({ cols, rows, scrollback: 10000, convertEol: false, allowProposedApi: true });

  await new Promise<void>((resolve) => term.write(raw, resolve));

  const buffer = term.buffer.active;
  const lines: RenderedLine[] = [];

  for (let y = 0; y < buffer.length; y++) {
    const line = buffer.getLine(y);
    if (!line) continue;

    const spans: RenderedSpan[] = [];
    let current: RenderedSpan | null = null;

    for (let x = 0; x < cols; x++) {
      const cell = line.getCell(x);
      if (!cell) continue;

      const chars = cell.getChars() || " ";
      const style: Omit<RenderedSpan, "text"> = {
        fg: cellColor(cell.isFgRGB(), cell.isFgPalette(), cell.isFgDefault(), cell.getFgColor()),
        bg: cellColor(cell.isBgRGB(), cell.isBgPalette(), cell.isBgDefault(), cell.getBgColor()),
        bold: !!cell.isBold(),
        italic: !!cell.isItalic(),
        underline: !!cell.isUnderline(),
      };

      if (current && spansEqual(current, style)) {
        current.text += chars;
      } else {
        current = { text: chars, ...style };
        spans.push(current);
      }
    }

    // Drop trailing unstyled whitespace so blank cells past the real
    // content don't inflate every line to the full terminal width.
    while (spans.length && !spans[spans.length - 1].fg && !spans[spans.length - 1].bg && spans[spans.length - 1].text.trim() === "") {
      spans.pop();
    }

    lines.push({ spans });
  }

  // Drop trailing fully-empty lines (unwritten rows past the last content).
  while (lines.length && lines[lines.length - 1].spans.length === 0) {
    lines.pop();
  }

  term.dispose();
  return lines;
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npm run test -- src/lib/ansiBlockParser.test.ts`
Expected: PASS（4 個測試）

- [ ] **Step 5: Commit**

```bash
git add src/lib/ansiBlockParser.ts src/lib/ansiBlockParser.test.ts
git commit -m "feat(terminal): add headless ANSI-to-styled-lines parser"
```

---

## Task 5: `useTerminalBlocks` — 資料結構擴充與 rawOutput 累積

**Files:**
- Modify: `src/hooks/useTerminalBlocks.ts`
- Test: `src/hooks/useTerminalBlocks.test.ts`（新檔案）

- [ ] **Step 1: 寫失敗的測試**

Create `src/hooks/useTerminalBlocks.test.ts`:

```ts
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";

const writePtyMock = vi.fn();
vi.mock("../ipc/pty", () => ({
  writePty: (...args: unknown[]) => writePtyMock(...args),
}));

import { useTerminalBlocks } from "./useTerminalBlocks";

async function writeToTerm(term: Terminal, data: string) {
  await new Promise<void>((resolve) => term.write(data, resolve));
}

let term: Terminal;

beforeEach(() => {
  writePtyMock.mockReset();
  term = new Terminal({ cols: 80, rows: 24 });
});

afterEach(() => {
  term.dispose();
});

describe("useTerminalBlocks", () => {
  it("creates a running block on submitCommand and appends PTY output into rawOutput", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => {
      result.current.submitCommand("echo hi");
    });
    expect(result.current.blocks).toHaveLength(1);
    expect(result.current.blocks[0].status).toBe("running");

    await act(async () => {
      await writeToTerm(term, "\x1b]133;C\x07");
    });

    act(() => {
      result.current.appendOutput("hi\r\n");
    });
    expect(result.current.blocks[0].rawOutput).toBe("hi\r\n");
  });

  it("marks block completed with exit code 0 and freezes rawOutput on OSC 133 D", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => {
      result.current.submitCommand("echo hi");
    });
    act(() => {
      result.current.appendOutput("hi\r\n");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });

    await waitFor(() => {
      expect(result.current.blocks[0].status).toBe("completed");
    });
    expect(result.current.blocks[0].exitCode).toBe(0);
    expect(result.current.blocks[0].rawOutput).toBe("hi\r\n");
    expect(result.current.blocks[0].endTime).toBeDefined();
  });

  it("marks block failed with non-zero exit code and produces renderedLines", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => {
      result.current.submitCommand("false");
    });
    act(() => {
      result.current.appendOutput("boom\r\n");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;1\x07");
    });

    await waitFor(() => {
      expect(result.current.blocks[0].renderedLines).toBeDefined();
    });
    expect(result.current.blocks[0].status).toBe("failed");
    expect(result.current.blocks[0].exitCode).toBe(1);
    expect(result.current.blocks[0].renderedLines?.[0].spans.map((s) => s.text).join("")).toBe("boom");
  });

  it("does not cross-contaminate rawOutput between two sequential blocks", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => { result.current.submitCommand("cmd1"); });
    act(() => { result.current.appendOutput("first\r\n"); });
    await act(async () => { await writeToTerm(term, "\x1b]133;D;0\x07"); });

    act(() => { result.current.submitCommand("cmd2"); });
    act(() => { result.current.appendOutput("second\r\n"); });
    await act(async () => { await writeToTerm(term, "\x1b]133;D;0\x07"); });

    await waitFor(() => {
      expect(result.current.blocks).toHaveLength(2);
      expect(result.current.blocks[1].status).toBe("completed");
    });
    expect(result.current.blocks[0].rawOutput).toBe("first\r\n");
    expect(result.current.blocks[1].rawOutput).toBe("second\r\n");
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npm run test -- src/hooks/useTerminalBlocks.test.ts`
Expected: FAIL，`appendOutput` 不存在於回傳值上

- [ ] **Step 3: 改寫 `useTerminalBlocks.ts`**

用以下內容完整取代 `src/hooks/useTerminalBlocks.ts`：

```ts
import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { writePty } from "../ipc/pty";
import { parseAnsiToRenderedLines, type RenderedLine } from "../lib/ansiBlockParser";
import type { GitBlockInfo } from "../ipc/vcs";

export interface TerminalBlock {
  id: string;
  command: string;
  status: "running" | "completed" | "failed";
  exitCode?: number;
  startTime: number;
  endTime?: number;
  cwd?: string;
  rawOutput: string;
  renderedLines?: RenderedLine[];
  gitInfo?: GitBlockInfo | null;
}

export interface UseTerminalBlocksResult {
  blocks: TerminalBlock[];
  submitCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  appendOutput: (chunk: string) => void;
  setBlockGitInfo: (id: string, info: GitBlockInfo | null) => void;
  isAlternateBuffer: boolean;
  termInstance: Terminal | null;
}

export function useTerminalBlocks(
  sessionId: string,
  term: Terminal | null,
  cwdRef?: React.RefObject<string>,
): UseTerminalBlocksResult {
  const [blocks, setBlocks] = useState<TerminalBlock[]>([]);
  const [isAlternateBuffer, setIsAlternateBuffer] = useState(false);

  const blocksRef = useRef<TerminalBlock[]>([]);
  const completionCallbacksRef = useRef<Map<string, (block: TerminalBlock) => void>>(new Map());

  const updateLatestBlock = useCallback((updater: (b: TerminalBlock) => TerminalBlock) => {
    const prev = blocksRef.current;
    if (prev.length === 0) return;
    const latest = prev[prev.length - 1];
    const updated = prev.map((b) => (b.id === latest.id ? updater(b) : b));
    blocksRef.current = updated;
    setBlocks(updated);
  }, []);

  const appendOutput = useCallback((chunk: string) => {
    updateLatestBlock((b) => (b.status === "running" ? { ...b, rawOutput: b.rawOutput + chunk } : b));
  }, [updateLatestBlock]);

  const setBlockGitInfo = useCallback((id: string, info: GitBlockInfo | null) => {
    const prev = blocksRef.current;
    const updated = prev.map((b) => (b.id === id ? { ...b, gitInfo: info } : b));
    blocksRef.current = updated;
    setBlocks(updated);
  }, []);

  useEffect(() => {
    if (!term) return;

    const onBufferChange = () => {
      setIsAlternateBuffer(term.buffer.active.type === "alternate");
    };
    const disposeBuffer = term.buffer.onBufferChange(onBufferChange);

    const disposeOsc = term.parser.registerOscHandler(133, (data) => {
      if (data === "C") {
        // Command start — no marker bookkeeping needed anymore; the block
        // was already created by submitCommand.
        return true;
      } else if (data.startsWith("D")) {
        const parts = data.split(";");
        const exitCode = parts.length > 1 ? parseInt(parts[1], 10) : 0;
        const endTime = Date.now();

        const prev = blocksRef.current;
        if (prev.length === 0) return true;
        const latest = prev[prev.length - 1];
        if (latest.status !== "running") return true;

        const finalExitCode = isNaN(exitCode) ? 0 : exitCode;
        const frozenOutput = latest.rawOutput;
        const cols = term.cols;

        const completedBlock: TerminalBlock = {
          ...latest,
          status: finalExitCode === 0 ? "completed" : "failed",
          exitCode: finalExitCode,
          endTime,
        };
        const updated = prev.map((b) => (b.id === latest.id ? completedBlock : b));
        blocksRef.current = updated;
        setBlocks(updated);

        term.clear();

        parseAnsiToRenderedLines(frozenOutput, cols).then((renderedLines) => {
          const withLines = blocksRef.current.map((b) =>
            b.id === latest.id ? { ...b, renderedLines } : b,
          );
          blocksRef.current = withLines;
          setBlocks(withLines);

          const cb = completionCallbacksRef.current.get(latest.id);
          if (cb) {
            completionCallbacksRef.current.delete(latest.id);
            const finalBlock = withLines.find((b) => b.id === latest.id)!;
            setTimeout(() => cb(finalBlock), 50);
          }
        });

        return true;
      }
      return false;
    });

    return () => {
      disposeBuffer.dispose();
      disposeOsc.dispose();
    };
  }, [term]);

  const submitCommand = useCallback(
    (cmd: string, onComplete?: (block: TerminalBlock) => void) => {
      if (!term || !sessionId) return;

      const newBlock: TerminalBlock = {
        id: Math.random().toString(36).substring(2, 15) + Date.now().toString(36),
        command: cmd,
        status: "running",
        startTime: Date.now(),
        cwd: cwdRef?.current,
        rawOutput: "",
      };

      if (onComplete) {
        completionCallbacksRef.current.set(newBlock.id, onComplete);
      }

      const updated = [...blocksRef.current, newBlock];
      blocksRef.current = updated;
      setBlocks(updated);

      // Clear the current line before sending the command.
      // On Windows conpty: \x15 echoes as visible "^U", and \x1b gets merged with
      // the first char of the command as an Alt+key (e.g. \x1b + "d" = Alt+D which
      // deletes a word, dropping the "d").  WarpInput owns all keyboard input so the
      // PTY line is always empty — no clear sequence needed on Windows.
      // On macOS/Linux, \x15 (Ctrl+U) clears bash/zsh input silently.
      const isWindows = navigator.platform.toLowerCase().startsWith("win");
      const clearSeq = isWindows ? "" : "\x15";
      writePty(sessionId, clearSeq + cmd + "\r").catch(console.error);
    },
    [sessionId, term, cwdRef],
  );

  return {
    blocks,
    submitCommand,
    appendOutput,
    setBlockGitInfo,
    isAlternateBuffer,
    termInstance: term,
  };
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npm run test -- src/hooks/useTerminalBlocks.test.ts`
Expected: PASS（4 個測試）

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useTerminalBlocks.ts src/hooks/useTerminalBlocks.test.ts
git commit -m "refactor(terminal): rearchitect useTerminalBlocks around headless parsing"
```

---

## Task 6: `TerminalBlockCard` 元件

**Files:**
- Create: `src/components/TerminalBlockCard.tsx`
- Create: `src/components/TerminalBlockCard.css`
- Test: `src/components/TerminalBlockCard.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

Create `src/components/TerminalBlockCard.test.tsx`:

```tsx
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TerminalBlockCard } from "./TerminalBlockCard";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

function makeBlock(overrides: Partial<TerminalBlock> = {}): TerminalBlock {
  return {
    id: "b1",
    command: "echo hi",
    status: "completed",
    exitCode: 0,
    startTime: 1000,
    endTime: 1500,
    cwd: "/Users/test/project",
    rawOutput: "hi\n",
    renderedLines: [{ spans: [{ text: "hi" }] }],
    gitInfo: { branch: "main", insertions: 2, deletions: 1 },
    ...overrides,
  };
}

describe("TerminalBlockCard", () => {
  it("renders command, cwd, duration, and git info in the header", () => {
    render(<TerminalBlockCard block={makeBlock()} />);
    expect(screen.getByText("echo hi")).toBeInTheDocument();
    expect(screen.getByText(/project/)).toBeInTheDocument();
    expect(screen.getByText(/main/)).toBeInTheDocument();
    expect(screen.getByText(/500ms|0\.5s/)).toBeInTheDocument();
  });

  it("renders output lines from renderedLines", () => {
    render(<TerminalBlockCard block={makeBlock()} />);
    expect(screen.getByText("hi")).toBeInTheDocument();
  });

  it("toggles collapse when the header is clicked", () => {
    render(<TerminalBlockCard block={makeBlock()} />);
    const header = screen.getByTestId("block-header");
    expect(screen.queryByTestId("block-body")).toBeInTheDocument();
    fireEvent.click(header);
    expect(screen.queryByTestId("block-body")).not.toBeInTheDocument();
    fireEvent.click(header);
    expect(screen.queryByTestId("block-body")).toBeInTheDocument();
  });

  it("truncates output beyond 500 lines with an expand affordance", () => {
    const manyLines = Array.from({ length: 600 }, (_, i) => ({ spans: [{ text: `line ${i}` }] }));
    render(<TerminalBlockCard block={makeBlock({ renderedLines: manyLines })} />);
    expect(screen.getByText(/還有 100 行|100 more/i)).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("block-expand"));
    expect(screen.getByText("line 599")).toBeInTheDocument();
  });

  it("calls onAskAi with the command and exit code for failed blocks", () => {
    const onAskAi = vi.fn();
    render(<TerminalBlockCard block={makeBlock({ status: "failed", exitCode: 1 })} onAskAi={onAskAi} />);
    fireEvent.click(screen.getByText(/Ask AI/));
    expect(onAskAi).toHaveBeenCalledWith("echo hi", 1);
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npm run test -- src/components/TerminalBlockCard.test.tsx`
Expected: FAIL，`Cannot find module './TerminalBlockCard'`

- [ ] **Step 3: 實作 `TerminalBlockCard.tsx`**

Create `src/components/TerminalBlockCard.tsx`:

```tsx
import { useState } from "react";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";
import "./TerminalBlockCard.css";

const MAX_VISIBLE_LINES = 500;

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const minutes = Math.floor(seconds / 60);
  const remSeconds = Math.round(seconds % 60);
  return `${minutes}m${remSeconds}s`;
}

function shortenCwd(cwd?: string): string {
  if (!cwd) return "";
  const parts = cwd.split("/");
  return parts.length > 2 ? `.../${parts.slice(-2).join("/")}` : cwd;
}

export interface TerminalBlockCardProps {
  block: TerminalBlock;
  highlightQuery?: string;
  onAskAi?: (command: string, exitCode: number | undefined) => void;
  onBookmark?: (command: string) => void;
  onCopy?: (command: string) => void;
}

function highlightText(text: string, query?: string): React.ReactNode {
  if (!query) return text;
  const idx = text.toLowerCase().indexOf(query.toLowerCase());
  if (idx === -1) return text;
  return (
    <>
      {text.slice(0, idx)}
      <mark>{text.slice(idx, idx + query.length)}</mark>
      {text.slice(idx + query.length)}
    </>
  );
}

export function TerminalBlockCard({ block, highlightQuery, onAskAi, onBookmark, onCopy }: TerminalBlockCardProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [expanded, setExpanded] = useState(false);

  const lines = block.renderedLines ?? [];
  const isTruncated = !expanded && lines.length > MAX_VISIBLE_LINES;
  const visibleLines = isTruncated ? lines.slice(0, MAX_VISIBLE_LINES) : lines;
  const hiddenCount = lines.length - MAX_VISIBLE_LINES;

  const duration = block.endTime ? formatDuration(block.endTime - block.startTime) : undefined;
  const exitClass = block.exitCode === 0 ? "aiterm-block-exit-ok" : "aiterm-block-exit-fail";

  return (
    <div className={`aiterm-block-card ${block.exitCode !== 0 ? "aiterm-block-card--failed" : ""}`}>
      <div className="aiterm-block-header" data-testid="block-header" onClick={() => setCollapsed((c) => !c)}>
        <span className="aiterm-block-cwd" title={block.cwd}>{shortenCwd(block.cwd)}</span>
        {block.gitInfo && (
          <span className="aiterm-block-git">
            git:({block.gitInfo.branch})
            {(block.gitInfo.insertions > 0 || block.gitInfo.deletions > 0) && (
              <>
                {" "}
                <span className="aiterm-block-git-add">+{block.gitInfo.insertions}</span>{" "}
                <span className="aiterm-block-git-del">-{block.gitInfo.deletions}</span>
              </>
            )}
          </span>
        )}
        {duration && <span className="aiterm-block-duration">({duration})</span>}
        <span className={exitClass}>{block.exitCode !== 0 ? `exit ${block.exitCode}` : ""}</span>
        <div className="aiterm-block-actions" onClick={(e) => e.stopPropagation()}>
          {block.exitCode !== 0 && onAskAi && (
            <button className="aiterm-block-btn aiterm-btn aiterm-btn--secondary" onClick={() => onAskAi(block.command, block.exitCode)}>
              ✨ Ask AI
            </button>
          )}
          {onBookmark && (
            <button className="aiterm-block-btn aiterm-btn aiterm-btn--secondary" onClick={() => onBookmark(block.command)}>
              Bookmark
            </button>
          )}
          {onCopy && (
            <button className="aiterm-block-btn aiterm-btn aiterm-btn--secondary" onClick={() => onCopy(block.command)}>
              Copy
            </button>
          )}
        </div>
      </div>
      <div className="aiterm-block-command">{highlightText(block.command, highlightQuery)}</div>
      {!collapsed && (
        <div className="aiterm-block-body" data-testid="block-body">
          <pre>
            {visibleLines.map((line, i) => (
              <div key={i} className="aiterm-block-line">
                {line.spans.map((span, j) => (
                  <span
                    key={j}
                    style={{
                      color: span.fg,
                      backgroundColor: span.bg,
                      fontWeight: span.bold ? "bold" : undefined,
                      fontStyle: span.italic ? "italic" : undefined,
                      textDecoration: span.underline ? "underline" : undefined,
                    }}
                  >
                    {highlightText(span.text, highlightQuery)}
                  </span>
                ))}
              </div>
            ))}
          </pre>
          {isTruncated && (
            <button className="aiterm-block-expand aiterm-btn aiterm-btn--secondary" data-testid="block-expand" onClick={() => setExpanded(true)}>
              顯示完整輸出（還有 {hiddenCount} 行）
            </button>
          )}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: 新增 CSS**

Create `src/components/TerminalBlockCard.css`（可先用簡單樣式，視覺細節之後可調整）：

```css
.aiterm-block-card {
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  margin: 6px 8px;
  overflow: hidden;
  background: rgba(255, 255, 255, 0.02);
}

.aiterm-block-card--failed {
  border-color: rgba(248, 113, 113, 0.4);
  background: rgba(248, 113, 113, 0.06);
}

.aiterm-block-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 10px;
  font-size: 11px;
  color: #888;
  background: rgba(255, 255, 255, 0.03);
  cursor: pointer;
}

.aiterm-block-git-add { color: #4ade80; }
.aiterm-block-git-del { color: #f87171; }
.aiterm-block-exit-ok { color: #4ade80; }
.aiterm-block-exit-fail { color: #f87171; font-weight: 600; }

.aiterm-block-actions {
  margin-left: auto;
  display: flex;
  gap: 6px;
}

.aiterm-block-command {
  padding: 6px 10px 2px;
  color: #9cdcfe;
  font-family: "SF Mono", Menlo, monospace;
  font-size: 13px;
}

.aiterm-block-body {
  padding: 2px 10px 8px;
}

.aiterm-block-body pre {
  margin: 0;
  font-family: "SF Mono", Menlo, monospace;
  font-size: 13px;
  white-space: pre-wrap;
  word-break: break-word;
}

.aiterm-block-line { min-height: 1.3em; }

.aiterm-block-expand {
  margin-top: 6px;
}
```

- [ ] **Step 5: 執行測試確認通過**

Run: `npm run test -- src/components/TerminalBlockCard.test.tsx`
Expected: PASS（5 個測試）

- [ ] **Step 6: Commit**

```bash
git add src/components/TerminalBlockCard.tsx src/components/TerminalBlockCard.css src/components/TerminalBlockCard.test.tsx
git commit -m "feat(terminal): add TerminalBlockCard with Warp-style header"
```

---

## Task 7: `TerminalView` — 移除舊 overlay，接上區塊清單與 rawOutput 累積

**Files:**
- Modify: `src/components/TerminalView.tsx`

> **新增範圍（Task 5 code review 發現，原計畫遺漏）：** 執行 `npx tsc -b`（而非之前誤用、對 `src/` 完全不做檢查的 `npx tsc --noEmit`）才會發現，除了 Step 4 要移除的 overlay 區塊，`TerminalView.tsx` 裡還有兩處各自獨立讀取已被 Task 5 移除的 `block.startLine`/`block.endLine`，用來從**即時 xterm 緩衝區重新掃描**指令輸出文字。這兩處都應該改成直接讀 `block.rawOutput`（Task 5 已經把完整原始輸出存在區塊上了，不需要再回頭掃描緩衝區——這也比舊做法更可靠，因為現在完成後會呼叫 `term.clear()`，緩衝區內容已經被清掉了）。新增以下 Step 0 一併處理，順序上必須在 Step 4（該處的 `blocks`/overlay 邏輯）之前完成，避免建置中斷。

- [ ] **Step 0a: 修正「遙控回應」功能改讀 `rawOutput`**

先讀取 `src/components/TerminalView.tsx:319-343` 確認內容與下方一致（`submitCommand(text, (block) => {...})`，透過 `startLine`/`endLine` 掃描 `term.buffer.active` 取得輸出文字，供 `sendRemoteResponse` 使用）。整段改為：

```ts
        submitCommand(text, (block) => {
          let output = block.rawOutput.trim();
          if (output.length > 0) {
            if (output.length > 4000) {
              output = output.substring(0, 4000) + "\n... (output truncated)";
            }
            sendRemoteResponse(output);
          } else {
            sendRemoteResponse(`(Command finished: ${block.command})`);
          }
        });
```

（移除對 `termRef.current`/`term.buffer.active` 的存取，因為不再需要重新掃描緩衝區；`startLine`/`endLine` 相關的兩行局部變數與註解一併刪除。）

- [ ] **Step 0b: 修正 Agent Mission 的 `onBlockDone` 改讀 `rawOutput`**

先讀取 `src/components/TerminalView.tsx:1323-1337` 確認內容與下方一致（`onBlockDone` callback，同樣用 `startLine`/`endLine` 掃描緩衝區取得 `rawOutput` 給 Agent Loop 下一步使用）。把：

```ts
    // Extract terminal output for this block
    const startY = completedBlock.startLine ?? 0;
    const endY = completedBlock.endLine ?? term.buffer.active.cursorY + term.buffer.active.baseY;
    let rawOutput = "";
    for (let i = startY; i < endY; i++) {
      rawOutput += term.buffer.active.getLine(i)?.translateToString(true) + "\n";
    }
    rawOutput = rawOutput.trim();
    if (rawOutput.length > 2000) rawOutput = rawOutput.slice(rawOutput.length - 2000);
```

改為：

```ts
    // Extract terminal output for this block
    let rawOutput = completedBlock.rawOutput.trim();
    if (rawOutput.length > 2000) rawOutput = rawOutput.slice(rawOutput.length - 2000);
```

- [ ] **Step 0c: 確認建置乾淨**

Run: `npx tsc -b 2>&1 | grep -v "startMarker\|b.endLine\|b.startLine\|b.endMarker"`（Step 4 完成前，overlay 區塊本身的錯誤還會存在，先確認 Step 0a/0b 涉及的那兩處錯誤已消失，overlay 相關錯誤留到 Step 4 之後再一併確認清空）
Expected: 只剩下 Step 4 尚未處理的 overlay 相關錯誤（`b.startMarker`/`b.endLine`/`b.endMarker` 這幾個，都在約 921-959 行）

- [ ] **Step 1: 新增 `lastCwdRef` 並在既有 cwd 輪詢中同步更新**

先讀取 `src/components/TerminalView.tsx:115-144` 確認目前內容與本計畫描述一致（`displayCwd` state + `setInterval` 輪詢 `getSessionCwd`）。在 `const [displayCwd, setDisplayCwd] = useState<string>("");` 之後（約 120 行）新增：

```ts
  const lastCwdRef = useRef<string>("");
```

在 `setDisplayCwd(pretty);` 那一行（約 139 行）之前新增：

```ts
        lastCwdRef.current = cwd;
```

- [ ] **Step 2: 把 `lastCwdRef` 傳入 `useTerminalBlocks`**

Modify `src/components/TerminalView.tsx:182`（`const { blocks, isAlternateBuffer, submitCommand } = useTerminalBlocks(...)`），改為：

```ts
  const { blocks, isAlternateBuffer, submitCommand, appendOutput, setBlockGitInfo } = useTerminalBlocks(
    sessionId,
    termState,
    lastCwdRef,
  );
```

（沿用原本呼叫時傳入的參數，只新增第三個 `lastCwdRef` 引數與解構出的 `appendOutput`/`setBlockGitInfo`。）

- [ ] **Step 3: 在 `onPtyData` 監聽處呼叫 `appendOutput`**

Modify `src/components/TerminalView.tsx:543-545`：

```ts
        unlistenData = await onPtyData(id, (bytes) => {
          const text = decoder.decode(bytes);
          term.write(text);
          appendOutput(text);
```

- [ ] **Step 4: 移除舊的座標猜測 overlay，改渲染區塊清單**

Modify `src/components/TerminalView.tsx:910-1030`（`hostRef` 容器 + 「React DOM overlay for visual blocks」整段），改為：

```tsx
        <div className="aiterm-block-list" ref={blockListRef}>
          {blocks
            .filter((b) => b.status !== "running" && b.renderedLines)
            .map((b) => (
              <TerminalBlockCard
                key={b.id}
                block={b}
                highlightQuery={searchOpen ? searchQuery : undefined}
                onAskAi={(command, exitCode) => {
                  window.dispatchEvent(new CustomEvent("aiterm:ask-ai", { detail: { command, exitCode } }));
                }}
                onBookmark={(command) => addBookmark(command)}
                onCopy={(command) => navigator.clipboard.writeText(command).catch(console.error)}
              />
            ))}
        </div>
        <div
          ref={hostRef}
          className="aiterm-terminal-root"
          style={{ height: "220px", width: "calc(100% - 20px)", marginLeft: "20px", boxSizing: "border-box", flexShrink: 0 }}
        />
```

在同一個檔案的 import 區塊加入：

```ts
import { TerminalBlockCard } from "./TerminalBlockCard";
```

在元件內（`hostRef` 定義附近）新增一個 ref 並在區塊數量變動時自動捲到底部：

```ts
  const blockListRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    blockListRef.current?.scrollTo({ top: blockListRef.current.scrollHeight });
  }, [blocks.length]);
```

把原本包住 `hostRef`＋overlay 的容器 `<div style={{ position: "relative", flex: 1, minHeight: 0, width: "100%" }}>`（約 877 行）的 `style` 改成直向排列：

```tsx
<div style={{ position: "relative", flex: 1, minHeight: 0, width: "100%", display: "flex", flexDirection: "column" }}>
```

把 `aiterm-block-list` 的樣式（可加進既有 `TerminalView.css`）：

```css
.aiterm-block-list {
  flex: 1 1 auto;
  overflow-y: auto;
  min-height: 0;
}
```

- [ ] **Step 4.5: 刪除舊 overlay 的死 CSS（Task 6 code review 發現，原計畫遺漏）**

`src/components/TerminalView.css` 第 74-150 行（`/* ── M6: Xterm Block Decorations ── */` 到 `/* ── Block Action Buttons ── */` 整段，含 `.aiterm-block-decoration`、`.aiterm-block-success`、`.aiterm-block-error`、`.aiterm-block-actions`、`.aiterm-block-btn*`）是舊 overlay 專用的樣式，Step 4 把對應的 JSX 刪掉後這整段變成死程式碼。**必須刪除**，不是可做可不做：`TerminalBlockCard.css` 裡的按鈕沿用了同樣的 class 名稱（`aiterm-block-btn`、`aiterm-btn--secondary` 組合），這段舊規則的 `.aiterm-block-btn.aiterm-btn--secondary { font-family: "Cascadia Mono", ...; font-size: 11px; padding: 2px 8px; }` 選擇器不看規則來自哪個檔案，仍會套用到新元件的按鈕上，讓新卡片的按鈕意外套用舊字型/padding。整段刪除（含 74 行上方的區塊標題註解、`@keyframes aiterm-block-fadein`）。

- [ ] **Step 5: 區塊完成時非同步抓取 git 資訊**

在元件內新增一個 `useEffect`，監看剛完成但還沒有 `gitInfo` 欄位的區塊，做 500ms 防抖後查詢：

```ts
  const gitFetchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    const pending = blocks.find((b) => b.status !== "running" && b.gitInfo === undefined && b.cwd);
    if (!pending) return;
    if (gitFetchTimerRef.current) return;

    gitFetchTimerRef.current = setTimeout(async () => {
      gitFetchTimerRef.current = null;
      const stillPending = blocksRef.current.filter((b) => b.status !== "running" && b.gitInfo === undefined && b.cwd);
      for (const b of stillPending) {
        try {
          const info = await getGitBlockInfo(b.cwd!);
          setBlockGitInfo(b.id, info);
        } catch {
          setBlockGitInfo(b.id, null);
        }
      }
    }, 500);
  }, [blocks]);
```

在 import 區塊加入：

```ts
import { getGitBlockInfo } from "../ipc/vcs";
```

> 註：`blocksRef` 在 `TerminalView.tsx:197` 已存在（`const blocksRef = useRef(blocks);`），這裡直接沿用，不需另外建立。

- [ ] **Step 6: 型別檢查與現有測試**

Run: `npx tsc -b`（注意：不是 `--noEmit`，這個專案的 solution-style tsconfig 會讓 `--noEmit` 對 `src/` 完全不做檢查）
Expected: 無錯誤

Run: `npm run test`
Expected: 全部通過（含 Task 4/5/6 新增的測試）

- [ ] **Step 7: 手動驗證**

Run: `npm run tauri:dev`，在終端機輸入幾個指令（包含至少一個會失敗的指令，例如 `false`），確認：
- 每個指令執行完後，上方出現對應的區塊卡片，含路徑／git branch（如果是 git 目錄）／耗時／exit code
- 失敗的指令卡片有紅色標示與「Ask AI」按鈕
- 即時終端機只顯示目前輸入/執行中的內容，不再累積歷史
- 進入 `vim` 等全螢幕程式，行為與改動前一致

- [ ] **Step 8: Commit**

```bash
git add src/components/TerminalView.tsx src/components/TerminalView.css
git commit -m "refactor(terminal): render completed blocks as a DOM list instead of xterm overlay"
```

---

## Task 8: 搜尋功能改造 — 跨區塊清單與即時內容

**Files:**
- Create: `src/lib/blockSearch.ts`
- Test: `src/lib/blockSearch.test.ts`
- Modify: `src/components/TerminalView.tsx`

> **已知限制（Task 6 code review 發現，刻意接受不修）：** `TerminalBlockCard` 的 `highlightQuery` prop 只是一個字串，只會高亮該區塊內容裡「第一個」符合的地方，無法表達「這是第 N 個符合項」。如果同一個區塊內有兩個以上符合搜尋字串的地方，搜尋游標在同一區塊內移動到第二個以後的符合項時，畫面上高亮的位置不會跟著移動（還是停在第一個）——但區塊本身還是會正確捲動、定位到，只是視覺反白的精確度在多重符合的情況下打折扣。這是刻意的取捨（避免把 offset-aware 的高亮機制做進去，增加不必要的複雜度），不是這個 Task 要修的東西，除非之後使用起來發現這個限制實際上很惱人，才考慮做進階版。

- [ ] **Step 1: 寫失敗的測試**

Create `src/lib/blockSearch.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { findNextBlockMatch, findPreviousBlockMatch, blockPlainText } from "./blockSearch";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

function makeBlock(id: string, text: string): TerminalBlock {
  return {
    id,
    command: `cmd-${id}`,
    status: "completed",
    exitCode: 0,
    startTime: 0,
    rawOutput: text,
    renderedLines: [{ spans: [{ text }] }],
  };
}

describe("blockPlainText", () => {
  it("joins all span text across all lines", () => {
    const block = makeBlock("a", "hello world");
    expect(blockPlainText(block)).toBe("hello world");
  });
});

describe("findNextBlockMatch", () => {
  it("finds a match in the first block after the given cursor", () => {
    const blocks = [makeBlock("a", "foo bar"), makeBlock("b", "no match here")];
    const match = findNextBlockMatch(blocks, "bar", null);
    expect(match?.blockId).toBe("a");
  });

  it("skips to the next block when the query isn't in the current block", () => {
    const blocks = [makeBlock("a", "foo"), makeBlock("b", "target")];
    const match = findNextBlockMatch(blocks, "target", { blockId: "a", offset: 0 });
    expect(match?.blockId).toBe("b");
  });

  it("returns null when no block contains the query", () => {
    const blocks = [makeBlock("a", "foo"), makeBlock("b", "bar")];
    expect(findNextBlockMatch(blocks, "zzz", null)).toBeNull();
  });
});

describe("findPreviousBlockMatch", () => {
  it("finds a match in the block before the given cursor", () => {
    const blocks = [makeBlock("a", "target"), makeBlock("b", "no match")];
    const match = findPreviousBlockMatch(blocks, "target", { blockId: "b", offset: 0 });
    expect(match?.blockId).toBe("a");
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npm run test -- src/lib/blockSearch.test.ts`
Expected: FAIL，`Cannot find module './blockSearch'`

- [ ] **Step 3: 實作 `blockSearch.ts`**

Create `src/lib/blockSearch.ts`:

```ts
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

export interface BlockSearchCursor {
  blockId: string;
  offset: number;
}

export function blockPlainText(block: TerminalBlock): string {
  return (block.renderedLines ?? []).map((line) => line.spans.map((s) => s.text).join("")).join("\n");
}

export function findNextBlockMatch(
  blocks: TerminalBlock[],
  query: string,
  cursor: BlockSearchCursor | null,
): BlockSearchCursor | null {
  if (!query) return null;
  const q = query.toLowerCase();
  const startIndex = cursor ? blocks.findIndex((b) => b.id === cursor.blockId) : -1;

  for (let i = Math.max(startIndex, 0); i < blocks.length; i++) {
    const block = blocks[i];
    const text = blockPlainText(block).toLowerCase();
    const searchFrom = i === startIndex ? cursor!.offset + 1 : 0;
    const idx = text.indexOf(q, searchFrom);
    if (idx !== -1) {
      return { blockId: block.id, offset: idx };
    }
  }
  return null;
}

export function findPreviousBlockMatch(
  blocks: TerminalBlock[],
  query: string,
  cursor: BlockSearchCursor | null,
): BlockSearchCursor | null {
  if (!query) return null;
  const q = query.toLowerCase();
  const startIndex = cursor ? blocks.findIndex((b) => b.id === cursor.blockId) : blocks.length;

  for (let i = Math.min(startIndex, blocks.length - 1); i >= 0; i--) {
    const block = blocks[i];
    const text = blockPlainText(block).toLowerCase();
    const searchUpTo = i === startIndex ? cursor!.offset - 1 : text.length;
    if (searchUpTo < 0) continue;
    const idx = text.lastIndexOf(q, searchUpTo);
    if (idx !== -1) {
      return { blockId: block.id, offset: idx };
    }
  }
  return null;
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npm run test -- src/lib/blockSearch.test.ts`
Expected: PASS（5 個測試）

- [ ] **Step 5: 串接進 `TerminalView.tsx` 的 `doSearch`/`closeSearch`**

先讀 `src/components/TerminalView.tsx:746-777` 確認現有 `doSearch`/`closeSearch`/自動搜尋 `useEffect` 內容與本計畫描述一致。新增 state 與 import：

```ts
import { findNextBlockMatch, findPreviousBlockMatch, type BlockSearchCursor } from "../lib/blockSearch";
```

```ts
  const [blockSearchCursor, setBlockSearchCursor] = useState<BlockSearchCursor | null>(null);
```

把 `doSearch` 改為：

```ts
  const doSearch = useCallback((query: string, direction: 'next' | 'prev') => {
    const addon = searchAddonRef.current;
    if (!addon || !query) { setSearchMatchInfo(""); return; }

    const foundLive = direction === 'next'
      ? addon.findNext(query, SEARCH_OPTS)
      : addon.findPrevious(query, SEARCH_OPTS);

    if (foundLive) {
      setBlockSearchCursor(null);
      setSearchMatchInfo("found");
      return;
    }

    const match = direction === 'next'
      ? findNextBlockMatch(blocksRef.current, query, blockSearchCursor)
      : findPreviousBlockMatch(blocksRef.current, query, blockSearchCursor);

    if (match) {
      setBlockSearchCursor(match);
      setSearchMatchInfo("found");
      requestAnimationFrame(() => {
        document.getElementById(`aiterm-block-${match.blockId}`)?.scrollIntoView({ block: "center", behavior: "smooth" });
      });
    } else {
      setSearchMatchInfo("not found");
    }
  }, [blockSearchCursor]);
```

把 `closeSearch` 改為額外重置 `blockSearchCursor`：

```ts
  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    setSearchQuery("");
    setSearchMatchInfo("");
    setBlockSearchCursor(null);
    searchAddonRef.current?.clearDecorations?.();
  }, []);
```

- [ ] **Step 6: 讓 `TerminalBlockCard` 的容器帶上可供捲動定位的 id**

Modify `src/components/TerminalView.tsx`（Task 7 Step 4 新增的 `<TerminalBlockCard key={b.id} ...>` 呼叫處），外面包一層帶 id 的 `div`：

```tsx
              <div id={`aiterm-block-${b.id}`} key={b.id}>
                <TerminalBlockCard
                  block={b}
                  highlightQuery={searchOpen && blockSearchCursor?.blockId === b.id ? searchQuery : undefined}
                  onAskAi={(command, exitCode) => {
                    window.dispatchEvent(new CustomEvent("aiterm:ask-ai", { detail: { command, exitCode } }));
                  }}
                  onBookmark={(command) => addBookmark(command)}
                  onCopy={(command) => navigator.clipboard.writeText(command).catch(console.error)}
                />
              </div>
```

（移除原本 `.map` 上的 `key={b.id}`，改放在外層 `div` 上，`TerminalBlockCard` 本身不用再帶 `key`。）

- [ ] **Step 7: 型別檢查與完整測試**

Run: `npx tsc -b && npm run test`（注意：不是 `--noEmit`）
Expected: 全部通過

- [ ] **Step 8: 手動驗證**

Run: `npm run tauri:dev`，執行數個指令產生多個區塊，按 `Cmd/Ctrl+F` 開啟搜尋，輸入只存在於某個歷史區塊輸出中的字串，確認畫面會捲動到該區塊並反白，`Enter`/`F3` 可以繼續往下一個符合項目跳。

- [ ] **Step 9: Commit**

```bash
git add src/lib/blockSearch.ts src/lib/blockSearch.test.ts src/components/TerminalView.tsx
git commit -m "feat(terminal): extend search to cover completed block content"
```

---

## Plan Self-Review Notes

- **Spec coverage**：六個 spec 段落（整體架構／資料擷取／git 標頭／搜尋／效能與邊界情況／測試）分別對應 Task 5-7（架構+資料）、Task 1-3（git 標頭後端）、Task 6-7（標頭 UI）、Task 8（搜尋）、Task 6 的截斷/摺疊（效能）、每個 Task 內建的測試步驟。
- **範疇排除項確認**：alternate buffer 渲染完全沒有改動（Task 5 的 OSC handler 只在 `D` 分支動作，`isAlternateBuffer` 偵測邏輯原封不動保留）；未引入任何虛擬捲動函式庫，符合排除項。
- **型別一致性**：`TerminalBlock.rawOutput`／`renderedLines`／`gitInfo`／`cwd`／`startTime`／`endTime` 從 Task 5 定義後，在 Task 6（`TerminalBlockCard` props）、Task 7（`TerminalView` 篩選/渲染條件）、Task 8（`blockSearch.ts`）中保持一致命名，沒有出現改名不同步的狀況。
