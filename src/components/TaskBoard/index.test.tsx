import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue("/repo"),
  confirm: vi.fn().mockResolvedValue(true),
}));

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
  saveTranscript: vi.fn().mockResolvedValue(undefined),
  markTaskDone: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: vi.fn(),
}));

import { listTasks, onTasksUpdated, moveTask } from "../../ipc/tasks";
import type { TaskWithAttachments } from "../../ipc/tasks";
import { serializeTerminal } from "../../lib/terminalInstanceRegistry";
import { TaskBoardView } from "./index";

const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "c1", title: "Card one", body: "", project_dir: "/r", status: "planning",
  parallel_ok: true, interactive: false, sort_order: 1, outcome: null, tab_id: null,
  transcript_path: null, error_message: null, created_at: "", dispatched_at: null,
  finished_at: null, attachments: [],
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

  // User-requested behavior: the card should visibly "leave" its column and
  // follow the cursor while dragging. Regression test for a real bug found
  // manually right after the first attempt at this (an in-place `transform`
  // on the card's own wrapper): every column has `overflow: hidden`, so a
  // card translated past its own column's edge got visually clipped the
  // instant it crossed into a neighboring column — exactly the "being
  // squashed underneath" the user reported. The fix renders a separate
  // "ghost" element via a portal into `document.body`, positioned with
  // `position: fixed` at the live cursor coordinates — a sibling of every
  // column, not a clipped descendant of one.
  it("the dragged card renders a cursor-following ghost outside any column's clipping", async () => {
    vi.mocked(listTasks).mockResolvedValue([card({ id: "p", title: "Draggable", status: "planning" })]);
    view();
    const cardEl = await screen.findByText("Draggable");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;

    const originalElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = vi.fn().mockReturnValue(null);
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });
      expect(screen.queryByTestId("task-drag-ghost")).not.toBeInTheDocument();

      fireEvent.mouseMove(window, { clientX: 130, clientY: 150 }); // past the drag threshold
      const ghost = await screen.findByTestId("task-drag-ghost");
      // Rendered by a portal — not nested inside the source column (or any
      // column), so no ancestor's overflow:hidden can clip it.
      expect(ghost.closest(".task-column")).toBeNull();
      expect(ghost.style.left).toBe("130px");
      expect(ghost.style.top).toBe("150px");
      expect(within(ghost).getByText("Draggable")).toBeInTheDocument();

      fireEvent.mouseUp(window, { clientX: 130, clientY: 150 });
      expect(screen.queryByTestId("task-drag-ghost")).not.toBeInTheDocument();
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

  // Regression test for a real bug: window.confirm() has no real
  // implementation in Tauri's webview (see NotebookSidebar.tsx's own
  // comment about the exact same pitfall — it returns without ever
  // showing anything, so a "delete" gated behind it either silently never
  // fires or silently always fires, neither of which is a real
  // confirmation). Must use @tauri-apps/plugin-dialog's async confirm()
  // instead, same as that established call site.
  it("deleting a done card confirms via the native dialog plugin, not window.confirm", async () => {
    const { deleteTask } = await import("../../ipc/tasks");
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Done one", status: "done", outcome: "success" }),
    ]);
    view();
    const user = userEvent.setup();
    await screen.findByText("Done one");
    await user.click(screen.getByRole("button", { name: /^刪除$|^Delete$/ }));
    await waitFor(() => expect(confirm).toHaveBeenCalled());
    await waitFor(() => expect(deleteTask).toHaveBeenCalledWith("d", false));
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

  it("new-card dialog: checking interactive mode hides the parallel toggle and is sent to createTask", async () => {
    const { createTask } = await import("../../ipc/tasks");
    vi.mocked(createTask).mockResolvedValue("id-new");
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /新增工作|New task/ }));
    await user.type(screen.getByLabelText(/標題|Title/), "Chat task");
    await user.type(screen.getByLabelText(/專案資料夾|Project folder/), "/repo");

    expect(screen.getByText(/可與其他任務並行|Can run alongside other tasks/)).toBeInTheDocument();
    await user.click(screen.getByLabelText(/互動模式|Interactive mode/));
    expect(screen.queryByText(/可與其他任務並行|Can run alongside other tasks/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() =>
      expect(createTask).toHaveBeenCalledWith(
        expect.objectContaining({ title: "Chat task", project_dir: "/repo", interactive: true }),
      ),
    );
  });

  it("new-card dialog defaults interactive to false when left unchecked", async () => {
    const { createTask } = await import("../../ipc/tasks");
    vi.mocked(createTask).mockResolvedValue("id-new");
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /新增工作|New task/ }));
    await user.type(screen.getByLabelText(/標題|Title/), "Auto task");
    await user.type(screen.getByLabelText(/專案資料夾|Project folder/), "/repo");
    await user.click(screen.getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() =>
      expect(createTask).toHaveBeenCalledWith(expect.objectContaining({ interactive: false })),
    );
  });

  // Regression test for a real complaint: a brand-new card has no id yet
  // (attachments hang off an existing task id), so the create dialog had no
  // attachment UI at all — you had to save first, then reopen via Edit. This
  // buffers picked files client-side and uploads them right after the new
  // id comes back from createTask, so it's one continuous flow.
  it("new-card dialog buffers picked files and uploads them once the card is created", async () => {
    const { createTask, addAttachment } = await import("../../ipc/tasks");
    vi.mocked(createTask).mockResolvedValue("id-new");
    vi.mocked(addAttachment).mockResolvedValue({
      id: "a1",
      task_id: "id-new",
      filename: "spec.md",
      stored_path: "/x/spec.md",
    });
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /新增工作|New task/ }));
    await user.type(screen.getByLabelText(/標題|Title/), "Ship it");
    await user.type(screen.getByLabelText(/專案資料夾|Project folder/), "/repo");

    const file = new File(["hello"], "spec.md", { type: "text/plain" });
    const fileInput = screen.getByLabelText(/加入附件|Add attachment/);
    await user.upload(fileInput, file);
    expect(screen.getByText("spec.md")).toBeInTheDocument();
    // Not uploaded yet — there's no card id to attach it to until Save.
    expect(addAttachment).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() =>
      expect(addAttachment).toHaveBeenCalledWith("id-new", "spec.md", expect.any(Uint8Array)),
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

  it("conversation dialog shows the task's own instructions and the raw terminal output", async () => {
    const { readTranscript } = await import("../../ipc/tasks");
    vi.mocked(readTranscript).mockResolvedValue("line A\nline B");
    vi.mocked(listTasks).mockResolvedValue([
      card({
        id: "d",
        title: "Done one",
        status: "done",
        outcome: "success",
        transcript_path: "/p/t.txt",
        body: "查詢目錄資訊",
      }),
    ]);
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /對話記錄|Conversation/ }));
    expect(await screen.findByText("查詢目錄資訊")).toBeInTheDocument();
    expect(await screen.findByText(/line A/)).toBeInTheDocument();
  });

  // Regression test for a real complaint: the raw transcript is a literal
  // dump of every terminal redraw (Claude Code's TUI repaints the same
  // spinner/status line many times per second), so long runs of consecutive
  // duplicate lines are the single biggest source of noise. Collapsing them
  // is a generic, low-risk cleanup — not a heuristic tied to any specific
  // spinner glyph — that meaningfully thins the output without pretending to
  // reconstruct a clean chat transcript (which would need a real terminal
  // screen-state emulator, out of scope here).
  it("collapses consecutive duplicate lines in the raw terminal output", async () => {
    const { readTranscript } = await import("../../ipc/tasks");
    vi.mocked(readTranscript).mockResolvedValue("start\nspinner\nspinner\nspinner\nspinner\ndone");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Done one", status: "done", outcome: "success", transcript_path: "/p/t.txt", body: "b" }),
    ]);
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /對話記錄|Conversation/ }));
    const raw = await screen.findByTestId("task-transcript-raw");
    expect(raw.textContent).toContain("start");
    expect(raw.textContent).toContain("done");
    expect(raw.textContent).not.toContain("spinner\nspinner");
  });

  // Regression coverage for the new "just finished → try to upgrade the
  // saved transcript" behavior. Uses a controllable listTasks mock (like the
  // existing "re-fetches when tasks-updated fires" test) to drive a real
  // status transition through refresh().
  it("upgrades the transcript once when a card transitions into done, with a live tab", async () => {
    const { saveTranscript } = await import("../../ipc/tasks");
    vi.mocked(serializeTerminal).mockReturnValue("clean serialized text");
    let fire: () => void = () => {};
    vi.mocked(onTasksUpdated).mockImplementation(async (cb) => { fire = cb; return () => {}; });

    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Running", status: "running", tab_id: "tab-1" }),
    ]);
    view();
    await screen.findByText("Running");
    expect(saveTranscript).not.toHaveBeenCalled();

    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Running", status: "done", outcome: "success", tab_id: "tab-1", transcript_path: "/p/t.txt" }),
    ]);
    fire();

    await waitFor(() => expect(saveTranscript).toHaveBeenCalledWith("d", "clean serialized text"));
  });

  it("does not upgrade on first load even if a card is already done", async () => {
    const { saveTranscript } = await import("../../ipc/tasks");
    vi.mocked(serializeTerminal).mockReturnValue("clean serialized text");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "AlreadyDone", status: "done", outcome: "success", tab_id: "tab-1" }),
    ]);
    view();
    await screen.findByText("AlreadyDone");
    expect(saveTranscript).not.toHaveBeenCalled();
  });

  it("does not upgrade when the tab is not live (serializeTerminal returns null)", async () => {
    const { saveTranscript } = await import("../../ipc/tasks");
    vi.mocked(serializeTerminal).mockReturnValue(null);
    let fire: () => void = () => {};
    vi.mocked(onTasksUpdated).mockImplementation(async (cb) => { fire = cb; return () => {}; });

    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Running", status: "running", tab_id: "tab-1" }),
    ]);
    view();
    await screen.findByText("Running");

    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Running", status: "done", outcome: "success", tab_id: "tab-1" }),
    ]);
    fire();

    await waitFor(() => expect(listTasks).toHaveBeenCalledTimes(2));
    expect(saveTranscript).not.toHaveBeenCalled();
  });
});
