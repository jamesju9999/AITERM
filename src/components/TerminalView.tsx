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
import "./TerminalView.css";

export function TerminalView() {
  const hostRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<string>("initializing…");

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
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current);
    requestAnimationFrame(() => fit.fit());

    const decoder = new TextDecoder("utf-8");

    let sessionId: string | null = null;
    let unlistenData: (() => void) | null = null;

    (async () => {
      try {
        const { rows, cols } = term;
        sessionId = await createPty({ rows, cols });
        setStatus(`connected (${sessionId.slice(0, 8)}…)`);

        unlistenData = await onPtyData(sessionId, (bytes) => {
          term.write(decoder.decode(bytes, { stream: true }));
        });

        // Forward keystrokes to the PTY.
        term.onData((data) => {
          if (sessionId) writePty(sessionId, data).catch(console.error);
        });

        // Forward resizes.
        term.onResize(({ rows: r, cols: c }) => {
          if (sessionId) resizePty(sessionId, { rows: r, cols: c }).catch(console.error);
        });
      } catch (e) {
        setStatus(`error: ${String(e)}`);
      }
    })();

    // Handle window resize.
    const onWindowResize = () => fit.fit();
    window.addEventListener("resize", onWindowResize);

    return () => {
      window.removeEventListener("resize", onWindowResize);
      if (unlistenData) unlistenData();
      if (sessionId) {
        closePty(sessionId).catch(() => {
          // ignore — may already be gone
        });
      }
      term.dispose();
    };
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div className="aiterm-status">AITerm · {status}</div>
      <div
        ref={hostRef}
        className="aiterm-terminal-root"
        style={{ flex: 1, minHeight: 0 }}
      />
    </div>
  );
}
