import { invoke } from "@tauri-apps/api/core";

export type RiskLevel = "safe" | "needs_confirm" | "dangerous";

export type AiError =
  | { kind: "not_configured" }
  | { kind: "network"; message: string }
  | { kind: "auth_failed" }
  | { kind: "rate_limit"; retry_after: string | null }
  | { kind: "model_error"; reason: string; raw: string };

export interface AiCommandReady {
  command: string;
  explanation: string;
  risk_level: RiskLevel;
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
  }
}
