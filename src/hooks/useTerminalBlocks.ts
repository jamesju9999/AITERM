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
   * kicks off headless ANSI parsing, and fires+clears its onComplete callback
   * once parsing resolves. Shared by the normal OSC 133 D path and the
   * defensive "orphaned block" path in submitCommand below, so a block can
   * never be finalized more than once and its callback can never be left
   * dangling regardless of which path finalizes it.
   */
  const finalizeBlock = useCallback(
    (blockId: string, exitCode: number, opts?: { clearTerminalOnParsed?: boolean }) => {
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

        // Only clear the live terminal once the card is actually ready to take over
        // (renderedLines set above) — clearing any earlier risks a blank-screen flash
        // between "raw output disappears" and "card appears", especially for large
        // outputs where headless ANSI parsing takes measurably longer.
        if (opts?.clearTerminalOnParsed) term?.clear();

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

    const disposeOsc = term.parser.registerOscHandler(133, (data) => {
      if (data === "C") {
        // Command start — no-op. The block was already created synchronously,
        // either by submitCommand (WarpInput) or by beginTrackedBlock (typed
        // directly into the live terminal) — both run well before this async
        // shell-emitted event round-trips back to the frontend.
        return true;
      } else if (data.startsWith("D")) {
        const parts = data.split(";");
        const exitCode = parts.length > 1 ? parseInt(parts[1], 10) : 0;

        const prev = blocksRef.current;
        if (prev.length === 0) return true;
        const latest = prev[prev.length - 1];
        if (latest.status !== "running") return true;

        finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode, { clearTerminalOnParsed: true });

        return true;
      }
      return false;
    });

    return () => {
      disposeBuffer.dispose();
      disposeOsc.dispose();
    };
  }, [term, finalizeBlock]);

  const submitCommand = useCallback(
    (cmd: string, onComplete?: (block: TerminalBlock) => void) => {
      if (!term || !sessionId) return;

      // On Windows conpty: \x15 echoes as visible "^U", and \x1b gets merged with
      // the first char of the command as an Alt+key (e.g. \x1b + "d" = Alt+D which
      // deletes a word, dropping the "d").  WarpInput owns all keyboard input so the
      // PTY line is always empty — no clear sequence needed on Windows.
      // On macOS/Linux, \x15 (Ctrl+U) clears bash/zsh input silently.
      const isWindows = navigator.platform.toLowerCase().startsWith("win");
      const clearSeq = isWindows ? "" : "\x15";

      if (cmd.trim() === "clear") {
        // `clear` wipes the whole block history, not just the live viewport —
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
    [sessionId, term, cwdRef, finalizeBlock, clearAllBlocks],
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

      if (cmd.trim() === "clear") {
        // Same reasoning as submitCommand's `clear` handling — wipe the whole
        // block history instead of tracking a card for it. The keystrokes
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
    [sessionId, cwdRef, clearAllBlocks],
  );

  return {
    blocks,
    submitCommand,
    beginTrackedBlock,
    appendOutput,
    setBlockGitInfo,
    isAlternateBuffer,
    termInstance: term,
  };
}
