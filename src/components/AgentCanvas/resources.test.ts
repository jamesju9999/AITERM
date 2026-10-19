import { describe, expect, it } from "vitest";

import type { AgentRow, DbQueryRow } from "../../ipc/agents";
import { accessLinks, linkPath, railItems, resourcesOf, type Resource } from "./resources";

const agent = (id: string, skill: string | null): AgentRow =>
  ({ id, kind: "coder", name: id, role: "", prompt_prefix: "", project_dir: "", canvas_x: 0, canvas_y: 0, worktree_path: null, branch: null,
    paused_task_id: null, created_at: "", last_merge_signature: null, produces_spec: false, db_skill: skill }) as AgentRow;
const q = (over: Partial<DbQueryRow>): DbQueryRow => ({
  id: 1, task_id: "t", agent_id: "a1", connection_id: "dev", connection_name: "Dev", sql: "SELECT 1",
  started_at: 100, finished_at: 101, rows: 1, ms: 5, error: null, decision: "allowed", ...over,
});
const conns = { dev: { name: "HTBKTW-DEV", sub: "mssql · HTBTWDB" }, chinook: { name: "Chinook", sub: "sqlite" } };

describe("resourcesOf", () => {
  it("只列出有 Agent 用到的連線，記下誰用、什麼權限", () => {
    const agents = [
      agent("a1", '{"connections":["dev","chinook"],"mode":"read"}'),
      agent("a2", '{"connections":["dev"],"mode":"approve"}'),
      agent("a3", null),
    ];
    const r = resourcesOf(agents, conns);
    expect(r.map((x) => x.id)).toEqual(["dev", "chinook"]);
    expect(r[0]).toMatchObject({ name: "HTBKTW-DEV", sub: "mssql · HTBTWDB", type: "db" });
    expect(r[0].holders).toEqual([{ agentId: "a1", mode: "read" }, { agentId: "a2", mode: "approve" }]);
  });

  it("找不到連線資料時用 id 當名稱", () => {
    expect(resourcesOf([agent("a1", '{"connections":["gone"],"mode":"read"}')], conns)[0].name).toBe("gone");
  });
});

describe("accessLinks", () => {
  const now = 1000;
  it("查詢中、剛結束（5 秒內）是 query；等核准是 wait；剛被擋下（2 秒內）是 denied", () => {
    const rows = [
      q({ id: 1, agent_id: "a1", connection_id: "dev", started_at: 999, finished_at: null }),
      q({ id: 2, agent_id: "a2", connection_id: "dev", started_at: 998, finished_at: null, decision: "pending" }),
      q({ id: 3, agent_id: "a1", connection_id: "chinook", started_at: 990, finished_at: 997 }),
      q({ id: 4, agent_id: "a3", connection_id: "dev", started_at: 999, finished_at: 999, decision: "denied" }),
    ];
    const links = accessLinks(rows, now);
    expect(links).toEqual(
      expect.arrayContaining([
        { agentId: "a1", resourceId: "dev", state: "query" },
        { agentId: "a2", resourceId: "dev", state: "wait" },
        { agentId: "a1", resourceId: "chinook", state: "query" },
        { agentId: "a3", resourceId: "dev", state: "denied" },
      ]),
    );
    expect(links).toHaveLength(4);
  });

  it("太久以前的不畫線；同一條線取最新的狀態", () => {
    expect(accessLinks([q({ finished_at: 990 }), q({ id: 2, decision: "denied", started_at: 996, finished_at: 996 })], now)).toEqual([]);
    expect(accessLinks([q({ id: 1, started_at: 990, finished_at: 998 }), q({ id: 2, started_at: 999, finished_at: null, decision: "pending" })], now)).toEqual([
      { agentId: "a1", resourceId: "dev", state: "wait" },
    ]);
  });
});

describe("railItems", () => {
  const res = (n: number): Resource[] =>
    Array.from({ length: n }, (_, i) => ({ id: `r${i}`, name: `R${i}`, sub: "", type: "db" as const, holders: [] }));

  it("4 個以內每個一顆", () => {
    expect(railItems(res(4), new Set()).map((i) => (i.kind === "one" ? i.resource.id : "group"))).toEqual(["r0", "r1", "r2", "r3"]);
  });

  it("超過 4 個：使用中的單獨拉出來，其餘依類型收合", () => {
    const items = railItems(res(6), new Set(["r4"]));
    expect(items).toHaveLength(2);
    expect(items[0]).toMatchObject({ kind: "one", resource: { id: "r4" } });
    expect(items[1]).toMatchObject({ kind: "group", type: "db" });
    if (items[1].kind === "group") expect(items[1].resources.map((r) => r.id)).toEqual(["r0", "r1", "r2", "r3", "r5"]);
  });
});

describe("linkPath", () => {
  const a = { x: 40, y: 40, w: 230, h: 150 };
  const pill = { x: 300, y: 600, w: 180, h: 42 };
  it("正下方沒有卡片：從卡片底部直接畫到資源", () => {
    expect(linkPath(a, pill, [])).toMatch(/^M155,190 C/);
  });
  it("正下方有別的卡片：從卡片左側繞下去，不穿過那張卡", () => {
    const below = { x: 40, y: 230, w: 230, h: 150 };
    const d = linkPath(a, pill, [below]);
    expect(d.startsWith("M40,115")).toBe(true);
    // 繞行的直線在兩張卡左邊
    const lane = Number(/V/.test(d) ? d.match(/Q(-?[\d.]+),/)?.[1] : NaN);
    expect(lane).toBeLessThan(40);
    // 最後從資源正上方垂直接到膠囊頂端
    expect(d).toMatch(new RegExp(`Q${pill.x + pill.w / 2},[\\d.]+ ${pill.x + pill.w / 2},[\\d.]+ V${pill.y}$`));
  });
  it("旁邊的卡片不算擋路", () => {
    expect(linkPath(a, pill, [{ x: 400, y: 230, w: 230, h: 150 }])).toMatch(/^M155,190 C/);
  });
});
