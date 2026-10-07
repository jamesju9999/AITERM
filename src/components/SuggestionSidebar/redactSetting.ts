const KEY = "aiterm-suggest-redact";

/** 送出前遮罩敏感資訊。預設開啟：只有使用者明確關掉（存 "false"）才是關。 */
export function loadRedactEnabled(): boolean {
  try { return localStorage.getItem(KEY) !== "false"; } catch { return true; }
}

export function saveRedactEnabled(enabled: boolean): void {
  try { localStorage.setItem(KEY, String(enabled)); } catch { /* ignore */ }
}
