import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue("/repo"),
  confirm: vi.fn().mockResolvedValue(true),
  message: vi.fn().mockResolvedValue(undefined),
  save: vi.fn().mockResolvedValue("/Users/me/驗收報告.html"),
}));
vi.mock("../../ipc/fs", () => ({ writeTextFile: vi.fn().mockResolvedValue(undefined) }));

// 抓住 tasks-updated 的回呼，測試裡手動觸發「後端說資料變了」。
let tasksUpdated: (() => void) | null = null;
vi.mock("../../ipc/tasks", () => ({
  listTasks: vi.fn(),
  listTokenUsage: vi.fn().mockResolvedValue({}),
  getTokenBudget: vi.fn().mockResolvedValue(null),
  setTokenBudget: vi.fn().mockResolvedValue(undefined),
  getTaskBoardConfig: vi.fn().mockResolvedValue({ max_concurrent: 3 }),
  onTasksUpdated: vi.fn((cb: () => void) => {
    tasksUpdated = cb;
    return Promise.resolve(() => {});
  }),
  readTranscript: vi.fn().mockResolvedValue(""),
  markTaskDone: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../ipc/agents", () => ({
  listAgents: vi.fn(),
  createAgent: vi.fn().mockResolvedValue("new-agent"),
  updateAgent: vi.fn().mockResolvedValue(undefined),
  deleteAgent: vi.fn().mockResolvedValue(undefined),
  resolveAgent: vi.fn().mockResolvedValue(undefined),
  mergeAgent: vi.fn().mockResolvedValue({ status: "merged" }),
  abortAgentMerge: vi.fn().mockResolvedValue(undefined),
  assignAgent: vi.fn().mockResolvedValue(undefined),
  overrideModule: vi.fn().mockResolvedValue(undefined),
  checkRepo: vi.fn().mockResolvedValue({ status: "ok", branch: "main" }),
  initRepo: vi.fn().mockResolvedValue(undefined),
  openRepo: vi.fn().mockResolvedValue(undefined),
  verifyReport: vi.fn().mockResolvedValue({ report: null, images: {} }),
  acceptVerification: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../ipc/projects", () => ({
  listProjects: vi.fn().mockResolvedValue([
    { id: "p1", name: "HTBKTW", description: "", path: "/p", status: "ok", counts: { planning: 0, queued: 0, running: 0, done: 0 }, error: null },
  ]),
  usedDirs: vi.fn().mockResolvedValue(["/repo/htbktw"]),
  usedLabels: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn().mockResolvedValue([]) }));
vi.mock("../../ipc/questions", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/questions")>("../../ipc/questions");
  return {
    ...actual,
    listQuestions: vi.fn().mockResolvedValue([]),
    answerQuestion: vi.fn().mockResolvedValue(undefined),
    onQuestionsUpdated: vi.fn().mockResolvedValue(() => {}),
  };
});
vi.mock("../../ipc/bridge", () => ({
  bridgeStatus: vi.fn().mockResolvedValue({ running: false, port: null, token: null, error: null }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(vi.fn()) }));

import {
  acceptVerification,
  assignAgent,
  checkRepo,
  createAgent,
  initRepo,
  listAgents,
  mergeAgent,
  openRepo,
  overrideModule,
  resolveAgent,
  updateAgent,
  verifyReport,
  type AgentRow,
} from "../../ipc/agents";
import { listTasks, listTokenUsage, markTaskDone, type TaskWithAttachments } from "../../ipc/tasks";
import { answerQuestion, listQuestions, type QuestionView } from "../../ipc/questions";
import { writeTextFile } from "../../ipc/fs";
import { listProjects } from "../../ipc/projects";
import { AgentCanvasView } from "./index";

const agent = (over: Partial<AgentRow> = {}): AgentRow => ({
  id: "a1", kind: "coder", name: "Coder Agent1", role: "Senior Coder", prompt_prefix: "p",
  project_dir: "/repo/htbktw", canvas_x: 40, canvas_y: 40, worktree_path: null, branch: null,
  paused_task_id: null, created_at: "", last_merge_signature: null, produces_spec: false, ...over,
});

const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "t", title: "t", body: "", project_dir: "/r", label: null, status: "planning", parallel_ok: true,
  interactive: false, sort_order: 0, outcome: null, tab_id: null, transcript_path: null, error_message: null,
  created_at: "", dispatched_at: null, finished_at: null, ai_summary: null, archived_at: null, session_id: null,
  session_path: null, use_bridge: false, bridge_tiers: null, worktree_path: null, worktree_branch: null,
  isolate_worktree: null, agent_id: null, start_sha: null, end_sha: null, card_kind: "work", module_id: null, review_of: null, verdict: null, review_reasons: null, module_override: false, is_spec: false, from_spec: null, attachments: [], ...over,
});

const view = () =>
  render(
    <LocaleProvider>
      <AgentCanvasView isActive />
    </LocaleProvider>,
  );

/** jsdom 沒有版面與命中測試，拖曳放開點底下是誰由這裡決定。 */
function drag(from: Element, dropOn: Element) {
  const original = document.elementFromPoint;
  document.elementFromPoint = vi.fn().mockReturnValue(dropOn);
  try {
    fireEvent.mouseDown(from, { clientX: 100, clientY: 100, button: 0 });
    fireEvent.mouseMove(window, { clientX: 140, clientY: 130 });
    fireEvent.mouseUp(window, { clientX: 140, clientY: 130 });
  } finally {
    document.elementFromPoint = original;
  }
}

/** 等程式庫檢查通過（按鈕才會啟用）再按「放上桌布」。 */
async function placeOnCanvas(dlg: HTMLElement) {
  await within(dlg).findByText(/^✓/);
  fireEvent.click(within(dlg).getByText(/放上桌布|Place on canvas/));
}

beforeEach(() => {
  vi.clearAllMocks();
  tasksUpdated = null;
  try {
    localStorage.clear();
  } catch {
    // ignore
  }
});

