import type { DbQueryRow, DbSkill, DbSkillMode } from "../../ipc/agents";

const MODES: DbSkillMode[] = ["read", "approve", "write"];

/** `agents.db_skill` 的 JSON。沒裝、壞掉或沒有任何連線都是 null（同後端 db_skill::parse）。 */
export function parseDbSkill(raw: string | null | undefined): DbSkill | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<DbSkill>;
    if (!Array.isArray(v.connections) || v.connections.length === 0) return null;
    if (!MODES.includes(v.mode as DbSkillMode)) return null;
    return { connections: v.connections.filter((c): c is string => typeof c === "string"), mode: v.mode as DbSkillMode };
  } catch {
    return null;
  }
}

/** 卡片標籤上的 SQL：壓成一行，最多 40 字。 */
export const SQL_PREVIEW_CHARS = 40;
export function sqlPreview(sql: string): string {
  const one = sql.replace(/\s+/g, " ").trim();
  return one.length > SQL_PREVIEW_CHARS ? `${one.slice(0, SQL_PREVIEW_CHARS)}…` : one;
}

/** 這位 Agent 最新一筆還沒結束的查詢（標籤亮起來用）。 */
export function activeQuery(rows: DbQueryRow[], agentId: string): DbQueryRow | null {
  return rows.filter((r) => r.agent_id === agentId && r.finished_at === null).sort((a, b) => b.started_at - a.started_at || b.id - a.id)[0] ?? null;
}
