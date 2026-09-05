import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listProjects = vi.fn();
const removeProject = vi.fn();
const confirmDialog = vi.fn();

vi.mock("../../ipc/projects", () => ({
  listProjects: (...a: unknown[]) => listProjects(...a),
  removeProject: (...a: unknown[]) => removeProject(...a),
  openProject: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: (...a: unknown[]) => confirmDialog(...a),
  open: vi.fn(),
}));
// onTasksUpdated calls the real @tauri-apps/api/event listen() under the
// hood, which throws outside a real Tauri webview — same reason
// index.test.tsx mocks the whole module (see its onTasksUpdated mock).
vi.mock("../../ipc/tasks", () => ({
  onTasksUpdated: vi.fn().mockResolvedValue(() => {}),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { ProjectList } from "./ProjectList";

const project = (over: Record<string, unknown> = {}) => ({
  id: "p1",
  name: "makemoney",
  description: "賺錢",
  path: "/projects/makemoney",
  status: "ok",
  counts: { planning: 2, queued: 1, running: 1, done: 3 },
  error: null,
  ...over,
});

const mount = (onOpen = vi.fn()) =>
  render(
    <LocaleProvider>
      <ProjectList onOpen={onOpen} />
    </LocaleProvider>,
  );

describe("ProjectList", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listProjects.mockResolvedValue([project()]);
    removeProject.mockResolvedValue(undefined);
  });

  it("列出專案與它的工作數", async () => {
    mount();
    expect(await screen.findByText("makemoney")).toBeInTheDocument();
    // 2 + 1 + 1 + 3 = 7
    expect(screen.getByTestId("project-total-p1")).toHaveTextContent("7");
  });

  it("有執行中工作時顯示指示點", async () => {
    mount();
    await screen.findByText("makemoney");
    expect(screen.getByTestId("project-running-p1")).toBeInTheDocument();
  });

  it("沒有執行中工作時不顯示指示點", async () => {
    listProjects.mockResolvedValue([
      project({ counts: { planning: 1, queued: 0, running: 0, done: 0 } }),
    ]);
    mount();
    await screen.findByText("makemoney");
    expect(screen.queryByTestId("project-running-p1")).not.toBeInTheDocument();
  });

  it("完全沒有專案時顯示空狀態", async () => {
    listProjects.mockResolvedValue([]);
    mount();
    expect(await screen.findByTestId("project-empty-state")).toBeInTheDocument();
  });

  it("點專案卡片會呼叫 onOpen", async () => {
    const onOpen = vi.fn();
    mount(onOpen);
    await userEvent.click(await screen.findByText("makemoney"));
    expect(onOpen).toHaveBeenCalledWith("p1");
  });

  it("遺失的專案顯示錯誤而非當機", async () => {
    listProjects.mockResolvedValue([
      project({ status: "missing", error: "專案資料夾或專案檔不存在", counts: { planning: 0, queued: 0, running: 0, done: 0 } }),
    ]);
    mount();
    expect(await screen.findByTestId("project-error-p1")).toHaveTextContent("不存在");
  });

  it("移除專案：兩段式詢問，第二段答否則不刪資料夾", async () => {
    confirmDialog.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    mount();
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));
    await waitFor(() => expect(removeProject).toHaveBeenCalledWith("p1", false));
    expect(confirmDialog).toHaveBeenCalledTimes(2);
  });

  it("移除專案：第二段答是才刪資料夾", async () => {
    confirmDialog.mockResolvedValueOnce(true).mockResolvedValueOnce(true);
    mount();
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));
    await waitFor(() => expect(removeProject).toHaveBeenCalledWith("p1", true));
  });

  it("移除專案：第一段就取消則完全不呼叫 removeProject", async () => {
    confirmDialog.mockResolvedValueOnce(false);
    mount();
    await screen.findByText("makemoney");
    await userEvent.click(screen.getByTestId("project-remove-p1"));
    await waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1));
    expect(removeProject).not.toHaveBeenCalled();
  });
});
