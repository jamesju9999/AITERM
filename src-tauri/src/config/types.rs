//! TOML-serializable configuration types.
//!
//! The TOML file stores provider metadata (id, type, model, base_url).
//! API keys are NEVER stored here — they live in the OS keychain under
//! "aiterm:{provider_id}".

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// The id of the currently active provider.
    #[serde(default)]
    pub default_provider: Option<String>,

    /// All configured providers.
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,

    /// How the app handles AI-generated commands (spec §4.0).
    #[serde(default)]
    pub execution_mode: ExecutionMode,

    /// Maximum number of iterations for the autonomous agent loop (0 = unlimited).
    #[serde(default = "default_max_agent_steps")]
    pub max_agent_steps: u32,

    /// Set to true after the first-run onboarding wizard completes.
    #[serde(default)]
    pub onboarding_done: bool,

    /// Set when the user declines the AppImage menu-entry prompt, so it is
    /// asked once rather than on every launch. Settings still offers it.
    #[serde(default)]
    pub appimage_integration_declined: bool,

    /// Set when the user declines the Claude Code terminal-bell prompt, so it is
    /// asked once rather than on every `claude` run.
    #[serde(default)]
    pub claude_notif_declined: bool,

    /// Which shortcut submits the command (Enter vs Shift+Enter, etc).
    #[serde(default)]
    pub submit_shortcut: SubmitShortcut,
    /// Which engine document conversion prefers (anydoc vs MarkItDown-only).
    #[serde(default)]
    pub doc_convert_engine: DocConvertEngine,

    /// Saved database connections (passwords stored separately in Keychain).
    #[serde(default)]
    pub db_connections: Vec<DbConnection>,

    /// Which tab type to open automatically when the app starts.
    #[serde(default)]
    pub default_tab: DefaultTab,

    /// Telegram chat ID for remote control.
    #[serde(default)]
    pub telegram_chat_id: Option<String>,

    /// Saved VCS connections (tokens/passwords stored separately in Keychain).
    #[serde(default)]
    pub vcs_connections: Vec<VcsConnection>,

    /// Saved mail accounts (passwords stored separately in Keychain).
    #[serde(default)]
    pub mail_accounts: Vec<MailAccountConfig>,

    /// Enterprise Management Server URL. When set, enterprise mode is active.
    #[serde(default)]
    pub enterprise_server_url: Option<String>,

    /// Unique device identifier assigned by the Management Server on registration.
    #[serde(default)]
    pub enterprise_device_id: Option<String>,

    /// Policy pushed by the Management Server. Overrides local settings when present.
    #[serde(default)]
    pub enterprise_policy: Option<EnterprisePolicy>,

    /// Whether MCP tool calling is globally enabled.
    #[serde(default = "default_true")]
    pub mcp_enabled: bool,

    /// Configured MCP server connections.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,

    /// Interpreter the user pointed us at when uv can't fetch one (offline or
    /// behind a proxy). The venv is still created under app data — this only
    /// changes which interpreter it's based on.
    #[serde(default)]
    pub python_interpreter: Option<String>,

    /// Package index for the managed environment, for networks that block PyPI.
    /// uv reads neither pip.conf nor PIP_INDEX_URL, so a corporate mirror has
    /// no effect unless it's passed explicitly.
    #[serde(default)]
    pub python_index_url: Option<String>,

    /// Claude Code 橋接設定。舊的 config.toml 沒有這個區塊，靠 `default` 補齊。
    #[serde(default)]
    pub claude_bridge: ClaudeBridgeConfig,

    /// MCP tool server 設定。舊的 config.toml 沒有這個區塊，靠 `default` 補齊。
    #[serde(default)]
    pub mcp_tool_server: McpToolServerConfig,

    /// Task board settings. Absent from older config.toml — `default` fills in.
    #[serde(default)]
    pub task_board: TaskBoardConfig,
}

fn default_max_agent_steps() -> u32 { 5 }

