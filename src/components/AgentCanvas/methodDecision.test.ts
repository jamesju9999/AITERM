import { describe, expect, it } from "vitest";

import { buildDiscussionPrompt, DECISION_PATH_PLACEHOLDER, formatDecision, parseDecisionJson, type MethodDecision } from "./methodDecision";

const good: MethodDecision = {
  approach: "batch",
  reason: "533 支畫面骨架相同",
  trial: "先試 5 支，看通過率",
  requirement: "把 Cm 畫面轉成網頁",
};

describe("parseDecisionJson", () => {
  it("reads a valid decision file", () => {
    expect(parseDecisionJson(JSON.stringify(good))).toEqual(good);
  });
  it("returns null for malformed JSON (the file may be half-written)", () => {
    expect(parseDecisionJson("{oops")).toBeNull();
  });
  it("returns null for empty text", () => {
    expect(parseDecisionJson("")).toBeNull();
  });
  it("rejects an unknown approach", () => {
    expect(parseDecisionJson(JSON.stringify({ ...good, approach: "magic" }))).toBeNull();
  });
  it("rejects an empty requirement", () => {
    expect(parseDecisionJson(JSON.stringify({ ...good, requirement: "  " }))).toBeNull();
  });
  it("accepts reason and trial being absent", () => {
    expect(parseDecisionJson(JSON.stringify({ approach: "single", requirement: "x" }))).toEqual({ approach: "single", reason: "", trial: "", requirement: "x" });
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

describe("buildDiscussionPrompt", () => {
  it("keeps the placeholder exactly once so the backend can fill the real path", () => {
    const p = buildDiscussionPrompt("要轉換舊平台");
    expect(p.split(DECISION_PATH_PLACEHOLDER).length - 1).toBe(1);
  });
  it("includes the user's opening and the file schema", () => {
    const p = buildDiscussionPrompt("要轉換舊平台");
    expect(p).toContain("要轉換舊平台");
    expect(p).toContain('"approach"');
    expect(p).toContain("single|batch|generator");
  });
  it("tells the AI it may read the reference folders and must not claim it cannot", () => {
    expect(buildDiscussionPrompt("x")).toContain("先讀");
  });
});
