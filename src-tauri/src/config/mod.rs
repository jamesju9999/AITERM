//! Persistent configuration backed by a TOML file.
//!
//! File location:
//!   Windows: `%APPDATA%\AITerm\config.toml`
//!   macOS/Linux: `~/.config/aiterm/config.toml`
//!
//! Writes are atomic: data is written to a `.tmp` sibling file and then
//! renamed, so a crash mid-write never produces a corrupt config.

pub mod types;
pub use types::*;

use std::path::PathBuf;
use parking_lot::RwLock;
use anyhow::{Context, Result};

pub struct ConfigStore {
    path: PathBuf,
    state: RwLock<AppConfig>,
}

impl ConfigStore {
    /// Create a new store. Loads existing config if the file exists;
    /// otherwise starts with defaults. Never panics — a corrupt or missing
    /// file just yields defaults (and logs a warning).
    pub fn new() -> Self {
        let path = config_path();
        let state = match Self::load_from(&path) {
            Ok(cfg) => cfg,
            Err(e) => {
                log::warn!("Failed to load config (using defaults): {e}");
                AppConfig::default()
            }
        };
        Self { path, state: RwLock::new(state) }
    }

    /// Return a snapshot of the current config.
    pub fn get(&self) -> AppConfig {
        self.state.read().clone()
    }

