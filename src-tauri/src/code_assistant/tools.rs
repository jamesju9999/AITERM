use std::path::{Path, PathBuf};
use std::fs;
use crate::code_assistant::tree;

pub const MAX_LIST_ENTRIES: usize = 200;
pub const MAX_FILE_BYTES: u64 = 100 * 1024; // 100 KB
pub const MAX_SEARCH_MATCHES: usize = 50;
const MAX_SEARCH_LINE_CHARS: usize = 300;
const MAX_SEARCH_TOTAL_BYTES: usize = 30_000; // ~7 500 tokens
pub const MAX_FIND_RESULTS: usize = 100;
const MAX_TREE_LINES: usize = 300;

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
    let canon_root = project_root.canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let target = resolve_safe(project_root, rel_path)?;

    if !target.is_dir() {
        return Err(format!("{rel_path} is not a directory"));
    }

    let entries = fs::read_dir(&target)
        .map_err(|e| format!("Cannot list directory: {e}"))?;

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| !tree::is_excluded(&e.path(), &canon_root))
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
    let canon_root = project_root.canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let target = resolve_safe(project_root, rel_path)?;

    if tree::is_excluded(&target, &canon_root) {
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

const MAX_READ_LINES: usize = 200;

/// Read a specific line range from a file (1-indexed, inclusive).
/// Returns lines prefixed with line numbers so the model can reference them.
pub fn read_file_lines(
    project_root: &Path,
    rel_path: &str,
    start_line: usize,
    end_line: usize,
) -> Result<ToolResult, String> {
    let canon_root = project_root.canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let target = resolve_safe(project_root, rel_path)?;

    if tree::is_excluded(&target, &canon_root) {
        return Err("This file type is not readable".into());
    }
    if target.is_dir() {
        return Err(format!("{rel_path} is a directory, not a file"));
    }

    let start = start_line.max(1);
    let end = end_line.max(start);

    let content = fs::read_to_string(&target)
        .map_err(|e| format!("Cannot read file (may be binary): {e}"))?;

    let total_lines = content.lines().count();

    // Cap window to MAX_READ_LINES
    let end_capped = end.min(start + MAX_READ_LINES - 1).min(total_lines);

    let lines: Vec<String> = content
        .lines()
        .enumerate()
        .filter(|(i, _)| {
            let line_no = i + 1;
            line_no >= start && line_no <= end_capped
        })
        .map(|(i, line)| format!("{:>5}: {}", i + 1, line))
        .collect();

    if lines.is_empty() {
        return Err(format!(
            "No lines in range {start}–{end_capped} (file has {total_lines} lines)"
        ));
    }

    let truncated = end_capped < end.min(total_lines);
    let mut result = lines.join("\n");
    if truncated {
        result.push_str(&format!(
            "\n[TRUNCATED: showing lines {start}–{end_capped} of {total_lines}; call again with start_line={} to continue]",
            end_capped + 1
        ));
    } else {
        result.push_str(&format!("\n[Lines {start}–{end_capped} of {total_lines}]"));
    }

    Ok(ToolResult { content: result, truncated })
}

pub fn search_in_files(
    project_root: &Path,
    query: &str,
    file_pattern: Option<&str>,
    search_path: Option<&str>,
) -> Result<ToolResult, String> {
    search_in_files_with_progress(project_root, query, file_pattern, search_path, &|_| {})
}

pub fn search_in_files_with_progress(
    project_root: &Path,
    query: &str,
    file_pattern: Option<&str>,
    search_path: Option<&str>,
    on_progress: &dyn Fn(&str),
) -> Result<ToolResult, String> {
    let canon_root = project_root.canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());

    let start_dir = match search_path {
        Some(p) if !p.trim_start_matches('/').is_empty() && p != "/" => {
            match resolve_safe(project_root, p) {
                Ok(d) if d.is_dir() => d,
                Ok(_) => return Err(format!("Not a directory: {p}")),
                Err(e) => return Err(e),
            }
        }
        _ => canon_root.clone(),
    };

    let mut matches: Vec<String> = Vec::new();
    search_recursive(&canon_root, &start_dir, query, file_pattern, &mut matches, on_progress);

    let count_truncated = matches.len() > MAX_SEARCH_MATCHES;
    if count_truncated {
        matches.truncate(MAX_SEARCH_MATCHES);
    }

    if matches.is_empty() {
        return Ok(ToolResult { content: "No matches found.".into(), truncated: false });
    }

    // Enforce a total byte cap so very long lines don't blow the context window.
    let content = matches.join("\n");
    let (content, size_truncated) = if content.len() > MAX_SEARCH_TOTAL_BYTES {
        let cut = &content[..MAX_SEARCH_TOTAL_BYTES];
        let safe = cut.rfind('\n').map_or(MAX_SEARCH_TOTAL_BYTES, |p| p);
        (format!("{}\n[TRUNCATED: result too large]", &content[..safe]), true)
    } else {
        (content, false)
    };

    Ok(ToolResult {
        content,
        truncated: count_truncated || size_truncated,
    })
}

fn search_recursive(
    root: &Path,
    dir: &Path,
    query: &str,
    pattern: Option<&str>,
    matches: &mut Vec<String>,
    on_progress: &dyn Fn(&str),
) {
    if matches.len() >= MAX_SEARCH_MATCHES {
        return;
    }

    // Emit current directory as progress (relative path)
    let rel_dir = dir.strip_prefix(root).unwrap_or(dir).to_string_lossy();
    if !rel_dir.is_empty() {
        on_progress(&rel_dir);
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
            search_recursive(root, &path, query, pattern, matches, on_progress);
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
                    let truncated_line = if line.len() > MAX_SEARCH_LINE_CHARS {
                        format!("{}…", &line[..MAX_SEARCH_LINE_CHARS])
                    } else {
                        line.to_string()
                    };
                    matches.push(format!("{rel}:{}: {truncated_line}", line_num + 1));
                }
            }
        }
    }
}

