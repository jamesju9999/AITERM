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
  archiveTask: vi.fn().mockResolvedValue(undefined),
  unarchiveTask: vi.fn().mockResolvedValue(undefined),
  archiveDoneTasks: vi.fn().mockResolvedValue(2),
  listArchivedTasks: vi.fn().mockResolvedValue({ rows: [], total: 0 }),
  setTaskLabel: vi.fn().mockResolvedValue(undefined),
}));

// TaskEditorDialog reads the project's already-used folders from ipc/projects
// on mount; unmocked that goes to the real `invoke` and throws outside a Tauri
// webview.
vi.mock("../../ipc/projects", () => ({
  usedDirs: vi.fn().mockResolvedValue([]),
  usedLabels: vi.fn().mockResolvedValue([]),
}));

vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: vi.fn(),
}));

vi.mock("../../lib/runningTaskTabRegistry", () => ({
  setRunningTaskTabs: vi.fn(),
}));

import { listTasks, onTasksUpdated, moveTask, archiveTask, archiveDoneTasks, listArchivedTasks, unarchiveTask, setTaskLabel } from "../../ipc/tasks";
import type { TaskWithAttachments } from "../../ipc/tasks";
import { setRunningTaskTabs } from "../../lib/runningTaskTabRegistry";
import { ProjectBoard } from "./ProjectBoard";

/** Every IPC assertion below pins this exact value as the first argument.
 * The board is now per-project and every `tasks_*` command is scoped by it,
 * so a dropped or wrong projectId is the single most likely regression of
 * the projects refactor — asserting on it is the point, never `expect.anything()`. */
const PROJECT_ID = "p1";

const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "c1", title: "Card one", body: "", project_dir: "/r", status: "planning",
  parallel_ok: true, interactive: false, sort_order: 1, outcome: null, tab_id: null,
  transcript_path: null, error_message: null, created_at: "", dispatched_at: null,
  finished_at: null, ai_summary: null, archived_at: null,
  session_id: null, session_path: null, use_bridge: false, bridge_tiers: null,
  label: null, attachments: [],
  ...over,
});

const view = () =>
  render(<LocaleProvider><ProjectBoard projectId={PROJECT_ID} onReport={() => {}} /></LocaleProvider>);

beforeEach(() => {
  // Repo vitest config does not set `clearMocks`, so call counts would leak
  // across tests in this file (the "toHaveBeenCalledTimes(1)" assertion below).
  vi.clearAllMocks();
  vi.mocked(listTasks).mockResolvedValue([]);
  vi.mocked(onTasksUpdated).mockResolvedValue(() => {});
});

