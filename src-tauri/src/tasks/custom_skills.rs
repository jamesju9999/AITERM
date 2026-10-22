//! Agent 的自訂技能：把指定的 SKILL.md 交給某位 Agent，精簡啟動照樣保留。
//!
//! 跟 Claude Code 原生技能一樣先只給清單：派工提示只附名稱、一句用途與路徑，
//! Agent 需要時才去讀全文（技能資料夾用 `--add-dir` 交給它）。
//! 讀了哪個技能從 PostToolUse hook 看得到（Read 的 file_path 是那個 SKILL.md），
//! 記在 `skill_uses`，畫面上技能列會亮、活動記錄會記一行。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

pub const SKILL_FILE: &str = "SKILL.md";
/// 一位 Agent 最多幾個自訂技能。
pub const MAX_SKILLS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    /// 技能資料夾（含 SKILL.md）的絕對路徑。
    pub path: String,
    /// `repo`（程式庫的 .claude/skills）、`user`（~/.claude/skills）、`folder`（使用者自己選的）。
    pub source: String,
}

/// 讀 SKILL.md 開頭的 frontmatter（`---` 之間）裡的 name 與 description。沒有就用資料夾名稱與空說明。
/// description 支援 YAML 的多行寫法（`>`、`>-`、`|`、`|-` 後面接縮排的行，接成一行）。
pub fn parse_frontmatter(text: &str, fallback_name: &str) -> (String, String) {
    let mut name = None;
    let mut description: Option<String> = None;
    let mut lines = text.trim_start_matches('\u{feff}').lines().peekable();
    if lines.next().map(str::trim) == Some("---") {
        let unquote = |v: &str| v.trim().trim_matches('"').trim_matches('\'').to_string();
        while let Some(raw) = lines.next() {
            let line = raw.trim();
            if line == "---" {
                break;
            }
            if let Some(v) = line.strip_prefix("name:") {
                name = Some(unquote(v));
            } else if let Some(v) = line.strip_prefix("description:") {
                let v = v.trim();
                if matches!(v, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
                    let mut parts = Vec::new();
                    while let Some(next) = lines.peek() {
                        if next.starts_with(' ') || next.starts_with('\t') || next.trim().is_empty() && !next.is_empty() {
                            parts.push(next.trim().to_string());
                            lines.next();
                        } else {
                            break;
                        }
                    }
                    description = Some(parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" "));
                } else {
                    description = Some(unquote(v));
                }
            }
        }
    }
    (name.filter(|n| !n.is_empty()).unwrap_or_else(|| fallback_name.to_string()), description.unwrap_or_default())
}

/// 讀一個技能資料夾。沒有 SKILL.md 就是 `None`。
pub fn read_skill(dir: &Path, source: &str) -> Option<SkillInfo> {
    let text = std::fs::read_to_string(dir.join(SKILL_FILE)).ok()?;
    let folder = dir.file_name()?.to_string_lossy().into_owned();
    let (name, description) = parse_frontmatter(&text, &folder);
    Some(SkillInfo { name, description, path: dir.to_string_lossy().into_owned(), source: source.into() })
}

/// 一個 skills 資料夾底下所有的技能（依名稱排序）。
fn scan(root: &Path, source: &str) -> Vec<SkillInfo> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<SkillInfo> = entries.flatten().filter(|e| e.path().is_dir()).filter_map(|e| read_skill(&e.path(), source)).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 使用者選的資料夾裡的技能：它本身是技能、它底下的技能，或它的 `.claude/skills` 裡的技能。
/// macOS 的資料夾選擇器預設看不到 `.claude` 這種隱藏資料夾，所以選上層資料夾也要找得到。
pub fn find_in(dir: &Path) -> Vec<SkillInfo> {
    if let Some(one) = read_skill(dir, "folder") {
        return vec![one];
    }
    let below = scan(dir, "folder");
    if !below.is_empty() {
        return below;
    }
    scan(&dir.join(".claude").join("skills"), "folder")
}

/// 可以選的技能：程式庫的 `.claude/skills`、使用者的 `~/.claude/skills`。
pub fn discover(repo_dir: Option<&Path>, home: Option<&Path>) -> Vec<SkillInfo> {
    let mut out = Vec::new();
    if let Some(repo) = repo_dir {
        out.extend(scan(&repo.join(".claude").join("skills"), "repo"));
    }
    if let Some(home) = home {
        out.extend(scan(&home.join(".claude").join("skills"), "user"));
    }
    out
}

