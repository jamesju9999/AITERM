//! Agent 資料庫技能的唯讀判定（見 `2026-10-19-agent-db-skill-design.md`）。
//!
//! 寧可錯擋：被擋的唯讀查詢只會讓 Agent 收到錯誤訊息改寫，錯放一句寫入的代價大得多。

/// 唯讀查詢可以用的開頭關鍵字。
const READ_STARTS: &[&str] = &["SELECT", "WITH", "SHOW", "DESCRIBE", "DESC", "EXPLAIN", "VALUES", "TABLE"];

/// 出現在任何位置都不算唯讀的關鍵字（寫入、DDL、權限、程序呼叫、連線狀態變更）。
/// `INTO` 擋 `SELECT … INTO new_table`。
const WRITE_WORDS: &[&str] = &[
    "INSERT", "UPDATE", "DELETE", "MERGE", "UPSERT", "REPLACE", "DROP", "ALTER", "CREATE", "TRUNCATE", "RENAME",
    "GRANT", "REVOKE", "EXEC", "EXECUTE", "CALL", "DO", "COPY", "LOAD", "ATTACH", "DETACH", "VACUUM", "REINDEX",
    "LOCK", "SET", "PRAGMA", "INTO", "COMMIT", "ROLLBACK", "BEGIN", "SAVEPOINT", "ANALYZE", "CLUSTER", "REFRESH",
    "IMPORT", "UNLOAD", "OPTIMIZE", "PREPARE", "DEALLOCATE", "LISTEN", "NOTIFY", "DISCARD", "RESET", "USE",
];

/// 去掉註解，字串常值與引號識別字換成空白（裡面的字不算關鍵字）。
/// 註解或字串沒有結束就回 `None`：判斷不了，當成不是唯讀。
fn strip(sql: &str) -> Option<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '-' && next == Some('-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            out.push(' ');
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            if i >= chars.len() {
                return None;
            }
            i += 2;
            out.push(' ');
        } else if c == '\'' || c == '"' || c == '`' {
            // 兩個連續的引號是跳脫，字串還沒結束。
            i += 1;
            loop {
                match chars.get(i) {
                    None => return None,
                    Some(&q) if q == c && chars.get(i + 1) == Some(&c) => i += 2,
                    Some(&q) if q == c => {
                        i += 1;
                        break;
                    }
                    Some(_) => i += 1,
                }
            }
            out.push(' ');
        } else if c == '$' {
            // PostgreSQL 的 $tag$ … $tag$ 字串。
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            if chars.get(j) == Some(&'$') {
                let tag: String = chars[i..=j].iter().collect();
                let rest: String = chars[j + 1..].iter().collect();
                match rest.find(&tag) {
                    Some(end) => i = j + 1 + rest[..end].chars().count() + tag.chars().count(),
                    None => return None,
                }
                out.push(' ');
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    Some(out)
}

/// 這句 SQL 是不是唯讀查詢。
pub fn is_read_only(sql: &str) -> bool {
    let Some(body) = strip(sql) else { return false };
    // 只能有一個敘述：第一個 `;` 之後只能剩空白（或更多 `;`）。
    let mut parts = body.splitn(2, ';');
    let first = parts.next().unwrap_or("");
    if parts.next().is_some_and(|rest| rest.chars().any(|c| !c.is_whitespace() && c != ';')) {
        return false;
    }
    let words: Vec<String> = first
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_uppercase)
        .collect();
    match words.first() {
        Some(w) if READ_STARTS.contains(&w.as_str()) => !words.iter().any(|w| WRITE_WORDS.contains(&w.as_str())),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_reads_pass() {
        for sql in [
            "SELECT * FROM orders",
            "  select id, name from users where id = 1;",
            "WITH t AS (SELECT 1 AS x) SELECT x FROM t",
            "SHOW TABLES",
            "DESCRIBE orders",
            "EXPLAIN SELECT * FROM orders",
            "SELECT 'DELETE FROM x' AS note",
            "SELECT \"update\" FROM t",
            "SELECT 1 -- DROP TABLE x\n",
            "SELECT /* INSERT */ 1",
            "SELECT $$ DROP TABLE x $$",
            "SELECT 'it''s; DELETE' FROM t",
            "SELECT updated_at, created_by FROM t",
        ] {
            assert!(is_read_only(sql), "應該是唯讀：{sql}");
        }
    }

    #[test]
    fn writes_ddl_and_tricks_are_blocked() {
        for sql in [
            "INSERT INTO t VALUES (1)",
            "update t set a = 1",
            "DELETE FROM t",
            "DROP TABLE t",
            "CREATE TABLE t (a int)",
            "TRUNCATE t",
            "SELECT 1; DELETE FROM t",
            "SELECT * INTO backup FROM t",
            "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
            "EXPLAIN ANALYZE DELETE FROM t",
            "EXEC sp_who",
            "CALL do_things()",
            "PRAGMA journal_mode = DELETE",
            "SET search_path = x",
            "",
            "   ",
            "-- 只有註解",
            "SELECT 1 /* 沒關的註解; DELETE FROM t",
            "SELECT 'unterminated",
            "SELECT $a$ DROP",
            "MERGE INTO t USING s ON 1=1 WHEN MATCHED THEN DELETE",
        ] {
            assert!(!is_read_only(sql), "應該擋下：{sql}");
        }
    }

    #[test]
    fn a_trailing_semicolon_is_fine() {
        assert!(is_read_only("SELECT 1;"));
        assert!(is_read_only("SELECT 1; ;  \n"));
    }
}
