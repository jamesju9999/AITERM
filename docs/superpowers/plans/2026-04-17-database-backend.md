# Database Tab — Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Rust `db` module to AITerm with adapters for PostgreSQL, MySQL, SQLite, MSSQL, and DB2, plus Tauri commands exposing connection management and query execution to the frontend.

**Architecture:** A `DbAdapter` trait defines the unified interface. Each database has its own adapter struct. `DbManager` holds live connections keyed by UUID. DB connection configs (without passwords) live in `config.toml`; passwords go to the existing `SecretStore`. All new commands are registered in `lib.rs`.

**Tech Stack:** `sqlx 0.8` (PG/MySQL/SQLite), `tiberius 0.12` (MSSQL), `odbc-api 8` (DB2 via ODBC), `tokio-util` (tiberius compat), existing `anyhow`, `async-trait`, `uuid`, `serde_json`.

---

## File Map

**Create:**
- `src-tauri/src/db/mod.rs`
- `src-tauri/src/db/adapter.rs` — DbAdapter trait + shared types
- `src-tauri/src/db/postgres.rs`
- `src-tauri/src/db/mysql.rs`
- `src-tauri/src/db/sqlite.rs`
- `src-tauri/src/db/mssql.rs`
- `src-tauri/src/db/db2.rs`
- `src-tauri/src/db/manager.rs`
- `src-tauri/src/commands/db.rs`

**Modify:**
- `src-tauri/Cargo.toml` — add sqlx, tiberius, odbc-api, tokio-util
- `src-tauri/src/lib.rs` — declare db module, register DbManager + commands
- `src-tauri/src/config/types.rs` — add `DbConnection`, `DbType`, `db_connections` to `AppConfig`
- `src-tauri/src/config/mod.rs` — add `add_db_connection`, `update_db_connection`, `remove_db_connection`

---

### Task 1: Add Cargo dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add dependencies**

Open `src-tauri/Cargo.toml` and add under `[dependencies]`:

```toml
# Database adapters
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "postgres", "mysql", "sqlite", "uuid"] }
tiberius = { version = "0.12", features = ["tds73", "rustls"] }
tokio-util = { version = "0.7", features = ["compat"] }
odbc-api = "8"
```

- [ ] **Step 2: Verify it compiles (no new code yet)**

```bash
cd src-tauri && cargo check 2>&1 | tail -5
```

Expected: warnings only, no errors.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore(db): add sqlx, tiberius, odbc-api dependencies"
```

---

### Task 2: DB adapter trait and shared types

**Files:**
- Create: `src-tauri/src/db/adapter.rs`
- Create: `src-tauri/src/db/mod.rs`

- [ ] **Step 1: Write the test**

Create `src-tauri/src/db/adapter.rs`:

```rust
//! DbAdapter trait and shared query result types.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Metadata about a table or view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    /// "table" or "view"
    pub table_type: String,
}

/// Metadata about a single column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
}

/// Result of executing a SQL statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    /// Each row is a list of JSON values (null, bool, number, string).
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Rows affected for INSERT/UPDATE/DELETE; None for SELECT.
    pub affected_rows: Option<u64>,
    pub execution_time_ms: u64,
    /// Non-None when the statement returned a DB error (not a Rust error).
    pub error: Option<String>,
}

/// Unified interface every database adapter must implement.
#[async_trait]
pub trait DbAdapter: Send + Sync {
    /// Ping the database. Used for connection health checks.
    async fn test(&self) -> Result<()>;
    /// Return all schema names visible to the connected user.
    async fn list_schemas(&self) -> Result<Vec<String>>;
    /// Return all tables and views in the given schema.
    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>>;
    /// Return column metadata for a specific table.
    async fn get_table_schema(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>>;
    /// Execute arbitrary SQL and return a QueryResult.
    /// This must never propagate DB-level errors as Err — instead,
    /// set `QueryResult::error` and return Ok.
    async fn execute(&self, sql: &str) -> Result<QueryResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_result_serializes() {
        let r = QueryResult {
            columns: vec!["id".into(), "name".into()],
            rows: vec![vec![serde_json::json!(1), serde_json::json!("Alice")]],
            affected_rows: None,
            execution_time_ms: 5,
            error: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("Alice"));
        assert!(json.contains("execution_time_ms"));
    }

    #[test]
    fn table_info_serializes() {
        let t = TableInfo { name: "users".into(), table_type: "table".into() };
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("users"));
    }
}
```

Create `src-tauri/src/db/mod.rs`:

```rust
pub mod adapter;
pub mod manager;
pub mod postgres;
pub mod mysql;
pub mod sqlite;
pub mod mssql;
pub mod db2;

pub use adapter::{DbAdapter, TableInfo, ColumnInfo, QueryResult};
pub use manager::DbManager;
```

- [ ] **Step 2: Run tests**

```bash
cd src-tauri && cargo test db::adapter 2>&1 | tail -10
```

Expected: `test db::adapter::tests::query_result_serializes ... ok` and `table_info_serializes ... ok`

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db/
git commit -m "feat(db): add DbAdapter trait and shared types"
```

