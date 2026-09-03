import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

import { invoke } from "@tauri-apps/api/core";
import { listTasks, createTask, moveTask, stopTask, onTasksUpdated, saveTranscript } from "./tasks";

beforeEach(() => vi.mocked(invoke).mockReset());

describe("ipc/tasks", () => {
  it("listTasks calls the tasks_list command", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await listTasks();
    expect(invoke).toHaveBeenCalledWith("tasks_list");
  });

  it("createTask forwards fields under an args key", async () => {
    vi.mocked(invoke).mockResolvedValue("new-id");
    const id = await createTask({ title: "t", body: "b", project_dir: "/r", parallel_ok: true });
    expect(id).toBe("new-id");
    expect(invoke).toHaveBeenCalledWith("tasks_create", {
      args: { title: "t", body: "b", project_dir: "/r", parallel_ok: true },
    });
  });

  it("moveTask forwards id/to_status/sort_order", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await moveTask("id1", "queued", 1.5);
    expect(invoke).toHaveBeenCalledWith("tasks_move", {
      args: { id: "id1", to_status: "queued", sort_order: 1.5 },
    });
  });

  it("stopTask forwards a bare id", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await stopTask("id1");
    expect(invoke).toHaveBeenCalledWith("tasks_stop", { id: "id1" });
  });

  it("onTasksUpdated subscribes to the tasks-updated event", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    await onTasksUpdated(() => {});
    expect(listen).toHaveBeenCalledWith("tasks-updated", expect.any(Function));
  });

  it("saveTranscript forwards id and text as bare params", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await saveTranscript("id1", "clean text");
    expect(invoke).toHaveBeenCalledWith("tasks_save_transcript", { id: "id1", text: "clean text" });
  });
});
