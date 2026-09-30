import { describe, it, expect } from "vitest";
import { formatCreatedAt, parseCreatedAt, sortByCreated } from "./cardSort";

const c = (id: string, created_at: string) => ({ id, created_at });
const ids = (rows: { id: string }[]) => rows.map((r) => r.id);

describe("parseCreatedAt", () => {
  it("把 SQLite 的無時區字串當成 UTC，不是本地時間", () => {
    expect(parseCreatedAt("2026-09-30 00:00:00")).toBe(Date.UTC(2026, 8, 30, 0, 0, 0));
  });

  it("認得 ISO 格式與純日期", () => {
    expect(parseCreatedAt("2026-09-05T10:00:00Z")).toBe(Date.UTC(2026, 8, 5, 10));
    expect(parseCreatedAt("2026-01-01")).toBe(Date.UTC(2026, 0, 1));
  });

  it("空字串、null、亂字串都回 null", () => {
    expect(parseCreatedAt("")).toBeNull();
    expect(parseCreatedAt(null)).toBeNull();
    expect(parseCreatedAt("not a date")).toBeNull();
  });
});

describe("formatCreatedAt", () => {
  it("輸出 YYYY-MM-DD HH:mm，且換算回同一個時間點（與執行環境時區無關）", () => {
    const out = formatCreatedAt("2026-09-30 06:07:45")!;
    expect(out).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
    // 把顯示結果當本地時間解回去，必須等於原本的時間點（到分鐘）。
    const back = new Date(out.replace(" ", "T")).getTime();
    expect(back).toBe(Date.UTC(2026, 8, 30, 6, 7, 0));
  });

  it("解析不了就回 null", () => {
    expect(formatCreatedAt("")).toBeNull();
  });
});

describe("sortByCreated", () => {
  const rows = [
    c("mid", "2026-02-01 00:00:00"),
    c("old", "2026-01-01 00:00:00"),
    c("new", "2026-03-01 00:00:00"),
  ];

  it("default 原樣回傳，不重排", () => {
    expect(ids(sortByCreated(rows, "default"))).toEqual(["mid", "old", "new"]);
  });

  it("created-desc 新到舊、created-asc 舊到新", () => {
    expect(ids(sortByCreated(rows, "created-desc"))).toEqual(["new", "mid", "old"]);
    expect(ids(sortByCreated(rows, "created-asc"))).toEqual(["old", "mid", "new"]);
  });

  it("不動傳入的陣列", () => {
    const copy = [...rows];
    sortByCreated(rows, "created-desc");
    expect(rows).toEqual(copy);
  });

  it("同一秒建立的卡片保持原本（手動）順序", () => {
    const tied = [c("a", "2026-01-01 00:00:00"), c("b", "2026-01-01 00:00:00"), c("c", "2026-01-01 00:00:00")];
    expect(ids(sortByCreated(tied, "created-desc"))).toEqual(["a", "b", "c"]);
    expect(ids(sortByCreated(tied, "created-asc"))).toEqual(["a", "b", "c"]);
  });

  it("沒有可解析日期的卡片不論升降冪都排最後", () => {
    const mixed = [c("blank", ""), c("old", "2026-01-01 00:00:00"), c("new", "2026-03-01 00:00:00")];
    expect(ids(sortByCreated(mixed, "created-desc"))).toEqual(["new", "old", "blank"]);
    expect(ids(sortByCreated(mixed, "created-asc"))).toEqual(["old", "new", "blank"]);
  });
});
