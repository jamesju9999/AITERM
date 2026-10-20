import type { ActivityNode, ActivitySnapshot } from "../../ipc/claudeActivity";

/** running 節點超過這麼久沒有任何事件就算卡住。 */
export const STUCK_MS = 3 * 60_000;
/** 失敗警示只在剛失敗時發；太久以前的失敗（重新掛載讀到的舊快照）不發。 */
const FRESH_MS = 60_000;

export type Alert =
  | { kind: "failed"; id: string; description: string | null }
  | { kind: "all_done"; count: number; failed: number };

export function detectTransitions(prev: ActivitySnapshot, next: ActivitySnapshot, now: number): Alert[] {
  const wasRunning = prev.nodes.some((n) => n.status === "running");
  const nowRunning = next.nodes.some((n) => n.status === "running");

  if (wasRunning && !nowRunning && next.nodes.length > 0) {
    const turnNodes = next.nodes.filter((n) => n.turn === next.turn);
    const nodes = turnNodes.length > 0 ? turnNodes : next.nodes;
    return [{ kind: "all_done", count: nodes.length, failed: nodes.filter((n) => n.status === "failed").length }];
  }

  const before = new Map(prev.nodes.map((n) => [n.id, n.status]));
  return next.nodes
    .filter((n) => n.status === "failed" && before.get(n.id) !== "failed" && now - (n.ended_ms ?? now) < FRESH_MS)
    .map((n) => ({ kind: "failed" as const, id: n.id, description: n.description }));
}

export function findStuck(nodes: ActivityNode[], now: number, notified: ReadonlySet<string>): ActivityNode[] {
  return nodes.filter((n) => n.status === "running" && now - n.last_event_ms >= STUCK_MS && !notified.has(n.id));
}
