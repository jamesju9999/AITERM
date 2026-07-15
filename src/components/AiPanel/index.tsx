import {
  useEffect, useRef, useState, useCallback,
  type KeyboardEvent, type PointerEvent,
} from "react";
import { readFileAsAttachment } from "../../types/attachment";
import type { Attachment } from "../../types/attachment";
import { useMcpChat } from "../../hooks/useMcpChat";
import { invokeAiChat, type ChatMessage as AiChatMessage } from "../../ipc/ai";
import { getSessionCwd, listDirectory } from "../../ipc/fs";
import { getPtyRecentOutput, writePty } from "../../ipc/pty";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { getMcpTools } from "../../ipc/mcp";
import { useLocale } from "../../contexts/LocaleContext";
import type { TerminalBlock } from "../../hooks/useTerminalBlocks";
import { MessageList } from "./MessageList";
import "./styles.css";

const MIN_WIDTH = 280;
const MAX_WIDTH_RATIO = 0.75;
const STORAGE_WIDTH_KEY = "aiterm-panel-width";

function loadSavedWidth(): number {
  try {
    const v = localStorage.getItem(STORAGE_WIDTH_KEY);
    if (v) return Math.max(MIN_WIDTH, parseInt(v, 10));
  } catch { /* ignore */ }
  return 420;
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
  const [historyOpen, setHistoryOpen] = useState(false);
  const [mcpEnabled, setMcpEnabled] = useState(true);
  const [mcpToolCount, setMcpToolCount] = useState(0);
  const [useMcp, setUseMcp] = useState(true);
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
  const [agentMode, setAgentMode] = useState(false);
  const [agentRunning, setAgentRunning] = useState(false);
  const [agentStep, setAgentStep] = useState(0);
  const agentAbortRef = useRef(false);
  const maxAgentStepsRef = useRef<number>(5);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      const [cfg, tools] = await Promise.all([getConfig(), getMcpTools()]);
      if (cancelled) return;
      // 0 = unlimited; use a large number internally
      maxAgentStepsRef.current = cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5);
      const globalEnabled = cfg.mcp_enabled ?? true;
      setMcpEnabled(globalEnabled);
      setMcpToolCount(tools.length);
      setSubmitShortcut(cfg.submit_shortcut ?? "enter");
    };
    load().catch(() => {});
    return () => { cancelled = true; };
  }, []);

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

    return `你是一個終端機 Agent，可透過 <cmd>...</cmd> 標籤執行 shell 指令，並根據結果迭代完成使用者的目標。

目前工作目錄：${cwd}
目錄內容（前 60 個項目）：
${dirList || "（無法取得）"}

規則：
1. 需要執行指令時，使用 <cmd>shell 指令</cmd>（每次只給一個）。
2. 系統會自動執行並將結果回傳，請繼續分析直到目標完成。
3. 目標完成後，用繁體中文給出最終說明，不要再給 <cmd> 標籤。
4. 不要執行破壞性或不可逆的操作（如 rm -rf /）。
5. 所有說明使用繁體中文。`;
  }, [sessionId]);

  /**
   * Recursive agent loop. Each call:
   * 1. Asks AI with current history
   * 2. If AI returns a <cmd>, execute it and recurse with the output
   * 3. If no <cmd>, loop ends
   */
  const runAgentLoop = useCallback(async (
    history: { role: "user" | "assistant"; content: string }[],
    systemPrompt: string,
    step: number,
  ): Promise<void> => {
    const maxSteps = maxAgentStepsRef.current;
    if (agentAbortRef.current || step >= maxSteps) {
      if (!agentAbortRef.current) {
        chat.addMessage({ role: "assistant", content: `（Agent 已達最大步驟數 ${maxSteps}，停止迭代）` });
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
          `指令 \`${cmd}\` 執行完成（退出碼 ${block.exitCode ?? 0}）。\n輸出：\n\`\`\`\n${output}\n\`\`\`\n\n請繼續分析。若目標已達成，請給出最終說明（不要再給 <cmd> 標籤）。`;

        const newHistory = [
          ...history,
          { role: "assistant" as const, content: reply },
          { role: "user" as const, content: resultContent },
        ];

        resolve();
        void runAgentLoop(newHistory, systemPrompt, step + 1);
      });
    });
  }, [chat, onExecuteCommand, sessionId, locale]);

  const submitAgent = useCallback(async (text: string) => {
    setAgentRunning(true);
    setAgentStep(0);
    agentAbortRef.current = false;

    chat.addMessage({ role: "user", content: text });

    let systemPrompt: string;
    try {
      systemPrompt = await buildAgentSystemPrompt();
    } catch {
      setAgentRunning(false);
      return;
    }

    const history = [{ role: "user" as const, content: text }];
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
        <span className="aiterm-ai-panel-title">AI Chat</span>
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
          <span>Agent 執行中… 步驟 {agentStep}/{maxAgentStepsRef.current >= 9999 ? "∞" : maxAgentStepsRef.current}</span>
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
          >
            ■ 停止
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
      <div className="aiterm-ai-panel-input-area">
        <button
          type="button"
          className={`aiterm-agent-toggle${agentMode ? " aiterm-agent-toggle--on" : ""}`}
          onClick={() => setAgentMode((m) => !m)}
          title={agentMode ? "停用 Agent 模式" : "啟用 Agent 模式（AI 自動執行指令迭代）"}
          disabled={isDisabled}
        >
          ⚡
        </button>
        {mcpEnabled && (
          <button
            type="button"
            title={mcpToolCount === 0 ? t.mcp_toggle_no_servers : (useMcp ? "MCP 開啟" : "MCP 關閉")}
            disabled={mcpToolCount === 0 || isDisabled}
            onClick={() => setUseMcp((v) => !v)}
            style={{
              fontSize: 11, padding: "2px 8px", borderRadius: 4,
              border: `1px solid ${useMcp && mcpToolCount > 0 ? "#34d399" : "#333"}`,
              background: useMcp && mcpToolCount > 0 ? "#0f2e23" : "transparent",
              color: useMcp && mcpToolCount > 0 ? "#34d399" : "#666",
              cursor: mcpToolCount === 0 ? "default" : "pointer",
              opacity: mcpToolCount === 0 ? 0.5 : 1,
            }}
          >
            {mcpToolCount > 0 ? t.mcp_toggle_on(mcpToolCount) : t.mcp_toggle_off}
          </button>
        )}
        <textarea
          ref={textareaRef}
          className="aiterm-ai-panel-input"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          placeholder={
            agentRunning ? "Agent 執行中…" :
            agentMode ? "輸入目標，Agent 將自動執行指令… (Enter)" :
            chat.isStreaming ? "等待 AI 回覆中..." : "輸入訊息，Enter 送出..."
          }
          rows={2}
          disabled={isDisabled}
        />
        <button
          type="button"
          className="aiterm-ai-panel-send-btn"
          onClick={handleSubmit}
          disabled={isDisabled || input.trim() === ""}
        >
          送出
        </button>
      </div>
    </div>
  );
}
