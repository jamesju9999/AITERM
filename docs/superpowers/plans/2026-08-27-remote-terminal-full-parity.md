# 遠端終端機共享 計畫③A：觀看端 UI/UX 完整對等 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓遠端分頁（`RemoteTerminalView`）的外觀、尺寸、分段卡片、書籤、結構化輸入框都跟本機終端機分頁一致，唯一差別是資料流由遠端而不是本機 PTY 而來。

**Architecture:** `useTerminalBlocks` 這個純粹解析輸出位元組流的 hook 改成可插拔寫入目標（`write`）與主控端平台（`hostPlatform`），本機分頁沿用預設值、行為不變；遠端分頁傳自己的 `write`（依控制權決定是否真的送出）與從協定新欄位 `host_os` 換算出的 `hostPlatform`。`RemoteTerminalView` 組裝這個共用 hook、既有的 `WarpInput`、既有的 `TerminalBlockCard`，加上一段自己的即時逐鍵轉送與字級自動縮放邏輯。

**Tech Stack:** React 19、TypeScript、xterm.js、Rust（Tauri commands 與 WebSocket 協定）。

---

## 背景事實（寫這份計畫時查證過現有程式碼，不要重查一次）

- `useTerminalBlocks` 目前**沒有**匯出 `clearAllBlocks`（`UseTerminalBlocksResult` 介面裡沒有這個欄位）。原 spec 說 Resync 要呼叫它，但沒提到要先把它加進回傳物件——這是原 spec 的缺口，這份計畫的 Task 2 會補上。
- `useTerminalBlocks` 新增的 `write`/`hostPlatform` 參數若直接被依賴陣列引用會有真正的效能/正確性問題：本機分頁不會傳這兩個參數，會落到函式簽名的**預設值運算式**——TypeScript/JavaScript 的預設參數值是「每次呼叫時求值」，`write` 的預設值 `(data) => writePty(sessionId, data)` 是一個箭頭函式，**每次呼叫 `useTerminalBlocks`（也就是每次 `TerminalView` 重新渲染）都會產生一個全新的函式參考**。如果把 `write` 直接放進某個 `useEffect`/`useCallback` 的依賴陣列，會讓本機分頁那些 effect 在每次 render 都重新註冊/取消註冊——這正是「本機分頁呼叫端字面上不用改」這句承諾看似成立、但行為上悄悄變差的陷阱。修法：用 `useRef` 橋接（這個 repo 到處都在用的既有模式，`phaseRef`、`onCommandSettled`、`onCommandStarted` 的 doc comment 都提到「必須是穩定的參考」），把 `write` 存進 `writeRef.current`，所有要呼叫 `write(...)` 的地方一律呼叫 `writeRef.current(...)`，這樣沒有任何依賴陣列需要放 `write` 本身。`hostPlatform` 沒有這個問題（它是字串，React 用 `Object.is` 比較字串值本身，同值不會觸發重跑），可以放心放進依賴陣列。
- `RemoteTerminalView` 已經有一段 StrictMode 斷線修復（`disconnectTimerRef` 那個 `useEffect`，2026-08-27 剛修好）——**這份計畫的所有 Task 都不能刪除或改動這段**，它跟這次要做的事完全獨立。
- `RemoteTerminalView` 現有的 `term.onData` 直接判斷 `phase.mode === "control"` 才呼叫 `shareViewerSend`；這次要把這段邏輯包成一個穩定的 `write` 函式（用 `useCallback` + `phaseRef` 讀最新值，身分只跟著 `connId` 變），同時餵給 `useTerminalBlocks` 的 `write` 參數、`WarpInput` 的送出路徑（`submitCommand`）、以及即時逐鍵轉送。
- `TerminalBlockCard` 的 `onAskAi`/`onBookmark`/`onCopy` 都是 optional props，且 Ask AI 按鈕本身是 `{isFailed && onAskAi && (...)}` 條件渲染，git 徽章是 `{block.gitInfo && (...)}` 條件渲染——這代表「隱藏 Ask AI、不顯示 git 徽章」不需要改 `TerminalBlockCard` 本身，遠端分頁只要不傳 `onAskAi`、永遠不呼叫 `setBlockGitInfo` 就自然達成。`onCopy`（複製到剪貼簿）不需要主控端本機資料，直接照樣接。
- 本機分頁 `term.onData` 裡有兩條特殊處理：忽略 focus-tracking 跳脫序列（`\x1b[O`/`\x1b[I`）、AI 面板開著時不轉送（`panelOpenRef.current`）。**遠端分頁只需要複製第一條**——`panelOpenRef` 是 `TerminalView` 自己的 component-local state，遠端分頁根本沒有 AI 面板這個概念（3A 明確排除），複製第二條等於憑空造一個永遠是 false 的判斷式，沒有意義。
- `useTerminalBlocks` 的 `sessionId` 參數，在拿掉 `writePty(sessionId, ...)` 這條路徑之後，剩下的用途只有「truthy 檢查」（`if (!term || !sessionId) return;`）——遠端分頁直接傳 `connId` 進去就好，不需要另外發明一個假的 session id 概念，語意上也合理（「這個 hook 實例對應哪一條連線」）。
- 字級自動縮放：用「量測目前字級下的字元格 CSS 像素尺寸，線性外推到目標字級」的算法，而不是二分搜尋反覆試探——字元格尺寸與字級成正比是排版學上站得住腳的假設（等寬字型的 em-box 本來就線性縮放），一次量測就能算出答案，不需要迭代設定字級。量測用這個 repo 已有先例的 escape hatch（`TerminalView.tsx` 算 `liveHeightPx` 那段用的同一條路徑 `_core._renderService.dimensions.css.cell`）。
- 協定新欄位 `host_os` 走的是 `Granted`/`Resize` 共用的同一個 `ServerMessage::Granted` 變體（`Resize` 分支目前拿 `mode: String::new()` 代表「這是後續 resize，不是初次授權」，`host_os` 也比照辦理給空字串）。
- `src-tauri/tests/share_end_to_end.rs` 有三處完整列出 `ServerMessage::Granted { mode: ..., cols: 80, rows: 24 }` 欄位的斷言（沒有用 `..` 展開），加了 `host_os` 欄位後這三處**不補上就會編譯失敗**——`src-tauri/tests/share_viewer.rs:130` 用的是 `{ cols, rows, .. }`，不受影響。

## 檔案總覽

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/src/share/protocol.rs` | 修改 | `Granted` 加 `host_os: String` |
| `src-tauri/src/share/server.rs` | 修改 | 送出 `Granted` 時帶上 `std::env::consts::OS.to_string()` |
| `src-tauri/src/share/viewer.rs` | 修改 | `ViewerEvent::Granted` 加 `host_os`；轉發時帶上；`Resize` 分支給空字串 |
| `src-tauri/src/share/viewer_manager.rs` | 修改 | `GrantedPayload` 加 `host_os`（camelCase → `hostOs`） |
| `src-tauri/tests/share_end_to_end.rs` | 修改 | 三處 `Granted` 斷言補 `host_os` |
| `src/ipc/shareViewer.ts` | 修改 | `ViewerGranted` 加 `hostOs: string` |
| `src/hooks/useTerminalBlocks.ts` | 修改 | 加 `write`/`hostPlatform` 參數（ref 橋接）、匯出 `clearAllBlocks`、三處寫入改用 `write`、Windows 判斷改用 `hostPlatform` |
| `src/hooks/useTerminalBlocks.test.ts` | 修改 | 新增兩個測試：`hostPlatform` 生效、`write` 生效 |
| `src/components/RemoteTerminalView/index.tsx` | 修改 | 外觀對等、自動縮放、整合 `useTerminalBlocks`/`WarpInput`/`TerminalBlockCard`、Resync 清卡片 |
| `src/components/RemoteTerminalView/index.css` | 修改 | 分段卡片容器、`WarpInput` 容器樣式 |
| `src/components/RemoteTerminalView/index.test.tsx` | 修改 | 新增三個測試：唯讀停用、AI 前綴提示、Resync 清卡片 |
| `src/lib/i18n.ts` | 修改 | 新增「AI 指令不支援於遠端分頁」字串（zh-TW + en） |

---

### Task 1: 協定新增 `host_os`（Rust 全端）

**Files:**
- Modify: `src-tauri/src/share/protocol.rs`
- Modify: `src-tauri/src/share/server.rs`
- Modify: `src-tauri/src/share/viewer.rs`
- Modify: `src-tauri/src/share/viewer_manager.rs`
- Modify: `src-tauri/tests/share_end_to_end.rs`

- [ ] **Step 1: `protocol.rs` 的 `Granted` 加欄位**

找到：

```rust
    /// 已獲准。`cols`/`rows` 是主控端的終端機尺寸——觀看端必須照這個建立
    /// xterm，不能用自己的視窗大小。緊接著會來一個二進位 frame 作為重播。
    Granted { mode: WireAccessMode, cols: u16, rows: u16 },
