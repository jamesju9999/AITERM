import type { Architecture, Edge } from "./model";
import type { TourStep } from "./qa";

/** 自動播放每步停留毫秒數。 */
export const autoplayDelayMs = 3000;

export interface TourRoute {
  /** 要高亮的連線：目前模組到下一步模組之間，沿路每一段的所有連線（不分方向）。 */
  edges: Edge[];
  /** edges 之中，連線方向與行進方向相反的那幾條（資料往回走，箭頭與光點要倒過來）。 */
  reversed: Set<Edge>;
  /** 路上經過的模組（含起訖）；沒有路時為空。 */
  modules: string[];
  /** 兩步的模組在圖上完全不相通（例如 AI 分析沒給任何連線）時，起訖模組；畫面改畫一條「導覽推論」的臨時虛線，這一步才看得出流向。 */
  gap?: { from: string; to: string };
}

const NO_ROUTE: TourRoute = { edges: [], reversed: new Set(), modules: [] };

/**
 * 第 index 步到下一步之間的路線。兩個模組有直接連線就只畫那一段；
 * 沒有（例如領域模型→前端，中間隔著 API 與後端）就沿既有連線找最短的一條，把沿途每一段都亮起來。
 * 最後一步、同一模組、或兩者在圖上完全不相通時為空。
 */
export function tourRoute(view: Architecture, steps: TourStep[], index: number): TourRoute {
  const cur = steps[index];
  const next = steps[index + 1];
  if (!cur || !next || cur.module === next.module) return NO_ROUTE;
  // 廣度優先找最短路（不分方向）；直接連線就是長度 1 的路。
  const prev = new Map<string, string>([[cur.module, ""]]);
  const queue = [cur.module];
  for (let q = 0; q < queue.length && !prev.has(next.module); q++) {
    const u = queue[q];
    for (const e of view.edges) {
      const v = e.from === u ? e.to : e.to === u ? e.from : null;
      if (v === null || prev.has(v)) continue;
      prev.set(v, u);
      queue.push(v);
    }
  }
  if (!prev.has(next.module)) return { ...NO_ROUTE, gap: { from: cur.module, to: next.module } };
  const modules: string[] = [];
  for (let m = next.module; m !== ""; m = prev.get(m)!) modules.unshift(m);
  const edges: Edge[] = [];
  const reversed = new Set<Edge>();
  for (let i = 0; i + 1 < modules.length; i++) {
    const u = modules[i];
    const v = modules[i + 1];
    for (const e of view.edges) {
      if (e.from === u && e.to === v) edges.push(e);
      else if (e.from === v && e.to === u) { edges.push(e); reversed.add(e); }
    }
  }
  return { edges, reversed, modules };
}

/** 第 index 步與下一步模組之間要高亮的連線（見 tourRoute）。 */
export function tourEdges(view: Architecture, steps: TourStep[], index: number): Edge[] {
  return tourRoute(view, steps, index).edges;
}

/** tourEdges 之中，連線方向與行進方向相反的那幾條。 */
export function tourReversedEdges(view: Architecture, steps: TourStep[], index: number): Set<Edge> {
  return tourRoute(view, steps, index).reversed;
}

export function stepBounds(steps: TourStep[], index: number): { first: boolean; last: boolean } {
  return { first: index <= 0, last: index >= steps.length - 1 };
}

export function clampStep(steps: TourStep[], index: number): number {
  return Math.max(0, Math.min(index, steps.length - 1));
}
