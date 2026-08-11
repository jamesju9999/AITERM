// src/components/AiPanel/MessageList.tsx
import { useEffect, useRef, useState } from "react";
import type { AiError } from "../../ipc/ai";
import { formatAiError } from "../../ipc/ai";
import type { McpChatMessage } from "../../hooks/useMcpChat";
import { MessageBubble } from "./MessageBubble";
import { truncateAtCmdTag } from "../../lib/cmdParser";
import { useLocale } from "../../contexts/LocaleContext";

interface MessageListProps {
  messages: McpChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  /** 忙碌指示的文案；`null` 代表不顯示。
   *
   *  用文案而不是布林，是因為 Agent 迴圈有兩個等待階段（等 AI 想、等指令跑完），
   *  兩段都要有指示、但要講不同的話。只用布林的話，執行指令那段對話框會完全
   *  安靜——實測回報「氣泡消失到下一則訊息之間的空檔很長」就是那一段。 */
  thinkingLabel?: string | null;
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
  thinkingLabel = null,
  error,
  onExecuteCommand,
  onRetry,
}: MessageListProps) {
  const { t } = useLocale();
  const listRef = useRef<HTMLDivElement>(null);
  // 串流途中不顯示 <cmd>——它會在最終訊息裡以 CmdTag 卡片出現。
  const streamPreview = truncateAtCmdTag(streamBuf);

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
      {/* 還沒有任何文字之前的等待指示。
          沒有它的話，從送出到第一個字之間畫面完全空白——使用者合理地以為
          沒在運作。這段空窗在 ChatGPT Web 這條路徑上特別長：要先跑 sentinel
          的兩段握手與工作量證明，模型才開始吐字。
          有字之後就交給下面的串流氣泡，不要兩個同時出現。 */}
      {(thinkingLabel || isStreaming) && !streamPreview && (
        <div className="aiterm-bubble aiterm-bubble-assistant aiterm-thinking" aria-busy="true">
          <span className="aiterm-thinking-dots" aria-hidden="true">
            <i />
            <i />
            <i />
          </span>
          <span className="aiterm-thinking-label">{thinkingLabel ?? t.ai_thinking}</span>
        </div>
      )}
      {isStreaming && streamPreview && (
        <MessageBubble
          role="assistant"
          content={streamPreview}
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
