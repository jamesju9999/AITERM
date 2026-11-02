import { describe, expect, it } from "vitest";

import { buildDiscussionMessage, formatDecision, parseDecision, type MethodDecision } from "./methodDecision";

const good: MethodDecision = {
  approach: "batch",
  reason: "533 支畫面骨架相同",
  trial: "先試 5 支，看通過率",
  requirement: "把 Cm 畫面轉成網頁",
};

const fence = (o: unknown) => "我建議這樣做。\n```decision\n" + JSON.stringify(o) + "\n```\n";

describe("parseDecision", () => {
  it("reads the decision block from a reply", () => {
    expect(parseDecision(fence(good))).toEqual(good);
  });
  it("returns null when there is no block", () => {
    expect(parseDecision("還在討論，先問你幾個問題")).toBeNull();
  });
  it("returns null for malformed JSON", () => {
    expect(parseDecision("```decision\n{oops\n```")).toBeNull();
  });
  it("rejects an unknown approach", () => {
    expect(parseDecision(fence({ ...good, approach: "magic" }))).toBeNull();
  });
  it("rejects a decision with an empty requirement", () => {
    expect(parseDecision(fence({ ...good, requirement: "  " }))).toBeNull();
  });
  it("takes the last block when the reply has several", () => {
    const reply = fence({ ...good, approach: "single" }) + fence(good);
    expect(parseDecision(reply)?.approach).toBe("batch");
  });
});

describe("formatDecision", () => {
  it("names the approach and keeps reason and trial", () => {
    const text = formatDecision(good);
    expect(text).toContain("批次");
    expect(text).toContain("533 支畫面骨架相同");
    expect(text).toContain("先試 5 支");
  });
  it("omits the trial line when there is none", () => {
    expect(formatDecision({ ...good, trial: "" })).not.toContain("試跑");
  });
});

describe("buildDiscussionMessage", () => {
  it("flattens the turns in order and includes the project context", () => {
    const m = buildDiscussionMessage(
      [
        { role: "user", content: "要轉換舊平台" },
        { role: "assistant", content: "有幾支？" },
        { role: "user", content: "533 支畫面" },
      ],
      "專案簡介 X",
      [{ path: "/old", note: "舊系統" }],
    );
    expect(m.indexOf("要轉換舊平台")).toBeLessThan(m.indexOf("有幾支？"));
    expect(m.indexOf("有幾支？")).toBeLessThan(m.indexOf("533 支畫面"));
    expect(m).toContain("專案簡介 X");
    expect(m).toContain("/old：舊系統");
  });
});
