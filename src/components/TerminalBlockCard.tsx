import { memo, useState } from "react";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";
import "./TerminalBlockCard.css";

const MAX_VISIBLE_LINES = 500;

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const minutes = Math.floor(seconds / 60);
  const remSeconds = Math.round(seconds % 60);
  return `${minutes}m${remSeconds}s`;
}

function shortenCwd(cwd?: string): string {
  if (!cwd) return "";
  const parts = cwd.split("/");
  return parts.length > 2 ? `.../${parts.slice(-2).join("/")}` : cwd;
}

export interface TerminalBlockCardProps {
  block: TerminalBlock;
  highlightQuery?: string;
  onAskAi?: (command: string, exitCode: number | undefined) => void;
  onBookmark?: (command: string) => void;
  onCopy?: (command: string) => void;
}

function highlightText(text: string, query?: string): React.ReactNode {
  if (!query) return text;
  const idx = text.toLowerCase().indexOf(query.toLowerCase());
  if (idx === -1) return text;
  return (
    <>
      {text.slice(0, idx)}
      <mark>{text.slice(idx, idx + query.length)}</mark>
      {text.slice(idx + query.length)}
    </>
  );
}

function TerminalBlockCardImpl({ block, highlightQuery, onAskAi, onBookmark, onCopy }: TerminalBlockCardProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [expanded, setExpanded] = useState(false);

  const lines = block.renderedLines ?? [];
  const isTruncated = !expanded && lines.length > MAX_VISIBLE_LINES;
  const visibleLines = isTruncated ? lines.slice(0, MAX_VISIBLE_LINES) : lines;
  const hiddenCount = lines.length - MAX_VISIBLE_LINES;

  const duration = block.endTime ? formatDuration(block.endTime - block.startTime) : undefined;
  // NOTE: deliberately keyed off `status` rather than `exitCode !== 0` — a running
  // block has `exitCode === undefined`, and `undefined !== 0` is true, which would
  // otherwise mislabel in-flight blocks as failed (red styling, "exit undefined"
  // text, and a premature "Ask AI" button).
  const isFailed = block.status === "failed";
  const exitClass = isFailed ? "aiterm-block-exit-fail" : "aiterm-block-exit-ok";

  return (
    <div className={`aiterm-block-card ${isFailed ? "aiterm-block-card--failed" : ""}`}>
      <div className="aiterm-block-header" data-testid="block-header" onClick={() => setCollapsed((c) => !c)}>
        <span className="aiterm-block-cwd" title={block.cwd}>{shortenCwd(block.cwd)}</span>
        {block.gitInfo && (
          <span className="aiterm-block-git">
            git:({block.gitInfo.branch})
            {(block.gitInfo.insertions > 0 || block.gitInfo.deletions > 0) && (
              <>
                {" "}
                <span className="aiterm-block-git-add">+{block.gitInfo.insertions}</span>{" "}
                <span className="aiterm-block-git-del">-{block.gitInfo.deletions}</span>
              </>
            )}
          </span>
        )}
        {duration && <span className="aiterm-block-duration">({duration})</span>}
        <span className={exitClass}>{isFailed && block.exitCode !== undefined ? `exit ${block.exitCode}` : ""}</span>
        <div className="aiterm-block-card__actions" onClick={(e) => e.stopPropagation()}>
          {isFailed && onAskAi && (
            <button className="aiterm-block-btn aiterm-btn aiterm-btn--secondary" onClick={() => onAskAi(block.command, block.exitCode)}>
              ✨ Ask AI
            </button>
          )}
          {onBookmark && (
            <button className="aiterm-block-btn aiterm-btn aiterm-btn--secondary" onClick={() => onBookmark(block.command)}>
              Bookmark
            </button>
          )}
          {onCopy && (
            <button className="aiterm-block-btn aiterm-btn aiterm-btn--secondary" onClick={() => onCopy(block.command)}>
              Copy
            </button>
          )}
        </div>
      </div>
      <div className="aiterm-block-command">{highlightText(block.command, highlightQuery)}</div>
      {!collapsed && (
        <div className="aiterm-block-body" data-testid="block-body">
          <pre>
            {visibleLines.map((line, i) => (
              <div key={i} className="aiterm-block-line">
                {line.spans.map((span, j) => (
                  <span
                    key={j}
                    style={{
                      color: span.fg,
                      backgroundColor: span.bg,
                      fontWeight: span.bold ? "bold" : undefined,
                      fontStyle: span.italic ? "italic" : undefined,
                      textDecoration: span.underline ? "underline" : undefined,
                    }}
                  >
                    {highlightText(span.text, highlightQuery)}
                  </span>
                ))}
              </div>
            ))}
          </pre>
          {isTruncated && (
            <button className="aiterm-block-expand aiterm-btn aiterm-btn--secondary" data-testid="block-expand" onClick={() => setExpanded(true)}>
              顯示完整輸出（還有 {hiddenCount} 行）
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Memoized so a completed block's card doesn't re-render on every PTY output
 * chunk from an unrelated, currently-running sibling block in the parent's
 * `blocks.map(...)` list (only the running block's own object reference
 * changes on each chunk).
 */
export const TerminalBlockCard = memo(TerminalBlockCardImpl);
