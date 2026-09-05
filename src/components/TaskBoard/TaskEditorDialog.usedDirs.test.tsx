import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedDirs = vi.fn();
vi.mock("../../ipc/projects", () => ({ usedDirs: (...a: unknown[]) => usedDirs(...a) }));
vi.mock("../../ipc/tasks", () => ({
  createTask: vi.fn(),
  updateTask: vi.fn(),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskEditorDialog } from "./TaskEditorDialog";

const mount = () =>
  render(
    <LocaleProvider>
      <TaskEditorDialog projectId="p1" card={null} onClose={vi.fn()} onSaved={vi.fn()} />
    </LocaleProvider>,
  );

describe("TaskEditorDialog 工作目錄快捷選項", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it("列出這個專案用過的目錄", async () => {
    usedDirs.mockResolvedValue(["/repo/web", "/repo/api"]);
    mount();
    expect(await screen.findByTestId("used-dir-/repo/web")).toBeInTheDocument();
    expect(screen.getByTestId("used-dir-/repo/api")).toBeInTheDocument();
    expect(usedDirs).toHaveBeenCalledWith("p1");
  });

  it("點快捷選項會填入目錄欄", async () => {
    usedDirs.mockResolvedValue(["/repo/web"]);
    mount();
    await userEvent.click(await screen.findByTestId("used-dir-/repo/web"));
    expect(screen.getByTestId("task-dir-input")).toHaveValue("/repo/web");
  });

  it("沒有用過的目錄時不顯示這一區", async () => {
    usedDirs.mockResolvedValue([]);
    mount();
    await screen.findByTestId("task-dir-input");
    expect(screen.queryByTestId("used-dirs-row")).not.toBeInTheDocument();
  });
});