describe("ProjectBoard", () => {
  it("renders four columns", async () => {
    view();
    await waitFor(() => expect(screen.getByText(/計畫中|Planned/)).toBeInTheDocument());
    // The board fetches only its own project's tasks.
    expect(listTasks).toHaveBeenCalledWith(PROJECT_ID);
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
      await waitFor(() =>
        expect(moveTask).toHaveBeenCalledWith(PROJECT_ID, "p", "queued", expect.any(Number)),
      );
    } finally {
      document.elementFromPoint = originalElementFromPoint;
    }
  });

  // 已完成的卡片留著的是它還在佇列時的 sort_order（finish 不碰那一欄），
  // 實機上那些值常常一模一樣，於是這一欄的順序等於未定義。後端已經改成
  // 依 finished_at 由新到舊回傳，但這裡如果還照 sort_order 重排一次，
  // 後端那個修正在畫面上完全看不出來。
  it("已完成欄依完成時間由新到舊排列", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "a", title: "最早完成", status: "done", outcome: "success", sort_order: 1, finished_at: 1000 }),
      card({ id: "c", title: "最晚完成", status: "done", outcome: "success", sort_order: 1, finished_at: 3000 }),
      card({ id: "b", title: "中間完成", status: "done", outcome: "success", sort_order: 1, finished_at: 2000 }),
    ]);
    view();
    await screen.findByText("最早完成");

    const titles = Array.from(
      screen.getByTestId("column-done").querySelectorAll(".task-card-title"),
    ).map((el) => el.textContent);
    expect(titles).toEqual(["最晚完成", "中間完成", "最早完成"]);
  });

  describe("看板搜尋", () => {
    const three = () =>
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "1", title: "整理打卡 API", body: "產出規格", project_dir: "/repo/hcp", status: "planning" }),
        card({ id: "2", title: "修 CI", body: "Windows 編譯錯誤", project_dir: "/repo/aiterm", status: "queued" }),
        card({ id: "3", title: "寫文件", body: "", project_dir: "/repo/hcp", status: "done", outcome: "success", finished_at: 1 }),
      ]);

    const search = async (text: string) => {
      await userEvent.type(await screen.findByTestId("board-search"), text);
    };

    it("跨欄過濾，只留下符合的卡片", async () => {
      three();
      view();
      await screen.findByText("整理打卡 API");

      await search("打卡");

      expect(screen.getByText("整理打卡 API")).toBeInTheDocument();
      expect(screen.queryByText("修 CI")).not.toBeInTheDocument();
      expect(screen.queryByText("寫文件")).not.toBeInTheDocument();
    });

    it("也比對工作內容與工作目錄", async () => {
      three();
      view();
      await screen.findByText("修 CI");

      await search("Windows");
      expect(screen.getByText("修 CI")).toBeInTheDocument();
      expect(screen.queryByText("整理打卡 API")).not.toBeInTheDocument();

      await userEvent.clear(screen.getByTestId("board-search"));
      await search("aiterm");
      expect(screen.getByText("修 CI")).toBeInTheDocument();
      expect(screen.queryByText("寫文件")).not.toBeInTheDocument();
    });

    it("欄位計數跟著過濾後的張數走", async () => {
      three();
      view();
      await screen.findByText("整理打卡 API");

      await search("hcp"); // 命中「計畫中」與「已完成」各一張

      const count = (status: string) =>
        within(screen.getByTestId(`column-${status}`)).getByText(
          (_, el) => el?.className === "task-column-count",
        ).textContent;
      expect(count("planning")).toBe("1");
      expect(count("queued")).toBe("0");
      expect(count("done")).toBe("1");
    });

    it("清掉關鍵字之後卡片全部回來", async () => {
      three();
      view();
      await search("打卡");
      expect(screen.queryByText("修 CI")).not.toBeInTheDocument();

      await userEvent.clear(screen.getByTestId("board-search"));

      expect(await screen.findByText("修 CI")).toBeInTheDocument();
    });

    // 「封存全部」送到後端的是 archive_all_done，它收走的是整欄、不管
    // 畫面上正在過濾什麼。詢問的張數若跟著過濾走，使用者會以為只收 1 張、
    // 實際上收走 12 張。
    it("搜尋中「封存全部」仍然以整欄的實際張數詢問", async () => {
      const { confirm } = await import("@tauri-apps/plugin-dialog");
      vi.mocked(listTasks).mockResolvedValue([
        // 兩個標題不可以互為子字串——「沒命中的」含有「命中的」的話兩張
        // 都會被搜到，過濾前後的張數一樣，這個測試就白寫了。
        card({ id: "d1", title: "甲工作", status: "done", outcome: "success", finished_at: 2 }),
        card({ id: "d2", title: "乙工作", status: "done", outcome: "success", finished_at: 1 }),
      ]);
      view();
      await screen.findByText("甲工作");

      await search("甲");
      await userEvent.click(screen.getByTestId("archive-column"));

      await waitFor(() => expect(confirm).toHaveBeenCalled());
      expect(vi.mocked(confirm).mock.calls[0][0]).toContain("2");
    });
  });

  describe("封存", () => {
    const doneCard = (over: Partial<TaskWithAttachments> = {}) =>
      card({ id: "d", title: "收工了", status: "done", outcome: "success", finished_at: 1, ...over });

    it("已完成的卡片才有封存按鈕", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        doneCard(),
        card({ id: "p", title: "還在想", status: "planning" }),
      ]);
      view();
      await screen.findByText("收工了");

      const done = within(screen.getByTestId("column-done"));
      expect(done.getByRole("button", { name: /^(封存|Archive)$/ })).toBeInTheDocument();
      const planning = within(screen.getByTestId("column-planning"));
      expect(planning.queryByRole("button", { name: /^(封存|Archive)$/ })).not.toBeInTheDocument();
    });

    it("封存單張卡片", async () => {
      vi.mocked(listTasks).mockResolvedValue([doneCard()]);
      view();
      await screen.findByText("收工了");

      const done = within(screen.getByTestId("column-done"));
      await userEvent.click(done.getByRole("button", { name: /^(封存|Archive)$/ }));

      await waitFor(() => expect(archiveTask).toHaveBeenCalledWith(PROJECT_ID, "d"));
    });

    it("整欄封存會先問過再送出", async () => {
      const { confirm } = await import("@tauri-apps/plugin-dialog");
      vi.mocked(listTasks).mockResolvedValue([doneCard(), doneCard({ id: "d2", title: "也收工" })]);
      view();
      await screen.findByText("收工了");

      await userEvent.click(screen.getByTestId("archive-column"));

      await waitFor(() => expect(confirm).toHaveBeenCalled());
      await waitFor(() => expect(archiveDoneTasks).toHaveBeenCalledWith(PROJECT_ID));
    });

    // 刪除不可復原、封存可以，但一次收走整欄仍然是個大動作，取消一定要
    // 真的不送出。
    it("整欄封存取消就不送出", async () => {
      const { confirm } = await import("@tauri-apps/plugin-dialog");
      // Once 而不是 mockResolvedValue：這個檔案的 beforeEach 只呼叫
      // vi.clearAllMocks()，那會清掉呼叫記錄但**留著實作**，所以用
      // mockResolvedValue 設成 false 會一路污染到後面的刪除測試。
      vi.mocked(confirm).mockResolvedValueOnce(false);
      vi.mocked(listTasks).mockResolvedValue([doneCard()]);
      view();
      await screen.findByText("收工了");

      await userEvent.click(screen.getByTestId("archive-column"));

      await waitFor(() => expect(confirm).toHaveBeenCalled());
      expect(archiveDoneTasks).not.toHaveBeenCalled();
    });

    // 實機回報：在封存視窗點對話記錄，視窗直接關掉、對話記錄也沒出來。
    // 原因是它把 taskId 交給 ProjectBoard 去 `tasks` 裡找卡片，而 `tasks`
    // 只有未封存的——封存的卡片永遠找不到，於是什麼都沒渲染。
    it("在封存視窗看對話記錄時，封存清單要留著", async () => {
      vi.mocked(listTasks).mockResolvedValue([]);
      vi.mocked(listArchivedTasks).mockResolvedValue({
        rows: [
          card({
            id: "a1", title: "去年的工作", status: "done", outcome: "success",
            archived_at: 1000, transcript_path: "/p/transcript.txt",
          }),
        ],
        total: 1,
      });
      view();

      await userEvent.click(await screen.findByTestId("open-archive"));
      await userEvent.click(await screen.findByTestId("archive-transcript-a1"));

      expect(await screen.findByTestId("task-transcript-raw")).toBeInTheDocument();
      expect(screen.getByText("去年的工作")).toBeInTheDocument();
    });

    // 封存清單是唯一會無限成長的地方，搜尋與分頁都必須在後端做——
    // 全部撈回來再前端過濾，遲早變成打開視窗就卡住。
    it("打字會把關鍵字送到後端", async () => {
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      await userEvent.click(await screen.findByTestId("open-archive"));
      await screen.findByTestId("archive-search");

      await userEvent.type(screen.getByTestId("archive-search"), "打卡");

      await waitFor(() =>
        expect(listArchivedTasks).toHaveBeenLastCalledWith(PROJECT_ID, "打卡", 20, 0),
      );
    });

    it("換頁時送出的是下一頁的 offset", async () => {
      vi.mocked(listTasks).mockResolvedValue([]);
      vi.mocked(listArchivedTasks).mockResolvedValue({
        rows: [card({ id: "a1", title: "第一頁的卡", status: "done", archived_at: 1 })],
        total: 45,
      });
      view();
      await userEvent.click(await screen.findByTestId("open-archive"));

      await userEvent.click(await screen.findByTestId("archive-next"));

      await waitFor(() =>
        expect(listArchivedTasks).toHaveBeenLastCalledWith(PROJECT_ID, "", 20, 20),
      );
    });

    // 只有一頁時翻頁控制項只會佔位置，還會讓人以為後面有東西。
    it("只有一頁時不顯示翻頁控制項", async () => {
      vi.mocked(listTasks).mockResolvedValue([]);
      vi.mocked(listArchivedTasks).mockResolvedValue({
        rows: [card({ id: "a1", title: "唯一一張", status: "done", archived_at: 1 })],
        total: 1,
      });
      view();
      await userEvent.click(await screen.findByTestId("open-archive"));
      await screen.findByText("唯一一張");

      expect(screen.queryByTestId("archive-next")).not.toBeInTheDocument();
    });

    it("封存清單列出封存的卡片並且可以放回看板", async () => {
      vi.mocked(listTasks).mockResolvedValue([]);
      vi.mocked(listArchivedTasks).mockResolvedValue({
        rows: [
          card({ id: "a1", title: "去年的工作", status: "done", outcome: "success", archived_at: 1000 }),
        ],
        total: 1,
      });
      view();

      await userEvent.click(await screen.findByTestId("open-archive"));
      await screen.findByText("去年的工作");

      await userEvent.click(screen.getByTestId("archive-restore-a1"));
      await waitFor(() => expect(unarchiveTask).toHaveBeenCalledWith(PROJECT_ID, "a1"));
    });
  });

  it("dropping a running interactive card on the done column calls markTaskDone, not moveTask", async () => {
    const { markTaskDone } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Chatting", status: "running", tab_id: "tab-1", interactive: true }),
    ]);
    view();
    const cardEl = await screen.findByText("Chatting");
    const doneCol = screen.getByTestId("column-done");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;
    expect(dragWrap).toBeTruthy();

    const originalElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = vi.fn().mockReturnValue(doneCol);
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });
      fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
      fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
      await waitFor(() => expect(markTaskDone).toHaveBeenCalledWith(PROJECT_ID, "r"));
      expect(moveTask).not.toHaveBeenCalled();
    } finally {
      document.elementFromPoint = originalElementFromPoint;
    }
  });

  it("dropping a running NON-interactive card on the done column does nothing", async () => {
    const { markTaskDone } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Auto running", status: "running", tab_id: "tab-1", interactive: false }),
    ]);
    view();
    const cardEl = await screen.findByText("Auto running");
    const doneCol = screen.getByTestId("column-done");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;

    const originalElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = vi.fn().mockReturnValue(doneCol);
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });
      fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
      fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
      // Not draggable at all — mousedown shouldn't even arm a drag for a
      // non-interactive running card, so neither call should ever fire.
      await new Promise((r) => setTimeout(r, 50));
      expect(markTaskDone).not.toHaveBeenCalled();
      expect(moveTask).not.toHaveBeenCalled();
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

  // The column header's status color line is driven off data-column-status
  // (see index.css). Deliberately a separate attribute from the data-testid
  // the drag code keys off, so renaming one can't silently restyle the
  // other — this asserts the styling hook exists on every column.
  it("each column carries its own status attribute for the header color line", async () => {
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    for (const status of ["planning", "queued", "running", "done"]) {
      const col = await screen.findByTestId(`column-${status}`);
      expect(col.getAttribute("data-column-status")).toBe(status);
    }
  });

  // User-requested: the ghost used to be a hand-rolled subset (title + path
  // only), so it looked nothing like the card it came from. It now renders
  // the real TaskCard, which also means the two can't drift apart as the
  // card evolves. Asserting on the structural pieces the old version was
  // missing — status bar, badges, action buttons — rather than just the
  // title, which the old version already had.
  it("the drag ghost renders the same full card content as the resting card", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "p", title: "Draggable", status: "planning", interactive: true }),
    ]);
    view();
    const cardEl = await screen.findByText("Draggable");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;

    const originalElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = vi.fn().mockReturnValue(null);
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });
      fireEvent.mouseMove(window, { clientX: 130, clientY: 150 });

      const ghost = await screen.findByTestId("task-drag-ghost");
      const ghostCard = ghost.querySelector(".task-card") as HTMLElement;
      expect(ghostCard).not.toBeNull();
      // Same status drives the same left color bar as the resting card.
      expect(ghostCard.getAttribute("data-task-status")).toBe("planning");
      // Interactive avatar chip and the planning-state action buttons —
      // none of which the old simplified ghost rendered at all.
      expect(ghostCard.querySelector(".task-card-avatar")).not.toBeNull();
      expect(within(ghostCard).getByText(/編輯工作|Edit/)).toBeInTheDocument();
      expect(within(ghostCard).getByText(/刪除|Delete/)).toBeInTheDocument();

      fireEvent.mouseUp(window, { clientX: 130, clientY: 150 });
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
    expect(stopTask).toHaveBeenCalledWith(PROJECT_ID, "r");
  });

  it("interactive running card shows the interactive badge and a Mark Done button that calls markTaskDone", async () => {
    const { markTaskDone } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Chatting", status: "running", tab_id: "tab-1", interactive: true }),
    ]);
    view();
    const user = userEvent.setup();
    await screen.findByText("Chatting");
    expect(screen.getByText(/互動|Interactive/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /標記完成|Mark Done/ }));
    expect(markTaskDone).toHaveBeenCalledWith(PROJECT_ID, "r");
  });

  it("running card carries a data-task-status attribute matching its status, for the CSS left-accent-bar", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Running one", status: "running", tab_id: "tab-1" }),
    ]);
    view();
    const cardEl = await screen.findByText("Running one");
    const cardRoot = cardEl.closest(".task-card") as HTMLElement;
    expect(cardRoot.dataset.taskStatus).toBe("running");
  });

  it("done+success card's data-task-status reflects the outcome, not just the status", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Done one", status: "done", outcome: "success" }),
    ]);
    view();
    const cardEl = await screen.findByText("Done one");
    const cardRoot = cardEl.closest(".task-card") as HTMLElement;
    expect(cardRoot.dataset.taskStatus).toBe("success");
  });

  it("interactive running card's Mark Done button uses the primary button style, Stop uses ghost", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Chatting", status: "running", tab_id: "tab-1", interactive: true }),
    ]);
    view();
    await screen.findByText("Chatting");
    expect(screen.getByRole("button", { name: /標記完成|Mark Done/ }).className).toContain("tb-btn--primary");
    expect(screen.getByRole("button", { name: /停止|Stop/ }).className).toContain("tb-btn--ghost");
  });

  it("non-interactive running card has no Mark Done button", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Auto running", status: "running", tab_id: "tab-1", interactive: false }),
    ]);
    view();
    await screen.findByText("Auto running");
    expect(screen.queryByRole("button", { name: /標記完成|Mark Done/ })).not.toBeInTheDocument();
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
    await waitFor(() => expect(deleteTask).toHaveBeenCalledWith(PROJECT_ID, "d", false));
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
    expect(cloneTask).toHaveBeenCalledWith(PROJECT_ID, "d");
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
        PROJECT_ID,
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
        PROJECT_ID,
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
      expect(createTask).toHaveBeenCalledWith(
        PROJECT_ID,
        expect.objectContaining({ interactive: false }),
      ),
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
      expect(addAttachment).toHaveBeenCalledWith(
        PROJECT_ID,
        "id-new",
        "spec.md",
        expect.any(Uint8Array),
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
      expect(updateTask).toHaveBeenCalledWith(
        PROJECT_ID,
        expect.objectContaining({ id: "p", title: "New title" }),
      ),
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
    expect(readTranscript).toHaveBeenCalledWith(PROJECT_ID, "d");
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

  // The transcript dialog is drag-resizable (CSS `resize: both`), and the
  // browser records a dragged size as an INLINE width/height on the element.
  // Inline styles outrank the maximized class's own sizing, so maximizing
  // must clear them and restoring must write them back — otherwise the
  // button silently does nothing once the user has dragged even once.
  it("maximize clears a dragged inline size, restore puts it back", async () => {
    const { readTranscript } = await import("../../ipc/tasks");
    vi.mocked(readTranscript).mockResolvedValue("output");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Done one", status: "done", outcome: "success", transcript_path: "/p/t.txt", body: "b" }),
    ]);
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /對話記錄|Conversation/ }));

    const dialog = (await screen.findByTestId("task-transcript-raw")).closest(
      ".task-transcript-dialog",
    ) as HTMLElement;
    // Simulate the user having dragged the corner.
    dialog.style.width = "700px";
    dialog.style.height = "500px";

    const maxBtn = screen.getByRole("button", { name: /放到最大|Maximize/ });
    await user.click(maxBtn);
    expect(dialog.classList.contains("task-transcript-dialog--max")).toBe(true);
    expect(dialog.style.width).toBe("");
    expect(dialog.style.height).toBe("");

    await user.click(screen.getByRole("button", { name: /還原大小|Restore size/ }));
    expect(dialog.classList.contains("task-transcript-dialog--max")).toBe(false);
    expect(dialog.style.width).toBe("700px");
    expect(dialog.style.height).toBe("500px");
  });
  // 對話記錄乾淨化的觸發已經搬到後端的 `task-finished` 事件 ＋
  // TerminalApp 的 useTranscriptUpgrader（永遠掛載）——因為看板只有在
  // 該專案是當前分頁時才掛載，別的專案完成時沒人在聽。原本掛在這裡的
  // 三個測試測的是舊的「看板比對前後狀態」觸發方式，已經沒有對應的
  // 程式碼；涵蓋範圍移到 useTranscriptUpgrader.test.tsx（觸發）與
  // transcriptUpgrade.test.ts（分頁不在／序列化失敗時的行為）。

  // Regression coverage for a real bug: closing a tab whose Task Board task
  // was still `running` gave no warning at all — TerminalView's own close
  // guard only knew about shell-command-busy/agent-mission state, nothing
  // about the Task Board. Fixed by having TaskBoardView keep a shared
  // registry (runningTaskTabRegistry) in sync with which tab ids currently
  // belong to a running task, which TerminalView's guard also consults.
  it("keeps the running-task-tab registry in sync with the task list", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r1", title: "Running one", status: "running", tab_id: "tab-1" }),
      card({ id: "r2", title: "Running two", status: "running", tab_id: "tab-2" }),
      card({ id: "p1", title: "Planned", status: "planning", tab_id: null }),
    ]);
    view();
    await screen.findByText("Running one");
    await waitFor(() =>
      expect(setRunningTaskTabs).toHaveBeenCalledWith(expect.arrayContaining(["tab-1", "tab-2"])),
    );
    const lastCallArg = vi.mocked(setRunningTaskTabs).mock.calls.at(-1)?.[0];
    expect(Array.from(lastCallArg ?? [])).toHaveLength(2);
  });

  it("drops a tab from the running-task-tab registry once its task finishes", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r1", title: "Running one", status: "running", tab_id: "tab-1" }),
    ]);
    let fire: () => void = () => {};
    vi.mocked(onTasksUpdated).mockImplementation(async (cb) => { fire = cb; return () => {}; });
    view();
    await screen.findByText("Running one");
    await waitFor(() =>
      expect(setRunningTaskTabs).toHaveBeenLastCalledWith(expect.arrayContaining(["tab-1"])),
    );

    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r1", title: "Running one", status: "done", outcome: "success", tab_id: "tab-1" }),
    ]);
    fire();

    await waitFor(() => expect(setRunningTaskTabs).toHaveBeenLastCalledWith([]));
  });

  // Regression test for a real bug: deleting a "done" card and confirming
  // "close the tab too" killed the PTY session on the backend but never
  // told the frontend's own tab list to remove it — the tab visually stayed
  // open (now attached to a dead session). TerminalApp's aiterm:close-tab
  // listener is the fix; this only proves TaskCard actually dispatches it.
  it("dispatches aiterm:close-tab when deleting a done card and confirming to close its tab", async () => {
    const { deleteTask } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Done one", status: "done", outcome: "success", tab_id: "tab-9" }),
    ]);
    view();
    const user = userEvent.setup();
    await screen.findByText("Done one");

    const events: CustomEvent<{ tabId?: string }>[] = [];
    const onCloseTab = (e: Event) => events.push(e as CustomEvent<{ tabId?: string }>);
    window.addEventListener("aiterm:close-tab", onCloseTab);
    try {
      await user.click(screen.getByRole("button", { name: /^刪除$|^Delete$/ }));
      await waitFor(() => expect(deleteTask).toHaveBeenCalledWith(PROJECT_ID, "d", true));
      await waitFor(() => expect(events).toHaveLength(1));
      expect(events[0].detail.tabId).toBe("tab-9");
    } finally {
      window.removeEventListener("aiterm:close-tab", onCloseTab);
    }
  });

  // Regression coverage for a real UX gap: a running+interactive card can
  // only legally be dropped on "done" (via markTaskDone) — dropping it on
  // planning/queued was already silently ignored by handleDrop, but the
  // column it was hovering over still lit up as if the drop would work,
  // which is misleading. The column should only highlight when the drop
  // would actually succeed for the card currently being dragged.
  it("does not highlight an illegal drop target while dragging a running interactive card", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Chatting", status: "running", tab_id: "tab-1", interactive: true }),
    ]);
    view();
    const cardEl = await screen.findByText("Chatting");
    const planningCol = screen.getByTestId("column-planning");
    const doneCol = screen.getByTestId("column-done");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;

    const originalElementFromPoint = document.elementFromPoint;
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });

      document.elementFromPoint = vi.fn().mockReturnValue(planningCol);
      fireEvent.mouseMove(window, { clientX: 100, clientY: 120 }); // past the drag threshold, over an illegal target
      expect(planningCol.className).not.toContain("task-column--drop-target");

      document.elementFromPoint = vi.fn().mockReturnValue(doneCol);
      fireEvent.mouseMove(window, { clientX: 100, clientY: 140 }); // now over the one legal target
      expect(doneCol.className).toContain("task-column--drop-target");

      fireEvent.mouseUp(window, { clientX: 100, clientY: 140 });
    } finally {
      document.elementFromPoint = originalElementFromPoint;
    }
  });

  it("still highlights the legal target when dragging a planning card over the queued column", async () => {
    vi.mocked(listTasks).mockResolvedValue([card({ id: "p", title: "Draggable", status: "planning" })]);
    view();
    const cardEl = await screen.findByText("Draggable");
    const queuedCol = screen.getByTestId("column-queued");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;

    const originalElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = vi.fn().mockReturnValue(queuedCol);
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });
      fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
      expect(queuedCol.className).toContain("task-column--drop-target");
      fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
    } finally {
      document.elementFromPoint = originalElementFromPoint;
    }
  });

  describe("Label 徽章", () => {
    it("有 label 的卡片顯示徽章", async () => {
      vi.mocked(listTasks).mockResolvedValue([card({ id: "1", label: "緊急" })]);
      view();
      // 單張有 label 的卡片會落在該 label 的群組裡，"緊急" 因此出現兩次：
      // 群組標頭一次，卡片自己的徽章一次。
      await waitFor(() => expect(screen.getAllByText("緊急")).toHaveLength(2));
    });

    it("沒有 label 的卡片不顯示徽章", async () => {
      vi.mocked(listTasks).mockResolvedValue([card({ id: "1", label: null })]);
      view();
      await screen.findByText("Card one");
      expect(screen.queryByText("緊急")).not.toBeInTheDocument();
    });
  });

  describe("Label 分組", () => {
    it("同一個 Label 的卡片收進同一個可摺疊群組，未分類卡片留在最上面", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "1", title: "沒分類", label: null, sort_order: 1 }),
        card({ id: "2", title: "緊急一", label: "緊急", sort_order: 2, created_at: "2026-01-01 00:00:00" }),
        card({ id: "3", title: "緊急二", label: "緊急", sort_order: 3, created_at: "2026-01-02 00:00:00" }),
      ]);
      view();

      await screen.findByText("沒分類");
      // "緊急" 文字會出現三次：群組標頭一次，兩張卡片各自的徽章各一次
      // （TaskCard 一律顯示自己的 label 徽章，不因為分組而省略）。
      expect(screen.getAllByText("緊急")).toHaveLength(3);
      expect(screen.getByText("(2)")).toBeInTheDocument();
      expect(screen.getByText("緊急一")).toBeInTheDocument();
      expect(screen.getByText("緊急二")).toBeInTheDocument();
    });

    it("搜尋關鍵字能比對 label", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "1", title: "不相干的卡", label: null }),
        card({ id: "2", title: "有分類的卡", label: "緊急" }),
      ]);
      view();
      await screen.findByText("不相干的卡");

      await userEvent.type(screen.getByTestId("board-search"), "緊急");
      expect(screen.getByText("有分類的卡")).toBeInTheDocument();
      expect(screen.queryByText("不相干的卡")).not.toBeInTheDocument();
    });
  });

  describe("非 planning 卡片的 Label 快速編輯", () => {
    it("已完成的卡片有「更改群組」按鈕，點了會開啟編輯視窗", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "1", title: "跑完的工作", status: "done", outcome: "success", label: "緊急" }),
      ]);
      view();
      await screen.findByText("跑完的工作");
      await userEvent.click(screen.getByRole("button", { name: /更改群組|Change group/ }));
      expect(await screen.findByTestId("task-label-quick-input")).toHaveValue("緊急");
    });

    it("儲存後呼叫 setTaskLabel 並重新整理看板", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "1", title: "跑完的工作", status: "done", outcome: "success", label: null }),
      ]);
      view();
      await screen.findByText("跑完的工作");
      await userEvent.click(screen.getByRole("button", { name: /更改群組|Change group/ }));
      await userEvent.type(await screen.findByTestId("task-label-quick-input"), "文件");
      await userEvent.click(screen.getByRole("button", { name: /儲存|Save/ }));
      expect(setTaskLabel).toHaveBeenCalledWith(PROJECT_ID, "1", "文件");
      expect(listTasks).toHaveBeenCalledTimes(2); // 初次載入 + 存檔後 refresh()
    });

    it("計畫中的卡片不顯示「更改群組」按鈕（已經有完整編輯視窗）", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "1", title: "還在想", status: "planning" }),
      ]);
      view();
      await screen.findByText("還在想");
      expect(screen.queryByRole("button", { name: /更改群組|Change group/ })).not.toBeInTheDocument();
    });
  });

  describe("同一狀態欄內拖曳換組", () => {
    it("拖到另一個 Label 群組上，呼叫 setTaskLabel 換成那個群組的 label", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "a", title: "Card A", status: "planning", label: null }),
        card({ id: "b", title: "Card B", status: "planning", label: "緊急" }),
      ]);
      view();
      const cardAEl = await screen.findByText("Card A");
      const cardBEl = screen.getByText("Card B");
      const dragWrapA = cardAEl.closest("[data-task-drag-id]") as HTMLElement;
      const dragTargetB = cardBEl.closest("[data-task-drag-id]") as HTMLElement;

      const originalElementFromPoint = document.elementFromPoint;
      document.elementFromPoint = vi.fn().mockReturnValue(dragTargetB);
      try {
        const { fireEvent } = await import("@testing-library/react");
        fireEvent.mouseDown(dragWrapA, { clientX: 100, clientY: 100, button: 0 });
        fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
        fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
        await waitFor(() => expect(setTaskLabel).toHaveBeenCalledWith(PROJECT_ID, "a", "緊急"));
      } finally {
        document.elementFromPoint = originalElementFromPoint;
      }
      // moveTask 不該被叫到——這是同欄換組，不是換狀態欄。
      expect(moveTask).not.toHaveBeenCalled();
    });

    it("拖到欄位空白處（不在任何卡片或群組上），清空 label", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "a", title: "Card A", status: "planning", label: "緊急" }),
      ]);
      view();
      const cardAEl = await screen.findByText("Card A");
      const dragWrapA = cardAEl.closest("[data-task-drag-id]") as HTMLElement;
      const planningCol = screen.getByTestId("column-planning");

      const originalElementFromPoint = document.elementFromPoint;
      document.elementFromPoint = vi.fn().mockReturnValue(planningCol);
      try {
        const { fireEvent } = await import("@testing-library/react");
        fireEvent.mouseDown(dragWrapA, { clientX: 100, clientY: 100, button: 0 });
        fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
        fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
        await waitFor(() => expect(setTaskLabel).toHaveBeenCalledWith(PROJECT_ID, "a", null));
      } finally {
        document.elementFromPoint = originalElementFromPoint;
      }
    });

    it("拖到自己原本所在的群組上（label 沒變），不呼叫 setTaskLabel", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "a", title: "Card A", status: "planning", label: "緊急" }),
        card({ id: "b", title: "Card B", status: "planning", label: "緊急" }),
      ]);
      view();
      const cardAEl = await screen.findByText("Card A");
      const cardBEl = screen.getByText("Card B");
      const dragWrapA = cardAEl.closest("[data-task-drag-id]") as HTMLElement;
      const dragTargetB = cardBEl.closest("[data-task-drag-id]") as HTMLElement;

      const originalElementFromPoint = document.elementFromPoint;
      document.elementFromPoint = vi.fn().mockReturnValue(dragTargetB);
      try {
        const { fireEvent } = await import("@testing-library/react");
        fireEvent.mouseDown(dragWrapA, { clientX: 100, clientY: 100, button: 0 });
        fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
        fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
      } finally {
        document.elementFromPoint = originalElementFromPoint;
      }
      expect(setTaskLabel).not.toHaveBeenCalled();
    });

    // 真機回報的 bug：不同狀態欄剛好用了同一個 label 名字時，拖曳只會
    // 影響同一欄，但另一欄同名的群組視覺上也一起亮了——因為高亮判斷
    // 當時只比對 label 文字，沒有連同欄位一起比對。
    it("兩個不同狀態欄剛好有同名群組時，拖曳只高亮自己那一欄的群組", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "p1", title: "P1", status: "planning", label: "測試" }),
        card({ id: "p2", title: "P2", status: "planning", label: null }),
        card({ id: "d1", title: "D1", status: "done", outcome: "success", label: "測試" }),
      ]);
      view();
      const cardP1El = await screen.findByText("P1");
      const cardP2El = screen.getByText("P2");
      const dragWrapP2 = cardP2El.closest("[data-task-drag-id]") as HTMLElement;
      const dragTargetP1 = cardP1El.closest("[data-task-drag-id]") as HTMLElement;

      const originalElementFromPoint = document.elementFromPoint;
      document.elementFromPoint = vi.fn().mockReturnValue(dragTargetP1);
      try {
        const { fireEvent } = await import("@testing-library/react");
        fireEvent.mouseDown(dragWrapP2, { clientX: 100, clientY: 100, button: 0 });
        fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
        const planningHighlighted = screen
          .getByTestId("column-planning")
          .querySelector(".task-label-group--drop-target");
        const doneHighlighted = screen
          .getByTestId("column-done")
          .querySelector(".task-label-group--drop-target");
        expect(planningHighlighted).toBeTruthy();
        expect(doneHighlighted).toBeNull();
        fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
      } finally {
        document.elementFromPoint = originalElementFromPoint;
      }
    });

    // 真機回報的另一個 bug：已完成的卡片完全無法拖曳（連 mousedown 都
    // 不會武裝拖曳狀態），導致這欄的卡片沒辦法用拖曳換組，只能靠
    // 「更改群組」小視窗，體驗跟其他三欄不一致。
    it("已完成的卡片也能拖曳換組", async () => {
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "d1", title: "D1", status: "done", outcome: "success", label: null }),
        card({ id: "d2", title: "D2", status: "done", outcome: "success", label: "測試" }),
      ]);
      view();
      const cardD1El = await screen.findByText("D1");
      const cardD2El = screen.getByText("D2");
      const dragWrapD1 = cardD1El.closest("[data-task-drag-id]") as HTMLElement;
      const dragTargetD2 = cardD2El.closest("[data-task-drag-id]") as HTMLElement;
      expect(dragWrapD1.className).toContain("task-card-drag-wrap--draggable");

      const originalElementFromPoint = document.elementFromPoint;
      document.elementFromPoint = vi.fn().mockReturnValue(dragTargetD2);
      try {
        const { fireEvent } = await import("@testing-library/react");
        fireEvent.mouseDown(dragWrapD1, { clientX: 100, clientY: 100, button: 0 });
        fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
        fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
        await waitFor(() => expect(setTaskLabel).toHaveBeenCalledWith(PROJECT_ID, "d1", "測試"));
      } finally {
        document.elementFromPoint = originalElementFromPoint;
      }
    });
  });
});