/// 橋接 server 的預設埠。被占用時啟動失敗而非漂移 —— 環境變數只能在分頁
/// spawn 的瞬間決定，埠若會漂移，已開的分頁會指向死位址。
pub fn default_bridge_port() -> u16 { 8317 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeBridgeConfig {
    /// server 是否常駐。
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_bridge_port")]
    pub port: u16,

    /// 新開的終端機分頁是否預設注入橋接環境變數。
    #[serde(default)]
    pub default_on_new_tab: bool,

    #[serde(default)]
    pub opus: Option<TierMapping>,
    #[serde(default)]
    pub sonnet: Option<TierMapping>,
    #[serde(default)]
    pub haiku: Option<TierMapping>,
}

impl Default for ClaudeBridgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_bridge_port(),
            default_on_new_tab: false,
            opus: None,
            sonnet: None,
            haiku: None,
        }
    }
}

pub fn default_mcp_tool_server_port() -> u16 { 8318 }

/// Settings for AITerm's MCP tool server (exposes DB connections and
/// knowledge base notebooks as MCP tools to external clients like Claude
/// Code CLI). Independent from `ClaudeBridgeConfig` — different concern,
/// different toggle, different port. See
/// `docs/superpowers/specs/2026-08-19-mcp-tool-server-design.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolServerConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_mcp_tool_server_port")]
    pub port: u16,

    /// Separately gates the 4 agent-coordination tools (spawn_tab/send_input/
    /// get_tab_status/wait_for_idle) — a strictly higher-risk capability than
    /// the DB/knowledge-base tools (it can run arbitrary commands in tabs it
    /// spawns), so it defaults off even when the server itself is enabled.
    #[serde(default)]
    pub coordination_enabled: bool,
}

impl Default for McpToolServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_mcp_tool_server_port(),
            coordination_enabled: false,
        }
    }
}

pub fn default_task_board_max_concurrent() -> u32 { 2 }
pub fn default_claude_command() -> String { "claude".to_string() }

/// Settings for the task board (see
/// `docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBoardConfig {
    /// Global cap on tasks in the `running` column at once. A per-card
    /// `parallel_ok = false` flag further restricts (a solo card waits for
    /// an empty running set and blocks others while it runs).
    #[serde(default = "default_task_board_max_concurrent")]
    pub max_concurrent: u32,
    /// The CLI launched in each dispatched tab. `claude` by default; a user
    /// could point this at another agent, but that's not a supported feature.
    #[serde(default = "default_claude_command")]
    pub claude_command: String,
}

impl Default for TaskBoardConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_task_board_max_concurrent(),
            claude_command: default_claude_command(),
        }
    }
}

/// 一個 Claude Code 模型層級要打到哪個供應商的哪個模型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TierMapping {
    pub provider_id: String,
    pub model: String,
}

impl AppConfig {
    /// Find a provider by id.
    pub fn find_provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Replace or insert a provider. Returns false if id not found (insert case).
    pub fn upsert_provider(&mut self, config: ProviderConfig) -> bool {
        if let Some(existing) = self.providers.iter_mut().find(|p| p.id == config.id) {
            *existing = config;
            true
        } else {
            self.providers.push(config);
            false
        }
    }

    /// Remove a provider by id. Returns true if it existed.
    pub fn remove_provider(&mut self, id: &str) -> bool {
        let before = self.providers.len();
        self.providers.retain(|p| p.id != id);
        // Clear default if it was the removed provider.
        if self.default_provider.as_deref() == Some(id) {
            self.default_provider = self.providers.first().map(|p| p.id.clone());
        }
        self.providers.len() < before
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_provider: None,
            providers: vec![],
            execution_mode: ExecutionMode::default(),
            max_agent_steps: default_max_agent_steps(),
            onboarding_done: false,
            appimage_integration_declined: false,
            claude_notif_declined: false,
            submit_shortcut: SubmitShortcut::default(),
            doc_convert_engine: DocConvertEngine::default(),
            db_connections: vec![],
            default_tab: DefaultTab::default(),
            telegram_chat_id: None,
            vcs_connections: vec![],
            mail_accounts: vec![],
            enterprise_server_url: None,
            enterprise_device_id: None,
            enterprise_policy: None,
            mcp_enabled: true,
            mcp_servers: vec![],
            python_interpreter: None,
            python_index_url: None,
            claude_bridge: ClaudeBridgeConfig::default(),
            mcp_tool_server: McpToolServerConfig::default(),
            task_board: TaskBoardConfig::default(),
        }
    }
}

