import type { ChatMessage } from "../ipc/ai";

/**
 * Keep only the last `limit` messages. Used to cap chat history before
 * sending it to the AI. `limit <= 0` yields an empty array.
 */
export function truncateHistory(
  msgs: ChatMessage[],
  limit: number,
): ChatMessage[] {
  if (limit <= 0) return [];
  if (msgs.length <= limit) return msgs;
  return msgs.slice(msgs.length - limit);
}
