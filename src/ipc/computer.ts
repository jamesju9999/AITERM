import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 與 Rust `computer::approval::APPROVAL_REQUEST_EVENT` 相同。 */
export const COMPUTER_APPROVAL_EVENT = "computer://approval-request";

export type ApprovalKind = "first_use" | "destructive" | "command";
export type ApprovalDecision = "allow_once" | "allow_always" | "deny";

export interface ComputerApprovalRequest {
  id: string;
  kind: ApprovalKind;
  app: string;
  action: string;
  detail: string;
}

export interface ComputerStatus {
  enabled: boolean;
  accessibility: boolean;
  screen_recording: boolean;
  allowed_apps: string[];
}

export interface ComputerAuditEntry {
  ts: number;
  tool: string;
  app: string;
  detail: string;
  outcome: string;
}

export function onComputerApproval(cb: (req: ComputerApprovalRequest) => void): Promise<UnlistenFn> {
  return listen<ComputerApprovalRequest>(COMPUTER_APPROVAL_EVENT, (e) => cb(e.payload));
}

/** 回傳 false 代表請求已逾時（後端早已視為拒絕）。 */
export const resolveComputerApproval = (id: string, decision: ApprovalDecision): Promise<boolean> =>
  invoke("computer_resolve_approval", { id, decision });

export const computerStatus = (): Promise<ComputerStatus> => invoke("computer_status");

export const computerRevokeApp = (key: string): Promise<void> => invoke("computer_revoke_app", { key });

export const computerAuditTail = (limit?: number): Promise<ComputerAuditEntry[]> =>
  invoke("computer_audit_tail", { limit });

export const computerOpenPrivacySettings = (pane: "accessibility" | "screen"): Promise<void> =>
  invoke("computer_open_privacy_settings", { pane });

export interface TerminalCommandGateResult {
  allowed: boolean;
  /** 不允許時的原因（要回給模型看）。 */
  reason: string | null;
}

/** 對話模式的 run_command 工具在執行前呼叫：依執行模式與指令風險決定直接跑、問使用者、或拒絕。 */
export const terminalCommandGate = (command: string): Promise<TerminalCommandGateResult> =>
  invoke("terminal_command_gate", { command });
