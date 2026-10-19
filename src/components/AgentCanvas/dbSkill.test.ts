import { describe, expect, it } from "vitest";

import type { DbQueryRow } from "../../ipc/agents";
import { parseDbSkill, skillActivity, sqlPreview } from "./dbSkill";

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

describe("skillActivity", () => {
  const now = 1000;
  it("等核准優先，其次執行中，再來是 5 秒內剛結束的", () => {
    expect(skillActivity([row({ finished_at: null, decision: "pending", sql: "UPDATE t" }), row({ id: 2, started_at: 999, finished_at: null })], "a1", now))
      .toEqual({ kind: "pending", sql: "UPDATE t" });
    expect(skillActivity([row({ started_at: 990, finished_at: null, sql: "SELECT run" })], "a1", now)).toEqual({ kind: "running", sql: "SELECT run" });
    expect(skillActivity([row({ started_at: 994, finished_at: 996, sql: "SELECT recent" })], "a1", now)).toEqual({ kind: "recent", sql: "SELECT recent" });
  });

  it("超過 5 秒、別人的、被擋下的都不亮", () => {
    expect(skillActivity([row({ started_at: 900, finished_at: 994 })], "a1", now)).toBeNull();
    expect(skillActivity([row({ agent_id: "a2", finished_at: 999 })], "a1", now)).toBeNull();
    expect(skillActivity([row({ finished_at: 999, decision: "denied" })], "a1", now)).toBeNull();
  });
});
