import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

import {
  closePty,
  createPty,
  onPtyData,
  resizePty,
  writePty,
} from "../ipc/pty";
import { invokeAiQuery, formatAiError, type AiError } from "../ipc/ai";
import { parseAiPrefix } from "./parseAiPrefix";
import { CommandPreview } from "./CommandPreview";
import "./TerminalView.css";

interface PreviewState {
  loading: boolean;
  visible: boolean;
  command: string;
  explanation: string;
}

const INITIAL_PREVIEW: PreviewState = {
  loading: false,
  visible: false,
  command: "",
  explanation: "",
};

export function TerminalView() {
  const hostRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<string>("initializing…");
  const [preview, setPreview] = useState<PreviewState>(INITIAL_PREVIEW);
  const previewRef = useRef<PreviewState>(INITIAL_PREVIEW);
  previewRef.current = preview;

  // Refs bridged into the useEffect closure.
  const termRef = useRef<Terminal | null>(null);
  const sessionRef = useRef<string | null>(null);
  const lineBufRef = useRef<string>("");

  useEffect(() => {
    if (!hostRef.current) return;

    const term = new Terminal({
      fontFamily: '"Cascadia Mono", Consolas, monospace',
      fontSize: 14,
      cursorBlink: true,
      theme: {
        background: "#0c0c0c",
        foreground: "#e6e6e6",
      },
      convertEol: false,
    });
    termRef.current = term;

    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current);
    requestAnimationFrame(() => fit.fit());

    const decoder = new TextDecoder("utf-8");

    let unlistenData: (() => void) | null = null;

    const writeRed = (msg: string) => {
      term.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`);
    };

    (async () => {
      try {
        const { rows, cols } = term;
        const sessionId = await createPty({ rows, cols });
        sessionRef.current = sessionId;
        setStatus(`connected (${sessionId.slice(0, 8)}…)`);

        unlistenData = await onPtyData(sessionId, (bytes) => {
          term.write(decoder.decode(bytes, { stream: true }));
        });

        term.onData((data) => {
          const session = sessionRef.current;
          if (!session) return;

          // Track a local line buffer so we can recognize `/ai ` at Enter time.
          for (const ch of data) {
            if (ch === "\r" || ch === "\n") {
              // User hit Enter. Check for /ai prefix.
              const line = lineBufRef.current;
              lineBufRef.current = "";
              const query = parseAiPrefix(line);
              if (query !== null) {
                // While loading, reject further /ai submissions.
                if (previewRef.current.loading) {
                  writeRed("aiterm: already waiting for AI response");
                  // Echo CR so the shell stays on a fresh line.
                  continue;
                }
                handleAiQuery(session, line, query, term, setPreview, writeRed);
                continue; // do NOT forward to PTY
              }
              // Non-/ai line: forward the Enter normally.
              writePty(session, ch).catch(console.error);
            } else if (ch === "\x7f" || ch === "\b") {
              // Backspace — keep the buffer in sync.
              lineBufRef.current = lineBufRef.current.slice(0, -1);
              writePty(session, ch).catch(console.error);
            } else if (ch === "\x03") {
              // Ctrl+C clears the line buffer.
              lineBufRef.current = "";
              writePty(session, ch).catch(console.error);
            } else {
              lineBufRef.current += ch;
              writePty(session, ch).catch(console.error);
            }
          }
        });

        term.onResize(({ rows: r, cols: c }) => {
          if (sessionRef.current) {
            resizePty(sessionRef.current, { rows: r, cols: c }).catch(console.error);
          }
        });
      } catch (e) {
        setStatus(`error: ${String(e)}`);
      }
    })();

    const onWindowResize = () => fit.fit();
    window.addEventListener("resize", onWindowResize);

    return () => {
      window.removeEventListener("resize", onWindowResize);
      if (unlistenData) unlistenData();
      const id = sessionRef.current;
      if (id) {
        closePty(id).catch(() => {
          // ignore — may already be gone
        });
      }
      term.dispose();
      termRef.current = null;
    };
  }, []);

  const handleConfirm = () => {
    const session = sessionRef.current;
    if (session && preview.command) {
      writePty(session, preview.command + "\r").catch(console.error);
    }
    setPreview(INITIAL_PREVIEW);
  };
  const handleCancel = () => setPreview(INITIAL_PREVIEW);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        position: "relative",
      }}
    >
      <div className="aiterm-status">AITerm · {status}</div>
      <div
        ref={hostRef}
        className="aiterm-terminal-root"
        style={{ flex: 1, minHeight: 0 }}
      />
      {preview.visible && (
        <CommandPreview
          command={preview.command}
          explanation={preview.explanation}
          onConfirm={handleConfirm}
          onCancel={handleCancel}
        />
      )}
    </div>
  );
}

/**
 * Kick off an /ai request: erase the typed line, show the "asking" indicator,
 * invoke the backend, and update preview state based on the result.
 */
function handleAiQuery(
  sessionId: string,
  originalLine: string,
  query: string,
  term: Terminal,
  setPreview: (p: PreviewState) => void,
  writeRed: (msg: string) => void,
) {
  // Erase the typed `/ai ...` line visually (CR + clear-line). We only touch
  // xterm's buffer here — the shell has not seen the bytes because we did
  // not forward them to PTY.
  void originalLine;
  term.write("\r\x1b[2K");
  term.write("→ asking AI...\r\n");
  setPreview({ loading: true, visible: false, command: "", explanation: "" });

  invokeAiQuery(query, sessionId)
    .then((resp) => {
      // Clear the "asking" line then show the preview.
      term.write("\x1b[1A\x1b[2K"); // move up one, clear line
      setPreview({
        loading: false,
        visible: true,
        command: resp.command,
        explanation: resp.explanation,
      });
    })
    .catch((rawErr: unknown) => {
      const err = normalizeAiError(rawErr);
      writeRed(formatAiError(err));
      setPreview(INITIAL_PREVIEW);
    });
}

/**
 * Tauri may deliver `AiError` either as the serialized object directly or
 * wrapped in an `Error` whose message is the JSON. Coerce both forms.
 */
function normalizeAiError(err: unknown): AiError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as AiError;
  }
  if (err instanceof Error) {
    try {
      const parsed = JSON.parse(err.message);
      if (parsed && typeof parsed === "object" && "kind" in parsed) {
        return parsed as AiError;
      }
    } catch {
      // fall through
    }
    return { kind: "network", message: err.message };
  }
  return { kind: "network", message: String(err) };
}
