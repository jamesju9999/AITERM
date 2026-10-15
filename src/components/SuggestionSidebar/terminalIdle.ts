/** 終端機超過這麼久沒有輸出，才算閒置（AI 工具回完了）。 */
export const IDLE_MS = 2_000;
/**
 * 滾輪之後這麼久內的 PTY 輸出，視為「捲動造成的重畫」而不是 AI 在工作。
 * 全螢幕程式（Claude Code 等）在 alternate buffer 裡收到 xterm 轉出的按鍵就重畫；
 * 連續捲動會持續刷新滾輪時間，所以視窗只需涵蓋單次重畫的延遲。
 */
export const SCROLL_ECHO_MS = 500;
/** 多久檢查一次 getIdleMs()。 */
export const POLL_MS = 1_000;
