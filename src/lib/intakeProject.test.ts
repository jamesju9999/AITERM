import { describe, expect, it } from "vitest";
import type { IntakeProject } from "../ipc/intake";
import { isUnder, projectForCwd } from "./intakeProject";

const p = (id: string, repo: string | null): IntakeProject => ({ id, name: id, repo_dir: repo, open_count: 0, total: 0, enabled: true });

describe("intakeProject", () => {
  it("isUnder respects directory boundaries, case and slash direction", () => {
    expect(isUnder("/work/app/src", "/work/app")).toBe(true);
    expect(isUnder("/work/app", "/work/app/")).toBe(true);
    expect(isUnder("/work/application", "/work/app")).toBe(false);
    expect(isUnder("C:\\Work\\App\\src", "c:/work/app")).toBe(true);
    expect(isUnder("/anything", "")).toBe(false);
  });

  it("picks the project whose directory contains the tab, preferring the deepest", () => {
    const projects = [p("a", "/work"), p("b", "/work/app"), p("c", null)];
    expect(projectForCwd(projects, "/work/app/src")?.id).toBe("b");
    expect(projectForCwd(projects, "/work/other")?.id).toBe("a");
    expect(projectForCwd(projects, "/elsewhere")).toBeNull();
    expect(projectForCwd(projects, null)).toBeNull();
  });
});
