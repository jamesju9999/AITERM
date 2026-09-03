import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/tasks", () => ({
  listTasks: vi.fn(),
  onTasksUpdated: vi.fn().mockResolvedValue(() => {}),
  moveTask: vi.fn().mockResolvedValue(undefined),
  stopTask: vi.fn().mockResolvedValue(undefined),
  deleteTask: vi.fn().mockResolvedValue(undefined),
  readTranscript: vi.fn().mockResolvedValue(""),
  createTask: vi.fn(),
  updateTask: vi.fn(),
  cloneTask: vi.fn().mockResolvedValue("new-id"),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));

import { listTasks, onTasksUpdated, moveTask } from "../../ipc/tasks";
import type { TaskWithAttachments } from "../../ipc/tasks";
import { TaskBoardView } from "./index";

const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "c1", title: "Card one", body: "", project_dir: "/r", status: "planning",
  parallel_ok: true, sort_order: 1, outcome: null, tab_id: null, transcript_path: null,
  error_message: null, created_at: "", dispatched_at: null, finished_at: null, attachments: [],
  ...over,
});

const view = () => render(<LocaleProvider><TaskBoardView /></LocaleProvider>);

beforeEach(() => {
  // Repo vitest config does not set `clearMocks`, so call counts would leak
  // across tests in this file (the "toHaveBeenCalledTimes(1)" assertion below).
  vi.clearAllMocks();
  vi.mocked(listTasks).mockResolvedValue([]);
  vi.mocked(onTasksUpdated).mockResolvedValue(() => {});
});

describe("TaskBoardView", () => {
  it("renders four columns", async () => {
    view();
    await waitFor(() => expect(screen.getByText(/計畫中|Planned/)).toBeInTheDocument());
    expect(screen.getByText(/待執行|Queued/)).toBeInTheDocument();
    expect(screen.getByText(/執行中|Running/)).toBeInTheDocument();
    expect(screen.getByText(/已完成|Done/)).toBeInTheDocument();
  });

  it("places each card in its status column", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "p", title: "PlanCard", status: "planning" }),
      card({ id: "r", title: "RunCard", status: "running" }),
    ]);
    view();
    const planning = await screen.findByTestId("column-planning");
    const running = screen.getByTestId("column-running");
    expect(within(planning).getByText("PlanCard")).toBeInTheDocument();
    expect(within(running).getByText("RunCard")).toBeInTheDocument();
  });

  it("re-fetches when tasks-updated fires", async () => {
    let fire: () => void = () => {};
    vi.mocked(onTasksUpdated).mockImplementation(async (cb) => { fire = cb; return () => {}; });
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    await waitFor(() => expect(listTasks).toHaveBeenCalledTimes(1));
    vi.mocked(listTasks).mockResolvedValue([card({ id: "x", title: "Appeared", status: "queued" })]);
    fire();
    expect(await screen.findByText("Appeared")).toBeInTheDocument();
  });

  it("dropping a planning card on the queued column calls moveTask", async () => {
    vi.mocked(listTasks).mockResolvedValue([card({ id: "p", title: "Draggable", status: "planning" })]);
    view();
    const cardEl = await screen.findByText("Draggable");
    const queuedCol = screen.getByTestId("column-queued");
    const draggable = cardEl.closest("[draggable]") as HTMLElement;
    const { fireEvent } = await import("@testing-library/react");
    fireEvent.dragStart(draggable);
    fireEvent.drop(queuedCol);
    await waitFor(() => expect(moveTask).toHaveBeenCalledWith("p", "queued", expect.any(Number)));
  });

  it("running card shows Stop, and Stop calls stopTask", async () => {
    const { stopTask } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([card({ id: "r", title: "Runner", status: "running", tab_id: "tab-1" })]);
    view();
    const user = userEvent.setup();
    await screen.findByText("Runner");
    await user.click(screen.getByRole("button", { name: /停止|Stop/ }));
    expect(stopTask).toHaveBeenCalledWith("r");
  });

  it("done+failed card shows the failed badge and its error message", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Broke", status: "done", outcome: "failed", error_message: "claude 以 exit code 127 結束" }),
    ]);
    view();
    expect(await screen.findByText(/失敗|Failed/)).toBeInTheDocument();
    expect(screen.getByText(/exit code 127/)).toBeInTheDocument();
  });

  it("done card re-dispatch calls cloneTask", async () => {
    const { cloneTask } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([card({ id: "d", title: "Redo me", status: "done", outcome: "success" })]);
    view();
    const user = userEvent.setup();
    await screen.findByText("Redo me");
    await user.click(screen.getByRole("button", { name: /重新派工|Re-dispatch/ }));
    expect(cloneTask).toHaveBeenCalledWith("d");
  });
});
