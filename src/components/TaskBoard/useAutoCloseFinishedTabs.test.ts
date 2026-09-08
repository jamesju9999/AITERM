import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RefObject } from "react";

const getTaskBoardConfig = vi.fn();
vi.mock("../../ipc/tasks", () => ({
  getTaskBoardConfig: (...a: unknown[]) => getTaskBoardConfig(...a),
}));

let taskFinishedHandler: ((e: { payload: unknown }) => void) | null = null;
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    if (event === "task-finished") taskFinishedHandler = handler;
    return Promise.resolve(() => {});
  }),
}));

import { useAutoCloseFinishedTabs } from "./useAutoCloseFinishedTabs";

function activeRef(id: string): RefObject<string> {
  return { current: id };
}

const PAYLOAD = { project_id: "p1", task_id: "t1", tab_id: "tab-9", outcome: "success" };

beforeEach(() => {
  vi.clearAllMocks();
  taskFinishedHandler = null;
  getTaskBoardConfig.mockResolvedValue({
    max_concurrent: 1,
    claude_command: "claude",
    auto_close_finished_tabs: true,
  });
});

describe("useAutoCloseFinishedTabs", () => {
  it("成功、非目前分頁、設定開啟 → dispatch aiterm:close-tab", async () => {
    renderHook(() => useAutoCloseFinishedTabs(activeRef("other-tab")));
    expect(taskFinishedHandler).not.toBeNull();

    const events: CustomEvent<{ tabId?: string }>[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent<{ tabId?: string }>);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: PAYLOAD });
      await waitFor(() => expect(events).toHaveLength(1));
      expect(events[0].detail.tabId).toBe("tab-9");
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });

  it("取消也會關", async () => {
    renderHook(() => useAutoCloseFinishedTabs(activeRef("other-tab")));
    const events: CustomEvent<{ tabId?: string }>[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent<{ tabId?: string }>);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: { ...PAYLOAD, outcome: "cancelled" } });
      await waitFor(() => expect(events).toHaveLength(1));
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });

  it("失敗不關", async () => {
    renderHook(() => useAutoCloseFinishedTabs(activeRef("other-tab")));
    const events: CustomEvent[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: { ...PAYLOAD, outcome: "failed" } });
      await new Promise((r) => setTimeout(r, 0));
      expect(events).toHaveLength(0);
      expect(getTaskBoardConfig).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });

  it("是目前正在看的分頁時不關", async () => {
    renderHook(() => useAutoCloseFinishedTabs(activeRef("tab-9")));
    const events: CustomEvent[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: PAYLOAD });
      await new Promise((r) => setTimeout(r, 0));
      expect(events).toHaveLength(0);
      expect(getTaskBoardConfig).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });

  it("設定關閉時不關", async () => {
    getTaskBoardConfig.mockResolvedValue({
      max_concurrent: 1,
      claude_command: "claude",
      auto_close_finished_tabs: false,
    });
    renderHook(() => useAutoCloseFinishedTabs(activeRef("other-tab")));
    const events: CustomEvent[] = [];
    const onClose = (e: Event) => events.push(e as CustomEvent);
    window.addEventListener("aiterm:close-tab", onClose);
    try {
      taskFinishedHandler!({ payload: PAYLOAD });
      await waitFor(() => expect(getTaskBoardConfig).toHaveBeenCalled());
      await new Promise((r) => setTimeout(r, 0));
      expect(events).toHaveLength(0);
    } finally {
      window.removeEventListener("aiterm:close-tab", onClose);
    }
  });
});
