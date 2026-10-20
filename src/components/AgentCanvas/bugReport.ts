import type { AgentRow } from "../../ipc/agents";
import type { TaskWithAttachments } from "../../ipc/tasks";

/** 標題取第一行的前幾個字。 */
const TITLE_MAX = 60;

/** 交給誰：沒選就是最不忙的工程師（不含產出規格的人——他寫規格，不修 bug）。 */
export function pickAssignee(agents: AgentRow[], tasks: TaskWithAttachments[], preferred: string | null): AgentRow | null {
  const coders = agents.filter((a) => a.kind === "coder" && !a.produces_spec);
  const hit = preferred && coders.find((a) => a.id === preferred);
  if (hit) return hit;
  const load = (a: AgentRow) => tasks.filter((x) => x.agent_id === a.id && (x.status === "queued" || x.status === "running")).length;
  return coders.reduce<AgentRow | null>((best, a) => (!best || load(a) < load(best) ? a : best), null);
}

/** 把使用者的描述變成卡片：第一行當標題，內文附上該怎麼做與相關模組。 */
export function bugCard(text: string, instructions: string, related: string | null): { title: string; body: string } {
  const trimmed = text.trim();
  const first = trimmed.split("\n")[0].trim();
  const title = first.length > TITLE_MAX ? `${first.slice(0, TITLE_MAX)}…` : first;
  return { title, body: [trimmed, related, instructions].filter(Boolean).join("\n\n") };
}
