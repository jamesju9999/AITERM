import { describe, expect, it } from "vitest";

import type { DbQueryRow } from "../../ipc/agents";
import { activeQuery, parseDbSkill, sqlPreview } from "./dbSkill";

const row = (over: Partial<DbQueryRow>): DbQueryRow => ({
  id: 1, task_id: "t", agent_id: "a1", connection_id: "dev", connection_name: "Dev", sql: "SELECT 1",
  started_at: 100, finished_at: 101, rows: 1, ms: 5, error: null, decision: "allowed", ...over,
});

describe("parseDbSkill", () => {
  it("沒裝、壞掉、沒有連線都當作沒裝", () => {
    expect(parseDbSkill(null)).toBeNull();
    expect(parseDbSkill(undefined)).toBeNull();
    expect(parseDbSkill("{")).toBeNull();
    expect(parseDbSkill('{"connections":[],"mode":"read"}')).toBeNull();
    expect(parseDbSkill('{"connections":["dev"],"mode":"bogus"}')).toBeNull();
    expect(parseDbSkill('{"connections":["dev"],"mode":"approve"}')).toEqual({ connections: ["dev"], mode: "approve" });
  });
});

describe("sqlPreview", () => {
  it("壓成一行，超過 40 字加省略號", () => {
    expect(sqlPreview("SELECT *\n  FROM t")).toBe("SELECT * FROM t");
    const long = "SELECT id, name, email, created_at FROM customers WHERE active = 1";
    expect(sqlPreview(long)).toBe(`${long.slice(0, 40)}…`);
    expect(sqlPreview(long).length).toBe(41);
  });
});

describe("activeQuery", () => {
  it("這位 Agent 最新一筆還沒結束的查詢", () => {
    const rows = [
      row({ id: 3, started_at: 300, finished_at: null, sql: "SELECT 3" }),
      row({ id: 2, started_at: 200, finished_at: null, agent_id: "a2" }),
      row({ id: 1, started_at: 100, finished_at: null, sql: "SELECT old" }),
    ];
    expect(activeQuery(rows, "a1")?.sql).toBe("SELECT 3");
    expect(activeQuery(rows, "a3")).toBeNull();
    expect(activeQuery([row({})], "a1")).toBeNull();
  });
});
