import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useLocale } from "../contexts/LocaleContext";
import type { Locale } from "../lib/i18n";
import { listen } from "@tauri-apps/api/event";
import { homeDir } from "@tauri-apps/api/path";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
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
import { getSessionCwd } from "../ipc/fs";
import { enterpriseCompleteTask, enterpriseOnComplete } from "../ipc/enterprise";
import { useTerminalBlocks } from "../hooks/useTerminalBlocks";
import { useAgentMission } from "../hooks/useAgentMission";
import { useTelegramRemoteControl } from "../hooks/useTelegramRemoteControl";
import { listProviders } from "../ipc/provider";
import { webSearch, webFetch } from "../ipc/web";
import { parseAiPrefix, parseAgentPrefix } from "./parseAiPrefix";
import { CommandPreview } from "./CommandPreview";
import { StreamingIndicator } from "./StreamingIndicator";
import { AiPanel } from "./AiPanel";
import { ProviderPalette } from "./ProviderPalette";
import { WarpInput } from "./WarpInput";
import { FileExplorer } from "./FileExplorer/FileExplorer";
import { CommandBookmarksPicker, addBookmark } from "./CommandBookmarks";
import { getActiveTheme, type AppTheme } from "../lib/themes";
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
  /** Called once with the backend-assigned PTY session ID when the PTY is created. */
  onSessionCreated?: (sessionId: string) => void;
  /** If set, the PTY starts in this directory (overrides last-cwd from localStorage). */
  initialCwd?: string;
  /** If set, the agent loop starts automatically after the PTY is ready. */
  initialMission?: { goal: string; maxSteps: number };
  /** Enterprise task metadata — triggers on_complete actions when the mission finishes. */
  enterpriseTask?: { taskId: string; workBranch: string; onComplete: unknown };
  /** Called on each agent step when running an enterprise task, for the progress panel. */
  onAgentProgress?: (done: number, total: number) => void;
}

const SEARCH_OPTS = {
  regex: false,
  caseSensitive: false,
  wholeWord: false,
  decorations: {
    matchBackground: '#ffff0040',
    matchBorder: '#ffff00',
    matchOverviewRuler: '#ffff00',
    activeMatchBackground: '#ff990040',
    activeMatchBorder: '#ff9900',
    activeMatchColorOverviewRuler: '#ff9900',
  },
};

