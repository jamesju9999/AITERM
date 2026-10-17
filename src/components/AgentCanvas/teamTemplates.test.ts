import { beforeEach, describe, expect, it } from "vitest";

import type { AgentRow } from "../../ipc/agents";
import { deleteTemplate, loadTemplates, saveTemplate, snapshot, toCreate } from "./teamTemplates";

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
    const out = toCreate(tpl, [ag({ kind: "reviewer" })], "/repo", 40, 40);
    expect(out.map((a) => [a.kind, a.name, a.project_dir, a.canvas_x, a.canvas_y])).toEqual([
      ["coder", "工程師1", "/repo", 40, 40],
      ["coder", "規格", "/repo", 40, 290],
    ]);
    expect(toCreate(tpl, [], "/repo", 0, 0).map((a) => a.kind)).toContain("reviewer");
  });

  it("記住每位 Agent 的 AI 工具；舊範本沒有就是 Claude Code", () => {
    const tpl = snapshot("t", [ag({ id: "c1", cli: "codex", model: "qwen" }), ag({ id: "c2" })]);
    expect(toCreate(tpl, [], "/repo", 0, 0).map((a) => a.cli)).toEqual(["codex", "claude"]);
    const old = { name: "舊", agents: tpl.agents.map(({ cli: _cli, ...rest }) => rest) };
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
