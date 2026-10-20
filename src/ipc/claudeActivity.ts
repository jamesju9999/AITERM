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
  /** 這個 agent 最近一次有任何事件的時間（毫秒）；超過門檻沒動靜就視為卡住。 */
  last_event_ms: number;
  /** 建立時的提問輪次（使用者每送出一次提問 +1）。 */
  turn: number;
}

/** 對應 Rust `claude_activity::tree::Snapshot`。 */
export interface ActivitySnapshot {
  nodes: ActivityNode[];
  /** 主 Claude 現在在用的工具。 */
  main_tool: string | null;
  /** 本輪主 Claude 的工具呼叫次數（含呼叫 Agent 本身）。 */
  main_tool_count: number;
  /** 本輪主 Claude 最近的工具提示，舊的在前，最多 10 筆。 */
  main_recent: string[];
  turn: number;
}

export const EMPTY_SNAPSHOT: ActivitySnapshot = { nodes: [], main_tool: null, main_tool_count: 0, main_recent: [], turn: 0 };

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
