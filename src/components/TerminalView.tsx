import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useLocale } from "../contexts/LocaleContext";
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
import { useTerminalBlocks } from "../hooks/useTerminalBlocks";
import { useAgentMission } from "../hooks/useAgentMission";
import { listProviders } from "../ipc/provider";
import { parseAiPrefix, parseAgentPrefix } from "./parseAiPrefix";
import { CommandPreview } from "./CommandPreview";
import { StreamingIndicator } from "./StreamingIndicator";
import { AiPanel } from "./AiPanel";
import { ProviderPalette } from "./ProviderPalette";
import { WarpInput } from "./WarpInput";
import { FileExplorer } from "./FileExplorer/FileExplorer";
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
function shouldAutoExecute(mode: ExecutionMode, risk: RiskLevel, agentActive = false): boolean {
  // When the agent loop is active, be more aggressive to keep the loop autonomous
  if (agentActive) {
    if (risk === "safe") return true;                                    // Always auto-exec safe in agent mode
    if (risk === "needs_confirm" && mode === "full-auto") return true;   // Full-auto agent: also auto-exec needs_confirm
    if (risk === "dangerous") return false;                              // Dangerous always requires manual confirmation
  }
  if (mode === "always-confirm") return false;
  if (mode === "graded") return risk === "safe";
  if (mode === "full-auto") return risk === "safe" || risk === "needs_confirm";
  return false;
}

export interface TerminalViewProps {
  isActive?: boolean;
  onToggleSidebar?: () => void;
  isSidebarOpen?: boolean;
}

