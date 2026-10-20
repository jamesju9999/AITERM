import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ActivityStatus = "running" | "done" | "failed" | "interrupted";

/** 對應 Rust `claude_activity::tree::Node`。 */
export interface ActivityNode {
  id: string;
  parent: string | null;
  agent_type: string;
  description: string | null;
  status: ActivityStatus;
  started_ms: number;
  ended_ms: number | null;
  current_tool: string | null;
  summary: string | null;
  tokens: number | null;
  tool_count: number | null;
}

export const claudeActivityEvent = (sessionId: string): string =>
  `claude-activity://${sessionId}`;

export function getClaudeActivity(sessionId: string): Promise<ActivityNode[]> {
  return invoke<ActivityNode[]>("claude_activity_snapshot", { sessionId });
}

/** 事件酬載永遠是完整快照，所以收到就整個取代。 */
export function listenClaudeActivity(
  sessionId: string,
  onSnapshot: (nodes: ActivityNode[]) => void,
): Promise<UnlistenFn> {
  return listen<ActivityNode[]>(claudeActivityEvent(sessionId), (e) => onSnapshot(e.payload));
}
