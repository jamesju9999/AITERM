// src/types/chat.ts
import type { ContentPart } from "../ipc/ai";

export interface McpChatMessage {
  role: "user" | "assistant" | "tool_call" | "tool_result";
  content: string | ContentPart[];
  tool_name?: string;
  tool_call_id?: string;
  is_error?: boolean;
  is_loading?: boolean;
}

export interface McpChatSession {
  id: string;
  title: string;
  messages: McpChatMessage[];
  savedAt: number;
}
