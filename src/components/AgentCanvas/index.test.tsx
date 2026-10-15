import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue("/repo"),
  confirm: vi.fn().mockResolvedValue(true),
  message: vi.fn().mockResolvedValue(undefined),
}));

// 抓住 tasks-updated 的回呼，測試裡手動觸發「後端說資料變了」。
let tasksUpdated: (() => void) | null = null;
vi.mock("../../ipc/tasks", () => ({
  listTasks: vi.fn(),
  getTaskBoardConfig: vi.fn().mockResolvedValue({ max_concurrent: 3 }),
  onTasksUpdated: vi.fn((cb: () => void) => {
    tasksUpdated = cb;
    return Promise.resolve(() => {});
  }),
  readTranscript: vi.fn().mockResolvedValue(""),
}));
vi.mock("../../ipc/agents", () => ({
  listAgents: vi.fn(),
  createAgent: vi.fn().mockResolvedValue("new-agent"),
  updateAgent: vi.fn().mockResolvedValue(undefined),
  deleteAgent: vi.fn().mockResolvedValue(undefined),
  resolveAgent: vi.fn().mockResolvedValue(undefined),
  mergeAgent: vi.fn().mockResolvedValue({ status: "merged" }),
  abortAgentMerge: vi.fn().mockResolvedValue(undefined),
  assignAgent: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../ipc/projects", () => ({
  listProjects: vi.fn().mockResolvedValue([
    { id: "p1", name: "HTBKTW", description: "", path: "/p", status: "ok", counts: { planning: 0, queued: 0, running: 0, done: 0 }, error: null },
  ]),
  usedDirs: vi.fn().mockResolvedValue(["/repo/htbktw"]),
  usedLabels: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn().mockResolvedValue([]) }));
vi.mock("../../ipc/bridge", () => ({
  bridgeStatus: vi.fn().mockResolvedValue({ running: false, port: null, token: null, error: null }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(vi.fn()) }));

import { assignAgent, createAgent, listAgents, mergeAgent, resolveAgent, type AgentRow } from "../../ipc/agents";
import { listTasks, type TaskWithAttachments } from "../../ipc/tasks";
import { AgentCanvasView } from "./index";

const agent = (over: Partial<AgentRow> = {}): AgentRow => ({
  id: "a1", kind: "coder", name: "Coder Agent1", role: "Senior Coder", prompt_prefix: "p",
  project_dir: "/repo/htbktw", canvas_x: 40, canvas_y: 40, worktree_path: null, branch: null,
  paused_task_id: null, created_at: "", ...over,
});

const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "t", title: "t", body: "", project_dir: "/r", label: null, status: "planning", parallel_ok: true,
  interactive: false, sort_order: 0, outcome: null, tab_id: null, transcript_path: null, error_message: null,
  created_at: "", dispatched_at: null, finished_at: null, ai_summary: null, archived_at: null, session_id: null,
  session_path: null, use_bridge: false, bridge_tiers: null, worktree_path: null, worktree_branch: null,
  isolate_worktree: null, agent_id: null, start_sha: null, end_sha: null, attachments: [], ...over,
});

const view = () =>
  render(
    <LocaleProvider>
      <AgentCanvasView isActive />
    </LocaleProvider>,
  );

/** jsdom 沒有版面與命中測試，拖曳放開點底下是誰由這裡決定。 */
function drag(from: Element, dropOn: Element) {
  const original = document.elementFromPoint;
  document.elementFromPoint = vi.fn().mockReturnValue(dropOn);
  try {
    fireEvent.mouseDown(from, { clientX: 100, clientY: 100, button: 0 });
    fireEvent.mouseMove(window, { clientX: 140, clientY: 130 });
    fireEvent.mouseUp(window, { clientX: 140, clientY: 130 });
  } finally {
    document.elementFromPoint = original;
  }
}

beforeEach(() => {
  vi.clearAllMocks();
  tasksUpdated = null;
  try {
    localStorage.clear();
  } catch {
    // ignore
  }
});

