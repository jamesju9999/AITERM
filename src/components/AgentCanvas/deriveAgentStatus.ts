import type { AgentRow } from "../../ipc/agents";
import type { TaskRow } from "../../ipc/tasks";
import type { ModuleState } from "./deriveModuleState";

/**
 * Agent 卡片上的狀態。完全由卡片推導，Agent 本身不存狀態——看板與
 * 協作頁看的是同一份資料，推導出來的東西才不會跟卡片分家。
 *
 * - attention：暫停中（某張卡失敗／卡住／被停止），等使用者重試或跳過
 * - coding／reviewing／merging：有卡片正在執行（依 Agent 種類）
 * - waiting：有待執行的卡片，但還沒輪到（全域名額滿了）
 * - done：有卡片、全部成功完成、沒有待執行的
 * - idle：其他（沒有卡片，或做完了但中間有跳過的失敗卡）
 */
export type AgentStatus = "attention" | "coding" | "reviewing" | "merging" | "verifying" | "waiting" | "done" | "idle";

/**
 * `moduleStates`：Coder 名下每個模組的狀態（見 deriveModuleState）。有傳就以
 * 模組為準——模組卡本身跑完不代表完成，還要等審查通過；審查不通過超過上限
 * 的模組要讓 Coder 也亮紅燈。
 */
export function deriveAgentStatus(agent: AgentRow, cards: TaskRow[], moduleStates?: ModuleState[]): AgentStatus {
  if (agent.paused_task_id) return "attention";
  const mine = cards.filter((c) => c.agent_id === agent.id);
  if (mine.some((c) => c.status === "running")) {
    // 同一個「執行中」依 Agent 種類換顏色與字：Coder 藍、Reviewer 黃、Orchestrator 綠、Verifier 青。
    return ({ reviewer: "reviewing", orchestrator: "merging", verifier: "verifying", coder: "coding" } as const)[agent.kind];
  }
  if (mine.some((c) => c.status === "queued")) return "waiting";
  if (moduleStates) {
    const stuck = moduleStates.some((m) => m === "attention" || m === "review_failed" || m === "failed");
    // Coder 自己的模組卡住要亮紅燈；Orchestrator 只是在等，不是它出錯。
    if (stuck) return agent.kind === "orchestrator" ? "idle" : "attention";
    return moduleStates.length > 0 && moduleStates.every((m) => m === "passed") ? "done" : "idle";
  }
  if (mine.length > 0 && mine.every((c) => c.status === "done" && c.outcome === "success")) return "done";
  return "idle";
}
