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
  auth_method: string | null;
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
  auth_method: string | null;
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

export const getGoogleAiModels = (apiKey: string): Promise<string[]> =>
  invoke("get_google_ai_models", { apiKey });

export const getGoogleAiModelsByProvider = (id: string): Promise<string[]> =>
  invoke("get_google_ai_models_by_provider", { id });

/** Check if a provider has an API key in the keychain. */
export const hasApiKey = (providerId: string): Promise<boolean> =>
  invoke("has_api_key", { providerId });

/** Open the browser to start the Anthropic OAuth flow. */
export const anthropicOAuthStart = (): Promise<void> =>
  invoke("anthropic_oauth_start");

/** Complete Anthropic OAuth by exchanging the code#state string for tokens. */
export const anthropicOAuthComplete = (
  providerId: string,
  codeAndState: string,
): Promise<void> =>
  invoke("anthropic_oauth_complete", { providerId, codeAndState });

/** Log out from Anthropic OAuth (clears stored tokens). */
export const anthropicOAuthLogout = (providerId: string): Promise<void> =>
  invoke("anthropic_oauth_logout", { providerId });

/** Fetch available Claude models using the stored OAuth token. */
export const getAnthropicOAuthModels = (providerId: string): Promise<string[]> =>
  invoke("get_anthropic_oauth_models", { providerId });

/** Start Google OAuth: opens browser, waits for callback, exchanges tokens. Blocks until done. */
export const googleOAuthLogin = (providerId: string): Promise<void> =>
  invoke("google_oauth_login", { providerId });

/** Log out from Google OAuth (clears stored tokens). */
export const googleOAuthLogout = (providerId: string): Promise<void> =>
  invoke("google_oauth_logout", { providerId });

/** Fetch available Gemini models using the stored Antigravity OAuth token. */
export const getGoogleOAuthModels = (providerId: string): Promise<string[]> =>
  invoke("get_google_oauth_models", { providerId });

export const getOpenRouterModels = (apiKey: string): Promise<string[]> =>
  invoke("get_openrouter_models", { apiKey });

export const getOpenRouterModelsByProvider = (id: string): Promise<string[]> =>
  invoke("get_openrouter_models_by_provider", { id });

export const getXaiModels = (apiKey: string): Promise<string[]> =>
  invoke("get_xai_models", { apiKey });

export const getXaiModelsByProvider = (id: string): Promise<string[]> =>
  invoke("get_xai_models_by_provider", { id });

export const getDeepseekModels = (apiKey: string): Promise<string[]> =>
  invoke("get_deepseek_models", { apiKey });

export const getDeepseekModelsByProvider = (id: string): Promise<string[]> =>
  invoke("get_deepseek_models_by_provider", { id });

export const getKimiModels = (apiKey: string): Promise<string[]> =>
  invoke("get_kimi_models", { apiKey });

export const getKimiModelsByProvider = (id: string): Promise<string[]> =>
  invoke("get_kimi_models_by_provider", { id });

/** Start Codex OAuth: opens browser, waits for the localhost:1455 callback, exchanges tokens. Blocks until done. */
export const codexOAuthLogin = (providerId: string): Promise<void> =>
  invoke("codex_oauth_login", { providerId });

/** Log out from Codex OAuth (clears stored tokens + cached ChatGPT account id). */
export const codexOAuthLogout = (providerId: string): Promise<void> =>
  invoke("codex_oauth_logout", { providerId });

/** Fetch available Codex models using the stored OAuth token. */
export const getCodexOAuthModels = (providerId: string): Promise<string[]> =>
  invoke("get_codex_oauth_models", { providerId });

// ── Display helpers ───────────────────────────────────────────────────────────

export const PROVIDER_TYPE_LABELS: Record<ProviderType, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  ollama: "Ollama",
  "openai-compatible": "OpenAI-Compatible",
  "github-copilot": "GitHub Copilot",
  "google-ai": "Google AI",
  openrouter: "OpenRouter",
  xai: "xAI (Grok)",
  deepseek: "DeepSeek",
  kimi: "Kimi (Moonshot)",
  "anthropic-compatible": "Anthropic-Compatible",
  codex: "Codex (ChatGPT)",
};

export const DEFAULT_MODELS: Record<ProviderType, string> = {
  openai: "gpt-4o-mini",
  anthropic: "claude-sonnet-4-5",
  ollama: "llama3.1:8b",
  "openai-compatible": "",
  "github-copilot": "gpt-4o-mini",
  "google-ai": "gemini-2.5-pro",
  openrouter: "",
  xai: "grok-4",
  deepseek: "deepseek-chat",
  kimi: "kimi-latest",
  "anthropic-compatible": "",
  codex: "gpt-5.1-codex",
};

export const DEFAULT_BASE_URLS: Record<ProviderType, string> = {
  openai: "",
  anthropic: "",
  ollama: "http://localhost:11434",
  "openai-compatible": "",
  "github-copilot": "https://api.githubcopilot.com",
  "google-ai": "https://generativelanguage.googleapis.com/v1beta/openai",
  openrouter: "https://openrouter.ai/api/v1",
  xai: "https://api.x.ai/v1",
  deepseek: "https://api.deepseek.com/v1",
  kimi: "https://api.moonshot.ai/v1",
  "anthropic-compatible": "",
  codex: "",
};

/** OpenAI-compatible quick-pick presets shown in the form — for servers with
 *  no dedicated provider type (self-hosted / local). OpenRouter and DeepSeek
 *  used to be listed here but are now dedicated provider types above; keep
 *  this list to genuine "bring your own endpoint" cases only. */
export const COMPATIBLE_PRESETS = [
  { label: "LM Studio", url: "http://localhost:1234/v1" },
  { label: "vLLM", url: "http://localhost:8000/v1" },
];

/** Anthropic-compatible quick-pick presets shown in the form. */
export const ANTHROPIC_COMPATIBLE_PRESETS = [
  { label: "Kimi Coding", url: "https://api.kimi.com/coding" },
];
