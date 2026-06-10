//! TOML-serializable configuration types.
//!
//! The TOML file stores provider metadata (id, type, model, base_url).
//! API keys are NEVER stored here — they live in the OS keychain under
//! "aiterm:{provider_id}".

use serde::{Deserialize, Serialize};

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

    /// Which shortcut submits the command (Enter vs Shift+Enter, etc).
    #[serde(default)]
    pub submit_shortcut: SubmitShortcut,

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

    /// Enterprise Management Server URL. When set, enterprise mode is active.
    #[serde(default)]
    pub enterprise_server_url: Option<String>,

    /// Unique device identifier assigned by the Management Server on registration.
    #[serde(default)]
    pub enterprise_device_id: Option<String>,

    /// Policy pushed by the Management Server. Overrides local settings when present.
    #[serde(default)]
    pub enterprise_policy: Option<EnterprisePolicy>,
}

fn default_max_agent_steps() -> u32 { 5 }

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
        ] {
            let w = W { ty };
            let serialized = toml::to_string(&w).unwrap();
            assert!(serialized.contains(expected_str), "got: {serialized}");
            let deserialized: W = toml::from_str(&serialized).unwrap();
            assert_eq!(deserialized.ty, w.ty);
        }
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
}
