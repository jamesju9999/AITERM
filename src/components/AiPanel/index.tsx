import {
  useEffect, useRef, useState, useCallback,
  type KeyboardEvent, type PointerEvent,
} from "react";
import { readFileAsAttachment, contentToDisplayString } from "../../types/attachment";
import type { Attachment } from "../../types/attachment";
import { useMcpChat } from "../../hooks/useMcpChat";
import { invokeAiChat, type ChatMessage as AiChatMessage } from "../../ipc/ai";
import { getSessionCwd, listDirectory } from "../../ipc/fs";
import { getPtyRecentOutput, writePty } from "../../ipc/pty";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { getMcpTools } from "../../ipc/mcp";
import { languageDirective } from "../../lib/i18n";
import { useLocale } from "../../contexts/LocaleContext";
import type { TerminalBlock } from "../../hooks/useTerminalBlocks";
import { MessageList } from "./MessageList";
import { ZapIcon, WrenchIcon } from "../Icons";
import "./styles.css";

const MIN_WIDTH = 280;
const MAX_WIDTH_RATIO = 0.75;
const STORAGE_WIDTH_KEY = "aiterm-panel-width";
const STORAGE_AGENT_MODE_KEY = "aiterm-agent-mode";
const STORAGE_USE_MCP_KEY = "aiterm-use-mcp";

function loadSavedWidth(): number {
  try {
    const v = localStorage.getItem(STORAGE_WIDTH_KEY);
    if (v) return Math.max(MIN_WIDTH, parseInt(v, 10));
  } catch { /* ignore */ }
  return 420;
}

function loadSavedAgentMode(): boolean {
  try {
    const v = localStorage.getItem(STORAGE_AGENT_MODE_KEY);
    if (v !== null) return v === "true";
  } catch { /* ignore */ }
  return false;
}

function loadSavedUseMcp(): boolean {
  try {
    const v = localStorage.getItem(STORAGE_USE_MCP_KEY);
    if (v !== null) return v === "true";
  } catch { /* ignore */ }
  return true;
}

export interface AiPanelProps {
  sessionId: string;
  isOpen: boolean;
  providerName: string;
  onClose: () => void;
  onExecuteCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  onOpenProviderPalette: () => void;
  sendRemoteResponse?: (text: string) => void;
}

/**
 * The panel stays mounted across open/close so `useMcpChat`'s event listener
 * keeps receiving streaming chunks while the user toggles Ctrl+I. We hide
 * the panel with a CSS class when `isOpen=false` rather than returning null.
 */
