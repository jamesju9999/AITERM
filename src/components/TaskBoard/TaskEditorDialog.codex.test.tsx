import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/projects", () => ({
  usedDirs: vi.fn().mockResolvedValue([]),
  usedLabels: vi.fn().mockResolvedValue([]),
}));
const createTask = vi.fn().mockResolvedValue("new-id");
const updateTask = vi.fn().mockResolvedValue(undefined);
const codexInfo = vi.fn();
vi.mock("../../ipc/tasks", () => ({
  createTask: (...a: unknown[]) => createTask(...a),
  updateTask: (...a: unknown[]) => updateTask(...a),
  codexInfo: () => codexInfo(),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn().mockResolvedValue([]) }));
// 橋接開著：選 Claude Code 時會出現橋接選項，選 Codex 時要藏起來（橋接只對 Claude 有意義）。
vi.mock("../../ipc/bridge", () => ({
  bridgeStatus: vi.fn().mockResolvedValue({ running: true, port: 8319, token: "t", error: null }),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskEditorDialog } from "./TaskEditorDialog";
import type { TaskWithAttachments } from "../../ipc/tasks";

const existing = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "t1", title: "既有卡片", body: "", project_dir: "/r", label: null,
  status: "planning", parallel_ok: true, interactive: false, sort_order: 1,
  outcome: null, tab_id: null, transcript_path: null, error_message: null,
  created_at: "2026-09-11", dispatched_at: null, finished_at: null,
  ai_summary: null, archived_at: null, session_id: null, session_path: null,
  use_bridge: false, bridge_tiers: null, worktree_path: null,
  worktree_branch: null, isolate_worktree: null, agent_id: null, start_sha: null, end_sha: null, card_kind: "work", module_id: null, review_of: null, verdict: null, review_reasons: null, module_override: false, is_spec: false, from_spec: null, attachments: [],
  ...over,
});

const mount = (card: TaskWithAttachments | null = null) =>
  render(
    <LocaleProvider>
      <TaskEditorDialog projectId="p1" card={card} onClose={vi.fn()} onSaved={vi.fn()} />
    </LocaleProvider>,
  );

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  codexInfo.mockResolvedValue({
    models: [{ slug: "gpt-6-luna", name: "GPT-6-Luna" }, { slug: "gpt-5.6-terra", name: "GPT-5.6-Terra" }],
    login: "Logged in using ChatGPT",
    error: null,
  });
});

describe("TaskEditorDialog 的 AI 工具", () => {
  it("預設 Claude Code，存出 cli: null；不查 Codex", async () => {
    mount();
    await userEvent.type(screen.getByTestId("task-title-input"), "標題");
    await userEvent.type(screen.getByTestId("task-dir-input"), "/r");
    await userEvent.click(screen.getByText("儲存"));
    await waitFor(() => expect(createTask).toHaveBeenCalled());
    expect(createTask.mock.calls[0][1]).toMatchObject({ cli: null, model: null });
    expect(codexInfo).not.toHaveBeenCalled();
  });

  it("選 Codex：列出模型、預選第一個、藏起橋接，存出 cli 與模型", async () => {
    mount();
    await screen.findByText(/^(派工方式|Dispatch via)$/);
    await userEvent.selectOptions(screen.getByTestId("task-cli-select"), "codex");
    const model = screen.getByLabelText(/^模型$|^Model$/) as HTMLInputElement;
    await waitFor(() => expect(model.value).toBe("gpt-6-luna"));
    expect(screen.getByText(/Logged in using ChatGPT/)).toBeTruthy();
    expect(screen.queryByText(/^(派工方式|Dispatch via)$/)).toBeNull();

    await userEvent.type(screen.getByTestId("task-title-input"), "標題");
    await userEvent.type(screen.getByTestId("task-dir-input"), "/r");
    await userEvent.clear(model);
    expect(screen.getByText("儲存").closest("button")).toBeDisabled();
    await userEvent.type(model, "gpt-5.6-terra");
    await userEvent.click(screen.getByText("儲存"));
    await waitFor(() => expect(createTask).toHaveBeenCalled());
    expect(createTask.mock.calls[0][1]).toMatchObject({ cli: "codex", model: "gpt-5.6-terra" });
  });

  it("編輯 Codex 卡片保留設定；改成互動卡片就回到 Claude Code", async () => {
    mount(existing({ cli: "codex", model: "gpt-5.6-terra" }));
    expect((screen.getByTestId("task-cli-select") as HTMLSelectElement).value).toBe("codex");
    expect((screen.getByLabelText(/^模型$|^Model$/) as HTMLInputElement).value).toBe("gpt-5.6-terra");
    await userEvent.click(screen.getByText(/互動模式|Interactive/));
    expect(screen.queryByTestId("task-cli-select")).toBeNull();
    await userEvent.click(screen.getByText("儲存"));
    await waitFor(() => expect(updateTask).toHaveBeenCalled());
    expect(updateTask.mock.calls[0][1]).toMatchObject({ interactive: true, cli: null, model: null });
  });
});
