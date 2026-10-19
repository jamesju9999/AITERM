import { invoke } from "@tauri-apps/api/core";

import type { MergeOutcome } from "./tasks";

// ── Types（鏡射 Rust tasks/agents.rs 的 AgentRow）──────────────────────────

export type AgentKind = "coder" | "reviewer" | "orchestrator" | "verifier";

export interface AgentRow {
  id: string;
  kind: AgentKind;
  name: string;
  /** 角色預設的名稱，純顯示；實際影響派工的是 prompt_prefix。 */
  role: string;
  prompt_prefix: string;
  /** 這個 Agent 負責的 git repo，建立後不可改。 */
  project_dir: string;
  canvas_x: number;
  canvas_y: number;
  /** 第一次派工時才會有。 */
  worktree_path: string | null;
  branch: string | null;
  /** 非 null = 暫停中，指向造成暫停的那張卡。 */
  paused_task_id: string | null;
  created_at: string;
  last_merge_signature: string | null;
  /** Coder：產出規格。 */
  produces_spec: boolean;
  /** 派工時的 `--model`。null = 沿用 claude 的預設。 */
  model?: string | null;
  /** 用哪個 AI 工具做事；舊資料沒有 = claude。 */
  cli?: AgentCli;
  /** 資料庫技能（JSON 字串，見 DbSkill）。null／沒有 = 沒裝。 */
  db_skill?: string | null;
  /** 自訂技能：技能資料夾路徑的 JSON 陣列。null／沒有 = 沒有。 */
  custom_skills?: string | null;
}

export type AgentCli = "claude" | "codex";

export interface NewAgentArgs {
  kind: AgentKind;
  name: string;
  role: string;
  prompt_prefix: string;
  project_dir: string;
  canvas_x: number;
  canvas_y: number;
  produces_spec?: boolean;
  /** 空字串或省略 = 沿用預設。 */
  model?: string;
  cli?: AgentCli;
}

export interface UpdateAgentArgs {
  id: string;
  name: string;
  role: string;
  prompt_prefix: string;
  canvas_x: number;
  canvas_y: number;
  /** 省略 = 不改（例如只是拖動位置）。 */
  produces_spec?: boolean;
  /** 省略 = 不改；空字串 = 清掉（沿用預設）。 */
  model?: string;
  /** 省略 = 不改。 */
  cli?: AgentCli;
}

// ── Commands ────────────────────────────────────────────────────────────────

export const listAgents = (projectId: string): Promise<AgentRow[]> =>
  invoke("agents_list", { projectId });

export const createAgent = (projectId: string, args: NewAgentArgs): Promise<string> =>
  invoke("agents_create", { projectId, args });

export const updateAgent = (projectId: string, args: UpdateAgentArgs): Promise<void> =>
  invoke("agents_update", { projectId, args });

export const deleteAgent = (projectId: string, id: string): Promise<void> =>
  invoke("agents_delete", { projectId, id });

/** 暫停中的 Agent：重試失敗的那張，或跳過它（半成品會被 git stash）。 */
export const resolveAgent = (projectId: string, id: string, action: "retry" | "skip"): Promise<void> =>
  invoke("agents_resolve", { projectId, id, action });

/** 把 Agent 的分支合併回它程式庫目前的分支。worktree 與分支保留。 */
export const mergeAgent = (projectId: string, id: string): Promise<MergeOutcome> =>
  invoke("agents_merge", { projectId, id });

export const abortAgentMerge = (projectId: string, id: string): Promise<void> =>
  invoke("agents_abort_merge", { projectId, id });

/** `agentId` 為 null 時取消指派（卡片退回計畫中）。 */
export const assignAgent = (projectId: string, taskId: string, agentId: string | null): Promise<void> =>
  invoke("tasks_assign_agent", { projectId, taskId, agentId });

/** 手動放行卡住的模組（審查不通過超過上限，或審查卡本身失敗）。 */
export const overrideModule = (projectId: string, taskId: string): Promise<void> =>
  invoke("agents_override_module", { projectId, taskId });

/** 程式庫可不可以給 Agent 用（見 tasks/repo_check.rs）。 */
export type RepoCheck =
  | { status: "ok"; branch: string | null }
  | { status: "missing" }
  | { status: "not_repo" }
  | { status: "no_commits" }
  | { status: "board_folder" };

export const checkRepo = (path: string): Promise<RepoCheck> => invoke("agents_check_repo", { path });

/** git init（必要時）＋把現有檔案建成第一個 commit。 */
/** `projectId`：看板專案資料夾在這個程式庫裡面時，第一個 commit 不收進它。 */
export const initRepo = (path: string, projectId?: string): Promise<void> =>
  invoke("agents_init_repo", { path, projectId: projectId ?? null });

