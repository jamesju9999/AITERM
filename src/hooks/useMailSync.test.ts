import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { useMailSync } from "./useMailSync";

const listeners: Record<string, (event: { payload: unknown }) => void> = {};

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((eventName: string, cb: (event: { payload: unknown }) => void) => {
    listeners[eventName] = cb;
    return Promise.resolve(() => { delete listeners[eventName]; });
  }),
}));

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn().mockResolvedValue(true),
  requestPermission: vi.fn().mockResolvedValue("granted"),
  sendNotification: vi.fn(),
}));

vi.mock("../ipc/mail", () => ({
  MAIL_SYNC_EVENT: "mail-sync-event",
  mailCountUnread: vi.fn().mockResolvedValue(2),
}));

import { mailCountUnread } from "../ipc/mail";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

describe("useMailSync", () => {
  beforeEach(() => {
    for (const key of Object.keys(listeners)) delete listeners[key];
    vi.clearAllMocks();
    vi.mocked(mailCountUnread).mockResolvedValue(2);
    vi.mocked(isPermissionGranted).mockResolvedValue(true);
    vi.mocked(requestPermission).mockResolvedValue("granted");
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("loads the initial unread count on mount", async () => {
    const { result } = renderHook(() => useMailSync());
    await waitFor(() => expect(result.current.unreadCount).toBe(2));
  });

  it("sends an OS notification when an important mail-sync-event arrives", async () => {
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    listeners["mail-sync-event"]({
      payload: { kind: "important", account_id: "a1", message_id: "m1", subject: "老闆找你", summary: "問今天能否開會" },
    });

    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({ title: "老闆找你", body: "問今天能否開會" }));
  });

  it("does not send a notification for a plain summary event", async () => {
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    listeners["mail-sync-event"]({
      payload: { kind: "summary", account_id: "a1", message_id: "m1" },
    });

    await new Promise((r) => setTimeout(r, 0));
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("refreshes the unread count when a sync event arrives", async () => {
    const { result } = renderHook(() => useMailSync());
    await waitFor(() => expect(result.current.unreadCount).toBe(2));

    vi.mocked(mailCountUnread).mockResolvedValue(7);
    listeners["mail-sync-event"]({
      payload: { kind: "summary", account_id: "a1", message_id: "m1" },
    });

    await waitFor(() => expect(result.current.unreadCount).toBe(7));
  });

  // Carry-forward fix: MailView marks a message read locally without any sync
  // event coming back from the backend, so the badge would keep showing a stale
  // count for up to poll_interval_secs (default 300s). The hook therefore hands
  // out `refreshUnread` for MailView to call after a successful mark-read.
  it("re-fetches the count when refreshUnread is called after a local mark-read", async () => {
    const { result } = renderHook(() => useMailSync());
    await waitFor(() => expect(result.current.unreadCount).toBe(2));

    vi.mocked(mailCountUnread).mockResolvedValue(1);
    await act(async () => { result.current.refreshUnread(); });

    expect(result.current.unreadCount).toBe(1);
  });

  it("requests permission before notifying when it has not been granted yet", async () => {
    vi.mocked(isPermissionGranted).mockResolvedValue(false);
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    listeners["mail-sync-event"]({
      payload: { kind: "important", account_id: "a1", message_id: "m1", subject: "s", summary: "b" },
    });

    await waitFor(() => expect(requestPermission).toHaveBeenCalled());
    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({ title: "s", body: "b" }));
  });

  it("does not notify when notification permission is denied", async () => {
    vi.mocked(isPermissionGranted).mockResolvedValue(false);
    vi.mocked(requestPermission).mockResolvedValue("denied");
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    listeners["mail-sync-event"]({
      payload: { kind: "important", account_id: "a1", message_id: "m1", subject: "s", summary: "b" },
    });

    await waitFor(() => expect(requestPermission).toHaveBeenCalled());
    expect(sendNotification).not.toHaveBeenCalled();
  });
});
