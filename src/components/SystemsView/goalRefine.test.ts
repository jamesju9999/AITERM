import { describe, expect, it } from "vitest";

import { buildGoalRefineMessage, GOAL_REFINE_SYSTEM, parseGoalReply } from "./goalRefine";

describe("goalRefine", () => {
  it("tells the model to keep specifics, not invent writes, and stop instead of guessing", () => {
    expect(GOAL_REFINE_SYSTEM).toContain("不要發明");
    expect(GOAL_REFINE_SYSTEM).toContain("只查看");
    expect(GOAL_REFINE_SYSTEM).toContain("不要亂試");
  });

  it("puts the system name and the draft in the message", () => {
    const m = buildGoalRefineMessage("每天查新郵件", "檢查新郵件");
    expect(m).toContain("檢查新郵件");
    expect(m).toContain("每天查新郵件");
  });

  it("strips a code fence and treats an empty reply as a failure", () => {
    expect(parseGoalReply("```\n查詢待處理訂單並回報筆數\n```")).toBe("查詢待處理訂單並回報筆數");
    expect(parseGoalReply("  \n ")).toBeNull();
  });
});