---

### Task 3: PostgreSQL adapter

**Files:**
- Create: `src-tauri/src/db/postgres.rs`

- [ ] **Step 1: Write the adapter**

```rust
//! PostgreSQL adapter using sqlx PgPool.

use anyhow::Result;
use async_trait::async_trait;
use sqlx::{Column, PgPool, Row};

use super::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};

pub struct PostgresAdapter {
    pool: PgPool,
}

impl PostgresAdapter {
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPool::connect(url).await?;
        Ok(Self { pool })
    }
}

/// Convert a single PG column value to serde_json::Value.
fn pg_col_to_json(row: &sqlx::postgres::PgRow, i: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
        return v.map(|n| n.into()).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
        return v
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
        return v.map(|b| b.into()).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return v.map(|s| s.into()).unwrap_or(serde_json::Value::Null);
    }
    // Fallback: render as string via Display (handles dates, uuids, etc.)
    if let Ok(v) = row.try_get_unchecked::<Option<String>, _>(i) {
        return v.map(|s| s.into()).unwrap_or(serde_json::Value::Null);
    }
    serde_json::Value::String("<unsupported>".into())
}

#[async_trait]
impl DbAdapter for PostgresAdapter {
    async fn test(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT schema_name FROM information_schema.schemata \
             WHERE schema_name NOT IN ('pg_catalog','information_schema','pg_toast') \
             ORDER BY schema_name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT table_name, table_type \
             FROM information_schema.tables \
             WHERE table_schema = $1 ORDER BY table_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, tt)| TableInfo {
                name,
                table_type: if tt == "VIEW" { "view".into() } else { "table".into() },
            })
            .collect())
    }

    async fn get_table_schema(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let rows: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT column_name, data_type, column_default, is_nullable \
             FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, data_type, default, nullable)| ColumnInfo {
                name,
                data_type,
                nullable: nullable == "YES",
                default,
            })
            .collect())
    }

    async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let start = std::time::Instant::now();
        match sqlx::query(sql).fetch_all(&self.pool).await {
            Ok(rows) => {
                let columns: Vec<String> = rows
                    .first()
                    .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
                    .unwrap_or_default();
                let result_rows = rows
                    .iter()
                    .map(|row| (0..columns.len()).map(|i| pg_col_to_json(row, i)).collect())
                    .collect();
                Ok(QueryResult {
                    columns,
                    rows: result_rows,
                    affected_rows: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: None,
                })
            }
            Err(sqlx::Error::RowNotFound) => Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                affected_rows: None,
                execution_time_ms: start.elapsed().as_millis() as u64,
                error: None,
            }),
            Err(_) => {
                // Try as non-SELECT (INSERT/UPDATE/DELETE/DDL)
                match sqlx::query(sql).execute(&self.pool).await {
                    Ok(r) => Ok(QueryResult {
                        columns: vec![],
                        rows: vec![],
                        affected_rows: Some(r.rows_affected()),
                        execution_time_ms: start.elapsed().as_millis() as u64,
                        error: None,
                    }),
                    Err(e) => Ok(QueryResult {
                        columns: vec![],
                        rows: vec![],
                        affected_rows: None,
                        execution_time_ms: start.elapsed().as_millis() as u64,
                        error: Some(e.to_string()),
                    }),
                }
            }
        }
    }
}
```

- [ ] **Step 2: Check it compiles**

```bash
cd src-tauri && cargo check 2>&1 | grep "^error" | head -10
```

Expected: no `error` lines.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db/postgres.rs
git commit -m "feat(db): add PostgreSQL adapter"
```

---

### Task 4: MySQL adapter

**Files:**
- Create: `src-tauri/src/db/mysql.rs`

- [ ] **Step 1: Write the adapter**

```rust
//! MySQL / MariaDB adapter using sqlx MySqlPool.

use anyhow::Result;
use async_trait::async_trait;
use sqlx::{Column, MySqlPool, Row};

use super::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};

pub struct MySqlAdapter {
    pool: MySqlPool,
    database: String,
}

impl MySqlAdapter {
    pub async fn connect(url: &str, database: &str) -> Result<Self> {
        let pool = MySqlPool::connect(url).await?;
        Ok(Self { pool, database: database.to_string() })
    }
}

fn mysql_col_to_json(row: &sqlx::mysql::MySqlRow, i: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
        return v.map(|n| n.into()).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
        return v
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
        return v.map(|b| b.into()).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return v.map(|s| s.into()).unwrap_or(serde_json::Value::Null);
    }
    serde_json::Value::Null
}

