import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import {
  assignAgent,
  createAgent,
  deleteAgent,
  listAgents,
  mergeAgent,
  resolveAgent,
  updateAgent,
} from "./agents";

beforeEach(() => vi.mocked(invoke).mockReset());

// Tauri 把頂層的 camelCase 參數名轉成 Rust 的 snake_case（projectId →
// project_id），但 `args` 物件裡的欄位原樣傳，所以那裡必須是 snake_case
// 才對得上 Rust 的 Deserialize。這兩條規則都靠這裡守。
describe("ipc/agents", () => {
  it("listAgents", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await listAgents("p1");
    expect(invoke).toHaveBeenCalledWith("agents_list", { projectId: "p1" });
  });

  it("createAgent sends snake_case fields under args", async () => {
    vi.mocked(invoke).mockResolvedValue("a1");
    const args = {
      kind: "coder" as const, name: "Coder Agent1", role: "Senior Coder",
      prompt_prefix: "p", project_dir: "/r", canvas_x: 1, canvas_y: 2,
    };
    expect(await createAgent("p1", args)).toBe("a1");
    expect(invoke).toHaveBeenCalledWith("agents_create", { projectId: "p1", args });
  });

  it("updateAgent / deleteAgent", async () => {
    const args = { id: "a1", name: "n", role: "r", prompt_prefix: "", canvas_x: 0, canvas_y: 0 };
    await updateAgent("p1", args);
    expect(invoke).toHaveBeenCalledWith("agents_update", { projectId: "p1", args });
    await deleteAgent("p1", "a1");
    expect(invoke).toHaveBeenCalledWith("agents_delete", { projectId: "p1", id: "a1" });
  });

  it("resolveAgent passes the action", async () => {
    await resolveAgent("p1", "a1", "skip");
    expect(invoke).toHaveBeenCalledWith("agents_resolve", { projectId: "p1", id: "a1", action: "skip" });
  });

  it("mergeAgent", async () => {
    vi.mocked(invoke).mockResolvedValue({ status: "merged" });
    expect(await mergeAgent("p1", "a1")).toEqual({ status: "merged" });
    expect(invoke).toHaveBeenCalledWith("agents_merge", { projectId: "p1", id: "a1" });
  });

  it("assignAgent uses camelCase top-level params and allows null to unassign", async () => {
    await assignAgent("p1", "t1", "a1");
    expect(invoke).toHaveBeenCalledWith("tasks_assign_agent", { projectId: "p1", taskId: "t1", agentId: "a1", fast: false });
    await assignAgent("p1", "t1", null);
    expect(invoke).toHaveBeenLastCalledWith("tasks_assign_agent", { projectId: "p1", taskId: "t1", agentId: null, fast: false });
    await assignAgent("p1", "t1", "a1", true);
    expect(invoke).toHaveBeenLastCalledWith("tasks_assign_agent", { projectId: "p1", taskId: "t1", agentId: "a1", fast: true });
  });
});
