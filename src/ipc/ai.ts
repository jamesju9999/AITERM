import { invoke } from "@tauri-apps/api/core";

export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } };

export type RiskLevel = "safe" | "needs_confirm" | "dangerous" | "blocked";

export type AiError =
  | { kind: "not_configured" }
  | { kind: "network"; message: string }
  | { kind: "auth_failed" }
  | { kind: "rate_limit"; retry_after: string | null }
  | { kind: "model_error"; reason: string; raw: string }
  | { kind: "invalid_input"; reason: string };

export interface AiCommandReady {
  command: string;
  explanation: string;
  risk_level: RiskLevel;
}

export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string | ContentPart[];
}

export interface AiChatReply {
  content: string;
}

export function invokeAiChat(
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string,
): Promise<AiChatReply> {
  return invoke<AiChatReply>("ai_chat", { messages, sessionId, providerId: providerId ?? null });
}

export type AiStreamKind = "query" | "chat";

export interface AiStreamEvent {
  session_id: string;
  kind: AiStreamKind;
  delta: string;
  done: boolean;
}

export function invokeAiQuery(
  query: string,
  sessionId: string,
): Promise<AiCommandReady> {
  return invoke<AiCommandReady>("ai_query", { query, sessionId });
}

export async function aiChat(
  message: string,
  systemPrompt: string,
  history: { role: "user" | "assistant"; content: string }[],
  providerId?: string,
): Promise<string> {
  const messages: ChatMessage[] = [
    { role: "system", content: systemPrompt },
    ...history,
    { role: "user", content: message },
  ];
  const reply = await invokeAiChat(messages, "db-ai-chat", providerId);
  return reply.content;
}

export function formatAiError(e: AiError): string {
  switch (e.kind) {
    case "not_configured":
      return "aiterm: 尚未設定 AI Provider。";
    case "network":
      if (
        e.message?.toLowerCase().includes("ollama") ||
        e.message?.toLowerCase().includes("connection refused") ||
        e.message?.toLowerCase().includes("connect error")
      ) {
        return "aiterm: 無法連線到 Ollama。請確認 Ollama 已啟動。";
      }
      return `aiterm: 網路錯誤 — ${e.message}`;
    case "auth_failed":
      return "aiterm: API Key 驗證失敗。";
    case "rate_limit":
      return e.retry_after
        ? `aiterm: 請求過於頻繁（${e.retry_after} 秒後重試）`
        : "aiterm: 請求過於頻繁，請稍後再試";
    case "model_error":
      return `aiterm: AI 回傳格式錯誤（${e.reason}）`;
    case "invalid_input":
      return `aiterm: 前端傳送的訊息格式無效（${e.reason}）`;
  }
}