```

改成：

```rust
    /// 已獲准。`cols`/`rows` 是主控端的終端機尺寸——觀看端必須照這個建立
    /// xterm，不能用自己的視窗大小。緊接著會來一個二進位 frame 作為重播。
    ///
    /// `host_os` 是主控端的 `std::env::consts::OS`（`"windows"`/`"macos"`/
    /// `"linux"`）。觀看端拿它取代 `navigator.platform` 判斷——這個值只有
    /// 觀看端的平台，不是主控端的，跨平台分享時原本的判斷會誤判或失效
    /// （見計畫③A 設計文件）。不含使用者名稱、路徑等敏感資訊。
    Granted { mode: WireAccessMode, cols: u16, rows: u16, host_os: String },
```

- [ ] **Step 2: `server.rs` 送出時帶上**

找到：

```rust
    if !send_control(&mut ws, &ServerMessage::Granted { mode, cols, rows }).await {
        state.registry.remove_viewer(&tab_id, &viewer_id);
        return;
    }
```

改成：

```rust
    if !send_control(
        &mut ws,
        &ServerMessage::Granted { mode, cols, rows, host_os: std::env::consts::OS.to_string() },
    )
    .await
    {
        state.registry.remove_viewer(&tab_id, &viewer_id);
        return;
    }
