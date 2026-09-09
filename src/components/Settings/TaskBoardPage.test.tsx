import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/tasks", () => ({
  getTaskBoardConfig: vi.fn(),
  setTaskBoardConfig: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("../../ipc/telegram", () => ({
  getTelegramConfig: vi.fn(),
}));

import { getTaskBoardConfig, setTaskBoardConfig } from "../../ipc/tasks";
import { getTelegramConfig } from "../../ipc/telegram";
import { TaskBoardPage } from "./TaskBoardPage";

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getTaskBoardConfig).mockResolvedValue({
    max_concurrent: 2,
    claude_command: "claude",
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
    stuck_timeout_secs: 1200,
  });
  vi.mocked(getTelegramConfig).mockResolvedValue({ bot_token: null, chat_id: null });
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

  it("toggling the auto-close checkbox sends the new value", async () => {
    const user = userEvent.setup();
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    const checkbox = screen.getByRole("checkbox", { name: /自動關閉分頁|Auto-close tabs/ });
    expect(checkbox).toBeChecked();
    await user.click(checkbox);
    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));
    await waitFor(() =>
      expect(setTaskBoardConfig).toHaveBeenCalledWith(
        expect.objectContaining({ auto_close_finished_tabs: false }),
      ),
    );
  });

  it("預設顯示桌面通知 checkbox，已勾選", async () => {
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    expect(
      screen.getByRole("checkbox", { name: /完成時發桌面通知|Desktop notification/ }),
    ).toBeChecked();
  });

  it("Telegram 未設定時不顯示 Telegram checkbox", async () => {
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    expect(
      screen.queryByRole("checkbox", { name: /Telegram/ }),
    ).not.toBeInTheDocument();
  });

  it("Telegram 已設定時顯示 checkbox，已勾選", async () => {
    vi.mocked(getTelegramConfig).mockResolvedValue({ bot_token: "abc", chat_id: "123" });
    view();
    await waitFor(() =>
      expect(screen.getByRole("checkbox", { name: /Telegram/ })).toBeChecked(),
    );
  });

  it("顯示疑似卡住判定時間，換算成分鐘（1200 秒 → 20）", async () => {
    view();
    await waitFor(() => expect(screen.getByDisplayValue("20")).toBeInTheDocument());
  });

  it("修改疑似卡住判定時間後儲存，換算回秒數送出", async () => {
    const user = userEvent.setup();
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    const n = screen.getByDisplayValue("20");
    await user.clear(n);
    await user.type(n, "5");
    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));
    await waitFor(() =>
      expect(setTaskBoardConfig).toHaveBeenCalledWith(
        expect.objectContaining({ stuck_timeout_secs: 300 }),
      ),
    );
  });
});
