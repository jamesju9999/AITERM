import { invoke } from "@tauri-apps/api/core";
import type { ProviderType } from "./config";

// ── Types ─────────────────────────────────────────────────────────────────────

/** Frontend view of a provider — never contains the raw API key. */
export interface ProviderInfo {
  id: string;
  display_name: string;
  provider_type: ProviderType;
  base_url: string | null;
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

/** Check if a provider has an API key in the keychain. */
export const hasApiKey = (providerId: string): Promise<boolean> =>
  invoke("has_api_key", { providerId });

// ── Display helpers ───────────────────────────────────────────────────────────

export const PROVIDER_TYPE_LABELS: Record<ProviderType, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  ollama: "Ollama",
  "openai-compatible": "OpenAI-Compatible",
};

export const DEFAULT_MODELS: Record<ProviderType, string> = {
  openai: "gpt-4o-mini",
  anthropic: "claude-sonnet-4-5",
  ollama: "llama3.1:8b",
  "openai-compatible": "",
};

export const DEFAULT_BASE_URLS: Record<ProviderType, string> = {
  openai: "",
  anthropic: "",
  ollama: "http://localhost:11434",
  "openai-compatible": "",
};

/** OpenAI-compatible quick-pick presets shown in the form. */
export const COMPATIBLE_PRESETS = [
  { label: "LM Studio", url: "http://localhost:1234/v1" },
  { label: "vLLM", url: "http://localhost:8000/v1" },
  { label: "OpenRouter", url: "https://openrouter.ai/api/v1" },
  { label: "DeepSeek", url: "https://api.deepseek.com/v1" },
];
