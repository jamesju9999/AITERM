import { useEffect, useRef } from "react";
import type { AiError, ChatMessage } from "../../ipc/ai";
import { formatAiError } from "../../ipc/ai";
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
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, streamBuf, error]);

  return (
    <div className="aiterm-message-list">
      {messages.map((m, i) => (
        <MessageBubble
          key={i}
          role={m.role === "assistant" ? "assistant" : "user"}
          content={m.content}
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
      <div ref={bottomRef} />
    </div>
  );
}
