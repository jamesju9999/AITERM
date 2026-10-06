import { describe, expect, it } from "vitest";
import {
  MAX_MILESTONES,
  MAX_MILESTONE_CHARS,
  buildMilestoneCheckRequest,
  buildMilestonePlanRequest,
  mergePlanKeepingDone,
  parseMilestoneCheck,
  parseMilestonePlan,
  sanitizeMilestoneState,
  type Milestone,
} from "./milestones";

const m = (id: string, text: string, done = false): Milestone => ({ id, text, done });

describe("parseMilestonePlan", () => {
  it("parses a plain JSON string array", () => {
    expect(parseMilestonePlan('["盤點 API","拆分登入模組"]')).toEqual(["盤點 API", "拆分登入模組"]);
  });

  it("tolerates code fences and surrounding prose", () => {
    expect(parseMilestonePlan('好的：\n```json\n["A","B"]\n```\n以上')).toEqual(["A", "B"]);
  });

  it("returns [] for garbage, objects and null", () => {
    expect(parseMilestonePlan("沒有 JSON")).toEqual([]);
    expect(parseMilestonePlan('{"a":1}')).toEqual([]);
    expect(parseMilestonePlan(null)).toEqual([]);
    expect(parseMilestonePlan("")).toEqual([]);
  });

  it("drops blanks and non-strings, trims, and removes duplicates (ignoring case and spacing)", () => {
    expect(parseMilestonePlan('["  A  ", "", 3, null, "a", "A", "B"]')).toEqual(["A", "B"]);
  });

  it("truncates long items and caps the count", () => {
    const long = "字".repeat(MAX_MILESTONE_CHARS + 30);
    const [first] = parseMilestonePlan(JSON.stringify([long]));
    expect(first.length).toBe(MAX_MILESTONE_CHARS);
    const many = Array.from({ length: MAX_MILESTONES + 5 }, (_, i) => `項目${i}`);
    expect(parseMilestonePlan(JSON.stringify(many))).toHaveLength(MAX_MILESTONES);
  });
});

describe("buildMilestonePlanRequest", () => {
  it("contains the goal and the language, asks for 5-8 ordered items, and JSON only", () => {
    const msg = buildMilestonePlanRequest("將舊程式轉成網頁版", "請使用繁體中文");
    expect(msg).toContain("將舊程式轉成網頁版");
    expect(msg).toContain("請使用繁體中文");
    expect(msg).toContain("5");
    expect(msg).toContain("8");
    expect(msg).toContain("JSON");
  });
});

describe("buildMilestoneCheckRequest", () => {
  const ms = [m("a", "盤點 API", true), m("b", "拆分登入模組"), m("c", "遷移資料庫")];

  it("numbers the milestones from 1, shows their state, and includes goal, history and the current screen", () => {
    const msg = buildMilestoneCheckRequest("把舊系統轉網頁版", ms, "OLD-SCREEN", "CURRENT-SCREEN", "LANG");
    expect(msg).toContain("把舊系統轉網頁版");
    expect(msg).toContain("1. [已完成] 盤點 API");
    expect(msg).toContain("2. [未完成] 拆分登入模組");
    expect(msg).toContain("3. [未完成] 遷移資料庫");
    expect(msg).toContain("3.");
    expect(msg).toContain("OLD-SCREEN");
    expect(msg).toContain("CURRENT-SCREEN");
    expect(msg.indexOf("OLD-SCREEN")).toBeLessThan(msg.indexOf("CURRENT-SCREEN"));
    expect(msg).toContain("LANG");
  });

  it("omits the history section when there is none", () => {
    const msg = buildMilestoneCheckRequest("g", ms, "", "CUR", "");
    expect(msg).not.toContain("較早的畫面");
  });
});

