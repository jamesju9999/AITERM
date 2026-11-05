import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/projects", () => ({
  usedDirs: vi.fn().mockResolvedValue([]),
  usedLabels: vi.fn().mockResolvedValue([]),
}));
const addAttachment = vi.fn().mockResolvedValue({ id: "a1", filename: "x" });
vi.mock("../../ipc/tasks", () => ({
  createTask: vi.fn(),
  updateTask: vi.fn(),
  addAttachment: (...a: unknown[]) => addAttachment(...a),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
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

beforeEach(() => vi.clearAllMocks());

describe("TaskEditorDialog 剪貼簿貼上附件", () => {
  it("貼上截圖會加進附件清單，並改成不撞名的檔名", async () => {
    mount();
    const shot = new File(["x"], "image.png", { type: "image/png" });
    fireEvent.paste(screen.getByTestId("task-title-input"), { clipboardData: { files: [shot] } });
    await waitFor(() => expect(screen.getByText(/^pasted-\d{14}\.png$/)).toBeTruthy());
  });

  it("純文字貼上不被攔截", () => {
    mount();
    const notPrevented = fireEvent.paste(screen.getByTestId("task-title-input"), {
      clipboardData: { files: [], getData: () => "hi" },
    });
    expect(notPrevented).toBe(true);
  });
});
