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
import { getConfig, type ExecutionMode, type SubmitShortcut } from "../ipc/config";
import { listProviders } from "../ipc/provider";
import { parseAiPrefix } from "./parseAiPrefix";
import { CommandPreview } from "./CommandPreview";
import { StreamingIndicator } from "./StreamingIndicator";
import { AiPanel } from "./AiPanel";
import { ProviderPalette } from "./ProviderPalette";
import { WarpInput } from "./WarpInput";
import { useTerminalBlocks } from "../hooks/useTerminalBlocks";
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

  const [panelOpen, setPanelOpen] = useState(false);
  const [sessionId, setSessionId] = useState<string>("");

  const panelOpenRef = useRef(false);
  useEffect(() => {
    panelOpenRef.current = panelOpen;
  }, [panelOpen]);

  // Streaming state
  const [streamText, setStreamText] = useState("");
  const streamingRef = useRef(false);

  // Provider status badge
  const [activeProvider, setActiveProvider] = useState<string>("");
  const [paletteOpen, setPaletteOpen] = useState(false);

  // Execution mode and shortcut are read once and cached; re-fetched when we return from settings.
  const executionModeRef = useRef<ExecutionMode>("always-confirm");
  const [submitShortcut, setSubmitShortcutState] = useState<SubmitShortcut>("enter");

  // Refs bridged into the useEffect closure.
  const termRef = useRef<Terminal | null>(null);
  const [termState, setTermState] = useState<Terminal | null>(null);
  const sessionRef = useRef<string | null>(null);
  const lineBufRef = useRef<string>("");
  const overlayRef = useRef<HTMLDivElement>(null);
  const [renderTick, setRenderTick] = useState(0);

  const { blocks, isAlternateBuffer, submitCommand } = useTerminalBlocks(
    sessionId,
    termState
  );

  useEffect(() => {
    if (termState) {
      termState.options.disableStdin = !isAlternateBuffer;
    }
  }, [isAlternateBuffer, termState]);

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
        setSubmitShortcutState(cfg.submit_shortcut);
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
      } else if (e.ctrlKey && (e.key === "i" || e.key === "I")) {
        e.preventDefault();
        setPanelOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [navigate]);

  // Listen for "Ask AI" clicks from block action buttons
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail as { command: string; exitCode: number };
      if (detail) {
        setPanelOpen(true);
        // Dispatch another event that AiPanel can pick up to pre-fill context
        setTimeout(() => {
          window.dispatchEvent(new CustomEvent("aiterm:prefill-chat", {
            detail: { text: `指令 \`${detail.command}\` 執行失敗 (exit code ${detail.exitCode})。請分析可能原因並提供修復建議。` }
          }));
        }, 100);
      }
    };
    window.addEventListener("aiterm:ask-ai", handler);
    return () => window.removeEventListener("aiterm:ask-ai", handler);
  }, []);

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
    setTermState(term);

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
        const id = await createPty({ rows, cols });
        sessionRef.current = id;
        setSessionId(id);
        setStatus(`connected (${id.slice(0, 8)}…)`);

        unlistenData = await onPtyData(id, (bytes) => {
          term.write(decoder.decode(bytes, { stream: true }));
        });

        // Set up native pixel-perfect scroll sync
        if (term.element) {
          const viewportEl = term.element.querySelector('.xterm-viewport');
          if (viewportEl && overlayRef.current) {
            viewportEl.addEventListener('scroll', () => {
              if (overlayRef.current) {
                overlayRef.current.style.transform = `translateY(-${viewportEl.scrollTop}px)`;
              }
            });
          }
        }
      } catch (err) {
        console.error("Failed to create pty", err);
      }
    })();

    unlistenStream = listen<AiStreamEvent>("ai-stream", (event) => {
      if (event.payload.kind !== "query") return;
      if (event.payload.session_id !== sessionRef.current) return;
      if (!event.payload.done) {
        setStreamText((t) => t + event.payload.delta);
      }
    });

    term.onData((data) => {
      // Panel owns keyboard while open — drop input.
      if (panelOpenRef.current) return;

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
              line, // originalLine
              query, // The parsed query string
              term,
              setPreview,
              setStreamText,
              streamingRef,
              executionModeRef,
              writeRed,
              submitCommand
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

    const onWindowResize = () => {
       fit.fit();
       setRenderTick(t => t + 1); // Force re-render of blocks to adapt to new cellHeight
    };
    window.addEventListener("resize", onWindowResize);

    return () => {
      window.removeEventListener("resize", onWindowResize);
      if (unlistenData) unlistenData();
      if (unlistenStream) unlistenStream.then(f => f());
      const id = sessionRef.current;
      if (id) {
        closePty(id).catch(() => {});
      }
      term.dispose();
      termRef.current = null;
    };
  }, []);

  const handleConfirm = () => {
    if (preview.command) {
      submitCommand(preview.command);
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

      <div style={{ position: "relative", flex: 1, minHeight: 0, width: "100%" }}>
        <div
          ref={hostRef}
          className="aiterm-terminal-root"
        />
        
        {/* React DOM overlay for visual blocks */}
        <div style={{ position: "absolute", top: 0, left: 0, right: 0, bottom: 0, pointerEvents: "none", overflow: "hidden", zIndex: 10 }}>
           <div ref={overlayRef} style={{ position: "absolute", top: 0, left: 0, right: 0, willChange: 'transform' }}>
             {blocks.map(b => {
               // We only render completed or failed blocks that have start and end lines
               if (b.status === "running" || b.startLine === undefined || b.endLine === undefined || !termState) return null;
               
               // Extract cell height from internal Xterm object, default to 14px 
               const cellHeight = (termState as any)._core?._renderService?.dimensions?.css?.cell?.height || 14;
               
               // Calculate absolute block pixel positions relative to the entire buffer
               const yLine = b.startLine;
               const heightLines = Math.max(1, b.endLine - b.startLine);
               
               const top = yLine * cellHeight;
               const heightPx = heightLines * cellHeight;
               
               return (
                 <div
                   key={b.id}
                   className="aiterm-block-decoration"
                   style={{
                      position: "absolute",
                      top: `${top}px`,
                      left: 0,
                      right: 0,
                      height: `${heightPx}px`,
                      pointerEvents: "none",
                      borderLeft: `2px solid ${b.exitCode === 0 ? '#34d399' : '#f87171'}`,
                      background: b.exitCode === 0 ? 'rgba(255, 255, 255, 0.02)' : 'rgba(255, 60, 60, 0.04)',
                      boxSizing: 'border-box',
                      zIndex: 20
                   }}
                 >
                   <div className="aiterm-block-actions" style={{ pointerEvents: "auto", position: "absolute", right: "8px", top: "2px", display: "flex", gap: "6px" }}>
                   {b.exitCode !== 0 && (
                     <button
                       className="aiterm-block-btn aiterm-block-btn-ai"
                       onClick={(e) => {
                         e.stopPropagation();
                         window.dispatchEvent(new CustomEvent('aiterm:ask-ai', { 
                            detail: { command: b.command, exitCode: b.exitCode } 
                         }));
                       }}
                     >
                       ✨ Ask AI
                     </button>
                   )}
                   <button
                       className="aiterm-block-btn"
                       onClick={(e) => {
                         e.stopPropagation();
                         navigator.clipboard.writeText(b.command).catch(console.error);
                         const btn = e.currentTarget;
                         btn.innerHTML = '✅ Copied';
                         setTimeout(() => btn.innerHTML = '📋 Copy', 2000);
                       }}
                     >
                       📋 Copy
                   </button>
                 </div>
               </div>
               );
             })}
           </div>
        </div>
      </div>
      {!isAlternateBuffer && (
        <WarpInput
          onSubmit={(cmd) => {
            const query = parseAiPrefix(cmd);
            if (query !== null) {
              if (previewRef.current.loading) {
                termRef.current?.write("\r\n\x1b[31maiterm: already waiting for AI response\x1b[0m\r\n");
                return;
              }
              if (termRef.current) {
                handleAiQuery(
                  sessionId,
                  cmd,
                  query,
                  termRef.current,
                  setPreview,
                  setStreamText,
                  streamingRef,
                  executionModeRef,
                  (msg) => termRef.current?.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`),
                  submitCommand
                );
              }
              return;
            }
            submitCommand(cmd);
          }}
          shortcut={submitShortcut}
        />
      )}
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
      {sessionId && (
        <AiPanel
          key={sessionId}
          sessionId={sessionId}
          isOpen={panelOpen}
          providerName={activeProvider}
          onClose={() => setPanelOpen(false)}
          onExecuteCommand={submitCommand}
          onOpenProviderPalette={() => {
            setPanelOpen(false);
            setPaletteOpen(true);
          }}
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
  submitCommand: (cmd: string) => void
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
        // Auto-execute: write a subtle confirmation line then submit to block manager.
        const riskColor = risk === "safe" ? "\x1b[32m" : "\x1b[33m";
        term.write(`${riskColor}▶ ${resp.command}\x1b[0m\r\n`);
        submitCommand(resp.command);
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
