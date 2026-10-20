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
  /** 建立時的提問輪次（使用者每送出一次提問 +1）。 */
  turn: number;
}

/** 對應 Rust `claude_activity::tree::Snapshot`。 */
export interface ActivitySnapshot {
  nodes: ActivityNode[];
  /** 主 Claude 現在在用的工具。 */
  main_tool: string | null;
  turn: number;
}

export const EMPTY_SNAPSHOT: ActivitySnapshot = { nodes: [], main_tool: null, turn: 0 };

export const claudeActivityEvent = (sessionId: string): string =>
  `claude-activity://${sessionId}`;

export function getClaudeActivity(sessionId: string): Promise<ActivitySnapshot> {
  return invoke<ActivitySnapshot>("claude_activity_snapshot", { sessionId });
}

/** 事件酬載永遠是完整快照，所以收到就整個取代。 */
export function listenClaudeActivity(
  sessionId: string,
  onSnapshot: (snapshot: ActivitySnapshot) => void,
): Promise<UnlistenFn> {
  return listen<ActivitySnapshot>(claudeActivityEvent(sessionId), (e) => onSnapshot(e.payload));
}