export function AiPanel({
  sessionId,
  isOpen,
  providerName,
  onClose,
  onExecuteCommand,
  onOpenProviderPalette,
  sendRemoteResponse,
}: AiPanelProps) {
  const { t, locale } = useLocale();
  const chat = useMcpChat(sessionId);
  const [input, setInput] = useState("");
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const MAX_IMAGE_BYTES = 5 * 1024 * 1024; // 5 MB
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [mcpEnabled, setMcpEnabled] = useState(true);
  const [mcpToolCount, setMcpToolCount] = useState(0);
  const [useMcp, setUseMcp] = useState(loadSavedUseMcp);
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>("enter");

  const processFiles = useCallback(async (files: FileList | File[]) => {
    const arr = Array.from(files);
    const results = await Promise.allSettled(arr.map(async (file) => {
      if (file.type.startsWith("image/") && file.size > MAX_IMAGE_BYTES) {
        throw new Error(`${file.name} 超過 5MB 限制`);
      }
      return readFileAsAttachment(file);
    }));
    const valid = results
      .filter((r): r is PromiseFulfilledResult<Attachment> => r.status === "fulfilled")
      .map((r) => r.value);
    setAttachments((prev) => [...prev, ...valid]);
  }, [MAX_IMAGE_BYTES]);

  const handlePaste = useCallback(async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = e.clipboardData?.files;
    if (files && files.length > 0) {
      e.preventDefault();
      await processFiles(files);
    }
    // No files → let default text paste proceed
  }, [processFiles]);

  const handleDragOver = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
  }, []);

  const handleDrop = useCallback(async (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      await processFiles(files);
    }
  }, [processFiles]);

  // ── Resize ────────────────────────────────────────────────────────────────
  const [panelWidth, setPanelWidth] = useState(loadSavedWidth);
  const isDraggingRef = useRef(false);
  const dragStartXRef = useRef(0);
  const dragStartWidthRef = useRef(0);

  const onResizePointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    isDraggingRef.current = true;
    dragStartXRef.current = e.clientX;
    dragStartWidthRef.current = panelWidth;
    (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
  };

  const onResizePointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!isDraggingRef.current) return;
    const delta = dragStartXRef.current - e.clientX; // drag left → wider
    const maxWidth = Math.floor(window.innerWidth * MAX_WIDTH_RATIO);
    const next = Math.max(MIN_WIDTH, Math.min(maxWidth, dragStartWidthRef.current + delta));
    setPanelWidth(next);
  };

  const onResizePointerUp = () => {
    if (!isDraggingRef.current) return;
    isDraggingRef.current = false;
    setPanelWidth((w) => {
      try { localStorage.setItem(STORAGE_WIDTH_KEY, String(w)); } catch { /* ignore */ }
      return w;
    });
  };

  // ── Agent mode ────────────────────────────────────────────────────────────
  const [agentMode, setAgentMode] = useState(loadSavedAgentMode);
  const [agentRunning, setAgentRunning] = useState(false);
  const [agentStep, setAgentStep] = useState(0);
  const agentAbortRef = useRef(false);
  const [maxAgentSteps, setMaxAgentSteps] = useState<number>(5);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      const [cfg, tools] = await Promise.all([getConfig(), getMcpTools()]);
      if (cancelled) return;
      // 0 = unlimited; use a large number internally
      setMaxAgentSteps(cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5));
      const globalEnabled = cfg.mcp_enabled ?? true;
      setMcpEnabled(globalEnabled);
      setMcpToolCount(tools.length);
      setSubmitShortcut(cfg.submit_shortcut ?? "enter");
    };
    load().catch(() => {});
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_AGENT_MODE_KEY, String(agentMode));
    } catch { /* ignore */ }
  }, [agentMode]);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_USE_MCP_KEY, String(useMcp));
    } catch { /* ignore */ }
  }, [useMcp]);

  /** Build system prompt with live CWD + dir listing. */
  const buildAgentSystemPrompt = useCallback(async (): Promise<string> => {
    const cwd = await getSessionCwd(sessionId).catch(() => null) ?? "(unknown)";
    let dirList = "";
    try {
      const entries = await listDirectory(sessionId, "");
      dirList = entries
        .slice(0, 60)
        .map((e) => (e.is_dir ? `${e.name}/` : e.name))
        .join("\n");
    } catch { /* ignore */ }

    return `You are a terminal Agent. You can execute shell commands via <cmd>...</cmd> tags, and iterate based on the results to accomplish the user's goal.

Current working directory: ${cwd}
Directory contents (first 60 entries):
${dirList || "(unavailable)"}

Rules:
1. When you need to run a command, use <cmd>shell command</cmd> (only one at a time).
2. The system will execute it automatically and return the result — keep analyzing until the goal is achieved.
3. Once the goal is achieved, give your final explanation in ${languageDirective(locale)}, and do not include any more <cmd> tags.
4. Never perform destructive or irreversible operations (e.g. rm -rf /).
5. Write all explanations in ${languageDirective(locale)}.`;
  }, [sessionId, locale]);

  /**
   * Recursive agent loop. Each call:
   * 1. Asks AI with current history
   * 2. If AI returns a <cmd>, execute it and recurse with the output
   * 3. If no <cmd>, loop ends
   */
  // Holds the latest runAgentLoop so the recursive call below reads through
  // a ref instead of closing over the `const` being defined (which
  // react-hooks/immutability flags as accessed-before-declared).
  const runAgentLoopRef = useRef<
    (history: { role: "user" | "assistant"; content: string }[], systemPrompt: string, step: number) => Promise<void>
  >(async () => {});

  const runAgentLoop = useCallback(async (
    history: { role: "user" | "assistant"; content: string }[],
    systemPrompt: string,
    step: number,
  ): Promise<void> => {
    const maxSteps = maxAgentSteps;
    if (agentAbortRef.current || step >= maxSteps) {
      if (!agentAbortRef.current) {
        chat.addMessage({
          role: "assistant",
          content: locale === "zh-TW"
            ? `（Agent 已達最大步驟數 ${maxSteps}，停止迭代）`
            : `(Agent reached the max step count of ${maxSteps} and stopped)`,
        });
      }
      setAgentRunning(false);
      return;
    }

    setAgentStep(step + 1);

    // Ask AI
    let reply: string;
    try {
      const agentMessages: AiChatMessage[] = [
        { role: "system", content: systemPrompt },
        ...history,
      ];
      const replyObj = await invokeAiChat(agentMessages, sessionId, undefined, false, locale);
      reply = replyObj.content ?? "";
    } catch {
      setAgentRunning(false);
      return;
    }

    if (agentAbortRef.current) { setAgentRunning(false); return; }

    // Show assistant reply in chat
    chat.addMessage({ role: "assistant", content: reply });
    if (sendRemoteResponse) sendRemoteResponse(reply);

    // Parse <cmd>
    const cmdMatch = reply.match(/<cmd>([\s\S]*?)<\/cmd>/i);
    if (!cmdMatch) {
      // No command → agent finished
      setAgentRunning(false);
      return;
    }

    const cmd = cmdMatch[1].trim();

    // Execute and wait for completion
    await new Promise<void>((resolve) => {
      onExecuteCommand(cmd, async (block) => {
        if (agentAbortRef.current) { setAgentRunning(false); resolve(); return; }

        const rawOutput = await getPtyRecentOutput(sessionId).catch(() => null)
          ?? block.output
          ?? "";
        const output = rawOutput.slice(-2000);

        const resultContent =
          `Command \`${cmd}\` finished (exit code ${block.exitCode ?? 0}).\nOutput:\n\`\`\`\n${output}\n\`\`\`\n\nContinue analyzing. If the goal has been achieved, give your final explanation (do not include any more <cmd> tags).`;

        const newHistory = [
          ...history,
          { role: "assistant" as const, content: reply },
          { role: "user" as const, content: resultContent },
        ];

        resolve();
        void runAgentLoopRef.current(newHistory, systemPrompt, step + 1);
      });
    });
  }, [chat, onExecuteCommand, sessionId, locale, sendRemoteResponse, maxAgentSteps]);

  useEffect(() => {
    runAgentLoopRef.current = runAgentLoop;
  }, [runAgentLoop]);

  const submitAgent = useCallback(async (text: string) => {
    setAgentRunning(true);
    setAgentStep(0);
    agentAbortRef.current = false;

    // Carry forward the prior conversation (e.g. a plan the AI already
    // proposed and is waiting on the user to confirm) — read chat.messages
    // BEFORE appending this turn's message so it reflects only prior turns.
    const priorHistory = chat.messages
      .filter((m): m is typeof m & { role: "user" | "assistant" } => m.role === "user" || m.role === "assistant")
      .map((m) => ({ role: m.role, content: contentToDisplayString(m.content) }));

    chat.addMessage({ role: "user", content: text });

    let systemPrompt: string;
    try {
      systemPrompt = await buildAgentSystemPrompt();
    } catch {
      setAgentRunning(false);
      return;
    }

    const history = [...priorHistory, { role: "user" as const, content: text }];
    await runAgentLoop(history, systemPrompt, 0);
  }, [chat, buildAgentSystemPrompt, runAgentLoop]);

  // ── Standard chat ─────────────────────────────────────────────────────────

  useEffect(() => {
    if (isOpen) textareaRef.current?.focus();
  }, [isOpen]);

  useEffect(() => {
    const onPrefill = (e: Event) => {
      const detail = (e as CustomEvent).detail as { text: string };
      if (detail?.text) setInput(detail.text);
    };
    window.addEventListener("aiterm:prefill-chat", onPrefill);
    return () => window.removeEventListener("aiterm:prefill-chat", onPrefill);
  }, []);

  // Forward new assistant messages to Telegram
  const prevMessagesLength = useRef(chat.messages.length);
  useEffect(() => {
    if (chat.messages.length > prevMessagesLength.current) {
      const lastMsg = chat.messages[chat.messages.length - 1];
      if (lastMsg?.role === "assistant" && sendRemoteResponse && !chat.isStreaming) {
        const text = typeof lastMsg.content === "string" ? lastMsg.content : Array.isArray(lastMsg.content) ? lastMsg.content.filter((p) => p.type === "text").map((p) => (p as { type: "text"; text: string }).text).join(" ") : "";
        sendRemoteResponse(text);
      }
    }
    prevMessagesLength.current = chat.messages.length;
  }, [chat.messages, chat.isStreaming, sendRemoteResponse]);

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") { e.preventDefault(); onClose(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter") {
      const shouldSubmit =
        (submitShortcut === "enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) ||
        (submitShortcut === "shift-enter" && e.shiftKey && !e.ctrlKey) ||
        (submitShortcut === "ctrl-enter" && (e.ctrlKey || e.metaKey) && !e.shiftKey);
      if (shouldSubmit) { e.preventDefault(); handleSubmit(); }
    }
  };

  const handleSubmit = () => {
    const text = input.trim();
    if ((!text && attachments.length === 0) || chat.isStreaming || agentRunning) return;
    setInput("");
    const currentAttachments = attachments;
    setAttachments([]);
    if (agentMode) {
      void submitAgent(text);
    } else {
      void chat.send(text, useMcp && mcpEnabled && mcpToolCount > 0, undefined, currentAttachments.length > 0 ? currentAttachments : undefined);
    }
  };

  const panelClass = isOpen ? "aiterm-ai-panel" : "aiterm-ai-panel aiterm-ai-panel-hidden";
  const isDisabled = chat.isStreaming || agentRunning;

  return (
    <div
      className={panelClass}
      aria-hidden={!isOpen}
      style={{ width: `${panelWidth}px` }}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
    >
      {/* Resize handle on the left edge */}
      <div
        className="aiterm-panel-resize-handle"
        onPointerDown={onResizePointerDown}
        onPointerMove={onResizePointerMove}
        onPointerUp={onResizePointerUp}
        title="拖曳調整寬度"
      />

      <div className="aiterm-ai-panel-header">
        <span className="aiterm-ai-panel-title" style={{ background: 'var(--accent-gradient)', WebkitBackgroundClip: 'text', WebkitTextFillColor: 'transparent', display: 'flex', alignItems: 'center', gap: '6px', fontWeight: 700, fontSize: '14px' }}>
          ✨ AITerm AI Studio
        </span>
        <button
          type="button"
          className="aiterm-ai-panel-provider-badge"
          onClick={onOpenProviderPalette}
          title="切換 Provider"
        >
          {providerName || "(no provider)"}
        </button>
        <div style={{ display: "flex", gap: "8px" }}>
          <button
            type="button"
            className={`aiterm-ai-panel-clear-btn${historyOpen ? " aiterm-ai-panel-clear-btn--active" : ""}`}
            onClick={() => setHistoryOpen((o) => !o)}
            title="對話歷史"
          >
            📋
          </button>
          <button
            type="button"
            className="aiterm-ai-panel-clear-btn"
            onClick={() => { chat.clear(); setHistoryOpen(false); }}
            disabled={isDisabled}
            title="清空當前對話"
          >
            🗑 New Chat
          </button>
          <button
            type="button"
            className="aiterm-ai-panel-clear-btn"
            onClick={onClose}
            title="關閉面板 (Esc)"
          >
            ✕
          </button>
        </div>
      </div>

      {/* History side panel */}
      {historyOpen && (
        <div className="aiterm-history-panel">
          <div className="aiterm-history-panel__header">
            <span className="aiterm-history-panel__title">對話歷史</span>
          </div>
          <div className="aiterm-history-panel__list">
            {chat.sessions.length === 0 && (
              <div className="aiterm-history-panel__empty">尚無歷史記錄</div>
            )}
            {[...chat.sessions].reverse().map((s) => (
              <div
                key={s.id}
                className="aiterm-history-panel__item"
                onClick={() => { chat.loadMessages(s.messages, s.id); setHistoryOpen(false); }}
              >
                <div className="aiterm-history-panel__item-content">
                  <div className="aiterm-history-panel__item-title">{s.title}</div>
                  <div className="aiterm-history-panel__item-date">
                    {new Date(s.savedAt).toLocaleDateString("zh-TW", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}
                  </div>
                </div>
                <button
                  type="button"
                  className="aiterm-history-panel__item-del"
                  title="刪除此對話"
                  onClick={(e) => { e.stopPropagation(); chat.deleteSession(s.id); }}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <MessageList
        messages={chat.messages}
        streamBuf={chat.streamBuf}
        isStreaming={chat.isStreaming}
        error={chat.error}
        onExecuteCommand={onExecuteCommand}
        onRetry={chat.resend}
      />

      {agentRunning && (
        <div className="aiterm-agent-status">
          <span className="aiterm-agent-status__spinner">⟳</span>
          <span>Agent 執行中… 步驟 {agentStep}/{maxAgentSteps >= 9999 ? "∞" : maxAgentSteps}</span>
          <button
            type="button"
            className="aiterm-agent-status__stop"
            onClick={() => {
              agentAbortRef.current = true;
              // Send Ctrl+C to PTY so a stuck command (e.g. pipe dquote>) gets
              // interrupted, the prompt reappears, and the onComplete callback
              // can fire to actually unblock the agent loop.
              writePty(sessionId, "\x03").catch(() => {});
            }}
            title="停止"
          >
            ■
          </button>
        </div>
      )}

      {attachments.length > 0 && (
        <div className="aiterm-attachment-pills">
          {attachments.map((att) => (
            <div key={att.id} className="aiterm-attachment-pill">
              {att.previewUrl && (
                <img src={att.previewUrl} alt={att.name} className="aiterm-pill-thumb" />
              )}
              <span className="aiterm-pill-name" title={att.name}>{att.name}</span>
              <button
                type="button"
                className="aiterm-pill-remove"
                onClick={() => setAttachments((prev) => prev.filter((a) => a.id !== att.id))}
              >×</button>
            </div>
          ))}
        </div>
      )}
      <input
        ref={fileInputRef}
        type="file"
        multiple
        style={{ display: "none" }}
        onChange={(e) => { if (e.target.files) void processFiles(e.target.files); }}
      />
      <div className="aiterm-ai-panel-input-area">
        <div style={{ display: "flex", gap: "6px", alignItems: "center" }}>
          <button
            type="button"
            className={`aiterm-agent-toggle${agentMode ? " aiterm-agent-toggle--on" : ""}`}
            onClick={() => setAgentMode((m) => !m)}
            title={agentMode ? "停用 Agent 模式" : "啟用 Agent 模式（AI 自動執行指令迭代）"}
            disabled={isDisabled}
          >
            <ZapIcon size={14} isFilled={agentMode} />
          </button>
          {mcpEnabled && (
            <button
              type="button"
              className={`aiterm-mcp-toggle${useMcp && mcpToolCount > 0 ? " aiterm-mcp-toggle--on" : ""}`}
              title={mcpToolCount === 0 ? t.mcp_toggle_no_servers : (useMcp ? "MCP 開啟" : "MCP 關閉")}
              disabled={mcpToolCount === 0 || isDisabled}
              onClick={() => setUseMcp((v) => !v)}
            >
              <WrenchIcon size={12} />
              <span>{mcpToolCount > 0 ? `MCP (${mcpToolCount})` : "MCP OFF"}</span>
            </button>
          )}
        </div>
        <div className="aiterm-input-pill-container">
          <button
            type="button"
            className="aiterm-pill-paperclip-btn"
            onClick={() => fileInputRef.current?.click()}
            title="附加檔案"
            disabled={isDisabled}
          >
            📎
          </button>
          <textarea
            ref={textareaRef}
            className="aiterm-ai-panel-input"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            placeholder={
              agentRunning ? "Agent 執行中…" :
              agentMode ? "目標... (Enter)" :
              chat.isStreaming ? "等待 AI 回覆..." : "Ask AI anything..."
            }
            rows={1}
            disabled={isDisabled}
          />
          <button
            type="button"
            className="aiterm-ai-panel-send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon"
            onClick={handleSubmit}
            disabled={isDisabled || input.trim() === ""}
            title="送出"
          >
            ▲
          </button>
        </div>
      </div>
    </div>
  );
}