export function TerminalView({ isActive = true, onToggleSidebar, isSidebarOpen = true }: TerminalViewProps) {
  type ViewTab = "terminal" | "files";
  const [viewTab, setViewTab] = useState<ViewTab>("terminal");
  const navigate = useNavigate();
  const { t } = useLocale();
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
  const executionModeRef = useRef<ExecutionMode>("always-confirm");
  
  const { agentMission, startMission, stopMission } = useAgentMission();

  // Provider status badge
  const [activeProvider, setActiveProvider] = useState<string>("");
  const [paletteOpen, setPaletteOpen] = useState(false);

  // Execution mode and shortcut are read once and cached; re-fetched when we return from settings.
  const [submitShortcut, setSubmitShortcutState] = useState<SubmitShortcut>("enter");
  const maxAgentStepsRef = useRef<number>(5);

  // Refs bridged into the useEffect closure.
  const termRef = useRef<Terminal | null>(null);
  const [termState, setTermState] = useState<Terminal | null>(null);
  const sessionRef = useRef<string | null>(null);
  const lineBufRef = useRef<string>("");
  const overlayRef = useRef<HTMLDivElement>(null);
  const [, setRenderTick] = useState(0);

  const { blocks, isAlternateBuffer, submitCommand } = useTerminalBlocks(
    sessionId,
    termState
  );

  // Bridge submitCommand into a ref so the stale term.onData closure can access the latest version
  const submitCommandRef = useRef(submitCommand);
  useEffect(() => { submitCommandRef.current = submitCommand; }, [submitCommand]);

  // Abort signal for agent loop — set to true to stop the loop
  const agentAbortRef = useRef(false);
  const agentMissionRef = useRef(agentMission);
  useEffect(() => { agentMissionRef.current = agentMission; }, [agentMission]);

  // Bridge blocks into a ref so the stale closure can check if a command is running
  const blocksRef = useRef(blocks);
  useEffect(() => { blocksRef.current = blocks; }, [blocks]);

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
        // 0 means unlimited — use a very large number internally
        maxAgentStepsRef.current = cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5);
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
    if (!isActive) return;
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
  }, [navigate, isActive]);

  // Listen for "Ask AI" clicks from block action buttons
  useEffect(() => {
    if (!isActive) return;
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail as { command?: string; exitCode?: number };
      setPanelOpen(true);
      
      if (detail && detail.command) {
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
  }, [isActive]);

  // (Agent loop is now callback-driven via runAgentLoop — no useEffect needed)

  useEffect(() => {
    if (!hostRef.current) return;

    const term = new Terminal({
      fontFamily: '"Cascadia Mono", Consolas, monospace',
      fontSize: 14,
      lineHeight: 1.4,
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
    let unlistenStream: Promise<() => void> | null = null;

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
          const text = decoder.decode(bytes, { stream: true });
          term.write(text);

          // Detect password prompts during agent mode
          if (agentMissionRef.current?.active) {
            const lower = text.toLowerCase();
            if (lower.includes("password") || lower.includes("密碼") || lower.includes("passphrase")) {
              term.write(`\r\n\x1b[33;1m🔒 [Agent: 等待密碼輸入，請在終端機中輸入密碼後按 Enter]\x1b[0m\r\n`);
            }
          }
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
          
          // If agent is running and user presses Enter:
          // - If a command is still running (e.g. awaiting password), pass Enter to PTY
          // - If no command is running (user typing a new command), interrupt the agent
          if (agentMissionRef.current?.active) {
             const lastBlock = blocksRef.current[blocksRef.current.length - 1];
             if (lastBlock?.status === "running") {
               // A command is running — user is probably entering a password, let it through
               writePty(session, ch).catch(console.error);
               continue;
             }
             // No running command — user is trying to type a new command, interrupt agent
             agentAbortRef.current = true;
             term.write("\r\n\x1b[33m[Agent Interrupted]\x1b[0m");
             stopMission();
          }

          // Read the visual line from the terminal buffer as fallback
          // (protects against IME / ANSI escape corruption of lineBufRef)
          let fallbackAiQuery: string | null = null;
          let fallbackAgentQuery: string | null = null;
          const baseY = term.buffer.active.baseY;
          const visualLine = term.buffer.active.getLine(term.buffer.active.cursorY + baseY)?.translateToString(true).trim() || "";
          const visualMatch = visualLine.match(/(?:%|\$|#|❯)\s*\/(ai|agent)\s+(.+)$/);
          if (visualMatch) {
             if (visualMatch[1] === "ai") fallbackAiQuery = visualMatch[2];
             if (visualMatch[1] === "agent") fallbackAgentQuery = visualMatch[2];
          }

          const agentQuery = parseAgentPrefix(line) || fallbackAgentQuery;
          const aiQuery = parseAiPrefix(line) || fallbackAiQuery;
          
          if (agentQuery !== null || aiQuery !== null) {
            const finalQuery = agentQuery || aiQuery!;
            if (previewRef.current.loading) {
              writeRed("aiterm: already waiting for AI response");
              continue;
            }
            // Start Agent Loop (callback-driven, no useEffect)
            agentAbortRef.current = false;
            startMission(finalQuery, 5);
            runAgentLoop({
              goal: finalQuery,
              sessionId: session,
              term,
              getSubmitCommand: () => submitCommandRef.current,
              setPreview,
              setStreamText,
              streamingRef,
              executionModeRef,
              writeRed,
              abortRef: agentAbortRef,
              stepCount: 0,
              maxSteps: maxAgentStepsRef.current,
              history: [],
              onComplete: () => {
                term.write(`\r\n\x1b[32m[Agent Mission Completed] 🎉\x1b[0m\r\n`);
                stopMission();
                writePty(session, "\r").catch(console.error);
              },
              onFail: (msg) => {
                term.write(`\r\n\x1b[33m⚠ Agent stopped: ${msg}\x1b[0m\r\n`);
                stopMission();
                writePty(session, "\r").catch(console.error);
              },
            });
            continue;
          }
          writePty(session, ch).catch(console.error);
        } else if (ch === "\x7f" || ch === "\b") {
          lineBufRef.current = lineBufRef.current.slice(0, -1);
          writePty(session, ch).catch(console.error);
        } else if (ch === "\x03" || ch === "\x15") {
          lineBufRef.current = "";
          writePty(session, ch).catch(console.error);
        } else {
          // Ignore ANSI escape sequence starts (like arrow keys) which corrupt the simple buffer
          if (ch !== "\x1b") {
            lineBufRef.current += ch;
          }
          writePty(session, ch).catch(console.error);
        }
      }
    });

    term.onResize(({ rows: r, cols: c }) => {
      if (sessionRef.current) {
        resizePty(sessionRef.current, { rows: r, cols: c }).catch(console.error);
      }
      setTimeout(() => {
        setRenderTick(t => t + 1);
        const viewportEl = hostRef.current?.querySelector('.xterm-viewport');
        if (viewportEl && overlayRef.current) {
          overlayRef.current.style.transform = `translateY(-${viewportEl.scrollTop}px)`;
        }
      }, 50);
    });

    let ro: ResizeObserver | null = null;
    if (hostRef.current) {
      ro = new ResizeObserver(() => {
        fit.fit();
      });
      ro.observe(hostRef.current);
    }

    return () => {
      if (ro && hostRef.current) ro.unobserve(hostRef.current);
      if (unlistenData) unlistenData();
      if (unlistenStream) unlistenStream.then((f: () => void) => f());
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
        height: "100%",
        position: "relative",
      }}
    >
      <div className="aiterm-status">
        <span className="aiterm-status-left" style={{ display: "flex", alignItems: "center", gap: "8px" }}>
          {onToggleSidebar && (
            <button
              className="aiterm-settings-btn"
              title="Toggle Sidebar (Ctrl+B)"
              onClick={onToggleSidebar}
              style={{ fontSize: "16px", padding: "0 4px", marginLeft: "-4px" }}
            >
              {isSidebarOpen ? "◧" : "☰"}
            </button>
          )}
          <span>AITerm · {status}</span>
        </span>
        {activeProvider && (
          <span style={{ display: "flex", alignItems: "center" }}>
            <button
              className="aiterm-status-provider"
              title="切換 Provider (Ctrl+Shift+P)"
              onClick={() => setPaletteOpen((o) => !o)}
            >
              {activeProvider}
            </button>
            <button
              className="aiterm-block-btn aiterm-block-btn-ai"
              title="開啟 AI 助手 (Ctrl+I)"
              onClick={(e) => {
                 e.stopPropagation();
                 window.dispatchEvent(new CustomEvent('aiterm:ask-ai', { detail: {} }));
              }}
              style={{ marginLeft: "8px", padding: "2px 8px" }}
            >
              ✨ Ask AI
            </button>
          </span>
        )}
      </div>

      {/* Sub-tabs: Terminal | Files */}
      <div className="aiterm-subtabs">
        <button
          className={`aiterm-subtab ${viewTab === "terminal" ? "aiterm-subtab--active" : ""}`}
          onClick={() => setViewTab("terminal")}
        >{t.terminal_tab}</button>
        <button
          className={`aiterm-subtab ${viewTab === "files" ? "aiterm-subtab--active" : ""}`}
          onClick={() => setViewTab("files")}
        >{t.files_tab}</button>
      </div>

      <div style={{ position: "relative", flex: 1, minHeight: 0, width: "100%" }}>
        {/* File Explorer */}
        {viewTab === "files" && sessionId && (
          <div style={{ height: "100%", overflow: "hidden" }}>
            <FileExplorer sessionId={sessionId} />
          </div>
        )}
        {/* Terminal */}
        <div style={{ display: viewTab === "terminal" ? "block" : "none", height: "100%" }}>
        <div
          ref={hostRef}
          className="aiterm-terminal-root"
          style={{ height: "100%", width: "calc(100% - 20px)", marginLeft: "20px", boxSizing: "border-box" }}
        />
        
        {/* React DOM overlay for visual blocks */}
        <div style={{ position: "absolute", top: 0, left: 0, right: 0, bottom: 0, pointerEvents: "none", overflow: "hidden", zIndex: 10 }}>
           <div ref={overlayRef} style={{ position: "absolute", top: 0, left: 0, right: 0, willChange: 'transform' }}>
             {(() => {
               const mappedBlocks = blocks.map((b) => {
                 let parsedPromptY = (b.startMarker?.line ?? b.startLine ?? 1) - 1;
                 if (termState && b.command) {
                   const maxLen = termState.buffer.active.length;
                   for (let d = 0; d < 100; d++) {
                     const up = parsedPromptY - d;
                     if (up >= 0 && up < maxLen) {
                       const upStr = termState.buffer.active.getLine(up)?.translateToString(true) || "";
                       if (upStr.includes(b.command)) { parsedPromptY = up; break; }
                     }
                     const down = parsedPromptY + d;
                     if (down >= 0 && down < maxLen) {
                       const downStr = termState.buffer.active.getLine(down)?.translateToString(true) || "";
                       if (downStr.includes(b.command)) { parsedPromptY = down; break; }
                     }
                   }
                 }
                 return { ...b, parsedPromptY, parsedStartY: parsedPromptY + 1 };
               });

               mappedBlocks.forEach((b, i) => {
                 if (i < mappedBlocks.length - 1) {
                   b.endLine = mappedBlocks[i+1].parsedPromptY;
                 } else {
                   if (termState) {
                     b.endLine = termState.buffer.active.cursorY + termState.buffer.active.baseY;
                   } else {
                     b.endLine = b.endMarker?.line ?? b.endLine ?? (b.parsedStartY + 1);
                   }
                 }
               });

               return mappedBlocks.map((b) => {
                 if (b.status === "running" || !termState) return null;
                 const termEl = (termState as any).element;
                 const fallbackHeight = termEl ? termEl.clientHeight / termState.rows : 14 * 1.4;
                 const cellHeight = (termState as any)._core?._renderService?.dimensions?.css?.cell?.height || fallbackHeight;
                 
                 const top = b.parsedStartY * cellHeight;
                 const heightLines = Math.max(1, b.endLine! - b.parsedStartY);
                 const heightPx = heightLines * cellHeight;

                 // Only render if it has visual height
                 if (heightPx <= 0) return null;

                 return (
                   <div
                     key={b.id}
                     className="aiterm-block-decoration"
                     style={{
                        position: "absolute",
                        top: `${top}px`,
                        left: "8px",
                        right: "12px",
                        height: `${heightPx}px`,
                        pointerEvents: "none",
                        borderLeft: `3px solid ${b.exitCode === 0 ? '#34d399' : '#f87171'}`,
                        background: b.exitCode === 0 ? 'rgba(255, 255, 255, 0.04)' : 'rgba(255, 60, 60, 0.08)',
                        borderRadius: "6px",
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
                           const orig = btn.innerHTML;
                           btn.innerHTML = '📋 Copied';
                           setTimeout(() => btn.innerHTML = orig, 1000);
                         }}
                     >
                       📋 Copy
                     </button>
                     </div>
                   </div>
                 );
               });
             })()}
           </div>
        </div>
        </div>{/* end terminal wrapper */}
      </div>{/* end relative container */}
      {!isAlternateBuffer && (
        <WarpInput
          onSubmit={(cmd) => {
            const agentQuery = parseAgentPrefix(cmd);
            const aiQuery = parseAiPrefix(cmd);
            if (agentQuery !== null || aiQuery !== null) {
              const finalQuery = agentQuery || aiQuery!;
              if (previewRef.current.loading) {
                termRef.current?.write("\r\n\x1b[31maiterm: already waiting for AI response\x1b[0m\r\n");
                return;
              }
              if (termRef.current && sessionId) {
                // Start Agent Loop (same as terminal Enter handler)
                agentAbortRef.current = false;
                startMission(finalQuery, 5);
                runAgentLoop({
                  goal: finalQuery,
                  sessionId,
                  term: termRef.current,
                  getSubmitCommand: () => submitCommandRef.current,
                  setPreview,
                  setStreamText,
                  streamingRef,
                  executionModeRef,
                  writeRed: (msg) => termRef.current?.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`),
                  abortRef: agentAbortRef,
                  stepCount: 0,
                  maxSteps: maxAgentStepsRef.current,
                  history: [],
                  onComplete: () => {
                    termRef.current?.write(`\r\n\x1b[32m[Agent Mission Completed] 🎉\x1b[0m\r\n`);
                    stopMission();
                    if (sessionId) writePty(sessionId, "\r").catch(console.error);
                  },
                  onFail: (msg) => {
                    termRef.current?.write(`\r\n\x1b[33m⚠ Agent stopped: ${msg}\x1b[0m\r\n`);
                    stopMission();
                    if (sessionId) writePty(sessionId, "\r").catch(console.error);
                  },
                });
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
 * Kick off a single /ai request: show the streaming indicator,
 * invoke the backend, then either auto-execute or show the preview.
 * For the agent loop, this is called with agentActive=true and onCommandComplete
 * which fires AFTER the executed command finishes in the PTY.
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
  submitCommand: (cmd: string, onComplete?: (block: import("../hooks/useTerminalBlocks").TerminalBlock) => void) => void,
  onDone?: () => void,
  agentActive = false,
  onCommandComplete?: (block: import("../hooks/useTerminalBlocks").TerminalBlock) => void
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
      
      if (resp.command === "DONE") {
        setPreview(INITIAL_PREVIEW);
        if (onDone) onDone();
        return;
      }

      const mode = executionModeRef.current;
      const risk = resp.risk_level;

      if (shouldAutoExecute(mode, risk, agentActive)) {
        // Auto-execute: write a subtle confirmation line then submit.
        const riskColor = risk === "safe" ? "\x1b[32m" : "\x1b[33m";
        term.write(`\r\n${riskColor}▶ ${resp.command}\x1b[0m\r\n`);
        // Pass onCommandComplete so the block hook calls it when OSC 133;D fires
        submitCommand(resp.command, onCommandComplete);
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
 * Callback-driven Agent Loop.
 * Each step: ask AI → auto-execute command → wait for block completion → extract output → repeat.
 * This does NOT rely on React useEffect — the loop is driven by OSC 133;D completion callbacks.
 */
interface AgentLoopParams {
  goal: string;
  sessionId: string;
  term: Terminal;
  getSubmitCommand: () => (cmd: string, onComplete?: (block: import("../hooks/useTerminalBlocks").TerminalBlock) => void) => void;
  setPreview: (p: PreviewState) => void;
  setStreamText: React.Dispatch<React.SetStateAction<string>>;
  streamingRef: React.MutableRefObject<boolean>;
  executionModeRef: React.MutableRefObject<ExecutionMode>;
  writeRed: (msg: string) => void;
  abortRef: React.MutableRefObject<boolean>;
  stepCount: number;
  maxSteps: number;
  history: { command: string; exitCode: number; output: string }[];
  onComplete: () => void;
  onFail: (msg: string) => void;
}

function runAgentLoop(params: AgentLoopParams) {
  const {
    goal, sessionId, term, getSubmitCommand,
    setPreview, setStreamText, streamingRef, executionModeRef,
    writeRed, abortRef, stepCount, maxSteps, history,
    onComplete, onFail,
  } = params;

  if (abortRef.current) return;
  if (stepCount >= maxSteps) {
    onFail(`已達最大迭代次數 (${maxSteps})`);
    return;
  }

  // Build the query for the AI
  let query: string;
  if (history.length === 0) {
    query = goal;
  } else {
    query = `Goal: ${goal}\n\nExecution History:\n${history.map((h, i) =>
      `Step ${i + 1}:\nCommand: ${h.command}\nExit code: ${h.exitCode}\nOutput:\n${h.output}`
    ).join('\n\n')}\n\nAnalyze the result above and decide the next step to achieve the goal. If the goal is fully achieved, respond with command set to 'DONE'.`;
  }

  if (stepCount > 0) {
    term.write(`\r\n\x1b[35m[Agent: 思考下一步... (${stepCount}/${maxSteps})]\x1b[0m\r\n`);
  }

  // This callback fires when the command FINISHES executing in the PTY (via OSC 133;D)
  let stepResolved = false; // Set to true when either block completes OR AI returns DONE
  const onBlockDone = (completedBlock: import("../hooks/useTerminalBlocks").TerminalBlock) => {
    stepResolved = true;
    if (abortRef.current) return;

    // Extract terminal output for this block
    const startY = completedBlock.startLine ?? 0;
    const endY = completedBlock.endLine ?? term.buffer.active.cursorY + term.buffer.active.baseY;
    let rawOutput = "";
    for (let i = startY; i < endY; i++) {
      rawOutput += term.buffer.active.getLine(i)?.translateToString(true) + "\n";
    }
    rawOutput = rawOutput.trim();
    if (rawOutput.length > 2000) rawOutput = rawOutput.slice(rawOutput.length - 2000);

    const newHistory = [...history, {
      command: completedBlock.command,
      exitCode: completedBlock.exitCode ?? 0,
      output: rawOutput,
    }];

    // Recurse to next step
    runAgentLoop({
      ...params,
      history: newHistory,
      stepCount: stepCount + 1,
    });
  };

  // Wrap onComplete so we mark the step as resolved (prevents timeout from firing)
  const wrappedOnComplete = () => {
    stepResolved = true;
    onComplete();
  };

  // Timeout: if the command hasn't completed in 60s, it likely needs user input
  setTimeout(() => {
    if (!stepResolved && !abortRef.current) {
      term.write(`\r\n\x1b[33m⚠ Agent 逾時：指令超過 60 秒未完成，可能需要手動輸入（如密碼）。Agent 已暫停。\x1b[0m\r\n`);
      onFail("指令逾時，可能需要互動式輸入");
    }
  }, 60000);

  // Call AI, auto-execute the returned command, wire up the completion callback
  handleAiQuery(
    sessionId,
    "",
    query,
    term,
    setPreview,
    setStreamText,
    streamingRef,
    executionModeRef,
    writeRed,
    getSubmitCommand(),  // always get the LATEST submitCommand
    wrappedOnComplete,   // onDone: AI returned "DONE" → mark resolved & complete
    true,                // agentActive: force auto-execute for safe commands
    onBlockDone,         // onCommandComplete: fires when OSC 133;D marks the block done
  );
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