/** 在系統的檔案管理員打開程式庫資料夾。 */
export const openRepo = (path: string): Promise<void> => invoke("agents_open_repo", { path });

// ── 驗收（鏡射 Rust tasks/workflow.rs 的 VerifyReport）─────────────────────

export interface VerifyItem {
  criterion: string;
  status: "pass" | "fail" | "manual";
  module_id?: string | null;
  /** 截圖路徑（相對於驗收卡資料夾）或指令輸出摘要。 */
  evidence: string[];
  steps?: string | null;
  expected?: string | null;
  actual?: string | null;
  notes?: string | null;
}

export interface VerifyReport {
  verdict: "pass" | "fail";
  project_type?: string | null;
  items: VerifyItem[];
  notes?: string | null;
}

export interface VerifyReportView {
  /** 驗收卡還沒寫出 verify.json 時是 null。 */
  report: VerifyReport | null;
  /** 證據路徑 → data URL（只有圖片）。 */
  images: Record<string, string>;
}

export const verifyReport = (projectId: string, taskId: string): Promise<VerifyReportView> =>
  invoke("agents_verify_report", { projectId, taskId });

/** 放行沒通過的驗收：當成通過，程式庫更新到受驗版本。 */
export const acceptVerification = (projectId: string, taskId: string): Promise<void> =>
  invoke("agents_accept_verification", { projectId, taskId });

/** Claude Code／Codex／git 找到的路徑（null = 沒裝）。見 tasks/tools_check.rs。 */
export interface ToolsCheck {
  platform: "macos" | "windows" | "linux";
  claude_command: string;
  claude: string | null;
  codex: string | null;
  git: string | null;
}

export const checkTools = (): Promise<ToolsCheck> => invoke("agents_check_tools");

// ── 資料庫技能（見 docs/superpowers/specs/2026-10-19-agent-db-skill-design.md）──

export type DbSkillMode = "read" | "approve" | "write";

export interface DbSkill {
  connections: string[];
  mode: DbSkillMode;
}

/** 一筆 Agent 的資料庫查詢（tasks/db_skill.rs 的 DbQueryRow）。 */
export interface DbQueryRow {
  id: number;
  task_id: string;
  agent_id: string | null;
  connection_id: string;
  connection_name: string;
  sql: string;
  started_at: number;
  finished_at: number | null;
  rows: number | null;
  ms: number | null;
  error: string | null;
  decision: "allowed" | "pending" | "approved" | "denied";
}

/** 裝上、修改或移除（null）Agent 的資料庫技能。 */
export const setDbSkill = (projectId: string, agentId: string, skill: DbSkill | null): Promise<void> =>
  invoke("agents_set_db_skill", { projectId, agentId, skill });

export const listDbQueries = (projectId: string): Promise<DbQueryRow[]> => invoke("agents_db_queries", { projectId });

// ── 參考資料夾（整個團隊都能讀、不能寫，見 tasks/references.rs）──

export interface Reference {
  path: string;
  /** 一句說明，會寫進派工提示。 */
  note: string;
}

export const getReferences = (projectId: string): Promise<Reference[]> => invoke("agents_get_references", { projectId });

/** 整份取代；回傳整理後的清單（去重複、去空白）。資料夾不存在會回錯誤。 */
export const setReferences = (projectId: string, references: Reference[]): Promise<Reference[]> =>
  invoke("agents_set_references", { projectId, references });

// ── 自訂技能（見 tasks/custom_skills.rs）──

export interface SkillInfo {
  name: string;
  description: string;
  /** 技能資料夾（含 SKILL.md）的絕對路徑。 */
  path: string;
  source: "repo" | "user" | "folder" | "agent";
}

export interface SkillUseRow {
  id: number;
  task_id: string;
  agent_id: string | null;
  skill: string;
  at: number;
}

/** 程式庫的 .claude/skills 與使用者的 ~/.claude/skills。 */
export const listSkills = (repoDir: string): Promise<SkillInfo[]> => invoke("agents_list_skills", { repoDir });
/** 使用者自己選的資料夾（要有 SKILL.md）。 */
export const readSkill = (path: string): Promise<SkillInfo> => invoke("agents_read_skill", { path });
export const setCustomSkills = (projectId: string, agentId: string, paths: string[]): Promise<void> =>
  invoke("agents_set_custom_skills", { projectId, agentId, paths });
export const listSkillUses = (projectId: string): Promise<SkillUseRow[]> => invoke("agents_skill_uses", { projectId });
