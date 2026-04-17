# File Content Viewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a user clicks a file in the Files tab, its content is displayed in a right-side panel beside the file tree.

**Architecture:** Split-pane layout inside `FileExplorer` — left column is the existing tree, right column is a new `FileViewer` component. A new Rust IPC command `pty_read_file` reads up to 10 MB of a file and returns content + truncation flag.

**Tech Stack:** Rust (Tauri command), TypeScript, React, Vitest + @testing-library/react

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `src-tauri/src/pty/commands.rs` | Add `pty_read_file` Tauri command |
| Modify | `src-tauri/src/lib.rs` | Register `pty_read_file` in invoke_handler |
| Modify | `src/ipc/fs.ts` | Add `readFile` IPC wrapper + `FileContent` type |
| Create | `src/components/FileExplorer/FileViewer.tsx` | Right-pane content display component |
| Create | `src/components/FileExplorer/FileViewer.test.tsx` | Tests for FileViewer |
| Modify | `src/components/FileExplorer/FileExplorer.tsx` | Add `selectedFile` state, split-pane layout |
| Create | `src/components/FileExplorer/FileExplorer.test.tsx` | Tests for FileExplorer click behaviour |
| Modify | `src/components/FileExplorer/FileExplorer.css` | Split-pane and selected-row styles |

---

### Task 1: Rust `pty_read_file` command

**Files:**
- Modify: `src-tauri/src/pty/commands.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `FileContent` struct and `pty_read_file` command to `commands.rs`**

Append this to the bottom of `src-tauri/src/pty/commands.rs`:

```rust
/// Returned by pty_read_file.
#[derive(serde::Serialize)]
pub struct FileContent {
    pub content: String,
    pub truncated: bool,
}

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// Read a text file's content. Caps at 10 MB; binary files return an error.
#[tauri::command]
pub fn pty_read_file(path: String) -> Result<FileContent, String> {
    use std::io::Read;

    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    let file_size = metadata.len();
    let truncated = file_size > MAX_FILE_BYTES;

    let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let read_size = MAX_FILE_BYTES.min(file_size) as usize;
    let mut buf = vec![0u8; read_size];
    file.read_exact(&mut buf).map_err(|e| e.to_string())?;

    let content = String::from_utf8(buf)
        .map_err(|_| "binary".to_string())?;

    Ok(FileContent { content, truncated })
}
```

- [ ] **Step 2: Register `pty_read_file` in `src-tauri/src/lib.rs`**

In `src-tauri/src/lib.rs`, update the import line:

```rust
use pty::commands::{pty_close, pty_create, pty_get_cwd, pty_list_dir, pty_read_file, pty_resize, pty_write};
```

In the `invoke_handler` macro, add `pty_read_file` after `pty_list_dir`:

```rust
            pty_list_dir,
            pty_read_file,
```

- [ ] **Step 3: Verify the Rust project compiles**

```bash
cd src-tauri && cargo check 2>&1
```

Expected: no errors. Warnings are OK.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/pty/commands.rs src-tauri/src/lib.rs
git commit -m "feat(pty): add pty_read_file command (10 MB cap, binary detection)"
```

---

### Task 2: TypeScript IPC wrapper

**Files:**
- Modify: `src/ipc/fs.ts`

- [ ] **Step 1: Add `FileContent` type and `readFile` function to `src/ipc/fs.ts`**

Append to the end of `src/ipc/fs.ts`:

```ts
export interface FileContent {
  content: string;
  truncated: boolean;
}

/** Read a text file's content. Throws if binary or unreadable. */
export const readFile = (path: string): Promise<FileContent> =>
  invoke<FileContent>("pty_read_file", { path });
```

- [ ] **Step 2: Commit**

```bash
git add src/ipc/fs.ts
git commit -m "feat(ipc): add readFile wrapper for pty_read_file"
```

---

### Task 3: `FileViewer` component (TDD)

**Files:**
- Create: `src/components/FileExplorer/FileViewer.tsx`
- Create: `src/components/FileExplorer/FileViewer.test.tsx`

- [ ] **Step 1: Write failing tests in `FileViewer.test.tsx`**

