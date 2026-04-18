# File Content Viewer — Design Spec

**Date:** 2026-04-17
**Status:** Approved

## Overview

When the user clicks a file in the Files tab, its content is displayed in a right-side panel alongside the file tree. The layout is a horizontal split-pane inside the existing `FileExplorer` component.

## Layout

```
FileExplorer (flex-row)
├── Left pane  (~260px, fixed)  — existing directory tree
└── Right pane (flex: 1)        — FileViewer component
```

The left pane retains all existing behaviour (expand/collapse directories, breadcrumb, dotfile toggle, refresh). The right pane is new. When no file is selected it shows an empty state hint.

## Data Flow

1. User clicks a non-directory entry in the file tree.
2. `FileExplorer` updates `selectedFile: DirEntry | null` state.
3. `FileViewer` receives `sessionId` and `file` props; calls the new IPC `pty_read_file(id, path)`.
4. Rust backend reads the file, caps at 10 MB, and returns `{ content: String, truncated: bool }`.
5. `FileViewer` renders the content with line numbers.

## New / Modified Files

### Rust — `src-tauri/src/pty/commands.rs`
Add `pty_read_file` Tauri command:
- Accepts `id: String`, `path: String`.
- Reads the file; if size > 10 MB, reads only the first 10 MB and sets `truncated = true`.
- Returns `FileContent { content: String, truncated: bool }` on success.
- Returns `Err(String)` if the file cannot be read or decoded as UTF-8 (binary).
- Register the command in `src-tauri/src/lib.rs`.

### TypeScript — `src/ipc/fs.ts`
Add:
```ts
export interface FileContent { content: string; truncated: boolean; }
export const readFile = (id: string, path: string): Promise<FileContent> =>
  invoke<FileContent>("pty_read_file", { id, path });
```

### New component — `src/components/FileExplorer/FileViewer.tsx`
Props:
```ts
interface FileViewerProps {
  sessionId: string;
  file: DirEntry | null;
}
```
Behaviour:
- `file === null` → empty state: grey centered text「選擇左側檔案以預覽內容」
- `file` set → call `readFile`, show loading spinner while fetching
- Success → render `<pre>` with line numbers, monospace font, scrollable
- `truncated === true` → yellow banner at top「檔案過大，僅顯示前 10 MB」
- Error (read failure) → red error message in content area
- Binary / UTF-8 decode failure → grey message「此檔案為二進位格式，無法預覽」

Header: show current file name (from `file.name`) and full path as tooltip.

### Modified — `src/components/FileExplorer/FileExplorer.tsx`
- Add `selectedFile: DirEntry | null` state (default `null`).
- In `renderEntries`: for non-directory entries, `onClick={() => setSelectedFile(entry)}`.
  Directory entries keep `onClick={() => handleToggleDir(entry)}`.
- Add selected highlight style to clicked file row.
- Wrap existing tree markup and new `<FileViewer>` in a `flex-row` container.

### CSS — `src/components/FileExplorer/FileExplorer.css`
- Add `.fe-split` flex-row container styles.
- Add `.fe-left` (fixed 260px, border-right) and `.fe-right` (flex: 1, overflow hidden) pane styles.
- Add `.fe-row--selected` highlight style.

## Error Handling

| Situation | UI Response |
|-----------|-------------|
| Read failure (permissions, missing) | Red error text in right pane |
| Binary / non-UTF-8 file | Grey message: 「此檔案為二進位格式，無法預覽」 |
| File > 10 MB | Show content + yellow banner at top |
| Loading | Spinner / loading text in right pane |

## Out of Scope

- Syntax highlighting (can be added later)
- Editable file content
- Search within file
- Resizable split pane divider
