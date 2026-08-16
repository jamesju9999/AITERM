import type { Tab } from "../components/TabBar";

export const SESSION_TABS_KEY = "aiterm-session-tabs";

// 只存重開 app 後還有意義的欄位。ptySessionId / attention / agentProgress
// 這類執行期狀態不存——重開後它們指向的 PTY、事件、進度都已經不存在，
// 存下來只會讓還原出來的分頁帶著假狀態。
type SavedTab = Pick<Tab, "title" | "type" | "dbConnectionId" | "cwd" | "aiSummary">;

export function restoreSessionTabs(): Tab[] | null {
  try {
    const raw = localStorage.getItem(SESSION_TABS_KEY);
    if (!raw) return null;
    const saved: SavedTab[] = JSON.parse(raw);
    if (!Array.isArray(saved) || saved.length === 0) return null;
    return saved.map((s) => ({ ...s, id: crypto.randomUUID() }));
  } catch {
    return null;
  }
}

export function saveSessionTabs(tabs: Tab[]) {
  const toSave: SavedTab[] = tabs.map(({ title, type, dbConnectionId, cwd, aiSummary }) => ({
    title,
    type,
    dbConnectionId,
    cwd,
    aiSummary,
  }));
  localStorage.setItem(SESSION_TABS_KEY, JSON.stringify(toSave));
}
