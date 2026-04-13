import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
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
import {
  invokeAiQuery,
  formatAiError,
  type AiError,
  type AiStreamEvent,
  type RiskLevel,
} from "../ipc/ai";
import { getConfig, type ExecutionMode } from "../ipc/config";
import { listProviders } from "../ipc/provider";
import { parseAiPrefix } from "./parseAiPrefix";
import { CommandPreview } from "./CommandPreview";
import { StreamingIndicator } from "./StreamingIndicator";
import { ProviderPalette } from "./ProviderPalette";
import "./TerminalView.css";

interface PreviewState {
  loading: boolean;
  visible: boolean;
  command: string;
  explanation: string;
  riskLevel: RiskLevel;
}

const INITIAL_PREVIEW: PreviewState = {
  loading: false,
  visible: false,
  command: "",
  explanation: "",
  riskLevel: "safe",
};

/** Decide whether to auto-execute based on execution mode and risk level. */
function shouldAutoExecute(mode: ExecutionMode, risk: RiskLevel): boolean {
  if (mode === "always-confirm") return false;
  if (mode === "graded") return risk === "safe";
  if (mode === "full-auto") return risk === "safe" || risk === "needs_confirm";
  return false;
}

export function TerminalView() {
  const navigate = useNavigate();
  const hostRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<string>("initializing…");
  const [preview, setPreview] = useState<PreviewState>(INITIAL_PREVIEW);
  const previewRef = useRef<PreviewState>(INITIAL_PREVIEW);
  previewRef.current = preview;

  // Streaming state
  const [streamText, setStreamText] = useState("");
  const streamingRef = useRef(false);

  // Provider status badge
  const [activeProvider, setActiveProvider] = useState<string>("");
  const [paletteOpen, setPaletteOpen] = useState(false);

  // Execution mode is read once and cached; re-fetched when we return from settings.
  const executionModeRef = useRef<ExecutionMode>("always-confirm");

  // Refs bridged into the useEffect closure.
  const termRef = useRef<Terminal | null>(null);
  const sessionRef = useRef<string | null>(null);
  const lineBufRef = useRef<string>("");

  /** Fetch active provider name and execution mode from config. */
  function refreshConfig() {
    listProviders()
      .then((list) => {
        const active = list.find((p) => p.is_default) ?? list[0];
        setActiveProvider(active?.display_name ?? "");
      })
      .catch(() => {});

    getConfig()
      .then((cfg) => {
        executionModeRef.current = cfg.execution_mode;
      })
      .catch(() => {});
  }

  useEffect(() => {
    refreshConfig();
  }, []);

  // Re-fetch config when returning from settings.
  useEffect(() => {
    const handler = () => {
      if (document.visibilityState === "visible") refreshConfig();
    };
    document.addEventListener("visibilitychange", handler);
    return () => document.removeEventListener("visibilitychange", handler);
  }, []);

  // Keyboard shortcuts: Ctrl+, → settings, Ctrl+Shift+P → palette
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === ",") {
        e.preventDefault();
        navigate("/settings");
      } else if (e.ctrlKey && e.shiftKey && e.key === "P") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [navigate]);

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
    let unlistenStream: (() => void) | null = null;

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

        // Listen for AI streaming events from this session.
        unlistenStream = await listen<AiStreamEvent>("ai-stream", (event) => {
          if (event.payload.kind !== "query") return;
          if (event.payload.session_id !== sessionId) return;
          if (!event.payload.done) {
            setStreamText((t) => t + event.payload.delta);
          }
        });

        term.onData((data) => {
          const session = sessionRef.current;
          if (!session) return;

          for (const ch of data) {
            if (ch === "\r" || ch === "\n") {
              const line = lineBufRef.current;
              lineBufRef.current = "";
              const query = parseAiPrefix(line);
              if (query !== null) {
                if (previewRef.current.loading) {
                  writeRed("aiterm: already waiting for AI response");
                  continue;
                }
                handleAiQuery(
                  session,
                  line,
                  query,
                  term,
                  setPreview,
                  setStreamText,
                  streamingRef,
                  executionModeRef,
                  writeRed,
                );
                continue;
              }
              writePty(session, ch).catch(console.error);
            } else if (ch === "\x7f" || ch === "\b") {
              lineBufRef.current = lineBufRef.current.slice(0, -1);
              writePty(session, ch).catch(console.error);
            } else if (ch === "\x03") {
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
      if (unlistenStream) unlistenStream();
      const id = sessionRef.current;
      if (id) {
        closePty(id).catch(() => {});
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
      <div className="aiterm-status">
        <span className="aiterm-status-left">AITerm · {status}</span>
        {activeProvider && (
          <button
            className="aiterm-status-provider"
            title="切換 Provider (Ctrl+Shift+P)"
            onClick={() => setPaletteOpen((o) => !o)}
          >
            {activeProvider}
          </button>
        )}
        <button
          className="aiterm-settings-btn"
          title="設定 (Ctrl+,)"
          onClick={() => navigate("/settings")}
        >
          ⚙
        </button>
      </div>
      <div
        ref={hostRef}
        className="aiterm-terminal-root"
        style={{ flex: 1, minHeight: 0 }}
      />
      {preview.loading && (
        <StreamingIndicator visible text={streamText} />
      )}
      {preview.visible && (
        <CommandPreview
          command={preview.command}
          explanation={preview.explanation}
          riskLevel={preview.riskLevel}
          onConfirm={handleConfirm}
          onCancel={handleCancel}
        />
      )}
      {paletteOpen && (
        <ProviderPalette
          onClose={() => setPaletteOpen(false)}
          onSwitch={(name) => setActiveProvider(name)}
        />
      )}
    </div>
  );
}

/**
 * Kick off an /ai request: erase the typed line, show the streaming indicator,
 * invoke the backend, then either auto-execute or show the preview based on
 * the execution mode and risk level.
 */
function handleAiQuery(
  sessionId: string,
  originalLine: string,
  query: string,
  term: Terminal,
  setPreview: (p: PreviewState) => void,
  setStreamText: React.Dispatch<React.SetStateAction<string>>,
  streamingRef: React.MutableRefObject<boolean>,
  executionModeRef: React.MutableRefObject<ExecutionMode>,
  writeRed: (msg: string) => void,
) {
  void originalLine;
  term.write("\r\x1b[2K");
  term.write("→ asking AI...\r\n");
  setStreamText("");
  streamingRef.current = true;
  setPreview({ loading: true, visible: false, command: "", explanation: "", riskLevel: "safe" });

  invokeAiQuery(query, sessionId)
    .then((resp) => {
      streamingRef.current = false;
      term.write("\x1b[1A\x1b[2K");

      const mode = executionModeRef.current;
      const risk = resp.risk_level;

      if (shouldAutoExecute(mode, risk)) {
        // Auto-execute: write a subtle confirmation line then send to PTY.
        const riskColor = risk === "safe" ? "\x1b[32m" : "\x1b[33m";
        term.write(`${riskColor}▶ ${resp.command}\x1b[0m\r\n`);
        writePty(sessionId, resp.command + "\r").catch(console.error);
        setPreview(INITIAL_PREVIEW);
      } else {
        // Show preview with risk badge.
        if (risk === "dangerous") {
          term.write("\x1b[31m⚠ 危險操作 — 請仔細確認後再執行\x1b[0m\r\n");
        }
        setPreview({
          loading: false,
          visible: true,
          command: resp.command,
          explanation: resp.explanation,
          riskLevel: risk,
        });
      }
    })
    .catch((rawErr: unknown) => {
      streamingRef.current = false;
      setStreamText("");
      const err = normalizeAiError(rawErr);
      writeRed(formatAiError(err));

      // Actionable follow-up hints
      if (err.kind === "not_configured") {
        term.write("\x1b[33m提示：按 Ctrl+, 開啟設定並新增一個 AI Provider。\x1b[0m\r\n");
      } else if (
        err.kind === "network" &&
        (err.message?.toLowerCase().includes("ollama") ||
          err.message?.toLowerCase().includes("connection refused"))
      ) {
        term.write(
          "\x1b[33m提示：請啟動 Ollama，或按 Ctrl+, 切換到雲端 Provider。\x1b[0m\r\n"
        );
      } else if (err.kind === "auth_failed") {
        term.write("\x1b[33m提示：請按 Ctrl+, 至設定頁更新 API Key。\x1b[0m\r\n");
      }

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
