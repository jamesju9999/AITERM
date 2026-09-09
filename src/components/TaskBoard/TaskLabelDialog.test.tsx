import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedLabels = vi.fn();
vi.mock("../../ipc/projects", () => ({
  usedLabels: (...a: unknown[]) => usedLabels(...a),
}));
const setTaskLabel = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tasks", () => ({
  setTaskLabel: (...a: unknown[]) => setTaskLabel(...a),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskLabelDialog } from "./TaskLabelDialog";

const mount = (label: string | null, onSaved = vi.fn(), onClose = vi.fn()) =>
  render(
    <LocaleProvider>
      <TaskLabelDialog
        projectId="p1"
        taskId="t1"
        label={label}
        onClose={onClose}
        onSaved={onSaved}
      />
    </LocaleProvider>,
  );

describe("TaskLabelDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    usedLabels.mockResolvedValue([]);
  });

  it("預先填入現有的 label", () => {
    mount("緊急");
    expect(screen.getByTestId("task-label-quick-input")).toHaveValue("緊急");
  });

  it("列出這個專案用過的 Label 快捷選項", async () => {
    usedLabels.mockResolvedValue(["緊急", "文件"]);
    mount(null);
    expect(await screen.findByTestId("used-label-quick-緊急")).toBeInTheDocument();
    expect(screen.getByTestId("used-label-quick-文件")).toBeInTheDocument();
    expect(usedLabels).toHaveBeenCalledWith("p1");
  });

  it("點快捷選項會填入輸入框", async () => {
    usedLabels.mockResolvedValue(["緊急"]);
    mount(null);
    await userEvent.click(await screen.findByTestId("used-label-quick-緊急"));
    expect(screen.getByTestId("task-label-quick-input")).toHaveValue("緊急");
  });

  it("儲存時呼叫 setTaskLabel（trim 過），成功後呼叫 onSaved", async () => {
    const onSaved = vi.fn();
    mount("", onSaved);
    await userEvent.type(screen.getByTestId("task-label-quick-input"), "  文件  ");
    await userEvent.click(screen.getByRole("button", { name: /儲存|Save/ }));
    expect(setTaskLabel).toHaveBeenCalledWith("p1", "t1", "文件");
    expect(onSaved).toHaveBeenCalled();
  });

  it("清空後儲存會送 null", async () => {
    const onSaved = vi.fn();
    mount("緊急", onSaved);
    await userEvent.clear(screen.getByTestId("task-label-quick-input"));
    await userEvent.click(screen.getByRole("button", { name: /儲存|Save/ }));
    expect(setTaskLabel).toHaveBeenCalledWith("p1", "t1", null);
  });

  it("點取消不會呼叫 setTaskLabel", async () => {
    const onClose = vi.fn();
    mount("緊急", vi.fn(), onClose);
    await userEvent.click(screen.getByRole("button", { name: /取消|Cancel/ }));
    expect(setTaskLabel).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });
});
