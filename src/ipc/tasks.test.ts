import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

import { invoke } from "@tauri-apps/api/core";
import {
  listTasks,
  createTask,
  moveTask,
  stopTask,
  onTasksUpdated,
  saveTranscript,
  markTaskDone,
} from "./tasks";

beforeEach(() => vi.mocked(invoke).mockReset());

describe("ipc/tasks", () => {
  it("listTasks calls the tasks_list command with projectId", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await listTasks("proj1");
    expect(invoke).toHaveBeenCalledWith("tasks_list", { projectId: "proj1" });
  });

  it("createTask forwards projectId and fields under an args key", async () => {
    vi.mocked(invoke).mockResolvedValue("new-id");
    const id = await createTask("proj1", {
      title: "t",
      body: "b",
      project_dir: "/r",
      parallel_ok: true,
      interactive: true,
    });
    expect(id).toBe("new-id");
    expect(invoke).toHaveBeenCalledWith("tasks_create", {
      projectId: "proj1",
      args: { title: "t", body: "b", project_dir: "/r", parallel_ok: true, interactive: true },
    });
  });

  it("moveTask forwards projectId and id/to_status/sort_order", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await moveTask("proj1", "id1", "queued", 1.5);
    expect(invoke).toHaveBeenCalledWith("tasks_move", {
      projectId: "proj1",
      args: { id: "id1", to_status: "queued", sort_order: 1.5 },
    });
  });

  it("stopTask forwards projectId and a bare id", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await stopTask("proj1", "id1");
    expect(invoke).toHaveBeenCalledWith("tasks_stop", { projectId: "proj1", id: "id1" });
  });

  it("onTasksUpdated subscribes to the tasks-updated event", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    await onTasksUpdated(() => {});
    expect(listen).toHaveBeenCalledWith("tasks-updated", expect.any(Function));
  });

  it("saveTranscript forwards projectId, id and text as bare params", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await saveTranscript("proj1", "id1", "clean text");
    expect(invoke).toHaveBeenCalledWith("tasks_save_transcript", {
      projectId: "proj1",
      id: "id1",
      text: "clean text",
    });
  });

  it("markTaskDone forwards projectId and the id as bare params", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await markTaskDone("proj1", "id1");
    expect(invoke).toHaveBeenCalledWith("tasks_mark_done", { projectId: "proj1", id: "id1" });
  });
});
