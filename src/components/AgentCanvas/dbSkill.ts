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

/** 查詢結束後技能列再亮幾秒：快的查詢只跑幾十毫秒，畫面抓到資料時早就結束了。 */
export const SKILL_LINGER_SECS = 5;

export interface SkillActivity {
  /** pending = 寫入等你核准；running = 查詢中；recent = 剛結束（SKILL_LINGER_SECS 內）。 */
  kind: "pending" | "running" | "recent";
  sql: string;
}

/** 這位 Agent 的技能列現在要怎麼亮。等核准最優先，其次是查詢中，再來是剛結束的最新一筆。 */
export function skillActivity(rows: DbQueryRow[], agentId: string, nowSecs: number): SkillActivity | null {
  const mine = rows.filter((r) => r.agent_id === agentId).sort((a, b) => b.started_at - a.started_at || b.id - a.id);
  const pending = mine.find((r) => r.decision === "pending" && r.finished_at === null);
  if (pending) return { kind: "pending", sql: pending.sql };
  const running = mine.find((r) => r.finished_at === null);
  if (running) return { kind: "running", sql: running.sql };
  const recent = mine.find((r) => r.decision !== "denied" && r.finished_at !== null && nowSecs - r.finished_at <= SKILL_LINGER_SECS);
  return recent ? { kind: "recent", sql: recent.sql } : null;
}

/** `agents.custom_skills`：技能資料夾路徑的 JSON 陣列。沒有或壞掉都是空的。 */
export function parseSkillPaths(raw: string | null | undefined): string[] {
  if (!raw) return [];
  try {
    const v: unknown = JSON.parse(raw);
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}
