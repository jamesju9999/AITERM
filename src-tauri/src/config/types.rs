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
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::Openai => write!(f, "OpenAI"),
            ProviderType::Anthropic => write!(f, "Anthropic"),
            ProviderType::Ollama => write!(f, "Ollama"),
            ProviderType::OpenaiCompatible => write!(f, "OpenAI-Compatible"),
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
                model: "gpt-4o-mini".into(),
                supports_json_mode: true,
            }],
            execution_mode: ExecutionMode::Graded,
            max_agent_steps: 5,
            submit_shortcut: SubmitShortcut::ShiftEnter,
            onboarding_done: true,
            db_connections: vec![],
            default_tab: DefaultTab::default(),
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
            model: "llama3.1".into(),
            supports_json_mode: false,
        };
        cfg.upsert_provider(updated);
        assert_eq!(cfg.providers.len(), 1);
        assert_eq!(cfg.providers[0].display_name, "New");
    }
}
