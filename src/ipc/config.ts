import { invoke } from "@tauri-apps/api/core";

// ── Types (mirrors Rust config/types.rs) ──────────────────────────────────────

export type ProviderType =
  | "openai"
  | "anthropic"
  | "ollama"
  | "openai-compatible";

export type ExecutionMode = "always-confirm" | "graded" | "full-auto";

export interface ProviderConfig {
  id: string;
  display_name: string;
  provider_type: ProviderType;
  base_url: string | null;
  model: string;
  supports_json_mode: boolean;
}

export interface AppConfig {
  default_provider: string | null;
  providers: ProviderConfig[];
  execution_mode: ExecutionMode;
  onboarding_done: boolean;
}

// ── Commands ──────────────────────────────────────────────────────────────────

export const getConfig = (): Promise<AppConfig> => invoke("get_config");

export const setExecutionMode = (mode: ExecutionMode): Promise<void> =>
  invoke("set_execution_mode", { mode });

export const isOnboardingDone = (): Promise<boolean> =>
  invoke("is_onboarding_done");

export const setOnboardingDone = (): Promise<void> =>
  invoke("set_onboarding_done");
