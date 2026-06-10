//! DB2 adapter — delegates all DB2 operations to the Java db2-sidecar process.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use super::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};
use super::db2_sidecar::Db2SidecarClient;

pub struct Db2Adapter {
    conn_id: String,
    client: Arc<Db2SidecarClient>,
}

impl Db2Adapter {
    /// Establish a DB2 connection via the sidecar.
    ///
    /// `conn_string` must be a JDBC URL, e.g.:
    /// `"jdbc:db2://myhost:50000/mydb"`
    pub async fn connect(
        client: Arc<Db2SidecarClient>,
        conn_string: String,
        username: String,
        password: String,
    ) -> Result<Self> {
        let conn_id = uuid::Uuid::new_v4().to_string();
        let resp = client
            .send(serde_json::json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "cmd": "connect",
                "conn_id": conn_id,
                "conn_string": conn_string,
                "username": username,
                "password": password,
            }))
            .await?;

        if !resp["ok"].as_bool().unwrap_or(false) {
            let err = resp["error"].as_str().unwrap_or("unknown").to_string();
            return Err(anyhow::anyhow!("{err}"));
        }

        Ok(Self { conn_id, client })
    }

    async fn cmd(&self, mut req: serde_json::Value) -> Result<serde_json::Value> {
        req["id"] = serde_json::Value::String(uuid::Uuid::new_v4().to_string());
        req["conn_id"] = serde_json::Value::String(self.conn_id.clone());
        self.client.send(req).await
    }
}

/// Send a disconnect on drop (fire-and-forget).
impl Drop for Db2Adapter {
    fn drop(&mut self) {
        let client = self.client.clone();
        let conn_id = self.conn_id.clone();
        tokio::spawn(async move {
            let _ = client
                .send(serde_json::json!({
                    "id": uuid::Uuid::new_v4().to_string(),
                    "cmd": "disconnect",
                    "conn_id": conn_id,
                }))
                .await;
        });
    }
}

#[async_trait]
impl DbAdapter for Db2Adapter {
    async fn test(&self) -> Result<()> {
        let resp = self.cmd(serde_json::json!({"cmd": "ping"})).await?;
        if resp["ok"].as_bool().unwrap_or(false) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("{}", resp["error"].as_str().unwrap_or("ping failed")))
        }
    }

    async fn list_schemas(&self) -> Result<Vec<String>> {
        let resp = self.cmd(serde_json::json!({"cmd": "list_schemas"})).await?;
        check_ok(&resp)?;
        Ok(resp["rows"]
            .as_array()
            .map(Vec::as_slice).unwrap_or(&[])
            .iter()
            .filter_map(|row| row.as_array()?.first()?.as_str().map(|s| s.to_string()))
            .collect())
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let resp = self
            .cmd(serde_json::json!({"cmd": "list_tables", "schema": schema}))
            .await?;
        check_ok(&resp)?;
        Ok(resp["rows"]
            .as_array()
            .map(Vec::as_slice).unwrap_or(&[])
            .iter()
            .filter_map(|row| {
                let arr = row.as_array()?;
                let name = arr.first()?.as_str()?.to_string();
                let tt = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
                Some(TableInfo {
                    name,
                    table_type: if tt == "V" { "view".into() } else { "table".into() },
                })
            })
            .collect())
    }

    async fn get_table_schema(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let resp = self
            .cmd(serde_json::json!({"cmd": "get_table_schema", "schema": schema, "table": table}))
            .await?;
        check_ok(&resp)?;
        Ok(resp["rows"]
            .as_array()
            .map(Vec::as_slice).unwrap_or(&[])
            .iter()
            .filter_map(|row| {
                let arr = row.as_array()?;
                let s = |i: usize| arr.get(i).and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_default();
                let default_val = s(2);
                Some(ColumnInfo {
                    name: s(0),
                    data_type: s(1),
                    nullable: s(3) == "Y",
                    default: if default_val.is_empty() { None } else { Some(default_val) },
                })
            })
            .collect())
    }

    async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let resp = self
            .cmd(serde_json::json!({"cmd": "execute", "sql": sql}))
            .await?;

        let elapsed = resp["execution_time_ms"].as_u64().unwrap_or(0);

        if !resp["ok"].as_bool().unwrap_or(false) {
            return Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                affected_rows: None,
                execution_time_ms: elapsed,
                error: Some(resp["error"].as_str().unwrap_or("unknown").to_string()),
            });
        }

        let columns: Vec<String> = resp["columns"]
            .as_array()
            .map(Vec::as_slice).unwrap_or(&[])
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let rows: Vec<Vec<serde_json::Value>> = resp["rows"]
            .as_array()
            .map(Vec::as_slice).unwrap_or(&[])
            .iter()
            .map(|row| {
                row.as_array()
                    .map(Vec::as_slice).unwrap_or(&[])
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::Null => serde_json::Value::Null,
                        serde_json::Value::String(s) => serde_json::Value::String(s.clone()),
                        other => other.clone(),
                    })
                    .collect()
            })
            .collect();

        Ok(QueryResult {
            columns,
            rows,
            affected_rows: resp["affected_rows"].as_u64(),
            execution_time_ms: elapsed,
            error: None,
        })
    }
}

fn check_ok(resp: &serde_json::Value) -> Result<()> {
    if resp["ok"].as_bool().unwrap_or(false) {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{}", resp["error"].as_str().unwrap_or("db2 error")))
    }
}