export function TerminalView({ isActive = true, onToggleSidebar, isSidebarOpen = true, onSessionCreated, initialCwd, initialMission, enterpriseTask, onAgentProgress }: TerminalViewProps) {
  type ViewTab = "terminal" | "files";
  const [viewTab, setViewTab] = useState<ViewTab>("terminal");
  const navigate = useNavigate();
  const { t, locale } = useLocale();
  const hostRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<string>("initializing…");
  const [preview, setPreview] = useState<PreviewState>(INITIAL_PREVIEW);
  const previewRef = useRef<PreviewState>(INITIAL_PREVIEW);
  previewRef.current = preview;

  const [panelOpen, setPanelOpen] = useState(false);
  const [bookmarksOpen, setBookmarksOpen] = useState(false);
  const [sessionId, setSessionId] = useState<string>("");
  const [displayCwd, setDisplayCwd] = useState<string>("");

  // Persist the active terminal's CWD to localStorage so it can be
  // restored on the next session. Also updates the status bar CWD display.
  useEffect(() => {
    if (!sessionId) return;
    let lastSaved = "";
    let homePath = "";
    homeDir().then((h) => { homePath = h.replace(/\\/g, "/").replace(/\/$/, ""); }).catch(() => {});
    const id = setInterval(async () => {
      try {
        const cwd = await getSessionCwd(sessionId);
        if (cwd && cwd !== lastSaved) {
          lastSaved = cwd;
          localStorage.setItem("aiterm_last_cwd", cwd);
          const normalized = cwd.replace(/\\/g, "/");
          const pretty = homePath && normalized.startsWith(homePath)
            ? "~" + normalized.slice(homePath.length)
            : normalized;
          setDisplayCwd(pretty);
        }
      } catch { /* session may not be ready yet */ }
    }, 2000);
    return () => clearInterval(id);
  }, [sessionId]);

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

  // Find in Buffer state
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchMatchInfo, setSearchMatchInfo] = useState<string>("");
  const searchAddonRef = useRef<SearchAddon | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

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

  // Apply font changes from Settings
  useEffect(() => {
    const handler = (e: Event) => {
      const { fontSize, fontFamily } = (e as CustomEvent).detail as { fontSize: number; fontFamily: string };
      const term = termRef.current;
      if (!term) return;
      term.options.fontSize = fontSize;
      term.options.fontFamily = fontFamily;
      const fit = fitAddonRef.current;
      if (fit) requestAnimationFrame(() => fit.fit());
    };
    window.addEventListener("aiterm:font-changed", handler);
    return () => window.removeEventListener("aiterm:font-changed", handler);
  }, []);

  // Apply theme changes from Settings
  useEffect(() => {
    const handler = (e: Event) => {
      const { theme } = (e as CustomEvent).detail as { theme: AppTheme };
      const term = termRef.current;
      if (!term) return;
      term.options.theme = theme.xterm;
    };
    window.addEventListener("aiterm:theme-changed", handler);
    return () => window.removeEventListener("aiterm:theme-changed", handler);
  }, []);

  // File paste/drop → write absolute path to PTY
  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;

    const handlePaste = async (e: ClipboardEvent) => {
      const files = e.clipboardData?.files;
      if (!files || files.length === 0) return; // let xterm handle text paste
      e.preventDefault();
      e.stopPropagation();
      const sid = sessionRef.current;
      if (!sid) return;
      const paths = Array.from(files)
        .map((f) => (f as File & { path?: string }).path ?? f.name)
        .join(" ");
      await writePty(sid, paths).catch(() => {});
    };

    const handleDragOver = (e: DragEvent) => {
      e.preventDefault();
    };

    const handleDrop = async (e: DragEvent) => {
      e.preventDefault();
      const files = e.dataTransfer?.files;
      if (!files || files.length === 0) return;
      const sid = sessionRef.current;
      if (!sid) return;
      const paths = Array.from(files)
        .map((f) => (f as File & { path?: string }).path ?? f.name)
        .join(" ");
      await writePty(sid, paths + " ").catch(() => {});
    };

    el.addEventListener("paste", handlePaste);
    el.addEventListener("dragover", handleDragOver);
    el.addEventListener("drop", handleDrop);
    return () => {
      el.removeEventListener("paste", handlePaste);
      el.removeEventListener("dragover", handleDragOver);
      el.removeEventListener("drop", handleDrop);
    };
  }, []); // hostRef and sessionRef are stable refs — no deps needed

  // Telegram Remote Control
  const { isRemoteEnabled, setIsRemoteEnabled, sendRemoteResponse } = useTelegramRemoteControl(
    sessionId,
    isActive,
    (text) => {
      const agentQuery = parseAgentPrefix(text);
      const aiQuery = parseAiPrefix(text);
      if (agentQuery !== null || aiQuery !== null) {
        const finalQuery = agentQuery || aiQuery!;
        if (previewRef.current.loading) return;
        agentAbortRef.current = false;
        startMission(finalQuery, 5);
        if (sessionRef.current && termRef.current) {
          runAgentLoop({
            t,
            goal: finalQuery,
            locale,
            sessionId: sessionRef.current,
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
            onComplete: (explanation?: string) => {
              if (explanation) {
                termRef.current?.write(`\r\n\x1b[36m${explanation.replace(/\n/g, "\r\n")}\x1b[0m\r\n`);
              }
              termRef.current?.write(`\r\n\x1b[32m[Agent Mission Completed] 🎉\x1b[0m\r\n`);
              stopMission();
              if (sessionRef.current) writePty(sessionRef.current, "\r").catch(console.error);
              sendRemoteResponse(explanation ? `Agent: ${explanation}` : "[Agent Mission Completed] 🎉");
            },
            onFail: (msg) => {
              termRef.current?.write(`\r\n\x1b[33m⚠ Agent stopped: ${msg}\x1b[0m\r\n`);
              stopMission();
              if (sessionRef.current) writePty(sessionRef.current, "\r").catch(console.error);
              sendRemoteResponse(`⚠ Agent stopped: ${msg}`);
            },
            onStepComplete: (info) => sendRemoteResponse(formatAgentStepForRemote(info)),
          });
        }
      } else {
        submitCommand(text, (block) => {
          if (!termRef.current) return;
          const term = termRef.current;
          // startLine points to the prompt + command echo.
          // endLine points to the new prompt line (or beyond).
          const startLine = block.startLine ?? 0;
          const endLine = block.endLine ?? term.buffer.active.baseY + term.buffer.active.cursorY;
          
          let output = "";
          // Extract lines strictly between the command line and the new prompt.
          for (let i = startLine + 1; i < endLine; i++) {
            const lineStr = term.buffer.active.getLine(i)?.translateToString(true) || "";
            output += lineStr + "\n";
          }
          
          output = output.trim();
          if (output.length > 0) {
            if (output.length > 4000) {
              output = output.substring(0, 4000) + "\n... (output truncated)";
            }
            sendRemoteResponse(output);
          } else {
            sendRemoteResponse(`(Command finished: ${block.command})`);
          }
        });
      }
    }
  );

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
      } else if (e.ctrlKey && e.shiftKey && e.key === "R") {
        e.preventDefault();
        setBookmarksOpen(true);
      } else if (e.ctrlKey && e.shiftKey && e.key === "P") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      } else if (e.ctrlKey && (e.key === "i" || e.key === "I")) {
        e.preventDefault();
        setPanelOpen((o) => !o);
      } else if (e.ctrlKey && e.key === "f") {
        e.preventDefault();
        setSearchOpen(true);
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

  // Auto-start agent loop when an enterprise task has been dispatched to this terminal.
  const initialMissionFiredRef = useRef(false);
  useEffect(() => {
    if (!initialMission || initialMissionFiredRef.current) return;
    if (!sessionId || !termRef.current) return;
    initialMissionFiredRef.current = true;
    const term = termRef.current;
    const session = sessionId;
    agentAbortRef.current = false;
    startMission(initialMission.goal, initialMission.maxSteps);
    // Brief delay to let the shell finish initializing before the first AI query.
    setTimeout(() => {
      runAgentLoop({
        t,
        goal: initialMission.goal,
        locale,
        sessionId: session,
        term,
        getSubmitCommand: () => submitCommandRef.current,
        setPreview,
        setStreamText,
        streamingRef,
        executionModeRef,
        writeRed: (msg) => term.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`),
        abortRef: agentAbortRef,
        stepCount: 0,
        maxSteps: initialMission.maxSteps,
        history: [],
        onStepComplete: ({ stepIndex, maxSteps: total }) => {
          onAgentProgress?.(stepIndex, total);
        },
        onComplete: () => {
          term.write(`\r\n\x1b[32m[Enterprise Task Completed] ✓\x1b[0m\r\n`);
          stopMission();
          writePty(session, "\r").catch(console.error);
          // Trigger on_complete (push + optional PR) and mark task done.
          if (enterpriseTask && initialCwd) {
            term.write(`\r\n\x1b[36m[Enterprise: running on_complete...]\x1b[0m\r\n`);
            enterpriseOnComplete({
              taskId: enterpriseTask.taskId,
              repoDir: initialCwd,
              workBranch: enterpriseTask.workBranch,
              onComplete: enterpriseTask.onComplete,
            }).then((msg) => {
              term.write(`\r\n\x1b[32m${msg}\x1b[0m\r\n`);
              enterpriseCompleteTask(enterpriseTask.taskId).catch(console.error);
            }).catch((err) => {
              term.write(`\r\n\x1b[31m[on_complete error: ${err}]\x1b[0m\r\n`);
              enterpriseCompleteTask(enterpriseTask.taskId).catch(console.error);
            });
          }
        },
        onFail: (msg) => {
          term.write(`\r\n\x1b[33m⚠ Enterprise Task stopped: ${msg}\x1b[0m\r\n`);
          stopMission();
          writePty(session, "\r").catch(console.error);
          if (enterpriseTask) {
            enterpriseCompleteTask(enterpriseTask.taskId).catch(console.error);
          }
        },
      });
    }, 1500);
  }, [sessionId, initialMission, startMission, stopMission, enterpriseTask, initialCwd]);

  useEffect(() => {
    if (!hostRef.current) return;

    const initFontSize = parseInt(localStorage.getItem("aiterm-font-size") ?? "14", 10) || 14;
    const initFontFamily = localStorage.getItem("aiterm-font-family") ?? '"Cascadia Mono", Consolas, monospace';
    const initTheme = getActiveTheme();

    const term = new Terminal({
      fontFamily: initFontFamily,
      fontSize: initFontSize,
      lineHeight: 1.1,
      cursorBlink: true,
      theme: initTheme.xterm,
      convertEol: false,
    });
    termRef.current = term;
    setTermState(term);

    const fit = new FitAddon();
    term.loadAddon(fit);
    fitAddonRef.current = fit;
    const searchAddon = new SearchAddon();
    term.loadAddon(searchAddon);
    searchAddonRef.current = searchAddon;
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
        const lastCwd = initialCwd ?? localStorage.getItem("aiterm_last_cwd") ?? undefined;
        const id = await createPty({ rows, cols }, lastCwd);
        sessionRef.current = id;
        setSessionId(id);
        onSessionCreated?.(id);
        setStatus(`connected (${id.slice(0, 8)}…)`);

        // On Windows, PowerShell doesn't emit OSC 133 shell-integration markers.
        // We detect the PS prompt pattern in PTY output and inject a synthetic
        // OSC 133;D so the agent loop (which waits for that signal) works normally.
        const isWindows = navigator.platform.toLowerCase().startsWith("win");
        let psPromptBuf = ""; // rolling buffer to handle prompts split across chunks

        unlistenData = await onPtyData(id, (bytes) => {
          const text = decoder.decode(bytes, { stream: true });
          term.write(text);

          // Windows: detect PS prompt → inject synthetic OSC 133 D
          if (isWindows) {
            psPromptBuf = (psPromptBuf + text).slice(-300);
            // Strip ANSI codes and check for a PS prompt at the end of the buffer
            const stripped = psPromptBuf.replace(/\x1b\[[^a-zA-Z]*[a-zA-Z]/g, "").trimEnd();
            if (/PS\s+[A-Za-z]:[^>]*>\s*$/.test(stripped)) {
              psPromptBuf = "";
              // Inject synthetic OSC 133;D — xterm.js parser fires our handler without
              // rendering anything visible.  Exit code 0 is safe: the agent reads the
              // full output text to determine success / failure.
              term.write("\x1b]133;D;0\x07");
            }
          }

          // Detect password prompts during agent mode
          if (agentMissionRef.current?.active) {
            const lower = text.toLowerCase();
            if (lower.includes("password") || lower.includes("密碼") || lower.includes("passphrase")) {
              term.write(`\r\n\x1b[33;1m${t.term_agent_wait_password}\x1b[0m\r\n`);
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

      // Drop focus-tracking events that xterm.js emits when it loses / gains focus.
      // PSReadLine enables focus tracking (ESC[?1004h), so xterm.js sends ESC[O (focus-out)
      // or ESC[I (focus-in) when the WarpInput textarea steals focus.  Forwarding them
      // character-by-character causes PowerShell to print "[O" / "[I" as literal text.
      if (data === "\x1b[O" || data === "\x1b[I") return;

      // Escape sequences (arrow keys, function keys, Home/End, etc.) must be sent as a
      // single atomic PTY write.  Iterating char-by-char produces separate Tauri IPC calls
      // with non-zero latency between the ESC byte and the rest of the sequence.  On Windows,
      // PSReadLine has a short escape-sequence timeout: if "[" doesn't arrive fast enough
      // after ESC, it treats ESC as a standalone keypress and "[A" / "[B" / "[C" / "[D"
      // then appear as literal text in the terminal.
      if (data.startsWith("\x1b")) {
        writePty(session, data).catch(console.error);
        return;
      }

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
              t,
              goal: finalQuery,
              locale,
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
                sendRemoteResponse("[Agent Mission Completed] 🎉");
              },
              onFail: (msg) => {
                term.write(`\r\n\x1b[33m⚠ Agent stopped: ${msg}\x1b[0m\r\n`);
                stopMission();
                writePty(session, "\r").catch(console.error);
                sendRemoteResponse(`⚠ Agent stopped: ${msg}`);
              },
              onStepComplete: (info) => sendRemoteResponse(formatAgentStepForRemote(info)),
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
        requestAnimationFrame(() => fit.fit());
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

  const doSearch = useCallback((query: string, direction: 'next' | 'prev') => {
    const addon = searchAddonRef.current;
    if (!addon || !query) { setSearchMatchInfo(""); return; }
    const found = direction === 'next'
      ? addon.findNext(query, SEARCH_OPTS)
      : addon.findPrevious(query, SEARCH_OPTS);
    setSearchMatchInfo(found ? "found" : "not found");
  }, []);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    setSearchQuery("");
    setSearchMatchInfo("");
    searchAddonRef.current?.clearDecorations?.();
  }, []);

  // Focus input when search opens
  useEffect(() => {
    if (searchOpen) {
      setTimeout(() => searchInputRef.current?.focus(), 50);
    }
  }, [searchOpen]);

  // Re-run search as user types
  useEffect(() => {
    if (searchQuery) {
      doSearch(searchQuery, 'next');
    } else {
      searchAddonRef.current?.clearDecorations?.();
      setSearchMatchInfo("");
    }
  }, [searchQuery, doSearch]);

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
        <span style={{ display: "flex", alignItems: "center" }}>
          {activeProvider ? (
            <button
              className="aiterm-status-provider"
              title={t.term_provider_tooltip_switch}
              onClick={() => setPaletteOpen((o) => !o)}
            >
              {activeProvider}
            </button>
          ) : (
            <button
              className="aiterm-status-provider aiterm-status-provider--empty"
              title={t.term_provider_tooltip_add}
              onClick={() => navigate("/settings")}
            >
              {t.ai_providers} ＋
            </button>
          )}
          <button
            className={`aiterm-block-btn aiterm-btn aiterm-btn--secondary ${isRemoteEnabled ? 'aiterm-agent-toggle--on' : ''}`}
            title={t.term_remote_tooltip}
            onClick={(e) => {
              e.stopPropagation();
              setIsRemoteEnabled((prev) => !prev);
            }}
            style={{ marginLeft: "8px", padding: "2px 8px" }}
          >
            📱 Remote
          </button>
          <button
            className="aiterm-block-btn aiterm-block-btn-ai aiterm-btn aiterm-btn--secondary"
            title={t.term_ai_helper_tooltip}
            onClick={(e) => {
               e.stopPropagation();
               window.dispatchEvent(new CustomEvent('aiterm:ask-ai', { detail: {} }));
            }}
            style={{ marginLeft: "8px", padding: "2px 8px" }}
          >
            ✨ Ask AI
          </button>
        </span>
      </div>

      {/* Sub-tabs: Terminal | Files | CWD path */}
      <div className="aiterm-subtabs">
        <button
          className={`aiterm-subtab ${viewTab === "terminal" ? "aiterm-subtab--active" : ""}`}
          onClick={() => setViewTab("terminal")}
        >{t.terminal_tab}</button>
        <button
          className={`aiterm-subtab ${viewTab === "files" ? "aiterm-subtab--active" : ""}`}
          onClick={() => setViewTab("files")}
        >{t.files_tab}</button>
        {displayCwd && (
          <span className="aiterm-status-cwd" title={displayCwd}>
            <span className="aiterm-status-cwd-icon">📁</span>
            {displayCwd}
          </span>
        )}
      </div>

      <div style={{ position: "relative", flex: 1, minHeight: 0, width: "100%" }}>
        {/* File Explorer */}
        {viewTab === "files" && sessionId && (
          <div style={{ height: "100%", overflow: "hidden" }}>
            <FileExplorer sessionId={sessionId} />
          </div>
        )}
        {/* Terminal */}
        <div style={{ display: viewTab === "terminal" ? "block" : "none", height: "100%", position: "relative" }}>
        {/* Find in Buffer search bar */}
        {searchOpen && (
          <div className="terminal-search-bar">
            <input
              ref={searchInputRef}
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') { e.preventDefault(); closeSearch(); }
                if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); doSearch(searchQuery, 'next'); }
                if (e.key === 'Enter' && e.shiftKey) { e.preventDefault(); doSearch(searchQuery, 'prev'); }
                if (e.key === 'F3' && !e.shiftKey) { e.preventDefault(); doSearch(searchQuery, 'next'); }
                if (e.key === 'F3' && e.shiftKey) { e.preventDefault(); doSearch(searchQuery, 'prev'); }
              }}
              placeholder={t.term_search_placeholder}
              className="terminal-search-input"
            />
            {searchMatchInfo && <span className="terminal-search-match-info">{searchMatchInfo}</span>}
            <button onClick={() => doSearch(searchQuery, 'prev')} title={t.term_search_prev} className="terminal-search-btn aiterm-btn aiterm-btn--secondary aiterm-btn--sm">↑</button>
            <button onClick={() => doSearch(searchQuery, 'next')} title={t.term_search_next} className="terminal-search-btn aiterm-btn aiterm-btn--secondary aiterm-btn--sm">↓</button>
            <button onClick={closeSearch} title={t.term_search_close} className="terminal-search-btn terminal-search-close aiterm-btn aiterm-btn--secondary aiterm-btn--sm">✕</button>
          </div>
        )}
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
                         className="aiterm-block-btn aiterm-block-btn-ai aiterm-btn aiterm-btn--secondary"
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
                          className="aiterm-block-btn aiterm-btn aiterm-btn--secondary"
                          onClick={(e) => {
                            e.stopPropagation();
                            addBookmark(b.command);
                            const btn = e.currentTarget;
                            const orig = btn.innerHTML;
                            btn.innerHTML = t.terminal_bookmark_saved;
                            setTimeout(() => btn.innerHTML = orig, 1200);
                          }}
                          title={t.term_bookmark_tooltip}
                      >
                        {t.terminal_bookmark_btn}
                      </button>
                      <button
                          className="aiterm-block-btn aiterm-btn aiterm-btn--secondary"
                          onClick={(e) => {
                            e.stopPropagation();
                            navigator.clipboard.writeText(b.command).catch(console.error);
                            const btn = e.currentTarget;
                            const orig = btn.innerHTML;
                            btn.innerHTML = t.terminal_copy_done;
                            setTimeout(() => btn.innerHTML = orig, 1000);
                          }}
                      >
                        {t.terminal_copy_btn}
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
                  t,
                  goal: finalQuery,
                  locale,
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
                  onStepComplete: (info) => sendRemoteResponse(formatAgentStepForRemote(info)),
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
      {bookmarksOpen && (
        <CommandBookmarksPicker
          onSelect={(cmd) => {
            setBookmarksOpen(false);
            window.dispatchEvent(new CustomEvent("aiterm:fill-input", { detail: { cmd } }));
          }}
          onClose={() => setBookmarksOpen(false)}
        />
      )}
      {paletteOpen && (
        <ProviderPalette
          onClose={() => setPaletteOpen(false)}
          onSwitch={(p) => setActiveProvider(p.display_name)}
        />
      )}
      {sessionId && (
        <AiPanel
          key={sessionId}
          sessionId={sessionId}
          isOpen={panelOpen}
          providerName={activeProvider}
          onClose={() => setPanelOpen(false)}
          onExecuteCommand={(cmd, onComplete) => submitCommand(cmd, onComplete)}
          onOpenProviderPalette={() => {
            setPanelOpen(false);
            setPaletteOpen(true);
          }}
          sendRemoteResponse={sendRemoteResponse}
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
  t: any,
  locale: Locale,
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
  onDone?: (explanation?: string) => void,
  agentActive = false,
  onCommandComplete?: (block: import("../hooks/useTerminalBlocks").TerminalBlock) => void,
  onAiError?: (err: AiError) => void,
  onWebAction?: (type: "search" | "fetch", value: string) => void
) {
  void originalLine;
  term.write("\r\x1b[2K");
  term.write("→ asking AI...\r\n");
  setStreamText("");
  streamingRef.current = true;
  setPreview({ loading: true, visible: false, command: "", explanation: "", riskLevel: "safe" });

  invokeAiQuery(query, sessionId, locale)
    .then((resp) => {
      streamingRef.current = false;
      term.write("\x1b[1A\x1b[2K");
      
      if (resp.command === "DONE") {
        setPreview(INITIAL_PREVIEW);
        if (onDone) onDone(resp.explanation);
        return;
      }

      // Intercept web search/fetch commands before PTY execution
      if (resp.command.startsWith("AITERM_WEB_SEARCH: ") && onWebAction) {
        const value = resp.command.slice("AITERM_WEB_SEARCH: ".length);
        setPreview(INITIAL_PREVIEW);
        onWebAction("search", value);
        return;
      }
      if (resp.command.startsWith("AITERM_WEB_FETCH: ") && onWebAction) {
        const value = resp.command.slice("AITERM_WEB_FETCH: ".length);
        setPreview(INITIAL_PREVIEW);
        onWebAction("fetch", value);
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
          term.write(`\x1b[31m${t.term_danger_warning}\x1b[0m\r\n`);
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
      term.write("\x1b[1A\x1b[2K");
      const err = normalizeAiError(rawErr);
      writeRed(formatAiError(err));

      // Actionable follow-up hints
      if (err.kind === "not_configured") {
        term.write(`\x1b[33m${t.term_setup_hint_provider}\x1b[0m\r\n`);
      } else if (
        err.kind === "network" &&
        (err.message?.toLowerCase().includes("ollama") ||
          err.message?.toLowerCase().includes("connection refused"))
      ) {
        term.write(
          `\x1b[33m${t.term_setup_hint_ollama}\x1b[0m\r\n`
        );
      } else if (err.kind === "auth_failed") {
        term.write(`\x1b[33m${t.term_setup_hint_api_key}\x1b[0m\r\n`);
      }

      setPreview(INITIAL_PREVIEW);
      if (onAiError) onAiError(err);
    });
}

/**
 * Callback-driven Agent Loop.
 * Each step: ask AI → auto-execute command → wait for block completion → extract output → repeat.
 * This does NOT rely on React useEffect — the loop is driven by OSC 133;D completion callbacks.
 */
interface AgentStepInfo {
  /** 1-based step index for display (matches the "[Agent: 思考下一步... (N/M)]" prompt). */
  stepIndex: number;
  maxSteps: number;
  command: string;
  exitCode: number;
  /** Already trimmed and length-capped (~2000 chars) by the agent loop. */
  output: string;
}

/** Format one agent step's command + output as a single Telegram message. */
function formatAgentStepForRemote(info: AgentStepInfo): string {
  // xterm's translateToString already returns plain text, but defend
  // against stray escape codes from copy-pasted prompts etc.
  const cleaned = info.output.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "").trim();
  const exitTag = info.exitCode === 0 ? "" : ` ⚠️ exit ${info.exitCode}`;
  const header = `[${info.stepIndex}/${info.maxSteps}] $ ${info.command}${exitTag}`;
  if (!cleaned) return header;

  // Telegram caps text messages at 4096 chars; reserve room for header + marker.
  const MAX = 3500;
  let body = cleaned;
  if (body.length > MAX) {
    const half = Math.floor(MAX / 2);
    body = `${body.slice(0, half)}\n... (truncated, ${body.length - MAX} chars omitted) ...\n${body.slice(-half)}`;
  }
  return `${header}\n${body}`;
}

interface AgentLoopParams {
  t: any;
  goal: string;
  locale: Locale;
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
  onComplete: (explanation?: string) => void;
  onFail: (msg: string) => void;
  /** Fires after each shell command finishes; used to mirror progress to Telegram. */
  onStepComplete?: (info: AgentStepInfo) => void;
}

function runAgentLoop(params: AgentLoopParams) {
  const {
    t, goal, locale, sessionId, term, getSubmitCommand,
    setPreview, setStreamText, streamingRef, executionModeRef,
    writeRed, abortRef, stepCount, maxSteps, history,
    onComplete, onFail,
  } = params;

  if (abortRef.current) return;
  if (stepCount >= maxSteps) {
    onFail(t.term_agent_max_steps(maxSteps));
    return;
  }

  // Build the query for the AI
  const webSearchNote = `\n\nNote: If you need to search the web for information, respond with command set to "AITERM_WEB_SEARCH: <your query>". If you need to fetch a specific URL, respond with command set to "AITERM_WEB_FETCH: <url>".`;
  let query: string;
  if (history.length === 0) {
    query = goal + `\n\nYou have access to web search. If you need internet information, respond with command set to "AITERM_WEB_SEARCH: <your query>" instead of a shell command.`;
  } else {
    query = `Goal: ${goal}\n\nExecution History:\n${history.map((h, i) =>
      `Step ${i + 1}:\nCommand: ${h.command}\nExit code: ${h.exitCode}\nOutput:\n${h.output}`
    ).join('\n\n')}\n\nAnalyze the result above and decide the next step to achieve the goal. If the goal is fully achieved, respond with command set to 'DONE'.${webSearchNote}`;
  }

  if (stepCount > 0) {
    term.write(`\r\n\x1b[35m${t.term_agent_thinking(stepCount + 1, maxSteps)}\x1b[0m\r\n`);
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

    const exitCode = completedBlock.exitCode ?? 0;

    // Mirror this step (command + output) to Telegram if the caller wired it up.
    params.onStepComplete?.({
      stepIndex: stepCount + 1,
      maxSteps,
      command: completedBlock.command,
      exitCode,
      output: rawOutput,
    });

    const newHistory = [...history, {
      command: completedBlock.command,
      exitCode,
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
  const wrappedOnComplete = (explanation?: string) => {
    stepResolved = true;
    onComplete(explanation);
  };

  // Timeout: if the command hasn't completed in 60s, it likely needs user input
  setTimeout(() => {
    if (!stepResolved && !abortRef.current) {
      term.write(`\r\n\x1b[33m${t.term_agent_timeout}\x1b[0m\r\n`);
      onFail(t.term_agent_timeout_fail);
    }
  }, 60000);

  // Handle web search/fetch actions from the AI (intercept before PTY execution)
  const onWebAction = (type: "search" | "fetch", value: string) => {
    stepResolved = true; // prevent timeout from firing while waiting for web result
    const label = type === "search" ? `\x1b[36m🔍 搜尋: ${value}\x1b[0m` : `\x1b[36m📄 取得: ${value}\x1b[0m`;
    term.write(`\r\n${label}\r\n`);
    const webPromise = type === "search" ? webSearch(value) : webFetch(value);
    webPromise
      .then((result) => {
        if (abortRef.current) return;
        stepResolved = false; // reset so next step timeout works
        const syntheticCommand = type === "search" ? `web_search("${value}")` : `web_fetch("${value}")`;
        const newHistory = [...history, {
          command: syntheticCommand,
          exitCode: 0,
          output: result,
        }];
        params.onStepComplete?.({
          stepIndex: stepCount + 1,
          maxSteps,
          command: syntheticCommand,
          exitCode: 0,
          output: result.length > 2000 ? result.slice(result.length - 2000) : result,
        });
        runAgentLoop({
          ...params,
          history: newHistory,
          stepCount: stepCount + 1,
        });
      })
      .catch((err) => {
        if (abortRef.current) return;
        onFail(`Web ${type} failed: ${String(err)}`);
      });
  };

  // Call AI, auto-execute the returned command, wire up the completion callback
  handleAiQuery(
    t,
    locale,
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
    (err) => {           // onAiError: AI call failed, abort the mission immediately
      stepResolved = true;
      let errMsg = "未知錯誤";
      if ("message" in err) errMsg = err.message;
      else if ("reason" in err) errMsg = err.reason;
      else if (err.kind === "not_configured") errMsg = "未設定 API Key";
      else if (err.kind === "rate_limit") errMsg = err.body ? `請求過於頻繁，請稍後再試\n${err.body}` : "請求過於頻繁，請稍後再試";
      onFail(`AI 請求失敗: ${errMsg}`);
    },
    onWebAction,          // onWebAction: intercept web search/fetch commands
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
