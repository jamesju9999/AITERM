import { useEffect, useId, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import type { ActivityNode, ActivityStatus } from "../../ipc/claudeActivity";
import { useLocale } from "../../contexts/LocaleContext";
import { formatDuration, formatTokens } from "./format";
import { CARD_H, CARD_W, GAP_X, PAD, ROOT_ID, layoutGraph, type Graph } from "./graphLayout";
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

interface GraphProps {
  graph: Graph;
  nodes: ActivityNode[];
  now: number;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  runningCount: number;
}

/** 流程圖本體＋選中節點的詳情。側欄與「放大」覆蓋層共用。 */
function GraphView({ graph, nodes, now, selectedId, onSelect, runningCount }: GraphProps) {
  const { t } = useLocale();
  const uid = useId().replace(/:/g, "");
  const byId = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);
  const selected = selectedId ? byId.get(selectedId) ?? null : null;

  const statusLabel: Record<ActivityStatus, string> = {
    running: t.claude_activity_status_running,
    done: t.claude_activity_status_done,
    failed: t.claude_activity_status_failed,
    interrupted: t.claude_activity_status_interrupted,
  };
  const elapsedOf = (n: ActivityNode) => formatDuration((n.ended_ms ?? now) - n.started_ms);
  const titleOf = (n: ActivityNode) => n.description ?? t.claude_activity_untitled;

  return (
    <>
      <div className="aiterm-cact__scroll">
        <div className="aiterm-cact__graph" style={{ width: graph.width, height: graph.height }}>
          <svg className="aiterm-cact__edges" width={graph.width} height={graph.height} aria-hidden="true">
            <defs>
              <marker id={`${uid}-arrow`} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                <path d="M0,0 L10,5 L0,10 z" className="aiterm-cact__arrow" />
              </marker>
            </defs>
            {graph.edges.map((e) => (
              <path
                key={`${e.from}>${e.to}`}
                data-testid={`edge-${e.from}-${e.to}`}
                data-status={e.status}
                className={`aiterm-cact__edge aiterm-cact__edge--${e.status}`}
                d={e.d}
                markerEnd={`url(#${uid}-arrow)`}
              />
            ))}
          </svg>
          {[...graph.boxes].map(([id, box]) => {
            const style = { left: box.x, top: box.y, width: CARD_W, height: CARD_H };
            const col = Math.round((box.x - PAD) / (CARD_W + GAP_X));
            if (id === ROOT_ID) {
              return (
                <div key={id} className="aiterm-cact__card aiterm-cact__card--root" style={style} data-node-id={id} data-col={col}>
                  <span className="aiterm-cact__card-title">{t.claude_activity_root}</span>
                  <span className="aiterm-cact__card-sub">
                    {t.claude_activity_running(runningCount)} · {t.claude_activity_total(nodes.length)}
                  </span>
                </div>
              );
            }
            const n = byId.get(id);
            if (!n) return null;
            return (
              <button
                key={id}
                type="button"
                className={`aiterm-cact__card aiterm-cact__card--${n.status}${selectedId === id ? " is-selected" : ""}`}
                style={style}
                data-node-id={id}
                data-col={col}
                aria-pressed={selectedId === id}
                onClick={() => onSelect(selectedId === id ? null : id)}
              >
                <span className="aiterm-cact__card-title">
                  <span className={`aiterm-cact__dot aiterm-cact__dot--${n.status}`} aria-hidden="true" />
                  <span className="aiterm-cact__card-text">{titleOf(n)}</span>
                </span>
                <span className="aiterm-cact__card-sub">
                  <span className="aiterm-cact__badge">{n.agent_type}</span>
                  <span>{statusLabel[n.status]}</span>
                </span>
                <span className="aiterm-cact__card-line">
                  {n.status === "running" && n.current_tool
                    ? n.current_tool
                    : [elapsedOf(n), n.tokens != null ? t.claude_activity_tokens(formatTokens(n.tokens)) : null]
                        .filter(Boolean)
                        .join(" · ")}
                </span>
              </button>
            );
          })}
        </div>
      </div>
      <div className="aiterm-cact__detail">
        {selected ? (
          <>
            <div className="aiterm-cact__detail-title">{titleOf(selected)}</div>
            <div className="aiterm-cact__meta">
              <span className="aiterm-cact__badge">{selected.agent_type}</span>
              <span>{statusLabel[selected.status]}</span>
              <span>{elapsedOf(selected)}</span>
              {selected.tokens != null && <span>{t.claude_activity_tokens(formatTokens(selected.tokens))}</span>}
              {selected.tool_count != null && <span>{t.claude_activity_tools(selected.tool_count)}</span>}
            </div>
            {selected.status === "running" && selected.current_tool && (
              <div className="aiterm-cact__tool">{selected.current_tool}</div>
            )}
            {selected.summary && <p className="aiterm-cact__summary">{selected.summary}</p>}
          </>
        ) : (
          <div className="aiterm-cact__hint">{t.claude_activity_pick}</div>
        )}
      </div>
    </>
  );
}

