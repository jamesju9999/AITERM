import { describe, expect, it } from "vitest";

import { BRIEF_REFINE_SYSTEM, buildBriefRefineMessage, parseBriefReply } from "./briefRefine";

describe("專案簡介的 AI 潤飾", () => {
  it("系統提示講清楚用途與規則：保留原意、不發明、不列模組分工、要精簡", () => {
    expect(BRIEF_REFINE_SYSTEM).toMatch(/每一位 AI 工程師/);
    expect(BRIEF_REFINE_SYSTEM).toMatch(/不要發明/);
    expect(BRIEF_REFINE_SYSTEM).toMatch(/不要列出模組分工/);
    expect(BRIEF_REFINE_SYSTEM).toMatch(/800 字/);
  });

  it("使用者訊息帶著草稿原文", () => {
    expect(buildBriefRefineMessage("做番茄鐘，用 React")).toContain("做番茄鐘，用 React");
  });

  it("回覆去掉程式碼區塊與頭尾空白；內文裡的 --- 原樣保留；空的回 null", () => {
    expect(parseBriefReply("```markdown\n目標：番茄鐘\n```")).toBe("目標：番茄鐘");
    expect(parseBriefReply("\n目標：A\n---\n不做：B\n")).toBe("目標：A\n---\n不做：B");
    expect(parseBriefReply("   ")).toBeNull();
    expect(parseBriefReply("```\n\n```")).toBeNull();
  });
});