#[async_trait]
impl DbAdapter for MySqlAdapter {
    async fn test(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT schema_name FROM information_schema.schemata \
             WHERE schema_name NOT IN ('information_schema','performance_schema','sys','mysql') \
             ORDER BY schema_name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT table_name, table_type \
             FROM information_schema.tables \
             WHERE table_schema = ? ORDER BY table_name",
        )
        .bind(schema)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, tt)| TableInfo {
                name,
                table_type: if tt == "VIEW" { "view".into() } else { "table".into() },
            })
            .collect())
    }

    async fn get_table_schema(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let rows: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT column_name, column_type, column_default, is_nullable \
             FROM information_schema.columns \
             WHERE table_schema = ? AND table_name = ? \
             ORDER BY ordinal_position",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, data_type, default, nullable)| ColumnInfo {
                name,
                data_type,
                nullable: nullable == "YES",
                default,
            })
            .collect())
    }

    async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let start = std::time::Instant::now();
        match sqlx::query(sql).fetch_all(&self.pool).await {
            Ok(rows) => {
                let columns: Vec<String> = rows
                    .first()
                    .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
                    .unwrap_or_default();
                let result_rows = rows
                    .iter()
                    .map(|row| (0..columns.len()).map(|i| mysql_col_to_json(row, i)).collect())
                    .collect();
                Ok(QueryResult {
                    columns,
                    rows: result_rows,
                    affected_rows: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: None,
                })
            }
            Err(_) => match sqlx::query(sql).execute(&self.pool).await {
                Ok(r) => Ok(QueryResult {
                    columns: vec![],
                    rows: vec![],
                    affected_rows: Some(r.rows_affected()),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: None,
                }),
                Err(e) => Ok(QueryResult {
                    columns: vec![],
                    rows: vec![],
                    affected_rows: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                }),
            },
        }
    }
}
```

- [ ] **Step 2: Compile check**

```bash
cd src-tauri && cargo check 2>&1 | grep "^error" | head -10
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db/mysql.rs
git commit -m "feat(db): add MySQL/MariaDB adapter"
```

---

### Task 5: SQLite adapter

**Files:**
- Create: `src-tauri/src/db/sqlite.rs`

- [ ] **Step 1: Write the adapter**

```rust
//! SQLite adapter using sqlx SqlitePool.

use anyhow::Result;
use async_trait::async_trait;
use sqlx::{Column, Row, SqlitePool};

use super::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};

pub struct SqliteAdapter {
    pool: SqlitePool,
}

impl SqliteAdapter {
    /// `path` is a file path, e.g. "/home/user/mydb.sqlite".
    /// Pass ":memory:" for in-memory (testing only).
    pub async fn connect(path: &str) -> Result<Self> {
        let url = if path == ":memory:" {
            "sqlite::memory:".to_string()
        } else {
            format!("sqlite:{path}")
        };
        let pool = SqlitePool::connect(&url).await?;
        Ok(Self { pool })
    }
}

fn sqlite_col_to_json(row: &sqlx::sqlite::SqliteRow, i: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
        return v.map(|n| n.into()).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
        return v
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return v.map(|s| s.into()).unwrap_or(serde_json::Value::Null);
    }
    serde_json::Value::Null
}

#[async_trait]
impl DbAdapter for SqliteAdapter {
    async fn test(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        // SQLite has no schemas; return a single "main"
        Ok(vec!["main".to_string()])
    }

