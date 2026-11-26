//! 依連線設定建適配器（PostgreSQL／MySQL／SQLite），以及把查詢結果整理成給 Agent 的文字。
//! SQL Server、ODBC、DB2 的適配器只在桌面版，這裡回「不支援」。

use anyhow::{anyhow, Result};

use crate::adapter::{DbAdapter, QueryResult};
use crate::mysql::MySqlAdapter;
use crate::postgres::PostgresAdapter;
use crate::sqlite::SqliteAdapter;

/// 給 Agent 的查詢結果最多幾個位元組（超過會截斷並註明）。
pub const MAX_RESULT_BYTES: usize = 100 * 1024;

/// server 端支援的資料庫類型（小寫，與桌面版設定檔的 `db_type` 同寫法）。
pub const SUPPORTED_TYPES: [&str; 3] = ["postgresql", "mysql", "sqlite"];

/// URL 的使用者名稱／密碼／資料庫名稱要百分比編碼（`@`、`:`、`/` 之類會破壞連線字串）。
pub fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

/// 建立連線並回傳適配器。SQLite 的 `host` 是檔案路徑。
pub async fn connect(db_type: &str, host: &str, port: u16, database: &str, username: &str, password: &str) -> Result<Box<dyn DbAdapter>> {
    match db_type {
        "postgresql" => {
            let url = format!("postgresql://{}:{}@{}:{}/{}", pct_encode(username), pct_encode(password), host, port, pct_encode(database));
            Ok(Box::new(PostgresAdapter::connect(&url).await?))
        }
        "mysql" => {
            let url = format!("mysql://{}:{}@{}:{}/{}", pct_encode(username), pct_encode(password), host, port, pct_encode(database));
            Ok(Box::new(MySqlAdapter::connect(&url).await?))
        }
        "sqlite" => Ok(Box::new(SqliteAdapter::connect(host).await?)),
        other => Err(anyhow!("server 不支援這種資料庫：{other}（支援 {}）", SUPPORTED_TYPES.join("、"))),
    }
}

fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 把查詢結果整理成文字（JSON）。DB 層的錯誤回 `Err`；太大就截斷。回傳（文字, 列數）；
/// 寫入沒有回傳列，列數改用受影響的列數。
pub fn format_result(result: &QueryResult) -> Result<(String, usize), String> {
    if let Some(e) = &result.error {
        return Err(e.clone());
    }
    let full = serde_json::to_string_pretty(result).map_err(|e| e.to_string())?;
    let rows = if result.rows.is_empty() { result.affected_rows.map_or(0, |n| n as usize) } else { result.rows.len() };
    if full.len() > MAX_RESULT_BYTES {
        Ok((format!("{}\n\n[TRUNCATED: result exceeds {} bytes; {} rows total]", safe_truncate(&full, MAX_RESULT_BYTES), MAX_RESULT_BYTES, result.rows.len()), rows))
    } else {
        Ok((full, rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_characters_are_percent_encoded() {
        assert_eq!(pct_encode("p@ss:w/rd"), "p%40ss%3Aw%2Frd");
        assert_eq!(pct_encode("abc-_.~123"), "abc-_.~123");
    }

    #[tokio::test]
    async fn sqlite_connects_and_unsupported_or_bad_targets_are_refused() {
        let db = connect("sqlite", ":memory:", 0, "", "", "").await.unwrap();
        db.test().await.unwrap();
        for t in ["mssql", "db2", "oracle"] {
            let e = connect(t, "h", 1, "d", "u", "p").await.err().expect("要拒絕").to_string();
            assert!(e.contains("不支援") && e.contains(t), "{e}");
        }
        // 壞的主機名稱：連線字串解析不過，立刻回錯誤，不是 panic。
        assert!(connect("postgresql", "bad host", 1, "d", "u", "p").await.is_err());
    }

    #[tokio::test]
    async fn results_are_formatted_with_row_counts_errors_and_truncation() {
        let db = connect("sqlite", ":memory:", 0, "", "", "").await.unwrap();
        db.execute("CREATE TABLE t (id INTEGER, s TEXT)").await.unwrap();
        let ins = db.execute("INSERT INTO t VALUES (1, 'a')").await.unwrap();
        assert_eq!(format_result(&ins).unwrap().1, 1, "寫入用受影響的列數");
        let sel = db.execute("SELECT * FROM t").await.unwrap();
        let (text, n) = format_result(&sel).unwrap();
        assert!(text.contains("\"a\"") && n == 1, "{text}");
        let bad = db.execute("SELECT * FROM nope").await.unwrap();
        assert!(format_result(&bad).unwrap_err().to_lowercase().contains("no such table"));
        let big = db.execute(&format!("SELECT '{}' AS huge", "x".repeat(MAX_RESULT_BYTES + 1000))).await.unwrap();
        let (text, _) = format_result(&big).unwrap();
        assert!(text.contains("TRUNCATED") && text.len() < MAX_RESULT_BYTES + 500, "{}", text.len());
    }
}
