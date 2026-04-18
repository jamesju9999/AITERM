# AITerm M2 — Multi-Provider + Settings UI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Users can add, edit, remove, and switch between multiple AI providers entirely through a Settings UI. API keys are stored in the OS keychain. A first-run onboarding wizard guides new users to configure their first provider. The router resolves the active provider from persisted config instead of a hardcoded env var.

**Architecture:** The backend gains three new AI provider implementations (`AnthropicClient`, `OllamaClient`, `OpenAiCompatibleClient`), a TOML-based `ConfigStore`, and a `SecretStore` wrapping the OS keychain. The `AiRouter` is rewritten to resolve the active provider by id from `ConfigStore` + `SecretStore` at query time. The frontend gains a `SettingsView` (accessible via `Ctrl+,` or gear icon) and an `OnboardingWizard` (shown on first launch).

**Tech Stack additions:**
- Rust: `toml 0.8`, `keyring 3` (OS keychain), `dirs 6` (platform config path)
- React: `react-router-dom 7` (page routing for settings vs terminal)

**Working directory:** `D:\Tool\AITerm`

---

## File Structure

### New files (backend)
- `src-tauri/src/ai/anthropic.rs` — `AnthropicClient` (Anthropic Messages API, SSE streaming)
- `src-tauri/src/ai/ollama.rs` — `OllamaClient` (Ollama `/api/chat` streaming)
- `src-tauri/src/ai/compatible.rs` — `OpenAiCompatibleClient` (wraps OpenAI-format with custom base_url)
- `src-tauri/src/config/mod.rs` — `ConfigStore` (TOML read/write/watch, atomic save)
- `src-tauri/src/config/types.rs` — `AppConfig`, `ProviderConfig`, `ProviderType` serde types
- `src-tauri/src/secret/mod.rs` — `SecretStore` (OS keychain CRUD via `keyring`)
- `src-tauri/src/commands/config.rs` — Tauri commands for config CRUD
- `src-tauri/src/commands/secret.rs` — Tauri commands for secret CRUD
- `src-tauri/src/commands/provider.rs` — Tauri commands: list providers, test connection, switch default
- `src-tauri/tests/anthropic_client.rs` — wiremock contract test
- `src-tauri/tests/ollama_client.rs` — wiremock contract test
- `src-tauri/tests/compatible_client.rs` — wiremock contract test
- `src-tauri/tests/config_store.rs` — round-trip, atomic write, corrupt recovery

### New files (frontend)
- `src/components/Settings/SettingsView.tsx` — Settings page shell with sidebar nav
- `src/components/Settings/SettingsView.css`
- `src/components/Settings/ProvidersPage.tsx` — Provider list + add/edit/remove + test connection
- `src/components/Settings/ProvidersPage.css`
- `src/components/Settings/ProviderForm.tsx` — Dynamic form per provider type
- `src/components/Settings/GeneralPage.tsx` — Execution mode selector (placeholder for future)
- `src/components/Onboarding/OnboardingWizard.tsx` — 3-step first-run wizard
- `src/components/Onboarding/OnboardingWizard.css`
- `src/ipc/config.ts` — Tauri invoke wrappers for config
- `src/ipc/secret.ts` — Tauri invoke wrappers for secret
- `src/ipc/provider.ts` — Tauri invoke wrappers for provider management

### Modified files
- `src-tauri/Cargo.toml` — add `toml`, `keyring`, `dirs`
- `src-tauri/src/lib.rs` — register new modules, managed state, invoke handlers
- `src-tauri/src/ai/mod.rs` — export new provider modules, add `health_check` to trait
- `src-tauri/src/ai/router.rs` — rewrite to resolve from config + secret store
- `src-tauri/src/commands/mod.rs` — export new command modules
- `src/App.tsx` — add routing: terminal view vs settings view vs onboarding
- `src/App.css` — layout adjustments for settings/onboarding
- `package.json` — add `react-router-dom`

---

## Phase 1: Backend Foundation — Config + Secret Store

### Task 1.1: Add Cargo dependencies