/// Configuration for a single AI provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Unique identifier, e.g. "claude-sonnet" or "local-llama".
    pub id: String,

    /// Human-readable name shown in the UI.
    pub display_name: String,

    /// The provider backend type.
    #[serde(rename = "type")]
    pub provider_type: ProviderType,

    /// Base URL override (required for Ollama and OpenAI-Compatible).
    #[serde(default)]
    pub base_url: Option<String>,

    /// Optional OAuth client id for providers that need device/web OAuth.
    #[serde(default)]
    pub oauth_client_id: Option<String>,

    /// Model identifier, e.g. "gpt-4o-mini" or "llama3.1:8b".
    pub model: String,

    /// Whether this provider supports JSON mode (OpenAI-compatible feature).
    /// Defaults to true for OpenAI/Anthropic/Compatible, false for Ollama.
    #[serde(default = "default_true")]
    pub supports_json_mode: bool,

    /// Auth method for Anthropic providers: None/"api_key" = API key, "oauth" = OAuth Bearer.
    #[serde(default)]
    pub auth_method: Option<String>,
}

fn default_true() -> bool { true }

/// The backend type of a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderType {
    Openai,
    Anthropic,
    Ollama,
    OpenaiCompatible,
    GithubCopilot,
    GoogleAi,
    Openrouter,
    Xai,
    Deepseek,
    Kimi,
    AnthropicCompatible,
    Codex,
    /// ChatGPT 網頁版（chatgpt.com/backend-api/conversation）。
    /// 與 `Codex` 不同：那是 Responses API + 原生 function calling，
    /// 這是網頁前端自己的後端 + prompt 模擬工具。兩者吃同一份訂閱額度。
    ChatgptWeb,
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::Openai => write!(f, "OpenAI"),
            ProviderType::Anthropic => write!(f, "Anthropic"),
            ProviderType::Ollama => write!(f, "Ollama"),
            ProviderType::OpenaiCompatible => write!(f, "OpenAI-Compatible"),
            ProviderType::GithubCopilot => write!(f, "GitHub Copilot"),
            ProviderType::GoogleAi => write!(f, "Google AI"),
            ProviderType::Openrouter => write!(f, "OpenRouter"),
            ProviderType::Xai => write!(f, "xAI (Grok)"),
            ProviderType::Deepseek => write!(f, "DeepSeek"),
            ProviderType::Kimi => write!(f, "Kimi (Moonshot)"),
            ProviderType::AnthropicCompatible => write!(f, "Anthropic-Compatible"),
            ProviderType::Codex => write!(f, "Codex"),
            ProviderType::ChatgptWeb => write!(f, "ChatGPT Web"),
        }
    }
}

/// How AI-generated commands are handled before execution (spec §4.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionMode {
    /// Every AI command requires explicit user confirmation (default).
    #[default]
    AlwaysConfirm,
    /// Safe commands execute automatically; others need confirmation.
    Graded,
    /// Safe + NeedsConfirm execute automatically; Dangerous still requires confirmation.
    FullAuto,
}

/// Which tab the app opens on startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DefaultTab {
    /// Open a terminal session (default).
    #[default]
    Terminal,
    /// Open the database browser.
    Database,
}

/// Keyboard shortcut choice for submitting commands in the terminal block interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SubmitShortcut {
    #[default]
    Enter,
    ShiftEnter,
    CtrlEnter,
}

/// Which engine document conversion prefers. `Auto` routes anydoc-covered
/// formats to anydoc (faster, better quality, no Python) and everything
/// else to MarkItDown; `MarkitdownOnly` disables anydoc entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocConvertEngine {
    #[default]
    Auto,
    MarkitdownOnly,
}

