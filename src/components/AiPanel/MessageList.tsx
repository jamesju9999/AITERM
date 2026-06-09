import { useEffect, useRef } from "react";
import type { AiError, ChatMessage, ContentPart } from "../../ipc/ai";
import { formatAiError } from "../../ipc/ai";

function contentToString(content: string | ContentPart[]): string {
  if (typeof content === "string") return content;
  return content
    .filter((p): p is Extract<ContentPart, { type: "text" }> => p.type === "text")
    .map((p) => p.text).join(" ");
}
import { MessageBubble } from "./MessageBubble";

interface MessageListProps {
  messages: ChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  error: AiError | null;
  onExecuteCommand: (cmd: string) => void;
  onRetry: () => void;
}

export function MessageList({
  messages,
  streamBuf,
  isStreaming,
  error,
  onExecuteCommand,
  onRetry,
}: MessageListProps) {
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, streamBuf, error]);

  return (
    <div ref={listRef} className="aiterm-message-list">
      {messages.map((m, i) => (
        <MessageBubble
          key={i}
          role={m.role === "assistant" ? "assistant" : "user"}
          content={contentToString(m.content)}
          onExecuteCommand={onExecuteCommand}
        />
      ))}
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
          <span>⚠ {formatAiError(error)}</span>
          <button
            type="button"
            className="aiterm-retry-btn"
            onClick={onRetry}
            disabled={isStreaming}
          >
            🔄 重試
          </button>
        </div>
      )}
    </div>
  );
}
