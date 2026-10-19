import type { AgentRow, DbQueryRow, DbSkillMode } from "../../ipc/agents";
import { parseDbSkill, SKILL_LINGER_SECS } from "./dbSkill";

/** 資源區：多位 Agent 共用的外部資源（目前只有資料庫）。原型見 .superpowers/brainstorm/agent-db-resource。 */
export type ResourceType = "db";

export interface Resource {
  id: string;
  name: string;
  /** 類型與資料庫名稱，例如「mssql · HTBTWDB」。 */
  sub: string;
  type: ResourceType;
  /** 誰可以用、什麼權限。 */
  holders: { agentId: string; mode: DbSkillMode }[];
}

/** 有 Agent 用到的連線（依第一次出現的順序）。`conns`：連線 id → 名稱與說明。 */
export function resourcesOf(agents: AgentRow[], conns: Record<string, { name: string; sub: string }>): Resource[] {
  const out = new Map<string, Resource>();
  for (const a of agents) {
    const skill = parseDbSkill(a.db_skill);
    if (!skill) continue;
    for (const id of skill.connections) {
      const r = out.get(id) ?? { id, name: conns[id]?.name ?? id, sub: conns[id]?.sub ?? "", type: "db" as const, holders: [] };
      r.holders.push({ agentId: a.id, mode: skill.mode });
      out.set(id, r);
    }
  }
  return [...out.values()];
}

/** 被擋下的紅線亮多久（秒）。查詢時間只記到秒，所以是「2 秒內」。 */
export const DENIED_FLASH_SECS = 2;

export type LinkState = "query" | "wait" | "denied";
export interface AccessLink {
  agentId: string;
  resourceId: string;
  state: LinkState;
}

/** 現在要畫的存取線：每位 Agent 對每個連線取最新一筆查詢決定狀態；閒置就不畫。 */
export function accessLinks(rows: DbQueryRow[], nowSecs: number): AccessLink[] {
  const latest = new Map<string, DbQueryRow>();
  for (const r of rows) {
    if (!r.agent_id) continue;
    const key = `${r.agent_id}\n${r.connection_id}`;
    const cur = latest.get(key);
    if (!cur || r.started_at > cur.started_at || (r.started_at === cur.started_at && r.id > cur.id)) latest.set(key, r);
  }
  const links: AccessLink[] = [];
  for (const r of latest.values()) {
    const base = { agentId: r.agent_id as string, resourceId: r.connection_id };
    if (r.decision === "pending" && r.finished_at === null) links.push({ ...base, state: "wait" });
    else if (r.decision === "denied") {
      if (nowSecs - (r.finished_at ?? r.started_at) <= DENIED_FLASH_SECS) links.push({ ...base, state: "denied" });
    } else if (r.finished_at === null || nowSecs - r.finished_at <= SKILL_LINGER_SECS) links.push({ ...base, state: "query" });
  }
  return links;
}

/** 超過幾個就把閒置的依類型收合。 */
export const RAIL_MAX_SHOWN = 4;

export type RailItem = { kind: "one"; resource: Resource } | { kind: "group"; type: ResourceType; resources: Resource[] };

/** 資源區要畫的膠囊：少的時候每個一顆；多的時候使用中的單獨拉出來（線要接在具體那一個上），其餘依類型收合。 */
export function railItems(resources: Resource[], busy: Set<string>): RailItem[] {
  if (resources.length <= RAIL_MAX_SHOWN) return resources.map((resource) => ({ kind: "one", resource }));
  const items: RailItem[] = resources.filter((r) => busy.has(r.id)).map((resource) => ({ kind: "one", resource }));
  const groups = new Map<ResourceType, Resource[]>();
  for (const r of resources) if (!busy.has(r.id)) groups.set(r.type, [...(groups.get(r.type) ?? []), r]);
  for (const [type, list] of groups) items.push({ kind: "group", type, resources: list });
  return items;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** 繞行時離卡片左緣多遠、轉角半徑。 */
const LANE_GAP = 22;
const CORNER = 10;

/**
 * Agent 卡到資源膠囊的線。正下方沒有別的卡就從卡片底部一條曲線下去；
 * 有卡擋在中間（例如兩位工程師上下排）就從左側出來、沿著所有擋路卡片的左邊下去，
 * 再橫到資源上方——不然線會從別人的卡片後面穿過去。
 */
export function linkPath(a: Rect, pill: Rect, others: Rect[]): string {
  const x1 = a.x + a.w / 2;
  const y1 = a.y + a.h;
  const x2 = pill.x + pill.w / 2;
  const y2 = pill.y;
  const blockers = others.filter((o) => o.y >= y1 && o.y < y2 && o.x < x1 && o.x + o.w > x1);
  if (blockers.length === 0) {
    const my = (y1 + y2) / 2;
    return `M${x1},${y1} C${x1},${my} ${x2},${my} ${x2},${y2}`;
  }
  const lane = Math.min(a.x, ...blockers.map((b) => b.x)) - LANE_GAP;
  const sy = a.y + a.h / 2;
  const turnY = y2 - 2 * CORNER - 6;
  const dir = x2 >= lane ? 1 : -1;
  return (
    `M${a.x},${sy} H${lane + CORNER} Q${lane},${sy} ${lane},${sy + CORNER} ` +
    `V${turnY - CORNER} Q${lane},${turnY} ${lane + dir * CORNER},${turnY} ` +
    `H${x2 - dir * CORNER} Q${x2},${turnY} ${x2},${turnY + CORNER} V${y2}`
  );
}
