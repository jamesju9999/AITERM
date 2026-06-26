import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage } from "./ai";
import type { TraceEntry, LoopConfig } from "../hooks/useOrchestratorLoop";

export interface LoopSessionSummary {
  id: string;
  goal: string;
  status: "running" | "paused" | "completed" | "failed";
  iteration: number;
  created_at: string;
  updated_at: string;
}

export interface LoopSessionData extends LoopSessionSummary {
  config_json: string;
  history_json: string;
  shared_context: string;
  trace_json: string;
}

export interface LoopSessionSnapshot {
  config: LoopConfig;
  orchestratorHistory: ChatMessage[];
  sharedContext: string;
  trace: TraceEntry[];
  iteration: number;
}

export function loopSessionSave(
  id: string,
  goal: string,
  status: LoopSessionSummary["status"],
  iteration: number,
  snapshot: Omit<LoopSessionSnapshot, "iteration">,
): Promise<void> {
  return invoke("loop_session_save", {
    args: {
      id,
      goal,
      status,
      iteration,
      config_json: JSON.stringify(snapshot.config),
      history_json: JSON.stringify(snapshot.orchestratorHistory),
      shared_context: snapshot.sharedContext,
      trace_json: JSON.stringify(snapshot.trace),
    },
  });
}

export function loopSessionList(): Promise<LoopSessionSummary[]> {
  return invoke("loop_session_list");
}

export function loopSessionLoad(id: string): Promise<LoopSessionData> {
  return invoke("loop_session_load", { id });
}

export function loopSessionDelete(id: string): Promise<void> {
  return invoke("loop_session_delete", { id });
}

export function loopSessionClearAll(): Promise<void> {
  return invoke("loop_session_clear_all");
}

export function loopProjectPickOpen(): Promise<string | null> {
  return invoke("loop_project_pick_open");
}

export function loopProjectPickSave(): Promise<string | null> {
  return invoke("loop_project_pick_save");
}

export function parseLoopSessionData(data: LoopSessionData): LoopSessionSnapshot {
  return {
    config: JSON.parse(data.config_json) as LoopConfig,
    orchestratorHistory: JSON.parse(data.history_json) as ChatMessage[],
    sharedContext: data.shared_context,
    trace: JSON.parse(data.trace_json) as TraceEntry[],
    iteration: data.iteration,
  };
}
