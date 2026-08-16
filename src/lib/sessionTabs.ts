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
    return saved.map(({ aiSummary, ...s }) => ({
      ...s,
      id: crypto.randomUUID(),
      // 每個分頁的 PTY 開回它自己上次的目錄。少了這個，所有分頁都會開在
      // 全域的 aiterm_last_cwd，然後第一次 cwd 輪詢就把每個分頁的 cwd 全部
      // 覆蓋成同一個值——存下來的目錄活不過開機後兩秒。
      initialCwd: s.cwd,
      // 存下來的摘要進 lastSessionSummary 而不是 aiSummary：後者會餵進標題列
      // （TerminalApp 的 titleBarText），而標題列沒有任何「上次」的框架，這個
      // session 還沒跑任何指令就顯示上次的摘要等於在說謊。首頁的卡片有「接續
      // 上次的工作」這個標題撐著，讀哪一個都成立。
      lastSessionSummary: aiSummary,
    }));
  } catch {
    return null;
  }
}

export function saveSessionTabs(tabs: Tab[]) {
  const toSave: SavedTab[] = tabs.map(({ title, type, dbConnectionId, cwd, aiSummary, lastSessionSummary }) => ({
    title,
    type,
    dbConnectionId,
    cwd,
    // 這個 session 有跑出新摘要就存新的，否則把上次那份留著——不然連續重開
    // 兩次而中間沒下過指令，摘要就消失了。
    aiSummary: aiSummary ?? lastSessionSummary,
  }));
  localStorage.setItem(SESSION_TABS_KEY, JSON.stringify(toSave));
}
