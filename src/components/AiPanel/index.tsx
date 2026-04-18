import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useAiChat } from "../../hooks/useAiChat";
import { MessageList } from "./MessageList";
import "./styles.css";

export interface AiPanelProps {
  sessionId: string;
  isOpen: boolean;
  providerName: string;
  onClose: () => void;
  onExecuteCommand: (cmd: string) => void;
  onOpenProviderPalette: () => void;
}

/**
 * The panel stays mounted across open/close so `useAiChat`'s event listener
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
}: AiPanelProps) {
  const chat = useAiChat(sessionId);
  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Autofocus when the panel transitions to open.
  useEffect(() => {
    if (isOpen) textareaRef.current?.focus();
  }, [isOpen]);

  // Listen for prefill events to inject context from error blocks.
  useEffect(() => {
    const onPrefill = (e: Event) => {
      const detail = (e as CustomEvent).detail as { text: string };
      if (detail && detail.text) {
        setInput(detail.text);
      }
    };
    window.addEventListener("aiterm:prefill-chat", onPrefill);
    return () => window.removeEventListener("aiterm:prefill-chat", onPrefill);
  }, []);

  // Global Escape handler — only active while the panel is open.
  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey) {
      e.preventDefault();
      submit();
    }
  };

  const submit = () => {
    const text = input.trim();
    if (!text) return;
    if (chat.isStreaming) return;
    setInput("");
    void chat.send(text);
  };

  const panelClass = isOpen
    ? "aiterm-ai-panel"
    : "aiterm-ai-panel aiterm-ai-panel-hidden";

  return (
    <div className={panelClass} aria-hidden={!isOpen}>
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
        <div style={{ display: 'flex', gap: '8px' }}>
          <button
            type="button"
            className="aiterm-ai-panel-clear-btn"
            onClick={chat.clear}
            disabled={chat.isStreaming}
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

      <MessageList
        messages={chat.messages}
        streamBuf={chat.streamBuf}
        isStreaming={chat.isStreaming}
        error={chat.error}
        onExecuteCommand={onExecuteCommand}
        onRetry={chat.resend}
      />

      <div className="aiterm-ai-panel-input-area">
        <textarea
          ref={textareaRef}
          className="aiterm-ai-panel-input"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={
            chat.isStreaming ? "等待 AI 回覆中..." : "輸入訊息，Enter 送出..."
          }
          rows={2}
          disabled={chat.isStreaming}
        />
        <button
          type="button"
          className="aiterm-ai-panel-send-btn"
          onClick={submit}
          disabled={chat.isStreaming || input.trim() === ""}
        >
          送出
        </button>
      </div>
    </div>
  );
}
