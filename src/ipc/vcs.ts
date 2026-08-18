import { invoke } from "@tauri-apps/api/core";

export type VcsType = "git" | "svn";
export type VcsWriteMode = "read_only" | "guarded" | "full_auto";

export interface VcsConnectionInfo {
  id: string;
  name: string;
  vcs_type: VcsType;
  url?: string | null;
  username?: string | null;
  write_mode: VcsWriteMode;
  has_secret: boolean;
}

export interface GitBlockInfo {
  branch: string;
  insertions: number;
  deletions: number;
}

export interface VcsConnectionInput {
  id?: string;
  name: string;
  vcs_type: VcsType;
  url?: string | null;
  username?: string | null;
  secret?: string | null;
  write_mode: VcsWriteMode;
}

export interface VcsRepoInfo {
  vcs_type: VcsType;
  root: string;
  remote_url?: string | null;
  connection_id?: string | null;
}

export interface CommitEntry {
  revision: string;
  author: string;
  date: string;
  message: string;
  files_changed: string[];
}

export interface BranchEntry {
  name: string;
  is_current: boolean;
  is_remote: boolean;
}

export interface PrEntry {
  number: number;
  title: string;
  author: string;
  state: string;
  url: string;
  updated_at: string;
}

export interface WorkflowRun {
  id: number;
  name: string;
  status: string;
  conclusion?: string | null;
  created_at: string;
  html_url: string;
}

export interface IssueEntry {
  number: number;
  title: string;
  author: string;
  state: string;
  url: string;
  created_at: string;
}

export interface BlameEntry {
  line_number: number;
  revision: string;
  author: string;
  date: string;
  content: string;
}

export interface ActiveFeature {
  number: number;
  title: string;
  author: string;
  draft: boolean;
  url: string;
  updated_at: string;
  head_ref: string;
  base_ref: string;
  files: string[];
}

export interface StartFeatureOutcome {
  branch_name: string;
  pr_number: number;
  pr_url: string;
}

export type VcsResult =
  | { type: "log"; commits: CommitEntry[]; truncated: boolean }
  | { type: "diff"; content: string; revision: string }
  | { type: "blame"; lines: BlameEntry[] }
  | { type: "branches"; branches: BranchEntry[] }
  | { type: "pr_list"; prs: PrEntry[] }
  | { type: "issue_list"; issues: IssueEntry[] }
  | { type: "actions_list"; runs: WorkflowRun[] }
  | { type: "write_confirm"; operation: string; preview: string; intent: unknown }
  | { type: "write_success"; operation: string; detail: string }
  | { type: "error"; message: string }
  | { type: "no_token"; required_level: number }
  | { type: "svn_not_installed" };

export function vcsListConnections(): Promise<VcsConnectionInfo[]> {
  return invoke("vcs_list_connections");
}

export function vcsAddConnection(input: VcsConnectionInput): Promise<string> {
  return invoke("vcs_add_connection", { input });
}

export function vcsUpdateConnection(input: VcsConnectionInput): Promise<void> {
  return invoke("vcs_update_connection", { input });
}

export function vcsRemoveConnection(id: string): Promise<void> {
  return invoke("vcs_remove_connection", { id });
}

export function vcsTestConnection(input: VcsConnectionInput): Promise<string> {
  return invoke("vcs_test_connection", { input });
}

export function vcsDetectRepo(path: string): Promise<VcsRepoInfo> {
  return invoke("vcs_detect_repo", { path });
}

export function vcsQuery(query: string, repoInfo: VcsRepoInfo, sessionId: string): Promise<VcsResult> {
  return invoke("vcs_query", { query, repoInfo, sessionId });
}

export function vcsListActiveFeatures(repoInfo: VcsRepoInfo): Promise<ActiveFeature[]> {
  return invoke<ActiveFeature[]>("vcs_list_active_features", { repoInfo });
}

export function vcsCheckOverlap(repoInfo: VcsRepoInfo, files: string[]): Promise<ActiveFeature[]> {
  return invoke<ActiveFeature[]>("vcs_check_overlap", { repoInfo, files });
}

export function vcsGetDefaultBranch(repoInfo: VcsRepoInfo): Promise<string> {
  return invoke<string>("vcs_get_default_branch", { repoInfo });
}

export function vcsStartFeature(
  repoInfo: VcsRepoInfo,
  featureName: string,
  baseBranch: string,
  declaredFiles: string[],
): Promise<StartFeatureOutcome> {
  return invoke<StartFeatureOutcome>("vcs_start_feature", {
    repoInfo,
    featureName,
    baseBranch,
    declaredFiles,
  });
}

export function vcsFinishFeature(repoInfo: VcsRepoInfo, prNumber: number): Promise<void> {
  return invoke<void>("vcs_finish_feature", { repoInfo, prNumber });
}

export function vcsGetFeatureDiff(repoInfo: VcsRepoInfo, base: string, head: string): Promise<string> {
  return invoke<string>("vcs_get_feature_diff", { repoInfo, base, head });
}

export function vcsMergeFeature(
  repoInfo: VcsRepoInfo,
  prNumber: number,
  branchToDelete?: string | null,
): Promise<void> {
  return invoke<void>("vcs_merge_feature", { repoInfo, prNumber, branchToDelete: branchToDelete ?? null });
}

// ── Agent loop types ─────────────────────────────────────────────────────────

export type VcsAgentHistoryEntry =
  | { role: "user"; text: string }
  | { role: "step"; step_num: number; operation: string; result_json: string; summary: string };

export interface VcsAgentDecision {
  done: boolean;
  intent?: unknown; // VcsIntent JSON — passed directly to vcsQuery or null
  summary: string;
  final_answer?: string | null;
}

export function vcsAgentStep(
  goal: string,
  history: VcsAgentHistoryEntry[],
  repoInfo: VcsRepoInfo,
  sessionId: string,
  providerId?: string | null,
): Promise<VcsAgentDecision> {
  return invoke("vcs_agent_step", {
    goal,
    history,
    repoInfo,
    sessionId,
    providerId: providerId ?? null,
  });
}

export function vcsAgentAbortStep(sessionId: string): Promise<void> {
  return invoke("vcs_agent_abort_step", { sessionId });
}

export function pickFolder(): Promise<string | null> {
  return invoke<string | null>("pick_folder");
}

export function getGitBlockInfo(cwd: string): Promise<GitBlockInfo | null> {
  return invoke("vcs_get_block_info", { cwd });
}
