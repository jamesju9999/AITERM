import type { TaskRow } from "../../ipc/tasks";
import { waitingPrerequisites } from "./prerequisites";

/**
 * 排隊中的卡片為什麼還沒開工：每張都在等別位工程師的前置模組（deps），還是前置都好了、
 * 只是全域並行上限沒空位（slot）。前置沒做完時說「等待全域名額」會誤導使用者去調上限。
 * 前置規則沿用 `waitingPrerequisites`（與後端 `blocked_cards` 相同）。
 */
export function waitingReason(queued: TaskRow[], all: TaskRow[], hasReviewer: boolean): "deps" | "slot" {
  if (queued.length === 0) return "slot";
  return queued.every((c) => waitingPrerequisites(c, all, hasReviewer).length > 0) ? "deps" : "slot";
}
