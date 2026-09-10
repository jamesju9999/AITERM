import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedDirs = vi.fn();
const usedLabels = vi.fn();
vi.mock("../../ipc/projects", () => ({
  usedDirs: (...a: unknown[]) => usedDirs(...a),
  usedLabels: (...a: unknown[]) => usedLabels(...a),
}));
const createTask = vi.fn().mockResolvedValue("new-id");
const updateTask = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tasks", () => ({
  createTask: (...a: unknown[]) => createTask(...a),
  updateTask: (...a: unknown[]) => updateTask(...a),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
// TaskEditorDialog 掛載時的 useEffect 會呼叫這兩個——沒 mock 的話會落到真正的
// `invoke()`，在 jsdom 裡丟出 unhandled rejection，讓整個測試檔案的結果變成
// 失敗，即使每一個具名測試自己都通過。
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

describe("TaskEditorDialog Label 欄位", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    usedDirs.mockResolvedValue([]);
    localStorage.clear();
  });

  it("列出這個專案用過的 Label", async () => {
    usedLabels.mockResolvedValue(["緊急", "文件"]);
    mount();
    expect(await screen.findByTestId("used-label-緊急")).toBeInTheDocument();
    expect(screen.getByTestId("used-label-文件")).toBeInTheDocument();
    expect(usedLabels).toHaveBeenCalledWith("p1");
  });

  it("點快捷選項會填入 Label 欄", async () => {
    usedLabels.mockResolvedValue(["緊急"]);
    mount();
    await userEvent.click(await screen.findByTestId("used-label-緊急"));
    expect(screen.getByTestId("task-label-input")).toHaveValue("緊急");
  });

  it("沒有用過的 Label 時不顯示這一區", async () => {
    usedLabels.mockResolvedValue([]);
    mount();
    await screen.findByTestId("task-label-input");
    expect(screen.queryByTestId("used-labels-row")).not.toBeInTheDocument();
  });

  it("儲存新卡片時，Label 有打字就照原樣送出（trim 過）", async () => {
    usedLabels.mockResolvedValue([]);
    mount();
    await userEvent.type(screen.getByTestId("task-title-input"), "標題");
    await userEvent.type(screen.getByTestId("task-dir-input"), "/repo");
    await userEvent.type(screen.getByTestId("task-label-input"), "  緊急  ");
    await userEvent.click(screen.getByRole("button", { name: /儲存|Save/ }));
    expect(createTask).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({ label: "緊急" }),
    );
  });

  it("儲存新卡片時，Label 是空白就送 null", async () => {
    usedLabels.mockResolvedValue([]);
    mount();
    await userEvent.type(screen.getByTestId("task-title-input"), "標題");
    await userEvent.type(screen.getByTestId("task-dir-input"), "/repo");
    await userEvent.click(screen.getByRole("button", { name: /儲存|Save/ }));
    expect(createTask).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({ label: null }),
    );
  });
});
