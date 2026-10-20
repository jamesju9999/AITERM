import { useEffect, useMemo, useState } from "react";
import type { ActivityNode, ActivityStatus } from "../../ipc/claudeActivity";
import { useLocale } from "../../contexts/LocaleContext";
import { flattenTree } from "./flattenTree";
import { formatDuration, formatTokens } from "./format";
import "./ClaudeActivityPanel.css";

const COLLAPSED_KEY = "aiterm-claude-activity-collapsed";

function loadCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

/** 有執行中的節點時每秒更新一次，讓耗時跳動；全部結束就停。 */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active]);
  return now;
}

interface Props {
  nodes: ActivityNode[];
  /** true＝獨佔側欄整欄；false＝與建議側欄上下並排時只佔上半部（有上限、可捲動）。 */
  fill: boolean;
}

export function ClaudeActivityPanel({ nodes, fill }: Props) {
  const { t } = useLocale();
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const [openResults, setOpenResults] = useState<Set<string>>(() => new Set());
  const rows = useMemo(() => flattenTree(nodes), [nodes]);
  const running = nodes.filter((n) => n.status === "running").length;
  const now = useNow(running > 0);

  const toggleCollapsed = () => {
    const next = !collapsed;
    setCollapsed(next);
    try { localStorage.setItem(COLLAPSED_KEY, next ? "1" : "0"); } catch { /* ignore */ }
  };
  const toggleResult = (id: string) =>
    setOpenResults((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const statusLabel: Record<ActivityStatus, string> = {
    running: t.claude_activity_status_running,
    done: t.claude_activity_status_done,
    failed: t.claude_activity_status_failed,
    interrupted: t.claude_activity_status_interrupted,
  };

  return (
    <section
      className={`aiterm-cact${fill ? " aiterm-cact--fill" : ""}${collapsed ? " aiterm-cact--collapsed" : ""}`}
      aria-label={t.claude_activity_title}
    >
      <header className="aiterm-cact__header">
        <span className="aiterm-cact__title">{t.claude_activity_title}</span>
        <span className="aiterm-cact__count">{t.claude_activity_running(running)}</span>
        <span className="aiterm-cact__count">{t.claude_activity_total(nodes.length)}</span>
        <button
          type="button"
          className="aiterm-cact__toggle"
          aria-expanded={!collapsed}
          onClick={toggleCollapsed}
        >
          {collapsed ? t.claude_activity_expand : t.claude_activity_collapse}
        </button>
      </header>
      {!collapsed && (
        <ul className="aiterm-cact__list">
          {rows.map(({ node, depth }) => {
            const elapsed = (node.ended_ms ?? now) - node.started_ms;
            const open = openResults.has(node.id);
            return (
              <li
                key={node.id}
                className={`aiterm-cact__row aiterm-cact__row--${node.status}`}
                data-depth={depth}
                style={{ paddingLeft: `${8 + depth * 14}px` }}
              >
                <div className="aiterm-cact__line">
                  <span className={`aiterm-cact__dot aiterm-cact__dot--${node.status}`} aria-hidden="true" />
                  <span className="aiterm-cact__desc">{node.description ?? t.claude_activity_untitled}</span>
                  <span className="aiterm-cact__badge">{node.agent_type}</span>
                  <span className="aiterm-cact__status">{statusLabel[node.status]}</span>
                </div>
                {node.status === "running" && node.current_tool && (
                  <div className="aiterm-cact__tool">{node.current_tool}</div>
                )}
                <div className="aiterm-cact__meta">
                  <span>{formatDuration(elapsed)}</span>
                  {node.tokens != null && <span>{t.claude_activity_tokens(formatTokens(node.tokens))}</span>}
                  {node.tool_count != null && <span>{t.claude_activity_tools(node.tool_count)}</span>}
                </div>
                {node.summary && (
                  <>
                    <button type="button" className="aiterm-cact__result-btn" onClick={() => toggleResult(node.id)}>
                      {open ? t.claude_activity_hide_result : t.claude_activity_show_result}
                    </button>
                    {open && <p className="aiterm-cact__summary">{node.summary}</p>}
                  </>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