/// Supported database backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbType {
    Postgresql,
    Mysql,
    Sqlite,
    Mssql,
    Db2,
}

impl std::fmt::Display for DbType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbType::Postgresql => write!(f, "PostgreSQL"),
            DbType::Mysql => write!(f, "MySQL"),
            DbType::Sqlite => write!(f, "SQLite"),
            DbType::Mssql => write!(f, "MSSQL"),
            DbType::Db2 => write!(f, "DB2"),
        }
    }
}

/// A saved database connection (no password — that lives in Keychain).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbConnection {
    pub id: String,
    pub name: String,
    pub db_type: DbType,
    /// Host or IP. For SQLite, this is the file path.
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(default)]
    pub default_schema: Option<String>,
}

/// VCS backend type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VcsType {
    Git,
    Svn,
}

impl std::fmt::Display for VcsType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VcsType::Git => write!(f, "Git"),
            VcsType::Svn => write!(f, "SVN"),
        }
    }
}

/// Controls how write operations are gated in the VCS panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VcsWriteMode {
    /// All write operations are disabled.
    ReadOnly,
    /// Write operations require preview + user confirmation (default).
    #[default]
    Guarded,
    /// Write operations execute immediately without confirmation.
    FullAuto,
}

/// A saved VCS connection (token/password lives in Keychain under "aiterm:vcs:{id}").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VcsConnection {
    pub id: String,
    pub name: String,
    pub vcs_type: VcsType,
    /// Remote URL — GitHub repo URL for Git, SVN repo URL for SVN. Optional for local-only Git.
    #[serde(default)]
    pub url: Option<String>,
    /// Username for SVN authentication.
    #[serde(default)]
    pub username: Option<String>,
    /// Write operation gating mode.
    #[serde(default)]
    pub write_mode: VcsWriteMode,
}

/// A saved mail account (IMAP/SMTP). Password lives in Keychain under "mail:{id}".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailAccountConfig {
    pub id: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    /// How often to poll while the app is open. Defaults to 5 minutes.
    #[serde(default = "default_mail_poll_interval_secs")]
    pub poll_interval_secs: u32,
}

fn default_mail_poll_interval_secs() -> u32 { 300 }

/// Policy pushed from the Management Server. Fields present here override local AppConfig.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnterprisePolicy {
    /// Version number — AITERM applies when this increases.
    pub version: i64,
    /// Override default AI provider id.
    pub ai_provider_id: Option<String>,
    /// Override execution mode.
    pub execution_mode: Option<ExecutionMode>,
    /// Max agent steps override.
    pub max_agent_steps: Option<u32>,
    /// VCS push branch pattern (informational, enforced by server).
    pub vcs_push_pattern: Option<String>,
}

/// Transport protocol for an MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    /// Launch a subprocess and communicate over stdin/stdout (most common).
    #[default]
    Stdio,
    /// HTTP request/response transport.
    Http,
    /// Server-Sent Events transport.
    Sse,
}

