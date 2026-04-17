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
        // Determine whether this looks like a query returning rows (SELECT / WITH / PRAGMA).
        let trimmed = sql.trim_start().to_uppercase();
        let is_select = trimmed.starts_with("SELECT")
            || trimmed.starts_with("WITH")
            || trimmed.starts_with("PRAGMA")
            || trimmed.starts_with("EXPLAIN");

        if is_select {
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
                Err(e) => Ok(QueryResult {
                    columns: vec![],
                    rows: vec![],
                    affected_rows: None,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                }),
            }
        } else {
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
