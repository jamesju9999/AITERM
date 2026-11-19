import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── 型別（鏡射 Rust systems/types.rs、approvals.rs、audit.rs、state.rs）──────

export type SystemKind = "web" | "desktop";

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
  /** AI 操作這個系統用的模型；null／沒有 = 沿用 Claude Code 的預設。 */
  model?: string | null;
  /** AI 可以從這些資料夾挑檔案上傳；空 = 不能上傳。 */
  upload_dirs?: string[];
  /** 連線種類；沒有這個欄位 = 網頁。 */
  kind?: SystemKind;
  /** 桌面 App 類型：綁定的 App（名稱或套件 ID）。 */
  app?: string;
  /** 桌面 App 類型：App 沒開時由 AITerm 啟動。 */
  launch_app?: boolean;
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
export interface SystemLoginInfo {
  username: string;
  /** 有沒有存密碼；密碼本身不會回傳。 */
  has_password: boolean;
}
export const systemsLoginGet = (id: string) => invoke<SystemLoginInfo>("systems_login_get", { id });
/** 存自動登入的帳密。password 空字串／null = 密碼不變；username 留空 = 清除。 */
export const systemsLoginSet = (id: string, username: string, password: string | null) =>
  invoke<void>("systems_login_set", { id, username, password });
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

// ── 定時任務 ───────────────────────────────────────────────────────────────

export type JobKind = "daily" | "weekly" | "interval";

export interface JobSpec {
  times: string[];
  /** 1=週一 … 7=週日。 */
  days: number[];
  every_minutes: number;
  window?: { from: string; to: string } | null;
}

export interface SystemJob {
  id: string;
  system_id: string;
  name: string;
  goal: string;
  kind: JobKind;
  spec_json: string;
  enabled: boolean;
  created_ms: number;
  last_fired_ms: number;
  fail_streak: number;
  disabled_reason: string | null;
  /** 這個任務用的模型；null = 沿用系統的預設。 */
  model?: string | null;
}

export interface JobRun {
  id: number;
  job_id: string;
  started_ms: number;
  finished_ms: number | null;
  status: "running" | "ok" | "failed" | "timeout" | "skipped";
  summary: string;
}

export interface NewJob {
  system_id: string;
  name: string;
  goal: string;
  kind: JobKind;
  spec: JobSpec;
  model?: string | null;
}

export const systemsJobs = (systemId?: string) => invoke<SystemJob[]>("systems_jobs", { systemId });
export const systemsJobSave = (job: NewJob, id?: string) => invoke<string>("systems_job_save", { id, job });
export const systemsJobDelete = (id: string) => invoke<void>("systems_job_delete", { id });
export const systemsJobSetEnabled = (id: string, enabled: boolean) => invoke<void>("systems_job_set_enabled", { id, enabled });
export const systemsJobRunNow = (id: string) => invoke<void>("systems_job_run_now", { id });
export const systemsJobRuns = (id: string) => invoke<JobRun[]>("systems_job_runs", { id });

export const onSystemsJobFinished = (cb: () => void): Promise<UnlistenFn> => listen("systems://job-finished", () => cb());
export const onSystemsJobDisabled = (cb: (name: string) => void): Promise<UnlistenFn> =>
  listen<{ name: string }>("systems://job-disabled", (e) => cb(e.payload.name));

// ── 請 AI 操作／存操作流程 ─────────────────────────────────────────────────────

export interface StepView {
  tool: string;
  intent: string;
  detail: string;
  write: boolean;
}

export interface AdhocReport {
  status: "ok" | "failed" | "timeout";
  summary: string;
  steps: StepView[];
  /** 系統猜到的參數候選（輸入的文字、下拉選的值）。 */
  params: { name: string; example: string }[];
  can_save: boolean;
}

export const systemsRunGoal = (systemId: string, goal: string, model?: string | null) =>
  invoke<AdhocReport>("systems_run_goal", { systemId, goal, model: model || null });
export const systemsRecipeSaveFromJournal = (systemId: string, name: string, description: string, params: { name: string; example: string }[]) =>
  invoke<string>("systems_recipe_save_from_journal", { systemId, name, description, params });
export const systemsJournalDiscard = (systemId: string) => invoke<void>("systems_journal_discard", { systemId });
export const systemsRecipeRun = (systemId: string, name: string, params: Record<string, string>) =>
  invoke<string>("systems_recipe_run", { systemId, name, params });
