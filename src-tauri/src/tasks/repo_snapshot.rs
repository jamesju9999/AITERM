//! 規格卡派工當下的「程式庫現況」：實際掃出來的頂層項目、分支、未提交變更、獨立 git 專案。
//!
//! 實機（ARES GUI To Web）：規格寫「目標路徑目前為空」，其實那是使用者自己另一個 git 專案，
//! 結果整批成果合不回去。規格撰寫者只看得到使用者的文字簡介，不會自己去 `ls`，所以由派工時代為掃描。

use std::path::Path;
use std::process::Command;

/// 最多列幾個頂層項目（太多就只列前面、註明總數）。
const MAX_ENTRIES: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// 資料夾本身有 `.git`：另一個獨立的 git 專案（或子模組），內容不屬於這個程式庫的歷史。
    pub nested_git: bool,
}

fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(dir).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 掃描 `dir`。不是資料夾就回空字串（呼叫端不加任何東西）。
pub fn snapshot(dir: &Path) -> String {
    if !dir.is_dir() {
        return String::new();
    }
    let branch = git_out(dir, &["symbolic-ref", "--short", "HEAD"]);
    let dirty = git_out(dir, &["status", "--porcelain"]).map(|s| s.lines().count());
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        return None;
                    }
                    let path = e.path();
                    let is_dir = path.is_dir();
                    Some(Entry { nested_git: is_dir && path.join(".git").exists(), name, is_dir })
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    format_snapshot(&dir.to_string_lossy(), branch.as_deref(), dirty, &entries)
}

pub fn format_snapshot(dir: &str, branch: Option<&str>, dirty: Option<usize>, entries: &[Entry]) -> String {
    let mut s = String::from("## 程式庫現況（派工當下實際掃描，不是推測）\n");
    s.push_str(&format!("- 位置：{dir}\n"));
    if let Some(b) = branch {
        s.push_str(&format!("- 目前分支：{b}\n"));
    }
    if let Some(n) = dirty.filter(|n| *n > 0) {
        s.push_str(&format!("- 有 {n} 個未提交的變更\n"));
    }
    if entries.is_empty() {
        s.push_str("- 頂層項目：沒有（這是空的程式庫）\n");
    } else {
        s.push_str("- 頂層項目：\n");
        for e in entries.iter().take(MAX_ENTRIES) {
            let note = if e.nested_git { "（獨立 git 專案，不屬於這個程式庫的歷史）" } else { "" };
            s.push_str(&format!("  - {}{}{note}\n", e.name, if e.is_dir { "/" } else { "" }));
        }
        if entries.len() > MAX_ENTRIES {
            s.push_str(&format!("  - …還有 {} 項\n", entries.len() - MAX_ENTRIES));
        }
    }
    s.push_str(
        "規格必須以這份現況為前提：不可以宣稱某個路徑「是空的」或「不存在」，除非上面的清單證明如此；\
         已經存在的資料夾，特別是標了「獨立 git 專案」的，不要覆蓋或刪除，要整合進去或避開，並在規格裡寫清楚怎麼處理。\n\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        assert!(Command::new("git").args(args).current_dir(dir).status().unwrap().success());
    }

    #[test]
    fn lists_top_level_entries_and_marks_independent_git_projects() {
        let repo = tempfile::tempdir().unwrap();
        let d = repo.path();
        git(d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(d.join("HTBTW_Web/.git")).unwrap();
        std::fs::create_dir_all(d.join("docs")).unwrap();
        std::fs::write(d.join("README.md"), "x").unwrap();
        std::fs::write(d.join(".hidden"), "x").unwrap();
        let s = snapshot(d);
        assert!(s.contains("HTBTW_Web/（獨立 git 專案"), "{s}");
        assert!(s.contains("  - docs/\n") && s.contains("  - README.md\n"), "{s}");
        assert!(!s.contains(".hidden") && !s.contains("- .git"), "隱藏項目不列：{s}");
        assert!(s.contains("目前分支：main") && s.contains("未提交的變更"), "{s}");
        assert!(s.contains("不可以宣稱某個路徑"), "{s}");
    }

    #[test]
    fn an_empty_repo_says_so_and_a_missing_dir_adds_nothing() {
        assert!(format_snapshot("/r", None, None, &[]).contains("沒有（這是空的程式庫）"));
        assert_eq!(snapshot(Path::new("/definitely/not/here")), "");
    }

    #[test]
    fn a_long_listing_is_cut_with_the_total() {
        let many: Vec<Entry> = (0..MAX_ENTRIES + 5).map(|i| Entry { name: format!("f{i:03}"), is_dir: false, nested_git: false }).collect();
        let s = format_snapshot("/r", None, None, &many);
        assert!(s.contains("還有 5 項") && !s.contains("f044"), "{s}");
    }
}
