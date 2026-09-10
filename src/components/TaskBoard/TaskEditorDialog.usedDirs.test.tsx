import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedDirs = vi.fn();
vi.mock("../../ipc/projects", () => ({
  usedDirs: (...a: unknown[]) => usedDirs(...a),
  usedLabels: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/tasks", () => ({
  createTask: vi.fn(),
  updateTask: vi.fn(),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
// TaskEditorDialog 掛載時的 useEffect 會呼叫這兩個——沒 mock 的話會落到真正的
// `invoke()`，在 jsdom 裡（沒有 Tauri runtime）丟出 unhandled rejection，讓整個
// 測試檔案的結果變成失敗，即使每一個具名測試自己都通過（CI 因此紅、本機用
// `npm run test` 的摘要文字卻看不出來，因為摘要只列具名測試，不代表 process
// 真正的 exit code）。
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn().mockResolvedValue([]) }));
vi.mock("../../ipc/bridge", () => ({
  bridgeStatus: vi.fn().mockResolvedValue({ running: false, port: null, token: null, error: null }),
}));

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
