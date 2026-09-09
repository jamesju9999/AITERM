import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RefObject } from "react";

const getTaskBoardConfig = vi.fn();
vi.mock("../../ipc/tasks", () => ({
  getTaskBoardConfig: (...a: unknown[]) => getTaskBoardConfig(...a),
}));

const sendTelegramMessage = vi.fn();
vi.mock("../../ipc/telegram", () => ({
  sendTelegramMessage: (...a: unknown[]) => sendTelegramMessage(...a),
}));

const sendNotification = vi.fn();
vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: (...a: unknown[]) => sendNotification(...a),
}));

const ensureNotificationPermission = vi.fn();
vi.mock("../../lib/notifyPermission", () => ({
  ensureNotificationPermission: (...a: unknown[]) => ensureNotificationPermission(...a),
}));

let taskFinishedHandler: ((e: { payload: unknown }) => void) | null = null;
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    if (event === "task-finished") taskFinishedHandler = handler;
    return Promise.resolve(() => {});
  }),
}));

import { useTaskCompletionNotifications } from "./useTaskCompletionNotifications";

function activeRef(id: string): RefObject<string> {
  return { current: id };
}

const PAYLOAD = {
  project_id: "p1",
  task_id: "t1",
  tab_id: "tab-9",
  outcome: "success" as const,
  title: "修 bug",
  project_name: "AITerm",
  error_message: null as string | null,
};

beforeEach(() => {
  vi.clearAllMocks();
  taskFinishedHandler = null;
  getTaskBoardConfig.mockResolvedValue({
    max_concurrent: 1,
    claude_command: "claude",
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
  });
  ensureNotificationPermission.mockResolvedValue(true);
  sendTelegramMessage.mockResolvedValue(undefined);
});

describe("useTaskCompletionNotifications", () => {
  it("成功、非目前分頁 → 桌面通知與 Telegram 都發", async () => {
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    expect(taskFinishedHandler).not.toBeNull();

    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({
      title: "修 bug",
      body: "AITerm · 完成",
    }));
    expect(sendTelegramMessage).toHaveBeenCalledWith("✅ AITerm — 修 bug\n完成");
  });

  it("是目前正在看的分頁 → 不發桌面通知，但仍發 Telegram", async () => {
    renderHook(() => useTaskCompletionNotifications(activeRef("tab-9")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendTelegramMessage).toHaveBeenCalled());
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("失敗時兩個管道都帶上失敗原因", async () => {
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({
      payload: { ...PAYLOAD, outcome: "failed", error_message: "claude 以 exit code 1 結束" },
    });

    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({
      title: "修 bug",
      body: "AITerm · 失敗：claude 以 exit code 1 結束",
    }));
    expect(sendTelegramMessage).toHaveBeenCalledWith(
      "❌ AITerm — 修 bug\n失敗：claude 以 exit code 1 結束",
    );
  });

  it("取消的文字", async () => {
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: { ...PAYLOAD, outcome: "cancelled" } });

    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({
      title: "修 bug",
      body: "AITerm · 已取消",
    }));
    expect(sendTelegramMessage).toHaveBeenCalledWith("⏹️ AITerm — 修 bug\n已取消");
  });

  it("notify_desktop_on_finish 關閉時不發桌面通知，Telegram 照發", async () => {
    getTaskBoardConfig.mockResolvedValue({
      max_concurrent: 1,
      claude_command: "claude",
      auto_close_finished_tabs: true,
      notify_desktop_on_finish: false,
      notify_telegram_on_finish: true,
    });
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendTelegramMessage).toHaveBeenCalled());
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("notify_telegram_on_finish 關閉時不發 Telegram，桌面通知照發", async () => {
    getTaskBoardConfig.mockResolvedValue({
      max_concurrent: 1,
      claude_command: "claude",
      auto_close_finished_tabs: true,
      notify_desktop_on_finish: true,
      notify_telegram_on_finish: false,
    });
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendNotification).toHaveBeenCalled());
    expect(sendTelegramMessage).not.toHaveBeenCalled();
  });

  it("沒有通知權限時不發桌面通知", async () => {
    ensureNotificationPermission.mockResolvedValue(false);
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendTelegramMessage).toHaveBeenCalled());
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("Telegram 發送失敗不拋出、不影響桌面通知", async () => {
    sendTelegramMessage.mockRejectedValue(new Error("no bot token configured"));
    renderHook(() => useTaskCompletionNotifications(activeRef("other-tab")));
    taskFinishedHandler!({ payload: PAYLOAD });

    await waitFor(() => expect(sendNotification).toHaveBeenCalled());
  });
});
