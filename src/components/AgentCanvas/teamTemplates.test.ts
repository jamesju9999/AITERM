import { beforeEach, describe, expect, it } from "vitest";

import type { AgentRow } from "../../ipc/agents";
import { translations } from "../../lib/i18n";
import { builtinTemplates, deleteTemplate, loadTemplates, saveTemplate, snapshot, toCreate } from "./teamTemplates";

const ag = (over: Partial<AgentRow>): AgentRow => ({
  id: "x", kind: "coder", name: "工程師1", role: "資深", prompt_prefix: "p", project_dir: "/old", canvas_x: 100, canvas_y: 50,
  worktree_path: null, branch: null, paused_task_id: null, created_at: "", last_merge_signature: null, produces_spec: false,
  model: null, ...over,
});

beforeEach(() => localStorage.clear());

describe("teamTemplates", () => {
  const team = [
    ag({ id: "c1" }),
    ag({ id: "c2", name: "規格", produces_spec: true, canvas_x: 100, canvas_y: 300 }),
    ag({ id: "r", kind: "reviewer", name: "審核者", model: "sonnet", canvas_x: 400, canvas_y: 50 }),
  ];

  it("存下角色設定與相對位置，不存程式庫", () => {
    const tpl = snapshot("兩工程師＋審核", team);
    expect(tpl.agents.map((a) => [a.kind, a.name, a.dx, a.dy, a.model, a.produces_spec])).toEqual([
      ["coder", "工程師1", 0, 0, "", false],
      ["coder", "規格", 0, 250, "", true],
      ["reviewer", "審核者", 300, 0, "sonnet", false],
    ]);
    expect(JSON.stringify(tpl)).not.toContain("/old");
  });

  it("套用：同一個程式庫、從指定位置擺；已經有的單一角色不重複建", () => {
    const tpl = snapshot("t", team);
    const out = toCreate(tpl, [ag({ kind: "reviewer", name: "審核者" })], "/repo", 40, 40);
    expect(out.map((a) => [a.kind, a.name, a.project_dir, a.canvas_x, a.canvas_y])).toEqual([
      ["coder", "工程師1", "/repo", 40, 40],
      ["coder", "規格", "/repo", 40, 290],
    ]);
    expect(toCreate(tpl, [], "/repo", 0, 0).map((a) => a.kind)).toContain("reviewer");
  });

  it("記住每位 Agent 的 AI 工具；舊範本沒有就是 Claude Code", () => {
    const tpl = snapshot("t", [ag({ id: "c1", cli: "codex", model: "qwen" }), ag({ id: "c2" })]);
    expect(toCreate(tpl, [], "/repo", 0, 0).map((a) => a.cli)).toEqual(["codex", "claude"]);
    const old = { name: "舊", agents: tpl.agents.map((a) => ({ ...a, cli: undefined })) };
    expect(toCreate(old, [], "/repo", 0, 0).map((a) => a.cli)).toEqual(["claude", "claude"]);
  });

  it("存在這台機器、同名覆蓋、可以刪除", () => {
    saveTemplate(snapshot("A", team));
    saveTemplate(snapshot("B", team.slice(0, 1)));
    saveTemplate(snapshot("A", team.slice(0, 2)));
    expect(loadTemplates().map((t) => [t.name, t.agents.length])).toEqual([["B", 1], ["A", 2]]);
    expect(deleteTemplate("B").map((t) => t.name)).toEqual(["A"]);
    localStorage.setItem("aiterm.agentCanvas.teamTemplates", "not json");
    expect(loadTemplates()).toEqual([]);
  });
});

describe("builtinTemplates", () => {
  const zh = translations["zh-TW"];
  const all = builtinTemplates(zh);

  it("五組內建範本，各有用途說明", () => {
    expect(all.map((x) => x.id)).toEqual(["quick", "bugfix", "standard", "spec_first", "migration"]);
    for (const tpl of all) {
      expect(tpl.builtin).toBe(true);
      expect(tpl.description.length).toBeGreaterThan(0);
      expect(tpl.agents.some((a) => a.kind === "coder")).toBe(true);
    }
  });

  it("有驗收者就一定有協調者（驗收在協調者合併之後才開工）", () => {
    for (const tpl of all) {
      if (tpl.agents.some((a) => a.kind === "verifier")) {
        expect(tpl.agents.some((a) => a.kind === "orchestrator"), tpl.name).toBe(true);
      }
    }
  });

  it("每個範本裡單一角色最多一位、名稱不重複；規格先行有一位產出規格的工程師", () => {
    for (const tpl of all) {
      for (const k of ["reviewer", "orchestrator", "verifier"] as const) {
        expect(tpl.agents.filter((a) => a.kind === k).length, `${tpl.name} ${k}`).toBeLessThanOrEqual(1);
      }
      expect(new Set(tpl.agents.map((a) => a.name)).size, tpl.name).toBe(tpl.agents.length);
    }
    const spec = all.find((x) => x.id === "spec_first");
    expect(spec?.agents.filter((a) => a.produces_spec)).toHaveLength(1);
  });

  it("角色與提示詞取自內建角色；審核者與驗收者預設 sonnet", () => {
    const std = all.find((x) => x.id === "standard");
    const fe = std?.agents.find((a) => a.role === zh.ac_role_frontend);
    expect(fe?.prompt_prefix).toBe(zh.ac_role_frontend_desc);
    expect(std?.agents.find((a) => a.kind === "reviewer")?.model).toBe("sonnet");
    expect(std?.agents.find((a) => a.kind === "verifier")?.model).toBe("sonnet");
  });

  it("切換語系時跟著換", () => {
    expect(builtinTemplates(translations.en)[0].name).not.toBe(all[0].name);
  });
});

describe("toCreate 名稱不重複", () => {
  it("桌布上已有同名的工程師時，新的往後編號", () => {
    const tpl = { name: "x", agents: [
      { kind: "coder" as const, name: "工程師1", role: "", prompt_prefix: "", model: "", produces_spec: false, dx: 0, dy: 0 },
      { kind: "coder" as const, name: "工程師2", role: "", prompt_prefix: "", model: "", produces_spec: false, dx: 0, dy: 190 },
    ] };
    const made = toCreate(tpl, [ag({ id: "a", name: "工程師1" })], "/r", 0, 0);
    // 「工程師1」已存在 → 改用下一個沒人用的「工程師2」；範本原本的「工程師2」因此順延成「工程師3」。
    expect(made.map((m) => m.name)).toEqual(["工程師2", "工程師3"]);
  });
});
