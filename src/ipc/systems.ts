import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── 型別（鏡射 Rust systems/types.rs、approvals.rs、audit.rs、state.rs）──────

export type SystemMode = "read_only" | "approve_each" | "rules" | "autonomous";

export interface SystemRule {
  method: string;
  url_pattern: string;
  action: "allow" | "ask";
}

export interface SystemConnection {
  id: string;
  name: string;
  start_url: string;
  allowed_hosts: string[];
  mode: SystemMode;
  rules: SystemRule[];
  enabled: boolean;
}

export interface SystemStatus {
  running: boolean;
  manual: boolean;
  pending: number;
  needs_login: boolean;
}

export interface PendingApproval {
  id: string;
  system_id: string;
  intent: string;
  method: string;
  url: string;
  declared_read: boolean;
  ts_ms: number;
}

export interface AuditRow {
  id: number;
  system_id: string;
  tool: string;
  intent: string;
  method: string | null;
  url: string | null;
  verdict: "allow" | "deny" | "approved" | "rejected" | "timeout" | "manual";
  source: string;
  ts_ms: number;
}

export interface RecipeStep {
  tool: string;
  args: unknown;
  intent: string;
}

export interface Recipe {
  id: number;
  system_id: string;
  name: string;
  description: string;
  params: { name: string; example: string }[];
  steps: RecipeStep[];
  ok_count: number;
  fail_count: number;
  last_run_ms: number | null;
}

export const systemsList = () => invoke<SystemConnection[]>("systems_list");
export const systemsSave = (conn: SystemConnection) => invoke<void>("systems_save", { conn });
export const systemsRemove = (id: string) => invoke<void>("systems_remove", { id });
export const systemsOpenLogin = (id: string) => invoke<void>("systems_open_login", { id });
export const systemsStatus = (id: string) => invoke<SystemStatus>("systems_status", { id });
export const systemsPending = () => invoke<PendingApproval[]>("systems_pending");
export const systemsDecide = (approvalId: string, approve: boolean) =>
  invoke<boolean>("systems_decide", { approvalId, approve });
export const systemsStop = (id: string) => invoke<number>("systems_stop", { id });
export const systemsTakeover = (id: string) => invoke<void>("systems_takeover", { id });
export const systemsHandback = (id: string) => invoke<void>("systems_handback", { id });
export const systemsAudit = (id: string, limit?: number) => invoke<AuditRow[]>("systems_audit", { id, limit });
export const systemsMcpConfig = () => invoke<string>("systems_mcp_config");

// ── 事件 ───────────────────────────────────────────────────────────────────

export const onSystemsNeedApproval = (cb: (a: PendingApproval) => void): Promise<UnlistenFn> =>
  listen<{ id: string; pending: Omit<PendingApproval, "id"> }>("systems://need-approval", (e) =>
    cb({ id: e.payload.id, ...e.payload.pending }),
  );
export const onSystemsResolved = (cb: (id: string) => void): Promise<UnlistenFn> =>
  listen<{ id: string }>("systems://resolved", (e) => cb(e.payload.id));
export const onSystemsState = (cb: (systemId: string) => void): Promise<UnlistenFn> =>
  listen<{ system_id: string }>("systems://state", (e) => cb(e.payload.system_id));
export const onSystemsDenied = (cb: () => void): Promise<UnlistenFn> => listen("systems://denied", () => cb());

export const systemsRecipes = (id: string) => invoke<Recipe[]>("systems_recipes", { id });
export const systemsRecipeDelete = (recipeId: number) => invoke<boolean>("systems_recipe_delete", { recipeId });
