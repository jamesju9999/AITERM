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
  sender_name: string | null;
  project_id: string;
  to_addrs: string | null;
  cc_addrs: string | null;
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

export interface IntakeConfigView {
  config: IntakeConfig;
  /** 這個專案是否存過自己的設定。 */
  saved: boolean;
  telegram_default: boolean;
}

export interface IntakeProject {
  id: string;
  name: string;
  /** 看板設定的專案目錄；沒設定為 null。 */
  repo_dir: string | null;
  /** 需要處理的題數（待派送＋待寄出草稿）與總數。 */
  open_count: number;
  total: number;
  enabled: boolean;
}

export interface IntakeProjects {
  projects: IntakeProject[];
  /** 尚未歸屬的題數（升級前的舊資料）。 */
  unassigned: number;
  /** 「未歸屬」底下還留著升級前的設定或白名單。 */
  legacy_settings: boolean;
}

export interface IntakeTab {
  tab_id: string;
  project_id: string;
  /** 這個分頁現在能收提問（偵測到 Claude Code 在跑）。false 時要先在分頁裡啟動 claude。 */
  claude_ready: boolean;
}

export const intakeProjects = () => invoke<IntakeProjects>("intake_projects");
export interface IntakeRefreshResult {
  /** 有去信箱收信。 */
  polled: boolean;
  ok: boolean;
  new_items: number;
  error: string | null;
}
/** 立刻去這個專案設定的信箱收一次信，收完回傳結果。 */
export const intakeRefresh = (projectId: string) => invoke<IntakeRefreshResult>("intake_refresh", { projectId });
export const intakeAdoptLegacy = (projectId: string) => invoke<void>("intake_adopt_legacy", { projectId });
export const intakeAssign = (id: string, projectId: string) => invoke<boolean>("intake_assign", { id, projectId });
export const intakeCopyLegacyConfig = (projectId: string) => invoke<void>("intake_copy_legacy_config", { projectId });
export const intakeGetConfig = (projectId: string) => invoke<IntakeConfigView>("intake_get_config", { projectId });
export const intakeSetConfig = (projectId: string, config: IntakeConfig, telegramDefault: boolean) =>
  invoke<void>("intake_set_config", { projectId, config, telegramDefault });
export const intakeList = (projectId: string, status?: IntakeStatus) =>
  invoke<IntakeItem[]>("intake_list", { projectId, status: status ?? null });
export const intakeSenders = (projectId: string) => invoke<IntakeSender[]>("intake_senders", { projectId });
export const intakeUpsertSender = (projectId: string, sender: IntakeSender) => invoke<void>("intake_upsert_sender", { projectId, sender });
export const intakeDeleteSender = (projectId: string, address: string) => invoke<boolean>("intake_delete_sender", { projectId, address });
export const intakeDropped = (projectId: string) => invoke<[string, number][]>("intake_dropped", { projectId });
export const intakeApprove = (id: string) => invoke<boolean>("intake_approve", { id });
export const intakeDelete = (id: string) => invoke<boolean>("intake_delete", { id });
export const intakeReject = (id: string) => invoke<boolean>("intake_reject", { id });
export const intakeUpdateDraft = (id: string, draft: string) => invoke<boolean>("intake_update_draft", { id, draft });
export const intakeSendReply = (id: string) => invoke<void>("intake_send_reply", { id });
export const intakeTabStart = (tabId: string, projectId: string) => invoke<void>("intake_tab_start", { tabId, projectId });
/** 把分頁切換到專案的專案目錄；分頁沒閒置在提示字元、路徑無法安全送出或切換失敗時丟出原因。 */
export const intakeTabCd = (tabId: string, projectId: string) => invoke<string>("intake_tab_cd", { tabId, projectId });
export const intakeTabStop = (tabId: string) => invoke<void>("intake_tab_stop", { tabId });
export const intakeTabs = () => invoke<IntakeTab[]>("intake_tabs");

export function onIntakeChanged(cb: () => void): Promise<UnlistenFn> {
  return listen(INTAKE_CHANGED_EVENT, cb);
}

export interface IntakeAttachment {
  id: string;
  item_id: string;
  filename: string;
  path: string;
  mime: string;
  size: number;
}
export const intakeAttachments = (itemId: string) => invoke<IntakeAttachment[]>("intake_attachments", { itemId });
/** 回傳 `data:` URL。 */
/** 純文字附件（log、txt…）的前 64KB，給預覽。 */
export const intakeAttachmentText = (id: string) => invoke<string>("intake_attachment_text", { id });
export const intakeAttachmentData = (id: string) => invoke<string>("intake_attachment_data", { id });
