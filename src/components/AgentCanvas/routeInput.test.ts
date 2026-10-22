import { describe, expect, it, vi } from "vitest";

import type { AgentRow } from "../../ipc/agents";
import type { TaskWithAttachments } from "../../ipc/tasks";
import { translations } from "../../lib/i18n";
import { buildRouteSystem, buildStatusSummary, dispatchInput, NoStationError, parseRoute, routeInput, type DispatchEnv } from "./routeInput";
import type { StartDeps } from "./startFromRequirement";

const t = translations.en;

const agent = (over: Partial<AgentRow> & { id: string }): AgentRow =>
  ({ kind: "coder", name: over.id, role: "", prompt_prefix: "", project_dir: "/r", canvas_x: 0, canvas_y: 0, worktree_path: null, branch: null, paused_task_id: null, created_at: "", last_merge_signature: null, produces_spec: false, ...over }) as AgentRow;

const task = (over: Partial<TaskWithAttachments> & { id: string }): TaskWithAttachments =>
  ({ title: over.id, status: "queued", agent_id: null, card_kind: "work", outcome: null, archived_at: null, attachments: [], ...over }) as unknown as TaskWithAttachments;

function deps(over: Partial<StartDeps> = {}): StartDeps {
  return {
    getBrief: vi.fn().mockResolvedValue("番茄鐘 App"),
    setBrief: vi.fn().mockResolvedValue(undefined),
    getReferences: vi.fn().mockResolvedValue([]),
    complete: vi.fn().mockRejectedValue(new Error("no ai")),
    createAgent: vi.fn(),
    createTask: vi.fn().mockResolvedValue("task-1"),
    assignAgent: vi.fn().mockResolvedValue(undefined),
    ...over,
  };
}

const spec = agent({ id: "spec", produces_spec: true });
const c1 = agent({ id: "c1" });
const c2 = agent({ id: "c2" });
const env = (agents: AgentRow[], tasks: TaskWithAttachments[] = [], over: Partial<DispatchEnv> = {}): DispatchEnv => ({
  projectId: "p",
  repoDir: "/repo",
  agents,
  tasks,
  t,
  providerId: "prov",
  ...over,
});

describe("parseRoute", () => {
  it("接受四種分類，容忍程式碼區塊", () => {
    expect(parseRoute('```json\n{"kind":"fast","reason":"改文字"}\n```')).toEqual({ kind: "fast", reason: "改文字" });
    for (const kind of ["requirement", "bug", "question"]) expect(parseRoute(`{"kind":"${kind}"}`)?.kind).toBe(kind);
  });
  it("不認得的分類或壞掉的回覆回 null", () => {
    expect(parseRoute('{"kind":"delete_everything"}')).toBeNull();
    expect(parseRoute("隨便")).toBeNull();
    expect(parseRoute("{壞")).toBeNull();
  });
});

describe("routeInput", () => {
  it("沒有 AI、AI 失敗或回得不對時退回新需求，而且標記為預設", async () => {
    const none = await routeInput(deps(), "p", "按鈕改紅色", undefined);
    expect(none).toEqual({ decision: { kind: "requirement", reason: "" }, usedDefault: true });
    expect((await routeInput(deps(), "p", "x", "prov")).decision.kind).toBe("requirement");
    const junk = await routeInput(deps({ complete: vi.fn().mockResolvedValue("不是 JSON") }), "p", "x", "prov");
    expect(junk.usedDefault).toBe(true);
  });
  it("用 AI 的分類，並把簡介與輸入都餵給它", async () => {
    const complete = vi.fn().mockResolvedValue('{"kind":"bug","reason":"回報錯誤"}');
    const r = await routeInput(deps({ complete }), "p", "按開始沒反應", "prov");
    expect(r).toEqual({ decision: { kind: "bug", reason: "回報錯誤" }, usedDefault: false });
    expect(complete.mock.calls[0][0][0].content).toContain("番茄鐘 App");
    expect(complete.mock.calls[0][0][0].content).toContain("按開始沒反應");
  });
  it("系統提示要求拿不準時選完整流程", () => {
    expect(buildRouteSystem()).toContain("拿不準 fast 還是 requirement 時選 requirement");
  });
});