```

- [ ] **Step 3: `viewer.rs` 的 `ViewerEvent::Granted` 加欄位並轉發**

找到：

```rust
pub enum ViewerEvent {
    /// 對方同意了。`cols`/`rows` 是主控端的終端機尺寸，觀看端必須照這個
    /// 建立 xterm，不能用自己的視窗大小。
    Granted { mode: String, cols: u16, rows: u16 },
```

改成：

```rust
pub enum ViewerEvent {
    /// 對方同意了。`cols`/`rows` 是主控端的終端機尺寸，觀看端必須照這個
    /// 建立 xterm，不能用自己的視窗大小。`host_os` 見
    /// `protocol::ServerMessage::Granted` 的說明；`Resize`（見下方轉發處）
    /// 沒有這個資訊，給空字串。
    Granted { mode: String, cols: u16, rows: u16, host_os: String },
```

找到：

```rust
                    match msg {
                        ServerMessage::Granted { mode, cols, rows } => {
                            let _ = events.send(ViewerEvent::Granted {
                                mode: wire_mode_str(mode),
                                cols,
                                rows,
                            });
                        }
                        ServerMessage::Resize { cols, rows } => {
                            // 尺寸變更跟 Granted 用同一個事件——上層只要照著
                            // 重新 fit 即可，不需要區分是初次還是後續。
                            let _ = events.send(ViewerEvent::Granted {
                                mode: String::new(),
                                cols,
                                rows,
                            });
                        }
```

改成：

```rust
                    match msg {
                        ServerMessage::Granted { mode, cols, rows, host_os } => {
                            let _ = events.send(ViewerEvent::Granted {
                                mode: wire_mode_str(mode),
                                cols,
                                rows,
                                host_os,
                            });
                        }
                        ServerMessage::Resize { cols, rows } => {
                            // 尺寸變更跟 Granted 用同一個事件——上層只要照著
                            // 重新 fit 即可，不需要區分是初次還是後續。`mode`
                            // 空字串代表非初次；`host_os` 同理，這個事件沒有
                            // 這個資訊，上層看到空字串就不更新已經記住的值。
                            let _ = events.send(ViewerEvent::Granted {
                                mode: String::new(),
                                cols,
                                rows,
                                host_os: String::new(),
                            });
                        }
```

- [ ] **Step 4: `viewer_manager.rs` 的 `GrantedPayload` 加欄位並轉發**

找到：

```rust
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GrantedPayload {
    mode: String,
    cols: u16,
    rows: u16,
}
```

改成：

```rust
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GrantedPayload {
    mode: String,
    cols: u16,
    rows: u16,
    host_os: String,
}
```

找到：

```rust
                    ViewerEvent::Granted { mode, cols, rows } => {
                        let _ = app.emit(
                            &format!("share-viewer://granted/{id}"),
                            GrantedPayload { mode, cols, rows },
                        );
                    }
```

改成：

```rust
                    ViewerEvent::Granted { mode, cols, rows, host_os } => {
                        let _ = app.emit(
                            &format!("share-viewer://granted/{id}"),
                            GrantedPayload { mode, cols, rows, host_os },
                        );
                    }
```

- [ ] **Step 5: 修三處測試斷言**

`src-tauri/tests/share_end_to_end.rs` 裡有三處（分別在 `a_read_only_viewer_cannot_type`、`a_controlling_viewer_can_type_and_sees_the_result`、`revoking_control_tells_the_viewer_it_can_no_longer_type` 這三個測試函式裡）：

```rust
        ServerMessage::Granted { mode: WireAccessMode::ReadOnly, cols: 80, rows: 24 }
```

改成：

```rust
        ServerMessage::Granted {
            mode: WireAccessMode::ReadOnly,
            cols: 80,
            rows: 24,
            host_os: std::env::consts::OS.to_string(),
        }
```

以及兩處：

```rust
        ServerMessage::Granted { mode: WireAccessMode::Control, cols: 80, rows: 24 }
```

都改成：

```rust
        ServerMessage::Granted {
            mode: WireAccessMode::Control,
            cols: 80,
            rows: 24,
            host_os: std::env::consts::OS.to_string(),
        }
```

- [ ] **Step 6: 編譯與既有測試確認**

Run: `cd src-tauri && cargo test --lib && cargo test --test share_end_to_end && cargo test --test share_viewer`
Expected: 全部 PASS（既有測試不受影響，只是多了一個欄位）

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/share/protocol.rs src-tauri/src/share/server.rs src-tauri/src/share/viewer.rs src-tauri/src/share/viewer_manager.rs src-tauri/tests/share_end_to_end.rs
git commit -m "feat(share): add host_os to the Granted message for cross-platform viewers"
```

---

### Task 2: `useTerminalBlocks` 介面擴充（`write`/`hostPlatform`/`clearAllBlocks`）

**Files:**
- Modify: `src/hooks/useTerminalBlocks.ts`
- Modify: `src/hooks/useTerminalBlocks.test.ts`

- [ ] **Step 1: 函式簽名加兩個參數，內部建立 `writeRef` 橋接**

找到：

```typescript
export function useTerminalBlocks(
  sessionId: string,
  term: Terminal | null,
  cwdRef?: React.RefObject<string>,
  onLiveClear?: () => void,
  /** 每次有指令跑完就呼叫，帶上它的 exit code。給側邊欄提示點用。
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接）——它進了下面
   *  OSC handler effect 的依賴陣列，每次換身分都會重新註冊 handler。 */
  onCommandSettled?: (exitCode: number) => void,
  /** 每次開始追蹤一個新指令就呼叫，帶上指令文字。給「偵測使用者跑了什麼」用。
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接）——它進了 submitCommand
   *  與 beginTrackedBlock 的依賴陣列，每次換身分都會讓兩者的識別跟著變。 */
  onCommandStarted?: (cmd: string) => void,
): UseTerminalBlocksResult {
  const [blocks, setBlocks] = useState<TerminalBlock[]>([]);
  const [isAlternateBuffer, setIsAlternateBuffer] = useState(false);
```

改成：

```typescript
export function useTerminalBlocks(
  sessionId: string,
  term: Terminal | null,
  cwdRef?: React.RefObject<string>,
  onLiveClear?: () => void,
  /** 每次有指令跑完就呼叫，帶上它的 exit code。給側邊欄提示點用。
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接）——它進了下面
   *  OSC handler effect 的依賴陣列，每次換身分都會重新註冊 handler。 */
  onCommandSettled?: (exitCode: number) => void,
  /** 每次開始追蹤一個新指令就呼叫，帶上指令文字。給「偵測使用者跑了什麼」用。
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接）——它進了 submitCommand
   *  與 beginTrackedBlock 的依賴陣列，每次換身分都會讓兩者的識別跟著變。 */
  onCommandStarted?: (cmd: string) => void,
  /** 指令怎麼寫出去。預設包一層 `writePty(sessionId, data)`，跟改動前的
   *  行為完全一樣。遠端分頁傳 `(data) => shareViewerSend(connId, data)`。
   *
   *  **不要把這個參數本身放進任何 useEffect/useCallback 的依賴陣列**：
   *  呼叫端沒有明確傳值時會落到這個預設值運算式，而預設參數是每次呼叫
   *  都重新求值的——本機分頁因此每次 render 都會拿到一個全新的函式參考。
   *  下面用 `writeRef` 橋接解決，內部一律呼叫 `writeRef.current(...)`。 */
  write: (data: string) => void = (data) => writePty(sessionId, data),
  /** 主控端平台，只影響 Windows ConPTY 的 Ctrl+L 清畫面同步邏輯。預設讀
   *  `navigator.platform`，跟改動前行為一樣；遠端分頁傳 `Granted` 訊息裡
   *  的 `host_os`。這是字串值，可以放心放進依賴陣列（不像 `write` 是函式
   *  參考，同樣的字串值不會觸發 React 重新執行 effect）。 */
  hostPlatform: "windows" | "other" = navigator.platform.toLowerCase().startsWith("win") ? "windows" : "other",
): UseTerminalBlocksResult {
  const [blocks, setBlocks] = useState<TerminalBlock[]>([]);
  const [isAlternateBuffer, setIsAlternateBuffer] = useState(false);

  const writeRef = useRef(write);
  writeRef.current = write;
```

- [ ] **Step 2: `UseTerminalBlocksResult` 加 `clearAllBlocks`**

找到：

```typescript
export interface UseTerminalBlocksResult {
  blocks: TerminalBlock[];
  submitCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  beginTrackedBlock: (cmd: string) => void;
  appendOutput: (chunk: string) => void;
  setBlockGitInfo: (id: string, info: GitBlockInfo | null) => void;
  isAlternateBuffer: boolean;
  termInstance: Terminal | null;
  /** 強制把一個 running 中的區塊結案（例如卡在 heredoc 的中斷）。
   *  會呼叫該區塊等待中的 onComplete callback——見 finalizeBlock 內部實作。 */
  finalizeBlock: (blockId: string, exitCode: number, opts?: { clearOnParsed?: boolean }) => void;
}
```

改成：

```typescript
export interface UseTerminalBlocksResult {
  blocks: TerminalBlock[];
  submitCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  beginTrackedBlock: (cmd: string) => void;
  appendOutput: (chunk: string) => void;
  setBlockGitInfo: (id: string, info: GitBlockInfo | null) => void;
  isAlternateBuffer: boolean;
  termInstance: Terminal | null;
  /** 強制把一個 running 中的區塊結案（例如卡在 heredoc 的中斷）。
   *  會呼叫該區塊等待中的 onComplete callback——見 finalizeBlock 內部實作。 */
  finalizeBlock: (blockId: string, exitCode: number, opts?: { clearOnParsed?: boolean }) => void;
  /** 清空整個分段卡片歷史。原本只在內部處理 `clear`/`cls` 指令時用；
   *  遠端分頁在收到 `Resync`（漏位元組、全量重播）時也要呼叫這個——
   *  漏掉的位元組可能連帶讓卡片內容跟畫面對不上，這跟本機分頁執行
   *  `clear`/`cls` 時「畫面跟卡片一起清空」是同一個邏輯。 */
  clearAllBlocks: () => void;
}
```

- [ ] **Step 3: OSC handler effect 改用 `writeRef`、`hostPlatform`**

找到：

```typescript
        const isWindows = navigator.platform.toLowerCase().startsWith("win");
        if (isWindows) {
          setTimeout(() => {
            term?.clear();
            term?.scrollToBottom();
            onLiveClear?.();
            // Re-sync ConPTY with xterm's now-cleared buffer. term.clear() only
            // reset xterm; ConPTY still models the prompt at whatever row it
            // scrolled to, and PowerShell/PSReadLine redraws the next input line
            // with ABSOLUTE cursor positioning (e.g. ESC[24;34H) from that stale
            // model — landing every keystroke far below the visible row-0 prompt
            // (proven via byte-stream logging: first input used ESC[1;..H and
            // worked; the second, after a scrolled command, used ESC[24;..H and
            // stuck at row 11). Ctrl+L makes the shell itself clear and re-home
            // the prompt to the top, so ConPTY's model and xterm's cleared
            // buffer agree again — and, as a bonus, a resize then replays only
            // the clean prompt instead of the stale duplicated output.
            writePty(sessionId, "\x0c").catch(console.error);
          }, 0);
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode);
        } else {
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode, { clearOnParsed: true });
        }
```

改成：

```typescript
        const isWindows = hostPlatform === "windows";
        if (isWindows) {
          setTimeout(() => {
            term?.clear();
            term?.scrollToBottom();
            onLiveClear?.();
            // Re-sync ConPTY with xterm's now-cleared buffer. term.clear() only
            // reset xterm; ConPTY still models the prompt at whatever row it
            // scrolled to, and PowerShell/PSReadLine redraws the next input line
            // with ABSOLUTE cursor positioning (e.g. ESC[24;34H) from that stale
            // model — landing every keystroke far below the visible row-0 prompt
            // (proven via byte-stream logging: first input used ESC[1;..H and
            // worked; the second, after a scrolled command, used ESC[24;..H and
            // stuck at row 11). Ctrl+L makes the shell itself clear and re-home
            // the prompt to the top, so ConPTY's model and xterm's cleared
            // buffer agree again — and, as a bonus, a resize then replays only
            // the clean prompt instead of the stale duplicated output.
            writeRef.current("\x0c");
          }, 0);
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode);
        } else {
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode, { clearOnParsed: true });
        }
```

找到這個 effect 結尾的依賴陣列：

```typescript
    // sessionId is in the deps because the Windows Ctrl+L resync in the D
    // handler writes to it — the effect must re-register once the PTY session
    // id lands (it's set async after `term`) so the handler's closure isn't
    // holding the initial empty id.
  }, [term, finalizeBlock, onLiveClear, sessionId, onCommandSettled]);
```

改成：

```typescript
    // `write` 故意不在這裡——它透過 writeRef 讀取，不需要讓這個 effect
    // 跟著它的身分重新註冊（本機分頁沒傳 write 時，每次 render 呼叫端拿到
    // 的都是函式簽名裡那個預設值運算式產生的全新參考，放進依賴陣列會讓
    // 這個 effect 每次 render 都 dispose+重新註冊）。`hostPlatform` 是字串，
    // 沒有這個問題，放心加進來。
  }, [term, finalizeBlock, onLiveClear, onCommandSettled, hostPlatform]);
```

- [ ] **Step 4: `submitCommand` 改用 `writeRef`、`hostPlatform`**

找到：

```typescript
  const submitCommand = useCallback(
    (cmd: string, onComplete?: (block: TerminalBlock) => void) => {
      if (!term || !sessionId) return;

      onCommandStarted?.(cmd);

      // On Windows conpty: \x15 echoes as visible "^U", and \x1b gets merged with
      // the first char of the command as an Alt+key (e.g. \x1b + "d" = Alt+D which
      // deletes a word, dropping the "d").  WarpInput owns all keyboard input so the
      // PTY line is always empty — no clear sequence needed on Windows.
      // On macOS/Linux, \x15 (Ctrl+U) clears bash/zsh input silently.
      const isWindows = navigator.platform.toLowerCase().startsWith("win");
      const clearSeq = isWindows ? "" : "\x15";

      if (isClearCommand(cmd)) {
        // `clear`/`cls` wipes the whole block history, not just the live viewport —
        // matches what a real terminal's clear does. Still forward the command
        // to the shell (keeps shell-side history/state in sync) but don't track
        // a block for it — there's nothing meaningful to show in a card for it.
        clearAllBlocks();
        writePty(sessionId, clearSeq + cmd + "\r").catch(console.error);
        return;
      }
```

改成：

```typescript
  const submitCommand = useCallback(
    (cmd: string, onComplete?: (block: TerminalBlock) => void) => {
      if (!term || !sessionId) return;

      onCommandStarted?.(cmd);

      // On Windows conpty: \x15 echoes as visible "^U", and \x1b gets merged with
      // the first char of the command as an Alt+key (e.g. \x1b + "d" = Alt+D which
      // deletes a word, dropping the "d").  WarpInput owns all keyboard input so the
      // PTY line is always empty — no clear sequence needed on Windows.
      // On macOS/Linux, \x15 (Ctrl+U) clears bash/zsh input silently.
      const isWindows = hostPlatform === "windows";
      const clearSeq = isWindows ? "" : "\x15";

      if (isClearCommand(cmd)) {
        // `clear`/`cls` wipes the whole block history, not just the live viewport —
        // matches what a real terminal's clear does. Still forward the command
        // to the shell (keeps shell-side history/state in sync) but don't track
        // a block for it — there's nothing meaningful to show in a card for it.
        clearAllBlocks();
        writeRef.current(clearSeq + cmd + "\r");
        return;
      }
```

再找到 `submitCommand` 最後寫入 PTY 那行：

```typescript
      // Clear the current line before sending the command (see isWindows/clearSeq above).
      writePty(sessionId, clearSeq + cmd + "\r").catch(console.error);
    },
    [sessionId, term, cwdRef, finalizeBlock, clearAllBlocks, onCommandStarted],
  );
```

改成：

```typescript
      // Clear the current line before sending the command (see isWindows/clearSeq above).
      writeRef.current(clearSeq + cmd + "\r");
    },
    [sessionId, term, cwdRef, finalizeBlock, clearAllBlocks, onCommandStarted, hostPlatform],
  );
```

- [ ] **Step 5: 回傳物件加 `clearAllBlocks`**

找到：

```typescript
  return {
    blocks,
    submitCommand,
    beginTrackedBlock,
    appendOutput,
    setBlockGitInfo,
    isAlternateBuffer,
    termInstance: term,
    finalizeBlock,
  };
}
```

改成：

```typescript
  return {
    blocks,
    submitCommand,
    beginTrackedBlock,
    appendOutput,
    setBlockGitInfo,
    isAlternateBuffer,
    termInstance: term,
    finalizeBlock,
    clearAllBlocks,
  };
}
```

- [ ] **Step 6: 確認既有測試不受影響**

Run: `npx vitest run src/hooks/useTerminalBlocks.test.ts src/hooks/useTerminalBlocks.interrupt.test.ts`
Expected: 全部 PASS（本機分頁呼叫端 `useTerminalBlocks("session-1", term)` 沒有傳 `write`/`hostPlatform`，行為應該完全不變）

- [ ] **Step 7: 寫兩個新測試**

在 `src/hooks/useTerminalBlocks.test.ts` 檔案最後（最後一個 `it(...)` 之後、`});` 收尾之前）加：

```typescript
  it("uses hostPlatform instead of navigator.platform for the Windows ConPTY resync", async () => {
    // 模擬「觀看端自己在 Windows 上跑，但主控端是別的系統」——這正是
    // hostPlatform 存在的理由：navigator.platform 量到的是觀看端的平台，
    // 用它來判斷「要不要送 ConPTY 專屬的 Ctrl+L」在跨平台分享時會誤判。
    const originalPlatform = navigator.platform;
    Object.defineProperty(navigator, "platform", { value: "Win32", configurable: true });

    try {
      const { result } = renderHook(() =>
        useTerminalBlocks(
          "session-1",
          term,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          "other",
        ),
      );

      act(() => {
        result.current.submitCommand("echo hi");
      });
      writePtyMock.mockClear();

      await act(async () => {
        await writeToTerm(term, "\x1b]133;D;0\x07");
      });

      await waitFor(() => {
        expect(result.current.blocks[0].status).toBe("completed");
      });

      // hostPlatform 是 "other"，即使 navigator.platform 說是 Windows，
      // 也不該送出 ConPTY 專屬的 Ctrl+L 同步位元組。
      expect(writePtyMock).not.toHaveBeenCalledWith("session-1", "\x0c");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });

  it("routes writes through a custom write function instead of writePty", async () => {
    // hostPlatform 明確傳 "other"（而不是留給預設值去讀 navigator.platform）
    // ——這個測試只關心「write 有沒有接對」，不該讓斷言的期待值隨著執行測試
    // 的機器 navigator.platform 是什麼而變動（"other" 時 submitCommand 會在
    // 指令前面加 "\x15" 清行，"windows" 時不會，見 submitCommand 內部的
    // clearSeq 邏輯）。
    const writeMock = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks(
        "session-1",
        term,
        undefined,
        undefined,
        undefined,
        undefined,
        writeMock,
        "other",
      ),
    );

    act(() => {
      result.current.submitCommand("echo hi");
    });

    expect(writeMock).toHaveBeenCalledWith("\x15echo hi\r");
    // 忘記接新的 write、其實還是寫去本機 PTY 的話，這裡就會抓到。
    expect(writePtyMock).not.toHaveBeenCalled();
  });
```

- [ ] **Step 8: 執行確認新測試過**

Run: `npx vitest run src/hooks/useTerminalBlocks.test.ts`
Expected: 全部 PASS（含兩個新測試）

- [ ] **Step 9: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤

- [ ] **Step 10: Commit**

```bash
git add src/hooks/useTerminalBlocks.ts src/hooks/useTerminalBlocks.test.ts
git commit -m "feat(share): make useTerminalBlocks' write target and platform pluggable"
```

---

### Task 3: `RemoteTerminalView` 外觀對等（字型/主題）

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx`

- [ ] **Step 1: import 區塊加需要的東西**

找到：

```typescript
import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  onShareViewerControlChanged,
  onShareViewerData,
  onShareViewerEnded,
  onShareViewerGranted,
  onShareViewerResync,
  shareViewerDisconnect,
  shareViewerSend,
} from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import type { Translations } from "../../lib/i18n";
import "./index.css";
```

改成：

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import {
  onShareViewerControlChanged,
  onShareViewerData,
  onShareViewerEnded,
  onShareViewerGranted,
  onShareViewerResync,
  shareViewerDisconnect,
  shareViewerSend,
} from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import { getActiveTheme, type AppTheme } from "../../lib/themes";
import type { Translations } from "../../lib/i18n";
import "./index.css";
```

`FitAddon` 被拿掉了——尺寸永遠由主控端決定（`term.resize(cols, rows)`），觀看端不會用 `FitAddon.fit()` 去重新計算 cols/rows（那會改變終端機尺寸，違反「主控端決定尺寸」的原則）。Task 4 會用一套不同的邏輯（縮字級，不是縮 cols/rows）取代它。

- [ ] **Step 2: xterm 建立時套用字型/主題設定**

找到：

```typescript
  // xterm 的建立與銷毀。刻意跟事件訂閱分開——訂閱只依賴 connId，終端機
  // 只依賴掛載，兩者的生命週期不同。
  useEffect(() => {
    if (!hostRef.current) return;
    const term = new Terminal({ convertEol: false, cursorBlink: false });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current);
    termRef.current = term;
```

改成：

```typescript
  // xterm 的建立與銷毀。刻意跟事件訂閱分開——訂閱只依賴 connId，終端機
  // 只依賴掛載，兩者的生命週期不同。
  useEffect(() => {
    if (!hostRef.current) return;

    // 跟本機分頁（TerminalView.tsx）讀同一份設定來源，讓遠端分頁的外觀
    // 一開始就對得上，不用等使用者去改設定才同步。
    const initFontSize = parseInt(localStorage.getItem("aiterm-font-size") ?? "14", 10) || 14;
    const initFontFamily = localStorage.getItem("aiterm-font-family") ?? '"Cascadia Mono", Consolas, monospace';
    const initTheme = getActiveTheme();

    const term = new Terminal({
      fontFamily: initFontFamily,
      fontSize: initFontSize,
      lineHeight: 1.1,
      cursorBlink: true,
      theme: initTheme.xterm,
      convertEol: false,
    });
    term.open(hostRef.current);
    termRef.current = term;
```

- [ ] **Step 3: 即時同步字型/主題變更**

在同一個 `useEffect` 裡，找到：

```typescript
    return () => {
      onData?.dispose?.();
      term.dispose();
      termRef.current = null;
    };
  }, [connId]);
```

改成（在 `return` 之前插入兩段事件監聽，並在清理函式裡一併移除）：

```typescript
    const onFontChanged = (e: Event) => {
      const { fontSize, fontFamily } = (e as CustomEvent).detail as { fontSize: number; fontFamily: string };
      term.options.fontSize = fontSize;
      term.options.fontFamily = fontFamily;
      // 字型/字級變了，同樣的 cols×rows 需要的容器空間也變了——Task 4
      // 加的 recomputeFontSize 會在這個函式存在後接手這件事。
      recomputeFontSizeRef.current?.();
    };
    window.addEventListener("aiterm:font-changed", onFontChanged);

    const onThemeChanged = (e: Event) => {
      const { theme } = (e as CustomEvent).detail as { theme: AppTheme };
      term.options.theme = theme.xterm;
    };
    window.addEventListener("aiterm:theme-changed", onThemeChanged);

    return () => {
      window.removeEventListener("aiterm:font-changed", onFontChanged);
      window.removeEventListener("aiterm:theme-changed", onThemeChanged);
      onData?.dispose?.();
      term.dispose();
      termRef.current = null;
    };
  }, [connId]);
```

這一步先把 ref 宣告（初始值 `null`）跟呼叫點都寫好，型別檢查會過（`recomputeFontSizeRef.current?.()` 在 ref 是 `useRef<(() => void) | null>(null)` 時完全合法）；**真正把它賦值成算字級的函式是 Task 4 做的事**，這個 Task 做完之後呼叫它都還只是 no-op（值仍是 `null`），這是預期中的暫時狀態，不是漏做。

在檔案裡新增這個 ref（跟 `disconnectTimerRef` 放在一起，元件最上面幾行）：

找到：

```typescript
  // `phase` 要在事件 callback 裡讀到最新值，但那些 callback 只註冊一次——
  // 用 ref 避免 stale closure（這個 repo 在 Tauri 事件監聽上踩過這個坑）。
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
```

改成：

```typescript
  // `phase` 要在事件 callback 裡讀到最新值，但那些 callback 只註冊一次——
  // 用 ref 避免 stale closure（這個 repo 在 Tauri 事件監聽上踩過這個坑）。
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  // Task 4 會賦值成真正的字級重算函式；這裡先放 ref 讓 xterm 建立那個
  // effect（先寫）能呼叫到它，即使賦值它的 effect（後寫）還沒跑。
  const recomputeFontSizeRef = useRef<(() => void) | null>(null);
```

- [ ] **Step 4: 型別檢查（`useCallback` 這時候還沒用到，但已經 import，避免未使用警告先確認）**

Run: `npx tsc -b`
Expected: 目前應該還沒有錯誤——`useCallback` 雖然 import 了但還沒使用，TypeScript 對「import 了但沒用的具名匯入」不報錯（這條規則只在 ESLint 的 `no-unused-vars` 才會抓，而那要等 Task 4 用到它就沒事了）。若這裡真的跳出 unused import 的 tsc 錯誤，先跳過這一步的檢查，直接進 Task 4（同一個檔案，馬上就會用到）。

- [ ] **Step 5: Commit**

```bash
git add src/components/RemoteTerminalView/index.tsx
git commit -m "feat(share): sync RemoteTerminalView's xterm appearance with app settings"
```

---

### Task 4: `RemoteTerminalView` 尺寸自動縮放

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Modify: `src/components/RemoteTerminalView/index.css`

- [ ] **Step 1: 加字級計算的純函式（檔案最後，`endReasonText` 後面）**

```typescript
/**
 * 量測 `term` 目前字級下的字元格 CSS 像素尺寸，線性外推到「剛好能讓
 * `cols`×`rows` 塞進 `containerWidth`×`containerHeight`」的字級。
 *
 * 用線性外推而不是二分搜尋反覆調字級再量測：等寬字型的字元格尺寸本來就
 * 跟字級成正比（CSS `font-size` 的定義就是線性縮放），量一次目前字級的
 * 尺寸就能直接算出答案，不需要迭代。
 *
 * 量測用的 `_core._renderService.dimensions.css.cell` 是 xterm.js 沒有
 * 公開的內部欄位——這個 repo 已經有先例（`TerminalView.tsx` 算
 * `liveHeightPx` 用的是同一條路徑），這裡沿用同樣的 escape hatch。量不到
 * 時（例如字型還沒載入完成）回傳 `null`，呼叫端維持目前字級不做任何事。
 *
 * 回傳值夾在 [8, 32] 之間：8 是「還能看得清楚」的下限（spec 用語：最小
 * 可讀字級），塞不下的話交給 CSS 捲軸，不繼續往下縮；32 純粹是防呆上限，
 * 避免極端情況（例如容器剛好非常大、cols/rows 很小）算出離譜的字級。
 */
function computeFittingFontSize(
  term: Terminal,
  cols: number,
  rows: number,
  containerWidth: number,
  containerHeight: number,
): number | null {
  const dims = (
    term as unknown as {
      _core?: { _renderService?: { dimensions?: { css?: { cell?: { width?: number; height?: number } } } } };
    }
  )._core?._renderService?.dimensions?.css?.cell;
  if (!dims?.width || !dims?.height || cols <= 0 || rows <= 0) return null;

  const currentFontSize = term.options.fontSize ?? 14;
  const maxByWidth = (containerWidth * currentFontSize) / (cols * dims.width);
  const maxByHeight = (containerHeight * currentFontSize) / (rows * dims.height);
  const fitted = Math.floor(Math.min(maxByWidth, maxByHeight));
  return Math.max(8, Math.min(fitted, 32));
}
```

- [ ] **Step 2: 元件內建立 cols/rows ref、`recomputeFontSize`、ResizeObserver**

找到（Task 3 加的那個 ref 後面）：

```typescript
  // Task 4 會賦值成真正的字級重算函式；這裡先放 ref 讓 xterm 建立那個
  // effect（先寫）能呼叫到它，即使賦值它的 effect（後寫）還沒跑。
  const recomputeFontSizeRef = useRef<(() => void) | null>(null);
```

改成：

```typescript
  // Task 4 會賦值成真正的字級重算函式；這裡先放 ref 讓 xterm 建立那個
  // effect（先寫）能呼叫到它，即使賦值它的 effect（後寫）還沒跑。
  const recomputeFontSizeRef = useRef<(() => void) | null>(null);

  // 主控端最後一次告知的尺寸——ResizeObserver 的 callback 要用到「當下」
  // 的 cols/rows，但它是掛載時註冊一次的閉包，不會自動看到後續 Granted/
  // Resize 事件更新的值，所以用 ref 橋接（這個 repo 在 Tauri 事件監聽上
  // 踩過同一類坑）。
  const sizeRef = useRef({ cols: 80, rows: 24 });

  const recomputeFontSize = useCallback(() => {
    const term = termRef.current;
    const host = hostRef.current;
    if (!term || !host) return;
    const { cols, rows } = sizeRef.current;
    const fitted = computeFittingFontSize(term, cols, rows, host.clientWidth, host.clientHeight);
    if (fitted !== null) term.options.fontSize = fitted;
  }, []);

  useEffect(() => {
    recomputeFontSizeRef.current = recomputeFontSize;
  }, [recomputeFontSize]);

  useEffect(() => {
    if (!hostRef.current) return;
    const ro = new ResizeObserver(() => recomputeFontSizeRef.current?.());
    ro.observe(hostRef.current);
    return () => ro.disconnect();
  }, []);
```

- [ ] **Step 3: `Granted`/`Resize` 事件更新 `sizeRef` 並重算字級**

找到：

```typescript
    track(
      onShareViewerGranted(connId, ({ mode, cols, rows }) => {
        // 尺寸由主控端說了算——照它給的建立，不用自己的視窗大小。
        // `mode` 為空字串代表這是後續的 resize 通知，不是初次核准。
        if (mode) setPhase({ kind: "live", mode });
        const term = termRef.current;
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
        }
      }),
    );
```

改成：

```typescript
    track(
      onShareViewerGranted(connId, ({ mode, cols, rows }) => {
        // 尺寸由主控端說了算——照它給的建立，不用自己的視窗大小。
        // `mode` 為空字串代表這是後續的 resize 通知，不是初次核准。
        if (mode) setPhase({ kind: "live", mode });
        const term = termRef.current;
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
          sizeRef.current = { cols, rows };
          recomputeFontSizeRef.current?.();
        }
      }),
    );
```

- [ ] **Step 4: CSS——內層容器才能出捲軸，外層維持 `clip`**

找到 `src/components/RemoteTerminalView/index.css` 的：

```css
.aiterm-remote-terminal__screen {
  flex: 1;
  min-height: 0;
  overflow: clip;
}
```

改成：

```css
.aiterm-remote-terminal__screen {
  flex: 1;
  min-height: 0;
  overflow: clip;
  display: flex;
}

/* 字級縮到最小（8px）仍塞不下 cols×rows 時，捲軸只開在這一層——外層
   `.aiterm-remote-terminal`/`.aiterm-remote-terminal__screen` 維持
   `overflow: clip`。整層改用 `overflow: auto` 會重現這個 repo 踩過的
   「貼上內容被瀏覽器捲出視野變空白」那個坑（見 .aiterm-live-frame 的
   同類註解）。 */
.aiterm-remote-terminal__scroll {
  flex: 1;
  min-width: 0;
  overflow: auto;
}
```

- [ ] **Step 5: JSX 加上這層捲軸容器，xterm 掛在它裡面**

找到：

```typescript
      <div className="aiterm-remote-terminal__screen" ref={hostRef} />
    </div>
  );
}
```

改成：

```typescript
      <div className="aiterm-remote-terminal__screen">
        <div className="aiterm-remote-terminal__scroll" ref={hostRef} />
      </div>
    </div>
  );
}
```

**注意**：這一步把 `ResizeObserver` 觀察的對象（`hostRef`）跟 xterm 掛載的對象維持同一個——`hostRef` 現在是內層可捲動容器，`term.open(hostRef.current)` 那行不用改，`hostRef.current.clientWidth/clientHeight` 量到的就是可捲動視窗的可視尺寸，符合「量容器能塞多少」的需求。

- [ ] **Step 6: 型別檢查與既有測試**

Run: `npx tsc -b && npx vitest run src/components/RemoteTerminalView`
Expected: 兩者都乾淨（既有 5 個測試不動，行為不受影響——`computeFittingFontSize` 在 jsdom 測試環境下量不到 `_core._renderService...`，會回傳 `null`，`recomputeFontSize` 因此是安全的 no-op，不會讓既有測試因為量不到尺寸而炸掉）

- [ ] **Step 7: Commit**

```bash
git add src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.css
git commit -m "feat(share): auto-scale RemoteTerminalView's font to fit cols×rows"
```

---

### Task 5: 接上 `useTerminalBlocks`

**Files:**
- Modify: `src/ipc/shareViewer.ts`
- Modify: `src/components/RemoteTerminalView/index.tsx`

- [ ] **Step 1: `ViewerGranted` 加 `hostOs`**

找到：

```typescript
export interface ViewerGranted {
  /** `"read_only"` 或 `"control"`。 */
  mode: string;
  /** 主控端的終端機尺寸——xterm 必須照這個建，不能用自己的視窗大小。 */
  cols: number;
  rows: number;
}
```

改成：

```typescript
export interface ViewerGranted {
  /** `"read_only"` 或 `"control"`。 */
  mode: string;
  /** 主控端的終端機尺寸——xterm 必須照這個建，不能用自己的視窗大小。 */
  cols: number;
  rows: number;
  /** 主控端的 `std::env::consts::OS`（`"windows"`/`"macos"`/`"linux"`）。
   *  後續 resize 通知（`mode` 為空字串那種）這欄也是空字串，不用理它。 */
  hostOs: string;
}
```

- [ ] **Step 2: `RemoteTerminalView` 記住 `hostOs`，轉成 `hostPlatform`**

找到：

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import {
  onShareViewerControlChanged,
  onShareViewerData,
  onShareViewerEnded,
  onShareViewerGranted,
  onShareViewerResync,
  shareViewerDisconnect,
  shareViewerSend,
} from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import { getActiveTheme, type AppTheme } from "../../lib/themes";
import type { Translations } from "../../lib/i18n";
import "./index.css";
```

改成：

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import {
  onShareViewerControlChanged,
  onShareViewerData,
  onShareViewerEnded,
  onShareViewerGranted,
  onShareViewerResync,
  shareViewerDisconnect,
  shareViewerSend,
} from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import { getActiveTheme, type AppTheme } from "../../lib/themes";
import { useTerminalBlocks } from "../../hooks/useTerminalBlocks";
import type { Translations } from "../../lib/i18n";
import "./index.css";
```

找到：

```typescript
  // 主控端最後一次告知的尺寸——ResizeObserver 的 callback 要用到「當下」
  // 的 cols/rows，但它是掛載時註冊一次的閉包，不會自動看到後續 Granted/
  // Resize 事件更新的值，所以用 ref 橋接（這個 repo 在 Tauri 事件監聽上
  // 踩過同一類坑）。
  const sizeRef = useRef({ cols: 80, rows: 24 });
```

改成：

```typescript
  // 主控端最後一次告知的尺寸——ResizeObserver 的 callback 要用到「當下」
  // 的 cols/rows，但它是掛載時註冊一次的閉包，不會自動看到後續 Granted/
  // Resize 事件更新的值，所以用 ref 橋接（這個 repo 在 Tauri 事件監聽上
  // 踩過同一類坑）。
  const sizeRef = useRef({ cols: 80, rows: 24 });

  // Granted 事件的 hostOs 空字串代表「這是後續 resize 通知，不是初次
  // 授權」——只有非空字串才更新，讓 hostPlatform 維持第一次拿到的值。
  const [hostPlatform, setHostPlatform] = useState<"windows" | "other">("other");
```

找到：

```typescript
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
          sizeRef.current = { cols, rows };
          recomputeFontSizeRef.current?.();
        }
      }),
    );
```

上面這段所在的 `onShareViewerGranted` callback，把解構參數也一併改掉。完整找到：

```typescript
    track(
      onShareViewerGranted(connId, ({ mode, cols, rows }) => {
        // 尺寸由主控端說了算——照它給的建立，不用自己的視窗大小。
        // `mode` 為空字串代表這是後續的 resize 通知，不是初次核准。
        if (mode) setPhase({ kind: "live", mode });
        const term = termRef.current;
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
          sizeRef.current = { cols, rows };
          recomputeFontSizeRef.current?.();
        }
      }),
    );
```

改成：

```typescript
    track(
      onShareViewerGranted(connId, ({ mode, cols, rows, hostOs }) => {
        // 尺寸由主控端說了算——照它給的建立，不用自己的視窗大小。
        // `mode` 為空字串代表這是後續的 resize 通知，不是初次核准。
        if (mode) setPhase({ kind: "live", mode });
        if (hostOs) setHostPlatform(hostOs === "windows" ? "windows" : "other");
        const term = termRef.current;
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
          sizeRef.current = { cols, rows };
          recomputeFontSizeRef.current?.();
        }
      }),
    );
```

- [ ] **Step 3: 把 `termRef` 換成「ref + state」雙軌**

`useTerminalBlocks` 內部的 `useEffect` 依賴 `term` 這個參數的值才能在 xterm 準備好之後才註冊 OSC handler——純 `useRef` 存的值不會觸發 re-render，`useTerminalBlocks` 因此永遠不會知道「xterm 現在建好了」。本機分頁 `TerminalView.tsx` 的做法是兩個都存：ref 給不需要 re-render 的地方快速讀取，state 給需要在建立完成後觸發下游 effect 的地方。這一步先把這個雙軌準備好，`onData` 內部邏輯留給下一步再動。

找到：

```typescript
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
```

改成：

```typescript
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const [termState, setTermState] = useState<Terminal | null>(null);
```

找到（Task 3 建立 xterm 的那個 `useEffect`）：

```typescript
    term.open(hostRef.current);
    termRef.current = term;

    const onData = term.onData((data: string) => {
      // **唯讀時按鍵根本不送出**，不是送了被伺服器拒絕。伺服器端還有一道
      // `may_send_input`，但那是安全邊界；這一層是給使用者的即時回饋。
      const p = phaseRef.current;
      if (p.kind === "live" && p.mode === "control") {
        void shareViewerSend(connId, data);
      }
    });
```

改成（只加 `setTermState(term)`，`onData` 內容不動）：

```typescript
    term.open(hostRef.current);
    termRef.current = term;
    setTermState(term);

    const onData = term.onData((data: string) => {
      // **唯讀時按鍵根本不送出**，不是送了被伺服器拒絕。伺服器端還有一道
      // `may_send_input`，但那是安全邊界；這一層是給使用者的即時回饋。
      const p = phaseRef.current;
      if (p.kind === "live" && p.mode === "control") {
        void shareViewerSend(connId, data);
      }
    });
```

找到同一個 `useEffect` 的清理函式：

```typescript
    return () => {
      window.removeEventListener("aiterm:font-changed", onFontChanged);
      window.removeEventListener("aiterm:theme-changed", onThemeChanged);
      onData?.dispose?.();
      term.dispose();
      termRef.current = null;
    };
  }, [connId]);
```

改成：

```typescript
    return () => {
      window.removeEventListener("aiterm:font-changed", onFontChanged);
      window.removeEventListener("aiterm:theme-changed", onThemeChanged);
      onData?.dispose?.();
      term.dispose();
      termRef.current = null;
      setTermState(null);
    };
  }, [connId]);
```

- [ ] **Step 4: 建立穩定的 `write`，改寫 `term.onData`**

在元件函式體裡（`phaseRef.current = phase;` 那行後面、`recomputeFontSizeRef` 那個 ref 之前）加：

```typescript
  // 有控制權時才真的送出，唯讀時整個是 no-op——**唯讀時按鍵根本不送出**，
  // 不是送了被伺服器拒絕（伺服器端還有 may_send_input 這道安全邊界，這
  // 一層是給使用者的即時回饋）。用 phaseRef 讀最新值，身分只跟著 connId
  // 變，這樣可以放心把它傳給 useTerminalBlocks（見該 hook 對 write 參數
  // 的說明：它自己也用 ref 橋接，不會因為這裡身分穩定與否而有額外負擔，
  // 但兩邊都求穩比較不容易出錯）。
  const write = useCallback(
    (data: string) => {
      const p = phaseRef.current;
      if (p.kind === "live" && p.mode === "control") {
        void shareViewerSend(connId, data);
      }
    },
    [connId],
  );
```

找到（Step 3 做完之後，`useEffect` 裡的這段——注意這裡已經有 Step 3 加的 `setTermState(term);`）：

```typescript
    term.open(hostRef.current);
    termRef.current = term;
    setTermState(term);

    const onData = term.onData((data: string) => {
      // **唯讀時按鍵根本不送出**，不是送了被伺服器拒絕。伺服器端還有一道
      // `may_send_input`，但那是安全邊界；這一層是給使用者的即時回饋。
      const p = phaseRef.current;
      if (p.kind === "live" && p.mode === "control") {
        void shareViewerSend(connId, data);
      }
    });
```

改成（`term.onData` 改叫上面新建的 `write`，並加上忽略 focus-tracking 跳脫序列——本機分頁 `term.onData` 裡還有一條「AI 面板開著時不轉送」，遠端分頁沒有 AI 面板這個概念，不需要複製那條；`setTermState(term)` 那行不動）：

```typescript
    term.open(hostRef.current);
    termRef.current = term;
    setTermState(term);

    const onData = term.onData((data: string) => {
      // Drop focus-tracking events that xterm.js emits when it loses / gains
      // focus（跟本機分頁 TerminalView.tsx 的 term.onData 同一條規則、同一個
      // 理由：PSReadLine 開了 focus tracking，逐字轉送這兩個序列會讓
      // PowerShell 把它們印成字面上的 "[O"/"[I"）。本機分頁另外還有一條
      // 「AI 面板開著時不轉送」，遠端分頁沒有 AI 面板這個概念，不複製。
      if (data === "\x1b[O" || data === "\x1b[I") return;
      write(data);
    });
```

- [ ] **Step 5: 呼叫 `useTerminalBlocks`**

`hostPlatform` state 在 Step 2 已經加好、`termState` 在 Step 3 已經加好——這一步只是把它們接進 `useTerminalBlocks`。在 `write` 定義之後（Step 4 加的那段）加：

```typescript
  const { blocks, submitCommand, clearAllBlocks } = useTerminalBlocks(
    connId,
    termState,
    undefined,
    undefined,
    undefined,
    undefined,
    write,
    hostPlatform,
  );
```

- [ ] **Step 6: 型別檢查與既有測試**

在跑之前，先更新 `index.test.tsx` 頂端的 xterm mock。`useTerminalBlocks` 內部還會存取 `term.parser.registerOscHandler`、`term.buffer.onBufferChange`/`term.buffer.active.type`、`term.scrollToBottom`、`term.cols`、`term.options.*`——既有的 mock class 只覆蓋了 `RemoteTerminalView` 自己直接用到的那幾個方法，沒有涵蓋這些，接上 `useTerminalBlocks` 就會需要它們（已經對照 `useTerminalBlocks.ts` 原始碼窮舉過，不是憑經驗猜的）。

另外，Task 3 已經把 `RemoteTerminalView/index.tsx` 裡 `FitAddon` 的 import 拿掉了（尺寸永遠由主控端決定，不再用 `FitAddon.fit()`），所以 `vi.mock("@xterm/addon-fit", ...)` 這行也該一併清掉，不然會變成指向一個元件已經不再使用的套件的死 mock。

找到：

```typescript
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    write = writeMock;
    clear = clearMock;
    open = vi.fn();
    dispose = vi.fn();
    onData = vi.fn();
    loadAddon = vi.fn();
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit = vi.fn(); } }));
```

改成：

```typescript
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    write = writeMock;
    clear = clearMock;
    open = vi.fn();
    dispose = vi.fn();
    onData = vi.fn();
    loadAddon = vi.fn();
    scrollToBottom = vi.fn();
    resize = vi.fn();
    cols = 80;
    options: Record<string, unknown> = {};
    parser = { registerOscHandler: vi.fn(() => ({ dispose: vi.fn() })) };
    buffer = { onBufferChange: vi.fn(() => ({ dispose: vi.fn() })), active: { type: "normal" } };
  },
}));
```

Run: `npx tsc -b && npx vitest run src/components/RemoteTerminalView`
Expected: 兩者皆乾淨

- [ ] **Step 7: Commit**

```bash
git add src/ipc/shareViewer.ts src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx
git commit -m "feat(share): wire useTerminalBlocks into RemoteTerminalView"
```

---

### Task 6: 分段卡片列表、書籤、`WarpInput`

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Modify: `src/components/RemoteTerminalView/index.css`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: i18n 加一句提示**

`src/lib/i18n.ts` 的 zh-TW 區塊，找到 `remote_terminal_read_only`（既有的遠端分頁字串應該在附近），在它後面加：

```typescript
    remote_terminal_ai_unsupported: "AI 指令目前不支援於遠端分頁。",
```

en 區塊的對應位置加：

```typescript
    remote_terminal_ai_unsupported: "AI commands are not supported in remote tabs yet.",
```

（若找不到 `remote_terminal_read_only` 這把 key，改用 `grep -n "remote_terminal_" src/lib/i18n.ts` 找到遠端分頁字串群組所在的兩個位置，各自在群組內加上這一行，維持 zh-TW/en 兩邊的 key 集合一致——這是 `Translations` 型別能自動推導成功的前提。）

- [ ] **Step 2: import `WarpInput`、`TerminalBlockCard`、`addBookmark`、`parseAiPrefix`/`parseAgentPrefix`**

找到：

```typescript
import { getActiveTheme, type AppTheme } from "../../lib/themes";
import { useTerminalBlocks } from "../../hooks/useTerminalBlocks";
import type { Translations } from "../../lib/i18n";
import "./index.css";
```

改成：

```typescript
import { getActiveTheme, type AppTheme } from "../../lib/themes";
import { useTerminalBlocks } from "../../hooks/useTerminalBlocks";
import { WarpInput } from "../WarpInput";
import { TerminalBlockCard } from "../TerminalBlockCard";
import { addBookmark } from "../CommandBookmarks";
import { parseAiPrefix, parseAgentPrefix } from "../parseAiPrefix";
import type { Translations } from "../../lib/i18n";
import "./index.css";
```

- [ ] **Step 3: AI 前綴偵測 + 送出邏輯**

在 `useTerminalBlocks` 呼叫後面加：

```typescript
  const [aiUnsupported, setAiUnsupported] = useState(false);

  // WarpInput 送出的整行文字先過一次 AI 前綴檢查——跟本機分頁用同一套
  // parseAiPrefix.ts 規則，不重新猜字首。是 /ai 或 /agent 開頭就不送出、
  // 顯示提示；否則走 submitCommand（會建立分段卡片並透過 write 送出）。
  const handleWarpSubmit = useCallback(
    (cmd: string) => {
      if (parseAiPrefix(cmd) !== null || parseAgentPrefix(cmd) !== null) {
        setAiUnsupported(true);
        return;
      }
      setAiUnsupported(false);
      submitCommand(cmd);
    },
    [submitCommand],
  );
```

- [ ] **Step 4: JSX——分段卡片列表 + `WarpInput`**

找到：

```typescript
      {phase.kind === "ended" && (
        <div className="aiterm-remote-terminal__banner aiterm-remote-terminal__banner--ended">
          {endReasonText(t, phase.reason)}
        </div>
      )}

      <div className="aiterm-remote-terminal__screen">
        <div className="aiterm-remote-terminal__scroll" ref={hostRef} />
      </div>
    </div>
  );
}
```

改成：

```typescript
      {phase.kind === "ended" && (
        <div className="aiterm-remote-terminal__banner aiterm-remote-terminal__banner--ended">
          {endReasonText(t, phase.reason)}
        </div>
      )}

      {/* 分段卡片：跟本機分頁同一套過濾條件（只顯示已結束且已完成 ANSI
          解析的），複用 TerminalBlockCard——不傳 onAskAi，Ask AI 按鈕本身
          是 `{isFailed && onAskAi && (...)}` 條件渲染，不傳就不會出現；
          block.gitInfo 永遠是 undefined（這裡從不呼叫 setBlockGitInfo），
          git 徽章同理自然不出現。 */}
      <div className="aiterm-remote-terminal__blocks">
        {blocks
          .filter((b) => b.status !== "running" && b.renderedLines)
          .map((b) => (
            <TerminalBlockCard
              key={b.id}
              block={b}
              onBookmark={(command) => addBookmark(command)}
              onCopy={(command) => navigator.clipboard.writeText(command).catch(console.error)}
            />
          ))}
      </div>

      <div className="aiterm-remote-terminal__screen">
        <div className="aiterm-remote-terminal__scroll" ref={hostRef} />
      </div>

      {aiUnsupported && (
        <div className="aiterm-remote-terminal__ai-unsupported">{t.remote_terminal_ai_unsupported}</div>
      )}

      <WarpInput
        onSubmit={handleWarpSubmit}
        disabled={!(phase.kind === "live" && phase.mode === "control")}
      />
    </div>
  );
}
```

（`WarpInput` 不傳 `sessionId`——`WarpInput.tsx` 本身在沒有 `sessionId` 時，路徑自動完成按鈕會 `disabled={!sessionId}`，資料夾選單開啟時也會直接給空陣列，這個功能因此自然停用，不需要另外處理。）

- [ ] **Step 5: CSS**

在 `src/components/RemoteTerminalView/index.css` 最後加：

```css
.aiterm-remote-terminal__blocks {
  overflow-y: auto;
  flex-shrink: 0;
  max-height: 40%;
}

.aiterm-remote-terminal__ai-unsupported {
  padding: 6px 12px;
  font-size: 12px;
  color: var(--aiterm-warn-fg, #fbbf24);
  background: var(--aiterm-warn-bg, #78350f33);
  flex-shrink: 0;
}
```

- [ ] **Step 6: 型別檢查與既有測試**

Run: `npx tsc -b && npx vitest run src/components/RemoteTerminalView`
Expected: 兩者皆乾淨

- [ ] **Step 7: Commit**

```bash
git add src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.css src/lib/i18n.ts
git commit -m "feat(share): add block cards, bookmarks, and WarpInput to RemoteTerminalView"
```

---

### Task 7: Resync 清空卡片 + 新測試

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Modify: `src/components/RemoteTerminalView/index.test.tsx`

- [ ] **Step 1: 寫三個失敗測試**

在 `src/components/RemoteTerminalView/index.test.tsx`，先確認頂端的 mock 區塊——`vi.mock("@xterm/xterm", ...)` 那個假 `Terminal` class 需要在 Task 5 Step 6 已經補齊 `parser`/`buffer`/`options`/`cols`（若那一步還沒做，先回頭做完，這三個新測試都要靠它才能讓 `useTerminalBlocks` 正常運作）。

加三個測試（`describe("RemoteTerminalView", ...)` 區塊內，既有五個測試後面）：

```typescript
  it("disables WarpInput while read-only", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c8" sas="7777" isActive />);
    await waitFor(() => expect(handlers["granted:c8"]).toBeDefined());

    handlers["granted:c8"]({ mode: "read_only", cols: 80, rows: 24, hostOs: "linux" } as never);

    await waitFor(() => {
      const textarea = screen.getByPlaceholderText(/輸入指令|Type a command/i);
      expect(textarea).toBeDisabled();
    });
  });

  it("shows a hint and does not send /ai or /agent commands", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c9" sas="8888" isActive />);
    await waitFor(() => expect(handlers["granted:c9"]).toBeDefined());
    handlers["granted:c9"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

    const textarea = await screen.findByPlaceholderText(/輸入指令|Type a command/i);
    await waitFor(() => expect(textarea).not.toBeDisabled());

    await userEvent.type(textarea, "/ai fix this{Enter}");

    expect(await screen.findByText(/AI 指令目前不支援|not supported in remote/i)).toBeInTheDocument();
    expect(sendMock).not.toHaveBeenCalled();
  });

  it("clears the block list on resync, not just the xterm buffer", async () => {
    render(<RemoteTerminalView tabId="t1" connId="c10" sas="9999" isActive />);
    await waitFor(() => expect(handlers["granted:c10"]).toBeDefined());
    handlers["granted:c10"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

    const textarea = await screen.findByPlaceholderText(/輸入指令|Type a command/i);
    await waitFor(() => expect(textarea).not.toBeDisabled());
    await userEvent.type(textarea, "echo hi{Enter}");

    await waitFor(() => expect(handlers["resync:c10"]).toBeDefined());
    handlers["resync:c10"](undefined as never);

    // Resync 之後不該還看得到 resync 之前追蹤的指令卡片標題。
    await waitFor(() => {
      expect(screen.queryByText("echo hi")).not.toBeInTheDocument();
    });
  });
```

- [ ] **Step 2: 執行確認前兩個已經過（Task 5/6 已經實作），第三個失敗**

Run: `npx vitest run src/components/RemoteTerminalView`
Expected: 前兩個新測試 PASS（Task 5/6 已經做了唯讀停用跟 AI 前綴提示），第三個 FAIL（Resync 還沒接 `clearAllBlocks`）

- [ ] **Step 3: Resync 呼叫 `clearAllBlocks`**

找到：

```typescript
    track(
      onShareViewerResync(connId, () => {
        // 清空再接全量重播。漏掉的位元組可能截斷 ANSI 逃脫序列，帶著壞掉
        // 的畫面繼續是不會自己好的。
        termRef.current?.clear();
      }),
    );
```

改成：

```typescript
    track(
      onShareViewerResync(connId, () => {
        // 清空再接全量重播。漏掉的位元組可能截斷 ANSI 逃脫序列，帶著壞掉
        // 的畫面繼續是不會自己好的。分段卡片內容也是從同一批位元組解析
        // 出來的，漏掉的部分同樣可能讓卡片內容跟畫面對不上——跟本機分頁
        // 執行 clear/cls 時「畫面跟卡片一起清空」是同一個邏輯。
        termRef.current?.clear();
        clearAllBlocksRef.current();
      }),
    );
```

`clearAllBlocks` 是 `useTerminalBlocks` 回傳的，而這個事件訂閱的 `useEffect` 只依賴 `[connId]`（在 `useTerminalBlocks` 呼叫「之前」定義，順序上 `RemoteTerminalView` 目前是先訂閱事件、後面才建立 xterm/呼叫 hook）——用 ref 橋接，不要把 `clearAllBlocks` 直接放進這個 effect 的依賴陣列（會導致每次 `blocks` 變動、`clearAllBlocks` 身分若不穩定就要重新訂閱一次所有 `share-viewer://*` 事件，沒有必要）。

在 `useTerminalBlocks` 呼叫後面加：

```typescript
  const clearAllBlocksRef = useRef(clearAllBlocks);
  clearAllBlocksRef.current = clearAllBlocks;
```

- [ ] **Step 4: 執行確認全部測試過**

Run: `npx vitest run src/components/RemoteTerminalView`
Expected: 全部 PASS（既有 5 個 + 新增 3 個 = 8 個）

- [ ] **Step 5: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤

- [ ] **Step 6: Commit**

```bash
git add src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx
git commit -m "feat(share): clear block cards on resync, not just the xterm buffer"
```

---

### Task 8: 全套驗證

**Files:** 無新增/修改——這個 task 只跑驗證。

- [ ] **Step 1: 前端全套**

Run: `npx tsc -b && npx vitest run`
Expected: `tsc` 無錯誤；vitest 全部 PASS

- [ ] **Step 2: Rust 全套**

Run: `cd src-tauri && cargo test --lib && cargo test --test share_end_to_end && cargo test --test share_viewer && cargo test --test share_commands`
Expected: 全部 PASS（既有的那個 pty::session 間歇性 flaky test 如果剛好在這次跑失敗，重跑一次確認是既有問題、不是這次改動造成的——不要因為它卡住這個 task）

- [ ] **Step 3: ESLint 沒有新增問題**

Run: `npm run lint 2>&1 | tail -5`
Expected: 錯誤/警告總數跟這份計畫開始前一致（不能比之前多）

- [ ] **Step 4: 本機分頁手動驗證（自動化測不到的部分）**

spec 明確要求：改完 `useTerminalBlocks` 後，要在**本機**分頁手動走一次這三個曾經記過教訓的行為，確認沒有退化：

1. WarpInput 歷史清單方向（↑ 往舊、↓ 往新）
2. Windows 上執行指令後畫面清除同步正常（若手邊沒有 Windows 機器，至少確認 macOS/Linux 分支——`clearOnParsed: true` 那條路徑——沒有被這次改動意外影響）
3. 貼上長內容不會被瀏覽器捲出視野變空白

- [ ] **Step 5: 兩台實機手動驗證遠端分頁（自動化測不到的部分）**

1. 遠端分頁的字型/主題是否跟本機分頁一致，改設定後遠端分頁是否即時跟著變
2. 縮放 AITerm 視窗大小，確認遠端分頁字級跟著縮放、cols/rows 不會被觀看端影響（去問主控端那邊的分頁尺寸有沒有意外被改變）
3. 縮到最小字級後畫面出現捲軸，而不是內容被裁掉或整個空白
4. 取得控制權後用 WarpInput 送出指令，確認畫面上出現分段卡片、書籤按鈕能正常加入書籤
5. 拿到控制權時直接在終端機畫面按 Ctrl+C／操作 vim，確認即時逐鍵轉送仍然正常（不透過 WarpInput 的那條通道）
6. 輸入 `/ai 隨便什麼`，確認出現「不支援」提示、指令沒有被送出
7. 主控端跟觀看端不同作業系統時（例如 Mac 主控端 + Windows 觀看端），確認 Windows 觀看端沒有誤用 Windows 專屬的 ConPTY 清畫面邏輯（那是給*主控端*是 Windows 時用的，不是觀看端自己是 Windows 就要套用）

- [ ] **Step 6: Commit（若前面步驟有任何修正）**

若 Step 1-5 過程中有修正任何檔案：

```bash
git add -A
git commit -m "fix: address issues found during final verification pass"
```

若都一次過，這個 task 不需要 commit。

---

## Spec 覆蓋檢查

| Spec 章節 | 對應 Task |
|---|---|
| 協定新增 `host_os` | Task 1 |
| `useTerminalBlocks` 的介面變更（`write`/`hostPlatform`） | Task 2 |
| 額外發現：`clearAllBlocks` 從未被匯出，Resync 需要它 | Task 2（匯出）、Task 7（接線） |
| xterm 外觀（字型/主題）跟本機分頁同步 | Task 3 |
| 尺寸：主控端決定 cols/rows，觀看端自動縮字級塞滿視窗，塞不下給捲軸 | Task 4 |
| `RemoteTerminalView` 整合 `useTerminalBlocks` | Task 5 |
| 分段卡片列表（隱藏 Ask AI、不顯示 git 徽章） | Task 6 |
| 書籤：直接重用 `CommandBookmarks` | Task 6 |
| `WarpInput` 整合（結構化輸入框、含歷史） | Task 6 |
| 即時逐鍵轉送（互動程式如 vim、Ctrl+C） | Task 5 Step 3 |
| `/ai`/`/agent` 開頭顯示提示、不送出 | Task 6 |
| 唯讀模式下 `WarpInput` 整個顯示為停用狀態 | Task 6 |
| Resync 多呼叫 `clearAllBlocks()` | Task 7 |
| 測試（`useTerminalBlocks` 新測試、`RemoteTerminalView` 新測試） | Task 2、Task 7 |
| 手動驗證（本機三個曾記過教訓的行為、遠端分頁完整流程） | Task 8 |
| 已知限制（IME/貼上快照比對不搬、git 徽章/AI 面板/路徑自動完成不可用） | 沒有對應程式碼——這些是刻意不做的事，已經在「範圍」的「不含」清單裡體現（沒有任何 Task 去實作它們） |
