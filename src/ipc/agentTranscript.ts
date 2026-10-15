import { invoke } from "@tauri-apps/api/core";

export type TranscriptTool = "claude" | "codex";

export interface TranscriptContext {
  tool: TranscriptTool;
  /** 最近兩輪（使用者／AI／工具呼叫），不含工具回傳結果。 */
  text: string;
}

/** 依目前畫面找出對應的 AI 工具對話記錄（Claude Code／Codex）。對不上就是 null。 */
export function agentTranscriptContext(screen: string): Promise<TranscriptContext | null> {
  return invoke<TranscriptContext | null>("agent_transcript_context", { screen });
}