/// Find files by name pattern (case-insensitive substring match).
pub fn find_files(
    project_root: &Path,
    name_pattern: &str,
    file_extension: Option<&str>,
) -> Result<ToolResult, String> {
    let canon_root = project_root.canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut results: Vec<String> = Vec::new();
    let pattern_lower = name_pattern.to_lowercase();
    find_recursive(&canon_root, &canon_root, &pattern_lower, file_extension, &mut results);

    let truncated = results.len() > MAX_FIND_RESULTS;
    if truncated {
        results.truncate(MAX_FIND_RESULTS);
    }

    if results.is_empty() {
        return Ok(ToolResult { content: "No files found.".into(), truncated: false });
    }

    Ok(ToolResult { content: results.join("\n"), truncated })
}

fn find_recursive(
    root: &Path,
    dir: &Path,
    pattern_lower: &str,
    ext_filter: Option<&str>,
    results: &mut Vec<String>,
) {
    if results.len() >= MAX_FIND_RESULTS { return; }
    let Ok(entries) = fs::read_dir(dir) else { return };

    let mut items: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    items.sort_by_key(|e| e.file_name());

    for entry in items {
        if results.len() >= MAX_FIND_RESULTS { break; }
        let path = entry.path();
        if tree::is_excluded(&path, root) { continue; }

        if path.is_dir() {
            find_recursive(root, &path, pattern_lower, ext_filter, results);
        } else {
            if let Some(ext) = ext_filter {
                let ext_norm = ext.trim_start_matches('.').to_lowercase();
                let file_ext = path.extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if file_ext != ext_norm { continue; }
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.contains(pattern_lower) {
                let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().to_string();
                results.push(rel);
            }
        }
    }
}

/// Get a multi-level directory tree (max depth 5, max 300 lines).
pub fn get_file_tree(
    project_root: &Path,
    rel_path: &str,
    depth: usize,
) -> Result<ToolResult, String> {
    let canon_root = project_root.canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let target = resolve_safe(project_root, rel_path)?;
    if !target.is_dir() {
        return Err(format!("{rel_path} is not a directory"));
    }

    let max_depth = depth.clamp(1, 5);
    let root_name = if rel_path.trim_matches('/').is_empty() {
        ".".to_string()
    } else {
        rel_path.trim_end_matches('/').to_string()
    };

    let mut lines: Vec<String> = vec![format!("{root_name}/")];
    build_tree(&target, &canon_root, 1, max_depth, "", &mut lines);

    let truncated = lines.len() > MAX_TREE_LINES;
    if truncated {
        lines.truncate(MAX_TREE_LINES);
        lines.push("[TRUNCATED]".into());
    }

    Ok(ToolResult { content: lines.join("\n"), truncated })
}

fn build_tree(
    dir: &Path,
    project_root: &Path,
    depth: usize,
    max_depth: usize,
    prefix: &str,
    lines: &mut Vec<String>,
) {
    if lines.len() >= MAX_TREE_LINES { return; }
    let Ok(entries) = fs::read_dir(dir) else { return };

    let mut items: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| !tree::is_excluded(&e.path(), project_root))
        .collect();
    // dirs first, then files, each group sorted alphabetically
    items.sort_by_key(|e| {
        let is_file = e.file_type().map(|t| !t.is_dir()).unwrap_or(true);
        (is_file, e.file_name())
    });

    let count = items.len();
    for (i, entry) in items.iter().enumerate() {
        if lines.len() >= MAX_TREE_LINES { break; }
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();

        if path.is_dir() {
            lines.push(format!("{prefix}{connector}{name}/"));
            if depth < max_depth {
                build_tree(&path, project_root, depth + 1, max_depth, &child_prefix, lines);
            }
        } else {
            lines.push(format!("{prefix}{connector}{name}"));
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
        let result = search_in_files(project.path(), "hello", None, None).unwrap();
        assert!(result.content.contains("main.rs") && result.content.contains("README.md"));
    }

    #[test]
    fn search_with_pattern_filters() {
        let project = make_project();
        let result = search_in_files(project.path(), "hello", Some(".md"), None).unwrap();
        assert!(result.content.contains("README.md"));
        assert!(!result.content.contains("main.rs"));
    }

    #[test]
    fn read_file_lines_returns_range() {
        let project = make_project();
        // src/main.rs has 1 line: fn main() { println!("hello"); }
        let result = read_file_lines(project.path(), "src/main.rs", 1, 1).unwrap();
        assert!(result.content.contains("fn main"));
        assert!(result.content.contains("    1:"));
    }

    #[test]
    fn read_file_lines_clamps_to_file_length() {
        let project = make_project();
        let result = read_file_lines(project.path(), "README.md", 1, 9999).unwrap();
        assert!(result.content.contains("hello world"));
        assert!(!result.truncated); // file is short, should not be truncated
    }

    #[test]
    fn search_scoped_to_subdirectory() {
        let project = make_project();
        let result = search_in_files(project.path(), "hello", None, Some("src")).unwrap();
        assert!(result.content.contains("main.rs"));
        assert!(!result.content.contains("README.md"));
    }
}