/// `agents.custom_skills` 的 JSON（技能資料夾路徑清單）。壞掉就當作沒有。
pub fn parse_paths(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|r| serde_json::from_str::<Vec<String>>(r).ok()).unwrap_or_default()
}

/// 派工時這位 Agent 的技能（讀不到 SKILL.md 的略過——資料夾可能被搬走了）。
pub fn load(paths: &[String]) -> Vec<SkillInfo> {
    paths.iter().filter_map(|p| read_skill(Path::new(p), "agent")).collect()
}

/// 已存的技能路徑裡，讀不到 SKILL.md 的（資料夾被搬走、改名或刪了）。
pub fn missing(paths: &[String]) -> Vec<String> {
    paths.iter().filter(|p| !Path::new(p).join(SKILL_FILE).is_file()).cloned().collect()
}

/// 附在卡片提示後面的一段：只有清單，需要時才讀全文。
pub fn prompt_note(skills: &[SkillInfo]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n你有這些技能可以用。需要時先用 Read 讀那個 SKILL.md 的全文並照做（不需要就不用讀）：\n");
    for k in skills {
        let file = Path::new(&k.path).join(SKILL_FILE);
        if k.description.is_empty() {
            s.push_str(&format!("- {}：{}\n", k.name, file.display()));
        } else {
            s.push_str(&format!("- {}：{}（{}）\n", k.name, k.description, file.display()));
        }
    }
    s.push_str("技能資料夾只能讀，不要修改裡面的檔案。");
    s
}

