// src/types/chat.ts
import type { ContentPart } from "../ipc/ai";

export interface McpChatMessage {
  role: "user" | "assistant" | "tool_call" | "tool_result";
  content: string | ContentPart[];
  tool_name?: string;
  tool_call_id?: string;
  is_error?: boolean;
  is_loading?: boolean;
  /** App 給使用者的提示（例如圖片沒送出）：照樣顯示，但不隨歷史送回模型。 */
  notice?: boolean;
}

export interface McpChatSession {
  id: string;
  title: string;
  messages: McpChatMessage[];
  savedAt: number;
}
