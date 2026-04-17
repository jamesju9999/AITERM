//! DB2 adapter via ODBC (requires IBM DB2 ODBC Driver installed on the host).
//!
//! Users must install IBM Data Server Driver Package before this adapter works.
//! On connection failure with an ODBC error mentioning "driver" or "dsn",
//! the error message will contain "odbc_driver_not_found" so the frontend
//! can show the installation guide instead of a generic error.

use anyhow::Result;
use async_trait::async_trait;
use odbc_api::{buffers::TextRowSet, ConnectionOptions, Cursor, Environment, ResultSetMetadata};

use super::adapter::{ColumnInfo, DbAdapter, QueryResult, TableInfo};

/// DB2 adapter using ODBC.
///
/// The ODBC `Environment` and `Connection` types are `!Send`, so we store
/// only the connection parameters and open a fresh connection per query via
/// `tokio::task::spawn_blocking`.
pub struct Db2Adapter {
    /// ODBC connection string, e.g. "DSN=mydb" or a full DSN-less string.
    conn_string: String,
    username: String,
    password: String,
}

impl Db2Adapter {
    pub fn new(conn_string: String, username: String, password: String) -> Self {
        Self { conn_string, username, password }
    }

    /// Test that the IBM ODBC driver is installed by attempting a connection.
    /// Returns Err with message prefixed "odbc_driver_not_found:" if driver missing.
    fn try_connect_sync(conn_string: &str, username: &str, password: &str) -> Result<()> {
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

    /// Execute a query synchronously. Must be called from `spawn_blocking`.
    fn execute_sync(conn_string: &str, username: &str, password: &str, sql: &str) -> QueryResult {
        let start = std::time::Instant::now();

        // Run the ODBC operations in a closure so borrow lifetimes are properly scoped.
        // The closure returns Result<(cols, rows), String> — the cursor is fully consumed
        // (and dropped) inside before we return owned data.
        let result: Result<(Vec<String>, Vec<Vec<serde_json::Value>>), String> = (|| {
            let env = Environment::new().map_err(|e| e.to_string())?;
            let conn = env
                .connect(conn_string, username, password, ConnectionOptions::default())
                .map_err(|e| e.to_string())?;

            let mut cursor = match conn.execute(sql, ()).map_err(|e| e.to_string())? {
                None => return Ok((vec![], vec![])),
                Some(c) => c,
            };

            let num_cols = match cursor.num_result_cols() {
                Ok(n) => n.max(0) as u16,
                Err(e) => return Err(e.to_string()),
            };
            let cols: Vec<String> = (1..=num_cols)
                .filter_map(|i| cursor.col_name(i).ok())
                .collect();

            let buf = TextRowSet::for_cursor(100, &mut cursor, Some(4096))
                .map_err(|e| e.to_string())?;

            let mut row_set_cursor = cursor.bind_buffer(buf).map_err(|e| e.to_string())?;

            let mut all_rows: Vec<Vec<serde_json::Value>> = vec![];
            loop {
                match row_set_cursor.fetch() {
                    Ok(Some(batch)) => {
                        for row_idx in 0..batch.num_rows() {
                            let row: Vec<serde_json::Value> = (0..cols.len())
                                .map(|col_idx| {
                                    batch
                                        .at(col_idx, row_idx)
                                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                                        .map(|s| serde_json::Value::String(s.to_string()))
                                        .unwrap_or(serde_json::Value::Null)
                                })
                                .collect();
                            all_rows.push(row);
                        }
                    }
                    Ok(None) => break,
                    Err(e) => return Err(e.to_string()),
                }
            }

            Ok((cols, all_rows))
        })();

        let elapsed = start.elapsed().as_millis() as u64;
        match result {
            Ok((cols, rows)) => QueryResult {
                columns: cols,
                rows,
                affected_rows: None,
                execution_time_ms: elapsed,
                error: None,
            },
            Err(msg) => QueryResult {
                columns: vec![],
                rows: vec![],
                affected_rows: None,
                execution_time_ms: elapsed,
                error: Some(msg),
            },
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
                   WHERE DEFINERTYPE = 'U' ORDER BY SCHEMANAME"
            .to_string();
        let result = self.execute(&sql).await?;
        Ok(result
            .rows
            .into_iter()
            .filter_map(|r| r.into_iter().next())
            .filter_map(|v| {
                if let serde_json::Value::String(s) = v {
                    Some(s)
                } else {
                    None
                }
            })
            .collect())
    }

    async fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>> {
        let sql = format!(
            "SELECT TABNAME, TYPE FROM SYSCAT.TABLES \
             WHERE TABSCHEMA = '{}' ORDER BY TABNAME",
            schema.replace('\'', "''")
        );
        let result = self.execute(&sql).await?;
        Ok(result
            .rows
            .into_iter()
            .filter_map(|r| {
                let name = r.get(0).and_then(|v| {
                    if let serde_json::Value::String(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })?;
                let tt = r
                    .get(1)
                    .and_then(|v| {
                        if let serde_json::Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                Some(TableInfo {
                    name,
                    // DB2 SYSCAT.TABLES.TYPE: 'T' = table, 'V' = view
                    table_type: if tt == "V" { "view".into() } else { "table".into() },
                })
            })
            .collect())
    }

    async fn get_table_schema(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let sql = format!(
            "SELECT COLNAME, TYPENAME, DEFAULT, NULLS \
             FROM SYSCAT.COLUMNS \
             WHERE TABSCHEMA = '{}' AND TABNAME = '{}' \
             ORDER BY COLNO",
            schema.replace('\'', "''"),
            table.replace('\'', "''")
        );
        let result = self.execute(&sql).await?;
        Ok(result
            .rows
            .into_iter()
            .map(|r| {
                let get_str = |i: usize| {
                    r.get(i)
                        .and_then(|v| {
                            if let serde_json::Value::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default()
                };
                let default_val = get_str(2);
                ColumnInfo {
                    name: get_str(0),
                    data_type: get_str(1),
                    nullable: get_str(3) == "Y",
                    default: if default_val.is_empty() { None } else { Some(default_val) },
                }
            })
            .collect())
    }

    async fn execute(&self, sql: &str) -> Result<QueryResult> {
        let cs = self.conn_string.clone();
        let u = self.username.clone();
        let p = self.password.clone();
        let sql_owned = sql.to_string();
        tokio::task::spawn_blocking(move || Self::execute_sync(&cs, &u, &p, &sql_owned))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking: {e}"))
    }
}

#[cfg(test)]
mod tests {
    // Integration tests require an IBM DB2 instance with the IBM ODBC driver installed.
    // Run with: cargo test db::db2 -- --ignored
    // (No #[ignore] tests exist yet; this block is a placeholder.)
}
