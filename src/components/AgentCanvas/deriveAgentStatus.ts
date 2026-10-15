import type { AgentRow } from "../../ipc/agents";
import type { TaskRow } from "../../ipc/tasks";

/**
 * Agent 卡片上的狀態。完全由卡片推導，Agent 本身不存狀態——看板與
 * 協作頁看的是同一份資料，推導出來的東西才不會跟卡片分家。
 *
 * - attention：暫停中（某張卡失敗／卡住／被停止），等使用者重試或跳過
 * - coding：有卡片正在執行
 * - waiting：有待執行的卡片，但還沒輪到（全域名額滿了）
 * - done：有卡片、全部成功完成、沒有待執行的
 * - idle：其他（沒有卡片，或做完了但中間有跳過的失敗卡）
 */
export type AgentStatus = "attention" | "coding" | "waiting" | "done" | "idle";

export function deriveAgentStatus(agent: AgentRow, cards: TaskRow[]): AgentStatus {
  if (agent.paused_task_id) return "attention";
  const mine = cards.filter((c) => c.agent_id === agent.id);
  if (mine.some((c) => c.status === "running")) return "coding";
  if (mine.some((c) => c.status === "queued")) return "waiting";
  if (mine.length > 0 && mine.every((c) => c.status === "done" && c.outcome === "success")) return "done";
  return "idle";
}
