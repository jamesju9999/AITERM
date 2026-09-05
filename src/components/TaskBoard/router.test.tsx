import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listProjects = vi.fn();

vi.mock("../../ipc/projects", () => ({
  listProjects: (...a: unknown[]) => listProjects(...a),
  removeProject: vi.fn(),
  openProject: vi.fn(),
  createProject: vi.fn(),
}));
vi.mock("../../ipc/tasks", () => ({
  listTasks: vi.fn().mockResolvedValue([]),
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
});