/// Configuration for a single MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique identifier, e.g. "filesystem" or "brave-search".
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Whether this server is active (connected on startup).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Transport protocol.
    #[serde(default)]
    pub transport: McpTransport,
    /// Executable to launch (e.g. "npx", "python3", "uvx"). stdio only.
    #[serde(default)]
    pub command: Option<String>,
    /// Arguments for the subprocess. stdio only.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables injected into the subprocess. stdio only.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Base URL for http/sse transport. http/sse only.
    #[serde(default)]
    pub url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_config_default_is_empty() {
        let cfg = AppConfig::default();
        assert!(cfg.providers.is_empty());
        assert!(cfg.default_provider.is_none());
        assert!(!cfg.onboarding_done);
        assert_eq!(cfg.execution_mode, ExecutionMode::AlwaysConfirm);
        assert_eq!(cfg.submit_shortcut, SubmitShortcut::Enter);
    }

    #[test]
    fn provider_type_roundtrips_toml() {
        // TOML requires a struct at the top level, so we wrap the enum.
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W { ty: ProviderType }
        for (ty, expected_str) in [
            (ProviderType::Openai, "openai"),
            (ProviderType::Anthropic, "anthropic"),
            (ProviderType::Ollama, "ollama"),
            (ProviderType::OpenaiCompatible, "openai-compatible"),
            (ProviderType::GithubCopilot, "github-copilot"),
            (ProviderType::GoogleAi, "google-ai"),
            (ProviderType::Openrouter, "openrouter"),
            (ProviderType::Xai, "xai"),
            (ProviderType::Deepseek, "deepseek"),
            (ProviderType::Kimi, "kimi"),
            (ProviderType::AnthropicCompatible, "anthropic-compatible"),
            (ProviderType::Codex, "codex"),
            (ProviderType::ChatgptWeb, "chatgpt-web"),
        ] {
            let w = W { ty };
            let serialized = toml::to_string(&w).unwrap();
            assert!(serialized.contains(expected_str), "got: {serialized}");
            let deserialized: W = toml::from_str(&serialized).unwrap();
            assert_eq!(deserialized.ty, w.ty);
        }
    }

    #[test]
    fn chatgpt_web_provider_type_round_trips() {
        let json = serde_json::to_string(&ProviderType::ChatgptWeb).unwrap();
        assert_eq!(json, r#""chatgpt-web""#);
        let back: ProviderType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ProviderType::ChatgptWeb);
        assert_eq!(ProviderType::ChatgptWeb.to_string(), "ChatGPT Web");
    }

    #[test]
    fn execution_mode_roundtrips_toml() {
        // TOML requires a struct at the top level, so we wrap the enum.
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W { mode: ExecutionMode }
        for mode in [ExecutionMode::AlwaysConfirm, ExecutionMode::Graded, ExecutionMode::FullAuto] {
            let w = W { mode };
            let s = toml::to_string(&w).unwrap();
            let d: W = toml::from_str(&s).unwrap();
            assert_eq!(d.mode, w.mode);
        }
    }

    #[test]
    fn doc_convert_engine_defaults_to_auto() {
        let cfg: AppConfig = toml::from_str("").expect("empty config should parse");
        assert_eq!(cfg.doc_convert_engine, DocConvertEngine::Auto);
    }

    #[test]
    fn doc_convert_engine_roundtrips_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W { e: DocConvertEngine }
        for (engine, expected) in [
            (DocConvertEngine::Auto, "auto"),
            (DocConvertEngine::MarkitdownOnly, "markitdown_only"),
        ] {
            let w = W { e: engine };
            let s = toml::to_string(&w).unwrap();
            assert!(s.contains(expected), "got: {s}");
            let d: W = toml::from_str(&s).unwrap();
            assert_eq!(d.e, w.e);
        }
    }

    #[test]
    fn app_config_full_roundtrip() {
        let mut cfg = AppConfig {
            default_provider: Some("gpt".into()),
            providers: vec![ProviderConfig {
                id: "gpt".into(),
                display_name: "GPT-4o".into(),
                provider_type: ProviderType::Openai,
                base_url: None,
                oauth_client_id: None,
                model: "gpt-4o-mini".into(),
                supports_json_mode: true,
                auth_method: None,
            }],
            execution_mode: ExecutionMode::Graded,
            max_agent_steps: 5,
            submit_shortcut: SubmitShortcut::ShiftEnter,
            onboarding_done: true,
            db_connections: vec![],
            default_tab: DefaultTab::default(),
            telegram_chat_id: None,
            vcs_connections: vec![],
            ..AppConfig::default()
        };
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.default_provider, cfg.default_provider);
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].id, "gpt");
        assert_eq!(parsed.execution_mode, ExecutionMode::Graded);
        assert_eq!(parsed.submit_shortcut, SubmitShortcut::ShiftEnter);
        assert!(parsed.onboarding_done);

        // Test remove_provider clears default
        cfg.remove_provider("gpt");
        assert!(cfg.providers.is_empty());
        assert!(cfg.default_provider.is_none());
    }

    #[test]
    fn db_type_roundtrips_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W { t: DbType }
        for (ty, expected) in [
            (DbType::Postgresql, "postgresql"),
            (DbType::Mysql, "mysql"),
            (DbType::Sqlite, "sqlite"),
            (DbType::Mssql, "mssql"),
            (DbType::Db2, "db2"),
        ] {
            let w = W { t: ty };
            let s = toml::to_string(&w).unwrap();
            assert!(s.contains(expected), "got: {s}");
            let d: W = toml::from_str(&s).unwrap();
            assert_eq!(d.t, w.t);
        }
    }

    #[test]
    fn app_config_has_db_connections_default() {
        let cfg = AppConfig::default();
        assert!(cfg.db_connections.is_empty());
    }

    #[test]
    fn upsert_updates_existing() {
        let mut cfg = AppConfig::default();
        let p = ProviderConfig {
            id: "x".into(),
            display_name: "Old".into(),
            provider_type: ProviderType::Ollama,
            base_url: None,
            oauth_client_id: None,
            model: "llama3".into(),
            supports_json_mode: false,
            auth_method: None,
        };
        cfg.upsert_provider(p);
        assert_eq!(cfg.providers.len(), 1);

        let updated = ProviderConfig {
            id: "x".into(),
            display_name: "New".into(),
            provider_type: ProviderType::Ollama,
            base_url: None,
            oauth_client_id: None,
            model: "llama3.1".into(),
            supports_json_mode: false,
            auth_method: None,
        };
        cfg.upsert_provider(updated);
        assert_eq!(cfg.providers.len(), 1);
        assert_eq!(cfg.providers[0].display_name, "New");
    }

    #[test]
    fn vcs_type_roundtrips_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W { t: VcsType }
        for (ty, expected) in [(VcsType::Git, "git"), (VcsType::Svn, "svn")] {
            let w = W { t: ty };
            let s = toml::to_string(&w).unwrap();
            assert!(s.contains(expected), "got: {s}");
            let d: W = toml::from_str(&s).unwrap();
            assert_eq!(d.t, w.t);
        }
    }

    #[test]
    fn vcs_write_mode_roundtrips_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W { m: VcsWriteMode }
        for (mode, expected) in [
            (VcsWriteMode::ReadOnly, "read_only"),
            (VcsWriteMode::Guarded, "guarded"),
            (VcsWriteMode::FullAuto, "full_auto"),
        ] {
            let w = W { m: mode };
            let s = toml::to_string(&w).unwrap();
            assert!(s.contains(expected), "got: {s}");
            let d: W = toml::from_str(&s).unwrap();
            assert_eq!(d.m, w.m);
        }
    }

    #[test]
    fn vcs_connection_roundtrips_toml() {
        #[derive(Serialize, Deserialize, Debug)]
        struct W { c: VcsConnection }
        let w = W {
            c: VcsConnection {
                id: "my-repo".into(),
                name: "My Repo".into(),
                vcs_type: VcsType::Git,
                url: Some("https://github.com/org/repo".into()),
                username: None,
                write_mode: VcsWriteMode::Guarded,
            },
        };
        let s = toml::to_string(&w).unwrap();
        let d: W = toml::from_str(&s).unwrap();
        assert_eq!(d.c.id, "my-repo");
        assert_eq!(d.c.vcs_type, VcsType::Git);
        assert_eq!(d.c.write_mode, VcsWriteMode::Guarded);
        assert_eq!(d.c.url.as_deref(), Some("https://github.com/org/repo"));
    }

    #[test]
    fn app_config_has_vcs_connections_default() {
        let cfg = AppConfig::default();
        assert!(cfg.vcs_connections.is_empty());
    }

    #[test]
    fn mcp_server_config_roundtrips_toml() {
        #[derive(Serialize, Deserialize, Debug)]
        struct W { s: McpServerConfig }

        let w = W {
            s: McpServerConfig {
                id: "fs".into(),
                name: "Filesystem".into(),
                enabled: true,
                transport: McpTransport::Stdio,
                command: Some("npx".into()),
                args: vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into()],
                env: {
                    let mut m = HashMap::new();
                    m.insert("FOO".into(), "bar".into());
                    m
                },
                url: None,
            },
        };
        let s = toml::to_string(&w).unwrap();
        let d: W = toml::from_str(&s).unwrap();
        assert_eq!(d.s.id, "fs");
        assert_eq!(d.s.transport, McpTransport::Stdio);
        assert_eq!(d.s.command.as_deref(), Some("npx"));
        assert_eq!(d.s.args.len(), 2);
        assert_eq!(d.s.env["FOO"], "bar");
    }

    #[test]
    fn mcp_transport_all_variants_roundtrip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W { t: McpTransport }
        for (t, expected) in [
            (McpTransport::Stdio, "stdio"),
            (McpTransport::Http, "http"),
            (McpTransport::Sse, "sse"),
        ] {
            let w = W { t };
            let s = toml::to_string(&w).unwrap();
            assert!(s.contains(expected), "got: {s}");
            let d: W = toml::from_str(&s).unwrap();
            assert_eq!(d.t, w.t);
        }
    }

    #[test]
    fn app_config_mcp_defaults_to_enabled_empty() {
        let cfg = AppConfig::default();
        assert!(cfg.mcp_enabled);
        assert!(cfg.mcp_servers.is_empty());
    }

    #[test]
    fn python_interpreter_defaults_to_none_for_existing_configs() {
        let cfg: AppConfig = toml::from_str("").expect("empty config should parse");
        assert_eq!(cfg.python_interpreter, None);
    }

    #[test]
    fn python_interpreter_round_trips() {
        let cfg: AppConfig =
            toml::from_str("python_interpreter = \"/usr/local/bin/python3.12\"").unwrap();
        assert_eq!(cfg.python_interpreter.as_deref(), Some("/usr/local/bin/python3.12"));
    }

    #[test]
    fn python_index_url_defaults_to_none_for_existing_configs() {
        let cfg: AppConfig = toml::from_str("").expect("empty config should parse");
        assert_eq!(cfg.python_index_url, None);
    }

    #[test]
    fn python_index_url_round_trips() {
        let cfg: AppConfig =
            toml::from_str("python_index_url = \"https://pypi.mycompany.com/simple\"").unwrap();
        assert_eq!(cfg.python_index_url.as_deref(), Some("https://pypi.mycompany.com/simple"));
    }

    #[test]
    fn task_board_config_has_sane_defaults() {
        let c = TaskBoardConfig::default();
        assert_eq!(c.max_concurrent, 2);
        assert_eq!(c.claude_command, "claude");
    }

    #[test]
    fn app_config_default_includes_task_board() {
        let c = AppConfig::default();
        assert_eq!(c.task_board.max_concurrent, 2);
    }

    #[test]
    fn task_board_config_deserialises_from_empty_table() {
        // Old config with no [task_board] section must still parse.
        let c: AppConfig = toml::from_str("").unwrap();
        assert_eq!(c.task_board.max_concurrent, 2);
    }
}

#[cfg(test)]
mod bridge_config_tests {
    use super::*;

    #[test]
    fn missing_section_gets_defaults() {
        // 舊的 config.toml 沒有 [claude_bridge] 區塊，必須照常載入。
        let cfg: AppConfig = toml::from_str("").expect("空 config 應可載入");
        assert!(!cfg.claude_bridge.enabled);
        assert_eq!(cfg.claude_bridge.port, 8317);
        assert!(cfg.claude_bridge.opus.is_none());
    }

    #[test]
    fn tier_mapping_round_trips() {
        let toml_src = r#"
[claude_bridge]
enabled = true
port = 9000

[claude_bridge.sonnet]
provider_id = "local-qwen"
model = "Qwen3.6-35B-A3B-4bit"
"#;
        let cfg: AppConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.claude_bridge.port, 9000);
        let sonnet = cfg.claude_bridge.sonnet.as_ref().unwrap();
        assert_eq!(sonnet.provider_id, "local-qwen");
        assert_eq!(sonnet.model, "Qwen3.6-35B-A3B-4bit");
    }
}
