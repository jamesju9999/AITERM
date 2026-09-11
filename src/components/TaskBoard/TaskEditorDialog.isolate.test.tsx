import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedDirs = vi.fn();
const usedLabels = vi.fn();
vi.mock("../../ipc/projects", () => ({
  usedDirs: (...a: unknown[]) => usedDirs(...a),
  usedLabels: (...a: unknown[]) => usedLabels(...a),
}));
const createTask = vi.fn().mockResolvedValue("new-id");
const updateTask = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tasks", () => ({
  createTask: (...a: unknown[]) => createTask(...a),
  updateTask: (...a: unknown[]) => updateTask(...a),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
// 掛載時的 useEffect 會呼叫這兩個——沒 mock 會落到真正的 invoke()，
// 在 jsdom 裡變成 unhandled rejection 把整個檔案判成失敗。
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn().mockResolvedValue([]) }));
vi.mock("../../ipc/bridge", () => ({
  bridgeStatus: vi.fn().mockResolvedValue({ running: false, port: null, token: null, error: null }),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskEditorDialog } from "./TaskEditorDialog";
import type { TaskWithAttachments } from "../../ipc/tasks";

const existing = (isolate: boolean | null): TaskWithAttachments => ({
  id: "t1", title: "既有卡片", body: "", project_dir: "/r", label: null,
  status: "planning", parallel_ok: true, interactive: false, sort_order: 1,
  outcome: null, tab_id: null, transcript_path: null, error_message: null,
  created_at: "2026-09-11", dispatched_at: null, finished_at: null,
  ai_summary: null, archived_at: null, session_id: null, session_path: null,
  use_bridge: false, bridge_tiers: null, worktree_path: null,
  worktree_branch: null, isolate_worktree: isolate, attachments: [],
});

const mount = (card: TaskWithAttachments | null = null) =>
  render(
    <LocaleProvider>
      <TaskEditorDialog projectId="p1" card={card} onClose={vi.fn()} onSaved={vi.fn()} />
    </LocaleProvider>,
  );

beforeEach(() => {
  vi.clearAllMocks();
  usedDirs.mockResolvedValue([]);
  usedLabels.mockResolvedValue([]);
  localStorage.clear();
});

describe("TaskEditorDialog 的 worktree 隔離選項", () => {
  it("預設是沿用全域設定，存出 null", async () => {
    mount();
    await userEvent.type(screen.getByTestId("task-title-input"), "標題");
    await userEvent.type(screen.getByTestId("task-dir-input"), "/r");
    await userEvent.click(screen.getByText("儲存"));

    expect(createTask).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({ isolate_worktree: null }),
    );
  });

  it("選「直接在專案目錄執行」存出 false", async () => {
    mount();
    await userEvent.type(screen.getByTestId("task-title-input"), "標題");
    await userEvent.type(screen.getByTestId("task-dir-input"), "/r");
    await userEvent.selectOptions(screen.getByTestId("task-isolate-select"), "off");
    await userEvent.click(screen.getByText("儲存"));

    expect(createTask).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({ isolate_worktree: false }),
    );
  });

  it("選「建立獨立的 worktree」存出 true", async () => {
    mount();
    await userEvent.type(screen.getByTestId("task-title-input"), "標題");
    await userEvent.type(screen.getByTestId("task-dir-input"), "/r");
    await userEvent.selectOptions(screen.getByTestId("task-isolate-select"), "on");
    await userEvent.click(screen.getByText("儲存"));

    expect(createTask).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({ isolate_worktree: true }),
    );
  });

  it("開啟既有卡片時回填它自己的設定", async () => {
    mount(existing(true));
    const sel = await screen.findByTestId<HTMLSelectElement>("task-isolate-select");
    expect(sel.value).toBe("on");
  });

  it("既有卡片是 false 時回填成「直接在專案目錄執行」", async () => {
    mount(existing(false));
    const sel = await screen.findByTestId<HTMLSelectElement>("task-isolate-select");
    expect(sel.value).toBe("off");
  });

  it("既有卡片是 null 時回填成「沿用全域設定」", async () => {
    mount(existing(null));
    const sel = await screen.findByTestId<HTMLSelectElement>("task-isolate-select");
    expect(sel.value).toBe("inherit");
  });
});
