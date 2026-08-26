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
}

// `clear` is the Unix/PowerShell screen-clear command; `cls` is cmd.exe's (and
// also a built-in PowerShell alias for Clear-Host, alongside `clear`). Matched
// case-insensitively since Windows commands aren't case-sensitive.
function isClearCommand(cmd: string): boolean {
  const trimmed = cmd.trim().toLowerCase();
  return trimmed === "clear" || trimmed === "cls";
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

        if (opts?.clearOnParsed) term?.clear();

        const cb = completionCallbacksRef.current.get(blockId);
        if (cb) {
          completionCallbacksRef.current.delete(blockId);
          const finalBlock = withLines.find((b) => b.id === blockId)!;
          setTimeout(() => cb(finalBlock), 50);
        }
      });
    },
    [term],
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
      if (data === "C") {
        // Command start — no-op. The block was already created synchronously,
        // either by submitCommand (WarpInput) or by beginTrackedBlock (typed
        // directly into the live terminal) — both run well before this async
        // shell-emitted event round-trips back to the frontend.
        return true;
      } else if (data.startsWith("D")) {
        const parts = data.split(";");
        // parts.length > 1 也拿來判斷「這次的 D 到底有沒有帶 exit code」，
        // 給下面的 onCommandSettled 用——這裡沿用既有 parse 出來的 exitCode
        // 本身完全不動，只是額外記住它是不是真的被送出來的。
        const hasExitCode = parts.length > 1;
        const exitCode = hasExitCode ? parseInt(parts[1], 10) : 0;

        const prev = blocksRef.current;
        if (prev.length === 0) return true;
        const latest = prev[prev.length - 1];
        if (latest.status !== "running") return true;

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
    // sessionId is in the deps because the Windows Ctrl+L resync in the D
    // handler writes to it — the effect must re-register once the PTY session
    // id lands (it's set async after `term`) so the handler's closure isn't
    // holding the initial empty id.
  }, [term, finalizeBlock, onLiveClear, sessionId, onCommandSettled]);

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
      writePty(sessionId, clearSeq + cmd + "\r").catch(console.error);
    },
    [sessionId, term, cwdRef, finalizeBlock, clearAllBlocks, onCommandStarted],
  );

  /**
   * Starts tracking a block for a command that was typed directly into the
   * live terminal (bypassing WarpInput's submitCommand — WarpInput isn't the
   * only way to type into a real terminal). Unlike submitCommand, this does
   * NOT write anything to the PTY: the caller (TerminalView's onData handler)
   * has already streamed the keystrokes to the PTY character-by-character as
   * the user typed, so writing here again would duplicate/corrupt input.
   * This only does the block-bookkeeping half of submitCommand.
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
