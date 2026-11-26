//! 技能資料夾的共用規則：桌面版上傳前、server 收到時都用這一份，兩邊的上限才不會不一致。
//! 純邏輯（不碰檔案系統），好測。

/// 技能資料夾根目錄必須有的檔案。
pub const SKILL_FILE: &str = "SKILL.md";
/// 一個技能最多幾個檔、總大小、單檔大小。
pub const MAX_FILES: usize = 300;
pub const MAX_TOTAL_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

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

/// 上傳檔案的相對路徑：不能是空的、絕對路徑、含 `..` 或空段、反斜線、控制字元。回傳用 `/` 分隔的乾淨路徑。
pub fn clean_path(raw: &str) -> Result<String, String> {
    let bad = |why: &str| Err(format!("檔案路徑不合法（{why}）：{raw}"));
    if raw.is_empty() || raw.len() > 300 {
        return bad("空的或太長");
    }
    if raw.starts_with('/') || raw.contains('\\') || raw.chars().any(|c| c.is_control()) || raw.contains(':') {
        return bad("要用相對路徑、不能有反斜線或冒號");
    }
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return bad("不能含 .. 或空的段落");
    }
    Ok(parts.join("/"))
}

/// 這個路徑要不要略過（版本控制、相依套件、系統雜檔）。
pub fn skippable(path: &str) -> bool {
    path.split('/').any(|p| matches!(p, ".git" | "node_modules" | ".DS_Store" | "__pycache__"))
}

/// 檢查檔案清單（相對路徑與大小）：有根目錄的 SKILL.md、數量與大小上限、路徑不重複。
pub fn check_files(files: &[(String, usize)]) -> Result<(), String> {
    if files.is_empty() {
        return Err("資料夾是空的".into());
    }
    if !files.iter().any(|(p, _)| p == SKILL_FILE) {
        return Err(format!("資料夾根目錄要有 {SKILL_FILE}"));
    }
    if files.len() > MAX_FILES {
        return Err(format!("檔案太多：{} 個（上限 {MAX_FILES} 個）", files.len()));
    }
    let mut total = 0usize;
    let mut seen = std::collections::HashSet::new();
    for (p, n) in files {
        if !seen.insert(p.as_str()) {
            return Err(format!("檔案路徑重複：{p}"));
        }
        if *n > MAX_FILE_BYTES {
            return Err(format!("{p} 太大：{} KB（單檔上限 {} KB）", n / 1024, MAX_FILE_BYTES / 1024));
        }
        total += n;
    }
    if total > MAX_TOTAL_BYTES {
        return Err(format!("總共太大：{} KB（上限 {} KB）", total / 1024, MAX_TOTAL_BYTES / 1024));
    }
    Ok(())
}

/// 技能名稱要能當資料夾名稱用（副本路徑、提示詞）：不含路徑分隔與控制字元，長度合理。
pub fn safe_dir_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    let cleaned: String = cleaned.chars().take(60).collect();
    if cleaned.is_empty() { "skill".into() } else { cleaned }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_reads_name_and_description_including_multiline() {
        let t = "---\nname: 寫測試\ndescription: >\n  先寫會紅的測試，\n  再實作。\n---\n內文";
        assert_eq!(parse_frontmatter(t, "資料夾"), ("寫測試".into(), "先寫會紅的測試， 再實作。".into()));
        assert_eq!(parse_frontmatter("沒有 frontmatter", "資料夾"), ("資料夾".into(), String::new()));
    }

    #[test]
    fn paths_must_be_clean_relative_and_free_of_traversal() {
        assert_eq!(clean_path("a/b/c.md").unwrap(), "a/b/c.md");
        for bad in ["", "/etc/passwd", "../x", "a/../b", "a//b", "a/./b", "a\\b", "C:/x", "a/\u{0}b", "a/"] {
            assert!(clean_path(bad).is_err(), "{bad:?} 要被擋下");
        }
    }

    #[test]
    fn noise_directories_are_skipped_anywhere_in_the_path() {
        for p in [".git/config", "a/node_modules/x.js", ".DS_Store", "a/b/__pycache__/c.pyc"] {
            assert!(skippable(p), "{p}");
        }
        assert!(!skippable("scripts/run.sh"));
        assert!(!skippable("gitignore.md"));
    }

    fn f(p: &str, n: usize) -> (String, usize) {
        (p.to_string(), n)
    }

    #[test]
    fn limits_are_enforced_and_a_root_skill_file_is_required() {
        assert!(check_files(&[f("SKILL.md", 10), f("a/b.txt", 20)]).is_ok());
        assert!(check_files(&[]).unwrap_err().contains("空的"));
        assert!(check_files(&[f("sub/SKILL.md", 10)]).unwrap_err().contains("根目錄"));
        assert!(check_files(&[f("SKILL.md", MAX_FILE_BYTES + 1)]).unwrap_err().contains("單檔上限"));
        let many: Vec<_> = std::iter::once(f("SKILL.md", 1)).chain((0..MAX_FILES).map(|i| f(&format!("f{i}"), 1))).collect();
        assert!(check_files(&many).unwrap_err().contains("檔案太多"));
        let big: Vec<_> = std::iter::once(f("SKILL.md", 1)).chain((0..6).map(|i| f(&format!("f{i}"), MAX_FILE_BYTES))).collect();
        assert!(check_files(&big).unwrap_err().contains("總共太大"));
        assert!(check_files(&[f("SKILL.md", 1), f("SKILL.md", 1)]).unwrap_err().contains("重複"));
    }

    #[test]
    fn skill_names_become_safe_folder_names() {
        assert_eq!(safe_dir_name("寫測試"), "寫測試");
        assert_eq!(safe_dir_name("../../etc/passwd"), "_.._etc_passwd", "路徑分隔換成底線，頭尾的點拿掉，不會往上跳");
        assert_eq!(safe_dir_name("  ..  "), "skill");
        assert!(!safe_dir_name("a/b\\c").contains(['/', '\\']));
    }
}
