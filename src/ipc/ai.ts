import { invoke } from "@tauri-apps/api/core";
import { LOCALE_STORAGE_KEY, translations, type Locale } from "../lib/i18n";

function getT() {
  const loc = (localStorage.getItem(LOCALE_STORAGE_KEY) || "zh-TW") as "zh-TW" | "en";
  return translations[loc] || translations["zh-TW"];
}

export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } };

export type RiskLevel = "safe" | "needs_confirm" | "dangerous" | "blocked";

export type ToolFallbackReason = "unsupported" | "subscription_billing";

export type AiError =
  | { kind: "not_configured" }
  /** 憑證存在但讀不出來（鑰匙圈拒絕存取等）。跟 not_configured 是兩件事。 */
  | { kind: "secret_access"; message: string }
  | { kind: "network"; message: string }
  | { kind: "auth_failed" }
  | { kind: "rate_limit"; retry_after: string | null; body?: string | null }
  | { kind: "model_error"; reason: string; raw: string }
  | { kind: "invalid_input"; reason: string };

export interface AiCommandReady {
  command: string;
  explanation: string;
  risk_level: RiskLevel;
}

export interface ChatMessage {
  role: "user" | "assistant" | "system" | "tool";
  content: string | ContentPart[] | null;
  tool_call_id?: string;   // required when role === "tool"
  tool_calls?: Array<{     // present when role === "assistant" with function calls
    id: string;
    type: "function";
    function: { name: string; arguments: string; thought_signature?: string };
  }>;
}

export interface AiToolCall {
  id: string;             // provider tool call ID (needed for tool result messages)
  server_id: string;      // sanitized server id (decoded from encoded name)
  tool_name: string;      // encoded: "server_id__tool_name"
  args: unknown;
  thought_signature?: string; // Gemini thinking-mode blob — must be echoed verbatim
}

export interface AiChatReply {
  content: string | null;               // null when tool_calls is non-empty
  tool_calls: AiToolCall[];             // AI-requested tool calls
  tool_calling_unsupported: boolean;    // true if provider doesn't support tools
  /** 降級原因。`unsupported` = 模型做不到；`subscription_billing` = 憑證的計費歸屬。 */
  tool_fallback_reason?: ToolFallbackReason | null;
}

export interface AgentToolDefinition {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
}

export interface AgentChatReply {
  content: string | null;
  tool_calls: AiToolCall[];
  tool_calling_unsupported: boolean;
  /** Raw tool_calls JSON from the provider — echo verbatim in conversation history for Gemini thinking models. */
  raw_tool_calls?: ChatMessage["tool_calls"];
}

export function agentChat(
  providerId: string,
  messages: ChatMessage[],
  tools: AgentToolDefinition[],
  sessionId: string,
): Promise<AgentChatReply> {
  return invoke<AgentChatReply>("agent_chat", {
    providerId,
    messages,
    tools,
    sessionId,
  });
}

export function invokeAiChat(
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string,
  useMcp = false,
  locale: Locale = "zh-TW",
): Promise<AiChatReply> {
  return invoke<AiChatReply>("ai_chat", { messages, sessionId, providerId: providerId ?? null, useMcp, locale });
}

export const aiChat = (
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string,
  useMcp = false,
  locale: Locale = "zh-TW",
): Promise<AiChatReply> =>
  invoke("ai_chat", {
    messages,
    sessionId,
    providerId: providerId ?? null,
    useMcp,
    locale,
  });

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
  locale: Locale = "zh-TW",
): Promise<AiCommandReady> {
  return invoke<AiCommandReady>("ai_query", { query, sessionId, locale });
}


/**
 * True for a 429 whose body carries no real explanation — Anthropic returns
 * `{"error":{"type":"rate_limit_error","message":"Error"}}` when it refuses a
 * request for a reason it won't disclose (e.g. subscription-OAuth traffic that
 * doesn't look like Claude Code), including on the very first call. Calling that
 * "too many requests" points debugging at quota, which is the wrong place.
 */
function isOpaqueRateLimit(body?: string | null): boolean {
  return !!body && /"message"\s*:\s*"Error"/.test(body);
}

export function formatAiError(e: AiError): string {
  const t = getT();
  switch (e.kind) {
    case "not_configured":
      return t.ai_err_not_configured;
    case "secret_access":
      return t.ai_err_secret_access(e.message);
    case "network":
      if (
        e.message?.toLowerCase().includes("ollama") ||
        e.message?.toLowerCase().includes("connection refused") ||
        e.message?.toLowerCase().includes("connect error")
      ) {
        return t.ai_err_ollama_failed;
      }
      return t.ai_err_network(e.message ?? "");
    case "auth_failed":
      return t.ai_err_auth_failed;
    case "rate_limit": {
      const base = isOpaqueRateLimit(e.body)
        ? t.ai_err_rate_limit_opaque
        : e.retry_after
          ? t.ai_err_rate_limit_secs(Number(e.retry_after))
          : t.ai_err_rate_limit;
      return e.body ? `${base}\n${e.body}` : base;
    }
    case "model_error":
      return t.ai_err_format(e.reason ?? "");
    case "invalid_input":
      return t.ai_err_invalid_input(e.reason ?? "");
  }
}