interface Props {
  nodes: ActivityNode[];
  /** true＝獨佔側欄整欄；false＝與建議側欄上下並排時只佔上半部（有上限）。 */
  fill: boolean;
}

export function ClaudeActivityPanel({ nodes, fill }: Props) {
  const { t } = useLocale();
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const [enlarged, setEnlarged] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const graph = useMemo(() => layoutGraph(nodes), [nodes]);
  const running = nodes.filter((n) => n.status === "running").length;
  const now = useNow(running > 0);

  useEffect(() => {
    if (!enlarged) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setEnlarged(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [enlarged]);

  const toggleCollapsed = () => {
    const next = !collapsed;
    setCollapsed(next);
    try { localStorage.setItem(COLLAPSED_KEY, next ? "1" : "0"); } catch { /* ignore */ }
  };

  const view = (
    <GraphView graph={graph} nodes={nodes} now={now} selectedId={selectedId} onSelect={setSelectedId} runningCount={running} />
  );

  return (
    <section
      className={`aiterm-cact${fill ? " aiterm-cact--fill" : ""}${collapsed ? " aiterm-cact--collapsed" : ""}`}
      aria-label={t.claude_activity_title}
    >
      <header className="aiterm-cact__header">
        <span className="aiterm-cact__title">{t.claude_activity_title}</span>
        <span className="aiterm-cact__count">{t.claude_activity_running(running)}</span>
        <span className="aiterm-cact__count">{t.claude_activity_total(nodes.length)}</span>
        <button type="button" className="aiterm-cact__btn aiterm-cact__btn--first" onClick={() => setEnlarged(true)}>
          {t.claude_activity_enlarge}
        </button>
        <button type="button" className="aiterm-cact__btn" aria-expanded={!collapsed} onClick={toggleCollapsed}>
          {collapsed ? t.claude_activity_expand : t.claude_activity_collapse}
        </button>
      </header>
      {!collapsed && view}
      {enlarged &&
        createPortal(
          <div className="aiterm-cact-overlay" onClick={() => setEnlarged(false)}>
            <div
              className="aiterm-cact aiterm-cact--fill aiterm-cact--overlay"
              role="dialog"
              aria-label={t.claude_activity_title}
              onClick={(e) => e.stopPropagation()}
            >
              <header className="aiterm-cact__header">
                <span className="aiterm-cact__title">{t.claude_activity_title}</span>
                <span className="aiterm-cact__count">{t.claude_activity_running(running)}</span>
                <span className="aiterm-cact__count">{t.claude_activity_total(nodes.length)}</span>
                <button type="button" className="aiterm-cact__btn aiterm-cact__btn--first" onClick={() => setEnlarged(false)}>
                  {t.claude_activity_close}
                </button>
              </header>
              {view}
            </div>
          </div>,
          document.body,
        )}
    </section>
  );
}
