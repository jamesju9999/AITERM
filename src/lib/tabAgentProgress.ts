import type { Tab } from "../components/TabBar";

/**
 * 把指定分頁的 agentProgress 欄位換成新值，其餘分頁不動。
 * 「設定進度」（傳 { done, total }）與「任務結束後清掉進度」（傳 undefined）
 * 共用同一個 reducer——差別只在傳入值，不該各寫一份 map。
 */
export function setTabAgentProgress(
  tabs: Tab[],
  tabId: string,
  progress: Tab["agentProgress"]
): Tab[] {
  return tabs.map((t) => (t.id === tabId ? { ...t, agentProgress: progress } : t));
}
