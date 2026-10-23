use std::path::{Component, Path, PathBuf};

fn file_path(root: &Path) -> PathBuf {
    root.join(".aiterm").join("architecture.json")
}

fn qa_path(root: &Path) -> PathBuf {
    root.join(".aiterm").join("architecture-qa.json")
}

fn read_opt(p: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(p) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn write_creating_dir(p: &Path, content: &str) -> Result<(), String> {
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(p, content).map_err(|e| e.to_string())
}

pub fn load(root: &Path) -> Result<Option<String>, String> {
    read_opt(&file_path(root))
}

pub fn save(root: &Path, json: &str) -> Result<(), String> {
    write_creating_dir(&file_path(root), json)
}

/// 問答記錄（與 architecture.json 分開存，重新分析架構不會洗掉對話）。
pub fn load_qa(root: &Path) -> Result<Option<String>, String> {
    read_opt(&qa_path(root))
}

pub fn save_qa(root: &Path, json: &str) -> Result<(), String> {
    write_creating_dir(&qa_path(root), json)
}

/// 把文字寫到使用者在「另存新檔」對話框選的路徑（匯出用）。路徑必須是絕對路徑且上層資料夾已存在。
pub fn export_text(path: &Path, content: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("export path must be absolute".into());
    }
    match path.parent() {
        Some(dir) if dir.is_dir() => std::fs::write(path, content).map_err(|e| e.to_string()),
        _ => Err("export folder does not exist".into()),
    }
}

/// 不呼叫 git 行程（Windows 會閃黑窗）；直接讀 .git/HEAD。
pub fn git_head(root: &Path) -> Option<String> {
    let git = root.join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref: ") else {
        return Some(head.to_string());
    };
    if let Ok(s) = std::fs::read_to_string(git.join(r)) {
        return Some(s.trim().to_string());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed
        .lines()
        .find_map(|l| l.strip_suffix(r).map(|h| h.trim().to_string()))
        .or_else(|| Some(r.to_string()))
}

/// 只讀專案根之內的相對路徑；絕對路徑、`..` 一律丟掉。每個檔案最多 `max_bytes`。
pub fn read_files(root: &Path, rels: &[String], max_bytes: usize) -> Vec<(String, String)> {
    rels.iter()
        .filter_map(|rel| {
            let p = Path::new(rel);
            if p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
                return None;
            }
            let bytes = std::fs::read(root.join(p)).ok()?;
            let cut = &bytes[..bytes.len().min(max_bytes)];
            Some((rel.clone(), String::from_utf8_lossy(cut).into_owned()))
        })
        .collect()
}

/// 把專案內的相對路徑解析成「確實在專案根之內」的正規化絕對路徑。
pub fn resolve_in_root(root: &Path, rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let canon_root = root.canonicalize().ok()?;
    let full = root.join(p).canonicalize().ok()?;
    // 符號連結指到根外時，正規化後就不再以根開頭。
    full.starts_with(&canon_root).then_some(full)
}

#[derive(Debug, serde::Serialize)]
pub struct FilePreview {
    pub content: String,
    pub truncated: bool,
    pub total_bytes: u64,
}

