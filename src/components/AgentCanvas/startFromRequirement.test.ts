import { describe, expect, it, vi } from "vitest";

import { translations } from "../../lib/i18n";
import { builtinTemplates } from "./teamTemplates";
import { pickFirstStation, startFromRequirement, type StartDeps } from "./startFromRequirement";

const t = translations.en;
const standard = builtinTemplates(t).find((x) => x.id === "standard")!;
const specFirst = builtinTemplates(t).find((x) => x.id === "spec_first")!;

function deps(over: Partial<StartDeps> = {}): StartDeps {
  let n = 0;
  return {
    getBrief: vi.fn().mockResolvedValue(null),
    setBrief: vi.fn().mockResolvedValue(undefined),
    getReferences: vi.fn().mockResolvedValue([]),
    complete: vi.fn().mockRejectedValue(new Error("no ai")),
    createAgent: vi.fn().mockImplementation(async () => `a${++n}`),
    createTask: vi.fn().mockResolvedValue("task-1"),
    assignAgent: vi.fn().mockResolvedValue(undefined),
    ...over,
  };
}

const base = { projectId: "p", repoDir: "/repo", t, providerId: "prov" };

describe("pickFirstStation", () => {
  it("prefers the spec producer", () => {
    const s = pickFirstStation([
      { id: "c1", kind: "coder", produces_spec: false },
      { id: "s1", kind: "coder", produces_spec: true },
    ]);
    expect(s).toBe("s1");
  });
  it("falls back to the first ordinary coder", () => {
    expect(
      pickFirstStation([
        { id: "r", kind: "reviewer", produces_spec: false },
        { id: "c1", kind: "coder", produces_spec: false },
        { id: "c2", kind: "coder", produces_spec: false },
      ]),
    ).toBe("c1");
  });
  it("returns null without a coder", () => {
    expect(pickFirstStation([{ id: "r", kind: "reviewer", produces_spec: false }])).toBeNull();
  });
});

describe("startFromRequirement", () => {
  it("writes the requirement as the brief only when the brief is empty", async () => {
    const d = deps({ getBrief: vi.fn().mockResolvedValue("  ") });
    await startFromRequirement({ ...base, requirement: "build a todo app" }, d);
    expect(d.setBrief).toHaveBeenCalledWith("p", "build a todo app");

    const d2 = deps({ getBrief: vi.fn().mockResolvedValue("existing brief") });
    await startFromRequirement({ ...base, requirement: "build a todo app" }, d2);
    expect(d2.setBrief).not.toHaveBeenCalled();
  });

  it("falls back to the standard template when the AI fails, and says so", async () => {
    const d = deps();
    const r = await startFromRequirement({ ...base, requirement: "x" }, d);
    expect(r.usedDefault).toBe(true);
    expect(d.createAgent).toHaveBeenCalledTimes(standard.agents.length);
  });

  it("falls back when the AI reply cannot be parsed", async () => {
    const d = deps({ complete: vi.fn().mockResolvedValue("not json") });
    const r = await startFromRequirement({ ...base, requirement: "x" }, d);
    expect(r.usedDefault).toBe(true);
  });

  it("uses the AI recommendation when valid and feeds brief + requirement to it", async () => {
    const reply = JSON.stringify({
      base: "standard",
      summary: "one coder",
      coders: [{ role: "coder", count: 1, reason: "simple" }],
      reviewer: { role: "general", reason: "check" },
      orchestrator: { include: false },
      verifier: { include: false },
    });
    const d = deps({ complete: vi.fn().mockResolvedValue(reply), getBrief: vi.fn().mockResolvedValue("my brief") });
    const r = await startFromRequirement({ ...base, requirement: "add login" }, d);
    expect(r.usedDefault).toBe(false);
    expect(r.summary).toBe("one coder");
    const sent = (d.complete as ReturnType<typeof vi.fn>).mock.calls[0][0][0].content as string;
    expect(sent).toContain("my brief");
    expect(sent).toContain("add login");
    expect(d.createAgent).toHaveBeenCalledTimes(2); // coder + reviewer
  });

  it("creates the first card with the requirement and assigns it to the first station", async () => {
    // 規格優先的團隊：卡片要派給產出規格的人，不是一般 Coder。
    const d = deps({ complete: vi.fn().mockRejectedValue(new Error("x")) });
    await startFromRequirement({ ...base, requirement: "line one\nline two", fallback: specFirst }, d);
    const taskArgs = (d.createTask as ReturnType<typeof vi.fn>).mock.calls[0][1];
    expect(taskArgs.title).toBe("line one");
    expect(taskArgs.body).toContain("line two");
    expect(taskArgs.project_dir).toBe("/repo");
    const specIdx = specFirst.agents.findIndex((a) => a.produces_spec);
    const specId = (d.createAgent as ReturnType<typeof vi.fn>).mock.results[specIdx];
    expect(d.assignAgent).toHaveBeenCalledWith("p", "task-1", await specId.value);
  });

  it("marks the failure stage so the UI can say the team already exists", async () => {
    const d = deps({ createTask: vi.fn().mockRejectedValue(new Error("boom")) });
    await expect(startFromRequirement({ ...base, requirement: "x" }, d)).rejects.toMatchObject({ teamCreated: true });

    const d2 = deps({ createAgent: vi.fn().mockRejectedValue(new Error("nope")) });
    await expect(startFromRequirement({ ...base, requirement: "x" }, d2)).rejects.toMatchObject({ teamCreated: false });
  });
});
