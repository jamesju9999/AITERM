use std::path::{Component, Path, PathBuf};

fn file_path(root: &Path) -> PathBuf {
    root.join(".aiterm").join("architecture.json")
}

pub fn load(root: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(file_path(root)) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn save(root: &Path, json: &str) -> Result<(), String> {
    let p = file_path(root);
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(p, json).map_err(|e| e.to_string())
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
}
