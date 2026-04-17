//! Tauri commands for database connection management and query execution.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::config::{
    types::{DbConnection, DbType},
    ConfigStore,
};
use crate::db::{
    adapter::{ColumnInfo, QueryResult, TableInfo},
    db2::Db2Adapter,
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
    pub is_connected: bool,
}

fn secret_key(id: &str) -> String {
    format!("db:{id}")
}

async fn build_adapter(
    conn: &DbConnection,
    password: &str,
) -> anyhow::Result<Box<dyn crate::db::adapter::DbAdapter>> {
    match conn.db_type {
        DbType::Postgresql => {
            let url = format!(
                "postgresql://{}:{}@{}:{}/{}",
                conn.username, password, conn.host, conn.port, conn.database
            );
            Ok(Box::new(PostgresAdapter::connect(&url).await?))
        }
        DbType::Mysql => {
            let url = format!(
                "mysql://{}:{}@{}:{}/{}",
                conn.username, password, conn.host, conn.port, conn.database
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
            let cs = format!("DSN={};DATABASE={}", conn.host, conn.database);
            Ok(Box::new(Db2Adapter::new(
                cs,
                conn.username.clone(),
                password.to_string(),
            )))
        }
    }
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
    let _ = secrets.delete(&secret_key(&id));
    Ok(())
}

#[tauri::command]
pub async fn db_test_connection(
    input: DbConnectionInput,
    secrets: State<'_, Arc<SecretStore>>,
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
    };

    let adapter = build_adapter(&temp_conn, &password)
        .await
        .map_err(|e| e.to_string())?;
    adapter.test().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_connect(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    manager: State<'_, DbManager>,
) -> Result<(), String> {
    if manager.is_connected(&id).await {
        return Ok(());
    }
    let conn = config
        .get()
        .db_connections
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("connection not found: {id}"))?;
    let password = secrets
        .get(&secret_key(&id))
        .ok()
        .flatten()
        .unwrap_or_default();
    let adapter = build_adapter(&conn, &password)
        .await
        .map_err(|e| e.to_string())?;
    manager.insert(id, adapter).await;
    Ok(())
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
    manager: State<'_, DbManager>,
) -> Result<QueryResult, String> {
    manager
        .execute(&connection_id, &sql)
        .await
        .map_err(|e| e.to_string())
}
