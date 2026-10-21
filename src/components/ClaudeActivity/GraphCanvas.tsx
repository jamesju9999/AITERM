import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { ActivityNode, ActivityStatus } from "../../ipc/claudeActivity";
import { useLocale } from "../../contexts/LocaleContext";
import { DONE_GROUP_ID } from "./fold";
import { formatDuration, formatTokens } from "./format";
import { ROOT_ID, colX, type Graph } from "./graphLayout";
import { CATEGORY_ICON, isToolId, isToolsMoreId } from "./toolNodes";

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 1.5;
const ZOOM_STEP = 0.1;
const clampZoom = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(z * 100) / 100));

interface Props {
  graph: Graph;
  nodes: ActivityNode[];
  now: number;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  runningCount: number;
  mainTool: string | null;
  mainToolCount: number;
  /** token 最多的節點（有 ≥2 個節點有 token 時才有）；卡片上標「最耗」。 */
  heaviestId: string | null;
  /** 點群組卡（已完成 N 個）：展開被折起來的節點。 */
  onExpandDone: () => void;
  /** 點「更早 N 個工具」群組卡：展開全部工具。 */
  onExpandTools: () => void;
}

/** 到根的祖先鏈（含自己與根）；選中節點時用來高亮路徑。 */
function pathTo(id: string | null, graph: Graph): Set<string> {
  const path = new Set<string>();
  if (!id) return path;
  const parentOf = new Map(graph.edges.map((e) => [e.to, e.from]));
  let cur: string | undefined = id;
  while (cur && !path.has(cur)) {
    path.add(cur);
    cur = parentOf.get(cur);
  }
  return path;
}

