import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type IntakeStatus = "pending" | "queued" | "running" | "draft" | "done" | "rejected";
export type Level = "internal" | "external";
export type MatchMode = "alias" | "subject_tag" | "alias_or_tag" | "dedicated";
export type DispatchMode = "auto" | "manual";

export interface IntakeItem {
  id: string;
  source: string;
  sender: string;
  sender_level: Level;
  subject: string;
  body: string;
  message_id: string;
  status: IntakeStatus;
  tab_id: string | null;
  reply_draft: string | null;
  error: string | null;
  created_at: number;
  updated_at: number;
  kind: "question" | "code_change" | null;
  snapshot_head: string | null;
  verification: string | null;
  verified: boolean | null;
}

export interface IntakeConfig {
  enabled: boolean;
  account_id: string;
  mode: MatchMode;
  alias: string;
  tag: string;
  auth_server: string;
  internal_without_auth: boolean;
  dispatch_internal: DispatchMode;
  dispatch_external: DispatchMode;
  reply_internal_auto: boolean;
  reply_external_auto: boolean;
  rate_limit_per_hour: number;
  verify_command: string;
  verify_timeout_secs: number;
  telegram_enabled: boolean;
  web_mode: "off" | "local" | "lan";
  web_port: number;
}

export interface IntakeSender {
  address: string;
  level: Level;
  skip_auth: boolean;
}

export const INTAKE_CHANGED_EVENT = "intake://changed";

export const intakeGetConfig = () => invoke<IntakeConfig>("intake_get_config");
export const intakeSetConfig = (config: IntakeConfig) => invoke<void>("intake_set_config", { config });
export const intakeList = (status?: IntakeStatus) => invoke<IntakeItem[]>("intake_list", { status: status ?? null });
export const intakeSenders = () => invoke<IntakeSender[]>("intake_senders");
export const intakeUpsertSender = (sender: IntakeSender) => invoke<void>("intake_upsert_sender", { sender });
export const intakeDeleteSender = (address: string) => invoke<boolean>("intake_delete_sender", { address });
export const intakeDropped = () => invoke<[string, number][]>("intake_dropped");
export const intakeApprove = (id: string) => invoke<boolean>("intake_approve", { id });
export const intakeReject = (id: string) => invoke<boolean>("intake_reject", { id });
export const intakeUpdateDraft = (id: string, draft: string) => invoke<boolean>("intake_update_draft", { id, draft });
export const intakeSendReply = (id: string) => invoke<void>("intake_send_reply", { id });
export const intakeTabStart = (tabId: string) => invoke<void>("intake_tab_start", { tabId });
export const intakeTabStop = (tabId: string) => invoke<void>("intake_tab_stop", { tabId });
export const intakeTabs = () => invoke<string[]>("intake_tabs");

export function onIntakeChanged(cb: () => void): Promise<UnlistenFn> {
  return listen(INTAKE_CHANGED_EVENT, cb);
}
