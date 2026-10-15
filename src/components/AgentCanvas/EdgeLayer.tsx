import { useLayoutEffect, useState, type RefObject } from "react";

import type { AgentRow } from "../../ipc/agents";
import { edgePath, type Box, type Edge, type Pulse } from "./edges";

/**
 * 桌布上的連線層。節點尺寸要從 DOM 量（內容多寡會改變高度），所以每次
 * 重繪後量一次，有變才更新——不會無限迴圈。
 */
export function EdgeLayer({
  containerRef,
  agents,
  edges,
  active,
  pulses,
  revision,
}: {
  containerRef: RefObject<HTMLElement | null>;
  /** 節點內容一變（卡片狀態變了）高度就可能變——用它觸發重新量測。 */
  revision: unknown;
  agents: AgentRow[];
  edges: Edge[];
  active: Map<string, string>;
  pulses: Pulse[];
}) {
  const [sizes, setSizes] = useState<Record<string, { w: number; h: number }>>({});

  useLayoutEffect(() => {
    const root = containerRef.current;
    if (!root) return;
    const next: Record<string, { w: number; h: number }> = {};
    let changed = false;
    for (const a of agents) {
      const el = root.querySelector<HTMLElement>(`[data-agent-id="${a.id}"]`);
      const w = el?.offsetWidth || 248;
      const h = el?.offsetHeight || 120;
      next[a.id] = { w, h };
      if (sizes[a.id]?.w !== w || sizes[a.id]?.h !== h) changed = true;
    }
    if (changed || Object.keys(sizes).length !== agents.length) setSizes(next);
  }, [containerRef, agents, revision, sizes]);

  const box = (id: string): Box | null => {
    const a = agents.find((x) => x.id === id);
    if (!a) return null;
    const s = sizes[id] ?? { w: 248, h: 120 };
    return { x: a.canvas_x, y: a.canvas_y, w: s.w, h: s.h };
  };

  return (
    <svg className="ac-edges" aria-hidden>
      <defs>
        <filter id="ac-blur" x="-20%" y="-20%" width="140%" height="140%">
          <feGaussianBlur stdDeviation="3" />
        </filter>
        <marker id="ac-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M0,0 L10,5 L0,10 z" fill="#4b5563" />
        </marker>
      </defs>
      {edges.map((e) => {
        const a = box(e.from);
        const b = box(e.to);
        if (!a || !b) return null;
        const d = edgePath(a, b);
        const color = active.get(`${e.from}>${e.to}`);
        return (
          <g key={`${e.from}>${e.to}`} data-testid={`edge-${e.from}-${e.to}`} data-active={color ?? ""}>
            <path className="ac-edge-base" d={d} markerEnd="url(#ac-arrow)" />
            {color && (
              <>
                <path className="ac-edge-glow" d={d} stroke={color} />
                <path className="ac-edge-flow" d={d} stroke={color} />
              </>
            )}
          </g>
        );
      })}
      {pulses.map((p) => {
        const a = box(p.from);
        const b = box(p.to);
        if (!a || !b) return null;
        const d = edgePath(a, b);
        return (
          <circle key={p.key} r={5} fill={p.color} className="ac-pulse">
            <animateMotion
              dur="1.1s"
              fill="freeze"
              path={d}
              keyPoints={p.reverse ? "1;0" : "0;1"}
              keyTimes="0;1"
              calcMode="linear"
            />
          </circle>
        );
      })}
    </svg>
  );
}
