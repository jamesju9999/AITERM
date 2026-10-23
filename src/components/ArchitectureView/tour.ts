import type { Architecture, Edge } from "./model";
import type { TourStep } from "./qa";

/** 自動播放每步停留毫秒數。 */
export const autoplayDelayMs = 3000;

/** 第 index 步與下一步模組之間要高亮的連線（不分方向，多條全回）；最後一步、同一模組或沒有直接連線時為空。 */
export function tourEdges(view: Architecture, steps: TourStep[], index: number): Edge[] {
  const cur = steps[index];
  const next = steps[index + 1];
  if (!cur || !next || cur.module === next.module) return [];
  return view.edges.filter(
    (e) => (e.from === cur.module && e.to === next.module) || (e.from === next.module && e.to === cur.module),
  );
}

/**
 * tourEdges 之中，連線方向與這一步的行進方向（目前模組→下一步模組）相反的那幾條。
 * 例如「控制器回傳給前端」：連線是前端→控制器（呼叫方向），這一步卻是資料往回走，畫面上要把箭頭與光點倒過來。
 */
export function tourReversedEdges(view: Architecture, steps: TourStep[], index: number): Set<Edge> {
  const cur = steps[index];
  const out = new Set<Edge>();
  if (!cur) return out;
  for (const e of tourEdges(view, steps, index)) if (e.from !== cur.module) out.add(e);
  return out;
}

export function stepBounds(steps: TourStep[], index: number): { first: boolean; last: boolean } {
  return { first: index <= 0, last: index >= steps.length - 1 };
}

export function clampStep(steps: TourStep[], index: number): number {
  return Math.max(0, Math.min(index, steps.length - 1));
}