- [ ] Add to `[dependencies]` in `Cargo.toml`:
  ```toml
  toml = "0.8"
  keyring = { version = "3", features = ["apple-native", "windows-native", "sync-secret-service"] }
  dirs = "6"
  ```
- [ ] Run `cargo check` to verify resolution

### Task 1.2: Config types (`src-tauri/src/config/types.rs`)

Define the TOML-serializable config schema. Matches spec §5.3.

- [ ] Create `config/types.rs` with:
  ```rust
  use serde::{Deserialize, Serialize};

  #[derive(Debug, Clone, Serialize, Deserialize, Default)]
  pub struct AppConfig {
      #[serde(default)]
      pub default_provider: Option<String>,
      #[serde(default)]
      pub providers: Vec<ProviderConfig>,
      #[serde(default)]
      pub execution_mode: ExecutionMode,
      #[serde(default)]
      pub onboarding_done: bool,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct ProviderConfig {
      pub id: String,
      pub display_name: String,
      #[serde(rename = "type")]
      pub provider_type: ProviderType,
      #[serde(default)]
      pub base_url: Option<String>,
      pub model: String,
      // api_key is NOT stored here — it goes to SecretStore under key "aiterm:{id}"
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
  #[serde(rename_all = "kebab-case")]
  pub enum ProviderType {
      #[default]
      Openai,
      Anthropic,
      Ollama,
      OpenaiCompatible,
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
  #[serde(rename_all = "kebab-case")]
  pub enum ExecutionMode {
      #[default]
      AlwaysConfirm,
      Graded,
      FullAuto,
  }
  ```
- [ ] Unit tests: round-trip serialization, default values, provider type kebab-case

### Task 1.3: ConfigStore (`src-tauri/src/config/mod.rs`)

Manages `%APPDATA%/AITerm/config.toml` (Windows) / `~/.config/aiterm/config.toml` (Unix). Spec §6.6.

- [ ] Create `config/mod.rs`:
  ```rust
  pub mod types;
  pub use types::*;
  ```
- [ ] Implement `ConfigStore` struct:
  - `path: PathBuf` (resolved via `dirs::config_dir()`)
  - `config: parking_lot::RwLock<AppConfig>` (in-memory cache)
- [ ] `ConfigStore::new()` — resolve path, create dirs, load or init defaults
- [ ] `ConfigStore::load()` — read TOML file, parse, return `AppConfig`. If file missing or corrupt, return defaults and log warning.
- [ ] `ConfigStore::save()` — atomic write (write to `.tmp`, rename to target). Serialize with `toml::to_string_pretty`.
- [ ] `ConfigStore::get()` — return cloned `AppConfig`
- [ ] `ConfigStore::update(f: impl FnOnce(&mut AppConfig))` — apply mutation, save, return result
- [ ] `ConfigStore::get_provider(&self, id: &str) -> Option<ProviderConfig>` — find by id
- [ ] Register as Tauri managed state in `lib.rs`
- [ ] Unit tests: load missing file → defaults, round-trip save/load, atomic write (verify no partial writes)

### Task 1.4: SecretStore (`src-tauri/src/secret/mod.rs`)

Wraps OS keychain. Keys are namespaced as `aiterm:{provider_id}`. Spec §9.

- [ ] Create `secret/mod.rs`
- [ ] Implement `SecretStore`:
  ```rust
  pub struct SecretStore {
      service_name: String, // "aiterm"
  }
  ```
- [ ] `SecretStore::new()` — set service_name = "aiterm"
- [ ] `SecretStore::set(id: &str, secret: &str) -> Result<()>` — `keyring::Entry::new("aiterm", id)?.set_password(secret)`
- [ ] `SecretStore::get(id: &str) -> Result<Option<String>>` — return None on NoEntry, Err on other failures
- [ ] `SecretStore::delete(id: &str) -> Result<()>` — delete, ignore NoEntry
- [ ] Register as Tauri managed state in `lib.rs`
- [ ] Unit test: set/get/delete round-trip (tests run with real keychain — document this)
- [ ] Update `lib.rs`: `pub mod config; pub mod secret;`, add `.manage(ConfigStore::new())`, `.manage(SecretStore::new())`

