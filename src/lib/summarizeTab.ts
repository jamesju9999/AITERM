import { invokeAiChat } from "../ipc/ai";
import { contentToDisplayString } from "../types/attachment";
import type { McpChatMessage } from "../hooks/useMcpChat";
import type { Locale } from "./i18n";

const MAX_CONTEXT_MESSAGES = 10;

function buildSummaryPrompt(messages: McpChatMessage[], locale: Locale): string {
  const recent = messages
    .filter((m) => m.role === "user" || m.role === "assistant")
    .slice(-MAX_CONTEXT_MESSAGES);
  const transcript = recent
    .map((m) => `${m.role}: ${contentToDisplayString(m.content)}`)
    .join("\n");

  return locale === "zh-TW"
    ? `請根據以下對話，用不超過 20 個字生成一句精簡的中文摘要，描述使用者目前在做什麼。不要標點符號結尾、不要加引號、只輸出摘要本身。\n\n${transcript}`
    : `Based on the following conversation, write a concise summary (40 characters or fewer) in English describing what the user is currently doing. No trailing punctuation, no quotes, output only the summary itself.\n\n${transcript}`;
}

/**
 * One-shot AI call that summarizes a terminal tab's recent /ai conversation
 * into a short title-bar-friendly string. Returns null on any failure
 * (network error, provider not configured, empty reply) — callers should
 * treat null as "leave the title bar as it was", never surface an error.
 */
export async function summarizeConversation(
  messages: McpChatMessage[],
  sessionId: string,
  locale: Locale,
): Promise<string | null> {
  const hasAssistantReply = messages.some((m) => m.role === "assistant");
  if (!hasAssistantReply) return null;

  try {
    const prompt = buildSummaryPrompt(messages, locale);
    const reply = await invokeAiChat(
      [{ role: "user", content: prompt }],
      `${sessionId}-summary`,
      undefined,
      false,
      locale,
    );
    const summary = reply.content?.trim();
    return summary || null;
  } catch {
    return null;
  }
}
