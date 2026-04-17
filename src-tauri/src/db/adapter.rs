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