    /// Apply a mutation function, then persist to disk atomically.
    pub fn update<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut AppConfig),
    {
        let mut guard = self.state.write();
        f(&mut guard);
        self.save_to(&self.path, &guard)
    }

    /// Add a new DB connection config.
    pub fn add_db_connection(&self, conn: DbConnection) -> anyhow::Result<()> {
        self.update(|cfg| {
            cfg.db_connections.push(conn);
        })
    }

    /// Update an existing DB connection by id. Silently no-ops if the id is not found.
    pub fn update_db_connection(&self, conn: DbConnection) -> anyhow::Result<()> {
        self.update(|cfg| {
            if let Some(existing) = cfg.db_connections.iter_mut().find(|c| c.id == conn.id) {
                *existing = conn;
            }
        })
    }

    /// Remove a DB connection by id.
    pub fn remove_db_connection(&self, id: &str) -> anyhow::Result<()> {
        self.update(|cfg| {
            cfg.db_connections.retain(|c| c.id != id);
        })
    }

    /// Add a new VCS connection config.
    pub fn add_vcs_connection(&self, conn: VcsConnection) -> anyhow::Result<()> {
        self.update(|cfg| {
            cfg.vcs_connections.push(conn);
        })
    }

    /// Update an existing VCS connection by id. Silently no-ops if not found.
    pub fn update_vcs_connection(&self, conn: VcsConnection) -> anyhow::Result<()> {
        self.update(|cfg| {
            if let Some(existing) = cfg.vcs_connections.iter_mut().find(|c| c.id == conn.id) {
                *existing = conn;
            }
        })
    }

    /// Remove a VCS connection by id.
    pub fn remove_vcs_connection(&self, id: &str) -> anyhow::Result<()> {
        self.update(|cfg| {
            cfg.vcs_connections.retain(|c| c.id != id);
        })
    }

    /// Shortcut: get a single provider by id.
    pub fn get_provider(&self, id: &str) -> Option<ProviderConfig> {
        self.state.read().find_provider(id).cloned()
    }

    // ── private helpers ──────────────────────────────────────────────────

    fn load_from(path: &PathBuf) -> Result<AppConfig> {
        if !path.exists() {
            return Ok(AppConfig::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: AppConfig = toml::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    fn save_to(&self, path: &PathBuf, cfg: &AppConfig) -> Result<()> {
        // Ensure the parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
        }

        let content = toml::to_string_pretty(cfg)
            .context("serializing config to TOML")?;

        // Write to a temporary file then rename for atomicity.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, content)
            .with_context(|| format!("writing temp file {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;

        Ok(())
    }
}

impl Default for ConfigStore {
    fn default() -> Self { Self::new() }
}

impl ConfigStore {
    /// Create a store from an existing `AppConfig` (no file I/O). Used in tests
    /// and when migrating from env-var based config.
    pub fn from_config(cfg: AppConfig) -> Self {
        let path = config_path();
        Self { path, state: RwLock::new(cfg) }
    }
}

/// Platform-specific config file path.
fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("AITerm").join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Helper: create a ConfigStore pointing at a temp dir.
    fn temp_store() -> (ConfigStore, PathBuf) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let store = ConfigStore {
            path: path.clone(),
            state: RwLock::new(AppConfig::default()),
        };
        // Keep dir alive by leaking — acceptable in tests.
        std::mem::forget(dir);
        (store, path)
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let cfg = ConfigStore::load_from(&path).unwrap();
        assert!(cfg.providers.is_empty());
        assert!(!cfg.onboarding_done);
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let (store, path) = temp_store();
        store.update(|cfg| {
            cfg.onboarding_done = true;
            cfg.providers.push(ProviderConfig {
                id: "gpt".into(),
                display_name: "GPT".into(),
                provider_type: ProviderType::Openai,
                base_url: None,
                oauth_client_id: None,
                model: "gpt-4o-mini".into(),
                supports_json_mode: true,
                auth_method: None,
            });
            cfg.default_provider = Some("gpt".into());
        }).unwrap();

        assert!(path.exists(), "config file should have been written");

        let reloaded = ConfigStore::load_from(&path).unwrap();
        assert!(reloaded.onboarding_done);
        assert_eq!(reloaded.providers.len(), 1);
        assert_eq!(reloaded.default_provider, Some("gpt".into()));
    }

    #[test]
    fn atomic_write_no_tmp_left_behind() {
        let (store, path) = temp_store();
        store.update(|cfg| { cfg.onboarding_done = true; }).unwrap();
        let tmp = path.with_extension("toml.tmp");
        assert!(!tmp.exists(), ".tmp file should not remain after successful write");
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid toml :::").unwrap();
        let store = ConfigStore {
            path: path.clone(),
            state: RwLock::new(ConfigStore::load_from(&path).unwrap_or_default()),
        };
        // The store should be alive with defaults even if the file is corrupt.
        assert!(store.get().providers.is_empty());
    }

    #[test]
    fn get_provider_by_id() {
        let (store, _) = temp_store();
        store.update(|cfg| {
            cfg.providers.push(ProviderConfig {
                id: "ollama-local".into(),
                display_name: "Ollama".into(),
                provider_type: ProviderType::Ollama,
                base_url: Some("http://localhost:11434".into()),
                oauth_client_id: None,
                model: "llama3".into(),
                supports_json_mode: false,
                auth_method: None,
            });
        }).unwrap();

        assert!(store.get_provider("ollama-local").is_some());
        assert!(store.get_provider("missing").is_none());
    }

    #[test]
    fn db_connection_crud() {
        use crate::config::types::{DbConnection, DbType};
        let (store, _) = temp_store();

        let conn = DbConnection {
            id: "conn-1".into(),
            name: "Local PG".into(),
            db_type: DbType::Postgresql,
            host: "localhost".into(),
            port: 5432,
            database: "mydb".into(),
            username: "postgres".into(),
            default_schema: Some("public".into()),
        };

        store.add_db_connection(conn.clone()).unwrap();
        assert_eq!(store.get().db_connections.len(), 1);

        let mut updated = conn.clone();
        updated.name = "Updated PG".into();
        store.update_db_connection(updated).unwrap();
        assert_eq!(store.get().db_connections[0].name, "Updated PG");

        store.remove_db_connection("conn-1").unwrap();
        assert!(store.get().db_connections.is_empty());
    }

    #[test]
    fn concurrent_updates_are_safe() {
        let store = Arc::new({
            let dir = tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::mem::forget(dir);
            ConfigStore { path, state: RwLock::new(AppConfig::default()) }
        });

        let handles: Vec<_> = (0..4).map(|i| {
            let s = store.clone();
            std::thread::spawn(move || {
                s.update(|cfg| {
                    cfg.providers.push(ProviderConfig {
                        id: format!("p{i}"),
                        display_name: format!("Provider {i}"),
                        provider_type: ProviderType::Openai,
                        base_url: None,
                        oauth_client_id: None,
                        model: "gpt-4o-mini".into(),
                        supports_json_mode: true,
                        auth_method: None,
                    });
                }).unwrap();
            })
        }).collect();

        for h in handles { h.join().unwrap(); }
        assert_eq!(store.get().providers.len(), 4);
    }
}
