//! 專案的參考資料夾：整個團隊都能讀、不能寫的資料夾（例如舊系統原始碼與它的文件）。
//! 派工時 Claude 卡加 `--add-dir`（讀那裡不會停下來等權限）並在設定檔禁止編輯；
//! Codex 卡也加 `--add-dir`；兩者的提示都附上清單與「只能讀」。
//! 存在專案自己的 tasks.db（`project_settings`）。

use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

const KEY: &str = "reference_dirs";
/// 最多幾個參考資料夾。
pub const MAX_REFERENCES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// 絕對路徑。
    pub path: String,
    /// 一句說明（會寫進派工提示），例如「舊系統原始碼與 ArcDocs」。
    #[serde(default)]
    pub note: String,
}

pub async fn get(pool: &SqlitePool) -> Result<Vec<Reference>, sqlx::Error> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM project_settings WHERE key = ?").bind(KEY).fetch_optional(pool).await?;
    Ok(raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default())
}

/// 整理使用者給的清單：去頭尾空白、去重複、必須是存在的資料夾（絕對路徑）。回傳整理後的清單或錯誤訊息。
pub fn normalize(refs: Vec<Reference>) -> Result<Vec<Reference>, String> {
    let mut out: Vec<Reference> = Vec::new();
    for r in refs {
        let path = r.path.trim().to_string();
        if path.is_empty() || out.iter().any(|o| o.path == path) {
            continue;
        }
        let p = Path::new(&path);
        if !p.is_absolute() {
            return Err(format!("參考資料夾要是完整路徑：{path}"));
        }
        if !p.is_dir() {
            return Err(format!("找不到這個資料夾：{path}"));
        }
        out.push(Reference { path, note: r.note.trim().to_string() });
    }
    if out.len() > MAX_REFERENCES {
        return Err(format!("參考資料夾最多 {MAX_REFERENCES} 個"));
    }
    Ok(out)
}

