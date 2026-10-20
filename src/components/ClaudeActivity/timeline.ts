import type { ActivityNode } from "../../ipc/claudeActivity";

export interface TimelineRow {
  node: ActivityNode;
  depth: number;
  /** 條的左緣與寬度，佔整條時間軸的百分比（0–100）。 */
  left: number;
  width: number;
}
export interface Timeline {
  rows: TimelineRow[];
  /** 最早開始到最晚結束（執行中的算到現在）的毫秒數。 */
  spanMs: number;
}

/** 太短的條仍要看得見。 */
const MIN_WIDTH = 1.5;

/** 每個 subagent 一條橫條，依開始時間排序，看誰跟誰同時跑、誰最慢。 */
export function buildTimeline(nodes: ActivityNode[], now: number): Timeline {
  if (nodes.length === 0) return { rows: [], spanMs: 0 };
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const depthOf = (n: ActivityNode): number => {
    let d = 0;
    const seen = new Set<string>([n.id]);
    let cur = n;
    while (cur.parent && byId.has(cur.parent) && !seen.has(cur.parent)) {
      cur = byId.get(cur.parent)!;
      seen.add(cur.id);
      d += 1;
    }
    return d;
  };
  const endOf = (n: ActivityNode) => Math.max(n.ended_ms ?? now, n.started_ms);
  const start = Math.min(...nodes.map((n) => n.started_ms));
  const span = Math.max(1, Math.max(...nodes.map(endOf)) - start);
  const rows = [...nodes]
    .sort((a, b) => a.started_ms - b.started_ms || a.id.localeCompare(b.id))
    .map((node) => {
      const left = ((node.started_ms - start) / span) * 100;
      const width = Math.max(MIN_WIDTH, ((endOf(node) - node.started_ms) / span) * 100);
      return { node, depth: depthOf(node), left: Math.min(left, 100 - MIN_WIDTH), width: Math.min(width, 100 - Math.min(left, 100 - MIN_WIDTH)) };
    });
  return { rows, spanMs: span };
}