Create `src/components/FileExplorer/FileViewer.test.tsx`:

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { FileViewer } from "./FileViewer";
import type { DirEntry } from "../../ipc/fs";

const mockFile: DirEntry = {
  name: "hello.ts",
  path: "/project/hello.ts",
  is_dir: false,
  size: 42,
};

beforeEach(() => {
  invokeMock.mockReset();
});

describe("FileViewer", () => {
  it("shows empty state when no file selected", () => {
    render(<FileViewer sessionId="s1" file={null} />);
    expect(screen.getByText(/選擇左側檔案以預覽內容/)).toBeInTheDocument();
  });

  it("shows file content after successful load", async () => {
    invokeMock.mockResolvedValueOnce({ content: "const x = 1;\n", truncated: false });
    render(<FileViewer sessionId="s1" file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/const x = 1;/)).toBeInTheDocument()
    );
    expect(screen.getByText("hello.ts")).toBeInTheDocument();
  });

  it("shows truncation banner when truncated=true", async () => {
    invokeMock.mockResolvedValueOnce({ content: "big content", truncated: true });
    render(<FileViewer sessionId="s1" file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/僅顯示前 10 MB/)).toBeInTheDocument()
    );
  });

  it("shows error message when read fails", async () => {
    invokeMock.mockRejectedValueOnce(new Error("permission denied"));
    render(<FileViewer sessionId="s1" file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/permission denied/)).toBeInTheDocument()
    );
  });

  it("shows binary message when error is 'binary'", async () => {
    invokeMock.mockRejectedValueOnce("binary");
    render(<FileViewer sessionId="s1" file={mockFile} />);
    await waitFor(() =>
      expect(screen.getByText(/二進位格式/)).toBeInTheDocument()
    );
  });
});
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
npm test -- FileViewer 2>&1
```

Expected: FAIL — `FileViewer` not found.

- [ ] **Step 3: Create `FileViewer.tsx`**

Create `src/components/FileExplorer/FileViewer.tsx`:

```tsx
import { useState, useEffect } from "react";
import { readFile } from "../../ipc/fs";
import type { DirEntry } from "../../ipc/fs";

interface FileViewerProps {
  sessionId: string;
  file: DirEntry | null;
}

type ViewState =
  | { kind: "empty" }
  | { kind: "loading" }
  | { kind: "ok"; content: string; truncated: boolean }
  | { kind: "binary" }
  | { kind: "error"; message: string };

