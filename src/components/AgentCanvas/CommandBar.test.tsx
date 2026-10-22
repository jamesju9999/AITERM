import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import type { TaskWithAttachments } from "../../ipc/tasks";

vi.mock("../../ipc/ai", () => ({ invokeAiComplete: vi.fn() }));
vi.mock("../../ipc/agents", () => ({ askAgent: vi.fn(), assignAgent: vi.fn(), createAgent: vi.fn(), getReferences: vi.fn() }));
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

const attach = vi.fn().mockResolvedValue(undefined);
const ask = vi.fn().mockResolvedValue({ task_id: "ask-1", agent_name: "c1" });
const bar = (d: StartDeps, onDispatched = vi.fn(), agents = [agent("c1")], tasks: TaskWithAttachments[] = []) => (
  <LocaleProvider>
    <CommandBar projectId="p" repo="/repo" agents={agents} tasks={tasks} onDispatched={onDispatched} deps={d} attach={attach} ask={ask} />
  </LocaleProvider>
);
const view = (d: StartDeps, onDispatched = vi.fn(), agents = [agent("c1")], tasks: TaskWithAttachments[] = []) => render(bar(d, onDispatched, agents, tasks));

const png = (name = "image.png") => new File([new Uint8Array([137, 80, 78, 71])], name, { type: "image/png" });
const paste = (files: File[]) => fireEvent.paste(screen.getByTestId("ac-cmd-input"), { clipboardData: { files } });

const type = (text: string) => {
  const input = screen.getByTestId("ac-cmd-input");
  fireEvent.change(input, { target: { value: text } });
  fireEvent.keyDown(input, { key: "Enter" });
};

