import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type IntakeStatus = "pending" | "queued" | "running" | "on_board" | "draft" | "done" | "rejected";
export type Level = "internal" | "external";
export type MatchMode = "alias" | "subject_tag" | "alias_or_tag" | "dedicated";
export type DispatchMode = "auto" | "manual";
/** 一個管道收到的問題要怎麼處理：只診斷（不改檔案）或可改檔案。 */
export type IntakeHandling = "diagnose" | "edit";
/** 參考資料夾：共用的所有題目都帶；僅內部的只有等級是「內部」的題目才另外帶。 */
export interface IntakeRefFolders {
  shared: string[];
  internal: string[];
}
/** 參考資料夾的檢查結果。 */
export interface IntakeFolderCheck {
  path: string;
  exists: boolean;
  is_dir: boolean;
  /** Claude Code 讀得到的檔案數。 */
  readable: number;
  /** 讀不了的 Office 類檔案數。 */
  office: number;
  /** 資料夾很大，只統計了前面一部分。 */
  truncated: boolean;
}
export interface IntakeChannelHandling {
  email: IntakeHandling;
  telegram: IntakeHandling;
  line: IntakeHandling;
  web: IntakeHandling;
}
export type IntakeRoute = "terminal" | "board" | "canvas" | "system";

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
  /** 走看板時對應的卡片 id。 */
  board_task_id: string | null;
  /** 走看板／桌布時卡片的種類。 */
  route_kind: "board" | "canvas_bug" | "canvas_ask" | null;
  /** 連到 server 時，領取（處理）這題的成員名稱；沒人領為 null。 */
  claimed_by?: string | null;
  claimed_at?: number | null;
  claim_expires_at?: number | null;
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
  line_enabled: boolean;
  /** LINE webhook 的本機埠號（所有專案共用）。 */
  line_port: number;
  /** 白名單內的 LINE 群組：客戶沒 @ 時，新問題也自動開題。 */
  line_auto_open: boolean;
  /** 測試模式：內部人員的訊息也當成客戶訊息。 */
  line_test_mode: boolean;
  /** LINE 群組訊息在本機保存幾天（所有專案共用；0＝不保存）。 */
  line_retention_days: number;
  web_mode: "off" | "local" | "lan";
  web_port: number;
  /** 各管道的處理方式。 */
  channel_handling: IntakeChannelHandling;
  /** 參考資料夾（共用＋僅內部另加）。 */
  reference_folders: IntakeRefFolders;
  /** 問題派送到哪裡處理。 */
  route: IntakeRoute;
  /** 派送目標是系統連線時，要用哪個系統（SystemConnection.id）。 */
  system_id: string;
}