---

## Phase 2: AI Provider Implementations

### Task 2.1: Add `health_check` to `AiProvider` trait

- [ ] Add to trait in `ai/mod.rs`:
  ```rust
  async fn health_check(&self) -> Result<(), AiError>;
  ```
- [ ] Implement on `OpenAiClient`: send a minimal completions request (1 max_token) to validate key. On success return Ok, on error return the classified error.
- [ ] Update existing tests if needed

### Task 2.2: AnthropicClient (`src-tauri/src/ai/anthropic.rs`)

Anthropic Messages API with SSE. Spec §5.2.

- [ ] Create `anthropic.rs`
- [ ] Struct: `AnthropicClient { api_key, model, base_url, client }`
- [ ] `new(api_key, model)` — default base_url `https://api.anthropic.com`
- [ ] `with_base_url(api_key, model, base_url)` — for wiremock tests
- [ ] Implement `AiProvider`:
  - `id()` → provider id from config
  - `display_name()` → "Anthropic"
  - `generate()`:
    - POST to `{base_url}/v1/messages`
    - Headers: `x-api-key`, `anthropic-version: 2023-06-01`, `content-type: application/json`
    - Body: `{ model, system, messages: [{role, content}], max_tokens, stream: true }`
    - Note: Anthropic puts `system` as top-level field, NOT in messages array
    - SSE events: `content_block_delta` → extract `delta.text`, `message_stop` → done
    - Error mapping: 401→AuthFailed, 429→RateLimit, 529→Network("overloaded")
  - `health_check()` → minimal request (max_tokens: 1, message: "hi")
- [ ] Register in `ai/mod.rs`: `pub mod anthropic;`
- [ ] Unit tests: request body construction, SSE payload parsing
- [ ] Integration test `tests/anthropic_client.rs`: wiremock contract tests (happy path SSE, 401, 429, 529)

### Task 2.3: OllamaClient (`src-tauri/src/ai/ollama.rs`)

Ollama chat API with NDJSON streaming. Spec §5.2.

- [ ] Create `ollama.rs`
- [ ] Struct: `OllamaClient { model, base_url, client }`
  - No api_key field — Ollama is local, no auth needed
- [ ] `new(model)` — default base_url `http://localhost:11434`
- [ ] `with_base_url(model, base_url)` — for tests
- [ ] Implement `AiProvider`:
  - `generate()`:
    - POST to `{base_url}/api/chat`
    - Body: `{ model, messages: [{role, content}], stream: true }`
    - Note: Ollama uses NDJSON (newline-delimited JSON), NOT SSE
    - Each line: `{ "message": { "content": "..." }, "done": false }` / `{ "done": true }`
    - On connection refused → `AiError::Network` with message "Ollama is not running"
  - `health_check()` → `GET {base_url}/api/tags` (returns 200 if running)
- [ ] Add `LocalEngineDown` variant to `AiError` enum (spec §5.4) — or reuse Network with a specific message pattern the frontend can match
- [ ] Register in `ai/mod.rs`: `pub mod ollama;`
- [ ] Unit tests: NDJSON parsing, request body
- [ ] Integration test `tests/ollama_client.rs`: wiremock contract tests (happy path NDJSON, connection refused simulation)

### Task 2.4: OpenAiCompatibleClient (`src-tauri/src/ai/compatible.rs`)

Reuses OpenAI SSE format but with arbitrary base_url and optional api_key. Spec §5.2.

