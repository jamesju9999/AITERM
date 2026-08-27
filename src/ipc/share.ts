import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ShareStatus {
  sharing: boolean;
  /** 6 位短碼。沒在分享時是 null。 */
  code: string | null;
  /** server 的 port。沒在分享時是 null。 */
  port: number | null;
  /** 這台機器的區網位址（不含 port）。查不到時是 null，面板退成只顯示 port。 */
  lanAddress: string | null;
}

/**
 * 一筆待審連線請求。
 *
 * **刻意沒有驗證碼欄位。** 主控端的 4 位碼永遠不離開 Rust——同意視窗要
 * 使用者輸入對方唸的碼，比對在 `share_approve` 裡做。前端拿不到那個值，
 * 所以不可能顯示它；使用者只能開口問對方。
 *
 * 若哪天有人想「順便把碼也傳過來顯示」：那會讓使用者照抄畫面上的數字而
 * 不問對方，人工核對變成自欺，而那次口頭核對正是整個防中間人保證的最後
 * 一哩。不要加。
 */
export interface PendingRequest {
  requestId: string;
  tabId: string;
  /** 對方自報的名字，**未經驗證**。文案不能讓它看起來像身分保證。 */
  displayName: string;
}

export interface Viewer {
  viewerId: string;
  /** 對方自報的名字，**未經驗證**。 */
  displayName: string;
  mode: "read_only" | "control";
}

export type Decision =
  | { kind: "approved"; viewerId: string }
  /** 輸入的碼不符——連線已被拒絕，不給重試。 */
  | { kind: "codeMismatch" }
  /** 控制權已被別人持有；請求還在，可以改用唯讀重新裁決。 */
  | { kind: "controlTaken" }
  /** 請求已經不在了（對方斷線或分享被停掉）。 */
  | { kind: "requestGone" };

export function shareStart(tabId: string): Promise<ShareStatus> {
  return invoke<ShareStatus>("share_start", { tabId });
}

export function shareStop(tabId: string): Promise<ShareStatus> {
  return invoke<ShareStatus>("share_stop", { tabId });
}

export function shareStatus(tabId: string): Promise<ShareStatus> {
  return invoke<ShareStatus>("share_status", { tabId });
}

export function sharePending(tabId: string): Promise<PendingRequest[]> {
  return invoke<PendingRequest[]>("share_pending", { tabId });
}

/** 把使用者輸入的碼送去 Rust 比對。前端不做比對——它沒有那個材料。 */
export function shareApprove(
  requestId: string,
  mode: "read_only" | "control",
  typedCode: string,
): Promise<Decision> {
  return invoke<Decision>("share_approve", { requestId, mode, typedCode });
}

export function shareDeny(requestId: string): Promise<void> {
  return invoke<void>("share_deny", { requestId });
}

export function shareViewers(tabId: string): Promise<Viewer[]> {
  return invoke<Viewer[]>("share_viewers", { tabId });
}

export function shareKick(tabId: string, viewerId: string): Promise<void> {
  return invoke<void>("share_kick", { tabId, viewerId });
}

export function shareRevokeControl(tabId: string): Promise<void> {
  return invoke<void>("share_revoke_control", { tabId });
}

/** 有人輸入短碼要連進來了——同意視窗該跳出來。 */
export function onSharePendingRequest(
  cb: (payload: PendingRequest) => void,
): Promise<UnlistenFn> {
  return listen<PendingRequest>("share://request-pending", (e) => cb(e.payload));
}

/**
 * 觀看者清單變了（有人連上或離開）。
 *
 * 刻意不帶內容——收到就去 `shareViewers` 重讀，避免推播的資料跟查詢的
 * 資料對不上。
 */
export function onShareViewersChanged(cb: () => void): Promise<UnlistenFn> {
  return listen<unknown>("share://viewers-changed", () => cb());
}