pub fn read_file_preview(root: &Path, rel: &str, max_bytes: usize) -> Result<FilePreview, String> {
    use std::io::Read;
    let full = resolve_in_root(root, rel).ok_or_else(|| format!("找不到檔案或路徑不在專案內：{rel}"))?;
    let meta = std::fs::metadata(&full).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("不是一般檔案：{rel}"));
    }
    let total_bytes = meta.len();
    let mut buf = Vec::new();
    std::fs::File::open(&full)
        .map_err(|e| e.to_string())?
        .take(max_bytes as u64)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.iter().take(8192).any(|&b| b == 0) {
        return Err("這是二進位檔，無法預覽".to_string());
    }
    Ok(FilePreview {
        content: String::from_utf8_lossy(&buf).into_owned(),
        truncated: total_bytes > max_bytes as u64,
        total_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);
        save(dir.path(), "{\"a\":1}").unwrap();
        assert_eq!(load(dir.path()).unwrap().as_deref(), Some("{\"a\":1}"));
        assert!(dir.path().join(".aiterm/architecture.json").is_file());
    }

    #[test]
    fn qa_is_stored_separately_from_the_architecture() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_qa(dir.path()).unwrap(), None);
        save_qa(dir.path(), "[1]").unwrap();
        save(dir.path(), "{}").unwrap();
        assert_eq!(load_qa(dir.path()).unwrap().as_deref(), Some("[1]"));
        assert_eq!(load(dir.path()).unwrap().as_deref(), Some("{}"));
        assert!(dir.path().join(".aiterm/architecture-qa.json").is_file());
    }

    #[test]
    fn export_text_writes_utf8_and_refuses_bad_paths() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("答.md");
        export_text(&out, "# 標題\n").unwrap();
        assert_eq!(fs::read_to_string(&out).unwrap(), "# 標題\n");
        assert!(export_text(Path::new("relative.md"), "x").is_err());
        assert!(export_text(&dir.path().join("nope").join("a.md"), "x").is_err());
        assert!(!dir.path().join("nope").exists());
    }

    #[test]
    fn git_head_resolves_branch_ref_and_detached() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git.join("refs/heads/main"), "abc123\n").unwrap();
        assert_eq!(git_head(dir.path()).as_deref(), Some("abc123"));
        fs::write(git.join("HEAD"), "def456\n").unwrap();
        assert_eq!(git_head(dir.path()).as_deref(), Some("def456"));
        assert_eq!(git_head(tempfile::tempdir().unwrap().path()), None);
    }

    #[test]
    fn read_files_rejects_paths_outside_root_and_truncates() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "x".repeat(100)).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "s").unwrap();

        let ok = read_files(dir.path(), &["a.txt".into()], 10);
        assert_eq!(ok[0].1.len(), 10);

        let evil = read_files(dir.path(), &["../secret.txt".into(), "/etc/passwd".into()], 10);
        assert!(evil.is_empty(), "越界路徑必須被丟掉: {evil:?}");
    }

    #[test]
    fn resolve_in_root_accepts_normal_and_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/sub")).unwrap();
        fs::write(dir.path().join("src/a.txt"), "x").unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("x"), "s").unwrap();

        let ok = resolve_in_root(dir.path(), "src/a.txt").unwrap();
        assert_eq!(ok, dir.path().join("src/a.txt").canonicalize().unwrap());
        assert!(resolve_in_root(dir.path(), "../x").is_none());
        assert!(resolve_in_root(dir.path(), "/etc/passwd").is_none());
        assert!(resolve_in_root(dir.path(), "src/sub/../a.txt").is_none());
        assert!(resolve_in_root(dir.path(), "./src/a.txt").is_none());
        assert!(resolve_in_root(dir.path(), "src/missing.txt").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_in_root_rejects_symlink_pointing_outside() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.txt"), dir.path().join("link.txt")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linkdir")).unwrap();
        assert!(resolve_in_root(dir.path(), "link.txt").is_none());
        assert!(resolve_in_root(dir.path(), "linkdir/secret.txt").is_none());
    }

    #[test]
    fn read_file_preview_reads_truncates_and_refuses() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello\nworld").unwrap();
        fs::write(dir.path().join("big.txt"), "y".repeat(100)).unwrap();
        fs::write(dir.path().join("bin.dat"), [b'a', 0, b'b']).unwrap();

        let p = read_file_preview(dir.path(), "a.txt", 1000).unwrap();
        assert_eq!(p.content, "hello\nworld");
        assert!(!p.truncated);
        assert_eq!(p.total_bytes, 11);

        let p = read_file_preview(dir.path(), "big.txt", 10).unwrap();
        assert_eq!(p.content.len(), 10);
        assert!(p.truncated);
        assert_eq!(p.total_bytes, 100);

        let e = read_file_preview(dir.path(), "bin.dat", 1000).unwrap_err();
        assert!(e.contains("二進位"), "{e}");
        assert!(read_file_preview(dir.path(), "../x", 10).is_err());
        assert!(read_file_preview(dir.path(), "nope.txt", 10).is_err());
    }
}
