import { useSyncExternalStore } from "react";

const ENABLED_KEY = "aiterm-claude-activity-enabled";
const EVENT = "aiterm:claude-activity-enabled";
const NOTIFY_KEY = "aiterm-claude-activity-notify";

/** 是否顯示 Claude 協作圖；預設開（仍預設收合，見面板）。 */
export function isClaudeActivityEnabled(): boolean {
  try {
    return localStorage.getItem(ENABLED_KEY) !== "0";
  } catch {
    return true;
  }
}

export function setClaudeActivityEnabled(on: boolean): void {
  try { localStorage.setItem(ENABLED_KEY, on ? "1" : "0"); } catch { /* ignore */ }
  window.dispatchEvent(new Event(EVENT));
}

function subscribe(cb: () => void): () => void {
  window.addEventListener(EVENT, cb);
  window.addEventListener("storage", cb);
  return () => {
    window.removeEventListener(EVENT, cb);
    window.removeEventListener("storage", cb);
  };
}

export function useClaudeActivityEnabled(): boolean {
  return useSyncExternalStore(subscribe, isClaudeActivityEnabled, () => true);
}

/** 失敗／全部完成／卡住時發桌面通知；預設開（只在使用者沒在看這個分頁時才會發）。 */
export function isClaudeActivityNotifyEnabled(): boolean {
  try {
    return localStorage.getItem(NOTIFY_KEY) !== "0";
  } catch {
    return true;
  }
}

export function setClaudeActivityNotifyEnabled(on: boolean): void {
  try { localStorage.setItem(NOTIFY_KEY, on ? "1" : "0"); } catch { /* ignore */ }
  window.dispatchEvent(new Event(EVENT));
}

export function useClaudeActivityNotifyEnabled(): boolean {
  return useSyncExternalStore(subscribe, isClaudeActivityNotifyEnabled, () => true);
}