describe("AgentCanvasView", () => {
  it("顯示 Agent、指派給它的卡片，與模組庫裡未指派的計畫中卡片", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A 登入", status: "queued", agent_id: "a1" }),
      card({ id: "C", title: "C 訂單", status: "planning" }),
      card({ id: "Q", title: "不屬於 Agent 的待執行卡", status: "queued" }),
    ]);
    view();
    const node = await screen.findByTestId("agent-a1");
    expect(within(node).getByText(/A 登入/)).toBeTruthy();
    expect(screen.getByTestId("module-C")).toBeTruthy();
    expect(screen.queryByTestId("module-Q")).toBeNull();
  });

  it("Agent 卡、模組 chip 與頂列顯示 token 用量（審查卡算在模組上）", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent(), agent({ id: "r1", kind: "reviewer", name: "審核者" })]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A 登入", status: "done", outcome: "success", agent_id: "a1", dispatched_at: 1 }),
      card({ id: "R", title: "審查 A", status: "done", card_kind: "review", module_id: "A", agent_id: "r1", dispatched_at: 2 }),
      card({ id: "B", title: "B 沒記錄", status: "done", agent_id: "a1", dispatched_at: 3 }),
    ]);
    const u = (n: number) => ({ input: n, cache_write: 0, cache_read: 0, output: 0, calls: 1 });
    vi.mocked(listTokenUsage).mockResolvedValue({ A: u(1_000_000), R: u(250_000) });
    view();
    await waitFor(() => expect(screen.getByTestId("tokens-a1").textContent).toBe("Σ 1.00M token"));
    expect(screen.getByTestId("tokens-a1").getAttribute("title")).toMatch(/另有 1 張卡沒有對話記錄/);
    expect(screen.getByTestId("tokens-r1").textContent).toBe("Σ 250.0K token");
    expect(screen.getByTestId("chip-A").getAttribute("title")).toMatch(/Σ 1.25M token$/);
    expect(screen.getByTestId("ac-tokens").textContent).toBe("Σ 1.25M token");
  });

  describe("驗收者", () => {
    const orch = agent({ id: "o1", kind: "orchestrator", name: "協調者", branch: "aiterm-integration" });
    const ver = agent({ id: "v1", kind: "verifier", name: "驗收者" });
    const modA = card({ id: "A", title: "A 登入", status: "done", outcome: "success", agent_id: "a1" });
    const merged = card({ id: "MG", title: "合併", status: "done", outcome: "success", card_kind: "merge", agent_id: "o1", finished_at: 10 });
    const verify = (over: Partial<TaskWithAttachments>) =>
      card({ id: "V", title: "驗收：合併結果", status: "done", outcome: "success", card_kind: "verify", agent_id: "v1", review_of: "MG", finished_at: 20, ...over });

    it("Agent 池有驗收者，已放就不能再放", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent(), ver]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      const item = await screen.findByTestId("pool-verifier");
      await waitFor(() => expect(item.getAttribute("aria-disabled")).toBe("true"));
    });

    it("合併完成、驗收中：頂列寫驗收中，協調者不放「合併分支」", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent(), orch, ver]);
      vi.mocked(listTasks).mockResolvedValue([modA, merged, verify({ status: "running", outcome: null, finished_at: null })]);
      view();
      await waitFor(() => expect(screen.getByTestId("ac-summary").textContent).toMatch(/驗收中/));
      expect(within(screen.getByTestId("agent-o1")).queryByText(/合併分支/)).toBeNull();
      expect(within(screen.getByTestId("agent-v1")).getByText(/驗收合併結果中/)).toBeTruthy();
    });

    it("驗收要你決定：卡上可以放行，報告列出每項與截圖", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent(), orch, ver]);
      vi.mocked(listTasks).mockResolvedValue([modA, merged, verify({ verdict: "attention", review_reasons: JSON.stringify(["整體樣式"]) })]);
      vi.mocked(verifyReport).mockResolvedValue({
        report: {
          verdict: "fail",
          project_type: "web",
          items: [
            { criterion: "登入成功", status: "pass", evidence: ["evidence/login.png"] },
            { criterion: "整體樣式", status: "fail", evidence: ["evidence/style.png"], steps: "1. 打開首頁", actual: "版面跑掉" },
            { criterion: "付款", status: "manual", evidence: [], notes: "要真實付款" },
          ],
        },
        images: { "evidence/login.png": "data:image/png;base64,AAAA", "evidence/style.png": "data:image/png;base64,BBBB" },
      });
      view();
      const node = await screen.findByTestId("agent-v1");
      await waitFor(() => expect(within(node).getByText(/需要你決定/)).toBeTruthy());
      expect(screen.getByTestId("ac-summary").textContent).toMatch(/驗收沒通過/);

      fireEvent.click(within(node).getByText("驗收報告"));
      const dlg = await screen.findByRole("dialog", { name: "驗收報告" });
      await waitFor(() => expect(within(dlg).getByTestId("verify-summary").textContent).toMatch(/通過 1 · 沒通過 1 · 需你確認 1/));
      expect(within(dlg).getAllByTestId(/^verify-item-/).map((e) => e.dataset.testid)).toEqual([
        "verify-item-fail",
        "verify-item-manual",
        "verify-item-pass",
      ]);
      expect(within(dlg).getByText("版面跑掉")).toBeTruthy();
      expect(within(dlg).getByText("要真實付款")).toBeTruthy();
      expect(within(dlg).getByAltText("evidence/style.png").getAttribute("src")).toBe("data:image/png;base64,BBBB");

      fireEvent.click(within(dlg).getByRole("button", { name: "匯出" }));
      await waitFor(() => expect(within(dlg).getByText(/已匯出：\/Users\/me\/驗收報告.html/)).toBeTruthy());
      const [path, html] = vi.mocked(writeTextFile).mock.calls[0];
      expect(path).toBe("/Users/me/驗收報告.html");
      expect(html).toContain("版面跑掉");
      expect(html).toContain("data:image/png;base64,BBBB");
      expect(html).toContain("HTBKTW");

      fireEvent.click(within(dlg).getByRole("button", { name: "放行" }));
      await waitFor(() => expect(vi.mocked(acceptVerification)).toHaveBeenCalledWith("p1", "V"));
    });

    it("模組都審過了，審核者不再顯示「等待工程師交件」", async () => {
      const rev = agent({ id: "r1", kind: "reviewer", name: "審核者" });
      vi.mocked(listAgents).mockResolvedValue([agent(), rev]);
      vi.mocked(listTasks).mockResolvedValue([
        modA,
        card({ id: "RA", title: "審查", status: "done", outcome: "success", card_kind: "review", module_id: "A", review_of: "A", agent_id: "r1", verdict: "pass" }),
      ]);
      view();
      const node = await screen.findByTestId("agent-r1");
      await waitFor(() => expect(within(node).getByText(/模組都審查通過了/)).toBeTruthy());
      expect(within(node).queryByText(/等待工程師交件/)).toBeNull();
    });

    it("驗收通過：驗收者顯示通過，頂列照程式庫結果", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent(), orch, ver]);
      vi.mocked(listTasks).mockResolvedValue([modA, { ...merged, verdict: "repo_merged" }, verify({ verdict: "pass" })]);
      view();
      const node = await screen.findByTestId("agent-v1");
      await waitFor(() => expect(within(node).getByText(/驗收通過/)).toBeTruthy());
      expect(screen.getByTestId("ac-summary").textContent).toMatch(/已合回程式庫/);
    });
  });

  it("把模組拖到 Agent 上會指派給它", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([card({ id: "C", title: "C 訂單", status: "planning" })]);
    view();
    const mod = await screen.findByTestId("module-C");
    drag(mod, screen.getByTestId("agent-a1"));
    await waitFor(() => expect(assignAgent).toHaveBeenCalledWith("p1", "C", "a1"));
  });

  it("把待執行的 chip 拖回模組庫會取消指派", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([card({ id: "B", title: "B", status: "queued", agent_id: "a1" })]);
    view();
    const chip = await screen.findByTestId("chip-B");
    const lib = document.querySelector('[data-drop="library"]') as Element;
    drag(chip, lib);
    await waitFor(() => expect(assignAgent).toHaveBeenCalledWith("p1", "B", null));
  });

  it("從 Agent 池拖到桌布會開指派角色對話框，送出才建立", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    const pool = await screen.findByTestId("pool-coder");
    drag(pool, document.querySelector('[data-drop="canvas"]') as Element);
    const dlg = await screen.findByRole("dialog");
    expect(createAgent).not.toHaveBeenCalled();
    await waitFor(() =>
      expect((within(dlg).getByLabelText(/程式庫|Repository/) as HTMLInputElement).value).toBe("/repo/htbktw"),
    );
    fireEvent.click(within(dlg).getByRole("button", { name: /從預設角色帶入|Start from a preset/ }));
    fireEvent.click(within(dlg).getByRole("menuitem", { name: /規格撰寫師|Spec Writer/ }));
    expect((within(dlg).getByLabelText(/^角色名稱$|^Role name$/) as HTMLInputElement).value).toMatch(/規格撰寫師|Spec Writer/);
    await placeOnCanvas(dlg);
    await waitFor(() =>
      expect(createAgent).toHaveBeenCalledWith(
        "p1",
        expect.objectContaining({
          kind: "coder",
          role: expect.stringMatching(/規格撰寫師|Spec Writer/),
          prompt_prefix: expect.stringMatching(/docs\/specs/),
          project_dir: "/repo/htbktw",
        }),
      ),
    );
  });

  it("審核者的角色選單有綜合審核者（一般＋資安），選了會帶入兩種檢查的提示詞", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    drag(await screen.findByTestId("pool-reviewer"), document.querySelector('[data-drop="canvas"]') as Element);
    const dlg = await screen.findByRole("dialog");
    fireEvent.click(within(dlg).getByRole("button", { name: /從預設角色帶入|Start from a preset/ }));
    fireEvent.click(within(dlg).getByRole("menuitem", { name: /綜合審核者/ }));
    expect((within(dlg).getByLabelText(/^角色名稱$|^Role name$/) as HTMLInputElement).value).toBe("綜合審核者（一般＋資安）");
    const prompt = (within(dlg).getByLabelText(/提示詞|Prompt/) as HTMLTextAreaElement).value;
    expect(prompt).toMatch(/正確性/);
    expect(prompt).toMatch(/注入/);
  });

  it("新的審核者、驗收者模型預設 sonnet；工程師沿用預設", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    for (const [kind, expected] of [["verifier", "sonnet"], ["reviewer", "sonnet"], ["coder", ""]] as const) {
      drag(await screen.findByTestId(`pool-${kind}`), document.querySelector('[data-drop="canvas"]') as Element);
      const dlg = await screen.findByRole("dialog");
      expect((within(dlg).getByLabelText(/^模型$|^Model$/) as HTMLInputElement).value).toBe(expected);
      fireEvent.click(within(dlg).getByRole("button", { name: /取消|Cancel/ }));
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    }
  });

  it("桌布空白時中間有「從團隊範本開始」", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    fireEvent.click(await screen.findByTestId("ac-templates-empty"));
    expect(await screen.findByRole("dialog", { name: "團隊範本" })).toBeTruthy();
  });

  it("每個分頁用自己記住的看板專案；切換專案寫回這個分頁", async () => {
    const proj = (id: string, name: string) => ({
      id, name, description: "", path: `/${id}`, status: "ok" as const, counts: { planning: 0, queued: 0, running: 0, done: 0 }, error: null,
    });
    vi.mocked(listProjects).mockResolvedValue([proj("p1", "HTBKTW"), proj("p2", "Pomodoro")]);
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    localStorage.setItem("aiterm.agentCanvas.project", "p1"); // 全域「上次選的」是 p1
    const onProjectChange = vi.fn();
    render(
      <LocaleProvider>
        <AgentCanvasView isActive initialProjectId="p2" onProjectChange={onProjectChange} />
      </LocaleProvider>,
    );
    const select = (await screen.findByDisplayValue("Pomodoro")) as HTMLSelectElement;
    expect(select.value).toBe("p2");
    // 名稱每次都回報（專案改名後標題跟上），id 不變。
    await waitFor(() => expect(onProjectChange).toHaveBeenCalledWith("p2", "Pomodoro"));
    await waitFor(() => expect(listAgents).toHaveBeenCalledWith("p2"));
    fireEvent.change(select, { target: { value: "p1" } });
    expect(onProjectChange).toHaveBeenLastCalledWith("p1", "HTBKTW");
  });

  it("新分頁沒有記住的專案：用上次選的，並記到這個分頁上", async () => {
    const proj = (id: string, name: string) => ({
      id, name, description: "", path: `/${id}`, status: "ok" as const, counts: { planning: 0, queued: 0, running: 0, done: 0 }, error: null,
    });
    vi.mocked(listProjects).mockResolvedValue([proj("p1", "HTBKTW"), proj("p2", "Pomodoro")]);
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    localStorage.setItem("aiterm.agentCanvas.project", "p2");
    const onProjectChange = vi.fn();
    render(
      <LocaleProvider>
        <AgentCanvasView isActive onProjectChange={onProjectChange} />
      </LocaleProvider>,
    );
    expect(((await screen.findByDisplayValue("Pomodoro")) as HTMLSelectElement).value).toBe("p2");
    await waitFor(() => expect(onProjectChange).toHaveBeenCalledWith("p2", "Pomodoro"));
  });

  it("團隊範本：存下桌布上的團隊，套用時用選的程式庫建立，已經有的審核者不重複", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent(), agent({ id: "r1", kind: "reviewer", name: "審核者", canvas_x: 400 })]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    fireEvent.click(await screen.findByTestId("ac-templates"));
    const dlg = await screen.findByRole("dialog", { name: "團隊範本" });
    fireEvent.change(within(dlg).getByLabelText("範本名稱"), { target: { value: "標準團隊" } });
    fireEvent.click(within(dlg).getByRole("button", { name: "存成範本" }));
    const item = await within(dlg).findByTestId("tpl-標準團隊");
    expect(item.textContent).toMatch(/Coder Agent1、審核者/);
    const apply = within(item).getByRole("button", { name: "套用" });
    await waitFor(() => expect(apply).not.toBeDisabled());
    fireEvent.click(apply);
    await waitFor(() => expect(createAgent).toHaveBeenCalledTimes(1));
    expect(createAgent).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({ kind: "coder", name: "Coder Agent1", project_dir: "/repo/htbktw", canvas_x: 40, canvas_y: 300 }),
    );
  });

  it("可以自訂角色名稱與提示詞，存成我的角色後下次從選單帶入", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    drag(await screen.findByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
    let dlg = await screen.findByRole("dialog");
    fireEvent.change(within(dlg).getByLabelText(/^角色名稱$|^Role name$/), { target: { value: "SAP 顧問" } });
    const prefix = dlg.querySelector("textarea") as HTMLTextAreaElement;
    fireEvent.change(prefix, { target: { value: "你熟悉 SAP 模組，負責把 ABAP 邏輯搬到網頁。" } });
    fireEvent.click(within(dlg).getByRole("button", { name: /存成我的角色|Save as my role/ }));
    expect(within(dlg).getByText(/已存成我的角色|Saved to my roles/)).toBeTruthy();
    fireEvent.click(within(dlg).getByText(/取消|Cancel/));

    // 重新開一次對話框：我的角色出現在選單裡，帶入名稱與提示詞
    drag(screen.getByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
    dlg = await screen.findByRole("dialog");
    fireEvent.click(within(dlg).getByRole("button", { name: /從預設角色帶入|Start from a preset/ }));
    fireEvent.click(within(dlg).getByRole("menuitem", { name: /SAP 顧問/ }));
    expect((dlg.querySelector("textarea") as HTMLTextAreaElement).value).toContain("ABAP");
  });

  it("建立失敗（例如不是 git repo）錯誤留在對話框裡", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(createAgent).mockRejectedValueOnce("Agent 的程式庫必須是 git repo：/x");
    view();
    drag(await screen.findByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
    const dlg = await screen.findByRole("dialog");
    await waitFor(() => expect((within(dlg).getByLabelText(/程式庫|Repository/) as HTMLInputElement).value).not.toBe(""));
    await placeOnCanvas(dlg);
    expect(await within(dlg).findByRole("alert")).toHaveTextContent("必須是 git repo");
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("tasks-updated 會重新載入", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    await screen.findByTestId("agent-a1");
    const before = vi.mocked(listAgents).mock.calls.length;
    tasksUpdated?.();
    await waitFor(() => expect(vi.mocked(listAgents).mock.calls.length).toBeGreaterThan(before));
  });

  it("暫停中的 Agent 顯示失敗卡片，重試與跳過呼叫 agents_resolve", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ paused_task_id: "F" })]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "F", title: "F 設定", status: "done", outcome: "failed", error_message: "boom", agent_id: "a1" }),
    ]);
    view();
    const node = await screen.findByTestId("agent-a1");
    expect(within(node).getByRole("alert")).toHaveTextContent("boom");
    fireEvent.click(within(node).getByText(/重試|Retry/));
    await waitFor(() => expect(resolveAgent).toHaveBeenCalledWith("p1", "a1", "retry"));
    fireEvent.click(within(node).getByText(/跳過|Skip/));
    await waitFor(() => expect(resolveAgent).toHaveBeenCalledWith("p1", "a1", "skip"));
  });

  it("活動記錄每一行有角色色點", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent(), agent({ id: "r1", kind: "reviewer", name: "R" })]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", status: "done", outcome: "success", agent_id: "a1", dispatched_at: 100, finished_at: 200 }),
      card({ id: "B", status: "done", outcome: "success", agent_id: "r1", card_kind: "review", dispatched_at: 300, finished_at: 400 }),
    ]);
    view();
    await screen.findByTestId("agent-r1");
    const log = document.querySelector(".ac-log") as HTMLElement;
    await waitFor(() => expect(log.querySelector(".ac-log-dot--reviewer")).not.toBeNull());
    expect(log.querySelector(".ac-log-dot--coder")).not.toBeNull();
  });

  it("自動排列：依流程由左到右寫回位置，工程師在最左、驗收者在最右", async () => {
    vi.mocked(updateAgent).mockClear();
    vi.mocked(listAgents).mockResolvedValue([
      agent({ id: "v1", kind: "verifier", name: "V", canvas_x: 0, canvas_y: 0 }),
      agent({ id: "a1", canvas_x: 900, canvas_y: 600 }),
      agent({ id: "r1", kind: "reviewer", name: "R", canvas_x: 50, canvas_y: 900 }),
    ]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    await screen.findByTestId("agent-v1");
    fireEvent.click(screen.getByRole("button", { name: /自動排列|Arrange/ }));
    await waitFor(() => expect(updateAgent).toHaveBeenCalledTimes(3));
    const x = (id: string) => vi.mocked(updateAgent).mock.calls.find(([, a]) => a.id === id)![1].canvas_x;
    expect(x("a1")).toBeLessThan(x("r1"));
    expect(x("r1")).toBeLessThan(x("v1"));
  });

  it("工程師卡底部顯示「模組 x/y」，分支名放在提示裡；圖示依角色", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ branch: "aiterm-agent/a1" })]);
    vi.mocked(listTasks).mockResolvedValue([card({ id: "A", status: "running", agent_id: "a1" })]);
    view();
    const node = await screen.findByTestId("agent-a1");
    const progress = await within(node).findByText(/^(模組|Modules) 0\/1$/);
    expect(progress.getAttribute("title")).toBe("aiterm-agent/a1");
    expect(within(node).queryByText(/⑂/)).toBeNull();
    expect(node.querySelector('[data-kind-icon="coder"]')).not.toBeNull();
  });

  it("有分支且閒置時可以合併，衝突時顯示還原合併", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ branch: "aiterm-agent/a1" })]);
    vi.mocked(listTasks).mockResolvedValue([card({ id: "A", status: "done", outcome: "success", agent_id: "a1" })]);
    vi.mocked(mergeAgent).mockResolvedValueOnce({ status: "conflict", files: ["x.ts"] });
    view();
    const node = await screen.findByTestId("agent-a1");
    fireEvent.click(within(node).getByText(/合併分支|Merge branch/));
    await waitFor(() => expect(mergeAgent).toHaveBeenCalledWith("p1", "a1"));
    expect(await within(node).findByRole("button", { name: /還原合併|Abort merge/ })).toBeTruthy();
  });

  it("執行中的 Agent 有 coding 狀態與經過時間", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A 登入", status: "running", agent_id: "a1", dispatched_at: Math.floor(Date.now() / 1000) - 75 }),
    ]);
    view();
    const node = await screen.findByTestId("agent-a1");
    expect(node.className).toContain("ac-agent--coding");
    expect(within(node).getByText(/1:1[5-9]/)).toBeTruthy();
  });

  it("撞到用量上限的 Agent 顯示等待額度與換帳號會自動繼續", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A 登入", status: "running", agent_id: "a1", dispatched_at: 1, quota_wait_at: 2 }),
    ]);
    view();
    const node = await screen.findByTestId("agent-a1");
    expect(within(node).getByText(/額度用完|Usage limit reached/)).toBeTruthy();
    expect(within(node).getByText(/換帳號|switch accounts/)).toBeTruthy();
  });

  it("Reviewer 已在桌布上時，池裡的 Reviewer 停用；Orchestrator 仍可拖", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent(), agent({ id: "r1", kind: "reviewer", name: "Code Reviewer" })]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    // 池在資料載入前就渲染了——要等 Agent 出現，停用狀態才會反映出來。
    await screen.findByTestId("agent-r1");
    const pool = screen.getByTestId("pool-reviewer");
    expect(pool.getAttribute("aria-disabled")).toBe("true");
    expect(screen.getByTestId("pool-orchestrator").getAttribute("aria-disabled")).toBe("false");
    drag(pool, document.querySelector('[data-drop="canvas"]') as Element);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("自動連線：Coder→Reviewer→Orchestrator，審查中的那條亮起來", async () => {
    vi.mocked(listAgents).mockResolvedValue([
      agent(),
      agent({ id: "r1", kind: "reviewer", name: "R" }),
      agent({ id: "o1", kind: "orchestrator", name: "O" }),
    ]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A", status: "done", outcome: "success", agent_id: "a1" }),
      card({ id: "RA", title: "審查：A", status: "running", agent_id: "r1", card_kind: "review", module_id: "A", review_of: "A" }),
    ]);
    view();
    const edge = await screen.findByTestId("edge-a1-r1");
    expect(edge.getAttribute("data-active")).not.toBe("");
    expect(screen.getByTestId("edge-r1-o1").getAttribute("data-active")).toBe("");
    expect(screen.getByTestId("agent-r1").className).toContain("ac-agent--reviewing");
    expect(within(screen.getByTestId("agent-r1")).getByText(/審查 A|Reviewing A/)).toBeTruthy();
  });

  it("審查不通過超過上限的模組，點 chip 會放行", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent(), agent({ id: "r1", kind: "reviewer", name: "R" })]);
    const rv = (id: string, of: string) =>
      card({ id, status: "done", outcome: "success", agent_id: "r1", card_kind: "review", module_id: "A", review_of: of, verdict: "fail" });
    const fx = (id: string, of: string) =>
      card({ id, status: "done", outcome: "success", agent_id: "a1", card_kind: "fix", module_id: "A", review_of: of });
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A 登入", status: "done", outcome: "success", agent_id: "a1" }),
      rv("R1", "A"), fx("F1", "R1"), rv("R2", "F1"), fx("F2", "R2"), rv("R3", "F2"),
    ]);
    view();
    const chip = await screen.findByTestId("chip-A");
    expect(chip.getAttribute("data-state")).toBe("attention");
    fireEvent.click(chip);
    await waitFor(() => expect(overrideModule).toHaveBeenCalledWith("p1", "A"));
  });

  it("修正卡與審查卡不會出現在模組庫，也不會變成 Coder 上的獨立 chip", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A", status: "done", outcome: "success", agent_id: "a1" }),
      card({ id: "F1", title: "修正：A", status: "queued", agent_id: "a1", card_kind: "fix", module_id: "A" }),
    ]);
    view();
    await screen.findByTestId("chip-A");
    expect(screen.queryByTestId("chip-F1")).toBeNull();
  });

  it("選規格撰寫師會勾起「產出規格」，建立時送出 produces_spec", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    drag(await screen.findByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
    const dlg = await screen.findByRole("dialog");
    const box = within(dlg).getByRole("checkbox", { name: /產出規格|Produces specs/ }) as HTMLInputElement;
    expect(box.checked).toBe(false);
    fireEvent.click(within(dlg).getByRole("button", { name: /從預設角色帶入|Start from a preset/ }));
    fireEvent.click(within(dlg).getByRole("menuitem", { name: /規格撰寫師|Spec Writer/ }));
    expect(box.checked).toBe(true);
    await waitFor(() => expect((within(dlg).getByLabelText(/程式庫|Repository/) as HTMLInputElement).value).not.toBe(""));
    await placeOnCanvas(dlg);
    await waitFor(() => expect(createAgent).toHaveBeenCalledWith("p1", expect.objectContaining({ produces_spec: true })));
  });

  it.each([
    ["repo_merged", /已合回程式庫|Merged into the repo$/],
    ["repo_diverged", /尚未合回程式庫：程式庫的分支有整合分支沒有的 commit|Not merged into the repo yet: the repo branch/],
    ["repo_dirty", /尚未合回程式庫：程式庫有未提交的改動|Not merged into the repo yet: the repo has uncommitted/],
    [null, /已合併進整合分支|Merged into the integration branch/],
  ] as const)("合併卡結束後，協調者顯示有沒有合回程式庫（%s）", async (verdict, text) => {
    vi.mocked(listAgents).mockResolvedValue([agent(), agent({ id: "o1", kind: "orchestrator", name: "O", branch: "aiterm-agent/o1" })]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "A", title: "A", status: "done", outcome: "success", agent_id: "a1" }),
      card({ id: "M", title: "合併 1 個工程師分支", status: "done", outcome: "success", agent_id: "o1", card_kind: "merge", finished_at: 9, verdict }),
    ]);
    view();
    const node = await screen.findByTestId("agent-o1");
    await waitFor(() => expect(within(node).getByText(text)).toBeTruthy());
  });

  it("協調者拆解中：規格模組顯示 🧭，Orchestrator 顯示拆解規格", async () => {
    vi.mocked(listAgents).mockResolvedValue([
      agent({ produces_spec: true, name: "規格撰寫師" }),
      agent({ id: "o1", kind: "orchestrator", name: "O" }),
    ]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "S", title: "A 規格", status: "done", outcome: "success", agent_id: "a1", is_spec: true }),
      card({ id: "P", title: "拆解規格：A 規格", status: "running", agent_id: "o1", card_kind: "plan", module_id: "S" }),
    ]);
    view();
    expect((await screen.findByTestId("chip-S")).getAttribute("data-state")).toBe("dispatching");
    expect(within(screen.getByTestId("agent-o1")).getByText(/拆解規格：A 規格|Splitting spec: A 規格/)).toBeTruthy();
    expect(within(screen.getByTestId("agent-o1")).getByText(/^拆解中$|^Splitting$/)).toBeTruthy();
    expect(within(screen.getByTestId("agent-a1")).getByLabelText(/產出規格|Produces specs/)).toBeTruthy();
  });

  it("選到不是 git repo 的資料夾：當場提示，一鍵初始化後才能放上桌布", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(checkRepo).mockResolvedValueOnce({ status: "not_repo" }).mockResolvedValue({ status: "ok", branch: "main" });
    view();
    drag(await screen.findByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
    const dlg = await screen.findByRole("dialog");
    const status = await within(dlg).findByTestId("repo-status");
    expect(status).toHaveTextContent(/不是 git repo|isn't a git repo/);
    const place = within(dlg).getByText(/放上桌布|Place on canvas/) as HTMLButtonElement;
    expect(place.disabled).toBe(true);

    fireEvent.click(within(status).getByRole("button", { name: /初始化為 git repo|Initialize as a git repo/ }));
    await waitFor(() => expect(initRepo).toHaveBeenCalledWith("/repo/htbktw", "p1"));
    await waitFor(() => expect(within(dlg).getByTestId("repo-status")).toHaveTextContent(/main/));
    expect(place.disabled).toBe(false);
  });

  describe("版面與總覽", () => {
    const orchestrated = (verdict: TaskWithAttachments["verdict"]) => {
      vi.mocked(listAgents).mockResolvedValue([
        agent({ branch: "aiterm-agent/a1" }),
        agent({ id: "r1", kind: "reviewer", name: "R" }),
        agent({ id: "o1", kind: "orchestrator", name: "O", branch: "aiterm-agent/bf7cb7ff-3c88-4149-b761-c4b4eadac581" }),
      ]);
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "A", title: "A", status: "done", outcome: "success", agent_id: "a1", module_override: true }),
        card({ id: "M", title: "合併", status: "done", outcome: "success", agent_id: "o1", card_kind: "merge", finished_at: 9, project_dir: "/repo/htbktw", verdict }),
      ]);
    };

    it("全部完成並合回程式庫：頂列寫出來，按「打開資料夾」開程式庫", async () => {
      orchestrated("repo_merged");
      view();
      expect(await screen.findByTestId("ac-summary")).toHaveTextContent(/全部完成：1 個模組已合回程式庫|All done: 1 modules merged into the repo/);
      fireEvent.click(screen.getByRole("button", { name: /打開資料夾|Open folder/ }));
      await waitFor(() => expect(openRepo).toHaveBeenCalledWith("/repo/htbktw"));
    });

    it("沒合回程式庫：頂列寫原因，沒有打開資料夾", async () => {
      orchestrated("repo_dirty");
      view();
      expect(await screen.findByTestId("ac-summary")).toHaveTextContent(/未提交的改動|uncommitted changes/);
      expect(screen.queryByRole("button", { name: /打開資料夾|Open folder/ })).toBeNull();
    });

    it("已合回程式庫就不再顯示合併分支；有協調者時工程師卡也不放合併分支；分支名縮短；審核者沒有分支列", async () => {
      orchestrated("repo_merged");
      view();
      const orch = await screen.findByTestId("agent-o1");
      expect(within(orch).queryByText(/^合併分支$|^Merge branch$/)).toBeNull();
      expect(within(screen.getByTestId("agent-a1")).queryByText(/^合併分支$|^Merge branch$/)).toBeNull();
      const branch = within(orch).getByText(/aiterm-agent\/bf7cb7ff$/);
      expect(branch.getAttribute("title")).toBe("aiterm-agent/bf7cb7ff-3c88-4149-b761-c4b4eadac581");
      expect(within(screen.getByTestId("agent-r1")).queryByText(/⑂/)).toBeNull();
    });

    it("還沒合回程式庫：協調者保留合併分支", async () => {
      orchestrated("repo_diverged");
      view();
      expect(within(await screen.findByTestId("agent-o1")).getByText(/^合併分支$|^Merge branch$/)).toBeTruthy();
    });

    it("協調者沒有在派工時，派工虛線淡掉", async () => {
      vi.mocked(listAgents).mockResolvedValue([
        agent({ id: "w", produces_spec: true, name: "規格" }),
        agent(),
        agent({ id: "o1", kind: "orchestrator", name: "O" }),
      ]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      const edge = await screen.findByTestId("edge-o1-a1");
      expect(edge.querySelector("path")?.getAttribute("class")).toContain("ac-edge-base--idle");
    });

    it("模組庫可以收合，記住設定；收合後仍可把模組拖回去", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent()]);
      vi.mocked(listTasks).mockResolvedValue([card({ id: "C", title: "C 訂單", status: "planning" })]);
      const first = view();
      await screen.findByTestId("module-C");
      fireEvent.click(screen.getByRole("button", { name: /收合模組庫|Collapse module library/ }));
      expect(screen.queryByTestId("module-C")).toBeNull();
      expect(document.querySelector('[data-drop="library"]')).toHaveTextContent(/· 1/);
      first.unmount();
      view();
      expect(await screen.findByRole("button", { name: /展開模組庫|Expand module library/ })).toBeTruthy();
    });

    it("活動記錄可以收合，收合後只留最新一筆", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent()]);
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "A", title: "A 登入", status: "done", outcome: "success", agent_id: "a1", dispatched_at: 100, finished_at: 200 }),
      ]);
      view();
      const title = await screen.findByRole("button", { name: /活動記錄|Activity/ });
      await waitFor(() => expect(document.querySelectorAll(".ac-log-line")).toHaveLength(2));
      fireEvent.click(title);
      expect(document.querySelectorAll(".ac-log-line")).toHaveLength(0);
      expect(document.querySelector(".ac-log-latest")?.textContent).toMatch(/A 登入/);
    });

    it("縮放按鈕與全部顯示", async () => {
      vi.mocked(listAgents).mockResolvedValue([
        agent({ canvas_x: 0, canvas_y: 0 }),
        agent({ id: "r1", kind: "reviewer", name: "R", canvas_x: 1800, canvas_y: 1200 }),
      ]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      await screen.findByTestId("agent-r1");
      const pct = screen.getByTestId("ac-zoom-pct");
      expect(pct).toHaveTextContent("100%");
      fireEvent.click(screen.getByRole("button", { name: /縮小|Zoom out/ }));
      expect(pct).toHaveTextContent("83%");
      fireEvent.click(screen.getByRole("button", { name: /放大|Zoom in/ }));
      expect(pct).toHaveTextContent("100%");

      // jsdom 沒有版面：給桌布一個 1000×600 的可視範圍。
      const canvas = document.querySelector<HTMLElement>('[data-drop="canvas"]') as HTMLElement;
      Object.defineProperty(canvas, "clientWidth", { configurable: true, value: 1000 });
      Object.defineProperty(canvas, "clientHeight", { configurable: true, value: 600 });
      fireEvent.click(screen.getByRole("button", { name: /全部顯示|Fit all/ }));
      // 節點群 2048×1340（含估計的節點尺寸）塞進 936×536：高度決定 → 40%。
      expect(pct).toHaveTextContent("40%");
      expect(screen.getByTestId("ac-world").style.transform).toBe("scale(0.4)");
    });
  });

  describe("相依關係與派工", () => {
    const setup = () => {
      vi.mocked(listAgents).mockResolvedValue([
        agent({ id: "a1", name: "工程師3" }),
        agent({ id: "a2", name: "工程師2" }),
        agent({ id: "r1", kind: "reviewer", name: "R" }),
      ]);
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "Y", title: "Y 模組", status: "done", outcome: "success", agent_id: "a2", from_spec: "S" }),
        card({ id: "RY", title: "審查：Y 模組", status: "queued", agent_id: "r1", card_kind: "review", module_id: "Y", review_of: "Y" }),
        card({
          id: "X",
          title: "整合",
          status: "queued",
          agent_id: "a1",
          from_spec: "S",
          depends_on: '["Y"]',
          created_at: "2026-10-16 10:00:00",
          assigned_from: "a2",
          assigned_at: 1_900_000_000,
        }),
        card({
          id: "Z",
          title: "Z 模組",
          status: "running",
          agent_id: "a1",
          dispatched_at: 1_900_000_100,
          premerged: '["Y"]',
        }),
      ]);
    };

    it("還在等別位工程師前置的模組：chip 顯示 ⛓ 與在等誰", async () => {
      setup();
      view();
      const chip = await screen.findByTestId("chip-X");
      expect(chip.textContent).toMatch(/^⛓/);
      expect(chip.getAttribute("title")).toMatch(/等前置模組：Y 模組|Waiting for prerequisites: Y 模組/);
    });

    it("活動記錄寫出派工、重新分配、開工前合併", async () => {
      setup();
      view();
      await screen.findByTestId("chip-X");
      const log = document.querySelector(".ac-log-lines") as HTMLElement;
      expect(log).toHaveTextContent(/派工：整合 → 工程師2|Dispatched: 整合 → 工程師2/);
      expect(log).toHaveTextContent(/重新分配：整合 從 工程師2 移給 工程師3|Reassigned: 整合 from 工程師2 to 工程師3/);
      expect(log).toHaveTextContent(/開工前合併：把 Y 模組 合進 工程師3|Merged before starting: Y 模組 into 工程師3/);
    });

    it("派工虛線滑過有說明：派工由系統決定", async () => {
      vi.mocked(listAgents).mockResolvedValue([
        agent({ id: "w", produces_spec: true, name: "規格" }),
        agent(),
        agent({ id: "o1", kind: "orchestrator", name: "O" }),
      ]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      const edge = await screen.findByTestId("edge-o1-a1");
      expect(edge.querySelector("title")?.textContent).toMatch(/由系統依忙碌程度與相依關係分派|assigns them by workload and dependencies/);
    });
  });

  describe("在等你", () => {
    it("停下來等回覆的 Agent：卡上寫在等你回覆，頂列計數", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent()]);
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "A", title: "規格", status: "running", agent_id: "a1", dispatched_at: 1, reply_wait_at: 2, tab_id: "tab-a" }),
      ]);
      view();
      const node = await screen.findByTestId("agent-a1");
      expect(within(node).getByRole("status")).toHaveTextContent(/在等你回覆|Waiting for your reply/);
      expect(within(node).getByText(/前往分頁|Open tab/)).toBeTruthy();
      expect(screen.getByTestId("ac-summary")).toHaveTextContent(/1 個 Agent 在等你|1 agent\(s\) waiting for you/);
    });

    it("執行中的互動卡：說明要手動完成，按標記完成", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent()]);
      vi.mocked(listTasks).mockResolvedValue([
        card({ id: "A", title: "規格", status: "running", agent_id: "a1", dispatched_at: 1, interactive: true }),
      ]);
      view();
      const node = await screen.findByTestId("agent-a1");
      expect(within(node).getByText(/互動卡|Interactive card/)).toBeTruthy();
      fireEvent.click(within(node).getByRole("button", { name: /標記完成|Mark done/ }));
      await waitFor(() => expect(markTaskDone).toHaveBeenCalledWith("p1", "A"));
    });
  });

  describe("依角色指定模型", () => {
    it("建立 Agent 時可以填模型，送出 model", async () => {
      vi.mocked(listAgents).mockResolvedValue([]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      drag(await screen.findByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
      const dlg = await screen.findByRole("dialog");
      fireEvent.change(within(dlg).getByLabelText(/^模型$|^Model$/), { target: { value: " sonnet " } });
      await waitFor(() => expect((within(dlg).getByLabelText(/程式庫|Repository/) as HTMLInputElement).value).not.toBe(""));
      await placeOnCanvas(dlg);
      await waitFor(() => expect(createAgent).toHaveBeenCalledWith("p1", expect.objectContaining({ model: "sonnet" })));
    });

    it("卡上顯示模型；編輯時帶出原本的模型", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent({ model: "opus" })]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      const node = await screen.findByTestId("agent-a1");
      expect(within(node).getByText(/^opus$/)).toBeTruthy();
      fireEvent.click(within(node).getByRole("button", { name: /設定|Settings/ }));
      const dlg = await screen.findByRole("dialog");
      expect((within(dlg).getByLabelText(/^模型$|^Model$/) as HTMLInputElement).value).toBe("opus");
    });
  });

  describe("AI 工具（Claude Code／Codex）", () => {
    it("選 Codex 時模型必填，送出帶 cli: codex", async () => {
      vi.mocked(listAgents).mockResolvedValue([]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      drag(await screen.findByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
      const dlg = await screen.findByRole("dialog");
      await within(dlg).findByText(/^✓/);
      fireEvent.click(within(dlg).getByRole("radio", { name: "Codex" }));
      const submit = within(dlg).getByRole("button", { name: /放上桌布|Place on canvas/ });
      await waitFor(() => expect(submit).toBeDisabled());
      fireEvent.change(within(dlg).getByLabelText(/^模型$|^Model$/), { target: { value: "Qwen3.6-35B-A3B-4bit" } });
      await waitFor(() => expect(submit).not.toBeDisabled());
      fireEvent.click(submit);
      await waitFor(() =>
        expect(createAgent).toHaveBeenCalledWith("p1", expect.objectContaining({ cli: "codex", model: "Qwen3.6-35B-A3B-4bit" })),
      );
    });

    it("Codex 的卡片標出 Codex", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent({ cli: "codex", model: "qwen" })]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      const node = await screen.findByTestId("agent-a1");
      expect(within(node).getByText(/^Codex · qwen$/)).toBeTruthy();
    });
  });

  describe("待決策收件匣", () => {
    const question = (over: Partial<QuestionView> = {}): QuestionView => ({
      project_id: "p1", project_name: "HTBKTW", task_title: "規格", agent_name: "Coder Agent1",
      id: 5, task_id: "A", agent_id: "a1", question: "名稱用中文？", options: '["中文","英文"]',
      recommended: "英文", context: "README 標題", status: "open", created_at: 1, ...over,
    });

    it("頂列顯示待決策數量；按選項回答，答案送回提問的 Agent", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent()]);
      vi.mocked(listTasks).mockResolvedValue([card({ id: "A", title: "規格", status: "running", agent_id: "a1", dispatched_at: 1 })]);
      vi.mocked(listQuestions).mockResolvedValue([question()]);
      view();
      const button = await screen.findByTestId("ac-inbox-button");
      await waitFor(() => expect(button).toHaveTextContent(/待決策 1|Decisions 1/));
      fireEvent.click(button);
      const item = await screen.findByTestId("question-5");
      expect(item).toHaveTextContent("名稱用中文？");
      expect(item).toHaveTextContent("README 標題");
      expect(within(item).getByRole("button", { name: /★ 英文/ })).toBeTruthy();
      fireEvent.click(within(item).getByRole("button", { name: /^中文$/ }));
      await waitFor(() => expect(answerQuestion).toHaveBeenCalledWith("p1", 5, "中文"));
      await waitFor(() => expect(screen.queryByTestId("question-5")).toBeNull());
    });

    it("也可以自己打字回答", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent()]);
      vi.mocked(listTasks).mockResolvedValue([]);
      vi.mocked(listQuestions).mockResolvedValue([question({ options: "[]", recommended: null })]);
      view();
      fireEvent.click(await screen.findByTestId("ac-inbox-button"));
      const item = await screen.findByTestId("question-5");
      fireEvent.change(within(item).getByRole("textbox"), { target: { value: " 用中文，副標寫英文 " } });
      fireEvent.click(within(item).getByRole("button", { name: /送出回答|Send answer/ }));
      await waitFor(() => expect(answerQuestion).toHaveBeenCalledWith("p1", 5, "用中文，副標寫英文"));
    });

    it("提問的 Agent 卡上顯示等決策，點一下打開那一題", async () => {
      vi.mocked(listAgents).mockResolvedValue([agent()]);
      vi.mocked(listTasks).mockResolvedValue([card({ id: "A", title: "規格", status: "running", agent_id: "a1", dispatched_at: 1 })]);
      vi.mocked(listQuestions).mockResolvedValue([question()]);
      view();
      const node = await screen.findByTestId("agent-a1");
      const hint = await within(node).findByText(/等決策：名稱用中文？|Waiting for your decision: 名稱用中文？/);
      fireEvent.click(hint);
      expect((await screen.findByTestId("question-5")).className).toContain("ac-q--focus");
    });
  });
});
