import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import type { TaskWithAttachments } from "../../ipc/tasks";

vi.mock("../../ipc/ai", () => ({ invokeAiComplete: vi.fn() }));
vi.mock("../../ipc/agents", () => ({ assignAgent: vi.fn(), createAgent: vi.fn(), getReferences: vi.fn() }));
vi.mock("../../ipc/tasks", () => ({ createTask: vi.fn(), getProjectBrief: vi.fn(), setProjectBrief: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn() }));
vi.mock("../ModelPickerButton", () => ({ ModelPickerButton: () => null }));

import { listProviders } from "../../ipc/provider";
import { CommandBar } from "./CommandBar";
import type { StartDeps } from "./startFromRequirement";

const agent = (id: string, over: Partial<AgentRow> = {}): AgentRow =>
  ({ id, kind: "coder", name: id, role: "", prompt_prefix: "", project_dir: "/r", canvas_x: 0, canvas_y: 0, worktree_path: null, branch: null, paused_task_id: null, created_at: "", last_merge_signature: null, produces_spec: false, ...over }) as AgentRow;

function deps(complete: StartDeps["complete"]): StartDeps {
  return {
    getBrief: vi.fn().mockResolvedValue("番茄鐘 App"),
    setBrief: vi.fn().mockResolvedValue(undefined),
    getReferences: vi.fn().mockResolvedValue([]),
    complete,
    createAgent: vi.fn(),
    createTask: vi.fn().mockResolvedValue("task-1"),
    assignAgent: vi.fn().mockResolvedValue(undefined),
  };
}

/** 分類回 `kind`；完成定義與問答另外給。 */
const ai = (route: string, other = "") =>
  vi.fn().mockImplementation(async (_m, _s, streamId: string) => (streamId === "input-route" ? route : other));

const view = (d: StartDeps, onDispatched = vi.fn(), agents = [agent("c1")], tasks: TaskWithAttachments[] = []) =>
  render(
    <LocaleProvider>
      <CommandBar projectId="p" repo="/repo" agents={agents} tasks={tasks} onDispatched={onDispatched} deps={d} />
    </LocaleProvider>,
  );

const type = (text: string) => {
  const input = screen.getByTestId("ac-cmd-input");
  fireEvent.change(input, { target: { value: text } });
  fireEvent.keyDown(input, { key: "Enter" });
};

beforeEach(() => {
  vi.mocked(listProviders).mockResolvedValue([{ id: "pv", name: "P", provider_type: "openai", model: "m", is_default: true } as never]);
});

describe("CommandBar", () => {
  it("Enter 先分類，顯示打算怎麼處理；確認後小修改以快速路徑派給工程師", async () => {
    const d = deps(ai('{"kind":"fast","reason":"只改一個顏色"}'));
    const onDispatched = vi.fn();
    view(d, onDispatched);
    type("按鈕改紅色");
    const plan = await screen.findByTestId("ac-cmd-plan");
    expect(plan.textContent).toContain("只改一個顏色");
    expect(screen.getByTestId("ac-cmd-kind-fast").getAttribute("aria-pressed")).toBe("true");
    expect(d.createTask).not.toHaveBeenCalled(); // 確認前什麼都不做
    fireEvent.click(screen.getByTestId("ac-cmd-confirm"));
    await waitFor(() => expect(d.assignAgent).toHaveBeenCalledWith("p", "task-1", "c1", true));
    expect(await screen.findByTestId("ac-cmd-notice")).toBeTruthy();
    expect((screen.getByTestId("ac-cmd-input") as HTMLTextAreaElement).value).toBe("");
    expect(onDispatched).toHaveBeenCalledWith(expect.objectContaining({ kind: "fast", taskId: "task-1" }));
  });

  it("確認前可以改類型：改成問問題就不建卡，直接顯示回答", async () => {
    const d = deps(ai('{"kind":"fast","reason":"x"}', "計時器已完成。"));
    view(d);
    type("計時器做完了嗎？");
    await screen.findByTestId("ac-cmd-plan");
    fireEvent.click(screen.getByTestId("ac-cmd-kind-question"));
    fireEvent.click(screen.getByTestId("ac-cmd-confirm"));
    expect((await screen.findByTestId("ac-cmd-answer")).textContent).toContain("計時器已完成。");
    expect(d.createTask).not.toHaveBeenCalled();
    expect(d.assignAgent).not.toHaveBeenCalled();
  });

  it("沒有 AI 時當成新需求並說明，仍然可以改類型", async () => {
    vi.mocked(listProviders).mockResolvedValue([]);
    const d = deps(ai("不會被呼叫"));
    view(d);
    type("做計時器");
    const plan = await screen.findByTestId("ac-cmd-plan");
    expect(screen.getByTestId("ac-cmd-kind-requirement").getAttribute("aria-pressed")).toBe("true");
    expect(plan.textContent).toMatch(/沒有 AI 判斷|No AI to decide/);
    expect(d.complete).not.toHaveBeenCalled();
  });

  it("取消不會建卡，輸入的文字還在", async () => {
    const d = deps(ai('{"kind":"requirement","reason":"新功能"}'));
    view(d);
    type("做計時器");
    await screen.findByTestId("ac-cmd-plan");
    fireEvent.click(screen.getByRole("button", { name: /取消|Cancel/ }));
    expect(screen.queryByTestId("ac-cmd-plan")).toBeNull();
    expect(d.createTask).not.toHaveBeenCalled();
    expect((screen.getByTestId("ac-cmd-input") as HTMLTextAreaElement).value).toBe("做計時器");
  });

  it("輸入法組字中的 Enter 不送出；Shift+Enter 是換行", () => {
    const d = deps(ai('{"kind":"bug","reason":""}'));
    view(d);
    const input = screen.getByTestId("ac-cmd-input");
    fireEvent.change(input, { target: { value: "你好" } });
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(d.complete).not.toHaveBeenCalled();
    expect(screen.queryByTestId("ac-cmd-plan")).toBeNull();
  });

  it("派工失敗顯示錯誤、保留文字，可以再試", async () => {
    const d = deps(ai('{"kind":"bug","reason":""}'));
    vi.mocked(d.assignAgent).mockRejectedValue(new Error("boom"));
    view(d);
    type("按開始沒反應");
    await screen.findByTestId("ac-cmd-plan");
    fireEvent.click(screen.getByTestId("ac-cmd-confirm"));
    expect((await screen.findByTestId("ac-cmd-error")).textContent).toContain("boom");
    expect((screen.getByTestId("ac-cmd-input") as HTMLTextAreaElement).value).toBe("按開始沒反應");
  });
});
