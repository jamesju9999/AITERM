import { invoke } from "@tauri-apps/api/core";

/** 共用看板專案的驗收設定：有沒有開「由 server 執行驗收」、畫布上有沒有驗收者。本機專案回 null。 */
export interface VerifySettings {
  enabled: boolean;
  has_verifier: boolean;
}

export const getVerify = (projectId: string): Promise<VerifySettings | null> => invoke("tasks_get_verify", { projectId });
/** 開關驗收（管理員限定，由 server 判斷；沒有驗收者或工作資料夾時 server 會拒絕並說明原因）。 */
export const setVerify = (projectId: string, enabled: boolean): Promise<void> => invoke("tasks_set_verify", { projectId, enabled });
/** 重新驗收一張沒通過的驗收卡（管理員限定）。 */
export const retryVerification = (taskId: string): Promise<void> => invoke("tasks_retry_verification", { taskId });

/** 禁止清單的一條規則（Claude Code 的 `Bash(命令:*)` 比對式）。 */
export interface DenyRule {
  rule: string;
  note: string;
  enabled: boolean;
}

/** 專案層的覆寫：`add` 新增這個專案專屬的規則，`remove` 把全域的某一條在這個專案移除。 */
export interface DenyOverride {
  rule: string;
  kind: "add" | "remove";
  enabled: boolean;
}

export interface ProjectDeny {
  global: DenyRule[];
  overrides: DenyOverride[];
  /** 這個專案這次驗收實際生效的規則。 */
  effective: string[];
}

export const getGlobalDeny = (): Promise<DenyRule[]> => invoke("verify_deny_global_get");
export const setGlobalDeny = (rules: DenyRule[]): Promise<DenyRule[]> => invoke("verify_deny_global_set", { rules });
export const resetGlobalDeny = (): Promise<DenyRule[]> => invoke("verify_deny_global_reset");
export const getProjectDeny = (projectId: string): Promise<ProjectDeny> => invoke("verify_deny_project_get", { projectId });
export const setProjectDeny = (projectId: string, overrides: DenyOverride[]): Promise<ProjectDeny> => invoke("verify_deny_project_set", { projectId, overrides });
