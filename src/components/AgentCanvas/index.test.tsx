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
  getProjectBrief: vi.fn().mockResolvedValue(null),
  setProjectBrief: vi.fn().mockResolvedValue(undefined),
  getTaskBoardConfig: vi.fn().mockResolvedValue({ max_concurrent: 3 }),
  onTasksUpdated: vi.fn((cb: () => void) => {
    tasksUpdated = cb;
    return Promise.resolve(() => {});
  }),
  readTranscript: vi.fn().mockResolvedValue(""),
  markTaskDone: vi.fn().mockResolvedValue(undefined),
  codexInfo: vi.fn().mockResolvedValue({ models: [], login: null, error: null }),
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
  checkTools: vi.fn(),
  setDbSkill: vi.fn().mockResolvedValue(undefined),
  listDbQueries: vi.fn(),
  getReferences: vi.fn(),
  setReferences: vi.fn(),
  listSkills: vi.fn(),
  readSkill: vi.fn(),
  setCustomSkills: vi.fn().mockResolvedValue(undefined),
  listSkillUses: vi.fn(),
}));
vi.mock("../../ipc/db", async () => ({
  ...(await vi.importActual<typeof import("../../ipc/db")>("../../ipc/db")),
  dbListConnections: vi.fn().mockResolvedValue([
    { id: "dev", name: "HTBKTW-DEV", db_type: "sqlite", host: "", port: 0, database: "dev.db", username: "", is_connected: false },
    { id: "prod", name: "HTBKTW-PROD", db_type: "postgresql", host: "h", port: 5432, database: "p", username: "u", is_connected: false },
  ]),
}));
vi.mock("../../ipc/projects", () => ({
  listProjects: vi.fn().mockResolvedValue([
    { id: "p1", name: "HTBKTW", description: "", path: "/p", status: "ok", counts: { planning: 0, queued: 0, running: 0, done: 0 }, error: null },
  ]),
  usedDirs: vi.fn().mockResolvedValue(["/repo/htbktw"]),
  usedLabels: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn().mockResolvedValue([]) }));
vi.mock("../../ipc/ai", async () => ({
  ...(await vi.importActual<typeof import("../../ipc/ai")>("../../ipc/ai")),
  invokeAiComplete: vi.fn(),
}));
// ModelPickerButton 會拉配額資訊。
vi.mock("../../ipc/usage", () => ({
  usageQuota: vi.fn().mockResolvedValue({ status: "not_applicable", provider_id: "x" }),
  usageQuotaAll: vi.fn().mockResolvedValue([]),
  primaryWindow: () => null,
}));
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
  checkTools,
  createAgent,
  listDbQueries,
  setDbSkill,
  getReferences,
  setReferences,
  listSkills,
  readSkill,
  setCustomSkills,
  listSkillUses,
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
import { codexInfo, getProjectBrief, listTasks, listTokenUsage, markTaskDone, setProjectBrief, type TaskWithAttachments } from "../../ipc/tasks";
import { answerQuestion, listQuestions, type QuestionView } from "../../ipc/questions";
import { writeTextFile } from "../../ipc/fs";
import { listProjects } from "../../ipc/projects";
import { invokeAiComplete } from "../../ipc/ai";
import { listProviders } from "../../ipc/provider";
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

const allTools = { platform: "macos" as const, claude_command: "claude", claude: "/c", codex: "/x", git: "/g" };

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(checkTools).mockResolvedValue(allTools);
  vi.mocked(listDbQueries).mockResolvedValue([]);
  vi.mocked(getReferences).mockResolvedValue([]);
  vi.mocked(listSkills).mockResolvedValue([
    { name: "htbtw-egl", description: "讀寫 MS950 檔案", path: "/repo/htbtw/.claude/skills/htbtw-egl", source: "repo" },
    { name: "karpathy-guidelines", description: "寫程式的紀律", path: "/Users/me/.claude/skills/karpathy-guidelines", source: "user" },
  ]);
  vi.mocked(listSkillUses).mockResolvedValue([]);
  vi.mocked(setReferences).mockImplementation(async (_p, refs) => refs);
  tasksUpdated = null;
  try {
    localStorage.clear();
    // 教學輔助預設「都看過、都關了」，既有測試不受導覽遮罩與提示影響；教學輔助自己的測試再清掉。
    localStorage.setItem(
      "aiterm.agentCanvas.guide",
      JSON.stringify({ tourSeen: true, checklistHidden: true, hintsOff: true, done: [], dismissedHints: [] }),
    );
  } catch {
    // ignore
  }
});