export interface IntakeSender {
  address: string;
  level: Level;
  skip_auth: boolean;
  /** 備註名稱（例如群組叫什麼）；沒有就是空字串。新增時可不給。 */
  label?: string;
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
export interface IntakeLineStatus {
  /** 憑證本身不會回傳，只告知有沒有存。 */
  has_secret: boolean;
  has_token: boolean;
  port: number;
  /** webhook 伺服器是否正在本機聽。 */
  listening: boolean;
}
/** 最近出現、但還沒加入白名單的 LINE 來源。 */
export interface IntakeLineUnknown {
  id: string;
  kind: "group" | "user";
  name: string;
  count: number;
  first_seen: number;
  last_seen: number;
}
export const intakeLineUnknown = () => invoke<IntakeLineUnknown[]>("intake_line_unknown");
export const intakeLineForgetUnknown = (id: string) => invoke<boolean>("intake_line_forget_unknown", { id });
export const intakeCheckFolders = (projectId: string, paths: string[]) =>
  invoke<IntakeFolderCheck[]>("intake_check_folders", { projectId, paths });
export const intakeLineStatus = () => invoke<IntakeLineStatus>("intake_line_status");
/** 空字串＝不變。 */
export const intakeLineSetCredentials = (channelSecret: string, accessToken: string) =>
  invoke<void>("intake_line_set_credentials", { channelSecret, accessToken });
export const intakeLineClearCredentials = () => invoke<void>("intake_line_clear_credentials");
export const intakeSetConfig = (projectId: string, config: IntakeConfig, telegramDefault: boolean) =>
  invoke<void>("intake_set_config", { projectId, config, telegramDefault });
export const intakeList = (projectId: string, status?: IntakeStatus) =>
  invoke<IntakeItem[]>("intake_list", { projectId, status: status ?? null });
export const intakeSenders = (projectId: string) => invoke<IntakeSender[]>("intake_senders", { projectId });
/** 設定（或傳空字串清除）白名單項目的備註名稱。 */
export const intakeSetSenderLabel = (projectId: string, address: string, label: string) =>
  invoke<void>("intake_set_sender_label", { projectId, address, label });
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

// ───────── 連到 server（Client 模式）─────────

export const INTAKE_REMOTE_EVENT = "intake://remote";

export type IntakeRemoteRole = "member" | "admin";

export interface IntakeRemoteStatus {
  /** 已連到 server（本機的收訊與派送因此停止）。 */
  connected: boolean;
  /** WebSocket 目前連得上；false 代表離線，清單是最後一次的內容。 */
  online: boolean;
  addr: string;
  name: string;
  role: IntakeRemoteRole | null;
  /** 需要使用者處理的原因（已被移除、憑證改變、版本不相容）；正常為 null。 */
  error: string | null;
}

export const intakeRemoteStatus = () => invoke<IntakeRemoteStatus>("intake_remote_status");
/** 用邀請碼配對並連線。 */
export const intakeRemotePair = (addr: string, code: string) => invoke<IntakeRemoteStatus>("intake_remote_pair", { addr, code });
export const intakeRemoteDisconnect = () => invoke<void>("intake_remote_disconnect");
export function onIntakeRemote(cb: (s: IntakeRemoteStatus) => void): Promise<UnlistenFn> {
  return listen<IntakeRemoteStatus>(INTAKE_REMOTE_EVENT, (e) => cb(e.payload));
}

/** 手動「我來處理」：領 server 上這一題，派進你「收集中」且閒置的分頁。 */
export const intakeRemoteClaim = (itemId: string) => invoke<void>("intake_remote_claim", { itemId });

/** server 的寄信帳號（網頁表單寄驗證碼與回覆用）；密碼不會回傳。 */
export interface IntakeServerMail {
  settings: { smtp_host: string; smtp_port: number; imap_host: string; imap_port: number; username: string; from: string };
  configured: boolean;
}
export const intakeServerMailStatus = () => invoke<IntakeServerMail>("intake_server_mail_status");
/** 把 AITerm 某個郵件帳號的 SMTP 設定與密碼套用到 server（管理員）。 */
export const intakeServerMailApply = (accountId: string) => invoke<void>("intake_server_mail_apply", { accountId });

/** server 的 AI 設定（題目分類與群組「是不是新問題」用）；金鑰不會回傳。 */
export interface IntakeServerAi {
  settings: { provider: string; model: string; base_url: string };
  configured: boolean;
  /** 最近一次實際呼叫的結果（server 重啟後清空）。 */
  health?: { last_ok: number | null; last_ms: number | null; error: string | null };
  /** 訂閱登入狀態：none 沒登入、ok 可用、needs_login 憑證失效要重新登入。 */
  login?: "none" | "ok" | "needs_login";
  login_error?: string | null;
}
export interface IntakeServerAiTest {
  ok: boolean;
  ms: number;
  message: string;
}
/** 畫面上還沒儲存的值也能問；`apiKey` 空白＝用 server 已存的。 */
export const intakeServerAiModels = (provider: string, baseUrl: string, apiKey: string) =>
  invoke<string[]>("intake_server_ai_models", { provider, baseUrl, apiKey });
export const intakeServerAiTest = (provider: string, model: string, baseUrl: string, apiKey: string) =>
  invoke<IntakeServerAiTest>("intake_server_ai_test", { provider, model, baseUrl, apiKey });
export const intakeServerAiStatus = () => invoke<IntakeServerAi>("intake_server_ai_status");
/** `apiKey` 空字串＝不變。 */
export const intakeServerAiApply = (provider: string, model: string, baseUrl: string, apiKey: string) =>
  invoke<void>("intake_server_ai_apply", { provider, model, baseUrl, apiKey });

/** server 的 Telegram 狀態與設定；機器人金鑰不會回傳。 */
export interface IntakeServerTelegram {
  settings: { owner_chat_id: string };
  has_token: boolean;
  listening: boolean;
  /** 有金鑰但沒有專案啟用 Telegram：server 沒在收訊。 */
  idle?: boolean;
  error: string | null;
}
export const intakeServerTelegramStatus = () => invoke<IntakeServerTelegram>("intake_server_telegram_status");
/** `botToken` 空字串＝不變。 */
export const intakeServerTelegramApply = (ownerChatId: string, botToken: string) =>
  invoke<void>("intake_server_telegram_apply", { ownerChatId, botToken });

/** 為 server 登入訂閱：開瀏覽器走授權，回傳授權網址（瀏覽器沒開時讓使用者自己開）。 */
export const intakeServerAiLoginStart = (provider: string) => invoke<string>("intake_server_ai_login_start", { provider });
/** 貼回授權碼（或完整回呼網址）完成登入；憑證只交給 server，不存進這台電腦。 */
export const intakeServerAiLoginComplete = (provider: string, codeAndState: string) =>
  invoke<void>("intake_server_ai_login_complete", { provider, codeAndState });
/** ChatGPT／Gemini 訂閱：開瀏覽器授權，由本機回呼完成；等到使用者在瀏覽器完成（最久 2 分鐘）才回傳。 */
export const intakeServerAiLoginBrowser = (provider: string) => invoke<void>("intake_server_ai_login_browser", { provider });
export const intakeServerAiLogout = () => invoke<void>("intake_server_ai_logout");
