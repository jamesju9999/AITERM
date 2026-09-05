import { invoke } from "@tauri-apps/api/core";

// ── Types（鏡射 Rust commands/projects.rs）──────────────────────────────

/** 專案資料夾目前的可用狀態。非 `ok` 時 counts 全為 0。 */
export type ProjectStatus = "ok" | "missing" | "invalid" | "incompatible";

export interface ProjectCounts {
  planning: number;
  queued: number;
  running: number;
  done: number;
}

export interface ProjectInfo {
  id: string;
  name: string;
  description: string;
  path: string;
  status: ProjectStatus;
  counts: ProjectCounts;
  error: string | null;
}

// ── Commands ───────────────────────────────────────────────────────────

export const listProjects = (): Promise<ProjectInfo[]> => invoke("projects_list");

export const createProject = (args: {
  parentDir: string;
  name: string;
  description: string;
}): Promise<string> =>
  invoke("projects_create", {
    args: { parent_dir: args.parentDir, name: args.name, description: args.description },
  });

/** 開啟既有專案。這同時就是「匯入」——別台機器複製過來的資料夾，挑它的 .aitprj。 */
export const openProject = (aitprjPath: string): Promise<string> =>
  invoke("projects_open", { aitprjPath });

/** `deleteFolder` 為 true 時連同磁碟資料夾一起刪除，且無法復原。 */
export const removeProject = (id: string, deleteFolder: boolean): Promise<void> =>
  invoke("projects_remove", { args: { id, delete_folder: deleteFolder } });

export const renameProject = (id: string, name: string, description: string): Promise<void> =>
  invoke("projects_rename", { args: { id, name, description } });

/** 這個專案的卡片用過的工作目錄，供新增工作時快捷選取。 */
export const usedDirs = (projectId: string): Promise<string[]> =>
  invoke("tasks_used_dirs", { projectId });
