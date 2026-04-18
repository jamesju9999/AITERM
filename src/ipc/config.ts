import { invoke } from "@tauri-apps/api/core";

// ── Types (mirrors Rust config/types.rs) ──────────────────────────────────────

export type ProviderType =
  | "openai"
  | "anthropic"
  | "ollama"
  | "openai-compatible";

export type ExecutionMode = "always-confirm" | "graded" | "full-auto";

export type SubmitShortcut = "enter" | "shift-enter" | "ctrl-enter";

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
  submit_shortcut: SubmitShortcut;
  onboarding_done: boolean;
  max_agent_steps: number; // 0 = unlimited
}

// ── Commands ──────────────────────────────────────────────────────────────────

export const getConfig = (): Promise<AppConfig> => invoke("get_config");

export const setExecutionMode = (mode: ExecutionMode): Promise<void> =>
  invoke("set_execution_mode", { mode });

export const isOnboardingDone = (): Promise<boolean> =>
  invoke("is_onboarding_done");

export const setOnboardingDone = (): Promise<void> =>
  invoke("set_onboarding_done");

export const setSubmitShortcut = (shortcut: SubmitShortcut): Promise<void> =>
  invoke("set_submit_shortcut", { shortcut });

export const setMaxAgentSteps = (steps: number): Promise<void> =>
  invoke("set_max_agent_steps", { steps });
