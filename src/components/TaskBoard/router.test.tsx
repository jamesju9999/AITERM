import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listProjects = vi.fn();
// Which project the mounted board actually fetches for is the only observable
// proof of *which* project is active — the tab bar filters unknown ids on its
// own, so a tab-level assertion alone can't tell a correct active project from
// a stale one.
const listTasks = vi.fn();

vi.mock("../../ipc/projects", () => ({
  listProjects: (...a: unknown[]) => listProjects(...a),
  removeProject: vi.fn(),
  openProject: vi.fn(),
  createProject: vi.fn(),
  usedDirs: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/tasks", () => ({
  listTasks: (...a: unknown[]) => listTasks(...a),
  onTasksUpdated: vi.fn().mockResolvedValue(() => {}),
  moveTask: vi.fn(),
  markTaskDone: vi.fn(),
  // ProjectBoard's subtree (TaskCard / TaskEditorDialog / TranscriptDialog /
  // transcriptUpgrade) pulls in these too — mocking only the 4 the plan
  // listed leaves them undefined and breaks rendering in a confusing way.
  cloneTask: vi.fn(),
  deleteTask: vi.fn(),
  stopTask: vi.fn(),
  addAttachment: vi.fn(),
  createTask: vi.fn(),
  removeAttachment: vi.fn(),
  updateTask: vi.fn(),
  readTranscript: vi.fn(),
  saveTranscript: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), open: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskBoardView } from "./index";

const proj = (id: string) => ({
  id,
  name: id,
  description: "",
  path: `/p/${id}`,
  status: "ok" as const,
  counts: { planning: 0, queued: 0, running: 0, done: 0 },
  error: null,
});

const mount = () =>
  render(
    <LocaleProvider>
      <TaskBoardView />
    </LocaleProvider>,
  );

describe("TaskBoardView 路由", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    listProjects.mockResolvedValue([proj("alpha"), proj("beta")]);
    listTasks.mockResolvedValue([]);
  });

  it("一開始顯示專案總覽，不顯示看板", async () => {
    mount();
    expect(await screen.findByText("alpha")).toBeInTheDocument();
    expect(screen.queryByTestId("column-planning")).not.toBeInTheDocument();
  });

  it("點專案後顯示看板與分頁列", async () => {
    mount();
    await userEvent.click(await screen.findByText("alpha"));
    expect(await screen.findByTestId("column-planning")).toBeInTheDocument();
    expect(screen.getByTestId("project-tab-alpha")).toBeInTheDocument();
  });

  it("關閉最後一個分頁後回到專案總覽", async () => {
    mount();
    await userEvent.click(await screen.findByText("alpha"));
    await screen.findByTestId("column-planning");
    await userEvent.click(screen.getByTestId("project-tab-close-alpha"));
    await waitFor(() =>
      expect(screen.queryByTestId("column-planning")).not.toBeInTheDocument(),
    );
  });

  it("開啟中的分頁存進 localStorage 並在重新掛載後還原", async () => {
    const first = mount();
    await userEvent.click(await screen.findByText("alpha"));
    await screen.findByTestId("column-planning");
    first.unmount();

    mount();
    expect(await screen.findByTestId("project-tab-alpha")).toBeInTheDocument();
  });

  it("還原時過濾掉已不存在的專案", async () => {
    localStorage.setItem("aiterm_board_open_projects", JSON.stringify(["gone", "alpha"]));
    localStorage.setItem("aiterm_board_active_project", "gone");
    mount();
    expect(await screen.findByTestId("project-tab-alpha")).toBeInTheDocument();
    expect(screen.queryByTestId("project-tab-gone")).not.toBeInTheDocument();
  });

  // 上面那個測試單靠分頁列是抓不到 bug 的：ProjectTabBar 自己就會把
  // projects 裡查不到的 id 濾掉，所以就算 TaskBoardView 完全不過濾、
  // 還把 active 留在已消失的 "gone" 上，分頁列看起來仍然正確——底下
  // 卻掛著一個指向不存在專案的看板。改用「看板實際去抓誰的工作」來釘。
  it("還原時的活躍專案已不存在，看板要落到還在的專案而不是已消失的那個", async () => {
    localStorage.setItem("aiterm_board_open_projects", JSON.stringify(["gone", "alpha"]));
    localStorage.setItem("aiterm_board_active_project", "gone");
    mount();
    await screen.findByTestId("column-planning");
    await waitFor(() => expect(listTasks).toHaveBeenCalledWith("alpha"));
    expect(listTasks).not.toHaveBeenCalledWith("gone");
  });

  // 關掉「目前這個」分頁時，還有別的分頁開著就該接手，不該退回專案總覽。
  it("關閉活躍分頁後接手剩下的分頁，而不是回到專案總覽", async () => {
    mount();
    await userEvent.click(await screen.findByText("alpha"));
    await screen.findByTestId("column-planning");
    await userEvent.click(screen.getByTestId("project-tab-back"));
    await userEvent.click(await screen.findByText("beta"));
    await waitFor(() => expect(listTasks).toHaveBeenCalledWith("beta"));

    listTasks.mockClear();
    await userEvent.click(screen.getByTestId("project-tab-close-beta"));

    expect(await screen.findByTestId("project-tab-alpha")).toBeInTheDocument();
    expect(screen.queryByTestId("project-tab-beta")).not.toBeInTheDocument();
    await waitFor(() => expect(listTasks).toHaveBeenCalledWith("alpha"));
    expect(screen.getByTestId("column-planning")).toBeInTheDocument();
  });
});
