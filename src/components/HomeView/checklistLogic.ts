import type { SettingsTarget } from "../OpenSettingsButton";

/** 首頁設定清單判斷「設好了沒」只需要這些數字。 */
export interface ChecklistInput {
  providers: number;
  mailAccounts: { enabled?: boolean }[];
  mcpServers: number;
  dbConnections: number;
  vcsConnections: number;
  bridgeEnabled: boolean;
}

export type ChecklistId = "provider" | "mail" | "mcp" | "db" | "vcs" | "bridge";

export interface ChecklistItem {
  id: ChecklistId;
  tab: SettingsTarget;
  /** todo＝還沒設；warn＝設了但關著（目前只有信箱帳號會這樣）；done＝完成。 */
  status: "done" | "todo" | "warn";
  /** 必要項目不能按「不需要」。 */
  required: boolean;
}

/** `enabled` 缺省視為啟用，跟後端對舊設定檔的處理一致。 */
export function buildChecklist(c: ChecklistInput): ChecklistItem[] {
  const mailStatus: ChecklistItem["status"] =
    c.mailAccounts.length === 0 ? "todo" : c.mailAccounts.some((a) => a.enabled !== false) ? "done" : "warn";
  return [
    { id: "provider", tab: "providers", status: c.providers > 0 ? "done" : "todo", required: true },
    { id: "mail", tab: "mail", status: mailStatus, required: false },
    { id: "mcp", tab: "mcp", status: c.mcpServers > 0 ? "done" : "todo", required: false },
    { id: "db", tab: "databases", status: c.dbConnections > 0 ? "done" : "todo", required: false },
    { id: "vcs", tab: "vcs", status: c.vcsConnections > 0 ? "done" : "todo", required: false },
    { id: "bridge", tab: "bridge", status: c.bridgeEnabled ? "done" : "todo", required: false },
  ];
}

const DISMISS_KEY = "aiterm_setup_dismissed";

export function loadDismissed(): Set<string> {
  try {
    const raw = localStorage.getItem(DISMISS_KEY);
    const arr = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(arr) ? arr.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

export function saveDismissed(ids: Set<string>): void {
  try {
    localStorage.setItem(DISMISS_KEY, JSON.stringify([...ids]));
  } catch { /* 存不了就只在這次有效 */ }
}
