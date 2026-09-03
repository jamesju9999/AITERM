import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/tasks", () => ({
  getTaskBoardConfig: vi.fn(),
  setTaskBoardConfig: vi.fn().mockResolvedValue(undefined),
}));

import { getTaskBoardConfig, setTaskBoardConfig } from "../../ipc/tasks";
import { TaskBoardPage } from "./TaskBoardPage";

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getTaskBoardConfig).mockResolvedValue({ max_concurrent: 2, claude_command: "claude" });
});

const view = () => render(<LocaleProvider><TaskBoardPage /></LocaleProvider>);

describe("TaskBoardPage", () => {
  it("loads and shows the saved config", async () => {
    view();
    await waitFor(() => expect(screen.getByDisplayValue("2")).toBeInTheDocument());
    expect(screen.getByDisplayValue("claude")).toBeInTheDocument();
  });

  it("saving sends the edited values", async () => {
    const user = userEvent.setup();
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    const n = screen.getByDisplayValue("2");
    await user.clear(n);
    await user.type(n, "3");
    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));
    await waitFor(() =>
      expect(setTaskBoardConfig).toHaveBeenCalledWith(
        expect.objectContaining({ max_concurrent: 3, claude_command: "claude" }),
      ),
    );
  });
});
