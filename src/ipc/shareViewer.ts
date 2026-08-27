import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ViewerConnected {
  /** 連線 id——後續所有事件都掛在它上面。 */
  connId: string;
  /**
   * **這一端算出的 4 位驗證碼，要唸給對方聽。**
   *
   * 跟著回傳值走而不是用事件送：它在 Rust 那邊握手完成時就算出來了，而
   * 訂閱者要等這個 Promise resolve、分頁開好、元件掛載之後才存在——用事件
   * 必然遺失。實機測試就是這樣抓到的（觀看端的驗證碼永遠空白）。
   *
   * 跟主控端相反：那邊的碼絕不送到前端。兩邊不對稱是刻意的。
   */
  sas: string;
}

/**
 * 連進別台機器分享出來的終端機。
 *
 * 傳輸跑在 Rust，不在這裡：要連的是 TLS ＋ 自簽憑證，而 webview 的
 * `new WebSocket("wss://...")` 會拒絕自簽憑證且沒有程式化例外。
 */
export function shareViewerConnect(
  host: string,
  port: number,
  code: string,
  displayName: string,
): Promise<ViewerConnected> {
  return invoke<ViewerConnected>("share_viewer_connect", { host, port, code, displayName });
}

/** 把按鍵送給對方。唯讀時不該呼叫——伺服器端還有一道授權檢查。 */
export function shareViewerSend(connId: string, data: string): Promise<void> {
  return invoke<void>("share_viewer_send", { connId, data });
}

export function shareViewerDisconnect(connId: string): Promise<void> {
  return invoke<void>("share_viewer_disconnect", { connId });
}

export interface ViewerGranted {
  /** `"read_only"` 或 `"control"`。 */
  mode: string;
  /** 主控端的終端機尺寸——xterm 必須照這個建，不能用自己的視窗大小。 */
  cols: number;
  rows: number;
}

export function onShareViewerGranted(
  connId: string,
  cb: (g: ViewerGranted) => void,
): Promise<UnlistenFn> {
  return listen<ViewerGranted>(`share-viewer://granted/${connId}`, (e) => cb(e.payload));
}

/** 遠端 PTY 畫面，base64 編碼（跟本機 `pty://data` 同樣的形狀）。 */
export function onShareViewerData(
  connId: string,
  cb: (base64: string) => void,
): Promise<UnlistenFn> {
  return listen<{ base64: string }>(`share-viewer://data/${connId}`, (e) =>
    cb(e.payload.base64),
  );
}

/**
 * 落後太多，要清空畫面——下一批 data 是全量重播。
 *
 * 不能忽略：漏掉的位元組可能截斷 ANSI 逃脫序列，帶著壞掉的畫面繼續是不會
 * 自己好的。
 */
export function onShareViewerResync(connId: string, cb: () => void): Promise<UnlistenFn> {
  return listen<unknown>(`share-viewer://resync/${connId}`, () => cb());
}

export function onShareViewerControlChanged(
  connId: string,
  cb: (mode: string) => void,
): Promise<UnlistenFn> {
  return listen<{ mode: string }>(`share-viewer://control/${connId}`, (e) =>
    cb(e.payload.mode),
  );
}

/** 連線結束。`reason` 是 `EndReason` 的 snake_case 字串。 */
export function onShareViewerEnded(
  connId: string,
  cb: (reason: string) => void,
): Promise<UnlistenFn> {
  return listen<{ reason: string }>(`share-viewer://ended/${connId}`, (e) =>
    cb(e.payload.reason),
  );
}
