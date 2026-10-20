import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ActivityNode, ActivitySnapshot } from "../../ipc/claudeActivity";

const sendNotification = vi.fn();
vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: (...a: unknown[]) => sendNotification(...a),
  isPermissionGranted: () => Promise.resolve(true),
  requestPermission: () => Promise.resolve("granted"),
}));

import { resetNotificationPermissionForTests } from "../../lib/notifyPermission";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { setClaudeActivityNotifyEnabled } from "./settings";
import { STUCK_MS } from "./alerts";
import { useClaudeActivityAlerts } from "./useClaudeActivityAlerts";
import { createElement, type ReactNode } from "react";

const NOW = 5_000_000;
const n = (id: string, status: ActivityNode["status"], over: Partial<ActivityNode> = {}): ActivityNode => ({
  id, parent: null, agent_type: "x", description: id, status, started_ms: NOW - 5_000,
  ended_ms: status === "running" ? null : NOW - 500, current_tool: null, summary: null, tokens: null,
  tool_count: null, last_event_ms: NOW - 500, turn: 1, ...over,
});
const snap = (...nodes: ActivityNode[]): ActivitySnapshot => ({
  nodes, main_tool: null, main_tool_count: 0, main_recent: [], turn: 1,
});
const wrapper = ({ children }: { children: ReactNode }) => createElement(LocaleProvider, null, children);

const flush = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  sendNotification.mockClear();
  resetNotificationPermissionForTests();
  localStorage.clear();
  vi.spyOn(document, "hasFocus").mockReturnValue(false);
});
afterEach(() => vi.useRealTimers());

const mount = (initial: ActivitySnapshot, isActive = true) =>
  renderHook(({ s, a }) => useClaudeActivityAlerts(s, a), { wrapper, initialProps: { s: initial, a: isActive } });

describe("useClaudeActivityAlerts", () => {
  it("沒在看（視窗沒焦點）時，節點失敗發通知", async () => {
    const h = mount(snap(n("a", "running"), n("b", "running")));
    h.rerender({ s: snap(n("a", "failed"), n("b", "running")), a: true });
    await flush();
    expect(sendNotification).toHaveBeenCalledTimes(1);
    expect(sendNotification.mock.calls[0][0].body).toContain("a");
  });

  it("正在看這個分頁（作用中且視窗有焦點）時不發", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const h = mount(snap(n("a", "running")));
    h.rerender({ s: snap(n("a", "failed")), a: true });
    await flush();
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("分頁在背景（即使視窗有焦點）時會發", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const h = mount(snap(n("a", "running")), false);
    h.rerender({ s: snap(n("a", "done")), a: false });
    await flush();
    expect(sendNotification).toHaveBeenCalledTimes(1);
  });

  it("設定關閉就不發", async () => {
    setClaudeActivityNotifyEnabled(false);
    const h = mount(snap(n("a", "running")));
    h.rerender({ s: snap(n("a", "failed")), a: true });
    await flush();
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("running 超過門檻沒動靜：卡住通知，且同一節點只通知一次", async () => {
    mount(snap(n("a", "running", { last_event_ms: NOW })));
    await act(async () => { vi.advanceTimersByTime(STUCK_MS + 30_000); });
    await flush();
    expect(sendNotification).toHaveBeenCalledTimes(1);
    expect(sendNotification.mock.calls[0][0].body).toContain("3");
    await act(async () => { vi.advanceTimersByTime(120_000); });
    await flush();
    expect(sendNotification).toHaveBeenCalledTimes(1);
  });
});
