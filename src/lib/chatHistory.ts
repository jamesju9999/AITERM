import type { ChatMessage, ContentPart } from "../ipc/ai";
import type { McpChatMessage } from "../types/chat";

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

/**
 * 對話 → 送給模型的歷史。App 的說明（notice）不送；拿掉之後相鄰的兩則 user
 * （例如停止後再送一次）併成一則，部分供應商不收連續的 user。
 */
export function toModelHistory(messages: McpChatMessage[]): ChatMessage[] {
  const out: ChatMessage[] = [];
  for (const m of messages) {
    if ((m.role !== "user" && m.role !== "assistant") || m.notice) continue;
    const prev = out.at(-1);
    if (m.role === "user" && prev?.role === "user") {
      out[out.length - 1] = { role: "user", content: joinContent(prev.content, m.content) };
    } else {
      out.push({ role: m.role, content: m.content });
    }
  }
  return out;
}

function joinContent(a: ChatMessage["content"], b: ChatMessage["content"]): ChatMessage["content"] {
  if (typeof a === "string" && typeof b === "string") return `${a}\n\n${b}`;
  const parts = (c: ChatMessage["content"]): ContentPart[] => (typeof c === "string" ? [{ type: "text", text: c }] : (c ?? []));
  return [...parts(a), ...parts(b)];
}
