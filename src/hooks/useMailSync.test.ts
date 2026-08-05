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

  // The poller emits one Summary event per inserted message (plus an Important
  // event for important ones), so a single poll cycle ingesting 20 messages
  // fires 20-40 refreshes in a burst. Those round-trips resolve independently,
  // so without a sequence guard the badge is last-RESOLVED-wins rather than
  // last-REQUESTED-wins, and a stale lower count can stick until the next poll.
  it("keeps the newest count when an older in-flight request resolves last", async () => {
    const deferred = () => {
      let resolve!: (n: number) => void;
      const promise = new Promise<number>((r) => { resolve = r; });
      return { promise, resolve };
    };
    const mount = deferred();
    const older = deferred();
    const newer = deferred();
    vi.mocked(mailCountUnread)
      .mockReturnValueOnce(mount.promise)
      .mockReturnValueOnce(older.promise)
      .mockReturnValueOnce(newer.promise);

    const { result } = renderHook(() => useMailSync());
    await act(async () => { mount.resolve(2); });
    expect(result.current.unreadCount).toBe(2);

    // Two overlapping refreshes; the OLDER request is the one that resolves last.
    act(() => {
      result.current.refreshUnread();
      result.current.refreshUnread();
    });
    await act(async () => { newer.resolve(9); });
    await act(async () => { older.resolve(4); });

    expect(result.current.unreadCount).toBe(9);
  });

  // Mail deleted on the server is removed from the local cache by the poller,
  // which emits a `removed` event. Without a refresh here the badge would keep
  // counting messages that no longer exist, for up to poll_interval_secs.
  it("refreshes the unread count when messages are removed server-side", async () => {
    const { result } = renderHook(() => useMailSync());
    await waitFor(() => expect(result.current.unreadCount).toBe(2));

    vi.mocked(mailCountUnread).mockResolvedValue(0);
    listeners["mail-sync-event"]({
      payload: { kind: "removed", account_id: "a1", removed_count: 2 },
    });

    await waitFor(() => expect(result.current.unreadCount).toBe(0));
  });

  it("does not raise a notification for a removal event", async () => {
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    listeners["mail-sync-event"]({
      payload: { kind: "removed", account_id: "a1", removed_count: 2 },
    });

    await new Promise((r) => setTimeout(r, 0));
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("checks notification permission only once across multiple important events", async () => {
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    listeners["mail-sync-event"]({
      payload: { kind: "important", account_id: "a1", message_id: "m1", subject: "s1", summary: "b1" },
    });
    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({ title: "s1", body: "b1" }));

    listeners["mail-sync-event"]({
      payload: { kind: "important", account_id: "a1", message_id: "m2", subject: "s2", summary: "b2" },
    });
    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({ title: "s2", body: "b2" }));

    expect(isPermissionGranted).toHaveBeenCalledTimes(1);
  });

  it("does not re-prompt for permission after the user has denied it", async () => {
    vi.mocked(isPermissionGranted).mockResolvedValue(false);
    vi.mocked(requestPermission).mockResolvedValue("denied");
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    const important = (id: string) => ({
      payload: { kind: "important", account_id: "a1", message_id: id, subject: "s", summary: "b" },
    });
    listeners["mail-sync-event"](important("m1"));
    await waitFor(() => expect(requestPermission).toHaveBeenCalledTimes(1));
    listeners["mail-sync-event"](important("m2"));

    await new Promise((r) => setTimeout(r, 0));
    expect(requestPermission).toHaveBeenCalledTimes(1);
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("falls back to a placeholder title when the subject is empty", async () => {
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    listeners["mail-sync-event"]({
      payload: { kind: "important", account_id: "a1", message_id: "m1", subject: "", summary: "問今天能否開會" },
    });

    // Matches the backend's own placeholder in parse_raw_message.
    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({
      title: "(no subject)", body: "問今天能否開會",
    }));
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

  // Connection health is tracked here as well as in MailView, because MailView
  // only exists while the Mail tab is open: an account that stops connecting
  // while the user sits in a terminal tab would otherwise be invisible.
  describe("connection health", () => {
    const emit = async (payload: unknown) => {
      await act(async () => { listeners["mail-sync-event"]({ payload }); });
    };

    it("marks an account as failing when its connection breaks", async () => {
      const { result } = renderHook(() => useMailSync());
      await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

      await emit({ kind: "connection_failed", account_id: "a1", message: "login failed: bad password" });

      expect(result.current.failedAccountCount).toBe(1);
    });

    it("clears the failure when the account reconnects", async () => {
      const { result } = renderHook(() => useMailSync());
      await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

      await emit({ kind: "connection_failed", account_id: "a1", message: "connection error: timed out" });
      await emit({ kind: "connection_restored", account_id: "a1" });

      expect(result.current.failedAccountCount).toBe(0);
    });

    // The indicator is per-account, not a single global flag: one account coming
    // back must not hide the fact that another one is still down.
    it("keeps reporting a failure while a second account is still down", async () => {
      const { result } = renderHook(() => useMailSync());
      await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

      await emit({ kind: "connection_failed", account_id: "a1", message: "login failed" });
      await emit({ kind: "connection_failed", account_id: "a2", message: "connection error" });
      expect(result.current.failedAccountCount).toBe(2);

      await emit({ kind: "connection_restored", account_id: "a1" });

      expect(result.current.failedAccountCount).toBe(1);
    });

    it("reports no failures while every account is healthy", async () => {
      const { result } = renderHook(() => useMailSync());
      await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

      await emit({ kind: "summary", account_id: "a1", message_id: "m1" });
      await emit({ kind: "important", account_id: "a1", message_id: "m2", subject: "s", summary: "b" });
      await emit({ kind: "removed", account_id: "a1", removed_count: 1 });

      expect(result.current.failedAccountCount).toBe(0);
    });

    // A broken mailbox is a standing condition the user finds when they next
    // look at the sidebar — not something worth interrupting them with, and
    // worse, the poller's reconnect attempts would make it recur.
    it("does not raise an OS notification for a connection failure", async () => {
      renderHook(() => useMailSync());
      await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

      await emit({ kind: "connection_failed", account_id: "a1", message: "login failed" });

      await new Promise((r) => setTimeout(r, 0));
      expect(sendNotification).not.toHaveBeenCalled();
    });
  });
});