export function GraphCanvas({ graph, nodes, now, selectedId, onSelect, runningCount, mainTool, mainToolCount, heaviestId, onExpandDone, onExpandTools }: Props) {
  const { t } = useLocale();
  const uid = useId().replace(/:/g, "");
  const scrollRef = useRef<HTMLDivElement>(null);
  const [zoom, setZoom] = useState(1);
  const byId = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);
  const cols = useMemo(() => new Map(Array.from({ length: 12 }, (_, d) => [colX(d), d] as const)), []);
  const path = useMemo(() => pathTo(selectedId, graph), [selectedId, graph]);
  // 選根只是要看主 Claude 的詳情，不需要把所有 subagent 都淡掉。
  const dimming = selectedId != null && selectedId !== ROOT_ID && path.size > 0;

  const statusLabel: Record<ActivityStatus, string> = {
    running: t.claude_activity_status_running,
    done: t.claude_activity_status_done,
    failed: t.claude_activity_status_failed,
    interrupted: t.claude_activity_status_interrupted,
  };

  // 新出現的執行中節點自動捲進視野（第一次渲染不捲：那是掛載，不是「新的」）。
  const seenRunning = useRef<Set<string> | null>(null);
  useEffect(() => {
    const running = new Set(nodes.filter((n) => n.status === "running").map((n) => n.id));
    const prev = seenRunning.current;
    seenRunning.current = running;
    if (!prev) return;
    const fresh = [...running].find((id) => !prev.has(id));
    if (!fresh) return;
    const el = scrollRef.current?.querySelector<HTMLElement>(`[data-node-id="${fresh}"]`);
    el?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [nodes]);

  const fit = () => {
    const w = scrollRef.current?.clientWidth ?? 0;
    if (w > 0) setZoom(clampZoom(w / graph.width));
  };
  return (
    <>
      <div className="aiterm-cact__zoom" role="group">
        <button type="button" className="aiterm-cact__btn" aria-label={t.claude_activity_zoom_out} onClick={() => setZoom((z) => clampZoom(z - ZOOM_STEP))}>−</button>
        <span className="aiterm-cact__zoom-val" data-testid="zoom-value">{Math.round(zoom * 100)}%</span>
        <button type="button" className="aiterm-cact__btn" aria-label={t.claude_activity_zoom_in} onClick={() => setZoom((z) => clampZoom(z + ZOOM_STEP))}>＋</button>
        <button type="button" className="aiterm-cact__btn" onClick={fit}>{t.claude_activity_zoom_fit}</button>
      </div>
      <div className="aiterm-cact__scroll" ref={scrollRef}>
        <div style={{ width: graph.width * zoom, height: graph.height * zoom }}>
          <div
            className="aiterm-cact__graph"
            data-testid="graph"
            style={{ width: graph.width, height: graph.height, transform: `scale(${zoom})`, transformOrigin: "0 0" }}
          >
            <svg className="aiterm-cact__edges" width={graph.width} height={graph.height} aria-hidden="true">
              <defs>
                <marker id={`${uid}-arrow`} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                  <path d="M0,0 L10,5 L0,10 z" className="aiterm-cact__arrow" />
                </marker>
              </defs>
              {graph.edges.map((e) => {
                const onPath = path.has(e.from) && path.has(e.to);
                return (
                  <g key={`${e.from}>${e.to}`}>
                    <path
                      data-testid={`edge-${e.from}-${e.to}`}
                      data-status={e.status}
                      className={`aiterm-cact__edge aiterm-cact__edge--${e.status}${onPath ? " is-path" : ""}${dimming && !onPath ? " is-dim" : ""}`}
                      d={e.d}
                      markerEnd={`url(#${uid}-arrow)`}
                    />
                    {e.status === "running" && (
                      <circle r="3" className="aiterm-cact__fly" data-testid={`fly-${e.from}-${e.to}`}>
                        <animateMotion dur="1.6s" repeatCount="indefinite" path={e.d} />
                      </circle>
                    )}
                  </g>
                );
              })}
            </svg>
            {[...graph.boxes].map(([id, box]) => {
              const style = { left: box.x, top: box.y, width: box.w, height: box.h };
              const col = id === ROOT_ID ? 0 : cols.get(box.x) ?? 0;
              const dim = dimming && !path.has(id);
              if (id === ROOT_ID) {
                return (
                  <button
                    key={id}
                    type="button"
                    className={`aiterm-cact__card aiterm-cact__card--root${selectedId === id ? " is-selected" : ""}${dim ? " is-dim" : ""}`}
                    style={style}
                    data-node-id={id}
                    data-col={col}
                    aria-pressed={selectedId === id}
                    onClick={() => onSelect(selectedId === id ? null : id)}
                  >
                    <span className="aiterm-cact__card-title">{t.claude_activity_root}</span>
                    <span className="aiterm-cact__card-sub">{t.claude_activity_running(runningCount)}</span>
                    {mainTool && <span className="aiterm-cact__card-line aiterm-cact__card-line--main" title={mainTool}>{mainTool}</span>}
                    {mainToolCount > 0 && <span className="aiterm-cact__card-line">{t.claude_activity_tools(mainToolCount)}</span>}
                  </button>
                );
              }
              const n = byId.get(id);
              if (!n) return null;
              const title = n.description ?? t.claude_activity_untitled;
              if (isToolsMoreId(id)) {
                return (
                  <button
                    key={id}
                    type="button"
                    className={`aiterm-cact__card aiterm-cact__card--group${dim ? " is-dim" : ""}`}
                    style={style}
                    data-node-id={id}
                    data-col={col}
                    onClick={onExpandTools}
                  >
                    <span className="aiterm-cact__card-title">{title}</span>
                  </button>
                );
              }
              if (isToolId(id)) {
                const cat = n.agent_type || "other";
                return (
                  <button
                    key={id}
                    type="button"
                    className={`aiterm-cact__card aiterm-cact__card--tool aiterm-cact__card--cat-${cat}${n.status === "failed" ? " is-failed" : ""}${selectedId === id ? " is-selected" : ""}${path.has(id) && selectedId !== id ? " is-path" : ""}${dim ? " is-dim" : ""}`}
                    style={style}
                    data-node-id={id}
                    data-col={col}
                    aria-pressed={selectedId === id}
                    title={title}
                    onClick={() => onSelect(selectedId === id ? null : id)}
                  >
                    <span className="aiterm-cact__tool-row">
                      <span aria-hidden="true">{CATEGORY_ICON[cat] ?? CATEGORY_ICON.other}</span>
                      <span className={`aiterm-cact__dot aiterm-cact__dot--${n.status}`} role="img" aria-label={statusLabel[n.status]} />
                      <span className="aiterm-cact__tool-text">{title}</span>
                      {n.status !== "done" && <span className="aiterm-cact__tool-state">{statusLabel[n.status]}</span>}
                      <span className="aiterm-cact__tool-dur">{formatDuration((n.ended_ms ?? now) - n.started_ms)}</span>
                    </span>
                  </button>
                );
              }
              if (id === DONE_GROUP_ID) {
                return (
                  <button
                    key={id}
                    type="button"
                    className={`aiterm-cact__card aiterm-cact__card--group${dim ? " is-dim" : ""}`}
                    style={style}
                    data-node-id={id}
                    data-col={col}
                    onClick={onExpandDone}
                  >
                    <span className="aiterm-cact__card-title">{title}</span>
                  </button>
                );
              }
              return (
                <button
                  key={id}
                  type="button"
                  className={`aiterm-cact__card aiterm-cact__card--${n.status}${selectedId === id ? " is-selected" : ""}${path.has(id) && selectedId !== id ? " is-path" : ""}${dim ? " is-dim" : ""}`}
                  style={style}
                  data-node-id={id}
                  data-col={col}
                  aria-pressed={selectedId === id}
                  title={title}
                  onClick={() => onSelect(selectedId === id ? null : id)}
                >
                  <span className="aiterm-cact__card-title">
                    <span className={`aiterm-cact__dot aiterm-cact__dot--${n.status}`} aria-hidden="true" />
                    <span className="aiterm-cact__card-text">{title}</span>
                  </span>
                  <span className="aiterm-cact__card-sub">
                    <span className="aiterm-cact__badge">{n.agent_type}</span>
                    <span>{statusLabel[n.status]}</span>
                    {heaviestId === id && <span className="aiterm-cact__badge aiterm-cact__badge--hot">{t.claude_activity_heaviest}</span>}
                  </span>
                  <span className="aiterm-cact__card-line">
                    {n.status === "running" && n.current_tool
                      ? n.current_tool
                      : [formatDuration((n.ended_ms ?? now) - n.started_ms), n.tokens != null ? t.claude_activity_tokens(formatTokens(n.tokens)) : null]
                          .filter(Boolean)
                          .join(" · ")}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      </div>
    </>
  );
}
