//! Manages live database connections keyed by connection UUID.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};

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
        let adapter = conns
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("not_connected: {id}"))?;
        adapter.execute(sql).await
    }

    /// List tables via the named connection.
    pub async fn list_tables(&self, id: &str, schema: &str) -> anyhow::Result<Vec<TableInfo>> {
        let conns = self.connections.read().await;
        let adapter = conns
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("not_connected: {id}"))?;
        adapter.list_tables(schema).await
    }

    /// List schemas via the named connection.
    pub async fn list_schemas(&self, id: &str) -> anyhow::Result<Vec<String>> {
        let conns = self.connections.read().await;
        let adapter = conns
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("not_connected: {id}"))?;
        adapter.list_schemas().await
    }

    /// Get table column info via the named connection.
    pub async fn get_table_schema(
        &self,
        id: &str,
        schema: &str,
        table: &str,
    ) -> anyhow::Result<Vec<ColumnInfo>> {
        let conns = self.connections.read().await;
        let adapter = conns
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("not_connected: {id}"))?;
        adapter.get_table_schema(schema, table).await
    }
}

impl Default for DbManager {
    fn default() -> Self {
        Self::new()
    }
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
