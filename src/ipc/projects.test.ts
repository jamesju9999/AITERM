import { describe, expect, it, vi, beforeEach } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { createProject, listProjects, removeProject, usedDirs } from "./projects";

describe("projects ipc", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue([]);
  });

  it("listProjects 呼叫 projects_list", async () => {
    await listProjects();
    expect(invoke).toHaveBeenCalledWith("projects_list");
  });

  it("createProject 以 snake_case 傳遞 args", async () => {
    invoke.mockResolvedValue("new-id");
    const id = await createProject({ parentDir: "/p", name: "n", description: "d" });
    expect(id).toBe("new-id");
    expect(invoke).toHaveBeenCalledWith("projects_create", {
      args: { parent_dir: "/p", name: "n", description: "d" },
    });
  });

  it("removeProject 傳遞 delete_folder", async () => {
    invoke.mockResolvedValue(undefined);
    await removeProject("pid", true);
    expect(invoke).toHaveBeenCalledWith("projects_remove", {
      args: { id: "pid", delete_folder: true },
    });
  });

  it("usedDirs 傳遞 projectId", async () => {
    invoke.mockResolvedValue(["/a"]);
    await expect(usedDirs("pid")).resolves.toEqual(["/a"]);
    expect(invoke).toHaveBeenCalledWith("tasks_used_dirs", { projectId: "pid" });
  });
});
