import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface VcsTaskConfig {
  repo_url: string;
  base_branch: string;
  work_branch: string;
}

export interface TaskPacket {
  task_id: string;
  mission_id: string;
  title: string;
  description: string;
  spec_path?: string;
  vcs: VcsTaskConfig;
  ai_provider_id: string;
  execution_mode: string;
  max_steps: number;
  on_complete: unknown;
  vcs_token: string;
  vcs_token_expires_at: string;
}

export interface TaskReadyPayload {
  task_id: string;
  mission_id: string;
  title: string;
  description: string;
  spec_content?: string;
  repo_dir: string;
  work_branch: string;
  ai_provider_id: string;
  execution_mode: string;
  max_steps: number;
  on_complete: unknown;
}

export interface SkillInstalledPayload {
  skill_id: string;
  version: string;
  content: string;
}

export function enterpriseAcceptTask(packet: TaskPacket): Promise<void> {
  return invoke("enterprise_accept_task", { packet });
}

export function enterpriseRejectTask(taskId: string): Promise<void> {
  return invoke("enterprise_reject_task", { taskId });
}

export function enterpriseUpdateTaskProgress(
  taskId: string,
  stepsDone: number,
  stepsTotal: number,
): Promise<void> {
  return invoke("enterprise_update_task_progress", { taskId, stepsDone, stepsTotal });
}

export function enterpriseCompleteTask(taskId: string): Promise<void> {
  return invoke("enterprise_complete_task", { taskId });
}

export function enterpriseOnComplete(args: {
  taskId: string;
  repoDir: string;
  workBranch: string;
  onComplete: unknown;
}): Promise<string> {
  return invoke("enterprise_on_complete", {
    taskId: args.taskId,
    repoDir: args.repoDir,
    workBranch: args.workBranch,
    onComplete: args.onComplete,
  });
}

export function enterpriseRegisterDevice(args: {
  serverUrl: string;
  deviceName: string;
  deviceType: string;
  role: string;
}): Promise<string> {
  return invoke("enterprise_register_device", {
    serverUrl: args.serverUrl,
    deviceName: args.deviceName,
    deviceType: args.deviceType,
    role: args.role,
  });
}

export function enterpriseInstallService(install: boolean): Promise<string> {
  return invoke("enterprise_install_service", { install });
}

export function onEnterpriseTaskReceived(
  cb: (packet: TaskPacket) => void,
): Promise<UnlistenFn> {
  return listen<TaskPacket>("enterprise:task-received", (e) => cb(e.payload));
}

export function onEnterpriseTaskReady(
  cb: (payload: TaskReadyPayload) => void,
): Promise<UnlistenFn> {
  return listen<TaskReadyPayload>("enterprise:task-ready", (e) => cb(e.payload));
}

export function onEnterpriseSkillInstalled(
  cb: (payload: SkillInstalledPayload) => void,
): Promise<UnlistenFn> {
  return listen<SkillInstalledPayload>("enterprise:skill-installed", (e) => cb(e.payload));
}

export function onEnterpriseAuthError(cb: () => void): Promise<UnlistenFn> {
  return listen("enterprise:auth-error", () => cb());
}