describe("AgentCanvasView", () => {
  it("把資料庫技能拖到工程師上：選連線與權限後存檔", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    const node = await screen.findByTestId("agent-a1");
    drag(screen.getByTestId("skill-db"), node);
    const dlg = await screen.findByTestId("db-skill-dialog");
    fireEvent.click(await within(dlg).findByLabelText(/HTBKTW-DEV/));
    fireEvent.click(within(dlg).getByLabelText(/每次核准|Approve each/));
    fireEvent.click(within(dlg).getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() => expect(setDbSkill).toHaveBeenCalledWith("p1", "a1", { connections: ["dev"], mode: "approve" }));
  });

  it("裝了技能的工程師顯示連線與權限；查詢中顯示 SQL 開頭 40 字，活動記錄顯示完整 SQL", async () => {
    const sql = "SELECT id, name, email, created_at FROM customers WHERE active = 1";
    vi.mocked(listAgents).mockResolvedValue([agent({ db_skill: '{"connections":["dev"],"mode":"read"}' })]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(listDbQueries).mockResolvedValue([
      { id: 2, task_id: "t", agent_id: "a1", connection_id: "dev", connection_name: "HTBKTW-DEV", sql, started_at: 200, finished_at: null, rows: null, ms: null, error: null, decision: "allowed" },
      { id: 1, task_id: "t", agent_id: "a1", connection_id: "dev", connection_name: "HTBKTW-DEV", sql: "SELECT count(*) FROM orders", started_at: 100, finished_at: 101, rows: 12, ms: 300, error: null, decision: "allowed" },
      { id: 0, task_id: "t", agent_id: "a1", connection_id: "dev", connection_name: "HTBKTW-DEV", sql: "DELETE FROM orders", started_at: 50, finished_at: 50, rows: null, ms: 0, error: "只開放唯讀", decision: "denied" },
    ]);
    view();
    const chip = await within(await screen.findByTestId("agent-a1")).findByTestId("agent-db-skill");
    await waitFor(() => expect(chip.className).toContain("ac-skill-row--active"));
    expect(chip.textContent).toContain("HTBKTW-DEV");
    expect(chip.textContent).toContain("唯讀");
    expect(chip.textContent).toContain(`${sql.slice(0, 40)}…`);
    expect(chip.textContent).not.toContain(sql);
    const log = screen.getByText(/SELECT count\(\*\) FROM orders/);
    expect(log.textContent).toMatch(/12 列/);
    expect(log.textContent).toMatch(/0\.3 秒/);
    expect(screen.getByText(/DELETE FROM orders/).textContent).toMatch(/擋下|拒絕/);
  });

  it("寫入等核准時技能列變琥珀色並寫「等你核准」，活動記錄也有一行", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ db_skill: '{"connections":["dev"],"mode":"approve"}' })]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(listDbQueries).mockResolvedValue([
      { id: 5, task_id: "t", agent_id: "a1", connection_id: "dev", connection_name: "HTBKTW-DEV", sql: "UPDATE orders SET qty = 9", started_at: 100, finished_at: null, rows: null, ms: null, error: null, decision: "pending" },
    ]);
    view();
    const row = await within(await screen.findByTestId("agent-a1")).findByTestId("agent-db-skill");
    await waitFor(() => expect(row.className).toContain("ac-skill-row--pending"));
    expect(row.textContent).toContain("等你核准");
    expect(screen.getByText(/等你核准 HTBKTW-DEV 的寫入/)).toBeTruthy();
  });

  it("資源區：有人用資料庫技能才出現；查詢中的資料庫發亮並拉一條流動的線", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([]);
    const { unmount } = view();
    await screen.findByTestId("agent-a1");
    expect(screen.queryByTestId("ac-resource-rail")).toBeNull();
    unmount();

    vi.mocked(listAgents).mockResolvedValue([
      agent({ db_skill: '{"connections":["dev"],"mode":"read"}' }),
      agent({ id: "a2", name: "工程師2", canvas_y: 240, db_skill: '{"connections":["dev"],"mode":"approve"}' }),
    ]);
    vi.mocked(listDbQueries).mockResolvedValue([
      { id: 1, task_id: "t", agent_id: "a1", connection_id: "dev", connection_name: "HTBKTW-DEV", sql: "SELECT 1", started_at: Math.floor(Date.now() / 1000), finished_at: null, rows: null, ms: null, error: null, decision: "allowed" },
    ]);
    view();
    const pill = await screen.findByTestId("resource-dev");
    await waitFor(() => expect(pill.className).toContain("ac-res--hot"));
    expect(pill.textContent).toContain("HTBKTW-DEV");
    expect(screen.getByTestId("access-a1-dev").getAttribute("data-state")).toBe("query");
    expect(screen.queryByTestId("access-a2-dev"), "閒置的不畫線").toBeNull();
    // 只有一位能用的資源直接寫是誰；兩位以上寫人數
    expect(pill.textContent).toContain("查詢中");

    // 滑鼠停在資料庫上：顯示誰有權限
    fireEvent.mouseEnter(pill);
    expect(screen.getByTestId("access-a2-dev").getAttribute("data-state")).toBe("hover");
    expect(within(screen.getByTestId("access-a2-dev")).getByText("工程師2 需核准")).toBeTruthy();
    expect(within(screen.getByTestId("access-a1-dev")).getByText(/唯讀$/)).toBeTruthy();
    fireEvent.mouseLeave(pill);
    expect(screen.queryByTestId("access-a2-dev")).toBeNull();
  });

  it("閒置的資源：一位可用時寫是誰與權限，多位時寫人數；標題只寫「資源」", async () => {
    vi.mocked(listAgents).mockResolvedValue([
      agent({ db_skill: '{"connections":["dev","prod"],"mode":"read"}' }),
      agent({ id: "a2", name: "工程師2", canvas_y: 240, db_skill: '{"connections":["dev"],"mode":"approve"}' }),
    ]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    const prod = await screen.findByTestId("resource-prod");
    await waitFor(() => expect(prod.textContent).toContain("Coder Agent1 · 唯讀"));
    expect(screen.getByTestId("resource-dev").textContent).toContain("2 位可用");
    const rail = screen.getByTestId("ac-resource-rail");
    expect(within(rail).getByText("資源").getAttribute("title")).toMatch(/多位 Agent 共用/);
  });

  it("資源多於 4 個：閒置的收合成一顆，使用中的單獨顯示；點開看清單", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ db_skill: '{"connections":["dev","prod","c3","c4","c5","c6"],"mode":"read"}' })]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(listDbQueries).mockResolvedValue([
      { id: 1, task_id: "t", agent_id: "a1", connection_id: "c3", connection_name: "c3", sql: "SELECT 1", started_at: Math.floor(Date.now() / 1000), finished_at: null, rows: null, ms: null, error: null, decision: "allowed" },
    ]);
    view();
    expect(await screen.findByTestId("resource-c3")).toBeTruthy();
    expect(screen.queryByTestId("resource-dev")).toBeNull();
    const group = screen.getByTestId("resource-group-db");
    expect(group.textContent).toMatch(/×\s*5/);
    fireEvent.click(within(group).getByRole("button"));
    expect(await within(group).findByText("HTBKTW-PROD")).toBeTruthy();
  });

  it("參考資料夾：頂列設定，存檔後資源區出現資料夾（全員唯讀）", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValueOnce("/Users/me/HTBTW_Source");
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    await screen.findByTestId("agent-a1");
    fireEvent.click(screen.getByTestId("ac-references"));
    const dlg = await screen.findByTestId("references-dialog");
    fireEvent.click(within(dlg).getByRole("button", { name: /加入資料夾|Add folder/ }));
    const note = await within(dlg).findByLabelText(/HTBTW_Source/);
    fireEvent.change(note, { target: { value: "舊系統原始碼與 ArcDocs" } });
    vi.mocked(getReferences).mockResolvedValue([{ path: "/Users/me/HTBTW_Source", note: "舊系統原始碼與 ArcDocs" }]);
    fireEvent.click(within(dlg).getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() =>
      expect(setReferences).toHaveBeenCalledWith("p1", [{ path: "/Users/me/HTBTW_Source", note: "舊系統原始碼與 ArcDocs" }]),
    );
    const pill = await screen.findByTestId("resource-ref:/Users/me/HTBTW_Source");
    expect(pill.textContent).toContain("HTBTW_Source");
    expect(pill.textContent).toContain("全員 · 唯讀");
    expect(screen.getByTestId("ac-references").textContent).toMatch(/1/);
  });

  it("自訂技能：拖到工程師上，勾選程式庫與自己的技能，也能從資料夾加入，存檔", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValueOnce("/Users/me/Downloads/htbtw-db");
    vi.mocked(readSkill).mockResolvedValueOnce({ name: "htbtw-db", description: "查資料庫", path: "/Users/me/Downloads/htbtw-db", source: "folder" });
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    const node = await screen.findByTestId("agent-a1");
    drag(screen.getByTestId("skill-custom"), node);
    const dlg = await screen.findByTestId("custom-skills-dialog");
    fireEvent.click(await within(dlg).findByLabelText(/htbtw-egl/));
    expect(listSkills).toHaveBeenCalledWith("/repo/htbktw");
    expect(within(dlg).getByText("寫程式的紀律")).toBeTruthy();
    fireEvent.click(within(dlg).getByRole("button", { name: /從資料夾加入|Add from folder/ }));
    expect(await within(dlg).findByLabelText(/htbtw-db/)).toBeTruthy();
    fireEvent.click(within(dlg).getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() =>
      expect(setCustomSkills).toHaveBeenCalledWith("p1", "a1", ["/repo/htbtw/.claude/skills/htbtw-egl", "/Users/me/Downloads/htbtw-db"]),
    );
  });

  it("自訂技能列：顯示技能名稱；剛讀過技能時亮起來並寫使用中，活動記錄記一行", async () => {
    vi.mocked(listAgents).mockResolvedValue([
      agent({ custom_skills: '["/repo/htbtw/.claude/skills/htbtw-egl","/Users/me/.claude/skills/karpathy-guidelines"]' }),
    ]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(listSkillUses).mockResolvedValue([{ id: 1, task_id: "t", agent_id: "a1", skill: "htbtw-egl", at: Math.floor(Date.now() / 1000) }]);
    view();
    const row = await within(await screen.findByTestId("agent-a1")).findByTestId("agent-custom-skills");
    expect(row.textContent).toContain("htbtw-egl");
    expect(row.textContent).toContain("karpathy-guidelines");
    await waitFor(() => expect(row.className).toContain("ac-skill-row--active"));
    expect(row.textContent).toMatch(/使用中/);
    expect(screen.getByText(/使用技能 htbtw-egl/)).toBeTruthy();
  });

  it("點技能標籤可以移除技能", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ db_skill: '{"connections":["dev"],"mode":"write"}' })]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    fireEvent.click(await screen.findByTestId("agent-db-skill"));
    const dlg = await screen.findByTestId("db-skill-dialog");
    expect((within(dlg).getByLabelText(/直接放行|Allow writes/) as HTMLInputElement).checked).toBe(true);
    fireEvent.click(within(dlg).getByRole("button", { name: /移除技能|Remove skill/ }));
    await waitFor(() => expect(setDbSkill).toHaveBeenCalledWith("p1", "a1", null));
  });

  it("缺 Claude Code 時顯示安裝指令；裝好後按重新檢查就消失", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(checkTools).mockResolvedValue({ ...allTools, claude: null });
    view();
    const banner = await screen.findByTestId("ac-tools");
    expect(within(banner).getByText("curl -fsSL https://claude.ai/install.sh | bash")).toBeTruthy();
    // 橫幅可能比 Agent 先出現（還沒有 Agent 時也預設要 Claude Code），卡片要用 find 等。
    expect(within(await screen.findByTestId("agent-a1")).getByTestId("agent-tool-missing")).toBeTruthy();

    vi.mocked(checkTools).mockResolvedValue(allTools);
    fireEvent.click(within(banner).getByRole("button", { name: /重新檢查|Check again/ }));
    await waitFor(() => expect(screen.queryByTestId("ac-tools")).toBeNull());
    expect(screen.queryByTestId("agent-tool-missing")).toBeNull();
  });

  it("還沒有任何專案時也顯示缺工具提示", async () => {
    vi.mocked(listProjects).mockResolvedValueOnce([]);
    vi.mocked(checkTools).mockResolvedValue({ ...allTools, claude: null, git: null });
    view();
    const banner = await screen.findByTestId("ac-tools");
    expect(within(banner).getByText("xcode-select --install")).toBeTruthy();
    expect(screen.getByText(/還沒有看板專案/)).toBeTruthy();
  });

  it("只用 Codex 的團隊不會因為沒裝 Claude Code 跳提示；檢查失敗也不顯示", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ cli: "codex" })]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(checkTools).mockResolvedValue({ ...allTools, claude: null });
    view();
    await screen.findByTestId("agent-a1");
    await waitFor(() => expect(checkTools).toHaveBeenCalled());
    expect(screen.queryByTestId("ac-tools")).toBeNull();

    vi.mocked(checkTools).mockRejectedValue("zsh 在 10 秒內沒有回應");
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(checkTools).toHaveBeenCalledTimes(2));
    expect(screen.queryByTestId("ac-tools")).toBeNull();
  });

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

    it("驗收退回：修正還在做寫「修正中」，做完了寫「等待重新合併」", async () => {
      const failed = verify({ verdict: "fail", review_reasons: JSON.stringify(["導覽列"]) });
      const fix = card({ id: "F", title: "修正：A 登入", status: "running", outcome: null, card_kind: "fix", agent_id: "a1", module_id: "A", review_of: "A" });
      vi.mocked(listAgents).mockResolvedValue([agent(), orch, ver]);
      vi.mocked(listTasks).mockResolvedValue([modA, merged, failed, fix]);
      const { unmount } = view();
      await waitFor(() => expect(within(screen.getByTestId("agent-v1")).getByText(/工程師修正中/)).toBeTruthy());
      unmount();

      vi.mocked(listTasks).mockResolvedValue([modA, merged, failed, { ...fix, status: "done", outcome: "success", finished_at: 30 }]);
      view();
      await waitFor(() => expect(within(screen.getByTestId("agent-v1")).getByText(/修正已完成，等待重新合併/)).toBeTruthy());
      expect(screen.getByTestId("ac-summary").textContent).toMatch(/等待重新合併/);
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

  /** 實機：空桌布套用範本，選的資料夾還不是程式庫，只叫人「先放一位 Agent 初始化」。 */
  it("團隊範本：空桌布選的資料夾還不是程式庫，在對話框裡直接初始化再套用", async () => {
    localStorage.setItem(
      "aiterm.agentCanvas.teamTemplates",
      JSON.stringify([{ name: "小隊", agents: [{ kind: "coder", name: "工程師1", role: "", prompt_prefix: "", model: "", produces_spec: false, dx: 0, dy: 0 }] }]),
    );
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(checkRepo).mockResolvedValueOnce({ status: "not_repo" }).mockResolvedValue({ status: "ok", branch: "main" });
    view();
    fireEvent.click(await screen.findByTestId("ac-templates-empty"));
    const dlg = await screen.findByRole("dialog", { name: "團隊範本" });
    expect(within(dlg).getByText(/按「選擇…」挑 Agent 要寫程式的資料夾/)).toBeTruthy();
    fireEvent.click(within(dlg).getByRole("button", { name: "選擇…" }));
    const apply = within(await within(dlg).findByTestId("tpl-小隊")).getByRole("button", { name: "套用" });
    fireEvent.click(await within(dlg).findByRole("button", { name: "初始化為 git repo" }));
    await waitFor(() => expect(initRepo).toHaveBeenCalledWith("/repo", "p1"));
    await waitFor(() => expect(apply).not.toBeDisabled());
    fireEvent.click(apply);
    await waitFor(() => expect(createAgent).toHaveBeenCalledWith("p1", expect.objectContaining({ name: "工程師1", project_dir: "/repo" })));
  });

  it("團隊範本：選到看板專案資料夾要說清楚，而且不給初始化", async () => {
    localStorage.setItem(
      "aiterm.agentCanvas.teamTemplates",
      JSON.stringify([{ name: "小隊", agents: [{ kind: "coder", name: "工程師1", role: "", prompt_prefix: "", model: "", produces_spec: false, dx: 0, dy: 0 }] }]),
    );
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(checkRepo).mockResolvedValueOnce({ status: "board_folder" });
    view();
    fireEvent.click(await screen.findByTestId("ac-templates-empty"));
    const dlg = await screen.findByRole("dialog", { name: "團隊範本" });
    fireEvent.click(within(dlg).getByRole("button", { name: "選擇…" }));
    expect((await within(dlg).findByTestId("repo-status")).textContent).toMatch(/看板專案資料夾，不是程式碼/);
    expect(within(dlg).queryByRole("button", { name: /初始化|第一個 commit/ })).toBeNull();
    expect(within(within(dlg).getByTestId("tpl-小隊")).getByRole("button", { name: "套用" })).toBeDisabled();
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

  it("工程師套用內建角色時，標題圖示換成角色的線條圖示", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent({ role: "架構師" })]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    const node = await screen.findByTestId("agent-a1");
    expect(node.querySelector('.ac-agent-ico [data-role-icon="landmark"]')).not.toBeNull();
    expect(node.querySelector('[data-kind-icon="coder"]')).toBeNull();
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
      fireEvent.change(within(dlg).getByLabelText(/^模型$|^Model$/), { target: { value: "gpt-5.6-terra" } });
      await waitFor(() => expect(submit).not.toBeDisabled());
      fireEvent.click(submit);
      await waitFor(() =>
        expect(createAgent).toHaveBeenCalledWith("p1", expect.objectContaining({ cli: "codex", model: "gpt-5.6-terra" })),
      );
    });

    it("選 Codex 時列出 OpenAI 的模型、預選第一個並顯示登入狀態", async () => {
      vi.mocked(codexInfo).mockResolvedValueOnce({
        models: [
          { slug: "gpt-6-luna", name: "GPT-6-Luna" },
          { slug: "gpt-5.6-terra", name: "GPT-5.6-Terra" },
        ],
        login: "Logged in using ChatGPT",
        error: null,
      });
      vi.mocked(listAgents).mockResolvedValue([]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      drag(await screen.findByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
      const dlg = await screen.findByRole("dialog");
      await within(dlg).findByText(/^✓/);
      expect(codexInfo).not.toHaveBeenCalled();
      fireEvent.click(within(dlg).getByRole("radio", { name: "Codex" }));
      const model = within(dlg).getByLabelText(/^模型$|^Model$/) as HTMLInputElement;
      await waitFor(() => expect(model.value).toBe("gpt-6-luna"));
      expect(within(dlg).getByText(/Logged in using ChatGPT/)).toBeTruthy();
      const opts = Array.from(document.querySelectorAll("#ac-codex-models option")).map((o) => (o as HTMLOptionElement).value);
      expect(opts).toEqual(["gpt-6-luna", "gpt-5.6-terra"]);
      expect(model.getAttribute("list")).toBe("ac-codex-models");
    });

    it("讀不到 Codex 模型清單時顯示原因", async () => {
      vi.mocked(codexInfo).mockResolvedValueOnce({ models: [], login: null, error: "command not found: codex" });
      vi.mocked(listAgents).mockResolvedValue([]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      drag(await screen.findByTestId("pool-coder"), document.querySelector('[data-drop="canvas"]') as Element);
      const dlg = await screen.findByRole("dialog");
      await within(dlg).findByText(/^✓/);
      fireEvent.click(within(dlg).getByRole("radio", { name: "Codex" }));
      expect(await within(dlg).findByText(/command not found: codex/)).toBeTruthy();
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

describe("教學輔助", () => {
  // 前面的測試設了待決策的問題（mockResolvedValue 不會被 clearAllMocks 清掉），
  // 有問題時「回答問題」的提示會排在最前面。
  beforeEach(() => vi.mocked(listQuestions).mockResolvedValue([]));
  const GUIDE = "aiterm.agentCanvas.guide";
  const setGuide = (over: Record<string, unknown>) =>
    localStorage.setItem(GUIDE, JSON.stringify({ tourSeen: true, checklistHidden: true, hintsOff: true, done: [], dismissedHints: [], ...over }));
  const stored = () => JSON.parse(localStorage.getItem(GUIDE) ?? "{}") as Record<string, unknown>;

  it("第一次打開自動跑導覽；略過後不再自動出現，可以從「?」重看", async () => {
    localStorage.removeItem(GUIDE);
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    const first = view();
    let tour = await screen.findByTestId("ac-tour");
    expect(within(tour).getByText("加入角色")).toBeTruthy();
    expect(within(tour).getByText("1 / 8")).toBeTruthy();
    fireEvent.click(within(tour).getByRole("button", { name: "下一步" }));
    expect(within(tour).getByText("團隊範本")).toBeTruthy();
    fireEvent.click(within(tour).getByRole("button", { name: "略過" }));
    expect(screen.queryByTestId("ac-tour")).toBeNull();
    expect(stored().tourSeen).toBe(true);
    first.unmount();

    view();
    await screen.findByTestId("ac-templates");
    expect(screen.queryByTestId("ac-tour")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "說明" }));
    const help = await screen.findByRole("dialog", { name: "Agent 協作怎麼運作" });
    fireEvent.click(within(help).getByRole("button", { name: "重看導覽" }));
    tour = await screen.findByTestId("ac-tour");
    expect(screen.queryByRole("dialog", { name: "Agent 協作怎麼運作" })).toBeNull();
    // 最後一步是「完成」。
    for (let i = 0; i < 7; i++) fireEvent.click(within(tour).getByRole("button", { name: "下一步" }));
    fireEvent.click(within(tour).getByRole("button", { name: "完成" }));
    expect(screen.queryByTestId("ac-tour")).toBeNull();
  });

  it("上手清單依桌布狀態打勾；關掉後從「?」打開", async () => {
    setGuide({ checklistHidden: false });
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([card({ id: "m1", card_kind: "work", agent_id: "a1", status: "queued" })]);
    view();
    const list = await screen.findByTestId("ac-checklist");
    await waitFor(() => expect(list.querySelector('[data-step="assign"]')?.getAttribute("data-state")).toBe("done"));
    expect(list.querySelector('[data-step="coder"]')?.getAttribute("data-state")).toBe("done");
    expect(list.querySelector('[data-step="module"]')?.getAttribute("data-state")).toBe("done");
    expect(list.querySelector('[data-step="reviewer"]')?.getAttribute("data-state")).toBe("now");
    expect(within(list).getByText(/把「審核者」拖到桌布上/)).toBeTruthy();
    expect(within(list).getByText("3/8")).toBeTruthy();
    expect(stored().done).toEqual(["coder", "module", "assign"]);

    fireEvent.click(within(list).getByRole("button", { name: "關閉上手清單" }));
    expect(screen.queryByTestId("ac-checklist")).toBeNull();
    expect(stored().checklistHidden).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "說明" }));
    fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "打開上手清單" }));
    expect(await screen.findByTestId("ac-checklist")).toBeTruthy();
  });

  it("上手清單依序打勾：前面沒完成，後面在別的專案做過也先不勾", async () => {
    // 別的專案已經新增過模組、放過審核者，但這台電腦還沒放過工程師。
    setGuide({ checklistHidden: false, done: ["module", "reviewer"] });
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    const list = await screen.findByTestId("ac-checklist");
    const state = (s: string) => list.querySelector(`[data-step="${s}"]`)?.getAttribute("data-state");
    expect(state("coder")).toBe("now");
    expect(state("module")).toBe("todo");
    expect(state("reviewer")).toBe("todo");
    expect(within(list).getByText("0/8")).toBeTruthy();
    // 進度照樣記著沒被清掉，前面補上時會一起亮起來。
    expect(stored().done).toEqual(["module", "reviewer"]);
  });

  it("情境提示指向下一步；關掉的那一項不再出現", async () => {
    setGuide({ hintsOff: false });
    // jsdom 沒有版面，給目標一個在畫面裡的位置。
    const rect = vi
      .spyOn(Element.prototype, "getBoundingClientRect")
      .mockReturnValue({ left: 10, top: 10, right: 110, bottom: 60, width: 100, height: 50, x: 10, y: 10, toJSON: () => ({}) });
    try {
      vi.mocked(listAgents).mockResolvedValue([agent()]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      const hint = await screen.findByTestId("ac-hint-add_module");
      expect(hint.textContent).toMatch(/在模組庫新增模組/);
      fireEvent.click(within(hint).getByRole("button", { name: "不再提示這一項" }));
      expect(screen.queryByTestId("ac-hint-add_module")).toBeNull();
      expect(stored().dismissedHints).toEqual(["add_module"]);
    } finally {
      rect.mockRestore();
    }
  });

  it("Agent 暫停時優先提示「需要你處理」；「×」只關這一次，不記進進度", async () => {
    setGuide({ hintsOff: false });
    const rect = vi
      .spyOn(Element.prototype, "getBoundingClientRect")
      .mockReturnValue({ left: 10, top: 10, right: 110, bottom: 60, width: 100, height: 50, x: 10, y: 10, toJSON: () => ({}) });
    try {
      vi.mocked(listAgents).mockResolvedValue([agent({ paused_task_id: "t1" })]);
      vi.mocked(listTasks).mockResolvedValue([card({ id: "t1", card_kind: "work", agent_id: "a1", status: "done", outcome: "failed" })]);
      const first = view();
      const hint = await screen.findByTestId("ac-hint-agent_paused");
      expect(hint.className).toContain("ac-hint--stuck");
      expect(hint.textContent).toMatch(/需要你處理/);
      expect(hint.textContent).toMatch(/按「重試」或「跳過」/);
      fireEvent.click(within(hint).getByRole("button", { name: "先不提示" }));
      expect(screen.queryByTestId("ac-hint-agent_paused")).toBeNull();
      expect(stored().dismissedHints).toEqual([]);
      first.unmount();
      view();
      expect(await screen.findByTestId("ac-hint-agent_paused")).toBeTruthy();
    } finally {
      rect.mockRestore();
    }
  });

  it("「?」裡關掉情境提示就不再出現", async () => {
    setGuide({ hintsOff: false });
    const rect = vi
      .spyOn(Element.prototype, "getBoundingClientRect")
      .mockReturnValue({ left: 10, top: 10, right: 110, bottom: 60, width: 100, height: 50, x: 10, y: 10, toJSON: () => ({}) });
    try {
      vi.mocked(listAgents).mockResolvedValue([agent()]);
      vi.mocked(listTasks).mockResolvedValue([]);
      view();
      await screen.findByTestId("ac-hint-add_module");
      fireEvent.click(screen.getByRole("button", { name: "說明" }));
      const help = await screen.findByRole("dialog");
      fireEvent.click(within(help).getByLabelText("顯示情境提示"));
      fireEvent.click(within(help).getByRole("button", { name: "關閉" }));
      expect(screen.queryByTestId("ac-hint-add_module")).toBeNull();
      expect(stored().hintsOff).toBe(true);
    } finally {
      rect.mockRestore();
    }
  });
});