describe("AgentCanvasView", () => {
  it("顯示 Agent、指派給它的卡片，與模組庫裡未指派的計畫中卡片", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A 登入", status: "queued", agent_id: "a1" }),
      card({ id: "C", title: "C 訂單", status: "planning" }),
      card({ id: "Q", title: "不屬於 Agent 的待執行卡", status: "queued" }),
    ]);
    view();
    const node = await screen.findByTestId("agent-a1");
    expect(within(node).getByText(/A 登入/)).toBeTruthy();
    expect(screen.getByTestId("module-C")).toBeTruthy();
    expect(screen.queryByTestId("module-Q")).toBeNull();
  });

  it("把模組拖到 Agent 上會指派給它", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([card({ id: "C", title: "C 訂單", status: "planning" })]);
    view();
    const mod = await screen.findByTestId("module-C");
    drag(mod, screen.getByTestId("agent-a1"));
    await waitFor(() => expect(assignAgent).toHaveBeenCalledWith("p1", "C", "a1"));
  });

  it("把待執行的 chip 拖回模組庫會取消指派", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([card({ id: "B", title: "B", status: "queued", agent_id: "a1" })]);
    view();
    const chip = await screen.findByTestId("chip-B");
    const lib = document.querySelector('[data-drop="library"]') as Element;
    drag(chip, lib);
    await waitFor(() => expect(assignAgent).toHaveBeenCalledWith("p1", "B", null));
  });

  it("從 Agent 池拖到桌布會開指派角色對話框，送出才建立", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    const pool = await screen.findByTestId("pool-coder");
    drag(pool, document.querySelector('[data-drop="canvas"]') as Element);
    const dlg = await screen.findByRole("dialog");
    expect(createAgent).not.toHaveBeenCalled();
    await waitFor(() =>
      expect((within(dlg).getByLabelText(/程式庫|Repository/) as HTMLInputElement).value).toBe("/repo/htbktw"),
    );
    fireEvent.click(within(dlg).getByText("Bug Fixer"));
    fireEvent.click(within(dlg).getByText(/放上桌布|Place on canvas/));
    await waitFor(() =>
      expect(createAgent).toHaveBeenCalledWith(
        "p1",
        expect.objectContaining({ kind: "coder", role: "Bug Fixer", project_dir: "/repo/htbktw" }),
      ),
    );
  });

  it("建立失敗（例如不是 git repo）錯誤留在對話框裡", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(createAgent).mockRejectedValueOnce("Agent 的程式庫必須是 git repo：/x");
    view();
    drag(await screen.findByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
    const dlg = await screen.findByRole("dialog");
    await waitFor(() => expect((within(dlg).getByLabelText(/程式庫|Repository/) as HTMLInputElement).value).not.toBe(""));
    fireEvent.click(within(dlg).getByText(/放上桌布|Place on canvas/));
    expect(await within(dlg).findByRole("alert")).toHaveTextContent("必須是 git repo");
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("tasks-updated 會重新載入", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    await screen.findByTestId("agent-a1");
    const before = vi.mocked(listAgents).mock.calls.length;
    tasksUpdated?.();
    await waitFor(() => expect(vi.mocked(listAgents).mock.calls.length).toBeGreaterThan(before));
  });

  it("暫停中的 Agent 顯示失敗卡片，重試與跳過呼叫 agents_resolve", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ paused_task_id: "F" })]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "F", title: "F 設定", status: "done", outcome: "failed", error_message: "boom", agent_id: "a1" }),
    ]);
    view();
    const node = await screen.findByTestId("agent-a1");
    expect(within(node).getByRole("alert")).toHaveTextContent("boom");
    fireEvent.click(within(node).getByText(/重試|Retry/));
    await waitFor(() => expect(resolveAgent).toHaveBeenCalledWith("p1", "a1", "retry"));
    fireEvent.click(within(node).getByText(/跳過|Skip/));
    await waitFor(() => expect(resolveAgent).toHaveBeenCalledWith("p1", "a1", "skip"));
  });

  it("有分支且閒置時可以合併，衝突時顯示還原合併", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ branch: "aiterm-agent/a1" })]);
    vi.mocked(listTasks).mockResolvedValue([card({ id: "A", status: "done", outcome: "success", agent_id: "a1" })]);
    vi.mocked(mergeAgent).mockResolvedValueOnce({ status: "conflict", files: ["x.ts"] });
    view();
    const node = await screen.findByTestId("agent-a1");
    fireEvent.click(within(node).getByText(/合併分支|Merge branch/));
    await waitFor(() => expect(mergeAgent).toHaveBeenCalledWith("p1", "a1"));
    expect(await within(node).findByRole("button", { name: /還原合併|Abort merge/ })).toBeTruthy();
  });

  it("執行中的 Agent 有 coding 狀態與經過時間", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A 登入", status: "running", agent_id: "a1", dispatched_at: Math.floor(Date.now() / 1000) - 75 }),
    ]);
    view();
    const node = await screen.findByTestId("agent-a1");
    expect(node.className).toContain("ac-agent--coding");
    expect(within(node).getByText(/1:1[5-9]/)).toBeTruthy();
  });
});
