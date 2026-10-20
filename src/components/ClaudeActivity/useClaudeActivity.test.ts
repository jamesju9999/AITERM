import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ActivityNode, ActivitySnapshot } from "../../ipc/claudeActivity";

const getClaudeActivity = vi.fn();
const handlers = new Map<string, (snapshot: ActivitySnapshot) => void>();
const unlisten = vi.fn();
vi.mock("../../ipc/claudeActivity", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ipc/claudeActivity")>()),
  getClaudeActivity: (...a: unknown[]) => getClaudeActivity(...a),
  listenClaudeActivity: (sid: string, cb: (n: ActivitySnapshot) => void) => {
    handlers.set(sid, cb);
    return Promise.resolve(unlisten);
  },
}));

import { useClaudeActivity } from "./useClaudeActivity";

const node = (id: string): ActivityNode => ({
  id, parent: null, agent_type: "general-purpose", description: id, status: "running",
  started_ms: 1, ended_ms: null, current_tool: null, summary: null, tokens: null, tool_count: null, last_event_ms: 1_000, turn: 1,
});
const snap = (...nodes: ActivityNode[]): ActivitySnapshot => ({ nodes, main_tool: null, main_tool_count: 0, main_recent: [], turn: 1 });

beforeEach(() => {
  getClaudeActivity.mockReset();
  handlers.clear();
  unlisten.mockClear();
});

describe("useClaudeActivity", () => {
  it("掛載時取快照；之後事件整份取代", async () => {
    getClaudeActivity.mockResolvedValue(snap(node("a")));
    const { result } = renderHook(() => useClaudeActivity("s1"));
    await waitFor(() => expect(result.current.nodes.map((n) => n.id)).toEqual(["a"]));
    act(() => handlers.get("s1")!(snap(node("a"), node("b"))));
    expect(result.current.nodes.map((n) => n.id)).toEqual(["a", "b"]);
  });

  it("事件比快照先到：較舊的快照不能蓋掉較新的事件", async () => {
    let resolveSnap!: (n: ActivitySnapshot) => void;
    getClaudeActivity.mockReturnValue(new Promise((r) => { resolveSnap = r; }));
    const { result } = renderHook(() => useClaudeActivity("s1"));
    await waitFor(() => expect(handlers.has("s1")).toBe(true));
    act(() => handlers.get("s1")!(snap(node("new"))));
    await act(async () => resolveSnap(snap(node("old"))));
    expect(result.current.nodes.map((n) => n.id)).toEqual(["new"]);
  });

  it("沒有 sessionId 時什麼都不訂閱", () => {
    renderHook(() => useClaudeActivity(""));
    expect(getClaudeActivity).not.toHaveBeenCalled();
    expect(handlers.size).toBe(0);
  });

  it("卸載後退訂，且晚到的事件不會 setState", async () => {
    getClaudeActivity.mockResolvedValue(snap());
    const { unmount } = renderHook(() => useClaudeActivity("s1"));
    await waitFor(() => expect(handlers.has("s1")).toBe(true));
    unmount();
    await waitFor(() => expect(unlisten).toHaveBeenCalled());
    expect(() => handlers.get("s1")!(snap(node("z")))).not.toThrow();
  });

  it("快照失敗不炸（沒有側欄而已）", async () => {
    getClaudeActivity.mockRejectedValue(new Error("boom"));
    const { result } = renderHook(() => useClaudeActivity("s1"));
    await waitFor(() => expect(getClaudeActivity).toHaveBeenCalled());
    expect(result.current.nodes).toEqual([]);
  });
});
