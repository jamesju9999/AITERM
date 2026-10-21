import { useLayoutEffect, useState, type RefObject } from "react";

import type { AgentRow } from "../../ipc/agents";
import { cometDuration, edgePath, type Box, type Edge, type Pulse } from "./edges";

/** 彗星：一顆亮點拖著逐漸變小、變淡的尾巴。尾巴是同一條路上晚幾格出發的小點。 */
const TAIL = 8;
const TAIL_STEP = 0.045; // 尾巴每一點比前一點晚幾秒（佔一圈的比例，見 Comet）
const COMETS_PER_EDGE = 2;


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
  dispatchHint,
}: {
  containerRef: RefObject<HTMLElement | null>;
  /** 節點內容一變（卡片狀態變了）高度就可能變——用它觸發重新量測。 */
  revision: unknown;
  agents: AgentRow[];
  edges: Edge[];
  active: Map<string, string>;
  pulses: Pulse[];
  /** 滑過派工虛線時的說明：派工是系統決定的，不是協調者。 */
  dispatchHint: string;
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
        <marker id="ac-arrow-dispatch" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M0,0 L10,5 L0,10 z" fill="#7c3aed" />
        </marker>
      </defs>
      {edges.map((e) => {
        const a = box(e.from);
        const b = box(e.to);
        if (!a || !b) return null;
        const d = edgePath(a, b);
        const color = active.get(`${e.from}>${e.to}`);
        return (
          <g key={`${e.from}>${e.to}`} data-testid={`edge-${e.from}-${e.to}`} data-active={color ?? ""} data-kind={e.kind ?? "flow"}>
            <path
              // 派工虛線只在協調者真的有東西派出去時顯眼；做完就淡到幾乎看不見，免得跟交件實線打架。
              className={`ac-edge-base${e.kind === "dispatch" ? ` ac-edge-base--dispatch${color ? "" : " ac-edge-base--idle"}` : ""}`}
              d={d}
              markerEnd={e.kind === "dispatch" ? "url(#ac-arrow-dispatch)" : "url(#ac-arrow)"}
            />
            {e.kind === "dispatch" && (
              <path className="ac-edge-hit" d={d}>
                <title>{dispatchHint}</title>
              </path>
            )}
            {color && (
              <>
                <path className="ac-edge-glow" d={d} stroke={color} />
                <path className="ac-edge-flow" d={d} stroke={color} />
                {Array.from({ length: COMETS_PER_EDGE }, (_, k) => (
                  <Comet key={k} d={d} color={color} dur={cometDuration(a, b)} phase={k / COMETS_PER_EDGE} />
                ))}
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


/**
 * 沿著 `d` 流動的彗星。每個點都是 animateMotion，用負的 begin 讓它們一開始就各就各位
 * （正的 begin 在出發前會畫在 SVG 原點）；越靠尾巴越晚出發，所以落在彗頭後面。
 * `phase`（0–1）把幾顆彗星錯開，同一條線上不會同時擠在一起。
 */
function Comet({ d, color, dur, phase }: { d: string; color: string; dur: number; phase: number }) {
  return (
    <g className="ac-comet" fill={color} style={{ color }}>
      {Array.from({ length: TAIL }, (_, i) => {
        const lag = i * TAIL_STEP * dur; // 這一點落後彗頭多少秒
        const begin = -(phase * dur + (TAIL * TAIL_STEP * dur - lag));
        const t = i / (TAIL - 1);
        return (
          <circle key={i} r={i === 0 ? 3.6 : 3.2 * (1 - t * 0.7)} opacity={i === 0 ? 1 : 0.55 * (1 - t) ** 1.4}>
            <animateMotion dur={`${dur}s`} begin={`${begin}s`} repeatCount="indefinite" path={d} calcMode="linear" />
          </circle>
        );
      })}
    </g>
  );
}