describe("parseMilestoneCheck", () => {
  const ms = [m("a", "一", true), m("b", "二"), m("c", "三"), m("d", "四")];

  it("maps the 1-based numbers back to ids and keeps the note", () => {
    const r = parseMilestoneCheck('{"done":[2,3],"note":"登入與遷移都完成了"}', ms);
    expect(r.doneIds).toEqual(["b", "c"]);
    expect(r.note).toBe("登入與遷移都完成了");
  });

  it("ignores numbers that are out of range, duplicated, non-numeric, or already done", () => {
    const r = parseMilestoneCheck('{"done":[1,2,2,9,0,"x",4]}', ms);
    expect(r.doneIds).toEqual(["b", "d"]);
  });

  it("accepts numeric strings and tolerates a code fence", () => {
    const r = parseMilestoneCheck('```json\n{"done":["3"],"note":""}\n```', ms);
    expect(r.doneIds).toEqual(["c"]);
  });

  it("returns nothing for garbage", () => {
    expect(parseMilestoneCheck("無法判斷", ms)).toEqual({ doneIds: [], note: "" });
    expect(parseMilestoneCheck(null, ms)).toEqual({ doneIds: [], note: "" });
    expect(parseMilestoneCheck('{"done":"all"}', ms).doneIds).toEqual([]);
  });
});

describe("mergePlanKeepingDone", () => {
  it("keeps finished items and adds the planned ones after them", () => {
    const existing = [m("a", "盤點 API", true), m("b", "舊的未完成項")];
    const out = mergePlanKeepingDone(existing, ["拆分登入模組", "遷移資料庫"]);
    expect(out.map((x) => x.text)).toEqual(["盤點 API", "拆分登入模組", "遷移資料庫"]);
    expect(out[0]).toEqual(existing[0]); // 同一個 id、仍是完成
    expect(out.slice(1).every((x) => !x.done)).toBe(true);
  });

  it("does not add a planned item that duplicates a finished one", () => {
    const out = mergePlanKeepingDone([m("a", "盤點 API", true)], [" 盤點  api ", "新項目"]);
    expect(out.map((x) => x.text)).toEqual(["盤點 API", "新項目"]);
  });

  it("gives new items fresh unique ids", () => {
    const out = mergePlanKeepingDone([], ["A", "B", "C"]);
    expect(new Set(out.map((x) => x.id)).size).toBe(3);
  });

  it("never exceeds the cap, finished items win", () => {
    const done = Array.from({ length: 4 }, (_, i) => m(`d${i}`, `完成${i}`, true));
    const planned = Array.from({ length: MAX_MILESTONES }, (_, i) => `計畫${i}`);
    const out = mergePlanKeepingDone(done, planned);
    expect(out).toHaveLength(MAX_MILESTONES);
    expect(out.slice(0, 4).every((x) => x.done)).toBe(true);
  });
});

describe("sanitizeMilestoneState", () => {
  it("accepts a valid state", () => {
    const s = { forGoal: "目標", items: [m("a", "一"), m("b", "二", true)] };
    expect(sanitizeMilestoneState(s)).toEqual(s);
  });

  it("returns undefined for non-objects and for states with no usable items", () => {
    expect(sanitizeMilestoneState(null)).toBeUndefined();
    expect(sanitizeMilestoneState("x")).toBeUndefined();
    expect(sanitizeMilestoneState({ forGoal: "g", items: [] })).toBeUndefined();
    expect(sanitizeMilestoneState({ forGoal: "g", items: "nope" })).toBeUndefined();
  });

  it("drops malformed items, fixes types, caps count and length", () => {
    const raw = {
      forGoal: 5,
      items: [
        { id: "a", text: "ok", done: true },
        { id: 1, text: "bad id", done: false },
        { id: "b", text: "", done: false },
        { id: "c", text: "x".repeat(MAX_MILESTONE_CHARS + 20), done: "yes" },
        null,
        { id: "a", text: "duplicate id", done: false },
      ],
    };
    const s = sanitizeMilestoneState(raw)!;
    expect(s.forGoal).toBe("");
    expect(s.items.map((x) => x.id)).toEqual(["a", "c"]);
    expect(s.items[1].text.length).toBe(MAX_MILESTONE_CHARS);
    expect(s.items[1].done).toBe(false);
    const many = { forGoal: "g", items: Array.from({ length: 30 }, (_, i) => ({ id: `i${i}`, text: `t${i}`, done: false })) };
    expect(sanitizeMilestoneState(many)!.items).toHaveLength(MAX_MILESTONES);
  });
});