- [ ] Create `compatible.rs`
- [ ] Struct: `OpenAiCompatibleClient { api_key: Option<String>, model, base_url, client }`
- [ ] `new(base_url, model, api_key)` — api_key is optional (some local servers don't need it)
- [ ] Implement `AiProvider`:
  - `generate()` — identical to OpenAiClient but:
    - Uses configurable base_url
    - Bearer auth only if api_key is Some
    - `response_format: json_object` only if the server supports it (add a `supports_json_mode: bool` config field, default true)
  - `health_check()` → same as OpenAI health check, adapted for the custom endpoint
- [ ] Register in `ai/mod.rs`: `pub mod compatible;`
- [ ] Integration test `tests/compatible_client.rs`: wiremock (same SSE format as OpenAI)

---

## Phase 3: Router Rewrite

### Task 3.1: Rewrite AiRouter to resolve from ConfigStore + SecretStore

The current router is hardcoded to read `OPENAI_API_KEY` from env. Rewrite it to:

- [ ] Change `AiRouter` to hold `Arc<ConfigStore>` + `Arc<SecretStore>` instead of a single provider
- [ ] New signature:
  ```rust
  pub struct AiRouter {
      config: Arc<ConfigStore>,
      secrets: Arc<SecretStore>,
  }
  ```
- [ ] `AiRouter::new(config, secrets)` — constructor
- [ ] `AiRouter::resolve(&self) -> Result<Arc<dyn AiProvider>, AiError>` — reads default_provider from config, looks up ProviderConfig, fetches api_key from SecretStore, constructs the appropriate client. Cache the constructed provider (invalidate when config changes).
  - If no default_provider set and no providers configured → `AiError::NotConfigured`
  - If provider id not found → `AiError::NotConfigured` with descriptive message
  - Match on `provider_type`: Openai, Anthropic, Ollama, OpenaiCompatible → construct the right client
  - Ollama doesn't need a secret; others do (except OpenaiCompatible where it's optional)
- [ ] `AiRouter::resolve_by_id(&self, id: &str) -> Result<Arc<dyn AiProvider>, AiError>` — for testing specific providers
- [ ] Backward compat: if no config file exists yet AND `OPENAI_API_KEY` env var is set, use it as a fallback (smooth transition from M1)
- [ ] Update `commands/ai.rs` to use `router.resolve()` instead of `router.require_provider()`
- [ ] Update `lib.rs`: construct `AiRouter::new(config_store.clone(), secret_store.clone())`
- [ ] Tests: missing config → NotConfigured, valid config resolves correct provider type, env var fallback

---

## Phase 4: Tauri Commands for Config/Secret/Provider Management

### Task 4.1: Config commands (`src-tauri/src/commands/config.rs`)

- [ ] `get_config() -> AppConfig` — return current config
- [ ] `set_execution_mode(mode: ExecutionMode)` — update and save
- [ ] `is_onboarding_done() -> bool` — check flag
- [ ] `set_onboarding_done()` — set flag and save
- [ ] Register all in `lib.rs` invoke handler

### Task 4.2: Provider management commands (`src-tauri/src/commands/provider.rs`)

- [ ] `list_providers() -> Vec<ProviderInfo>` — return list with id, display_name, type, model, has_api_key (bool from SecretStore)
- [ ] `add_provider(config: ProviderConfig, api_key: Option<String>)` — add to config, store key in keychain
- [ ] `update_provider(id: String, config: ProviderConfig, api_key: Option<String>)` — update config + optionally update key
- [ ] `remove_provider(id: String)` — remove from config + delete key from keychain
- [ ] `set_default_provider(id: String)` — update default_provider in config
- [ ] `test_provider(id: String) -> Result<String, AiError>` — call health_check on the resolved provider, return "ok" or error
- [ ] `get_ollama_models(base_url: Option<String>) -> Result<Vec<String>, AiError>` — call Ollama `/api/tags` to list available models (for dynamic form dropdown)
- [ ] Register all in `lib.rs` invoke handler

### Task 4.3: Secret commands (`src-tauri/src/commands/secret.rs`)

- [ ] `has_api_key(provider_id: String) -> bool` — check if keychain has a secret for this provider
- [ ] `delete_api_key(provider_id: String)` — remove from keychain
- [ ] Note: `set` is handled through `add_provider`/`update_provider`, never exposed directly
- [ ] Register in `lib.rs`

---

## Phase 5: Frontend — Settings UI

### Task 5.1: Add react-router-dom and app routing

- [ ] `npm install react-router-dom`
- [ ] Refactor `App.tsx` to use `BrowserRouter` (or `MemoryRouter` for Tauri):
  ```tsx
  <MemoryRouter>
    <Routes>
      <Route path="/" element={<TerminalView />} />
      <Route path="/settings/*" element={<SettingsView />} />
      <Route path="/onboarding" element={<OnboardingWizard />} />
    </Routes>
  </MemoryRouter>
  ```
