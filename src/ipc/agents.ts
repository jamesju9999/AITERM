import { invoke } from "@tauri-apps/api/core";

import type { MergeOutcome } from "./tasks";

// ── Types（鏡射 Rust tasks/agents.rs 的 AgentRow）──────────────────────────

export type AgentKind = "coder" | "reviewer" | "orchestrator";

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
}

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
  | { status: "no_commits" };

export const checkRepo = (path: string): Promise<RepoCheck> => invoke("agents_check_repo", { path });

/** git init（必要時）＋把現有檔案建成第一個 commit。 */
/** `projectId`：看板專案資料夾在這個程式庫裡面時，第一個 commit 不收進它。 */
export const initRepo = (path: string, projectId?: string): Promise<void> =>
  invoke("agents_init_repo", { path, projectId: projectId ?? null });

/** 在系統的檔案管理員打開程式庫資料夾。 */
export const openRepo = (path: string): Promise<void> => invoke("agents_open_repo", { path });