    async fn list_tables(&self, _schema: &str) -> Result<Vec<TableInfo>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, type FROM sqlite_master \
             WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, tt)| TableInfo { name, table_type: tt })
            .collect())
    }

    async fn get_table_schema(&self, _schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let rows: Vec<(i64, String, String, bool, Option<String>)> =
            sqlx::query_as(&format!("PRAGMA table_info('{table}')"))
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(_cid, name, data_type, not_null, default)| ColumnInfo {
                name,
                data_type,
                nullable: !not_null,
                default,
            })
            .collect())
    }

    async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let start = std::time::Instant::now();
        match sqlx::query(sql).fetch_all(&self.pool).await {
            Ok(rows) => {
                let columns: Vec<String> = rows
                    .first()
                    .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
                    .unwrap_or_default();
                let result_rows = rows
                    .iter()
                    .map(|row| (0..columns.len()).map(|i| sqlite_col_to_json(row, i)).collect())
                    .collect();
                Ok(QueryResult {
                    columns,
                    rows: result_rows,
                    affected_rows: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: None,
                })
            }
            Err(_) => match sqlx::query(sql).execute(&self.pool).await {
                Ok(r) => Ok(QueryResult {
                    columns: vec![],
                    rows: vec![],
                    affected_rows: Some(r.rows_affected()),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: None,
                }),
                Err(e) => Ok(QueryResult {
                    columns: vec![],
                    rows: vec![],
                    affected_rows: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_in_memory_roundtrip() {
        let db = SqliteAdapter::connect(":memory:").await.unwrap();
        db.test().await.unwrap();

        let create = db.execute("CREATE TABLE t (id INTEGER, name TEXT)").await.unwrap();
        assert!(create.error.is_none());

        let insert = db.execute("INSERT INTO t VALUES (1, 'Alice')").await.unwrap();
        assert_eq!(insert.affected_rows, Some(1));

        let select = db.execute("SELECT id, name FROM t").await.unwrap();
        assert_eq!(select.columns, vec!["id", "name"]);
        assert_eq!(select.rows.len(), 1);
        assert_eq!(select.rows[0][1], serde_json::json!("Alice"));
    }

    #[tokio::test]
    async fn sqlite_list_tables() {
        let db = SqliteAdapter::connect(":memory:").await.unwrap();
        db.execute("CREATE TABLE users (id INTEGER)").await.unwrap();
        let tables = db.list_tables("main").await.unwrap();
        assert!(tables.iter().any(|t| t.name == "users"));
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cd src-tauri && cargo test db::sqlite 2>&1 | tail -10
```

Expected: both tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db/sqlite.rs
git commit -m "feat(db): add SQLite adapter with in-memory tests"
```

---

### Task 6: MSSQL adapter

**Files:**
- Create: `src-tauri/src/db/mssql.rs`

- [ ] **Step 1: Write the adapter**

```rust
//! MSSQL adapter using tiberius (pure-Rust TDS protocol).

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tiberius::{AuthMethod, Client, Config, QueryItem, Row};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use super::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};

pub struct MssqlAdapter {
    client: Arc<Mutex<Client<tokio_util::compat::Compat<TcpStream>>>>,
    database: String,
}

impl MssqlAdapter {
    pub async fn connect(host: &str, port: u16, database: &str, username: &str, password: &str) -> Result<Self> {
        let mut config = Config::new();
        config.host(host);
        config.port(port);
        config.database(database);
        config.authentication(AuthMethod::sql_server(username, password));
        config.trust_cert(); // Accept self-signed certs; adjust for production

        let tcp = TcpStream::connect(config.get_addr())
            .await
            .with_context(|| format!("TCP connect to {host}:{port}"))?;
        tcp.set_nodelay(true)?;

        let client = Client::connect(config, tcp.compat_write())
            .await
            .context("MSSQL TDS handshake")?;

        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            database: database.to_string(),
        })
    }
}

fn mssql_col_to_json(row: &Row, i: usize) -> serde_json::Value {
    // tiberius Row::get returns Option<T> for known types
    if let Some(Some(v)) = row.get::<Option<i64>, _>(i).ok() {
        return v.into();
    }
    if let Some(Some(v)) = row.get::<Option<i32>, _>(i).ok() {
        return (v as i64).into();
    }
    if let Some(Some(v)) = row.get::<Option<f64>, _>(i).ok() {
        return serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null);
    }
    if let Some(Some(v)) = row.get::<Option<bool>, _>(i).ok() {
        return v.into();
    }
    if let Some(Some(v)) = row.get::<Option<&str>, _>(i).ok() {
        return v.to_string().into();
    }
    serde_json::Value::Null
}

#[async_trait]
impl DbAdapter for MssqlAdapter {
    async fn test(&self) -> Result<()> {
        let mut client = self.client.lock().await;
        client.simple_query("SELECT 1").await?.into_results().await?;
        Ok(())
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        let mut client = self.client.lock().await;
        let rows = client
            .simple_query(
                "SELECT name FROM sys.schemas \
                 WHERE name NOT IN ('sys','INFORMATION_SCHEMA','guest','db_owner',\
                 'db_accessadmin','db_securityadmin','db_ddladmin','db_backupoperator',\
                 'db_datareader','db_datawriter','db_denydatareader','db_denydatawriter') \
                 ORDER BY name",
            )
            .await?
            .into_first_result()
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.get::<&str, _>(0).ok().flatten().map(|s| s.to_string()))
            .collect())
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let mut client = self.client.lock().await;
        let sql = format!(
            "SELECT TABLE_NAME, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_SCHEMA = '{schema}' ORDER BY TABLE_NAME"
        );
        let rows = client.simple_query(&sql).await?.into_first_result().await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let name = r.get::<&str, _>(0).ok().flatten().unwrap_or("").to_string();
                let tt = r.get::<&str, _>(1).ok().flatten().unwrap_or("").to_string();
                TableInfo {
                    name,
                    table_type: if tt.contains("VIEW") { "view".into() } else { "table".into() },
                }
            })
            .collect())
    }

    async fn get_table_schema(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let mut client = self.client.lock().await;
        let sql = format!(
            "SELECT COLUMN_NAME, DATA_TYPE, COLUMN_DEFAULT, IS_NULLABLE \
             FROM INFORMATION_SCHEMA.COLUMNS \
             WHERE TABLE_SCHEMA = '{schema}' AND TABLE_NAME = '{table}' \
             ORDER BY ORDINAL_POSITION"
        );
        let rows = client.simple_query(&sql).await?.into_first_result().await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let name = r.get::<&str, _>(0).ok().flatten().unwrap_or("").to_string();
                let data_type = r.get::<&str, _>(1).ok().flatten().unwrap_or("").to_string();
                let default = r.get::<&str, _>(2).ok().flatten().map(|s| s.to_string());
                let nullable = r.get::<&str, _>(3).ok().flatten().unwrap_or("NO") == "YES";
                ColumnInfo { name, data_type, nullable, default }
            })
            .collect())
    }

    async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let start = std::time::Instant::now();
        let mut client = self.client.lock().await;

        match client.simple_query(sql).await {
            Ok(stream) => {
                match stream.into_results().await {
                    Ok(result_sets) => {
                        // Take the first non-empty result set
                        let rows = result_sets.into_iter().find(|rs| !rs.is_empty()).unwrap_or_default();
                        if rows.is_empty() {
                            return Ok(QueryResult {
                                columns: vec![],
                                rows: vec![],
                                affected_rows: None,
                                execution_time_ms: start.elapsed().as_millis() as u64,
                                error: None,
                            });
                        }
                        let col_count = rows[0].columns().len();
                        let columns: Vec<String> = rows[0].columns().iter().map(|c| c.name().to_string()).collect();
                        let result_rows = rows.iter()
                            .map(|row| (0..col_count).map(|i| mssql_col_to_json(row, i)).collect())
                            .collect();
                        Ok(QueryResult {
                            columns,
                            rows: result_rows,
                            affected_rows: None,
                            execution_time_ms: start.elapsed().as_millis() as u64,
                            error: None,
                        })
                    }
                    Err(e) => Ok(QueryResult {
                        columns: vec![],
                        rows: vec![],
                        affected_rows: None,
                        execution_time_ms: start.elapsed().as_millis() as u64,
                        error: Some(e.to_string()),
                    }),
                }
            }
            Err(e) => Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                affected_rows: None,
                execution_time_ms: start.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
            }),
        }
    }
}
```

- [ ] **Step 2: Compile check**

```bash
cd src-tauri && cargo check 2>&1 | grep "^error" | head -10
```

Expected: no errors. (Integration tests require a live MSSQL instance — skip for now.)

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db/mssql.rs
git commit -m "feat(db): add MSSQL adapter via tiberius"
```

---

### Task 7: DB2 adapter

**Files:**
- Create: `src-tauri/src/db/db2.rs`

- [ ] **Step 1: Write the adapter**

```rust
//! DB2 adapter via ODBC (requires IBM DB2 ODBC Driver installed on the host).
//!
//! Users must install IBM Data Server Driver Package before this adapter works.
//! On connection failure with an ODBC error mentioning "driver not found",
//! the Tauri command returns error code `odbc_driver_not_found` so the frontend
//! can show the installation guide instead of a generic error.

use anyhow::{bail, Result};
use async_trait::async_trait;
use odbc_api::{buffers::TextRowSet, Connection, ConnectionOptions, Cursor, Environment, ResultSetMetadata};
use std::sync::{Arc, Mutex};

use super::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};

/// Thread-safe wrapper: odbc-api connections are `!Send`, so we wrap in Mutex
/// and use `spawn_blocking` to keep them off async threads.
pub struct Db2Adapter {
    /// DSN string, e.g. "DSN=mydb" or a full ODBC connection string.
    conn_string: String,
    username: String,
    password: String,
}

impl Db2Adapter {
    pub fn new(conn_string: String, username: String, password: String) -> Self {
        Self { conn_string, username, password }
    }

    /// Test that the IBM ODBC driver is installed by attempting a connection.
    /// Returns Err with message containing "odbc_driver_not_found" if missing.
    pub fn try_connect_sync(conn_string: &str, username: &str, password: &str) -> Result<()> {
        let env = Environment::new().map_err(|e| anyhow::anyhow!("ODBC env: {e}"))?;
        env.connect(conn_string, username, password, ConnectionOptions::default())
            .map_err(|e| {
                let msg = e.to_string();
                if msg.to_lowercase().contains("driver") || msg.to_lowercase().contains("dsn") {
                    anyhow::anyhow!("odbc_driver_not_found: {msg}")
                } else {
                    anyhow::anyhow!("{msg}")
                }
            })?;
        Ok(())
    }

    /// Execute a query synchronously (must be called from spawn_blocking).
    fn execute_sync(conn_string: &str, username: &str, password: &str, sql: &str) -> QueryResult {
        let start = std::time::Instant::now();
        let env = match Environment::new() {
            Ok(e) => e,
            Err(e) => return QueryResult {
                columns: vec![], rows: vec![], affected_rows: None,
                execution_time_ms: start.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
            },
        };
        let conn = match env.connect(conn_string, username, password, ConnectionOptions::default()) {
            Ok(c) => c,
            Err(e) => return QueryResult {
                columns: vec![], rows: vec![], affected_rows: None,
                execution_time_ms: start.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
            },
        };

        match conn.execute(sql, ()) {
            Err(e) => QueryResult {
                columns: vec![], rows: vec![], affected_rows: None,
                execution_time_ms: start.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
            },
            Ok(None) => {
                // Non-SELECT (DDL / DML)
                QueryResult {
                    columns: vec![], rows: vec![], affected_rows: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: None,
                }
            }
            Ok(Some(mut cursor)) => {
                let cols: Vec<String> = (1..=cursor.num_result_cols().unwrap_or(0) as u16)
                    .filter_map(|i| cursor.col_name(i).ok())
                    .collect();

                let mut buf = match TextRowSet::for_cursor(100, &mut cursor, Some(4096)) {
                    Ok(b) => b,
                    Err(e) => return QueryResult {
                        columns: cols, rows: vec![], affected_rows: None,
                        execution_time_ms: start.elapsed().as_millis() as u64,
                        error: Some(e.to_string()),
                    },
                };

                let mut row_set_cursor = match cursor.bind_buffer(&mut buf) {
                    Ok(c) => c,
                    Err(e) => return QueryResult {
                        columns: cols, rows: vec![], affected_rows: None,
                        execution_time_ms: start.elapsed().as_millis() as u64,
                        error: Some(e.to_string()),
                    },
                };

                let mut all_rows: Vec<Vec<serde_json::Value>> = vec![];
                while let Ok(Some(batch)) = row_set_cursor.fetch() {
                    for row_idx in 0..batch.num_rows() {
                        let row: Vec<serde_json::Value> = (0..cols.len())
                            .map(|col_idx| {
                                batch.at(col_idx, row_idx)
                                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                                    .map(|s| serde_json::Value::String(s.to_string()))
                                    .unwrap_or(serde_json::Value::Null)
                            })
                            .collect();
                        all_rows.push(row);
                    }
                }

                QueryResult {
                    columns: cols,
                    rows: all_rows,
                    affected_rows: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: None,
                }
            }
        }
    }
}

#[async_trait]
impl DbAdapter for Db2Adapter {
    async fn test(&self) -> Result<()> {
        let cs = self.conn_string.clone();
        let u = self.username.clone();
        let p = self.password.clone();
        tokio::task::spawn_blocking(move || Self::try_connect_sync(&cs, &u, &p))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking: {e}"))??;
        Ok(())
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        let sql = "SELECT DISTINCT SCHEMANAME FROM SYSCAT.SCHEMATA \
                   WHERE DEFINERTYPE = 'U' ORDER BY SCHEMANAME".to_string();
        let result = self.execute(&sql).await?;
        Ok(result.rows.into_iter()
            .filter_map(|r| r.into_iter().next())
            .filter_map(|v| if let serde_json::Value::String(s) = v { Some(s) } else { None })
            .collect())
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let sql = format!(
            "SELECT TABNAME, TYPE FROM SYSCAT.TABLES \
             WHERE TABSCHEMA = '{schema}' ORDER BY TABNAME"
        );
        let result = self.execute(&sql).await?;
        Ok(result.rows.into_iter().filter_map(|r| {
            let name = r.get(0).and_then(|v| if let serde_json::Value::String(s) = v { Some(s.clone()) } else { None })?;
            let tt = r.get(1).and_then(|v| if let serde_json::Value::String(s) = v { Some(s.clone()) } else { None }).unwrap_or_default();
            Some(TableInfo {
                name,
                table_type: if tt == "V" { "view".into() } else { "table".into() },
            })
        }).collect())
    }

    async fn get_table_schema(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let sql = format!(
            "SELECT COLNAME, TYPENAME, DEFAULT, NULLS \
             FROM SYSCAT.COLUMNS \
             WHERE TABSCHEMA = '{schema}' AND TABNAME = '{table}' \
             ORDER BY COLNO"
        );
        let result = self.execute(&sql).await?;
        Ok(result.rows.into_iter().map(|r| {
            let get = |i: usize| r.get(i).and_then(|v| if let serde_json::Value::String(s) = v { Some(s.clone()) } else { None }).unwrap_or_default();
            ColumnInfo {
                name: get(0),
                data_type: get(1),
                nullable: get(3) == "Y",
                default: { let d = get(2); if d.is_empty() { None } else { Some(d) } },
            }
        }).collect())
    }

    async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let cs = self.conn_string.clone();
        let u = self.username.clone();
        let p = self.password.clone();
        let sql = sql.to_string();
        let result = tokio::task::spawn_blocking(move || {
            Self::execute_sync(&cs, &u, &p, &sql)
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking: {e}"))?;
        Ok(result)
    }
}
```

- [ ] **Step 2: Compile check**

```bash
cd src-tauri && cargo check 2>&1 | grep "^error" | head -10
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db/db2.rs
git commit -m "feat(db): add DB2 adapter via ODBC"
```

---

### Task 8: DbManager

**Files:**
- Create: `src-tauri/src/db/manager.rs`

- [ ] **Step 1: Write tests first**

```rust
//! Manages live database connections keyed by connection UUID.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::adapter::{DbAdapter, QueryResult, TableInfo, ColumnInfo};

pub struct DbManager {
    connections: Arc<RwLock<HashMap<String, Box<dyn DbAdapter>>>>,
}

impl DbManager {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Store a live adapter under the given connection id.
    pub async fn insert(&self, id: String, adapter: Box<dyn DbAdapter>) {
        self.connections.write().await.insert(id, adapter);
    }

    /// Remove a connection. Returns true if it existed.
    pub async fn remove(&self, id: &str) -> bool {
        self.connections.write().await.remove(id).is_some()
    }

    /// Returns true if a live connection exists for this id.
    pub async fn is_connected(&self, id: &str) -> bool {
        self.connections.read().await.contains_key(id)
    }

    /// Execute SQL via the named connection. Returns Err if not connected.
    pub async fn execute(&self, id: &str, sql: &str) -> anyhow::Result<QueryResult> {
        let conns = self.connections.read().await;
        let adapter = conns.get(id)
            .ok_or_else(|| anyhow::anyhow!("not_connected: {id}"))?;
        adapter.execute(sql).await
    }

    /// List tables via the named connection.
    pub async fn list_tables(&self, id: &str, schema: &str) -> anyhow::Result<Vec<TableInfo>> {
        let conns = self.connections.read().await;
        let adapter = conns.get(id)
            .ok_or_else(|| anyhow::anyhow!("not_connected: {id}"))?;
        adapter.list_tables(schema).await
    }

    /// List schemas via the named connection.
    pub async fn list_schemas(&self, id: &str) -> anyhow::Result<Vec<String>> {
        let conns = self.connections.read().await;
        let adapter = conns.get(id)
            .ok_or_else(|| anyhow::anyhow!("not_connected: {id}"))?;
        adapter.list_schemas().await
    }

    /// Get table column info via the named connection.
    pub async fn get_table_schema(&self, id: &str, schema: &str, table: &str) -> anyhow::Result<Vec<ColumnInfo>> {
        let conns = self.connections.read().await;
        let adapter = conns.get(id)
            .ok_or_else(|| anyhow::anyhow!("not_connected: {id}"))?;
        adapter.get_table_schema(schema, table).await
    }
}

impl Default for DbManager {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sqlite::SqliteAdapter;

    #[tokio::test]
    async fn insert_and_execute() {
        let mgr = DbManager::new();
        let adapter = SqliteAdapter::connect(":memory:").await.unwrap();
        mgr.insert("test-id".into(), Box::new(adapter)).await;

        assert!(mgr.is_connected("test-id").await);

        let result = mgr.execute("test-id", "SELECT 42 as n").await.unwrap();
        assert_eq!(result.columns, vec!["n"]);
        assert_eq!(result.rows[0][0], serde_json::json!(42));
    }

    #[tokio::test]
    async fn remove_connection() {
        let mgr = DbManager::new();
        let adapter = SqliteAdapter::connect(":memory:").await.unwrap();
        mgr.insert("x".into(), Box::new(adapter)).await;
        assert!(mgr.remove("x").await);
        assert!(!mgr.is_connected("x").await);
    }

    #[tokio::test]
    async fn execute_not_connected_returns_err() {
        let mgr = DbManager::new();
        let err = mgr.execute("nonexistent", "SELECT 1").await.unwrap_err();
        assert!(err.to_string().contains("not_connected"));
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cd src-tauri && cargo test db::manager 2>&1 | tail -10
```

Expected: all 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db/manager.rs
git commit -m "feat(db): add DbManager with tests"
```

---

### Task 9: Add DbConnection to config types

**Files:**
- Modify: `src-tauri/src/config/types.rs`

- [ ] **Step 1: Add types (write tests first)**

Add to the end of `src-tauri/src/config/types.rs` (before the `#[cfg(test)]` block):

```rust
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
```

Also add `db_connections` to `AppConfig`:

```rust
    /// Saved database connections (passwords stored separately in Keychain).
    #[serde(default)]
    pub db_connections: Vec<DbConnection>,
```

- [ ] **Step 2: Add a test to the existing tests block**

In the `#[cfg(test)]` block at the bottom of `types.rs`, add:

```rust
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
```

- [ ] **Step 3: Run tests**

```bash
cd src-tauri && cargo test config::types 2>&1 | tail -15
```

Expected: all tests pass including the two new ones.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/config/types.rs
git commit -m "feat(config): add DbConnection and DbType to AppConfig"
```

---

### Task 10: Add config CRUD for DB connections

**Files:**
- Modify: `src-tauri/src/config/mod.rs`

- [ ] **Step 1: Add methods to ConfigStore**

Add these methods to `impl ConfigStore` in `src-tauri/src/config/mod.rs`:

```rust
    /// Add a new DB connection config. Returns Err if id already exists.
    pub fn add_db_connection(&self, conn: crate::config::types::DbConnection) -> anyhow::Result<()> {
        self.update(|cfg| {
            cfg.db_connections.push(conn);
        })
    }

    /// Update an existing DB connection. Returns Err if id not found.
    pub fn update_db_connection(&self, conn: crate::config::types::DbConnection) -> anyhow::Result<()> {
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
```

- [ ] **Step 2: Add tests in config/mod.rs test block**

```rust
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
```

- [ ] **Step 3: Run tests**

```bash
cd src-tauri && cargo test config 2>&1 | tail -15
```

Expected: all config tests pass including `db_connection_crud`.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/config/mod.rs
git commit -m "feat(config): add DB connection CRUD methods"
```

---

### Task 11: Tauri DB commands

**Files:**
- Create: `src-tauri/src/commands/db.rs`

- [ ] **Step 1: Write the commands**

```rust
//! Tauri commands for database connection management and query execution.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::config::{types::{DbConnection, DbType}, ConfigStore};
use crate::db::{
    adapter::QueryResult,
    adapter::{ColumnInfo, TableInfo},
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

async fn build_adapter(conn: &DbConnection, password: &str) -> anyhow::Result<Box<dyn crate::db::adapter::DbAdapter>> {
    match conn.db_type {
        DbType::Postgresql => {
            let url = format!("postgresql://{}:{}@{}:{}/{}", conn.username, password, conn.host, conn.port, conn.database);
            Ok(Box::new(PostgresAdapter::connect(&url).await?))
        }
        DbType::Mysql => {
            let url = format!("mysql://{}:{}@{}:{}/{}", conn.username, password, conn.host, conn.port, conn.database);
            Ok(Box::new(MySqlAdapter::connect(&url, &conn.database).await?))
        }
        DbType::Sqlite => {
            Ok(Box::new(SqliteAdapter::connect(&conn.host).await?))
        }
        DbType::Mssql => {
            Ok(Box::new(MssqlAdapter::connect(&conn.host, conn.port, &conn.database, &conn.username, password).await?))
        }
        DbType::Db2 => {
            let cs = format!("DSN={};DATABASE={}", conn.host, conn.database);
            Ok(Box::new(Db2Adapter::new(cs, conn.username.clone(), password.to_string())))
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
    secrets.set(&secret_key(&id), &input.password).map_err(|e| e.to_string())?;
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
    config.update_db_connection(conn).map_err(|e| e.to_string())?;
    if !input.password.is_empty() {
        secrets.set(&secret_key(&id), &input.password).map_err(|e| e.to_string())?;
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
    config.remove_db_connection(&id).map_err(|e| e.to_string())?;
    let _ = secrets.delete(&secret_key(&id));
    Ok(())
}

#[tauri::command]
pub async fn db_test_connection(
    input: DbConnectionInput,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    // If id is provided, look up saved password if new password is empty
    let password = if !input.password.is_empty() {
        input.password.clone()
    } else if let Some(id) = &input.id {
        secrets.get(&secret_key(id)).ok().flatten().unwrap_or_default()
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

    let adapter = build_adapter(&temp_conn, &password).await.map_err(|e| e.to_string())?;
    adapter.test().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_connect(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    manager: State<'_, DbManager>,
) -> Result<(), String> {
    // Already connected? Nothing to do.
    if manager.is_connected(&id).await {
        return Ok(());
    }
    let conn = config.get().db_connections.into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("connection not found: {id}"))?;
    let password = secrets.get(&secret_key(&id))
        .ok().flatten().unwrap_or_default();
    let adapter = build_adapter(&conn, &password).await.map_err(|e| e.to_string())?;
    manager.insert(id, adapter).await;
    Ok(())
}

#[tauri::command]
pub async fn db_disconnect(
    id: String,
    manager: State<'_, DbManager>,
) -> Result<(), String> {
    manager.remove(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn db_list_schemas(
    connection_id: String,
    manager: State<'_, DbManager>,
) -> Result<Vec<String>, String> {
    manager.list_schemas(&connection_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_list_tables(
    connection_id: String,
    schema: String,
    manager: State<'_, DbManager>,
) -> Result<Vec<TableInfo>, String> {
    manager.list_tables(&connection_id, &schema).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_get_table_schema(
    connection_id: String,
    schema: String,
    table: String,
    manager: State<'_, DbManager>,
) -> Result<Vec<ColumnInfo>, String> {
    manager.get_table_schema(&connection_id, &schema, &table).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_execute_query(
    connection_id: String,
    sql: String,
    manager: State<'_, DbManager>,
) -> Result<QueryResult, String> {
    manager.execute(&connection_id, &sql).await.map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Compile check**

```bash
cd src-tauri && cargo check 2>&1 | grep "^error" | head -10
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands/db.rs
git commit -m "feat(db): add Tauri DB commands"
```

---

### Task 12: Register db module in lib.rs

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Update lib.rs**

Add `pub mod db;` at the top with the other module declarations:

```rust
pub mod db;
```

Add the db command imports:

```rust
use commands::db::{
    db_add_connection, db_connect, db_disconnect, db_execute_query,
    db_get_table_schema, db_list_connections, db_list_schemas,
    db_list_tables, db_remove_connection, db_test_connection, db_update_connection,
};
use db::manager::DbManager;
```

Add `.manage(DbManager::new())` to the builder (after the existing `.manage(router)`):

```rust
        .manage(DbManager::new())
```

Add all db commands to `tauri::generate_handler![]`:

```rust
            // Database
            db_list_connections,
            db_add_connection,
            db_update_connection,
            db_remove_connection,
            db_test_connection,
            db_connect,
            db_disconnect,
            db_list_schemas,
            db_list_tables,
            db_get_table_schema,
            db_execute_query,
```

- [ ] **Step 2: Full build check**

```bash
cd src-tauri && cargo build 2>&1 | grep "^error" | head -10
```

Expected: builds cleanly (warnings OK).

- [ ] **Step 3: Run all backend tests**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(db): register db module and commands in Tauri app"
```

---

*Backend plan complete. Continue with `2026-04-17-database-frontend.md` for the frontend implementation.*
