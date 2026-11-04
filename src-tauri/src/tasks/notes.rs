//! 專案記事本：各張卡留下「別人也該知道」的事（慣例、踩過的坑、已定的決定），
//! 之後每張卡開始時都會帶進去。平行的 Agent 彼此看不到對方的對話，這是它們
//! 唯一共用的長期記憶，補足單一 session 才有的「記得之前發生的事」。

use serde::Serialize;
use sqlx::SqlitePool;

/// 一則筆記的字數上限；要短，才會每張卡都帶得動。
pub const NOTE_MAX_CHARS: usize = 500;
/// 放進提示時的上限（字元）。超過時留最新的，舊的丟掉。
pub const NOTES_PROMPT_LIMIT: usize = 4000;
/// 資料庫裡最多留幾則；超過就清掉最舊的，不然只有注入時被截斷、列會永遠增加。
pub const MAX_NOTES: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct NoteRow {
    pub id: i64,
    pub author: String,
    pub text: String,
    pub created_at: String,
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS project_notes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            author TEXT NOT NULL,
            text TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 多行壓成一行（筆記是一行一則）；空白與過長都擋下並說明原因。
fn clean(text: &str) -> Result<String, String> {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return Err("筆記不能是空的".into());
    }
    if one_line.chars().count() > NOTE_MAX_CHARS {
        return Err(format!("筆記太長（上限 {NOTE_MAX_CHARS} 字），請濃縮成一兩句"));
    }
    Ok(one_line)
}

/// 新增一則。跟既有的某則字句完全相同就不重複記（回傳那一則）。
pub async fn add_note(pool: &SqlitePool, author: &str, text: &str) -> Result<NoteRow, String> {
    let text = clean(text)?;
    if let Some(existing) = sqlx::query_as::<_, NoteRow>("SELECT * FROM project_notes WHERE text = ? ORDER BY id LIMIT 1")
        .bind(&text)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(existing);
    }
    let id = sqlx::query("INSERT INTO project_notes (author, text) VALUES (?, ?)")
        .bind(author.trim())
        .bind(&text)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .last_insert_rowid();
    // 只在真的新增之後清（重複的在上面就回傳了）；id 遞增，所以最大的 MAX_NOTES 個就是最新的。
    sqlx::query("DELETE FROM project_notes WHERE id NOT IN (SELECT id FROM project_notes ORDER BY id DESC LIMIT ?)")
        .bind(MAX_NOTES as i64)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query_as::<_, NoteRow>("SELECT * FROM project_notes WHERE id = ?").bind(id).fetch_one(pool).await.map_err(|e| e.to_string())
}

/// 舊的在前、新的在後。
pub async fn list_notes(pool: &SqlitePool) -> Result<Vec<NoteRow>, sqlx::Error> {
    sqlx::query_as::<_, NoteRow>("SELECT * FROM project_notes ORDER BY id").fetch_all(pool).await
}

