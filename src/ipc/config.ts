import { invoke } from "@tauri-apps/api/core";

// ── Types (mirrors Rust config/types.rs) ──────────────────────────────────────

export type ProviderType =
  | "openai"
  | "anthropic"
  | "ollama"
  | "openai-compatible"
  | "github-copilot"
  | "google-ai"
  | "openrouter"
  | "xai"
  | "deepseek"
  | "kimi"
  | "anthropic-compatible"
  | "codex";

export type ExecutionMode = "always-confirm" | "graded" | "full-auto";

export type SubmitShortcut = "enter" | "shift-enter" | "ctrl-enter";

export type DefaultTab = "terminal" | "database";

export interface ProviderConfig {
  id: string;
  display_name: string;
  provider_type: ProviderType;
  base_url: string | null;
  oauth_client_id: string | null;
  model: string;
  supports_json_mode: boolean;
}

export interface EnterprisePolicy {
  version: number;
  ai_provider_id: string | null;
  execution_mode: ExecutionMode | null;
  max_agent_steps: number | null;
  vcs_push_pattern: string | null;
}

export interface AppConfig {
  default_provider: string | null;
  providers: ProviderConfig[];
  execution_mode: ExecutionMode;
  submit_shortcut: SubmitShortcut;
  onboarding_done: boolean;
  max_agent_steps: number; // 0 = unlimited
  default_tab: DefaultTab;
  enterprise_server_url: string | null;
  enterprise_device_id: string | null;
  enterprise_policy: EnterprisePolicy | null;
  mcp_enabled?: boolean;
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

export const setDefaultTab = (tab: DefaultTab): Promise<void> =>
  invoke("set_default_tab", { tab });

export type AppImageIntegrationState =
  | { state: "not_appimage" }
  | { state: "available" }
  | { state: "integrated"; exec_path: string };

export const appimageIntegrationState = (): Promise<AppImageIntegrationState> =>
  invoke<AppImageIntegrationState>("appimage_integration_state");

export const appimageIntegrate = (): Promise<void> =>
  invoke<void>("appimage_integrate");

export const appimageRemoveIntegration = (): Promise<void> =>
  invoke<void>("appimage_remove_integration");

export const isAppImageIntegrationDeclined = (): Promise<boolean> =>
  invoke<boolean>("is_appimage_integration_declined");

export const setAppImageIntegrationDeclined = (): Promise<void> =>
  invoke<void>("set_appimage_integration_declined");