describe("交件摘要", () => {
  it("模組 chip 的提示顯示審核者寫的交件摘要", async () => {
    vi.mocked(listAgents).mockResolvedValue([agent()]);
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "m1", title: "計時器", card_kind: "work", agent_id: "a1", status: "done", outcome: "success", provides: "useTimer()，回傳剩餘秒數" }),
      card({ id: "m2", title: "統計頁", card_kind: "work", agent_id: "a1", status: "queued" }),
    ]);
    view();
    expect((await screen.findByTestId("chip-m1")).getAttribute("title")).toContain("\n提供：useTimer()，回傳剩餘秒數");
    expect(screen.getByTestId("chip-m2").getAttribute("title")).not.toContain("提供：");
  });
});

describe("專案簡介", () => {
  it("打開會載入現有內容，儲存送出去掉頭尾空白的內容，按鈕標上已寫", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(getProjectBrief).mockResolvedValue(null);
    view();
    const btn = await screen.findByTestId("ac-brief");
    expect(btn.textContent).not.toContain("✓");
    vi.mocked(getProjectBrief).mockResolvedValue("舊的簡介");
    fireEvent.click(btn);
    const dlg = await screen.findByRole("dialog", { name: "專案簡介" });
    const box = within(dlg).getByRole("textbox", { name: "專案簡介" }) as HTMLTextAreaElement;
    await waitFor(() => expect(box.value).toBe("舊的簡介"));
    fireEvent.change(box, { target: { value: "  番茄鐘 App，React＋Vite  " } });
    expect(within(dlg).getByText("22 字")).toBeTruthy();
    fireEvent.click(within(dlg).getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(setProjectBrief).toHaveBeenCalledWith("p1", "番茄鐘 App，React＋Vite"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "專案簡介" })).toBeNull());
    expect(screen.getByTestId("ac-brief").textContent).toContain("✓");
  });

  it("清空就刪除；超過 2000 字會提醒", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(getProjectBrief).mockResolvedValue("有內容");
    view();
    await waitFor(() => expect(screen.getByTestId("ac-brief").textContent).toContain("✓"));
    fireEvent.click(screen.getByTestId("ac-brief"));
    const dlg = await screen.findByRole("dialog", { name: "專案簡介" });
    const box = within(dlg).getByRole("textbox", { name: "專案簡介" }) as HTMLTextAreaElement;
    await waitFor(() => expect(box.value).toBe("有內容"));
    fireEvent.change(box, { target: { value: "字".repeat(2001) } });
    expect(within(dlg).getByText(/超過 2000 字了/)).toBeTruthy();
    fireEvent.change(box, { target: { value: "   " } });
    expect(within(dlg).queryByText(/超過 2000 字了/)).toBeNull();
    fireEvent.click(within(dlg).getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(setProjectBrief).toHaveBeenCalledWith("p1", null));
    await waitFor(() => expect(screen.getByTestId("ac-brief").textContent).not.toContain("✓"));
  });
});

