import type { TaskRow } from "../../ipc/tasks";
import { deriveModuleState } from "./deriveModuleState";

/** `depends_on`／`premerged` 這類 JSON 陣列字串。沒有或壞掉都當作空的。 */
export function idList(json: string | null | undefined): string[] {
  if (!json) return [];
  try {
    const v: unknown = JSON.parse(json);
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

/**
 * 一張排隊中的卡還在等哪些前置：別位工程師的、還沒審查通過的。規則與後端
 * `tasks::workflow::blocked_cards` 相同——同一位工程師的前置靠佇列順序，不算在等。
 */
export function waitingPrerequisites(card: TaskRow, all: TaskRow[], hasReviewer: boolean): TaskRow[] {
  if (card.status !== "queued") return [];
  return idList(card.depends_on)
    .map((id) => all.find((t) => t.id === id))
    .filter((d): d is TaskRow => !!d && d.agent_id !== card.agent_id && deriveModuleState(d, all, hasReviewer) !== "passed");
}
