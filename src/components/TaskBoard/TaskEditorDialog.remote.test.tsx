import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedDirs = vi.fn();
const usedLabels = vi.fn();
vi.mock("../../ipc/projects", () => ({
  usedDirs: (...a: unknown[]) => usedDirs(...a),
  usedLabels: (...a: unknown[]) => usedLabels(...a),
}));
vi.mock("../../ipc/tasks", () => ({
  createTask: vi.fn(),
  updateTask: vi.fn(),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn().mockResolvedValue([]) }));
vi.mock("../../ipc/bridge", () => ({
  bridgeStatus: vi.fn().mockResolvedValue({ running: false, port: null, token: null, error: null }),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskEditorDialog } from "./TaskEditorDialog";

const mount = (projectId: string) =>
  render(
    <LocaleProvider>
      <TaskEditorDialog projectId={projectId} card={null} onClose={vi.fn()} onSaved={vi.fn()} />
    </LocaleProvider>,
  );

beforeEach(() => {
  vi.clearAllMocks();
  usedDirs.mockResolvedValue([]);
  usedLabels.mockResolvedValue([]);
});

describe("新增工作對話框：共用看板（專案 id 是 remote:…）", () => {
  it("卡片在領取者的電腦執行，所以專案資料夾、AI 工具、worktree 隔離不顯示", () => {
    mount("remote:srv-1");
    expect(screen.queryByTestId("task-dir-input")).toBeNull();
    expect(screen.queryByTestId("task-cli-select")).toBeNull();
    expect(screen.queryByTestId("task-isolate-select")).toBeNull();
    // 標題、內容、群組、互動模式等共用的欄位仍在。
    expect(screen.getByTestId("task-label-input")).toBeTruthy();
  });

  it("本機看板的對話框完全不變", () => {
    mount("p1");
    expect(screen.getByTestId("task-dir-input")).toBeTruthy();
    expect(screen.getByTestId("task-cli-select")).toBeTruthy();
    expect(screen.getByTestId("task-isolate-select")).toBeTruthy();
  });
});
