import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { writePty } from "../ipc/pty";
import { parseAiPrefix } from "../components/parseAiPrefix";

import type { IMarker } from "@xterm/xterm";

export interface TerminalBlock {
  id: string;
  command: string;      // The actual command text
  output: string;       // Captured ANSI output for this block
  status: "running" | "completed" | "failed";
  exitCode?: number;
  exitCode?: number;
  startMarker?: IMarker;
  endMarker?: IMarker;
  startLine?: number;
  endLine?: number;
  decorationCreated?: boolean;
}

export interface UseTerminalBlocksResult {
  blocks: TerminalBlock[];
  submitCommand: (cmd: string) => void;
  // Expose active buffer type so we can hide React Input if alternating
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
  const updateBlocks = (newBlocks: TerminalBlock[]) => {
    blocksRef.current = newBlocks;
    setBlocks(newBlocks);
  };

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
                updateBlocks(prev.map((b) => (b.id === latest.id ? { ...b, startMarker: marker } : b)));
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

        const startMarkerToDecorate = latest.startMarker;
        const targetBlockId = latest.id;

        updateBlocks(prev.map((b) => (b.id === latest.id ? { 
          ...b, 
          status: exitCode === 0 ? "completed" : "failed", 
          exitCode: isNaN(exitCode) ? 0 : exitCode,
          endMarker: endMarker || undefined,
          startLine: startMarkerToDecorate?.line,
          endLine: endMarker?.line,
          decorationCreated: true
        } : b)));

        // Instead of Xterm decorations, we just rely on the React overlay
        // so we don't do anything here except updating the state!
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
   * Here we inject a special sequence after the user command. 
   * Since we are building Method B, the command string from React 
   * is guaranteed to be whole. We can wrap it to return exit code.
   * Format: <cmd> \n echo -e "\x1b]133;D;$?\x07"
   * Since Windows pwsh uses a different format, we might need a generic approach.
   * For now, we will track the command natively.
   */
  const submitCommand = useCallback(
    (cmd: string) => {
      if (!term || !sessionId) return;

      const newBlock: TerminalBlock = {
        id: Math.random().toString(36).substring(2, 15) + Date.now().toString(36),
        command: cmd,
        output: "", // initially empty
        status: "running",
      };
      
      updateBlocks([...blocksRef.current, newBlock]);

      // For MVP of M5, we just write the command to PTY.
      // E.g. add a marker manually via simple bash echo for completed detection.
      // To strictly avoid ZLE (zsh line editor) messing with our appended marker:
      // We send \x15 (Ctrl+U to clear line) then the command + \r
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
