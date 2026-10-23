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
    let edges = resolve_edges(root, &files);
    let deps = read_deps(root);
    Ok(ScanResult {
        root: root.to_string_lossy().into_owned(),
        files,
        edges,
        deps,
        truncated,
        git_head: None,
    })
}

use std::collections::{HashMap, HashSet};

use super::types::ScanEdge;

/// 從一行取出引號內的字串（`from "x"`、`require('x')`、`import "x"`）。
fn quoted(line: &str) -> Option<&str> {
    let start = line.find(|c| c == '"' || c == '\'')?;
    let q = line.as_bytes()[start] as char;
    let rest = &line[start + 1..];
    let end = rest.find(q)?;
    Some(&rest[..end])
}

fn parent_dir(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// 把 `a/b/../c` 這類路徑正規化；回傳 None 代表跳出專案根。
fn normalize(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => { out.pop()?; }
            s => out.push(s),
        }
    }
    Some(out.join("/"))
}

const JS_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte"];

fn resolve_js(from: &str, spec: &str, all: &HashSet<&str>) -> Option<String> {
    if !spec.starts_with('.') {
        return None; // 套件，不是專案內檔案
    }
    let base = normalize(&format!("{}/{}", parent_dir(from), spec))?;
    if all.contains(base.as_str()) {
        return Some(base);
    }
    for ext in JS_EXTS {
        let c = format!("{base}.{ext}");
        if all.contains(c.as_str()) { return Some(c); }
    }
    for ext in JS_EXTS {
        let c = format!("{base}/index.{ext}");
        if all.contains(c.as_str()) { return Some(c); }
    }
    None
}

fn resolve_py(spec: &str, all: &HashSet<&str>) -> Option<String> {
    let base = spec.replace('.', "/");
    for c in [format!("{base}.py"), format!("{base}/__init__.py")] {
        if all.contains(c.as_str()) { return Some(c); }
    }
    None
}

fn resolve_rs(from: &str, name: &str, all: &HashSet<&str>) -> Option<String> {
    // `mod x;` 與 `use crate::x::…`：先找同層，再找 src/ 下。
    let dir = parent_dir(from);
    for c in [format!("{dir}/{name}.rs"), format!("{dir}/{name}/mod.rs"), format!("src/{name}.rs"), format!("src/{name}/mod.rs")] {
        if all.contains(c.as_str()) { return Some(c); }
    }
    None
}

/// Java／Kotlin 的 import 是全名（`tw.app.infra.UserMapper`），不是相對路徑，專案可能有多個 source root
/// （`backend/app/src/main/java/…`），所以把每個 .java／.kt 檔「路徑的所有尾段」都登記起來，之後用 `tw/app/infra/UserMapper.java` 查。
/// 同尾段有多個檔案時取排序第一個（files 已排序），這種全名重複在同一專案裡本來就罕見。
fn jvm_suffix_index(files: &[ScanFile]) -> HashMap<String, &str> {
    let mut idx: HashMap<String, &str> = HashMap::new();
    for f in files.iter().filter(|f| f.lang == "java" || f.lang == "kotlin") {
        let segs: Vec<&str> = f.path.split('/').collect();
        for i in 0..segs.len() {
            idx.entry(segs[i..].join("/")).or_insert(f.path.as_str());
        }
    }
    idx
}

/// 解析一行 `import a.b.C;`、`import static a.b.C.m;`、Kotlin 的 `import a.b.C as D`。
/// 萬用字元（`a.b.*`）不指向單一檔案，略過；內部類別與靜態成員靠逐段往上退到有檔案的那一層。
fn resolve_jvm(line: &str, idx: &HashMap<String, &str>) -> Option<String> {
    let rest = line.strip_prefix("import ")?;
    let rest = rest.strip_prefix("static ").unwrap_or(rest);
    let name = rest.split(|c: char| c == ';' || c.is_whitespace()).next()?;
    if name.is_empty() || name.ends_with('*') {
        return None;
    }
    let mut segs: Vec<&str> = name.split('.').collect();
    while segs.len() >= 2 {
        let base = segs.join("/");
        for ext in ["java", "kt"] {
            if let Some(p) = idx.get(&format!("{base}.{ext}")) {
                return Some((*p).to_string());
            }
        }
        segs.pop();
    }
    None
}

fn resolve_edges(root: &Path, files: &[ScanFile]) -> Vec<ScanEdge> {
    let all: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let jvm = jvm_suffix_index(files);
    let mut edges = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for f in files {
        let Ok(src) = std::fs::read_to_string(root.join(&f.path)) else { continue };
        for line in src.lines() {
            let t = line.trim_start();
            let target = match f.lang.as_str() {
                "typescript" | "javascript" | "vue" | "svelte" => {
                    let is_import = t.starts_with("import ") || t.starts_with("export ") && t.contains(" from ") || t.contains("require(");
                    if is_import { quoted(t).and_then(|s| resolve_js(&f.path, s, &all)) } else { None }
                }
                "python" => {
                    if let Some(rest) = t.strip_prefix("from ") {
                        rest.split_whitespace().next().and_then(|m| resolve_py(m, &all))
                    } else if let Some(rest) = t.strip_prefix("import ") {
                        rest.split(|c: char| c == ',' || c.is_whitespace()).next().and_then(|m| resolve_py(m, &all))
                    } else { None }
                }
                "rust" => {
                    let name = if let Some(r) = t.strip_prefix("mod ").or_else(|| t.strip_prefix("pub mod ")) {
                        r.trim_end_matches(';').split_whitespace().next()
                    } else if let Some(r) = t.strip_prefix("use crate::") {
                        r.split("::").next()
                    } else { None };
                    name.and_then(|n| resolve_rs(&f.path, n.trim_end_matches(';'), &all))
                }
                "java" | "kotlin" => resolve_jvm(t, &jvm),
                _ => None,
            };
            if let Some(to) = target {
                if to != f.path && seen.insert((f.path.clone(), to.clone())) {
                    edges.push(ScanEdge { from: f.path.clone(), to });
                }
            }
        }
    }
    edges
}