- [ ] On mount: check `is_onboarding_done()` → if false, redirect to `/onboarding`
- [ ] Add keyboard listener for `Ctrl+,` → navigate to `/settings`
- [ ] Add a small gear icon button (absolute positioned, top-right of terminal) → navigate to `/settings`

### Task 5.2: IPC wrappers (`src/ipc/config.ts`, `src/ipc/provider.ts`)

- [ ] `config.ts`:
  - `getConfig()`, `setExecutionMode()`, `isOnboardingDone()`, `setOnboardingDone()`
  - TypeScript types mirroring Rust: `AppConfig`, `ProviderConfig`, `ProviderType`, `ExecutionMode`
- [ ] `provider.ts`:
  - `listProviders()`, `addProvider()`, `updateProvider()`, `removeProvider()`
  - `setDefaultProvider()`, `testProvider()`, `getOllamaModels()`
  - `ProviderInfo` type

### Task 5.3: SettingsView (`src/components/Settings/SettingsView.tsx`)

- [ ] Left sidebar with navigation items: "AI Providers", "General"
- [ ] Right content area renders the selected page
- [ ] Back button / `Esc` → navigate back to `/`
- [ ] Route structure: `/settings` → default to providers page, `/settings/general` → general page
- [ ] Minimal styling: dark theme consistent with terminal

### Task 5.4: ProvidersPage (`src/components/Settings/ProvidersPage.tsx`)

The main settings page. Lists configured providers, allows CRUD.

- [ ] On mount: call `listProviders()` to populate list
- [ ] Display each provider as a card/row: icon (by type) + display_name + model + health indicator
- [ ] Each row has: "Test" button (calls `testProvider`), "Edit" button, "Remove" button
- [ ] Top: "Default Provider" dropdown — calls `setDefaultProvider` on change
- [ ] "+ Add Provider" button → opens `ProviderForm` in add mode
- [ ] Edit button → opens `ProviderForm` in edit mode with pre-filled values

### Task 5.5: ProviderForm (`src/components/Settings/ProviderForm.tsx`)

Dynamic form that changes fields based on provider type. Spec §6.4.

- [ ] Provider type selector (dropdown): OpenAI, Anthropic, Ollama, OpenAI-Compatible
- [ ] Common fields: Display Name (auto-generated from type if blank), Model
- [ ] **OpenAI**: API Key (password input), Model (text input, default "gpt-4o-mini")
- [ ] **Anthropic**: API Key (password input), Model (text input, default "claude-sonnet-4-5")
- [ ] **Ollama**: Base URL (default "http://localhost:11434"), Model (dropdown populated from `getOllamaModels`, with fallback to text input if Ollama unreachable)
- [ ] **OpenAI-Compatible**: Base URL (required), API Key (optional), Model (text input), quick-pick presets: LM Studio (`http://localhost:1234/v1`), vLLM, OpenRouter, DeepSeek
- [ ] "Test Connection" button — calls `testProvider`, shows success/error inline
- [ ] "Save" button — calls `addProvider` or `updateProvider`
- [ ] "Cancel" button — closes form
- [ ] Validation: id must be unique, required fields non-empty

### Task 5.6: GeneralPage (`src/components/Settings/GeneralPage.tsx`)

- [ ] Execution mode selector: radio buttons for "Always Confirm", "Graded Auto", "Full Auto Agent"
  - Brief description of each mode below the radio
- [ ] Calls `setExecutionMode` on change
- [ ] This is a placeholder for future settings (theme, shell, etc.)

---

## Phase 6: Onboarding Wizard

### Task 6.1: OnboardingWizard (`src/components/Onboarding/OnboardingWizard.tsx`)

3-step wizard shown on first launch. Spec §6.5.

- [ ] Step 1: Welcome
  - "Welcome to AITerm" heading
  - Brief feature overview (3 bullet points: natural language commands, multiple AI backends, command safety)
  - "Get Started" button → next step
