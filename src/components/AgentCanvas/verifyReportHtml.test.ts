import { describe, expect, it } from "vitest";

import { verifyReportHtml, type VerifyReportLabels } from "./verifyReportHtml";

const L: VerifyReportLabels = {
  title: "驗收報告", pass: "通過", fail: "沒通過", manual: "需你確認", steps: "重現步驟", expected: "預期",
  actual: "實際", notes: "說明", evidence: "證據", overall: "整體說明",
  summary: (p, f, m) => `通過 ${p} · 沒通過 ${f} · 需你確認 ${m}`,
  project: "專案", date: "驗收日期", projectType: "專案類型", conclusion: "驗收結論", verdictPass: "驗收通過",
  verdictFail: "驗收未通過", overview: "驗收總覽", detail: "驗收明細", criterion: "驗收項目", result: "結果", generatedBy: "由 AITerm 產生",
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
    expect(html).toContain("<dd>Ask Test</dd>");
    expect(html).toContain("<dd>2026-10-17</dd>");
    expect(html).toContain("<dd>web</dd>");
    expect(html).toContain("通過 1 · 沒通過 1 · 需你確認 0");
    expect(html).toContain("npm test 通過");
    expect(html).toContain("已關閉伺服器");
    expect(html.indexOf("按 +1")).toBeLessThan(html.indexOf("歸零"));
  });

  it("有結論橫幅、總覽表（連到明細）與每項編號", () => {
    const html = verifyReportHtml(view, { project: "P", date: "d" }, L);
    expect(html).toContain('class="verdict fail"');
    expect(html).toContain("驗收未通過");
    expect(html).toContain('<a href="#i1">');
    expect(html).toContain('id="i2"');
    const pass = verifyReportHtml({ ...view, report: { ...view.report, verdict: "pass" as const } }, { project: "P", date: "d" }, L);
    expect(pass).toContain('class="verdict pass"');
  });

  it("沒有任何項目時不會出現總覽表", () => {
    const html = verifyReportHtml({ report: { verdict: "pass", items: [] }, images: {} }, { project: "P", date: "d" }, L);
    expect(html).not.toContain("<table>");
    expect(html).not.toContain("NaN");
  });

  it("報告內容是 Agent 寫的，一律跳脫", () => {
    const html = verifyReportHtml(view, { project: "<b>x</b>", date: "d" }, L);
    expect(html).not.toContain("<script>alert(1)</script>");
    expect(html).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(html).not.toContain("<b>x</b>");
  });
});
