import { invoke } from "@tauri-apps/api/core";

import type { AgentCli, AgentKind } from "./agents";

/**
 * 團隊編制：一個專案可以存好幾組團隊，依階段切換。
 * 見 docs/superpowers/specs/2026-10-20-agent-canvas-team-formations-design.md。
 */
export interface FormationAgent {
  kind: AgentKind;
  name: string;
  role: string;
  prompt_prefix: string;
  model: string;
  cli: AgentCli;
  produces_spec: boolean;
  /** 桌布上的位置。 */
  dx: number;
  dy: number;
  db_skill?: string | null;
  custom_skills?: string | null;
}

export interface FormationSummary {
  id: string;
  name: string;
  agent_count: number;
}

export interface FormationLogRow {
  at: number;
  from_name: string | null;
  to_name: string;
}

export interface FormationsState {
  formations: FormationSummary[];
  active: string | null;
  /** 等目前的工作做完後要切換過去的編制。 */
  pending: string | null;
  /** 還在工作、擋住切換的 Agent 名稱。 */
  busy: string[];
  log: FormationLogRow[];
}

export type SwitchOutcome = { status: "switched" } | { status: "busy"; names: string[] };

/** 已經從桌布移除的 Agent：已完成卡片的歷史還指向它。 */
export interface RetiredAgent {
  id: string;
  name: string;
  kind: AgentKind;
}

/** 舊專案（有 Agent、沒有編制）第一次讀取時用目前的團隊建立 `defaultName`。 */
export const formationsState = (projectId: string, defaultName: string): Promise<FormationsState> =>
  invoke("formations_state", { projectId, defaultName });
export const createFormation = (projectId: string, name: string, agents: FormationAgent[]): Promise<string> =>
  invoke("formations_create", { projectId, name, agents });
/** 把目前桌布另存成新編制並設為目前。 */
export const saveFormationAs = (projectId: string, name: string): Promise<string> => invoke("formations_save_as", { projectId, name });
export const renameFormation = (projectId: string, id: string, name: string): Promise<void> =>
  invoke("formations_rename", { projectId, id, name });
export const deleteFormation = (projectId: string, id: string): Promise<void> => invoke("formations_delete", { projectId, id });
export const switchFormation = (projectId: string, id: string): Promise<SwitchOutcome> => invoke("formations_switch", { projectId, id });
/** 做完後切換。 */
export const requestFormationSwitch = (projectId: string, id: string): Promise<void> =>
  invoke("formations_request_switch", { projectId, id });
export const cancelFormationSwitch = (projectId: string): Promise<void> => invoke("formations_cancel_switch", { projectId });
export const listRetiredAgents = (projectId: string): Promise<RetiredAgent[]> => invoke("agents_retired", { projectId });
