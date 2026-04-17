//! MSSQL adapter using tiberius (pure-Rust TDS protocol).

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tiberius::{AuthMethod, Client, Config, Row};
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
    // tiberius 0.12: Row::get returns Option<T> directly (not Result<Option<T>>)
    if let Some(v) = row.get::<i64, _>(i) {
        return serde_json::Value::Number(v.into());
    }
    if let Some(v) = row.get::<i32, _>(i) {
        return serde_json::Value::Number((v as i64).into());
    }
    if let Some(v) = row.get::<f64, _>(i) {
        return serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null);
    }
    if let Some(v) = row.get::<bool, _>(i) {
        return serde_json::Value::Bool(v);
    }
    if let Some(v) = row.get::<&str, _>(i) {
        return serde_json::Value::String(v.to_string());
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
            .filter_map(|r| r.get::<&str, _>(0).map(|s: &str| s.to_string()))
            .collect())
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let mut client = self.client.lock().await;
        let sql = format!(
            "SELECT TABLE_NAME, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES \
             WHERE TABLE_SCHEMA = '{}' ORDER BY TABLE_NAME",
            schema.replace('\'', "''")
        );
        let rows = client.simple_query(&sql).await?.into_first_result().await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let name = r.get::<&str, _>(0).unwrap_or("").to_string();
                let tt = r.get::<&str, _>(1).unwrap_or("").to_string();
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
             WHERE TABLE_SCHEMA = '{}' AND TABLE_NAME = '{}' \
             ORDER BY ORDINAL_POSITION",
            schema.replace('\'', "''"),
            table.replace('\'', "''")
        );
        let rows = client.simple_query(&sql).await?.into_first_result().await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let name = r.get::<&str, _>(0).unwrap_or("").to_string();
                let data_type = r.get::<&str, _>(1).unwrap_or("").to_string();
                let default = r.get::<&str, _>(2).map(|s: &str| s.to_string());
                let nullable = r.get::<&str, _>(3).unwrap_or("NO") == "YES";
                ColumnInfo { name, data_type, nullable, default }
            })
            .collect())
    }

    async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let start = std::time::Instant::now();

        // Eagerly collect results inside a block so the MutexGuard (and QueryStream borrow)
        // are fully dropped before we build the QueryResult.
        let query_result: std::result::Result<Vec<Vec<tiberius::Row>>, tiberius::error::Error> = {
            let mut client = self.client.lock().await;
            let x = match client.simple_query(sql).await {
                Ok(stream) => stream.into_results().await,
                Err(e) => Err(e),
            }; x
        };

        match query_result {
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
}