describe("dispatchInput", () => {
  it("requirement：建卡附完成定義，派給規格站", async () => {
    const complete = vi.fn().mockResolvedValue('{"criteria":["按開始會倒數"]}');
    const d = deps({ complete });
    const r = await dispatchInput("requirement", "做計時器", env([c1, spec]), d);
    expect(r).toMatchObject({ kind: "requirement", taskId: "task-1", agentName: "spec" });
    expect(d.assignAgent).toHaveBeenCalledWith("p", "task-1", "spec");
    expect(vi.mocked(d.createTask).mock.calls[0][1].body).toContain("- 按開始會倒數");
  });

  it("fast：派給最不忙的一般工程師，並帶 fast 旗標，不派給規格站", async () => {
    const busy = [task({ id: "t1", agent_id: "c1", status: "running" }), task({ id: "t2", agent_id: "c1", status: "queued" })];
    const d = deps();
    const r = await dispatchInput("fast", "按鈕改紅色", env([spec, c1, c2], busy), d);
    expect(d.assignAgent).toHaveBeenCalledWith("p", "task-1", "c2", true);
    expect(r).toMatchObject({ kind: "fast", agentName: "c2" });
  });

  it("fast：只有規格站時退回完整流程，並告知", async () => {
    const d = deps();
    const r = await dispatchInput("fast", "按鈕改紅色", env([spec]), d);
    expect(r).toMatchObject({ kind: "requirement", fellBack: true, agentName: "spec" });
    expect(d.assignAgent).toHaveBeenCalledWith("p", "task-1", "spec");
  });

  it("bug：派給工程師、走一般流程（不帶 fast），內文附修正說明", async () => {
    const d = deps();
    await dispatchInput("bug", "按開始沒反應", env([spec, c1]), d);
    expect(d.assignAgent).toHaveBeenCalledWith("p", "task-1", "c1", false);
    expect(vi.mocked(d.createTask).mock.calls[0][1].body).toContain(t.ac_bug_instructions_noshot);
  });

  it("question：不建卡，用簡介與進度回答", async () => {
    const complete = vi.fn().mockResolvedValue("  計時器已完成。 ");
    const d = deps({ complete });
    const tasks = [task({ id: "t1", title: "計時器", agent_id: "c1", status: "done", outcome: "success" })];
    const r = await dispatchInput("question", "計時器做完了嗎？", env([c1], tasks), d);
    expect(r).toEqual({ kind: "question", answer: "計時器已完成。" });
    expect(d.createTask).not.toHaveBeenCalled();
    const sent = complete.mock.calls[0][0][0].content as string;
    expect(sent).toContain("番茄鐘 App");
    expect(sent).toContain("計時器：完成");
    expect(sent).toContain("計時器做完了嗎？");
  });

  it("沒有工程師時 requirement 丟 NoStationError，不建卡", async () => {
    const d = deps();
    await expect(dispatchInput("requirement", "x", env([]), d)).rejects.toBeInstanceOf(NoStationError);
    expect(d.createTask).not.toHaveBeenCalled();
  });
});

describe("buildStatusSummary", () => {
  it("列出每位 Agent、暫停狀態與他的模組結果，略過封存與非模組卡", () => {
    const s = buildStatusSummary(
      [agent({ id: "c1", name: "工程師1", paused_task_id: "x" })],
      [
        task({ id: "a", title: "登入", agent_id: "c1", status: "done", outcome: "failed" }),
        task({ id: "b", title: "舊的", agent_id: "c1", archived_at: 5 }),
        task({ id: "r", title: "審查", agent_id: "c1", card_kind: "review" }),
      ],
    );
    expect(s).toContain("工程師1（coder）（暫停中）");
    expect(s).toContain("登入：失敗");
    expect(s).not.toContain("舊的");
    expect(s).not.toContain("審查");
  });
});
