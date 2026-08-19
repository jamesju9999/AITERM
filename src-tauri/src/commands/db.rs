//! Tauri commands for database connection management and query execution.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::time::timeout;

use crate::config::{
    types::{DbConnection, DbType},
    ConfigStore,
};
use crate::db::{
    adapter::{ColumnInfo, QueryResult, TableInfo},
    db2::Db2Adapter,
    db2_sidecar::Db2SidecarState,
    manager::DbManager,
    mssql::MssqlAdapter,
    mysql::MySqlAdapter,
    postgres::PostgresAdapter,
    sqlite::SqliteAdapter,
};
use crate::secret::SecretStore;

/// Connection config sent from the frontend for add/update/test operations.
#[derive(Debug, Deserialize)]
pub struct DbConnectionInput {
    pub id: Option<String>,
    pub name: String,
    pub db_type: DbType,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub default_schema: Option<String>,
}

/// Connection info returned to the frontend (no password).
#[derive(Debug, Serialize)]
pub struct DbConnectionInfo {
    pub id: String,
    pub name: String,
    pub db_type: DbType,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub default_schema: Option<String>,
    pub is_connected: bool,
}

pub(crate) fn secret_key(id: &str) -> String {
    format!("db:{id}")
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Percent-encode a string for use in a URL userinfo or path component.
/// Encodes all bytes except unreserved characters: ALPHA / DIGIT / "-" / "." / "_" / "~"
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

fn is_db2_sidecar_transport_error_message(msg: &str) -> bool {
    msg.contains("os error 232")
        || msg.contains("Broken pipe")
        || msg.contains("db2_sidecar_died")
        || msg.contains("EOF on stdout")
}

fn is_db2_sidecar_transport_error(err: &anyhow::Error) -> bool {
    is_db2_sidecar_transport_error_message(&err.to_string())
}

async fn build_adapter(
    conn: &DbConnection,
    password: &str,
    sidecar: &Db2SidecarState,
) -> anyhow::Result<Box<dyn crate::db::adapter::DbAdapter>> {
    match conn.db_type {
        DbType::Postgresql => {
            let url = format!(
                "postgresql://{}:{}@{}:{}/{}",
                pct_encode(&conn.username),
                pct_encode(password),
                conn.host,
                conn.port,
                pct_encode(&conn.database)
            );
            Ok(Box::new(PostgresAdapter::connect(&url).await?))
        }
        DbType::Mysql => {
            let url = format!(
                "mysql://{}:{}@{}:{}/{}",
                pct_encode(&conn.username),
                pct_encode(password),
                conn.host,
                conn.port,
                pct_encode(&conn.database)
            );
            Ok(Box::new(MySqlAdapter::connect(&url).await?))
        }
        DbType::Sqlite => Ok(Box::new(SqliteAdapter::connect(&conn.host).await?)),
        DbType::Mssql => Ok(Box::new(
            MssqlAdapter::connect(
                &conn.host,
                conn.port,
                &conn.database,
                &conn.username,
                password,
            )
            .await?,
        )),
        DbType::Db2 => {
            let cs = format!(
                "jdbc:db2://{}:{}/{}",
                conn.host, conn.port, conn.database
            );
            let username = conn.username.clone();
            let password = password.to_string();
            let client = sidecar.get_client().await?;

            match Db2Adapter::connect(client, cs.clone(), username.clone(), password.clone()).await {
                Ok(adapter) => Ok(Box::new(adapter)),
                Err(err) if is_db2_sidecar_transport_error(&err) => {
                    // Recover from stale/crashed sidecar instance and retry once.
                    sidecar.reset().await;
                    let client = sidecar.get_client().await?;
                    Ok(Box::new(
                        Db2Adapter::connect(client, cs, username, password).await?,
                    ))
                }
                Err(err) => Err(err),
            }
        }
    }
}

async fn run_connection_test(
    conn: &DbConnection,
    password: &str,
    sidecar: &Db2SidecarState,
) -> Result<(), String> {
    let adapter = build_adapter(conn, password, sidecar)
        .await
        .map_err(|e| e.to_string())?;
    adapter.test().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_list_connections(
    config: State<'_, Arc<ConfigStore>>,
    manager: State<'_, DbManager>,
) -> Result<Vec<DbConnectionInfo>, String> {
    let conns = config.get().db_connections;
    let mut result = Vec::with_capacity(conns.len());
    for c in conns {
        let is_connected = manager.is_connected(&c.id).await;
        result.push(DbConnectionInfo {
            id: c.id,
            name: c.name,
            db_type: c.db_type,
            host: c.host,
            port: c.port,
            database: c.database,
            username: c.username,
            default_schema: c.default_schema,
            is_connected,
        });
    }
    Ok(result)
}

#[tauri::command]
pub async fn db_add_connection(
    input: DbConnectionInput,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = DbConnection {
        id: id.clone(),
        name: input.name,
        db_type: input.db_type,
        host: input.host,
        port: input.port,
        database: input.database,
        username: input.username,
        default_schema: normalize_optional_string(input.default_schema),
    };
    config.add_db_connection(conn).map_err(|e| e.to_string())?;
    secrets
        .set(&secret_key(&id), &input.password)
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn db_update_connection(
    input: DbConnectionInput,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let id = input.id.clone().ok_or("missing id")?;
    let conn = DbConnection {
        id: id.clone(),
        name: input.name,
        db_type: input.db_type,
        host: input.host,
        port: input.port,
        database: input.database,
        username: input.username,
        default_schema: normalize_optional_string(input.default_schema),
    };
    config
        .update_db_connection(conn)
        .map_err(|e| e.to_string())?;
    if !input.password.is_empty() {
        secrets
            .set(&secret_key(&id), &input.password)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn db_remove_connection(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    manager: State<'_, DbManager>,
) -> Result<(), String> {
    manager.remove(&id).await;
    config
        .remove_db_connection(&id)
        .map_err(|e| e.to_string())?;
    // Best-effort: config is already removed; ignore Keychain errors so
    // the command doesn't fail if the secret was never stored.
    let _ = secrets.delete(&secret_key(&id));
    Ok(())
}

#[tauri::command]
pub async fn db_test_connection(
    input: DbConnectionInput,
    secrets: State<'_, Arc<SecretStore>>,
    sidecar: State<'_, Db2SidecarState>,
) -> Result<(), String> {
    let password = if !input.password.is_empty() {
        input.password.clone()
    } else if let Some(id) = &input.id {
        secrets
            .get(&secret_key(id))
            .ok()
            .flatten()
            .unwrap_or_default()
    } else {
        String::new()
    };

    let temp_conn = DbConnection {
        id: input.id.clone().unwrap_or_default(),
        name: input.name.clone(),
        db_type: input.db_type,
        host: input.host.clone(),
        port: input.port,
        database: input.database.clone(),
        username: input.username.clone(),
        default_schema: normalize_optional_string(input.default_schema.clone()),
    };

    let first_attempt = timeout(
        Duration::from_secs(20),
        run_connection_test(&temp_conn, &password, &sidecar),
    )
    .await
    .map_err(|_| "connection test timed out after 20s".to_string())?;

    match first_attempt {
        Ok(()) => Ok(()),
        Err(err)
            if temp_conn.db_type == DbType::Db2
                && is_db2_sidecar_transport_error_message(&err) =>
        {
            sidecar.reset().await;
            timeout(
                Duration::from_secs(20),
                run_connection_test(&temp_conn, &password, &sidecar),
            )
            .await
            .map_err(|_| "connection test timed out after 20s".to_string())?
        }
        Err(err) => Err(err),
    }
}

/// Ensure a live connection exists for `id`, connecting on demand if it doesn't
/// (reads the stored password, builds the right adapter for the DB type, and
/// retries once if a dead DB2 sidecar caused the first attempt to fail).
/// Shared by the `db_connect` Tauri command (used when the user opens the DB
/// tab) and the MCP tool server's `execute_query`/`list_tables`/etc. (used
/// when an external MCP client calls a tool without the DB tab ever being
/// open) — both need identical connect-on-demand behavior.
pub(crate) async fn ensure_connected(
    id: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<(), String> {
    if manager.is_connected(id).await {
        return Ok(());
    }
    let conn = config
        .get()
        .db_connections
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("connection not found: {id}"))?;
    let password = secrets
        .get(&secret_key(id))
        .ok()
        .flatten()
        .unwrap_or_default();
    let adapter = match build_adapter(&conn, &password, sidecar).await {
        Ok(adapter) => adapter,
        Err(err) if conn.db_type == DbType::Db2 && is_db2_sidecar_transport_error(&err) => {
            sidecar.reset().await;
            build_adapter(&conn, &password, sidecar)
                .await
                .map_err(|e| e.to_string())?
        }
        Err(err) => return Err(err.to_string()),
    };
    manager.insert(id.to_string(), adapter).await;
    Ok(())
}

#[tauri::command]
pub async fn db_connect(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    manager: State<'_, DbManager>,
    sidecar: State<'_, Db2SidecarState>,
) -> Result<(), String> {
    ensure_connected(&id, &config, &secrets, &manager, &sidecar).await
}

#[tauri::command]
pub async fn db_disconnect(id: String, manager: State<'_, DbManager>) -> Result<(), String> {
    manager.remove(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn db_list_schemas(
    connection_id: String,
    manager: State<'_, DbManager>,
) -> Result<Vec<String>, String> {
    manager
        .list_schemas(&connection_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_list_tables(
    connection_id: String,
    schema: String,
    manager: State<'_, DbManager>,
) -> Result<Vec<TableInfo>, String> {
    manager
        .list_tables(&connection_id, &schema)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_get_table_schema(
    connection_id: String,
    schema: String,
    table: String,
    manager: State<'_, DbManager>,
) -> Result<Vec<ColumnInfo>, String> {
    manager
        .get_table_schema(&connection_id, &schema, &table)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_execute_query(
    connection_id: String,
    sql: String,
    schema: Option<String>,
    config: State<'_, Arc<ConfigStore>>,
    manager: State<'_, DbManager>,
) -> Result<QueryResult, String> {
    if let Some(conn) = config
        .get()
        .db_connections
        .into_iter()
        .find(|c| c.id == connection_id)
    {
        if conn.db_type == DbType::Db2 {
            if let Some(schema_name) = schema.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
                let escaped_schema = schema_name.replace('"', "\"\"");
                let set_schema_sql = format!("SET CURRENT SCHEMA \"{escaped_schema}\"");
                manager
                    .execute(&connection_id, &set_schema_sql)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    manager
        .execute(&connection_id, &sql)
        .await
        .map_err(|e| e.to_string())
}

/// Preview table data with correct paging SQL for each database dialect.
#[tauri::command]
pub async fn db_preview_table(
    connection_id: String,
    schema: String,
    table: String,
    page: u32,
    page_size: u32,
    config: State<'_, Arc<ConfigStore>>,
    manager: State<'_, DbManager>,
) -> Result<QueryResult, String> {
    let conn = config
        .get()
        .db_connections
        .into_iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("connection not found: {connection_id}"))?;

    let offset = page * page_size;

    let sql = match conn.db_type {
        DbType::Mssql => {
            // MSSQL uses bracket quoting and OFFSET/FETCH (SQL Server 2012+)
            format!(
                "SELECT * FROM [{schema}].[{table}] ORDER BY (SELECT NULL) OFFSET {offset} ROWS FETCH NEXT {page_size} ROWS ONLY"
            )
        }
        DbType::Db2 => {
            // DB2 supports OFFSET/FETCH for paging.
            format!(
                "SELECT * FROM \"{schema}\".\"{table}\" OFFSET {offset} ROWS FETCH NEXT {page_size} ROWS ONLY"
            )
        }
        // PostgreSQL, MySQL, SQLite all support LIMIT/OFFSET
        _ => {
            format!("SELECT * FROM \"{schema}\".\"{table}\" LIMIT {page_size} OFFSET {offset}")
        }
    };

    manager
        .execute(&connection_id, &sql)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod ensure_connected_tests {
    use super::*;
    use crate::config::types::{DbConnection, DbType};
    use crate::secret::SecretStore;

    fn sqlite_connection(id: &str, path: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: "test".to_string(),
            db_type: DbType::Sqlite,
            host: path.to_string(),
            port: 0,
            database: String::new(),
            username: String::new(),
            default_schema: None,
        }
    }

    #[tokio::test]
    async fn connects_when_not_already_connected() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        config
            .add_db_connection(sqlite_connection("sq1", ":memory:"))
            .unwrap();
        let secrets = SecretStore::new();
        let manager = DbManager::new();
        let sidecar = Db2SidecarState::new(dir.path().to_path_buf());

        assert!(!manager.is_connected("sq1").await);
        ensure_connected("sq1", &config, &secrets, &manager, &sidecar)
            .await
            .unwrap();
        assert!(manager.is_connected("sq1").await);
    }

    #[tokio::test]
    async fn is_a_no_op_when_already_connected() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        config
            .add_db_connection(sqlite_connection("sq1", ":memory:"))
            .unwrap();
        let secrets = SecretStore::new();
        let manager = DbManager::new();
        let sidecar = Db2SidecarState::new(dir.path().to_path_buf());

        ensure_connected("sq1", &config, &secrets, &manager, &sidecar)
            .await
            .unwrap();
        // Second call must not error even though a live connection already exists.
        ensure_connected("sq1", &config, &secrets, &manager, &sidecar)
            .await
            .unwrap();
        assert!(manager.is_connected("sq1").await);
    }

    #[tokio::test]
    async fn errors_when_connection_id_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let secrets = SecretStore::new();
        let manager = DbManager::new();
        let sidecar = Db2SidecarState::new(dir.path().to_path_buf());

        let err = ensure_connected("nonexistent", &config, &secrets, &manager, &sidecar)
            .await
            .unwrap_err();
        assert!(err.contains("connection not found"), "{err}");
    }
}
