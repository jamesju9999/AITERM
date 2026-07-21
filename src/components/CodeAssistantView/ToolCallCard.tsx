import { useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { ToolCallState } from "../../hooks/useCodeAssistant";

interface ToolCallCardProps {
  toolCall: ToolCallState;
}

function formatArgs(args: Record<string, unknown>): string {
  const entries = Object.entries(args ?? {});
  if (entries.length === 0) return "";
  const str = (v: unknown): string =>
    v !== null && typeof v === "object" ? JSON.stringify(v) : String(v ?? "");
  if (entries.length === 1) return str(entries[0][1]);
  return entries.map(([k, v]) => `${k}=${str(v)}`).join(", ");
}

export function ToolCallCard({ toolCall }: ToolCallCardProps) {
  const { t } = useLocale();
  const [expanded, setExpanded] = useState(false);
  const isDone = toolCall.result !== undefined;
  const isError = isDone && toolCall.result!.content.startsWith("Error:");

  return (
    <div className={`ca-tool-card ${isDone ? "ca-tool-card--done" : "ca-tool-card--loading"} ${isError ? "ca-tool-card--error" : ""}`}>
      <button
        className="ca-tool-card__header"
        onClick={() => isDone && setExpanded((v) => !v)}
        aria-expanded={expanded}
      >
        <span className="ca-tool-card__status">
          {!isDone && <span className="ca-tool-card__spinner" />}
          {isDone && !isError && "✓"}
          {isError && "✗"}
        </span>
        <span className="ca-tool-card__name">{toolCall.tool}</span>
        <span className="ca-tool-card__args">{formatArgs(toolCall.args)}</span>
        {isDone && (
          <span className="ca-tool-card__toggle">{expanded ? "▲" : "▼"}</span>
        )}
      </button>
      {!isDone && toolCall.progress && (
        <div className="ca-tool-card__progress">{toolCall.progress}</div>
      )}
      {expanded && isDone && (
        <div className="ca-tool-card__content">
          {toolCall.result!.truncated && (
            <div className="ca-tool-card__truncated">{t.ca_tool_truncated}</div>
          )}
          <pre className="ca-tool-card__pre">{toolCall.result!.content}</pre>
        </div>
      )}
    </div>
  );
}
