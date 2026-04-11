import { invoke } from "@tauri-apps/api/core";

export type AiError =
  | { kind: "not_configured" }
  | { kind: "network"; message: string }
  | { kind: "auth_failed" }
  | { kind: "rate_limit"; retry_after: string | null }
  | { kind: "model_error"; reason: string; raw: string };

export interface AiCommandReady {
  command: string;
  explanation: string;
}

export function invokeAiQuery(
  query: string,
  sessionId: string,
): Promise<AiCommandReady> {
  return invoke<AiCommandReady>("ai_query", { query, sessionId });
}

export function formatAiError(e: AiError): string {
  switch (e.kind) {
    case "not_configured":
      return "aiterm: OPENAI_API_KEY not set. Set the env var and restart AITerm.";
    case "network":
      return `aiterm: network error — ${e.message}`;
    case "auth_failed":
      return "aiterm: authentication failed. Check your OPENAI_API_KEY.";
    case "rate_limit":
      return e.retry_after
        ? `aiterm: rate limit exceeded (retry after ${e.retry_after})`
        : "aiterm: rate limit exceeded, try again later";
    case "model_error":
      return `aiterm: AI returned invalid response (${e.reason})\n        raw: ${e.raw}`;
  }
}
