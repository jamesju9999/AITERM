import { describe, expect, it } from "vitest";

import { buildSummaryPrompt, buildReportPrompt, MAX_CARDS } from "./reportPrompts";
import type { TaskWithAttachments } from "../../ipc/tasks";

const card = (over: Partial<TaskWithAttachments> = {}): TaskWithAttachments => ({
  id: "t1",
  title: "做一件事",
  body: "詳細說明",
  project_dir: "/repo",
  status: "done",
  parallel_ok: true,
  interactive: false,
  sort_order: 1,
  outcome: "success",
  tab_id: null,
  transcript_path: null,
  error_message: null,
  created_at: "2026-09-05T10:00:00Z",
  dispatched_at: null,
  finished_at: null,
  ai_summary: null,
  archived_at: null,
  attachments: [],
  ...over,
});

describe("buildSummaryPrompt", () => {
  it("帶入標題、內容與對話記錄", () => {
    const p = buildSummaryPrompt(card({ title: "重構登入" }), "終端機輸出內容");
    expect(p).toContain("重構登入");
    expect(p).toContain("終端機輸出內容");
  });

  it("沒有對話記錄時也能組出提示詞，並說明只有欄位資料", () => {
    const p = buildSummaryPrompt(card(), null);
    expect(p).toContain("沒有對話記錄");
  });

  it("要求限制字數，避免第二階段輸入爆掉", () => {
    expect(buildSummaryPrompt(card(), "x")).toContain("300");
  });
});

describe("buildReportPrompt", () => {
  const cards = [
    card({ id: "a", title: "已完成的", status: "done", ai_summary: "摘要 A" }),
    card({ id: "b", title: "執行中的", status: "running", outcome: null }),
    card({ id: "c", title: "還沒開始的", status: "planning", outcome: null }),
  ];

  it("兩種風格產生不同的提示詞", () => {
    const review = buildReportPrompt(cards, "review", "我的專案");
    const formal = buildReportPrompt(cards, "formal", "我的專案");
    expect(review).not.toEqual(formal);
  });

  it("包含全部四欄的卡片，不只已完成的", () => {
    const p = buildReportPrompt(cards, "review", "我的專案");
    expect(p).toContain("已完成的");
    expect(p).toContain("執行中的");
    expect(p).toContain("還沒開始的");
  });

  it("帶入已完成卡片的摘要", () => {
    expect(buildReportPrompt(cards, "review", "我的專案")).toContain("摘要 A");
  });

  it("要求輸出 artifact-html", () => {
    expect(buildReportPrompt(cards, "review", "我的專案")).toContain("artifact-html");
  });

  // 卡片太多時第二階段的輸入會爆掉。取最近的，並且要讓報告知道
  // 自己看到的不是全部——不講的話 AI 會把「最近 100 張」當成「全部」。
  it("超過上限時只取最近的並在提示詞中註明", () => {
    const many = Array.from({ length: MAX_CARDS + 20 }, (_, i) =>
      card({ id: `t${i}`, title: `卡片 ${i}` }),
    );
    const p = buildReportPrompt(many, "review", "我的專案");
    expect(p).toContain(String(MAX_CARDS));
    expect(p).not.toContain("卡片 0");
    expect(p).toContain(`卡片 ${MAX_CARDS + 19}`);
  });
});
