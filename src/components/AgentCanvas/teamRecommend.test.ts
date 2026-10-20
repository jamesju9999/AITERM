import { describe, expect, it } from "vitest";

import { translations } from "../../lib/i18n";
import { buildRecommendMessage, buildRecommendSystem, MAX_RECOMMENDED_CODERS, parseRecommendation } from "./teamRecommend";

const zh = translations["zh-TW"];
const reply = (o: unknown) => "```json\n" + JSON.stringify(o) + "\n```";

describe("parseRecommendation", () => {
  it("照 AI 的選擇組隊：自訂名稱沿用內建提示詞，規格欄在最左，理由對到卡片上的名稱", () => {
    const rec = parseRecommendation(
      reply({
        summary: "先盤點再轉換",
        coders: [
          { role: "analyst", reason: "需求還不清楚" },
          { role: "inventory_researcher", name: "EGL 研究工程師", count: 2, reason: "舊系統是 EGL" },
        ],
        reviewer: { role: "docs", reason: "產出是文件" },
        orchestrator: { include: true, reason: "合併文件" },
        verifier: { include: false },
      }),
      zh,
    )!;
    const coders = rec.template.agents.filter((a) => a.kind === "coder");
    expect(coders.map((a) => [a.role, a.produces_spec])).toEqual([
      [zh.ac_role_analyst, true],
      ["EGL 研究工程師", false],
      ["EGL 研究工程師", false],
    ]);
    expect(coders[1].prompt_prefix).toBe(zh.ac_tpl_inventory_researcher);
    expect(Math.min(...coders.slice(1).map((a) => a.dx))).toBeGreaterThan(coders[0].dx);
    expect(rec.template.agents.map((a) => a.kind)).toEqual(["coder", "coder", "coder", "reviewer", "orchestrator"]);
    expect(rec.template.agents.find((a) => a.kind === "reviewer")?.role).toBe(zh.ac_tpl_inventory_reviewer_role);
    expect(rec.summary).toBe("先盤點再轉換");
    expect(rec.reasons).toContainEqual({ label: "EGL 研究工程師", reason: "舊系統是 EGL" });
    expect(rec.reasons).toContainEqual({ label: zh.ac_role_integrator, reason: "合併文件" });
    expect(rec.notes).toEqual([]);
  });

  it("不合規則的自動修正並註明：不認得的角色、第二位產出規格、超過人數、驗收者沒有協調者", () => {
    const rec = parseRecommendation(
      reply({
        coders: [
          { role: "analyst" },
          { role: "spec" },
          { role: "wizard" },
          { role: "coder", count: 9 },
        ],
        reviewer: { role: "nobody" },
        orchestrator: { include: false },
        verifier: { include: true },
      }),
      zh,
    )!;
    const coders = rec.template.agents.filter((a) => a.kind === "coder");
    expect(coders).toHaveLength(MAX_RECOMMENDED_CODERS);
    expect(coders.filter((a) => a.produces_spec)).toHaveLength(1);
    expect(rec.template.agents.map((a) => a.kind)).toContain("orchestrator");
    expect(rec.template.agents.find((a) => a.kind === "reviewer")?.role).toBe(zh.ac_role_general_reviewer);
    expect(rec.notes).toEqual([
      zh.ac_rec_note_one_spec,
      zh.ac_rec_note_unknown_role("wizard"),
      zh.ac_rec_note_max_coders(MAX_RECOMMENDED_CODERS),
      zh.ac_rec_note_unknown_role("nobody"),
      zh.ac_rec_note_orch_for_verifier,
    ]);
  });

  it("有產出規格的人卻沒有協調者：補上協調者", () => {
    const rec = parseRecommendation(reply({ coders: [{ role: "spec" }, { role: "coder" }], reviewer: { role: "general" } }), zh)!;
    expect(rec.template.agents.map((a) => a.kind)).toContain("orchestrator");
    expect(rec.notes).toEqual([zh.ac_rec_note_orch_for_spec]);
  });

  it("前後有多餘的文字也取得到 JSON；不是 JSON 或沒有合法的工程師回 null", () => {
    expect(parseRecommendation('好的，建議如下：\n{"coders":[{"role":"coder"}],"reviewer":{"role":"general"}}\n以上。', zh)).not.toBeNull();
    expect(parseRecommendation("我建議找三位工程師", zh)).toBeNull();
    expect(parseRecommendation(reply({ coders: [{ role: "wizard" }] }), zh)).toBeNull();
  });

  it("名稱過長截斷；數量不合理當成 1", () => {
    const rec = parseRecommendation(reply({ coders: [{ role: "coder", name: "一".repeat(40), count: -3 }] }), zh)!;
    const coders = rec.template.agents.filter((a) => a.kind === "coder");
    expect(coders).toHaveLength(1);
    expect(coders[0].role).toHaveLength(24);
  });
});

describe("提示", () => {
  it("系統提示列出全部角色 key 與內建範本；訊息帶簡介與參考資料夾", () => {
    const sys = buildRecommendSystem(zh);
    for (const key of ["analyst", "inventory_researcher", "combined", "docs"]) expect(sys).toContain(`- ${key}：`);
    expect(sys).toContain("- inventory（舊系統盤點）");
    const msg = buildRecommendMessage("  轉換 EGL 系統 ", [{ path: "/old/src", note: "舊系統原始碼" }]);
    expect(msg).toContain("專案簡介：\n轉換 EGL 系統");
    expect(msg).toContain("- /old/src：舊系統原始碼");
    expect(buildRecommendMessage("x", [])).toContain("參考資料夾：沒有");
  });
});
