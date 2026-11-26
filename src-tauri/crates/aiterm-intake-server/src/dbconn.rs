//! server 上的資料庫連線：設定存 intake.db、密碼存加密金鑰檔、連線池依連線快取。
//! 連線由管理員從 AITerm 分享上來；支援 PostgreSQL、MySQL、SQLite（SQL Server、ODBC、DB2 只在桌面版）。

use std::sync::Arc;
use std::time::Duration;

use aiterm_db::adapter::DbAdapter;
use aiterm_db::connect::SUPPORTED_TYPES;
use aiterm_intake_core::board_store::{self as bs, BoardDbConn, NewDbConn};
use aiterm_intake_core::host::Secrets;

use crate::state::AppState;

/// 連線（含認證）最久等幾秒；連不上不能讓 Agent 的工具呼叫一直卡著。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

pub fn secret_key(id: &str) -> String {
    format!("dbconn:{id}")
}

/// 管理員分享上來的連線設定。`password` 是 `None` 表示沿用已存的（同名取代時）。
pub struct ConnInput<'a> {
    pub name: &'a str,
    pub db_type: &'a str,
    pub host: &'a str,
    pub port: i64,
    pub database: &'a str,
    pub username: &'a str,
    pub default_schema: Option<&'a str>,
    pub password: Option<&'a str>,
}

/// 檢查輸入；回傳不合法的原因。
pub fn validate(c: &ConnInput<'_>) -> Result<(), String> {
    if c.name.trim().is_empty() || c.name.chars().count() > 80 || c.name.chars().any(|ch| ch.is_control()) {
        return Err("連線名稱不能是空的、超過 80 字或含控制字元".into());
    }
    if !SUPPORTED_TYPES.contains(&c.db_type) {
        return Err(format!("server 不支援這種資料庫：{}（支援 {}；SQL Server、ODBC、DB2 只在桌面版）", c.db_type, SUPPORTED_TYPES.join("、")));
    }
    if c.host.trim().is_empty() {
        return Err(if c.db_type == "sqlite" { "SQLite 要填 server 主機上的檔案路徑".into() } else { "主機不能是空的".into() });
    }
    if c.db_type != "sqlite" && !(1..=65535).contains(&c.port) {
        return Err("埠要在 1～65535".into());
    }
    if c.host.chars().any(|ch| ch.is_control()) || c.database.chars().any(|ch| ch.is_control()) || c.username.chars().any(|ch| ch.is_control()) {
        return Err("欄位不能含控制字元".into());
    }
    Ok(())
}

/// 新增或取代一個連線（同名保留 id）。連線池會被丟掉，下次使用才用新設定重連。
pub async fn upsert(st: &AppState, c: &ConnInput<'_>, by: &str) -> Result<BoardDbConn, String> {
    validate(c)?;
    let conn = bs::upsert_db_conn(
        &st.pools.intake,
        NewDbConn { name: c.name.trim(), db_type: c.db_type, host: c.host.trim(), port: c.port, database: c.database, username: c.username, default_schema: c.default_schema.filter(|s| !s.is_empty()), created_by: by },
    )
    .await
    .map_err(|e| e.to_string())?;
    if let Some(pw) = c.password {
        st.secrets.set(&secret_key(&conn.id), pw)?;
    }
    invalidate(st, &conn.id).await;
    Ok(conn)
}

/// 刪除連線、密碼與連線池。
pub async fn remove(st: &AppState, id: &str) -> Result<bool, String> {
    let gone = bs::delete_db_conn(&st.pools.intake, id).await.map_err(|e| e.to_string())?;
    let _ = st.secrets.delete(&secret_key(id));
    invalidate(st, id).await;
    Ok(gone)
}

pub async fn invalidate(st: &AppState, id: &str) {
    st.db_pools.lock().await.remove(id);
}

/// 這個連線的適配器：有快取用快取，沒有才連線。找不到連線或連不上回說明。
pub async fn adapter_for(st: &AppState, id: &str) -> Result<Arc<dyn DbAdapter>, String> {
    if let Some(a) = st.db_pools.lock().await.get(id) {
        return Ok(a.clone());
    }
    let conn = bs::get_db_conn(&st.pools.intake, id).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("找不到這個資料庫連線：{id}"))?;
    let password = st.secrets.get(&secret_key(id)).unwrap_or_default();
    let built = tokio::time::timeout(CONNECT_TIMEOUT, aiterm_db::connect::connect(&conn.db_type, &conn.host, conn.port as u16, &conn.database, &conn.username, &password))
        .await
        .map_err(|_| format!("連線逾時（超過 {} 秒）：{}", CONNECT_TIMEOUT.as_secs(), conn.name))?
        .map_err(|e| format!("連不上 {}：{e}", conn.name))?;
    let adapter: Arc<dyn DbAdapter> = Arc::from(built);
    st.db_pools.lock().await.insert(id.to_string(), adapter.clone());
    Ok(adapter)
}

/// 實際連一次並 ping（管理員在分享後按「測試」）。不使用快取，才測得到最新的設定與密碼。
pub async fn test(st: &AppState, id: &str) -> Result<(), String> {
    invalidate(st, id).await;
    let a = adapter_for(st, id).await?;
    tokio::time::timeout(CONNECT_TIMEOUT, a.test()).await.map_err(|_| "測試逾時".to_string())?.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(db_type: &'a str, host: &'a str, port: i64) -> ConnInput<'a> {
        ConnInput { name: "正式機", db_type, host, port, database: "app", username: "u", default_schema: None, password: Some("p@ss") }
    }

    #[test]
    fn only_supported_types_with_sane_fields_pass() {
        assert!(validate(&input("postgresql", "10.0.0.1", 5432)).is_ok());
        assert!(validate(&input("sqlite", "/data/app.db", 0)).is_ok());
        assert!(validate(&input("mssql", "h", 1433)).unwrap_err().contains("不支援"));
        assert!(validate(&input("postgresql", "", 5432)).unwrap_err().contains("主機"));
        assert!(validate(&input("sqlite", "", 0)).unwrap_err().contains("檔案路徑"));
        assert!(validate(&input("mysql", "h", 0)).unwrap_err().contains("埠"));
        assert!(validate(&ConnInput { name: "  ", ..input("mysql", "h", 3306) }).is_err());
        assert!(validate(&ConnInput { host: "a\nb", ..input("mysql", "h", 3306) }).is_err());
    }
}
