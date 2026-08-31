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

// `clear` is the Unix/PowerShell screen-clear command; `cls` is cmd.exe's (and
// also a built-in PowerShell alias for Clear-Host, alongside `clear`). Matched
// case-insensitively since Windows commands aren't case-sensitive.
function isClearCommand(cmd: string): boolean {
  const trimmed = cmd.trim().toLowerCase();
  return trimmed === "clear" || trimmed === "cls";
}

/**
 * 從一個絕對緩衝區座標（OSC 133 B 標記記錄的「輸入從這裡開始」位置）到目前
 * 遊標所在行，把畫面上的文字截出來當作還原出的指令文字。只在「沒有本機
 * 追蹤區塊」時才會被呼叫——見設計文件
 * docs/superpowers/specs/2026-08-27-remote-command-text-recovery-design.md
 * 的「recoverUntrackedCommand() 演算法」一節。
 */
function recoverUntrackedCommand(
  term: Terminal,
  promptEnd: { row: number; col: number } | null,
): string | null {
  if (!promptEnd) return null;
  const { row: startRow, col: startCol } = promptEnd;
  // OSC C 觸發時，遊標已經因為 Enter 換行到新的一行，所以往上一行才是輸入
  // 內容實際結束的地方。
  const endRow = term.buffer.active.cursorY + term.buffer.active.baseY - 1;
  if (endRow < startRow) return null;

  let fullLine = "";
  for (let row = startRow; row <= endRow; row++) {
    const line = term.buffer.active.getLine(row);
    if (!line) return null;
    // A row after the first must be a genuine auto-wrap continuation of the
    // previous row (isWrapped) — a real newline in between (e.g. a remote
    // viewer's multi-line paste landing before the actual submitting Enter)
    // means this isn't a single logical input line anymore, and gluing the
    // rows together would silently fabricate wrong command text that LOOKS
    // like a successful recovery. Bail out to the safe fallback instead.
    if (row > startRow && !line.isWrapped) return null;
    fullLine += row === startRow ? line.translateToString(true, startCol) : line.translateToString(true);
  }
  const trimmed = fullLine.trim();
  return trimmed.length > 0 ? trimmed : null;
}

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
  /** **保底機制**——只有在 `recoverUntrackedCommand`（見同檔案內的定義）
   *  無法從畫面內容還原出指令文字時才會被呼叫，例如這個連線還沒收過任何
   *  OSC 133 B 標記（見 `promptEndRef`）。正常情況下遠端指令會直接透過
   *  `beginTrackedBlock` 變成完整的卡片，不會走到這裡。
   *
   *  實機測試抓到的原始 bug：遠端觀看者透過分享連線寫進 PTY 的指令，不會
   *  經過 `submitCommand`/`beginTrackedBlock`，導致 `TerminalView.tsx`
   *  「即時窗格自動撐高」邏輯賴以判斷的「有沒有一個 running 中的區塊」
   *  信號永遠是 false。這個 callback 讓呼叫端在指令文字還原失敗的少見
   *  情況下，仍能拿到「現在有東西在跑」這個單純的訊號、維持窗格至少不
   *  裁切輸出——完整解法是 `recoverUntrackedCommand` 成功時直接呼叫
   *  `beginTrackedBlock`，不需要這個訊號介入。
   *
   *  C 只在**沒有**本機追蹤區塊、且還原也失敗時才視為「開始」；D 沿用既有
   *  兩個提早 return 的分支（`prev.length === 0` / `latest.status !==
   *  "running"`）——那兩個分支本來就是「沒有東西可以結案」的判斷，同一個
   *  條件借來判斷「這次 D 沒有對應本機區塊」。
   *
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接），理由跟
   *  `onCommandSettled`/`onCommandStarted` 一樣。 */
  onUntrackedCommandBoundary?: (kind: "start" | "end") => void,
): UseTerminalBlocksResult {
  const [blocks, setBlocks] = useState<TerminalBlock[]>([]);
  const [isAlternateBuffer, setIsAlternateBuffer] = useState(false);

  const writeRef = useRef(write);
  writeRef.current = write;

  const blocksRef = useRef<TerminalBlock[]>([]);
  const completionCallbacksRef = useRef<Map<string, (block: TerminalBlock) => void>>(new Map());
  // OSC 133 B 標記記錄的「輸入從這裡開始」絕對座標，給 recoverUntrackedCommand
  // 用——只在遠端指令（沒有本機追蹤區塊）時才會被讀取，見該函式的文件註解。
  const promptEndRef = useRef<{ row: number; col: number } | null>(null);

  /**
   * 實機測試抓到的第二個 bug（跟 appendOutput 的 race 是不同根因）：
   * finalizeBlock 的 term.clear() 要等非同步的 parseAnsiToRenderedLines
   * 解析完才觸發（見下方 finalizeBlock 的文件註解，這是刻意的設計，避免
   * 「畫面先變空白、卡片才出現」的閃爍）。如果這個延遲夠久，久到下一個
   * 指令（可能是遠端觀看者送的）自己的 OSC 133 B 已經先被記錄進
   * promptEndRef，這次延遲的 clear() 一旦真的執行，就會讓那個座標對不
   * 上——但這裡不需要因此放棄「清畫面時機保持不變」這個對本機終端機
   * 已經運作良好的設計。
   *
   * 讀 xterm.js 原始碼（node_modules/@xterm/xterm/src/browser/Terminal.ts
   * 的 clear()）確認它實際上不是「整個清空重來」：它把「遊標目前所在的
   * 那一行」搬去當新的第 0 行，其餘全部丟棄。只要 clear() 觸發的當下，
   * 遊標還停在 promptEndRef 記錄的那一行（也就是使用者還沒按下 Enter、
   * 那一行的內容還原封不動），這一行的內容就會被完整保留、只是行號
   * 變成 0——欄位（col）完全不受影響。這裡就是在 clear() 發生的同時，
   * 讓 promptEndRef 的行號跟著一起「搬家」，而不是讓它整個作廢。
   *
   * 「遊標還停在 promptEndRef 記錄的那一行」這個前提**必須先檢查、不能
   * 假設一定成立**：如果使用者輸入的指令已經因為欄寬不夠而換行（游標
   * 現在停在接續行，不是 B 當初記錄的那一行），clear() 保留下來的是
   * 接續行的內容，跟 promptEndRef.col 對不上——這時候若還是無條件把
   * row 搬成 0，recoverUntrackedCommand 會從錯誤的一行切出內容，得到
   * 一個看似成功、實際上是錯的還原結果，比直接還原失敗、退回
   * onUntrackedCommandBoundary 保底機制還糟（保底機制不會生出錯誤資料）。
   * 所以要在呼叫 clear() 之前，先讀一次目前遊標的絕對行號，只有在它
   * 剛好等於 promptEndRef 記錄的行號時，才代表這個前提成立、可以安心
   * 搬遷；不成立就維持原樣，讓既有的 endRow < startRow 防呆邏輯自然
   * 接手判斷失敗。
   */
  const clearAndRebasePromptEnd = useCallback((t: Terminal) => {
    const cursorRowBeforeClear = t.buffer.active.cursorY + t.buffer.active.baseY;
    t.clear();
    // TEMP DIAG — this client-side clear is the suspected origin of the
    // xterm-vs-ConPTY row divergence behind the stuck-input report; ConPTY is
    // never told it happened. Pair this with [cursor-diag] in TerminalView.
    console.log(
      `[cursor-diag] term.clear() called — cursorY before=${cursorRowBeforeClear} after=${t.buffer.active.cursorY} baseY after=${t.buffer.active.baseY}`,
    );
    if (promptEndRef.current && promptEndRef.current.row === cursorRowBeforeClear) {
      promptEndRef.current = { row: 0, col: promptEndRef.current.col };
    }
  }, []);

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

  // Wipes the whole block history — used when the user runs `clear`, which in a
  // real terminal resets everything visible, not just whatever the live viewport
  // happens to be showing. Any block still "running" at this point is simply
  // dropped: appendOutput/finalizeBlock already no-op safely against an empty
  // blocksRef (see their own `prev.length === 0` guards), so no crash risk.
  const clearAllBlocks = useCallback(() => {
    blocksRef.current = [];
    setBlocks([]);
  }, []);

  /**
   * Marks a still-running block as completed/failed, freezes its rawOutput,
   * and kicks off headless ANSI parsing that fires+clears its onComplete
   * callback once parsing resolves. Shared by the normal OSC 133 D path and
   * the defensive "orphaned block" path in submitCommand below, so a block
   * can never be finalized more than once and its callback can never be left
   * dangling regardless of which path finalizes it.
   *
   * `opts.clearOnParsed` clears the live terminal once the card is ready to
   * take over (the original, Mac/Linux behavior — avoids a blank-screen flash
   * between "raw output disappears" and "card appears"). The OSC 133 D path
   * skips this on Windows, where it instead clears synchronously-deferred and
   * force-repaints — see the registerOscHandler callback below for why.
   */
  const finalizeBlock = useCallback(
    (blockId: string, exitCode: number, opts?: { clearOnParsed?: boolean }) => {
      const prev = blocksRef.current;
      const target = prev.find((b) => b.id === blockId);
      if (!target || target.status !== "running") return;

      const endTime = Date.now();
      const frozenOutput = target.rawOutput;
      const cols = term?.cols ?? 80;

      const finalized: TerminalBlock = {
        ...target,
        status: exitCode === 0 ? "completed" : "failed",
        exitCode,
        endTime,
      };
      const updated = prev.map((b) => (b.id === blockId ? finalized : b));
      blocksRef.current = updated;
      setBlocks(updated);

      parseAnsiToRenderedLines(frozenOutput, cols).then((renderedLines) => {
        const withLines = blocksRef.current.map((b) =>
          b.id === blockId ? { ...b, renderedLines } : b,
        );
        blocksRef.current = withLines;
        setBlocks(withLines);

        if (opts?.clearOnParsed && term) clearAndRebasePromptEnd(term);

        const cb = completionCallbacksRef.current.get(blockId);
        if (cb) {
          completionCallbacksRef.current.delete(blockId);
          const finalBlock = withLines.find((b) => b.id === blockId)!;
          setTimeout(() => cb(finalBlock), 50);
        }
      });
    },
    [term, clearAndRebasePromptEnd],
  );

  /**
   * Starts tracking a block for a command that was typed directly into the
   * live terminal (bypassing WarpInput's submitCommand — WarpInput isn't the
   * only way to type into a real terminal). Unlike submitCommand, this does
   * NOT write anything to the PTY: the caller (TerminalView's onData handler,
   * or the OSC 133 B/C recovery path below for remote-viewer-issued commands)
   * has already streamed the keystrokes to the PTY, so writing here again
   * would duplicate/corrupt input. This only does the block-bookkeeping half
   * of submitCommand.
   *
   * 搬到這裡（原本在 submitCommand 之後、檔案偏下方）是因為下面的 OSC 133
   * effect 需要直接呼叫它——effect 的 closure 抓的是變數本身，函式定義必須
   * 出現在它前面。
   */
  const beginTrackedBlock = useCallback(
    (cmd: string) => {
      if (!sessionId) return;

      onCommandStarted?.(cmd);

      if (isClearCommand(cmd)) {
        // Same reasoning as submitCommand's `clear`/`cls` handling — wipe the
        // whole block history instead of tracking a card for it. The keystrokes
        // (including the trailing Enter) are already streaming to the PTY via
        // onData, so there's nothing to write here.
        clearAllBlocks();
        return;
      }

      const prevBlocks = blocksRef.current;
      const prevLatest = prevBlocks[prevBlocks.length - 1];
      if (prevLatest?.status === "running") {
        // Already tracking a block — most likely this Enter press belongs to
        // a submitCommand-initiated command whose OSC 133 D hasn't fired yet.
        // Don't create a second, competing block.
        return;
      }

      const newBlock: TerminalBlock = {
        id: Math.random().toString(36).substring(2, 15) + Date.now().toString(36),
        command: cmd,
        status: "running",
        startTime: Date.now(),
        cwd: cwdRef?.current,
        rawOutput: "",
      };

      const updated = [...blocksRef.current, newBlock];
      blocksRef.current = updated;
      setBlocks(updated);
    },
    [sessionId, cwdRef, clearAllBlocks, onCommandStarted],
  );

  useEffect(() => {
    if (!term) return;

    const onBufferChange = () => {
      setIsAlternateBuffer(term.buffer.active.type === "alternate");
    };
    const disposeBuffer = term.buffer.onBufferChange(onBufferChange);
    // 註冊當下就同步一次，不要只等事件。
    //
    // 這個 effect 的 deps 含三個 callback（finalizeBlock / onLiveClear /
    // onCommandSettled），任一個識別性改變就會重跑：舊監聽 dispose、新監聽
    // 註冊——中間沒有人重讀「現在是不是 alternate」。於是若程式已經切進
    // alt-screen 之後才發生重跑，狀態會永遠停在 false，即時窗格縮回
    // MAX_LIVE_ROWS。實際症狀是 Claude Code 時而滿版、時而只有 16 列；
    // vim 較少遇到只是因為開啟時比較不會剛好觸發那幾個 callback 重建。
    //
    // 冪等：setState 同值不會觸發重繪。
    onBufferChange();

    const disposeOsc = term.parser.registerOscHandler(133, (data) => {
      if (data === "B") {
        // Prompt text has just finished being drawn (this marker is embedded
        // at the tail of the shell's PS1/prompt output itself — see
        // src-tauri/src/pty/shell.rs — so it's guaranteed to arrive AFTER the
        // visible prompt characters, unlike A which fires from a hook BEFORE
        // the prompt is drawn). Record exactly where input begins so
        // recoverUntrackedCommand can slice from here.
        promptEndRef.current = {
          row: term.buffer.active.cursorY + term.buffer.active.baseY,
          col: term.buffer.active.cursorX,
        };
        return true;
      } else if (data === "C") {
        // Command start — usually a no-op, since the block was already
        // created synchronously by submitCommand (WarpInput) or
        // beginTrackedBlock (typed directly into the live terminal), both of
        // which run well before this async shell-emitted event round-trips
        // back to the frontend. But if nothing is tracked as "running" at
        // this point, the command didn't come through either of those two
        // paths — it was written to the PTY some other way (e.g. a remote
        // viewer with control access). Try to recover the actual typed text
        // from the screen content between the last B marker and here; only
        // fall back to the lighter-weight boundary signal if that fails (see
        // recoverUntrackedCommand's doc comment for when/why it can fail).
        const prev = blocksRef.current;
        const latest = prev[prev.length - 1];
        if (!latest || latest.status !== "running") {
          const recovered = recoverUntrackedCommand(term, promptEndRef.current);
          if (recovered !== null) {
            beginTrackedBlock(recovered);
          } else {
            onUntrackedCommandBoundary?.("start");
          }
        }
        return true;
      } else if (data.startsWith("D")) {
        const parts = data.split(";");
        // parts.length > 1 也拿來判斷「這次的 D 到底有沒有帶 exit code」，
        // 給下面的 onCommandSettled 用——這裡沿用既有 parse 出來的 exitCode
        // 本身完全不動，只是額外記住它是不是真的被送出來的。
        const hasExitCode = parts.length > 1;
        const exitCode = hasExitCode ? parseInt(parts[1], 10) : 0;

        const prev = blocksRef.current;
        if (prev.length === 0) {
          onUntrackedCommandBoundary?.("end");
          return true;
        }
        const latest = prev[prev.length - 1];
        if (latest.status !== "running") {
          onUntrackedCommandBoundary?.("end");
          return true;
        }

        // Windows-only: clear on the next tick instead of waiting for the async
        // headless-parse in finalizeBlock (avoids a race where a long/scrolled
        // command's completion and the shell's next prompt can arrive batched
        // together — Windows/ConPTY specific, not seen on zsh/bash's separate
        // precmd writes). Root-caused via DevTools diagnostic logging: the
        // live pane going stale after long output was ConPTY re-transmitting
        // its currently-visible screen content in response to a PTY resize
        // that forceLiveRepaint's fit() call was spuriously triggering (see
        // that function for the full mechanism) — fixed there, not here, but
        // this deferred-clear timing is still needed on its own merits.
        const isWindows = hostPlatform === "windows";
        if (isWindows) {
          setTimeout(() => {
            clearAndRebasePromptEnd(term);
            term?.scrollToBottom();
            onLiveClear?.();
          }, 0);
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode);
        } else {
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode, { clearOnParsed: true });
        }

        // 兩條分支（Windows/ConPTY 與其他平台）的差別只在畫面清除時機，
        // 對「指令結束了、結果是什麼」沒有影響，所以放在合流之後呼叫一次。
        //
        // 但只有真的帶了 exit code 才通知側邊欄。cmd.exe 的 PROMPT（見
        // src-tauri/src/pty/shell.rs）只送出裸的 "D"，不像 PowerShell/zsh/bash
        // 那樣附上 exit code——這種情況下無法分辨指令是成功還是失敗，若照舊
        // 假設成 0 會把失敗的指令顯示成綠色的 "done"，是誤報。沒有 exit code
        // 就什麼提示點都不設，符合「寧可漏報，不可誤報」的原則。
        if (hasExitCode) {
          onCommandSettled?.(isNaN(exitCode) ? 0 : exitCode);
        }

        return true;
      }
      return false;
    });

    return () => {
      disposeBuffer.dispose();
      disposeOsc.dispose();
    };
    // `write` 故意不在這裡——它透過 writeRef 讀取，不需要讓這個 effect
    // 跟著它的身分重新註冊（本機分頁沒傳 write 時，每次 render 呼叫端拿到
    // 的都是函式簽名裡那個預設值運算式產生的全新參考，放進依賴陣列會讓
    // 這個 effect 每次 render 都 dispose+重新註冊）。`hostPlatform` 是字串，
    // 沒有這個問題，放心加進來。
  }, [term, finalizeBlock, beginTrackedBlock, clearAndRebasePromptEnd, onLiveClear, onCommandSettled, hostPlatform, onUntrackedCommandBoundary]);

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

      // If the previous block is still "running", its OSC 133 D never fired
      // (caller raced ahead, or the shell doesn't reliably emit exactly one D
      // per command). Defensively finalize it as failed/interrupted (exitCode
      // -1 is a sentinel, not a real process exit code) so it never lingers
      // forever and its onComplete callback — which the agent loop awaits —
      // is never silently dropped, leaking the completionCallbacksRef entry
      // and hanging the caller.
      const prevBlocks = blocksRef.current;
      if (prevBlocks.length > 0) {
        const prevLatest = prevBlocks[prevBlocks.length - 1];
        if (prevLatest.status === "running") {
          finalizeBlock(prevLatest.id, -1);
        }
      }

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

      // Clear the current line before sending the command (see isWindows/clearSeq above).
      writeRef.current(clearSeq + cmd + "\r");
    },
    [sessionId, term, cwdRef, finalizeBlock, clearAllBlocks, onCommandStarted, hostPlatform],
  );

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