- [ ] Step 2: Add Your First Provider
  - Embedded `ProviderForm` component (reuse from Task 5.5)
  - "Skip for now" link at bottom → next step (sets no default provider)
  - On successful save → next step
- [ ] Step 3: Choose Execution Mode
  - Same radio group as GeneralPage (reuse component)
  - Default: "Always Confirm" pre-selected
  - "Finish" button → call `setOnboardingDone()`, navigate to `/`
- [ ] Step indicator (1/3, 2/3, 3/3) at top
- [ ] Styling: centered card layout, clean and welcoming

---

## Phase 7: Integration & Polish

### Task 7.1: Provider status in terminal view

- [ ] Show current active provider name in a subtle status area (bottom-right or top bar)
- [ ] Quick-switch shortcut `Ctrl+Shift+P`: show a small overlay listing providers, click/arrow-select to switch default
  - Calls `setDefaultProvider` + `listProviders` to refresh

### Task 7.2: Error UX improvements

- [ ] When `/ai` returns `NotConfigured` → show a message with "Open Settings" button (navigate to `/settings`)
- [ ] When Ollama returns connection error → show "Ollama is not running. Start Ollama or switch to a cloud provider." with action buttons
- [ ] When auth fails → "API key is invalid. Open Settings to update it."

### Task 7.3: End-to-end testing

- [ ] Manual test: fresh install → onboarding → add OpenAI provider → `/ai list files` → command preview → execute
- [ ] Manual test: add Ollama provider → switch default → `/ai` works with local model
- [ ] Manual test: add OpenAI-Compatible (e.g. LM Studio) → works
- [ ] Manual test: remove provider → provider disappears from list and dropdown
- [ ] Manual test: test connection on invalid key → shows auth error
- [ ] Manual test: test connection on dead Ollama → shows "not running" error
- [ ] Verify: API keys never appear in config.toml
- [ ] Verify: settings persist across app restart

---

## Dependency Graph

```
Phase 1 (Config + Secret)
   │
   ├── Phase 2 (Provider Implementations)  ← can start 2.2-2.4 in parallel
   │       │
   │       └── Phase 3 (Router Rewrite)
   │               │
   │               └── Phase 4 (Tauri Commands)
   │                       │
   │                       ├── Phase 5 (Settings UI)  ← needs commands
   │                       │
   │                       └── Phase 6 (Onboarding)   ← needs ProviderForm from 5.5
   │
   └── Phase 7 (Integration)  ← after everything
```

Phases 2.2, 2.3, 2.4 (the three provider implementations) are independent of each other and can be developed in parallel. Phase 5 and 6 share the `ProviderForm` component, so 5.5 should complete before 6.1.

---

## Key Design Decisions

**D1: Provider resolution is lazy (per-query), not eager.**
The router does not hold a pre-constructed provider. It resolves from config+secrets on each `ai_query` call. This means config changes take effect immediately without restarting the app or rebuilding state. The cost is one keychain read per query, which is negligible (~1ms).

**D2: Config changes trigger no events — the frontend re-fetches.**
After any config mutation (add/remove/update provider, change default), the frontend re-fetches the provider list. No need for a Tauri event system for config changes — the UI is the only writer.

**D3: OpenAiCompatibleClient is separate from OpenAiClient.**
Although they share SSE format, keeping them separate avoids complicating `OpenAiClient` with optional fields (optional api_key, optional json_mode). The SSE consumption function `consume_sse` is extracted to a shared util.

**D4: Env var fallback for M1→M2 migration.**
If `config.toml` doesn't exist and `OPENAI_API_KEY` is in the environment, the router auto-creates a transient OpenAI provider. This prevents breaking existing M1 development setups. The fallback is removed in M3.

**D5: Onboarding state is a simple boolean in config.toml.**
`onboarding_done = true` means the wizard won't show again. Users can reset this by deleting the line from config.toml or adding a "Re-run Setup" button later.

**D6: MemoryRouter, not BrowserRouter.**
Tauri loads `index.html` from a custom protocol, not `http://localhost`. `MemoryRouter` works without a real URL bar and avoids routing issues with Tauri's asset protocol.