export function FileViewer({ file }: FileViewerProps) {
  const [state, setState] = useState<ViewState>({ kind: "empty" });

  useEffect(() => {
    if (!file) {
      setState({ kind: "empty" });
      return;
    }
    setState({ kind: "loading" });
    readFile(file.path)
      .then(({ content, truncated }) =>
        setState({ kind: "ok", content, truncated })
      )
      .catch((err: unknown) => {
        const msg = err === "binary" || String(err) === "binary"
          ? "binary"
          : err instanceof Error ? err.message : String(err);
        if (msg === "binary") {
          setState({ kind: "binary" });
        } else {
          setState({ kind: "error", message: msg });
        }
      });
  }, [file?.path]);

  if (state.kind === "empty") {
    return (
      <div className="fv-empty">
        選擇左側檔案以預覽內容
      </div>
    );
  }

  return (
    <div className="fv-root">
      {file && (
        <div className="fv-header" title={file.path}>
          {file.name}
        </div>
      )}

      {state.kind === "loading" && (
        <div className="fv-status">載入中…</div>
      )}

      {state.kind === "binary" && (
        <div className="fv-status fv-status--muted">
          此檔案為二進位格式，無法預覽
        </div>
      )}

      {state.kind === "error" && (
        <div className="fv-status fv-status--error">{state.message}</div>
      )}

      {state.kind === "ok" && (
        <>
          {state.truncated && (
            <div className="fv-banner">⚠ 檔案過大，僅顯示前 10 MB</div>
          )}
          <div className="fv-content">
            <pre className="fv-pre">
              {state.content.split("\n").map((line, i) => (
                <div key={i} className="fv-line">
                  <span className="fv-lineno">{i + 1}</span>
                  <span className="fv-linetext">{line}</span>
                </div>
              ))}
            </pre>
          </div>
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run tests — verify they pass**

```bash
npm test -- FileViewer 2>&1
```

Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/FileExplorer/FileViewer.tsx src/components/FileExplorer/FileViewer.test.tsx
git commit -m "feat(FileViewer): add file content viewer component with tests"
```

---

### Task 4: Wire up `FileExplorer` split-pane (TDD)

**Files:**
- Modify: `src/components/FileExplorer/FileExplorer.tsx`
- Create: `src/components/FileExplorer/FileExplorer.test.tsx`

- [ ] **Step 1: Write failing tests in `FileExplorer.test.tsx`**

Create `src/components/FileExplorer/FileExplorer.test.tsx`:

```tsx
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { FileExplorer } from "./FileExplorer";

beforeEach(() => {
  invokeMock.mockReset();
});

describe("FileExplorer — file selection", () => {
  it("shows empty viewer state initially", async () => {
    invokeMock.mockResolvedValueOnce([]); // listDirectory → empty
    invokeMock.mockResolvedValueOnce(""); // getSessionCwd
    render(<FileExplorer sessionId="s1" />);
    await waitFor(() =>
      expect(screen.getByText(/選擇左側檔案以預覽內容/)).toBeInTheDocument()
    );
  });

  it("clicking a file loads its content in the viewer", async () => {
    invokeMock
      .mockResolvedValueOnce([ // listDirectory
        { name: "index.ts", path: "/p/index.ts", is_dir: false, size: 20 },
      ])
      .mockResolvedValueOnce("/p") // getSessionCwd
      .mockResolvedValueOnce({ content: "export default 1;", truncated: false }); // readFile

    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => screen.getByText("index.ts"));

    await userEvent.click(screen.getByText("index.ts"));

    await waitFor(() =>
      expect(screen.getByText(/export default 1;/)).toBeInTheDocument()
    );
  });

  it("clicking a directory does NOT load file content", async () => {
    invokeMock
      .mockResolvedValueOnce([ // listDirectory
        { name: "src", path: "/p/src", is_dir: true, size: null },
      ])
      .mockResolvedValueOnce("/p") // getSessionCwd
      .mockResolvedValueOnce([]); // listDirectory for expanded dir

    render(<FileExplorer sessionId="s1" />);
    await waitFor(() => screen.getByText("src"));

    await userEvent.click(screen.getByText("src"));

    // readFile should NOT have been called (invoke called 3 times max)
    expect(invokeMock).not.toHaveBeenCalledWith("pty_read_file", expect.anything());
    expect(screen.getByText(/選擇左側檔案以預覽內容/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
npm test -- FileExplorer.test 2>&1
```

Expected: FAIL — empty viewer state and click behaviour not yet implemented.

- [ ] **Step 3: Update `FileExplorer.tsx`**

Replace the entire content of `src/components/FileExplorer/FileExplorer.tsx` with:

```tsx
import { useState, useEffect, useCallback } from "react";
import { listDirectory, getSessionCwd, type DirEntry } from "../../ipc/fs";
import { FileViewer } from "./FileViewer";
import "./FileExplorer.css";

interface FileExplorerProps {
  sessionId: string;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function getFileIcon(entry: DirEntry): string {
  if (entry.is_dir) return "📁";
  const ext = entry.name.split(".").pop()?.toLowerCase() ?? "";
  const icons: Record<string, string> = {
    ts: "🔷", tsx: "🔷", js: "🟡", jsx: "🟡",
    json: "📋", md: "📝", rs: "🦀", toml: "⚙️",
    css: "🎨", html: "🌐", png: "🖼️", jpg: "🖼️",
    jpeg: "🖼️", gif: "🖼️", svg: "🎨", sh: "⚡",
    py: "🐍", go: "🔵", yaml: "⚙️", yml: "⚙️",
    txt: "📄", pdf: "📕", zip: "🗜️", tar: "🗜️",
    gz: "🗜️", lock: "🔒",
  };
  return icons[ext] ?? "📄";
}

export function FileExplorer({ sessionId }: FileExplorerProps) {
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [cwd, setCwd] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [subEntries, setSubEntries] = useState<Record<string, DirEntry[]>>({});
  const [showDotfiles, setShowDotfiles] = useState(false);
  const [selectedFile, setSelectedFile] = useState<DirEntry | null>(null);

  const loadDir = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const result = await listDirectory(sessionId, path);
      const filtered = showDotfiles ? result : result.filter(e => !e.name.startsWith("."));
      setEntries(filtered);
      setCwd(path || (await getSessionCwd(sessionId)) || "");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionId, showDotfiles]);

  useEffect(() => {
    loadDir("");
  }, [loadDir]);

  const handleToggleDir = async (entry: DirEntry) => {
    if (!entry.is_dir) return;
    const key = entry.path;
    if (expanded.has(key)) {
      const next = new Set(expanded);
      next.delete(key);
      setExpanded(next);
    } else {
      if (!subEntries[key]) {
        try {
          const children = await listDirectory(sessionId, key);
          setSubEntries(prev => ({ ...prev, [key]: children }));
        } catch {
          // silently ignore
        }
      }
      setExpanded(prev => new Set([...prev, key]));
    }
  };

  const goUp = () => {
    const parent = cwd.split("/").slice(0, -1).join("/") || "/";
    loadDir(parent);
    setExpanded(new Set());
    setSubEntries({});
  };

  const renderEntries = (list: DirEntry[], depth = 0) => (
    <>
      {list.map((entry) => (
        <div key={entry.path}>
          <div
            className={`fe-row ${entry.is_dir ? "fe-row--dir" : "fe-row--file"} ${selectedFile?.path === entry.path ? "fe-row--selected" : ""}`}
            style={{ paddingLeft: `${12 + depth * 16}px` }}
            onClick={() => {
              if (entry.is_dir) {
                handleToggleDir(entry);
              } else {
                setSelectedFile(entry);
              }
            }}
          >
            <span className="fe-arrow">
              {entry.is_dir ? (expanded.has(entry.path) ? "▼" : "▶") : " "}
            </span>
            <span className="fe-icon">{getFileIcon(entry)}</span>
            <span className="fe-name">{entry.name}</span>
            {entry.size != null && (
              <span className="fe-size">{formatSize(entry.size)}</span>
            )}
          </div>
          {entry.is_dir && expanded.has(entry.path) && subEntries[entry.path] && (
            renderEntries(subEntries[entry.path], depth + 1)
          )}
        </div>
      ))}
    </>
  );

  const cwdParts = cwd.split("/").filter(Boolean);

  return (
    <div className="file-explorer">
      {/* Toolbar */}
      <div className="fe-toolbar">
        <button className="fe-btn" onClick={goUp} title="上一層" disabled={!cwd || cwd === "/"}>
          ↑
        </button>
        <button className="fe-btn" onClick={() => loadDir(cwd)} title="重新整理">
          ↻
        </button>
        <div className="fe-breadcrumb">
          <span className="fe-breadcrumb-item" onClick={() => loadDir("/")}>
            /
          </span>
          {cwdParts.map((part, i) => {
            const path = "/" + cwdParts.slice(0, i + 1).join("/");
            return (
              <span key={path}>
                <span className="fe-breadcrumb-sep">/</span>
                <span className="fe-breadcrumb-item" onClick={() => loadDir(path)}>
                  {part}
                </span>
              </span>
            );
          })}
        </div>
        <button
          className={`fe-btn fe-btn--dot ${showDotfiles ? "fe-btn--active" : ""}`}
          onClick={() => setShowDotfiles(p => !p)}
          title="顯示/隱藏隱藏檔案"
        >
          .
        </button>
      </div>

      {/* Split body */}
      <div className="fe-split">
        {/* Left: file tree */}
        <div className="fe-left">
          <div className="fe-body">
            {loading && <div className="fe-status">載入中…</div>}
            {error && <div className="fe-status fe-status--error">{error}</div>}
            {!loading && !error && entries.length === 0 && (
              <div className="fe-status">（空目錄）</div>
            )}
            {!loading && !error && renderEntries(entries)}
          </div>
        </div>

        {/* Right: file viewer */}
        <div className="fe-right">
          <FileViewer sessionId={sessionId} file={selectedFile} />
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run tests — verify they pass**

```bash
npm test -- FileExplorer.test 2>&1
```

Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/FileExplorer/FileExplorer.tsx src/components/FileExplorer/FileExplorer.test.tsx
git commit -m "feat(FileExplorer): split-pane layout with file selection"
```

---

### Task 5: CSS for split-pane and FileViewer

**Files:**
- Modify: `src/components/FileExplorer/FileExplorer.css`

- [ ] **Step 1: Add split-pane and selected-row styles to `FileExplorer.css`**

Append to the end of `src/components/FileExplorer/FileExplorer.css`:

```css
/* ── Split pane ── */
.fe-split {
  display: flex;
  flex-direction: row;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.fe-left {
  width: 260px;
  min-width: 180px;
  flex-shrink: 0;
  border-right: 1px solid #222;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.fe-left .fe-body {
  flex: 1;
  overflow-y: auto;
}

.fe-right {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

/* ── File row — clickable ── */
.fe-row--file {
  cursor: pointer;
}

.fe-row--selected {
  background: #1e3a5f !important;
}

.fe-row--selected .fe-name {
  color: #79c0ff;
}
```

- [ ] **Step 2: Add FileViewer styles to `FileExplorer.css`**

Continue appending to `FileExplorer.css`:

```css
/* ── FileViewer ── */
.fv-root {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
  font-family: "Cascadia Mono", Consolas, monospace;
  font-size: 13px;
  background: #0c0c0c;
}

.fv-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #444;
  font-size: 13px;
  font-family: "Cascadia Mono", Consolas, monospace;
}

.fv-header {
  padding: 6px 12px;
  background: #141414;
  border-bottom: 1px solid #222;
  color: #aaa;
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex-shrink: 0;
}

.fv-banner {
  padding: 4px 12px;
  background: #3a2e00;
  color: #fcd34d;
  font-size: 11px;
  flex-shrink: 0;
}

.fv-status {
  padding: 16px;
  color: #555;
  font-size: 12px;
  text-align: center;
}

.fv-status--error {
  color: #f87171;
}

.fv-status--muted {
  color: #555;
}

.fv-content {
  flex: 1;
  overflow: auto;
  padding: 8px 0;
}

.fv-content::-webkit-scrollbar {
  width: 5px;
  height: 5px;
}

.fv-content::-webkit-scrollbar-thumb {
  background: #333;
  border-radius: 3px;
}

.fv-pre {
  margin: 0;
  padding: 0;
  font-family: inherit;
  font-size: inherit;
  white-space: pre;
}

.fv-line {
  display: flex;
  min-height: 1.4em;
}

.fv-lineno {
  user-select: none;
  text-align: right;
  min-width: 40px;
  padding-right: 12px;
  color: #444;
  flex-shrink: 0;
}

.fv-linetext {
  color: #ccc;
  flex: 1;
}
```

- [ ] **Step 3: Run the full test suite**

```bash
npm test 2>&1
```

Expected: all tests PASS.

- [ ] **Step 4: Commit**

```bash
git add src/components/FileExplorer/FileExplorer.css
git commit -m "style(FileExplorer): split-pane layout and FileViewer styles"
```

---

### Task 6: Full integration smoke test

**Files:** none (manual verification)

- [ ] **Step 1: Build and run the app**

```bash
npm run tauri:dev 2>&1
```

- [ ] **Step 2: Verify behaviour**

1. Open the Files tab — file tree appears on the left, right pane shows「選擇左側檔案以預覽內容」
2. Click a text file — right pane shows file name in header and content with line numbers
3. Click another file — right pane updates to new file
4. Click a directory — tree expands/collapses, right pane does not change
5. Click a binary file (e.g. a `.png`) — right pane shows「此檔案為二進位格式，無法預覽」
6. Click a file > 10 MB (if available) — yellow truncation banner appears

- [ ] **Step 3: Commit if any fixes needed, then tag the feature complete**

```bash
git add -p  # stage any fixes
git commit -m "fix: <describe any fix>"
```