/// 讀的檔案是不是某個技能的 SKILL.md；是的話回傳技能名稱。
pub fn skill_of_read(skills: &[(String, PathBuf)], file_path: &str) -> Option<String> {
    let read = Path::new(file_path);
    skills.iter().find(|(_, dir)| read == dir.join(SKILL_FILE)).map(|(name, _)| name.clone())
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let _ = sqlx::query("ALTER TABLE agents ADD COLUMN custom_skills TEXT").execute(pool).await;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS skill_uses (
            id       INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id  TEXT NOT NULL,
            agent_id TEXT,
            skill    TEXT NOT NULL,
            at       INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_agent_skills(pool: &SqlitePool, agent_id: &str, paths: &[String]) -> Result<(), sqlx::Error> {
    let raw = (!paths.is_empty()).then(|| serde_json::to_string(paths).unwrap_or_default());
    sqlx::query("UPDATE agents SET custom_skills = ? WHERE id = ?").bind(raw).bind(agent_id).execute(pool).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct SkillUseRow {
    pub id: i64,
    pub task_id: String,
    pub agent_id: Option<String>,
    pub skill: String,
    pub at: i64,
}

pub async fn record_use(pool: &SqlitePool, task_id: &str, agent_id: Option<&str>, skill: &str) -> Result<(), sqlx::Error> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    sqlx::query("INSERT INTO skill_uses (task_id, agent_id, skill, at) VALUES (?, ?, ?, ?)")
        .bind(task_id)
        .bind(agent_id)
        .bind(skill)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn recent_uses(pool: &SqlitePool, limit: i64) -> Result<Vec<SkillUseRow>, sqlx::Error> {
    sqlx::query_as::<_, SkillUseRow>("SELECT * FROM skill_uses ORDER BY at DESC, id DESC LIMIT ?").bind(limit).fetch_all(pool).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(root: &Path, folder: &str, front: &str) -> PathBuf {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SKILL_FILE), front).unwrap();
        dir
    }

    #[test]
    fn missing_lists_only_paths_without_a_skill_file() {
        let root = tempfile::tempdir().unwrap();
        let ok = make_skill(root.path(), "htbtw-db", "---\nname: htbtw-db\n---\n");
        let moved = root.path().join("old-place").join("htbtw-egl");
        let paths = vec![ok.to_string_lossy().into_owned(), moved.to_string_lossy().into_owned()];
        assert_eq!(missing(&paths), vec![moved.to_string_lossy().into_owned()]);
    }

    #[test]
    fn frontmatter_name_and_description_with_fallbacks() {
        let text = "---\nname: htbtw-egl\ndescription: \"讀寫 MS950 檔案\"\ndisable-model-invocation: true\n---\n# 內文";
        assert_eq!(parse_frontmatter(text, "x"), ("htbtw-egl".into(), "讀寫 MS950 檔案".into()));
        assert_eq!(parse_frontmatter("# 沒有 frontmatter", "folder"), ("folder".into(), String::new()));
        assert_eq!(parse_frontmatter("\u{feff}---\ndescription: 'd'\n---", "f"), ("f".into(), "d".into()));
    }

    #[test]
    fn folded_and_literal_yaml_descriptions_are_joined() {
        let folded = "---\nname: use-spark\ndescription: >-\n  Use the Spark MCP\n  extension.\nother: x\n---";
        assert_eq!(parse_frontmatter(folded, "f").1, "Use the Spark MCP extension.");
        let literal = "---\ndescription: |\n  第一行\n  第二行\n---";
        assert_eq!(parse_frontmatter(literal, "f").1, "第一行 第二行");
    }

    /// macOS 的資料夾選擇器看不到 `.claude`：選上層資料夾也要找得到裡面的技能。
    #[test]
    fn finding_skills_under_any_chosen_folder() {
        let repo = tempfile::tempdir().unwrap();
        make_skill(&repo.path().join(".claude/skills"), "egl", "---\nname: egl\n---");
        make_skill(&repo.path().join(".claude/skills"), "db", "---\nname: db\n---");
        let names = |v: Vec<SkillInfo>| v.into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names(find_in(repo.path())), vec!["db", "egl"], "選程式庫根目錄 → .claude/skills 裡的");
        assert_eq!(names(find_in(&repo.path().join(".claude/skills"))), vec!["db", "egl"], "選 skills 資料夾 → 底下全部");
        assert_eq!(names(find_in(&repo.path().join(".claude/skills/egl"))), vec!["egl"], "選技能資料夾本身");
        let other = tempfile::tempdir().unwrap();
        assert!(find_in(other.path()).is_empty());
        assert!(find_in(&other.path().join("nope")).is_empty());
    }

    #[test]
    fn discover_lists_repo_then_user_skills_and_skips_folders_without_skill_md() {
        let repo = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        make_skill(&repo.path().join(".claude/skills"), "b-skill", "---\nname: b\n---");
        make_skill(&repo.path().join(".claude/skills"), "a-skill", "---\nname: a\n---");
        std::fs::create_dir_all(repo.path().join(".claude/skills/empty")).unwrap();
        make_skill(&home.path().join(".claude/skills"), "mine", "---\nname: mine\ndescription: 我的\n---");
        let found = discover(Some(repo.path()), Some(home.path()));
        let names: Vec<(&str, &str)> = found.iter().map(|s| (s.name.as_str(), s.source.as_str())).collect();
        assert_eq!(names, vec![("a", "repo"), ("b", "repo"), ("mine", "user")]);
        assert!(discover(Some(&repo.path().join("nope")), None).is_empty());
    }

    #[test]
    fn prompt_note_lists_skills_with_their_files_only() {
        assert_eq!(prompt_note(&[]), "");
        let k = SkillInfo { name: "htbtw-egl".into(), description: "讀寫 MS950".into(), path: "/s/htbtw-egl".into(), source: "repo".into() };
        let note = prompt_note(&[k]);
        assert!(note.contains("- htbtw-egl：讀寫 MS950（/s/htbtw-egl/SKILL.md）"), "{note}");
        assert!(note.contains("需要時") && note.contains("只能讀"), "{note}");
    }

    #[test]
    fn a_read_of_the_skill_file_is_recognised() {
        let skills = vec![("htbtw-egl".to_string(), PathBuf::from("/s/htbtw-egl"))];
        assert_eq!(skill_of_read(&skills, "/s/htbtw-egl/SKILL.md").as_deref(), Some("htbtw-egl"));
        assert_eq!(skill_of_read(&skills, "/s/htbtw-egl/helpers.ps1"), None, "讀輔助檔不算");
        assert_eq!(skill_of_read(&skills, "/other/SKILL.md"), None);
    }

    #[tokio::test]
    async fn skills_and_uses_round_trip() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        record_use(&pool, "t", Some("a1"), "htbtw-egl").await.unwrap();
        let uses = recent_uses(&pool, 10).await.unwrap();
        assert_eq!((uses[0].skill.as_str(), uses[0].agent_id.as_deref()), ("htbtw-egl", Some("a1")));
        assert_eq!(parse_paths(Some(r#"["/a","/b"]"#)), vec!["/a", "/b"]);
        assert!(parse_paths(Some("bad")).is_empty());
        let dir = tempfile::tempdir().unwrap();
        let k = make_skill(dir.path(), "k", "---\nname: k\n---");
        assert_eq!(load(&[k.to_string_lossy().into_owned(), "/gone".into()]).len(), 1, "讀不到的略過");
    }
}
