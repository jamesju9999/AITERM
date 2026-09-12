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

// 收集實際註冊的監聽器，讓測試能模擬後端送事件。
const listeners = new Map<string, (e: { payload: unknown }) => void>();
const unlistenSpy = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(name, cb);
    return Promise.resolve(unlistenSpy);
  },
}));

function emitProgress(payload: { task_id: string; step: string }) {
  listeners.get("task-merge-progress")?.({ payload });
}

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
  isolate_worktree: null,
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
  listeners.clear();
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

  it("合併前先把這張卡還開著的分頁從畫面上移除", async () => {
    // 後端會在清理 worktree 之前關掉這張卡的 PTY（Windows 上不關就刪不掉那個
    // 目錄）。分頁若留在畫面上，就變成一個連線已死的空分頁。
    const closed: unknown[] = [];
    const onClose = (e: Event) => closed.push((e as CustomEvent).detail);
    window.addEventListener("aiterm:close-tab", onClose);
    mergeTaskWorktree.mockResolvedValue({ status: "merged" });

    render(
      <LocaleProvider>
        <TaskCard
          projectId="p1"
          card={card({ tab_id: "tab-9" })}
          onEdit={vi.fn()}
          onViewTranscript={vi.fn()}
          onEditLabel={vi.fn()}
          onChanged={vi.fn()}
        />
      </LocaleProvider>,
    );
    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(closed).toEqual([{ tabId: "tab-9", skipGuard: true }]));
    window.removeEventListener("aiterm:close-tab", onClose);
  });

  it("沒有分頁的卡片不會送出關閉事件", async () => {
    const closed: unknown[] = [];
    const onClose = (e: Event) => closed.push((e as CustomEvent).detail);
    window.addEventListener("aiterm:close-tab", onClose);
    mergeTaskWorktree.mockResolvedValue({ status: "merged" });
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(mergeTaskWorktree).toHaveBeenCalled());
    expect(closed).toEqual([]);
    window.removeEventListener("aiterm:close-tab", onClose);
  });

  it("合併成功但 worktree 清不掉時，要講清楚合併已完成、不是失敗", async () => {
    // 實機踩過：合併其實成功了，但最後的 git worktree remove 因為還有行程的
    // 工作目錄在裡面而失敗，畫面上卻只丟出一個看起來像「整個失敗」的錯誤。
    mergeTaskWorktree.mockResolvedValue({
      status: "merged_but_not_cleaned",
      path: "C:\\proj\\tasks\\abc\\worktree",
      detail: "error: failed to delete ...: Directory not empty",
    });
    const onChanged = mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    const shown = String(messageDialog.mock.calls[0][0]);
    expect(shown).toContain("已經成功合併");
    expect(shown).toContain("C:\\proj\\tasks\\abc\\worktree");
    expect(shown).toContain("Directory not empty");
    // 後端已經清掉 DB 欄位，前端一定要重新整理，否則按鈕還留在畫面上，
    // 再按一次就會跑在半刪除的 worktree 上。
    expect(onChanged).toHaveBeenCalled();
  });

  it("清不掉時用的是警告而不是錯誤——合併確實成功了", async () => {
    mergeTaskWorktree.mockResolvedValue({
      status: "merged_but_not_cleaned",
      path: "/p/worktree",
      detail: "d",
    });
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    expect(messageDialog.mock.calls[0][1]).toMatchObject({ kind: "warning" });
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

  it("進度監聽器在按下按鈕之前就已經掛好", async () => {
    // 這個 repo 踩過兩次同一個 race：事件在前端訂閱之前送出就永遠遺失。
    // listen() 是非同步的，若等按下去才訂閱，第一個步驟事件（checking）
    // 幾乎一定跑在前面。掛載當下就訂閱才沒有這個問題。
    mount();
    expect(listeners.has("task-merge-progress")).toBe(true);
  });

  it("依後端回報的步驟顯示不同文字", async () => {
    let release: (v: unknown) => void = () => {};
    mergeTaskWorktree.mockReturnValue(new Promise((r) => { release = r; }));
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    emitProgress({ task_id: "t1", step: "checking" });
    await screen.findByText("合併中：檢查原分支…");

    emitProgress({ task_id: "t1", step: "cleaning" });
    await screen.findByText("合併中：移除 worktree…");

    release({ status: "merged" });
    await waitFor(() => expect(screen.queryByText(/合併中/)).toBeNull());
  });

  it("別張卡片的進度事件不會影響這張卡", async () => {
    let release: (v: unknown) => void = () => {};
    mergeTaskWorktree.mockReturnValue(new Promise((r) => { release = r; }));
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));
    await screen.findByText("合併中…");

    emitProgress({ task_id: "另一張卡", step: "cleaning" });

    // 仍然是沒有步驟的通用文字，不該被別張卡的事件改掉。
    expect(screen.getByText("合併中…")).toBeTruthy();
    release({ status: "merged" });
  });

  it("認不得的步驟名退回通用文字，不顯示空白按鈕", async () => {
    let release: (v: unknown) => void = () => {};
    mergeTaskWorktree.mockReturnValue(new Promise((r) => { release = r; }));
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));
    emitProgress({ task_id: "t1", step: "後端之後新增的步驟" });

    expect(screen.getByText("合併中…")).toBeTruthy();
    release({ status: "merged" });
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
