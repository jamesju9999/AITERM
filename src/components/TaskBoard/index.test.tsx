import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn().mockResolvedValue("/repo") }));

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

  // Regression test for a real bug: Tauri's window-level `dragDropEnabled`
  // (default true, not overridden in tauri.conf.json — see DocConverterView's
  // reliance on the native `tauri://drag-drop` event, and TabBar's own
  // deliberate avoidance of HTML5 DnD for its tab-reorder drag) intercepts
  // any native OS drag session before the DOM's dragstart/dragover/drop ever
  // fire. A `fireEvent.dragStart`/`fireEvent.drop` test — which never invokes
  // a real OS drag session — could pass forever while the feature is
  // completely broken in the real app. This test drives the same mouse-event
  // mechanism (mousedown → mousemove past a threshold → mouseup) that
  // TabBar's proven-working tab reorder uses, with `document.elementFromPoint`
  // stubbed (jsdom doesn't implement layout/hit-testing) to report the queued
  // column under the release point.
  it("dropping a planning card on the queued column calls moveTask", async () => {
    vi.mocked(listTasks).mockResolvedValue([card({ id: "p", title: "Draggable", status: "planning" })]);
    view();
    const cardEl = await screen.findByText("Draggable");
    const queuedCol = screen.getByTestId("column-queued");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;
    expect(dragWrap).toBeTruthy();

    const originalElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = vi.fn().mockReturnValue(queuedCol);
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });
      fireEvent.mouseMove(window, { clientX: 100, clientY: 120 }); // past the drag threshold
      fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
      await waitFor(() => expect(moveTask).toHaveBeenCalledWith("p", "queued", expect.any(Number)));
    } finally {
      document.elementFromPoint = originalElementFromPoint;
    }
  });

  // Regression test for a real UX bug found manually after the fix above:
  // dragging gave zero visual feedback (no cursor-following ghost, no fade on
  // the source card), which made the interaction feel broken even once the
  // underlying mechanism worked — the user couldn't tell a drag was in
  // progress. This asserts the dragged card's wrapper gets a visible
  // "dragging" marker once the threshold is crossed, and loses it on release.
  it("marks the dragged card's wrapper while a drag is in progress", async () => {
    vi.mocked(listTasks).mockResolvedValue([card({ id: "p", title: "Draggable", status: "planning" })]);
    view();
    const cardEl = await screen.findByText("Draggable");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;

    const originalElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = vi.fn().mockReturnValue(null);
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });
      expect(dragWrap.className).not.toContain("task-card-drag-wrap--dragging");
      fireEvent.mouseMove(window, { clientX: 100, clientY: 120 }); // past the drag threshold
      expect(dragWrap.className).toContain("task-card-drag-wrap--dragging");
      fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
      expect(dragWrap.className).not.toContain("task-card-drag-wrap--dragging");
    } finally {
      document.elementFromPoint = originalElementFromPoint;
    }
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

  it("new-card dialog creates a task with the typed fields", async () => {
    const { createTask } = await import("../../ipc/tasks");
    vi.mocked(createTask).mockResolvedValue("id-new");
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /新增工作|New task/ }));
    await user.type(screen.getByLabelText(/標題|Title/), "Ship it");
    await user.type(screen.getByLabelText(/工作內容|Task detail/), "do the thing");
    await user.type(screen.getByLabelText(/專案資料夾|Project folder/), "/repo");
    await user.click(screen.getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() =>
      expect(createTask).toHaveBeenCalledWith(
        expect.objectContaining({ title: "Ship it", body: "do the thing", project_dir: "/repo", parallel_ok: true }),
      ),
    );
  });

  it("editing an existing planning card calls updateTask", async () => {
    const { updateTask } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([card({ id: "p", title: "Old", status: "planning", project_dir: "/r" })]);
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /編輯工作|Edit task/ }));
    const title = screen.getByLabelText(/標題|Title/);
    await user.clear(title);
    await user.type(title, "New title");
    await user.click(screen.getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() =>
      expect(updateTask).toHaveBeenCalledWith(expect.objectContaining({ id: "p", title: "New title" })),
    );
  });

  it("transcript dialog shows the backend transcript text", async () => {
    const { readTranscript } = await import("../../ipc/tasks");
    vi.mocked(readTranscript).mockResolvedValue("line A\nline B");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Done one", status: "done", outcome: "success", transcript_path: "/p/t.txt" }),
    ]);
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /查看逆紀錄|View transcript/ }));
    expect(await screen.findByText(/line A/)).toBeInTheDocument();
  });
});
