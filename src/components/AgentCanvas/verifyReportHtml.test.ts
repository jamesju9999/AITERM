import { describe, expect, it } from "vitest";

import { verifyReportHtml, type VerifyReportLabels } from "./verifyReportHtml";

const L: VerifyReportLabels = {
  title: "驗收報告", pass: "通過", fail: "沒通過", manual: "需你確認", steps: "重現步驟", expected: "預期",
  actual: "實際", notes: "說明", evidence: "證據", overall: "整體說明",
  summary: (p, f, m) => `通過 ${p} · 沒通過 ${f} · 需你確認 ${m}`,
};

describe("verifyReportHtml", () => {
  const view = {
    report: {
      verdict: "fail" as const,
      project_type: "web",
      items: [
        { criterion: "歸零", status: "pass" as const, evidence: ["evidence/a.png", "npm test 通過"] },
        { criterion: "按 +1 <script>alert(1)</script>", status: "fail" as const, evidence: [], steps: "1. 按 +1", actual: "顯示 2" },
      ],
      notes: "已關閉伺服器",
    },
    images: { "evidence/a.png": "data:image/png;base64,AAAA" },
  };

  it("自成一檔：截圖內嵌、摘要、整體說明，沒通過的排前面", () => {
    const html = verifyReportHtml(view, { project: "Ask Test", date: "2026-10-17" }, L);
    expect(html.startsWith("<!doctype html>")).toBe(true);
    expect(html).toContain('<img src="data:image/png;base64,AAAA"');
    expect(html).toContain("Ask Test · 2026-10-17 · 通過 1 · 沒通過 1 · 需你確認 0 · web");
    expect(html).toContain("npm test 通過");
    expect(html).toContain("已關閉伺服器");
    expect(html.indexOf("按 +1")).toBeLessThan(html.indexOf("歸零"));
  });

  it("報告內容是 Agent 寫的，一律跳脫", () => {
    const html = verifyReportHtml(view, { project: "<b>x</b>", date: "d" }, L);
    expect(html).not.toContain("<script>alert(1)</script>");
    expect(html).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(html).not.toContain("<b>x</b>");
  });
});