describe("專案簡介的 AI 潤飾", () => {
  it("用選的模型把草稿改寫成簡介；可以還原；儲存的是改寫後的內容", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(getProjectBrief).mockResolvedValue(null);
    vi.mocked(listProviders).mockResolvedValue([
      { id: "p-a", display_name: "Provider A", provider_type: "anthropic", base_url: null, oauth_client_id: null, model: "m",
        supports_json_mode: true, has_api_key: true, is_default: true, auth_method: null },
    ]);
    vi.mocked(invokeAiComplete).mockResolvedValue({ content: "```\n目標：\n- 個人用番茄鐘\n技術選型：\n- React\n```" } as never);
    view();
    fireEvent.click(await screen.findByTestId("ac-brief"));
    const dlg = await screen.findByRole("dialog", { name: "專案簡介" });
    const box = within(dlg).getByRole("textbox", { name: "專案簡介" }) as HTMLTextAreaElement;
    await waitFor(() => expect(box).not.toBeDisabled());
    const refine = within(dlg).getByTestId("brief-refine");
    expect(refine).toBeDisabled(); // 沒有草稿不能潤飾
    fireEvent.change(box, { target: { value: "我要做番茄鐘 用react" } });
    await waitFor(() => expect(refine).not.toBeDisabled());
    fireEvent.click(refine);
    await waitFor(() => expect(box.value).toBe("目標：\n- 個人用番茄鐘\n技術選型：\n- React"));
    const [messages, system, , provider] = vi.mocked(invokeAiComplete).mock.calls[0];
    expect(messages[0].content).toContain("我要做番茄鐘 用react");
    expect(system).toMatch(/專案簡介/);
    expect(provider).toBe("p-a");

    fireEvent.click(within(dlg).getByTestId("brief-refine-undo"));
    expect(box.value).toBe("我要做番茄鐘 用react");
    fireEvent.click(refine);
    await waitFor(() => expect(box.value).toContain("個人用番茄鐘"));
    fireEvent.click(within(dlg).getByRole("button", { name: "儲存" }));
    await waitFor(() => expect(setProjectBrief).toHaveBeenCalledWith("p1", "目標：\n- 個人用番茄鐘\n技術選型：\n- React"));
  });

  it("AI 回空的或出錯：內容不動，顯示錯誤", async () => {
    vi.mocked(listAgents).mockResolvedValue([]);
    vi.mocked(listTasks).mockResolvedValue([]);
    vi.mocked(getProjectBrief).mockResolvedValue("原本的");
    vi.mocked(invokeAiComplete).mockResolvedValueOnce({ content: "  " } as never).mockRejectedValueOnce("network down");
    view();
    fireEvent.click(await screen.findByTestId("ac-brief"));
    const dlg = await screen.findByRole("dialog", { name: "專案簡介" });
    const box = within(dlg).getByRole("textbox", { name: "專案簡介" }) as HTMLTextAreaElement;
    await waitFor(() => expect(box.value).toBe("原本的"));
    fireEvent.click(within(dlg).getByTestId("brief-refine"));
    expect(await within(dlg).findByText("AI 沒有回覆任何內容，請再試一次")).toBeTruthy();
    expect(box.value).toBe("原本的");
    fireEvent.click(within(dlg).getByTestId("brief-refine"));
    expect(await within(dlg).findByText(/network down/)).toBeTruthy();
    expect(box.value).toBe("原本的");
    expect(within(dlg).queryByTestId("brief-refine-undo")).toBeNull();
  });
});
