use std::path::Path;

use ignore::WalkBuilder;

use super::types::{ScanFile, ScanResult};

/// 不論 .gitignore 怎麼寫都跳過（沒 git 的資料夾也要有基本衛生）。
const ALWAYS_SKIP: &[&str] = &["node_modules", "target", "dist", "build", ".git", "__pycache__", ".venv", "venv", ".next"];

pub fn lang_of(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    Some(match ext {
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "vue" => "vue",
        "svelte" => "svelte",
        "sql" => "sql",
        _ => return None,
    })
}

pub fn scan(root: &Path, max_files: usize) -> Result<ScanResult, String> {
    if !root.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }
    let mut files = Vec::new();
    let mut truncated = false;
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .filter_entry(|e| {
            e.file_name().to_str().map_or(true, |n| !ALWAYS_SKIP.contains(&n))
        })
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().map_or(false, |t| t.is_file()) {
            continue;
        }
        let rel = match entry.path().strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let Some(lang) = lang_of(&rel) else { continue };
        if files.len() >= max_files {
            truncated = true;
            break;
        }
        let lines = std::fs::read_to_string(entry.path()).map(|s| s.lines().count()).unwrap_or(0);
        files.push(ScanFile { path: rel, lang: lang.to_string(), lines });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(ScanResult {
        root: root.to_string_lossy().into_owned(),
        files,
        edges: Vec::new(),
        deps: Vec::new(),
        truncated,
        git_head: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &std::path::Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    #[test]
    fn walks_files_respecting_gitignore_and_uses_forward_slashes() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join(".git")).unwrap(); // ignore crate 需要 .git 才套用 .gitignore
        write(r, ".gitignore", "dist/\n");
        write(r, "src/a.ts", "export const a = 1;\n");
        write(r, "dist/out.js", "x\n");
        write(r, "node_modules/p/index.js", "x\n");
        let res = scan(r, 100).unwrap();
        let paths: Vec<_> = res.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["src/a.ts"]);
        assert_eq!(res.files[0].lang, "typescript");
        assert_eq!(res.files[0].lines, 1);
        assert!(!res.truncated);
    }

    #[test]
    fn truncates_when_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            write(dir.path(), &format!("f{i}.py"), "x = 1\n");
        }
        let res = scan(dir.path(), 3).unwrap();
        assert_eq!(res.files.len(), 3);
        assert!(res.truncated);
    }
}
