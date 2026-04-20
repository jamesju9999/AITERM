import { invoke } from "@tauri-apps/api/core";
import type { ProviderType } from "./config";

// ── Types ─────────────────────────────────────────────────────────────────────

/** Frontend view of a provider — never contains the raw API key. */
export interface ProviderInfo {
  id: string;
  display_name: string;
  provider_type: ProviderType;
  base_url: string | null;
  oauth_client_id: string | null;
  model: string;
  supports_json_mode: boolean;
  has_api_key: boolean;
  is_default: boolean;
}

/** Payload for add / update operations. api_key=null means "don't change". */
export interface ProviderInput {
  id: string;
  display_name: string;
  provider_type: ProviderType;
  base_url: string | null;
  oauth_client_id: string | null;
  model: string;
  supports_json_mode: boolean;
  api_key: string | null;
}

export interface AiError {
  kind:
    | "not_configured"
    | "network"
    | "auth_failed"
    | "rate_limit"
    | "model_error";
  message?: string;
  reason?: string;
  retry_after?: string | null;
}

export interface GithubDeviceStartResponse {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export type GithubDevicePollResponse =
  | { status: "authorized"; access_token: string }
  | { status: "authorization_pending" }
  | { status: "slow_down" }
  | { status: "access_denied"; message: string }
  | { status: "expired_token"; message: string }
  | { status: "error"; message: string };

// ── Commands ──────────────────────────────────────────────────────────────────

export const listProviders = (): Promise<ProviderInfo[]> =>
  invoke("list_providers");

export const addProvider = (input: ProviderInput): Promise<void> =>
  invoke("add_provider", { input });

export const updateProvider = (input: ProviderInput): Promise<void> =>
  invoke("update_provider", { input });

export const removeProvider = (id: string): Promise<void> =>
  invoke("remove_provider", { id });

export const setDefaultProvider = (id: string): Promise<void> =>
  invoke("set_default_provider", { id });

/** Returns "ok" on success, throws AiError on failure. */
export const testProvider = (id: string): Promise<string> =>
  invoke("test_provider", { id });

/** Returns list of locally available Ollama model names. */
export const getOllamaModels = (baseUrl?: string): Promise<string[]> =>
  invoke("get_ollama_models", { baseUrl: baseUrl ?? null });

export const githubCopilotDeviceStart = (clientId?: string): Promise<GithubDeviceStartResponse> =>
  invoke("github_copilot_device_start", { clientId: clientId ?? null });

export const githubCopilotDevicePoll = (
  deviceCode: string,
  clientId?: string,
): Promise<GithubDevicePollResponse> =>
  invoke("github_copilot_device_poll", { clientId: clientId ?? null, deviceCode });

export const getGithubCopilotModels = (
  accessToken: string,
  baseUrl?: string,
): Promise<string[]> =>
  invoke("get_github_copilot_models", { accessToken, baseUrl: baseUrl ?? null });

export const getGithubCopilotModelsByProvider = (
  id: string,
  baseUrl?: string,
): Promise<string[]> =>
  invoke("get_github_copilot_models_by_provider", { id, baseUrl: baseUrl ?? null });

export const googleGeminiOauthAuth = (
  providerId: string,
  clientId: string,
  clientSecret: string,
): Promise<string> =>
  invoke("google_gemini_oauth_auth", { providerId, clientId, clientSecret });

/** Check if a provider has an API key in the keychain. */
export const hasApiKey = (providerId: string): Promise<boolean> =>
  invoke("has_api_key", { providerId });

// ── Display helpers ───────────────────────────────────────────────────────────

export const PROVIDER_TYPE_LABELS: Record<ProviderType, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  ollama: "Ollama",
  "openai-compatible": "OpenAI-Compatible",
  "github-copilot": "GitHub Copilot",
  "google-ai": "Google AI (API Key)",
  "google-gemini-oauth": "Google Gemini (OAuth)",
};

export const DEFAULT_MODELS: Record<ProviderType, string> = {
  openai: "gpt-4o-mini",
  anthropic: "claude-sonnet-4-5",
  ollama: "llama3.1:8b",
  "openai-compatible": "",
  "github-copilot": "gpt-4o-mini",
  "google-ai": "gemini-2.5-pro",
  "google-gemini-oauth": "gemini-2.5-pro",
};

export const DEFAULT_BASE_URLS: Record<ProviderType, string> = {
  openai: "",
  anthropic: "",
  ollama: "http://localhost:11434",
  "openai-compatible": "",
  "github-copilot": "https://api.githubcopilot.com",
  "google-ai": "https://generativelanguage.googleapis.com/v1beta/openai",
  "google-gemini-oauth": "https://generativelanguage.googleapis.com/v1beta/openai",
};

/** OpenAI-compatible quick-pick presets shown in the form. */
export const COMPATIBLE_PRESETS = [
  { label: "LM Studio", url: "http://localhost:1234/v1" },
  { label: "vLLM", url: "http://localhost:8000/v1" },
  { label: "OpenRouter", url: "https://openrouter.ai/api/v1" },
  { label: "DeepSeek", url: "https://api.deepseek.com/v1" },
];