fn read_deps(root: &Path) -> Vec<String> {
    let mut deps: Vec<String> = Vec::new();
    if let Ok(s) = std::fs::read_to_string(root.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
            for key in ["dependencies", "devDependencies"] {
                if let Some(o) = v.get(key).and_then(|x| x.as_object()) {
                    deps.extend(o.keys().cloned());
                }
            }
        }
    }
    if let Ok(s) = std::fs::read_to_string(root.join("Cargo.toml")) {
        let mut in_deps = false;
        for line in s.lines() {
            let l = line.trim();
            if l.starts_with('[') {
                in_deps = l.ends_with("dependencies]");
            } else if in_deps {
                if let Some((name, _)) = l.split_once('=') {
                    deps.push(name.trim().to_string());
                }
            }
        }
    }
    if let Ok(s) = std::fs::read_to_string(root.join("requirements.txt")) {
        for line in s.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') { continue; }
            let name: String = l.chars().take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.').collect();
            if !name.is_empty() { deps.push(name); }
        }
    }
    deps.sort();
    deps.dedup();
    deps
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

    #[test]
    fn resolves_relative_ts_imports_to_files() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "src/a.ts", "import { b } from \"./b\";\nimport x from \"./lib\";\nimport r from \"react\";\n");
        write(r, "src/b.ts", "export const b = 1;\n");
        write(r, "src/lib/index.ts", "export default 1;\n");
        let res = scan(r, 100).unwrap();
        let mut got: Vec<_> = res.edges.iter().map(|e| (e.from.as_str(), e.to.as_str())).collect();
        got.sort();
        assert_eq!(got, vec![("src/a.ts", "src/b.ts"), ("src/a.ts", "src/lib/index.ts")]);
    }

    #[test]
    fn resolves_python_and_rust_module_imports() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "app/main.py", "from app.db import conn\nimport os\n");
        write(r, "app/db.py", "conn = 1\n");
        write(r, "src/main.rs", "mod store;\nuse crate::store::Db;\n");
        write(r, "src/store.rs", "pub struct Db;\n");
        let res = scan(r, 100).unwrap();
        let mut got: Vec<_> = res.edges.iter().map(|e| (e.from.as_str(), e.to.as_str())).collect();
        got.sort();
        got.dedup();
        assert_eq!(got, vec![("app/main.py", "app/db.py"), ("src/main.rs", "src/store.rs")]);
    }

    #[test]
    fn resolves_java_and_kotlin_imports_across_source_roots() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "app/src/main/java/tw/x/app/UserService.java", "package tw.x.app;\nimport tw.x.infra.UserMapper;\nimport tw.x.infra.UserMapper.Key;\nimport static tw.x.domain.Util.fmt;\nimport tw.x.domain.*;\nimport java.util.List;\n");
        write(r, "infra/src/main/java/tw/x/infra/UserMapper.java", "package tw.x.infra;\npublic interface UserMapper { class Key {} }\n");
        write(r, "domain/src/main/java/tw/x/domain/Util.java", "package tw.x.domain;\n");
        write(r, "k/src/main/kotlin/tw/x/k/Main.kt", "import tw.x.domain.Util as U\nimport tw.x.infra.UserMapper\n");
        let res = scan(r, 100).unwrap();
        let mut got: Vec<_> = res.edges.iter().map(|e| (e.from.as_str(), e.to.as_str())).collect();
        got.sort();
        assert_eq!(got, vec![
            ("app/src/main/java/tw/x/app/UserService.java", "domain/src/main/java/tw/x/domain/Util.java"),
            ("app/src/main/java/tw/x/app/UserService.java", "infra/src/main/java/tw/x/infra/UserMapper.java"),
            ("k/src/main/kotlin/tw/x/k/Main.kt", "domain/src/main/java/tw/x/domain/Util.java"),
            ("k/src/main/kotlin/tw/x/k/Main.kt", "infra/src/main/java/tw/x/infra/UserMapper.java"),
        ]);
    }

    #[test]
    fn reads_dependency_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "package.json", r#"{"dependencies":{"react":"1","zod":"1"},"devDependencies":{"vitest":"1"}}"#);
        write(r, "Cargo.toml", "[package]\nname=\"x\"\n[dependencies]\nserde = \"1\"\ntokio = { version = \"1\" }\n[dev-dependencies]\ntempfile = \"3\"\n");
        write(r, "requirements.txt", "flask==2.0\n# c\nrequests>=2\n");
        let res = scan(r, 100).unwrap();
        for d in ["react", "zod", "vitest", "serde", "tokio", "tempfile", "flask", "requests"] {
            assert!(res.deps.contains(&d.to_string()), "missing {d}: {:?}", res.deps);
        }
    }
}
