//! 參考資料夾的檢查：路徑展開、能讀／讀不了的檔案計數。純檔案系統，三平台通用。
//! 設計：docs/superpowers/specs/2026-11-23-intake-reference-folders-design.md
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Claude Code 讀得到的文字類檔案（pdf 它可以直接讀）。
pub const READABLE: &[&str] = &["md", "markdown", "txt", "rst", "pdf", "csv", "json", "yaml", "yml", "xml", "html", "htm", "log"];
/// Claude Code 讀不了、需要先轉成 md／pdf 的文件類型。
pub const OFFICE: &[&str] = &["doc", "docx", "docm", "odt", "ppt", "pptx", "pps", "xls", "xlsx", "xlsm", "ods", "odp", "rtf", "epub"];
/// 最多看這麼多個項目（檔案與資料夾），超過就停並標示已截斷。
pub const MAX_ENTRIES: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderCheck {
    pub path: String,
    pub exists: bool,
    pub is_dir: bool,
    /// Claude Code 讀得到的檔案數。
    pub readable: usize,
    /// 讀不了的 Office 類檔案數。
    pub office: usize,
    /// 項目太多，只看了前面一部分。
    pub truncated: bool,
}

/// `~` 展開成家目錄；相對路徑接在 `base`（專案資料夾）後面；其餘原樣。
pub fn expand(path: &str, base: Option<&Path>) -> PathBuf {
    let p = path.trim();
    if p == "~" || p.starts_with("~/") || p.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            return home.join(p[1..].trim_start_matches(['/', '\\']));
        }
    }
    let pb = PathBuf::from(p);
    match base {
        Some(b) if pb.is_relative() => b.join(pb),
        _ => pb,
    }
}

/// 檢查一個參考資料夾：存不存在、是不是資料夾、遞迴數有多少讀得到／讀不了的檔案（略過 `.` 開頭的項目，不跟著符號連結走）。
pub fn check(path: &str, base: Option<&Path>) -> FolderCheck {
    let full = expand(path, base);
    let mut r = FolderCheck { path: path.to_string(), exists: full.exists(), is_dir: full.is_dir(), readable: 0, office: 0, truncated: false };
    if !r.is_dir {
        return r;
    }
    let mut seen = 0usize;
    let mut stack = vec![full];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.filter_map(|e| e.ok()) {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            seen += 1;
            if seen > MAX_ENTRIES {
                r.truncated = true;
                return r;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                let ext = e.path().extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                if READABLE.contains(&ext.as_str()) {
                    r.readable += 1;
                } else if OFFICE.contains(&ext.as_str()) {
                    r.office += 1;
                }
            }
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(dir: &Path, rel: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "x").unwrap();
    }

    #[test]
    fn a_missing_path_and_a_plain_file_are_reported_as_such() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = check(tmp.path().join("nope").to_str().unwrap(), None);
        assert_eq!((missing.exists, missing.is_dir, missing.readable), (false, false, 0));
        touch(tmp.path(), "a.md");
        let file = check(tmp.path().join("a.md").to_str().unwrap(), None);
        assert_eq!((file.exists, file.is_dir), (true, false), "是檔案不是資料夾");
    }

    #[test]
    fn readable_and_office_files_are_counted_recursively_and_hidden_or_other_files_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        for f in ["a.md", "b/c.PDF", "b/d/e.txt", "手冊.docx", "b/表.XLSX", "b/簡報.pptx", "img.png", "script.sh", ".hidden/x.md", ".secret.md"] {
            touch(tmp.path(), f);
        }
        let c = check(tmp.path().to_str().unwrap(), None);
        assert_eq!((c.exists, c.is_dir, c.readable, c.office, c.truncated), (true, true, 3, 3, false), "{c:?}");
    }

    #[test]
    fn relative_paths_resolve_against_the_base_and_tilde_against_home() {
        let tmp = tempfile::tempdir().unwrap();
        touch(tmp.path(), "docs/faq/a.md");
        let c = check("docs/faq", Some(tmp.path()));
        assert_eq!((c.is_dir, c.readable), (true, 1));
        assert!(!check("docs/faq", None).exists, "沒有基準目錄時相對路徑對不上");
        let abs = tmp.path().join("docs").to_string_lossy().to_string();
        assert!(check(&abs, Some(Path::new("/elsewhere"))).is_dir, "絕對路徑不受基準影響");
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expand("~", None), home);
            assert_eq!(expand("~/x/y", None), home.join("x").join("y"));
            assert_eq!(expand("~\\x", None), home.join("x"));
        }
    }

    #[test]
    fn a_huge_folder_stops_early_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..(MAX_ENTRIES + 20) {
            fs::write(tmp.path().join(format!("f{i}.md")), "x").unwrap();
        }
        let c = check(tmp.path().to_str().unwrap(), None);
        assert!(c.truncated && c.readable <= MAX_ENTRIES && c.readable > 0, "{c:?}");
    }
}
