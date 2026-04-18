import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { writePty } from "../ipc/pty";

import type { IMarker } from "@xterm/xterm";

export interface TerminalBlock {
  id: string;
  command: string;      // The actual command text
  output: string;       // Captured ANSI output for this block
  status: "running" | "completed" | "failed";
  exitCode?: number;
  startMarker?: IMarker;
  endMarker?: IMarker;
  startLine?: number;
  endLine?: number;
  decorationCreated?: boolean;
}

export interface UseTerminalBlocksResult {
  blocks: TerminalBlock[];
  submitCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  isAlternateBuffer: boolean;
  termInstance: Terminal | null;
}

export function useTerminalBlocks(
  sessionId: string,
  term: Terminal | null,
): UseTerminalBlocksResult {
  const [blocks, setBlocks] = useState<TerminalBlock[]>([]);
  const [isAlternateBuffer, setIsAlternateBuffer] = useState(false);

  // We keep a mutable ref to blocks so the data listener can modify the last block's text.
  const blocksRef = useRef<TerminalBlock[]>([]);
  // Map of block ID → onComplete callback (for agent loop)
  const completionCallbacksRef = useRef<Map<string, (block: TerminalBlock) => void>>(new Map());

  useEffect(() => {
    if (!term) return;

    const onBufferChange = () => {
      setIsAlternateBuffer(term.buffer.active.type === "alternate");
    };
    const disposeBuffer = term.buffer.onBufferChange(onBufferChange);

    // Register OSC 133 handler
    const disposeOsc = term.parser.registerOscHandler(133, (data) => {
      console.log("[AITerm] Received OSC 133:", data);

      if (data === "C") {
        try {
          const marker = term.registerMarker(0);
          if (marker) {
            const prev = blocksRef.current;
            if (prev.length > 0) {
              const latest = prev[prev.length - 1];
              if (latest.status === "running") {
                const updated = prev.map((b) => (b.id === latest.id ? { ...b, startMarker: marker } : b));
                blocksRef.current = updated;
                setBlocks(updated);
              }
            }
          }
        } catch(e) { console.error("Error creating start marker", e); }
        return true;
      } else if (data.startsWith("D")) {
        // Command finished
        const parts = data.split(";");
        const exitCode = parts.length > 1 ? parseInt(parts[1], 10) : 0;
        const endMarker = term.registerMarker(0);

        const prev = blocksRef.current;
        if (prev.length === 0) return true;
        const latest = prev[prev.length - 1];
        if (latest.status !== "running") return true;

        const completedBlock: TerminalBlock = {
          ...latest,
          status: exitCode === 0 ? "completed" : "failed",
          exitCode: isNaN(exitCode) ? 0 : exitCode,
          endMarker: endMarker || undefined,
          startLine: latest.startMarker?.line,
          endLine: endMarker?.line,
          decorationCreated: true,
        };

        const updated = prev.map((b) => (b.id === latest.id ? completedBlock : b));
        blocksRef.current = updated;
        setBlocks(updated);

        // Fire the completion callback if registered for this block
        const cb = completionCallbacksRef.current.get(latest.id);
        if (cb) {
          completionCallbacksRef.current.delete(latest.id);
          setTimeout(() => cb(completedBlock), 50);
        }

        return true;
      }
      return false;
    });

    return () => {
      disposeBuffer.dispose();
      disposeOsc.dispose();
    };
  }, [term]);

  /**
   * Submit a command to the PTY and track it as a block.
   * Optionally provide an onComplete callback that fires when the block finishes.
   */
  const submitCommand = useCallback(
    (cmd: string, onComplete?: (block: TerminalBlock) => void) => {
      if (!term || !sessionId) return;

      const newBlock: TerminalBlock = {
        id: Math.random().toString(36).substring(2, 15) + Date.now().toString(36),
        command: cmd,
        output: "",
        status: "running",
      };
      
      // Register the completion callback before creating the block
      if (onComplete) {
        completionCallbacksRef.current.set(newBlock.id, onComplete);
      }

      const updated = [...blocksRef.current, newBlock];
      blocksRef.current = updated;
      setBlocks(updated);

      // Send \\x15 (Ctrl+U to clear line) then the command + \\r
      writePty(sessionId, "\x15" + cmd + "\r").catch(console.error);
    },
    [sessionId, term],
  );

  return {
    blocks,
    submitCommand,
    isAlternateBuffer,
    termInstance: term,
  };
}
