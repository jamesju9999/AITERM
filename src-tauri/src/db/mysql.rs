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
