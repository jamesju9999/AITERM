import {
  useEffect, useRef, useState, useCallback,
  type KeyboardEvent, type ReactNode, type PointerEvent, type MouseEvent,
} from "react";
import type { AiError, ToolFallbackReason } from "../../ipc/ai";
import type { SubmitShortcut } from "../../ipc/config";
import type { McpChatMessage, McpChatSession } from "../../types/chat";
import { useLocale } from "../../contexts/LocaleContext";
import { MessageList } from "../AiPanel/MessageList";
import { ModeHint, type PanelMode } from "../AiPanel/ModeHint";
import { MaximizeIcon, MinimizeIcon, ZapIcon } from "../Icons";
import { ArtifactPanelProvider, useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactPanel } from "../ArtifactPanel/ArtifactPanel";
import "./styles.css";

const MIN_WIDTH = 280;
const MAX_WIDTH_RATIO = 0.75;
const STORAGE_WIDTH_KEY = "aiterm-panel-width";
const MIN_CHAT_COLUMN_WIDTH = 220;
const MIN_ARTIFACT_COLUMN_WIDTH = 260;

function loadSavedWidth(): number {
  try {
    const v = localStorage.getItem(STORAGE_WIDTH_KEY);
    if (v) return Math.max(MIN_WIDTH, parseInt(v, 10));
  } catch { /* ignore */ }
  return 420;
}

export interface ChatPanelShellProps {
  isOpen: boolean;
  onClose: () => void;

  messages: McpChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  thinkingLabel: string | null;
  error: AiError | string | null;
  onRetry: () => void;
  onExecuteCommand: (cmd: string) => void;

  agentMode: boolean;
  onToggleAgentMode: () => void;
  onSend: (text: string) => void;
  onSubmitAgent: (text: string) => void;
  mode: PanelMode;
  maxAgentSteps: number;
  mcpToolCount?: number;

  agentRunning: boolean;
  agentPhase: "thinking" | "running";
  agentStep: number;
  onAbortAgent: () => void;

  providerName: string;
  onOpenProviderPalette: () => void;
  /** provider 徽章旁的插槽——呼叫端應該塞 <QuotaBadge> 進來（AiPanel 就是這樣接的）。 */
  headerBadge?: ReactNode;

  sessions: McpChatSession[];
  onLoadSession: (s: McpChatSession) => void;
  onNewChat: () => void;
  onDeleteSession: (id: string) => void;

  toolFallbackReason?: ToolFallbackReason | null;

  /** 送出鍵的觸發鍵——設定裡的 submit_shortcut，預設 Enter 送出。 */
  submitShortcut?: SubmitShortcut;
  /** 文字是空的也允許送出（例如本機面板：只有附件、沒打字也能送）。 */
  allowEmptySubmit?: boolean;
  /** 貼上事件轉發給呼叫端（例如本機面板用它接住剪貼簿裡的檔案）。 */
  onPaste?: (e: React.ClipboardEvent<HTMLTextAreaElement>) => void;
  /** 輸入框前面的額外控制項（例如本機面板的附加檔案迴紋針按鈕）。 */
  inputPrefixControls?: ReactNode;

  /** 貼在 agent-mode 開關旁那一排（輸入框上方）——AiPanel 把 MCP 開關放這裡。
   *  跟 `inputPrefixControls`（貼在輸入框「裡面」，textarea 左側）是不同位置，別搞混。 */
  extraInputControls?: ReactNode;
  /** 插在整個輸入區塊「之上」（ModeHint／agent 狀態列之後、輸入框之前）——
   *  AiPanel 放附件 pills、隱藏的檔案 input、卡住提示。 */
  extraAboveInput?: ReactNode;
  /** Windows 上背景毛玻璃看不清楚，要改成不透明樣式（見 styles.css）；預設 false。 */
  isWindows?: boolean;
  /** 外部強制禁用輸入區（例如唯讀連線）；跟既有的 isStreaming/agentRunning
   *  禁用邏輯是 OR 關係——三者任一為真就禁用，預設 false 不影響原本行為。 */
  inputDisabled?: boolean;

  onDragOver?: (e: React.DragEvent<HTMLDivElement>) => void;
  onDrop?: (e: React.DragEvent<HTMLDivElement>) => void;
}

/** 對外仍然只有一個 ChatPanelShell——ArtifactPanelProvider 包在這一層，讓每個
 *  分頁各自的 ChatPanelShell 實例都有自己獨立的 artifact 狀態（見
 *  docs/superpowers/specs/2026-09-01-artifact-panel-design.md「per-tab」段落）。
 *  Provider 一定要包在「消費它的元件」外面，所以實際內容拆到 Inner 元件。 */
export function ChatPanelShell(props: ChatPanelShellProps) {
  return (
    <ArtifactPanelProvider>
      <ChatPanelShellInner {...props} />
    </ArtifactPanelProvider>
  );
}

