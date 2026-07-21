import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage } from "./ai";

export type CodeAssistantEvent =
  | { kind: "tool_call";     session_id: string; call_id: string; tool: string; args: Record<string, unknown> }
  | { kind: "tool_progress"; session_id: string; call_id: string; message: string }
  | { kind: "tool_result";   session_id: string; call_id: string; content: string; truncated: boolean }
  | { kind: "text_delta";    session_id: string; delta: string }
  | { kind: "done";          session_id: string }
  | { kind: "error";         session_id: string; message: string }
  | { kind: "fallback_mode"; session_id: string }
  | { kind: "token_count";  session_id: string; count: number; limit: number };

export const CODE_ASSISTANT_EVENT = "code-assistant-event";

export function invokeCodeAssistantChat(
  projectRoot: string,
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string | null,
  locale: string = "zh-TW",
): Promise<void> {
  return invoke<void>("code_assistant_chat", {
    projectRoot,
    messages,
    sessionId,
    providerId: providerId ?? null,
    locale,
  });
}