beforeEach(() => {
  attach.mockClear();
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

  it("確認前可以改類型：改成問問題就交給 Agent 讀程式庫，卡片完成後才顯示它寫的答案", async () => {
    const d = deps(ai('{"kind":"fast","reason":"x"}'));
    const onDispatched = vi.fn();
    const view1 = view(d, onDispatched);
    type("計時器做完了嗎？");
    await screen.findByTestId("ac-cmd-plan");
    fireEvent.click(screen.getByTestId("ac-cmd-kind-question"));
    fireEvent.click(screen.getByTestId("ac-cmd-confirm"));
    expect((await screen.findByTestId("ac-cmd-asking")).textContent).toContain("c1");
    expect(ask).toHaveBeenCalledWith("p", "計時器做完了嗎？");
    expect(d.createTask).not.toHaveBeenCalled();
    expect(d.assignAgent).not.toHaveBeenCalled();
    expect(screen.queryByTestId("ac-cmd-answer")).toBeNull(); // 還沒完成：不能先用別的東西回答

    const running = { id: "ask-1", status: "running", outcome: null, ai_summary: null, error_message: null } as unknown as TaskWithAttachments;
    view1.rerender(bar(d, onDispatched, [agent("c1")], [running]));
    expect(screen.getByTestId("ac-cmd-asking")).toBeTruthy(); // 還在跑：繼續等，不當成失敗
    expect(screen.queryByTestId("ac-cmd-error")).toBeNull();

    const card = { id: "ask-1", status: "done", outcome: "success", ai_summary: "  原始碼裡接的是 RSS。 ", error_message: null } as unknown as TaskWithAttachments;
    view1.rerender(bar(d, onDispatched, [agent("c1")], [card]));
    expect((await screen.findByTestId("ac-cmd-answer")).textContent).toContain("原始碼裡接的是 RSS。");
  });

  it("問答卡失敗：顯示原因，不會一直等", async () => {
    const d = deps(ai('{"kind":"question","reason":""}'));
    const onDispatched = vi.fn();
    const v = view(d, onDispatched);
    type("這是什麼？");
    fireEvent.click(await screen.findByTestId("ac-cmd-confirm"));
    await screen.findByTestId("ac-cmd-asking");
    const card = { id: "ask-1", status: "done", outcome: "failed", ai_summary: null, error_message: "Agent 沒有產出 answer.md" } as unknown as TaskWithAttachments;
    v.rerender(bar(d, onDispatched, [agent("c1")], [card]));
    expect((await screen.findByTestId("ac-cmd-error")).textContent).toContain("answer.md");
    expect(screen.queryByTestId("ac-cmd-asking")).toBeNull();
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

  it("貼上截圖會出現縮圖，可以移除；不貼圖照樣能送", async () => {
    const d = deps(ai('{"kind":"bug","reason":""}'));
    view(d);
    paste([png(), png()]);
    const shots = await screen.findByTestId("ac-cmd-shots");
    expect(shots.querySelectorAll(".ac-bug-shot")).toHaveLength(2);
    // 貼上的截圖都叫 image.png：要改名，附件才不會撞名。
    const names = [...shots.querySelectorAll(".ac-bug-shot")].map((n) => n.getAttribute("title"));
    expect(new Set(names).size).toBe(2);
    fireEvent.click(screen.getAllByRole("button", { name: /移除|Remove/ })[0]);
    expect(shots.querySelectorAll(".ac-bug-shot")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: /移除|Remove/ }));
    expect(screen.queryByTestId("ac-cmd-shots")).toBeNull();

    type("按開始沒反應");
    await screen.findByTestId("ac-cmd-plan");
    fireEvent.click(screen.getByTestId("ac-cmd-confirm"));
    await waitFor(() => expect(d.assignAgent).toHaveBeenCalled());
    expect(attach).not.toHaveBeenCalled();
    expect(vi.mocked(d.createTask).mock.calls[0][1].body).toContain("不必找截圖");
  });

  it("附圖：圖片存成卡片附件，在派工之前；內文要求先看截圖，建完圖就清掉", async () => {
    const d = deps(ai('{"kind":"bug","reason":""}'));
    const order: string[] = [];
    attach.mockImplementation(async () => void order.push("attach"));
    vi.mocked(d.assignAgent).mockImplementation(async () => void order.push("assign"));
    view(d);
    paste([png()]);
    type("版面跑掉了");
    await screen.findByTestId("ac-cmd-plan");
    fireEvent.click(screen.getByTestId("ac-cmd-confirm"));
    await waitFor(() => expect(d.assignAgent).toHaveBeenCalled());
    expect(order).toEqual(["attach", "assign"]);
    expect(attach).toHaveBeenCalledWith("p", "task-1", expect.stringMatching(/^screenshot-\d+\.png$/), expect.any(Uint8Array));
    expect(vi.mocked(d.createTask).mock.calls[0][1].body).toContain("附件的截圖");
    await waitFor(() => expect(screen.queryByTestId("ac-cmd-shots")).toBeNull());
  });

  it("只貼圖、沒打字也能送（圖就是需求）", async () => {
    const d = deps(ai('{"kind":"requirement","reason":""}'));
    view(d);
    const send = screen.getByTestId("ac-cmd-send") as HTMLButtonElement;
    expect(send.disabled).toBe(true);
    paste([png()]);
    await waitFor(() => expect(send.disabled).toBe(false));
    fireEvent.click(send);
    await screen.findByTestId("ac-cmd-plan");
    fireEvent.click(screen.getByTestId("ac-cmd-confirm"));
    await waitFor(() => expect(d.createTask).toHaveBeenCalled());
    expect(vi.mocked(d.createTask).mock.calls[0][1].title).toMatch(/請看附圖|See the attached image/);
    expect(attach).toHaveBeenCalledTimes(1);
  });

  it("問問題不看圖：有附圖時不能確認，並說明要改類型或移除圖片", async () => {
    const d = deps(ai('{"kind":"question","reason":""}', "答案"));
    view(d);
    paste([png()]);
    type("這是什麼？");
    await screen.findByTestId("ac-cmd-plan");
    expect(screen.getByTestId("ac-cmd-question-no-image")).toBeTruthy();
    expect((screen.getByTestId("ac-cmd-confirm") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByTestId("ac-cmd-kind-bug"));
    expect((screen.getByTestId("ac-cmd-confirm") as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByTestId("ac-cmd-question-no-image")).toBeNull();
  });
});