pub async fn set(pool: &SqlitePool, refs: &[Reference]) -> Result<(), sqlx::Error> {
    if refs.is_empty() {
        sqlx::query("DELETE FROM project_settings WHERE key = ?").bind(KEY).execute(pool).await?;
    } else {
        sqlx::query("INSERT INTO project_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(KEY)
            .bind(serde_json::to_string(refs).unwrap_or_default())
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// 附在卡片提示後面的一段。
pub fn prompt_note(refs: &[Reference]) -> String {
    if refs.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n參考資料（只能讀，不要在這些資料夾裡新增、修改或刪除任何檔案）：\n");
    for r in refs {
        if r.note.is_empty() {
            s.push_str(&format!("- {}\n", r.path));
        } else {
            s.push_str(&format!("- {}：{}\n", r.path, r.note));
        }
    }
    s.push_str("需要時直接讀；裡面若有 CLAUDE.md 或 README，先讀它了解怎麼查。");
    s
}

/// Claude 的 `--add-dir`（每個資料夾一個）。路徑有特殊字元、沒辦法安全帶進指令的就略過。
pub fn add_dir_args(refs: &[Reference]) -> Option<String> {
    let flags: Vec<String> = refs.iter().filter_map(|r| crate::tasks::quota::path_flag("--add-dir", Path::new(&r.path))).collect();
    (!flags.is_empty()).then(|| flags.join(" "))
}

/// Claude 權限規則：禁止用內建工具編輯參考資料夾裡的檔案。絕對路徑在規則裡寫成 `//` 開頭；
/// Windows 的 `C:\a\b` 寫成 `//C:/a/b`。
pub fn deny_edit_rules(refs: &[Reference]) -> Vec<String> {
    refs.iter()
        .map(|r| {
            let p = r.path.replace('\\', "/");
            let p = p.trim_end_matches('/');
            format!("Edit(/{}{}/**)", if p.starts_with('/') { "" } else { "/" }, p)
        })
        .collect()
}

/// 在派工設定檔（JSON）加上 `permissions.deny`，保留原本的內容。
pub fn settings_with_deny(settings_json: &str, rules: &[String]) -> String {
    if rules.is_empty() {
        return settings_json.to_string();
    }
    let mut v: serde_json::Value = serde_json::from_str(settings_json).unwrap_or_else(|_| serde_json::json!({}));
    let deny = v["permissions"]["deny"].as_array().cloned().unwrap_or_default();
    let mut all: Vec<serde_json::Value> = deny;
    all.extend(rules.iter().map(|r| serde_json::Value::from(r.as_str())));
    v["permissions"]["deny"] = serde_json::Value::Array(all);
    v.to_string()
}

/// 讀了參考資料的記錄：每張卡、每個資料夾一筆，讀了幾次、第一次與最後一次的時間。
/// 畫面用最後一次的時間決定要不要畫「讀取中」的線，活動記錄每張卡一行不洗版。
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct ReferenceReadRow {
    pub task_id: String,
    pub agent_id: Option<String>,
    /// 參考資料夾（跟清單裡的 `Reference::path` 一字不差）。
    pub ref_path: String,
    /// 讀取次數（Read 一個檔、Grep／Glob 搜一次各算一次）。
    pub reads: i64,
    pub first_at: i64,
    pub last_at: i64,
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS reference_reads (
            task_id  TEXT NOT NULL,
            agent_id TEXT,
            ref_path TEXT NOT NULL,
            reads    INTEGER NOT NULL,
            first_at INTEGER NOT NULL,
            last_at  INTEGER NOT NULL,
            PRIMARY KEY (task_id, ref_path)
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// PostToolUse 的工具呼叫讀的是不是某個參考資料夾裡的東西；是的話回傳那個資料夾。
/// 看得到路徑的只有 Read／Grep／Glob／LS／NotebookRead；Bash 指令裡的路徑不解析（猜錯比沒畫更糟）。
pub fn reference_of_access<'a>(refs: &'a [String], tool: &str, input: &serde_json::Value) -> Option<&'a String> {
    let key = match tool {
        "Read" => "file_path",
        "NotebookRead" => "notebook_path",
        "Grep" | "Glob" | "LS" => "path",
        _ => return None,
    };
    let target = Path::new(input.get(key)?.as_str()?);
    if !target.is_absolute() {
        return None;
    }
    refs.iter().find(|r| target.starts_with(Path::new(r.as_str())))
}

pub async fn record_read(pool: &SqlitePool, task_id: &str, agent_id: Option<&str>, ref_path: &str, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO reference_reads (task_id, agent_id, ref_path, reads, first_at, last_at) VALUES (?, ?, ?, 1, ?, ?)
         ON CONFLICT(task_id, ref_path) DO UPDATE SET reads = reads + 1, last_at = excluded.last_at",
    )
    .bind(task_id)
    .bind(agent_id)
    .bind(ref_path)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn recent_reads(pool: &SqlitePool, limit: i64) -> Result<Vec<ReferenceReadRow>, sqlx::Error> {
    sqlx::query_as::<_, ReferenceReadRow>("SELECT * FROM reference_reads ORDER BY last_at DESC LIMIT ?").bind(limit).fetch_all(pool).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(path: &str, note: &str) -> Reference {
        Reference { path: path.into(), note: note.into() }
    }

    #[test]
    fn normalize_trims_dedupes_and_rejects_bad_paths() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().to_string_lossy().into_owned();
        let out = normalize(vec![r(&format!("  {p}  "), " 舊系統 "), r(&p, "重複"), r("", "")]).unwrap();
        assert_eq!(out, vec![r(&p, "舊系統")]);
        assert!(normalize(vec![r("relative/dir", "")]).unwrap_err().contains("完整路徑"));
        assert!(normalize(vec![r(&format!("{p}/nope"), "")]).unwrap_err().contains("找不到"));
        let many: Vec<Reference> = (0..=MAX_REFERENCES)
            .map(|i| {
                let d = dir.path().join(format!("d{i}"));
                std::fs::create_dir(&d).unwrap();
                r(&d.to_string_lossy(), "")
            })
            .collect();
        assert!(normalize(many).unwrap_err().contains("最多"));
    }

    #[test]
    fn prompt_note_lists_folders_and_says_read_only() {
        assert_eq!(prompt_note(&[]), "");
        let note = prompt_note(&[r("/a/HTBTW_Source", "舊系統原始碼"), r("/b/docs", "")]);
        assert!(note.contains("- /a/HTBTW_Source：舊系統原始碼") && note.contains("- /b/docs\n"), "{note}");
        assert!(note.contains("只能讀"), "{note}");
    }

    #[test]
    fn add_dir_args_quote_paths_and_skip_unsafe_ones() {
        assert_eq!(add_dir_args(&[]), None);
        assert_eq!(
            add_dir_args(&[r("/a b/src", ""), r("/x$y", ""), r("/c", "")]).as_deref(),
            Some(r#"--add-dir "/a b/src" --add-dir "/c""#)
        );
    }

    #[test]
    fn deny_rules_use_absolute_rule_paths_on_every_platform() {
        assert_eq!(deny_edit_rules(&[r("/Users/j/HTBTW_Source/", "")]), vec!["Edit(//Users/j/HTBTW_Source/**)"]);
        assert_eq!(deny_edit_rules(&[r(r"C:\Github\HTBTW_Source", "")]), vec!["Edit(//C:/Github/HTBTW_Source/**)"]);
    }

    #[test]
    fn deny_rules_are_added_without_losing_hooks() {
        let base = r#"{"hooks":{"Stop":[]},"permissions":{"deny":["Bash(rm:*)"]}}"#;
        let v: serde_json::Value = serde_json::from_str(&settings_with_deny(base, &["Edit(//a/**)".into()])).unwrap();
        assert_eq!(v["permissions"]["deny"], serde_json::json!(["Bash(rm:*)", "Edit(//a/**)"]));
        assert!(v["hooks"]["Stop"].is_array());
        assert_eq!(settings_with_deny(base, &[]), base);
    }

    #[tokio::test]
    async fn references_round_trip_through_project_settings() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        assert!(get(&pool).await.unwrap().is_empty());
        set(&pool, &[r("/a", "x")]).await.unwrap();
        assert_eq!(get(&pool).await.unwrap(), vec![r("/a", "x")]);
        set(&pool, &[]).await.unwrap();
        assert!(get(&pool).await.unwrap().is_empty());
    }

    #[test]
    fn which_reference_a_tool_call_reads() {
        let refs = vec!["/old/HTBTW_Source".to_string(), "/docs".to_string()];
        let hit = |tool: &str, input: serde_json::Value| reference_of_access(&refs, tool, &input).cloned();
        assert_eq!(hit("Read", serde_json::json!({"file_path": "/old/HTBTW_Source/FR/FR1000.egl"})).as_deref(), Some("/old/HTBTW_Source"));
        assert_eq!(hit("Grep", serde_json::json!({"pattern": "x", "path": "/docs"})).as_deref(), Some("/docs"));
        assert_eq!(hit("Glob", serde_json::json!({"pattern": "**/*.egl", "path": "/old/HTBTW_Source/AC"})).as_deref(), Some("/old/HTBTW_Source"));
        // 路徑字首相同但是別的資料夾、相對路徑、沒有路徑的搜尋、Bash：都不算。
        assert_eq!(hit("Read", serde_json::json!({"file_path": "/old/HTBTW_Source2/a.egl"})), None);
        assert_eq!(hit("Read", serde_json::json!({"file_path": "FR/FR1000.egl"})), None);
        assert_eq!(hit("Grep", serde_json::json!({"pattern": "x"})), None);
        assert_eq!(hit("Bash", serde_json::json!({"command": "cat /docs/a.md"})), None);
        assert_eq!(hit("Edit", serde_json::json!({"file_path": "/docs/a.md"})), None);
    }

    #[tokio::test]
    async fn reads_aggregate_per_card_and_folder() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        record_read(&pool, "t1", Some("a1"), "/docs", 100).await.unwrap();
        record_read(&pool, "t1", Some("a1"), "/docs", 130).await.unwrap();
        record_read(&pool, "t2", Some("a2"), "/docs", 120).await.unwrap();
        let rows = recent_reads(&pool, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].task_id.as_str(), rows[0].reads, rows[0].first_at, rows[0].last_at), ("t1", 2, 100, 130));
        assert_eq!((rows[1].task_id.as_str(), rows[1].reads), ("t2", 1));
    }
}