function ChatPanelShellInner({
  isOpen,
  onClose,
  messages,
  streamBuf,
  isStreaming,
  thinkingLabel,
  error,
  onRetry,
  onExecuteCommand,
  agentMode,
  onToggleAgentMode,
  onSend,
  onSubmitAgent,
  mode,
  maxAgentSteps,
  mcpToolCount = 0,
  agentRunning,
  agentPhase,
  agentStep,
  onAbortAgent,
  providerName,
  onOpenProviderPalette,
  headerBadge,
  sessions,
  onLoadSession,
  onNewChat,
  onDeleteSession,
  toolFallbackReason,
  submitShortcut = "enter",
  allowEmptySubmit = false,
  onPaste,
  inputPrefixControls,
  extraInputControls,
  extraAboveInput,
  isWindows = false,
  inputDisabled = false,
  onDragOver,
  onDrop,
}: ChatPanelShellProps) {
  const { t } = useLocale();
  const { activeArtifact } = useArtifactPanel();
  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);

  // 有 artifact 顯示時，視覺上一律當成「已展開」——不是把 expanded 這個
  // state 本身改掉（使用者手動按過的展開偏好要保留），只是讓寬度/樣式判斷
  // 多一個 OR 條件。artifact 收掉後會自動回到使用者原本手動設定的展開狀態。
  const effectiveExpanded = expanded || !!activeArtifact;

  // ── Resize（面板整體寬度，收合時） ─────────────────────────────────────────
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

  // ── Resize（聊天欄 vs Artifact 面板的內部分割，有 artifact 時） ───────────────
  // 做法比照 src/components/DesignView/DesignView.tsx 既有的手刻拖拉分割：
  // 用容器的 getBoundingClientRect() 算出滑鼠絕對位置對應的左欄寬度，而不是
  // 累加 delta——這樣邏輯簡單、也是這個 repo 既有分割版型的一致寫法。
  const [chatColumnWidth, setChatColumnWidth] = useState(320);
  const [isArtifactResizing, setIsArtifactResizing] = useState(false);
  const splitContainerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!isArtifactResizing) {
      document.body.style.userSelect = "";
      return;
    }
    document.body.style.userSelect = "none";
    const onMouseMove = (e: globalThis.MouseEvent) => {
      if (!splitContainerRef.current) return;
      const rect = splitContainerRef.current.getBoundingClientRect();
      const newWidth = e.clientX - rect.left;
      const constrained = Math.max(
        MIN_CHAT_COLUMN_WIDTH,
        Math.min(newWidth, rect.width - MIN_ARTIFACT_COLUMN_WIDTH),
      );
      setChatColumnWidth(constrained);
    };
    const onMouseUp = () => setIsArtifactResizing(false);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      document.body.style.userSelect = "";
    };
  }, [isArtifactResizing]);

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

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") { e.preventDefault(); onClose(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);

  const isDisabled = isStreaming || agentRunning || inputDisabled;

  const submit = useCallback(() => {
    const text = input.trim();
    if ((!text && !allowEmptySubmit) || isDisabled) return;
    setInput("");
    if (agentMode) {
      onSubmitAgent(text);
    } else {
      onSend(text);
    }
  }, [input, allowEmptySubmit, isDisabled, agentMode, onSubmitAgent, onSend]);

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter") {
      const shouldSubmit =
        (submitShortcut === "enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) ||
        (submitShortcut === "shift-enter" && e.shiftKey && !e.ctrlKey) ||
        (submitShortcut === "ctrl-enter" && (e.ctrlKey || e.metaKey) && !e.shiftKey);
      if (shouldSubmit) { e.preventDefault(); submit(); }
    }
  };

  const panelClass = [
    "aiterm-ai-panel",
    isOpen ? "" : "aiterm-ai-panel-hidden",
    // Windows can't blur the terminal behind the glass panel — see styles.css.
    isWindows ? "aiterm-ai-panel--solid" : "",
    effectiveExpanded ? "aiterm-ai-panel--expanded" : "",
    activeArtifact ? "aiterm-ai-panel--split" : "",
  ].filter(Boolean).join(" ");

  const onArtifactResizeMouseDown = (e: MouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsArtifactResizing(true);
  };

  return (
    <div
      className={panelClass}
      aria-hidden={!isOpen}
      style={{ width: effectiveExpanded ? "100%" : `${panelWidth}px` }}
      onDragOver={onDragOver}
      onDrop={onDrop}
      ref={splitContainerRef}
    >
      {/* Resize handle on the left edge — 滿版時左邊沒有終端機可以讓，收起來。 */}
      {!effectiveExpanded && (
        <div
          className="aiterm-panel-resize-handle"
          onPointerDown={onResizePointerDown}
          onPointerMove={onResizePointerMove}
          onPointerUp={onResizePointerUp}
          title="拖曳調整寬度"
        />
      )}

      <div
        className="aiterm-ai-panel-chat-column"
        style={activeArtifact ? { width: `${chatColumnWidth}px`, flexShrink: 0, flexGrow: 0 } : { flex: 1 }}
      >
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
            {headerBadge}
          </button>
          <div style={{ display: "flex", gap: "8px" }}>
            <button
              type="button"
              className={`aiterm-ai-panel-clear-btn aiterm-ai-panel-icon-btn${effectiveExpanded ? " aiterm-ai-panel-clear-btn--active" : ""}`}
              onClick={() => setExpanded((e) => !e)}
              title={effectiveExpanded ? "縮小面板" : "放大面板"}
            >
              {effectiveExpanded ? <MinimizeIcon size={15} /> : <MaximizeIcon size={15} />}
            </button>
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
              onClick={() => { onNewChat(); setHistoryOpen(false); }}
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
              {sessions.length === 0 && (
                <div className="aiterm-history-panel__empty">尚無歷史記錄</div>
              )}
              {[...sessions].reverse().map((s) => (
                <div
                  key={s.id}
                  className="aiterm-history-panel__item"
                  onClick={() => { onLoadSession(s); setHistoryOpen(false); }}
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
                    onClick={(e) => { e.stopPropagation(); onDeleteSession(s.id); }}
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}

        <MessageList
          messages={messages}
          streamBuf={streamBuf}
          isStreaming={isStreaming}
          thinkingLabel={thinkingLabel}
          error={error}
          onExecuteCommand={onExecuteCommand}
          onRetry={onRetry}
        />

        {/* 這個憑證無法使用原生工具呼叫時，後端會自動改用「工具描述注入系統提示」
            的文字協定。工具照樣能跑，但切換方案不該靜默發生。 */}
        {toolFallbackReason && (
          <div className="aiterm-mode-hint aiterm-mode-hint--degraded">
            <span aria-hidden="true">⚠</span>
            <span>
              {toolFallbackReason === "subscription_billing"
                ? t.ai_tool_fallback_billing
                : t.ai_tool_fallback_unsupported}
            </span>
          </div>
        )}

        {/* Agent 跑起來之後由狀態列接手（它有步驟數與中止鈕），兩條堆在一起是噪音。 */}
        {!agentRunning && (
          <ModeHint mode={mode} maxAgentSteps={maxAgentSteps} mcpToolCount={mcpToolCount} />
        )}

        {agentRunning && (
          <div className="aiterm-agent-status">
            <span
              className={`aiterm-agent-status__spinner aiterm-agent-status__spinner--${agentPhase}`}
              aria-hidden="true"
            >
              {agentPhase === "thinking" ? "⟳" : "▶"}
            </span>
            <span>
              {agentPhase === "thinking" ? t.ai_agent_thinking : t.ai_agent_executing}
              {" "}步驟 {agentStep}/{maxAgentSteps >= 9999 ? "∞" : maxAgentSteps}
            </span>
            <button
              type="button"
              className="aiterm-agent-status__stop"
              onClick={onAbortAgent}
              title="停止"
            >
              ■
            </button>
          </div>
        )}

        {extraAboveInput}

        <div className="aiterm-ai-panel-input-area">
          <div style={{ display: "flex", gap: "6px", alignItems: "center" }}>
            <button
              type="button"
              className={`aiterm-agent-toggle${agentMode ? " aiterm-agent-toggle--on" : ""}`}
              onClick={onToggleAgentMode}
              title={agentMode ? "停用 Agent 模式" : "啟用 Agent 模式（AI 自動執行指令迭代）"}
              disabled={isDisabled}
            >
              <ZapIcon size={14} isFilled={agentMode} />
            </button>
            {extraInputControls}
          </div>
          <div className="aiterm-input-pill-container">
            {inputPrefixControls}
            <textarea
              ref={textareaRef}
              className="aiterm-ai-panel-input"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              onPaste={onPaste}
              placeholder={
                agentRunning ? "Agent 執行中…" :
                agentMode ? "目標... (Enter)" :
                isStreaming ? "等待 AI 回覆..." : "Ask AI anything..."
              }
              rows={1}
              disabled={isDisabled}
            />
            <button
              type="button"
              className="aiterm-ai-panel-send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon"
              onClick={submit}
              disabled={isDisabled || input.trim() === ""}
              title="送出"
            >
              ▲
            </button>
          </div>
        </div>
      </div>

      {activeArtifact && (
        <>
          <div
            className="aiterm-artifact-resizer"
            onMouseDown={onArtifactResizeMouseDown}
            title="拖曳調整寬度"
          />
          <ArtifactPanel />
        </>
      )}
    </div>
  );
}
