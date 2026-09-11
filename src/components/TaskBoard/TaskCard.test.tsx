import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mergeTaskWorktree = vi.fn();
const abortMerge = vi.fn();
const archiveTask = vi.fn();
const messageDialog = vi.fn();
const confirmDialog = vi.fn();

vi.mock("../../ipc/tasks", () => ({
  mergeTaskWorktree: (...a: unknown[]) => mergeTaskWorktree(...a),
  abortMerge: (...a: unknown[]) => abortMerge(...a),
  archiveTask: (...a: unknown[]) => archiveTask(...a),
  cloneTask: vi.fn(),
  deleteTask: vi.fn(),
  markTaskDone: vi.fn(),
  stopTask: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: (...a: unknown[]) => confirmDialog(...a),
  message: (...a: unknown[]) => messageDialog(...a),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskCard } from "./TaskCard";
import type { TaskWithAttachments } from "../../ipc/tasks";

const card = (over: Partial<TaskWithAttachments> = {}): TaskWithAttachments => ({
  id: "t1",
  title: "查詢 BIC 代碼",
  body: "",
  project_dir: "C:\\work\\swift",
  label: null,
  status: "done",
  parallel_ok: false,
  interactive: false,
  sort_order: 0,
  outcome: "success",
  tab_id: null,
  transcript_path: null,
  error_message: null,
  created_at: "2026-09-11",
  dispatched_at: null,
  finished_at: null,
  ai_summary: null,
  archived_at: null,
  session_id: null,
  session_path: null,
  use_bridge: false,
  bridge_tiers: null,
  worktree_path: "C:\\store\\t1\\worktree",
  worktree_branch: "aiterm-task/t1",
  attachments: [],
  ...over,
});

function mount(onChanged = vi.fn()) {
  render(
    <LocaleProvider>
      <TaskCard
        projectId="p1"
        card={card()}
        onEdit={vi.fn()}
        onViewTranscript={vi.fn()}
        onEditLabel={vi.fn()}
        onChanged={onChanged}
      />
    </LocaleProvider>,
  );
  return onChanged;
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("TaskCard 的動作失敗時", () => {
  it("合併失敗要把後端的錯誤講出來，不能靜靜失敗", async () => {
    // 實機回報：Windows 上按「合併回原分支」完全沒反應。後端其實回了
    // git 的 stderr（見 vcs/git.rs 的 git()），但呼叫端是 `void run(...)`，
    // rejected promise 被整個丟掉，使用者只看到什麼都沒發生。
    mergeTaskWorktree.mockRejectedValue(
      "error: Your local changes to the following files would be overwritten by merge",
    );
    const onChanged = mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    expect(String(messageDialog.mock.calls[0][0])).toContain("would be overwritten by merge");
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("錯誤是物件時不能顯示成 [object Object]", async () => {
    // 這個 repo 踩過：Tauri invoke reject 丟回來的可能是 Rust 端序列化的
    // 物件，不是字串，String(e) 只會印 [object Object]。
    mergeTaskWorktree.mockRejectedValue({ kind: "merge_conflict", detail: "CONFLICT in app.jar" });
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    const shown = String(messageDialog.mock.calls[0][0]);
    expect(shown).not.toContain("[object Object]");
    expect(shown).toContain("CONFLICT in app.jar");
  });

  it("成功時不跳錯誤視窗，而且會通知重新整理", async () => {
    mergeTaskWorktree.mockResolvedValue({ status: "merged" });
    const onChanged = mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(messageDialog).not.toHaveBeenCalled();
  });

  it("衝突時問使用者，選「還原」才呼叫 abort", async () => {
    mergeTaskWorktree.mockResolvedValue({ status: "conflict", files: ["a.txt", "b.txt"] });
    confirmDialog.mockResolvedValue(true); // true = okLabel = 還原
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(abortMerge).toHaveBeenCalledWith("p1", "t1"));
    expect(String(confirmDialog.mock.calls[0][0])).toContain("a.txt");
    expect(String(confirmDialog.mock.calls[0][0])).toContain("b.txt");
  });

  it("衝突時選「我自己解」就不還原，保持半合併狀態", async () => {
    mergeTaskWorktree.mockResolvedValue({ status: "conflict", files: ["a.txt"] });
    confirmDialog.mockResolvedValue(false);
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(confirmDialog).toHaveBeenCalled());
    expect(abortMerge).not.toHaveBeenCalled();
  });

  it("原分支不乾淨時只提示，不會動到任何東西", async () => {
    mergeTaskWorktree.mockResolvedValue({
      status: "blocked",
      reason: "dirty_base",
      files: ["config.yml"],
    });
    const onChanged = mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    expect(String(messageDialog.mock.calls[0][0])).toContain("config.yml");
    expect(abortMerge).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("原目錄還停在上一次合併時，提示的是不同的訊息", async () => {
    mergeTaskWorktree.mockResolvedValue({
      status: "blocked",
      reason: "merge_in_progress",
      files: ["a.txt"],
    });
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    expect(String(messageDialog.mock.calls[0][0])).toContain("上一次沒有收尾的合併");
  });

  it("合併期間按鈕顯示進度文字", async () => {
    // worktree 大的時候 git status/add/worktree remove 動輒數十秒，按鈕只是
    // 變灰看起來就像沒反應——這正是使用者實機回報的觀感。
    let release: (v: unknown) => void = () => {};
    mergeTaskWorktree.mockReturnValue(new Promise((r) => { release = r; }));
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));
    await screen.findByText("合併中…");

    release({ status: "merged" });
    await waitFor(() => expect(screen.queryByText("合併中…")).toBeNull());
  });

  it("同一個 run() 包住的其他動作也一樣會顯示錯誤", async () => {
    // run() 是共用的，封存失敗過去一樣是靜靜失敗。
    archiveTask.mockRejectedValue("db is locked");
    mount();

    await userEvent.click(screen.getByText("封存"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    expect(String(messageDialog.mock.calls[0][0])).toContain("db is locked");
  });
});
