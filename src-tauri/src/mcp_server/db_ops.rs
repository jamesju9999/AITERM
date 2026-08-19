//! Business logic for the DB-connection MCP tools (`list_connections`,
//! `list_schemas`, `list_tables`, `get_table_schema`, `execute_query`).
//! Plain async functions returning `Result<String, String>` (Ok = tool
//! content, Err = tool-level error message) — kept separate from
//! `tools.rs`'s `#[tool]`-annotated methods so this logic can be unit tested
//! without going through the MCP protocol layer.

use crate::commands::db::ensure_connected;
use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::knowledge_base::tools::safe_truncate;
use crate::secret::SecretStore;

const MAX_QUERY_RESULT_BYTES: usize = 100 * 1024;

pub(crate) async fn list_connections(config: &ConfigStore) -> Result<String, String> {
    let conns = config.get().db_connections;
    if conns.is_empty() {
        return Ok("No DB connections configured in AITerm.".to_string());
    }
    let list: Vec<serde_json::Value> = conns.iter().map(|c| serde_json::json!({
        "id": c.id, "name": c.name, "db_type": c.db_type, "database": c.database,
    })).collect();
    serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
}

pub(crate) async fn list_schemas(
    connection_id: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<String, String> {
    ensure_connected(connection_id, config, secrets, manager, sidecar).await?;
    let schemas = manager.list_schemas(connection_id).await.map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&schemas).map_err(|e| e.to_string())
}

pub(crate) async fn list_tables(
    connection_id: &str,
    schema: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<String, String> {
    ensure_connected(connection_id, config, secrets, manager, sidecar).await?;
    let tables = manager.list_tables(connection_id, schema).await.map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&tables).map_err(|e| e.to_string())
}

pub(crate) async fn get_table_schema(
    connection_id: &str,
    schema: &str,
    table: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<String, String> {
    ensure_connected(connection_id, config, secrets, manager, sidecar).await?;
    let cols = manager.get_table_schema(connection_id, schema, table).await.map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&cols).map_err(|e| e.to_string())
}

/// Executes arbitrary SQL (not restricted to read-only — this is a
/// deliberate product decision, see the design doc). Auto-connects if the
/// connection isn't live yet. A DB-level error (bad SQL, permission denied)
/// comes back as `QueryResult::error` per `DbAdapter::execute`'s contract,
/// not as a Rust `Err` — that's surfaced here as `Err` so the caller (the
/// `#[tool]` method) reports it as a tool-level error to the MCP client.
pub(crate) async fn execute_query(
    connection_id: &str,
    sql: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<String, String> {
    ensure_connected(connection_id, config, secrets, manager, sidecar).await?;
    let result = manager.execute(connection_id, sql).await.map_err(|e| e.to_string())?;
    if let Some(db_err) = &result.error {
        return Err(db_err.clone());
    }
    let full = serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?;
    if full.len() > MAX_QUERY_RESULT_BYTES {
        Ok(format!(
            "{}\n\n[TRUNCATED: result exceeds {} bytes; {} rows total]",
            safe_truncate(&full, MAX_QUERY_RESULT_BYTES),
            MAX_QUERY_RESULT_BYTES,
            result.rows.len(),
        ))
    } else {
        Ok(full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{DbConnection, DbType};

    fn sqlite_connection(id: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: "test".to_string(),
            db_type: DbType::Sqlite,
            host: ":memory:".to_string(),
            port: 0,
            database: String::new(),
            username: String::new(),
            default_schema: None,
        }
    }

    fn setup(id: &str) -> (tempfile::TempDir, ConfigStore, SecretStore, DbManager, Db2SidecarState) {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        config.add_db_connection(sqlite_connection(id)).unwrap();
        let secrets = SecretStore::new();
        let manager = DbManager::new();
        let sidecar = Db2SidecarState::new(dir.path().to_path_buf());
        (dir, config, secrets, manager, sidecar)
    }

    #[tokio::test]
    async fn list_connections_reports_configured_connections() {
        let (_dir, config, _secrets, _manager, _sidecar) = setup("sq1");
        let out = list_connections(&config).await.unwrap();
        assert!(out.contains("sq1"));
        assert!(out.contains("sqlite"));
    }

    #[tokio::test]
    async fn execute_query_auto_connects_when_not_yet_connected() {
        let (_dir, config, secrets, manager, sidecar) = setup("sq1");
        assert!(!manager.is_connected("sq1").await);
        let out = execute_query("sq1", "SELECT 42 as n", &config, &secrets, &manager, &sidecar).await.unwrap();
        assert!(manager.is_connected("sq1").await);
        assert!(out.contains("42"));
    }

    #[tokio::test]
    async fn execute_query_reports_db_level_errors() {
        let (_dir, config, secrets, manager, sidecar) = setup("sq1");
        let err = execute_query("sq1", "SELECT * FROM nonexistent_table", &config, &secrets, &manager, &sidecar)
            .await
            .unwrap_err();
        assert!(err.to_lowercase().contains("no such table"), "{err}");
    }

    #[tokio::test]
    async fn execute_query_truncates_large_results() {
        let (_dir, config, secrets, manager, sidecar) = setup("sq1");
        // A single very long string value blows past MAX_QUERY_RESULT_BYTES on its own.
        let sql = format!("SELECT '{}' as huge", "x".repeat(MAX_QUERY_RESULT_BYTES + 1000));
        let out = execute_query("sq1", &sql, &config, &secrets, &manager, &sidecar).await.unwrap();
        assert!(out.contains("TRUNCATED"), "{out}");
        assert!(out.len() < MAX_QUERY_RESULT_BYTES + 500, "output should be capped near the byte limit, got {} bytes", out.len());
    }
}