pub async fn delete_note(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM project_notes WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

/// 放進卡片提示的那一段；沒有筆記是空字串。太多時留最新的。
pub fn notes_section(notes: &[NoteRow]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = notes
        .iter()
        .map(|n| if n.author.is_empty() { format!("- {}", n.text) } else { format!("- （{}）{}", n.author, n.text) })
        .collect();
    let mut kept: Vec<&String> = Vec::new();
    let mut used = 0usize;
    for line in lines.iter().rev() {
        let len = line.chars().count() + 1;
        if used + len > NOTES_PROMPT_LIMIT && !kept.is_empty() {
            break;
        }
        used += len;
        kept.push(line);
    }
    kept.reverse();
    let dropped = lines.len() - kept.len();
    let mut s = String::from("### 專案記事本\n（先前的卡片留下的，可能有用；跟這張卡的要求衝突時以這張卡為準。）\n");
    if dropped > 0 {
        s.push_str(&format!("（更早的 {dropped} 則已省略）\n"));
    }
    for line in kept {
        s.push_str(line);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn notes_round_trip_in_order_and_can_be_deleted() {
        let pool = pool().await;
        let a = add_note(&pool, "工程師1", "計時器一律用 performance.now()").await.unwrap();
        add_note(&pool, "工程師2", "樣式放 src/styles").await.unwrap();
        let all = list_notes(&pool).await.unwrap();
        assert_eq!(all.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["計時器一律用 performance.now()", "樣式放 src/styles"]);
        assert_eq!(all[0].author, "工程師1");
        delete_note(&pool, a.id).await.unwrap();
        assert_eq!(list_notes(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn multiline_notes_become_one_line_and_blank_or_huge_are_refused() {
        let pool = pool().await;
        let n = add_note(&pool, "a", "第一行\n  第二行\t第三行").await.unwrap();
        assert_eq!(n.text, "第一行 第二行 第三行");
        assert!(add_note(&pool, "a", "  \n ").await.is_err());
        let huge = "字".repeat(NOTE_MAX_CHARS + 1);
        assert!(add_note(&pool, "a", &huge).await.unwrap_err().contains("太長"));
        assert!(add_note(&pool, "a", &"字".repeat(NOTE_MAX_CHARS)).await.is_ok());
    }

    #[tokio::test]
    async fn an_identical_note_is_not_stored_twice() {
        let pool = pool().await;
        let first = add_note(&pool, "工程師1", "用 pnpm").await.unwrap();
        let again = add_note(&pool, "工程師2", "用  pnpm").await.unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(list_notes(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_51st_note_pushes_out_the_oldest_and_keeps_max_notes() {
        let pool = pool().await;
        for i in 1..=MAX_NOTES + 1 {
            add_note(&pool, "a", &format!("第 {i} 則")).await.unwrap();
        }
        let all = list_notes(&pool).await.unwrap();
        assert_eq!(all.len(), MAX_NOTES);
        assert!(!all.iter().any(|n| n.text == "第 1 則"), "最舊的要被清掉");
        assert_eq!(all[0].text, "第 2 則", "順序仍是舊的在前");
        assert_eq!(all.last().unwrap().text, format!("第 {} 則", MAX_NOTES + 1));
    }

    #[tokio::test]
    async fn a_duplicate_does_not_count_as_new_and_deletes_nothing() {
        let pool = pool().await;
        for i in 1..=MAX_NOTES {
            add_note(&pool, "a", &format!("第 {i} 則")).await.unwrap();
        }
        // 已滿 50 則時再寫一則重複的：不是新增，最舊那則不能被誤刪。
        add_note(&pool, "b", "第 7 則").await.unwrap();
        let all = list_notes(&pool).await.unwrap();
        assert_eq!(all.len(), MAX_NOTES);
        assert_eq!(all[0].text, "第 1 則");
    }

    fn note(id: i64, text: &str) -> NoteRow {
        NoteRow { id, author: "工程師1".into(), text: text.into(), created_at: String::new() }
    }

    #[test]
    fn no_notes_means_no_section() {
        assert_eq!(notes_section(&[]), "");
    }

    #[test]
    fn the_section_lists_notes_with_their_author() {
        let s = notes_section(&[note(1, "用 pnpm"), NoteRow { author: String::new(), ..note(2, "別動 legacy/") }]);
        assert!(s.starts_with("### 專案記事本"));
        assert!(s.contains("- （工程師1）用 pnpm\n"));
        assert!(s.contains("- 別動 legacy/\n"));
    }

    #[test]
    fn when_too_long_the_newest_notes_are_kept_and_the_count_of_dropped_is_said() {
        // 每則 ~400 字，12 則遠超上限；留下的必須是最後幾則。
        let notes: Vec<NoteRow> = (1..=12).map(|i| note(i, &format!("{i:02}{}", "字".repeat(400)))).collect();
        let s = notes_section(&notes);
        assert!(s.contains("12字"), "最新的一則必須在");
        assert!(!s.contains("01字"), "最舊的要被丟掉");
        assert!(s.contains("更早的") && s.contains("則已省略"));
        assert!(s.chars().count() < NOTES_PROMPT_LIMIT + 400, "總長要受限");
    }

    #[test]
    fn a_single_oversized_newest_note_is_still_kept() {
        let s = notes_section(&[note(1, "舊"), note(2, &"字".repeat(NOTE_MAX_CHARS))]);
        assert!(s.contains(&"字".repeat(NOTE_MAX_CHARS)));
    }
}
