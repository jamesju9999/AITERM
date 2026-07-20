use std::path::{Path, PathBuf};
use std::fs;
use crate::code_assistant::tree;

pub const MAX_LIST_ENTRIES: usize = 200;
pub const MAX_FILE_BYTES: u64 = 100 * 1024; // 100 KB
pub const MAX_SEARCH_MATCHES: usize = 50;

pub struct ToolResult {
    pub content: String,
    pub truncated: bool,
}

/// Resolve and validate a relative path against the project root.
/// Returns Err if the resolved path escapes the project root.
fn resolve_safe(project_root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let norm = rel_path.trim_start_matches('/');
    let target = if norm.is_empty() || norm == "." {
        project_root.to_path_buf()
    } else {
        project_root.join(norm)
    };

    let canonical_root = project_root
        .canonicalize()
        .map_err(|e| format!("Cannot resolve project root: {e}"))?;
    let canonical_target = target
        .canonicalize()
        .map_err(|e| format!("Path not found: {e}"))?;

    if !canonical_target.starts_with(&canonical_root) {
        return Err("Path is outside project root".into());
    }
    Ok(canonical_target)
}

pub fn list_directory(project_root: &Path, rel_path: &str) -> Result<ToolResult, String> {
    let target = resolve_safe(project_root, rel_path)?;

    if !target.is_dir() {
        return Err(format!("{rel_path} is not a directory"));
    }

    let entries = fs::read_dir(&target)
        .map_err(|e| format!("Cannot list directory: {e}"))?;

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| !tree::is_excluded(&e.path(), project_root))
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let suffix = match e.file_type() {
                Ok(t) if t.is_dir() => "/",
                Ok(t) if t.is_symlink() => "@",
                _ => "",
            };
            format!("{name}{suffix}")
        })
        .collect();

    names.sort();
    let truncated = names.len() > MAX_LIST_ENTRIES;
    if truncated {
        names.truncate(MAX_LIST_ENTRIES);
    }

    Ok(ToolResult {
        content: names.join("\n"),
        truncated,
    })
}

pub fn read_file(project_root: &Path, rel_path: &str) -> Result<ToolResult, String> {
    let target = resolve_safe(project_root, rel_path)?;

    if tree::is_excluded(&target, project_root) {
        return Err("This file type is not readable".into());
    }

    if target.is_dir() {
        return Err(format!("{rel_path} is a directory, not a file"));
    }

    let metadata = fs::metadata(&target)
        .map_err(|e| format!("Cannot stat file: {e}"))?;

    if metadata.len() > MAX_FILE_BYTES {
        use std::io::Read;
        let mut f = fs::File::open(&target)
            .map_err(|e| format!("Cannot open file: {e}"))?;
        let mut raw = Vec::with_capacity(MAX_FILE_BYTES as usize);
        f.take(MAX_FILE_BYTES).read_to_end(&mut raw)
            .map_err(|e| format!("Cannot read file: {e}"))?;
        let content = String::from_utf8_lossy(&raw).into_owned();
        return Ok(ToolResult {
            content: format!("{content}\n\n[TRUNCATED: file exceeds 100 KB limit]"),
            truncated: true,
        });
    }

    let content = fs::read_to_string(&target)
        .map_err(|e| format!("Cannot read file (may be binary): {e}"))?;

    Ok(ToolResult { content, truncated: false })
}

pub fn search_in_files(
    project_root: &Path,
    query: &str,
    file_pattern: Option<&str>,
) -> Result<ToolResult, String> {
    let mut matches: Vec<String> = Vec::new();
    search_recursive(project_root, project_root, query, file_pattern, &mut matches);

    let truncated = matches.len() > MAX_SEARCH_MATCHES;
    if truncated {
        matches.truncate(MAX_SEARCH_MATCHES);
    }

    if matches.is_empty() {
        return Ok(ToolResult { content: "No matches found.".into(), truncated: false });
    }

    Ok(ToolResult {
        content: matches.join("\n"),
        truncated,
    })
}

fn search_recursive(
    root: &Path,
    dir: &Path,
    query: &str,
    pattern: Option<&str>,
    matches: &mut Vec<String>,
) {
    if matches.len() >= MAX_SEARCH_MATCHES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };

    for entry in entries.filter_map(|e| e.ok()) {
        if matches.len() >= MAX_SEARCH_MATCHES {
            break;
        }
        let path = entry.path();
        if tree::is_excluded(&path, root) {
            continue;
        }
        if path.is_dir() {
            search_recursive(root, &path, query, pattern, matches);
        } else {
            if let Some(pat) = pattern {
                let ext = format!(
                    ".{}",
                    path.extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                );
                let needle = pat.trim_start_matches('*').to_lowercase();
                if !ext.ends_with(&needle) {
                    continue;
                }
            }

            let Ok(content) = fs::read_to_string(&path) else { continue };
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy();
            let query_lower = query.to_lowercase();

            for (line_num, line) in content.lines().enumerate() {
                if matches.len() >= MAX_SEARCH_MATCHES {
                    break;
                }
                if line.to_lowercase().contains(&query_lower) {
                    matches.push(format!("{rel}:{}: {line}", line_num + 1));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        let mut f = fs::File::create(dir.path().join("src/main.rs")).unwrap();
        writeln!(f, "fn main() {{ println!(\"hello\"); }}").unwrap();
        let mut g = fs::File::create(dir.path().join("README.md")).unwrap();
        writeln!(g, "# My Project\nhello world").unwrap();
        dir
    }

    #[test]
    fn list_directory_lists_files() {
        let project = make_project();
        let result = list_directory(project.path(), "/").unwrap();
        assert!(result.content.contains("src/"));
        assert!(result.content.contains("README.md"));
    }

    #[test]
    fn list_directory_rejects_escape() {
        let project = make_project();
        let result = list_directory(project.path(), "../../etc");
        assert!(result.is_err());
    }

    #[test]
    fn read_file_returns_content() {
        let project = make_project();
        let result = read_file(project.path(), "README.md").unwrap();
        assert!(result.content.contains("hello world"));
        assert!(!result.truncated);
    }

    #[test]
    fn read_file_rejects_escape() {
        let project = make_project();
        let result = read_file(project.path(), "../../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn search_finds_match() {
        let project = make_project();
        let result = search_in_files(project.path(), "hello", None).unwrap();
        assert!(result.content.contains("main.rs") && result.content.contains("README.md"));
    }

    #[test]
    fn search_with_pattern_filters() {
        let project = make_project();
        let result = search_in_files(project.path(), "hello", Some(".md")).unwrap();
        assert!(result.content.contains("README.md"));
        assert!(!result.content.contains("main.rs"));
    }
}
