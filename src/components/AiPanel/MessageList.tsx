// src/components/AiPanel/MessageList.tsx
import { useEffect, useRef, useState } from "react";
import type { AiError } from "../../ipc/ai";
import { formatAiError } from "../../ipc/ai";
import type { McpChatMessage } from "../../hooks/useMcpChat";
import { MessageBubble } from "./MessageBubble";
import { useLocale } from "../../contexts/LocaleContext";

interface MessageListProps {
  messages: McpChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  error: AiError | string | null;
  onExecuteCommand: (cmd: string) => void;
  onRetry: () => void;
}

function formatError(error: AiError | string | null): string {
  if (!error) return "";
  if (typeof error === "string") return error;
  return formatAiError(error);
}

function ToolCallCard({
  callMsg,
  resultMsg,
}: {
  callMsg: McpChatMessage;
  resultMsg: McpChatMessage | undefined;
}) {
  const { t } = useLocale();
  const [expanded, setExpanded] = useState(false);
  const toolDisplayName = callMsg.tool_name?.includes("__")
    ? callMsg.tool_name.split("__").slice(1).join("__")
    : (callMsg.tool_name ?? "tool");

  const isLoading = callMsg.is_loading;
  const isError = resultMsg?.is_error ?? callMsg.is_error;
  const hasResult = !!resultMsg && !isLoading;

  return (
    <div className={`aiterm-tool-card${isError ? " aiterm-tool-card--error" : ""}`}>
      <button
        type="button"
        className="aiterm-tool-card-header"
        onClick={() => setExpanded(e => !e)}
      >
        <span className="aiterm-tool-card-icon">⚙</span>
        <span className="aiterm-tool-card-name">{toolDisplayName}</span>
        <span className="aiterm-tool-card-status">
          {isLoading && <span className="aiterm-tool-spinner">⟳</span>}
          {!isLoading && hasResult && !isError && "✓"}
          {!isLoading && isError && "✗"}
        </span>
        <span className="aiterm-tool-card-chevron">{expanded ? "▲" : "▼"}</span>
      </button>
      {expanded && (
        <div className="aiterm-tool-card-body">
          <div className="aiterm-tool-card-section">
            <div className="aiterm-tool-card-label">{t.tool_call_input}</div>
            <pre className="aiterm-tool-card-content">{typeof callMsg.content === "string" ? callMsg.content : ""}</pre>
          </div>
          {hasResult && (
            <div className="aiterm-tool-card-section">
              <div className="aiterm-tool-card-label">{t.tool_call_output}</div>
              <pre className="aiterm-tool-card-content">{typeof resultMsg!.content === "string" ? resultMsg!.content : ""}</pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function MessageList({
  messages,
  streamBuf,
  isStreaming,
  error,
  onExecuteCommand,
  onRetry,
}: MessageListProps) {
  const { t } = useLocale();
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, streamBuf, error]);

  // Build a map of tool_call_id → tool_result message for card merging
  const resultMap = new Map<string, McpChatMessage>();
  for (const m of messages) {
    if (m.role === "tool_result" && m.tool_call_id) {
      resultMap.set(m.tool_call_id, m);
    }
  }

  return (
    <div ref={listRef} className="aiterm-message-list">
      {messages.map((m, i) => {
        if (m.role === "tool_result") {
          // Rendered as part of the tool_call card above — skip
          return null;
        }
        if (m.role === "tool_call") {
          const result = m.tool_call_id ? resultMap.get(m.tool_call_id) : undefined;
          return (
            <ToolCallCard key={i} callMsg={m} resultMsg={result} />
          );
        }
        return (
          <MessageBubble
            key={i}
            role={m.role === "assistant" ? "assistant" : "user"}
            content={m.content}
            onExecuteCommand={onExecuteCommand}
          />
        );
      })}
      {isStreaming && streamBuf && (
        <MessageBubble
          role="assistant"
          content={streamBuf}
          onExecuteCommand={onExecuteCommand}
          streaming
        />
      )}
      {error && (
        <div className="aiterm-bubble aiterm-bubble-error" role="alert">
          <span>⚠ {formatError(error)}</span>
          <button
            type="button"
            className="aiterm-retry-btn"
            onClick={onRetry}
            disabled={isStreaming}
          >
            {t.ai_retry}
          </button>
        </div>
      )}
    </div>
  );
}
