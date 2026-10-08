import { memo, useEffect, useState } from "react";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";
import { useLocale } from "../contexts/LocaleContext";
import "./TerminalBlockCard.css";

/** 「已加入書籤」就地回饋顯示多久。夠看清楚、又不會久到擋住再次操作。 */
const FLASH_MS = 1500;

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
  /** 回傳 false＝這個指令本來就在書籤裡（沒有新增）；其餘（含 void）視為新增成功。 */
  onBookmark?: (command: string) => boolean | void;
  /** 可回傳 Promise：reject 時按鈕顯示「複製失敗」。 */
  onCopy?: (command: string) => void | Promise<void>;
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

function lineText(line: { spans: { text: string }[] }): string {
  return line.spans.map((s) => s.text).join("");
}

function TerminalBlockCardImpl({ block, highlightQuery, onAskAi, onBookmark, onCopy }: TerminalBlockCardProps) {
  const { t } = useLocale();
  const [collapsed, setCollapsed] = useState(false);
  const [expanded, setExpanded] = useState(false);
  // 書籤按鈕就地回饋：按鈕文字短暫變成「✓ 已加入書籤」，不彈窗、不佔版面、
  // 不擋任何操作。
  const [bookmarkFlash, setBookmarkFlash] = useState<"added" | "exists" | null>(null);
  const [copyFlash, setCopyFlash] = useState<"done" | "failed" | null>(null);
  useEffect(() => {
    if (!bookmarkFlash) return;
    const id = setTimeout(() => setBookmarkFlash(null), FLASH_MS);
    return () => clearTimeout(id);
  }, [bookmarkFlash]);
  useEffect(() => {
    if (!copyFlash) return;
    const id = setTimeout(() => setCopyFlash(null), FLASH_MS);
    return () => clearTimeout(id);
  }, [copyFlash]);
  const handleCopy = (command: string) => {
    void Promise.resolve()
      .then(() => onCopy?.(command))
      .then(() => setCopyFlash("done"))
      .catch((e) => { console.error(e); setCopyFlash("failed"); });
  };
  // running 中的卡片跟 Warp 一樣顯示持續跳動的耗時（見設計文件
  // 2026-09-16-live-block-rendering-design.md）——running 中沒有
  // `endTime`，靠這個 tick 強制重新渲染讓 `formatDuration(Date.now() -
  // startTime)` 讀到新值，不需要真的把耗時存進 state。
  const [, forceTick] = useState(0);
  useEffect(() => {
    if (block.status !== "running") return;
    const interval = setInterval(() => forceTick((t) => t + 1), 200);
    return () => clearInterval(interval);
  }, [block.status]);

  // 沒有 OSC 133 的 shell（舊版 host、ssh 進 NAS）沒有真的結束碼：靠畫面判斷結案
  // （exitUnknown），或下一個指令送出時被強制結案（exitCode -1 這個哨兵值）。
  // 這兩種都不是「指令失敗」，不能顯示成紅色 exit -1。
  const exitIsUnknown = block.exitUnknown === true || block.exitCode === -1;
  const allLines = block.renderedLines ?? [];
  // 同一批沒有 C 標記的 shell 也不會讓輸出起點跳過回顯，卡片第一列會是指令自己
  // 又印一次——跟上面的指令列重複。只在結束碼未知時剝掉，有 OSC 133 的卡片不動。
  const lines =
    exitIsUnknown && allLines.length > 0 && lineText(allLines[0]).trim() === block.command.trim()
      ? allLines.slice(1)
      : allLines;
  const isTruncated = !expanded && lines.length > MAX_VISIBLE_LINES;
  const visibleLines = isTruncated ? lines.slice(0, MAX_VISIBLE_LINES) : lines;
  const hiddenCount = lines.length - MAX_VISIBLE_LINES;

  const duration = block.endTime
    ? formatDuration(block.endTime - block.startTime)
    : block.status === "running"
      ? formatDuration(Date.now() - block.startTime)
      : undefined;
  // NOTE: deliberately keyed off `status` rather than `exitCode !== 0` — a running
  // block has `exitCode === undefined`, and `undefined !== 0` is true, which would
  // otherwise mislabel in-flight blocks as failed (red styling, "exit undefined"
  // text, and a premature "Ask AI" button).
  const isFailed = block.status === "failed" && !exitIsUnknown;
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
            <button
              className={`aiterm-block-btn aiterm-btn aiterm-btn--secondary aiterm-block-btn--bookmark${bookmarkFlash ? " is-done" : ""}`}
              onClick={() => setBookmarkFlash(onBookmark(block.command) === false ? "exists" : "added")}
              aria-live="polite"
            >
              {bookmarkFlash === "added" ? t.block_bookmarked : bookmarkFlash === "exists" ? t.block_bookmark_exists : "Bookmark"}
            </button>
          )}
          {onCopy && (
            <button
              className={`aiterm-block-btn aiterm-btn aiterm-btn--secondary aiterm-block-btn--copy${copyFlash === "done" ? " is-done" : ""}`}
              onClick={() => handleCopy(block.command)}
              aria-live="polite"
            >
              {copyFlash === "done" ? t.block_copied : copyFlash === "failed" ? t.block_copy_failed : "Copy"}
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
