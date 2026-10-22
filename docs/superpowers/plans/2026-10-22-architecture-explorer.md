# 架構導覽分頁 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增「架構導覽」分頁：選專案資料夾後，靜態掃描＋AI 分析，畫出前端→傳輸→後端→資料層的可點選分層圖。

**Architecture:** Rust 的 `architecture/` 模組負責掃描（檔案、import、依賴、git HEAD）與讀寫 `.aiterm/architecture.json`、讀取關鍵檔案。AI 分析放在前端（沿用 `invokeAiComplete` + `stripFence` 的既有做法，如 `briefRefine.ts`），回應在前端以掃描結果驗證，丟掉幻覺。圖用 SVG＋絕對定位按鈕自繪，不新增圖形函式庫。

**Tech Stack:** Rust（`ignore` crate 遍歷、serde）、React 19、Vitest、Tauri IPC。

**與 spec 的差異：** spec 寫 analyzer 在 Rust；實作改放前端，因為 repo 內所有「一次性 AI 補全」都走 `ai_complete` 由前端驅動，不需新增後端 AI 呼叫路徑。spec 已同步更新（Task 9）。

---

## File Structure

| 檔案 | 責任 |
|------|------|
| `src-tauri/src/architecture/mod.rs` | 公開 API：`scan_project`、`read_key_files`、`load`/`save` |
| `src-tauri/src/architecture/types.rs` | `ScanResult`、`ScanFile`、`ScanEdge` |
| `src-tauri/src/architecture/scanner.rs` | 遍歷、語言判斷、import 解析、依賴清單 |
| `src-tauri/src/architecture/store.rs` | `.aiterm/architecture.json` 讀寫、git HEAD、安全讀檔 |
| `src-tauri/src/commands/architecture.rs` | `arch_scan`、`arch_load`、`arch_save`、`arch_read_files` |
| `src/ipc/architecture.ts` | 型別與 invoke 包裝 |
| `src/components/ArchitectureView/model.ts` | `Architecture` 型別、`buildBaseGraph`（純掃描的預設分層） |
| `src/components/ArchitectureView/analyze.ts` | 系統提示、`pickKeyFiles`、`buildAnalyzeMessage`、`parseAnalysis`（驗證） |
| `src/components/ArchitectureView/layout.ts` | 分層座標與連線路徑（純函式） |
| `src/components/ArchitectureView/DetailPanel.tsx` | 右側詳情面板 |
| `src/components/ArchitectureView/index.tsx` / `index.css` | 分頁主體 |
| 修改：`TabBar/index.tsx`、`NewTabPicker/tabCatalog.tsx`、`tabCatalog.test.tsx`、`TerminalApp.tsx`、`tabTypeCoverage.test.ts`、`lib/i18n.ts`、`Icons.tsx` | 分頁型別接線 |
| 修改：`src-tauri/Cargo.toml`、`src-tauri/src/lib.rs`、`src-tauri/src/commands/mod.rs` | 註冊 |

**工作前提：** 在獨立 worktree 進行（`superpowers:using-git-worktrees`），symlink `node_modules`/`target`/`binaries`。Rust 驗證一律 `cd src-tauri && cargo test --workspace --no-fail-fast`，看每一行 `test result:`；前端型別檢查用 `npx tsc -b`。

---

### Task 1: Rust 型別與依賴

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/architecture/mod.rs`, `src-tauri/src/architecture/types.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 加依賴**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 加：

```toml
ignore = "0.4"
```

- [ ] **Step 2: 寫型別**

`src-tauri/src/architecture/types.rs`：

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanFile {
    /// 相對專案根、一律用 `/` 分隔（跨平台一致）。
    pub path: String,
    pub lang: String,
    pub lines: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanResult {
    pub root: String,
    pub files: Vec<ScanFile>,
    pub edges: Vec<ScanEdge>,
    /// 依賴清單（package.json／Cargo.toml／requirements.txt 的套件名）。
    pub deps: Vec<String>,
    /// 檔案數超過上限，只掃了一部分。
    pub truncated: bool,
    pub git_head: Option<String>,
}
```

`src-tauri/src/architecture/mod.rs`：

```rust
//! 架構導覽：靜態掃描專案，產出檔案／import／依賴，供前端畫分層圖。
//! 見 docs/superpowers/specs/2026-10-22-architecture-explorer-design.md

pub mod scanner;
pub mod store;
pub mod types;
```

在 `src-tauri/src/lib.rs` 的 `pub mod` 區（`pub mod api_docs;` 附近）加 `pub mod architecture;`。先建兩個空檔 `scanner.rs`、`store.rs` 讓編譯過。

- [ ] **Step 3: 編譯檢查**

Run: `cd src-tauri && cargo check`
Expected: 成功（須先跑過 `scripts/setup-uv-*` 腳本，見 CLAUDE.md）。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/architecture src-tauri/src/lib.rs
git commit -m "feat(architecture): 掃描結果型別與 ignore 依賴

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: 掃描器——檔案遍歷與語言

**Files:**
- Modify: `src-tauri/src/architecture/scanner.rs`

- [ ] **Step 1: 寫失敗測試**

`scanner.rs` 底部：

```rust
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
```

- [ ] **Step 2: 確認失敗**

Run: `cd src-tauri && cargo test --lib architecture::scanner -- --nocapture`
Expected: 編譯失敗 `cannot find function scan`。

- [ ] **Step 3: 實作**

```rust
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
```

- [ ] **Step 4: 確認通過**

Run: `cd src-tauri && cargo test --lib architecture::scanner`
Expected: 2 passed。（`truncates` 測試排序前截斷，只驗數量與旗標。）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/architecture/scanner.rs
git commit -m "feat(architecture): 掃描器遍歷檔案並判斷語言

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: 掃描器——import 解析與依賴清單

**Files:**
- Modify: `src-tauri/src/architecture/scanner.rs`

- [ ] **Step 1: 寫失敗測試**（加進同一個 `tests` 模組）

```rust
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
```

- [ ] **Step 2: 確認失敗**

Run: `cd src-tauri && cargo test --lib architecture::scanner`
Expected: 三個新測試 FAIL（edges／deps 為空）。

- [ ] **Step 3: 實作**

在 `scanner.rs` 加入，並在 `scan` 回傳前呼叫：`let edges = resolve_edges(root, &files); let deps = read_deps(root);` 填進 `ScanResult`。

```rust
use std::collections::HashSet;

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

fn resolve_edges(root: &Path, files: &[ScanFile]) -> Vec<ScanEdge> {
    let all: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
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
```

`scan` 內改為（`files.sort_by` 之後）：

```rust
    let edges = resolve_edges(root, &files);
    let deps = read_deps(root);
```

並把 `edges`、`deps` 填入 `ScanResult`（取代 `Vec::new()`）。

- [ ] **Step 4: 確認通過**

Run: `cd src-tauri && cargo test --lib architecture::scanner`
Expected: 5 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/architecture/scanner.rs
git commit -m "feat(architecture): 解析 TS/Python/Rust import 與依賴清單

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 儲存、git HEAD、安全讀檔

**Files:**
- Modify: `src-tauri/src/architecture/store.rs`

- [ ] **Step 1: 寫失敗測試**

```rust
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
```

- [ ] **Step 2: 確認失敗**

Run: `cd src-tauri && cargo test --lib architecture::store`
Expected: 編譯失敗（函式未定義）。

- [ ] **Step 3: 實作**

```rust
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
```

- [ ] **Step 4: 確認通過**

Run: `cd src-tauri && cargo test --lib architecture::store`
Expected: 3 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/architecture/store.rs
git commit -m "feat(architecture): 儲存、git HEAD 與越界防護的讀檔

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Tauri commands 與註冊

**Files:**
- Create: `src-tauri/src/commands/architecture.rs`
- Modify: `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs`

- [ ] **Step 1: 寫 commands**

```rust
// src-tauri/src/commands/architecture.rs
use std::path::PathBuf;

use crate::architecture::{scanner, store, types::ScanResult};

/// 檔案數上限；超過時 `truncated = true`。
const MAX_FILES: usize = 3000;
/// 每個關鍵檔案送給 AI 的上限（位元組）。
const MAX_FILE_BYTES: usize = 6000;

#[tauri::command]
pub async fn arch_scan(root: String) -> Result<ScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = PathBuf::from(root);
        let mut res = scanner::scan(&root, MAX_FILES)?;
        res.git_head = store::git_head(&root);
        Ok(res)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn arch_load(root: String) -> Result<Option<String>, String> {
    store::load(&PathBuf::from(root))
}

#[tauri::command]
pub fn arch_save(root: String, json: String) -> Result<(), String> {
    store::save(&PathBuf::from(root), &json)
}

#[tauri::command]
pub async fn arch_read_files(root: String, paths: Vec<String>) -> Result<Vec<(String, String)>, String> {
    tauri::async_runtime::spawn_blocking(move || store::read_files(&PathBuf::from(root), &paths, MAX_FILE_BYTES))
        .await
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 2: 註冊**

`commands/mod.rs` 加 `pub mod architecture;`。`lib.rs` 的 `use commands::{ … }` 加：

```rust
    architecture::{arch_load, arch_read_files, arch_save, arch_scan},
```

`invoke_handler` 清單（`api_docs_auth_status,` 之後）加：

```rust
            arch_scan,
            arch_load,
            arch_save,
            arch_read_files,
```

- [ ] **Step 3: 驗證**

Run: `cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep "test result"`
Expected: 每一行都是 `ok`，且 failed 為 0（若有既有環境性失敗如 aiterm-core 的 pty 測試，記錄並確認與本次無關）。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands src-tauri/src/lib.rs
git commit -m "feat(architecture): arch_scan／load／save／read_files 指令

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: 前端 IPC 與模型（預設分層）

**Files:**
- Create: `src/ipc/architecture.ts`, `src/components/ArchitectureView/model.ts`, `src/components/ArchitectureView/model.test.ts`

- [ ] **Step 1: IPC 包裝**

```ts
// src/ipc/architecture.ts
import { invoke } from "@tauri-apps/api/core";

export interface ScanFile { path: string; lang: string; lines: number }
export interface ScanEdge { from: string; to: string }
export interface ScanResult {
  root: string;
  files: ScanFile[];
  edges: ScanEdge[];
  deps: string[];
  truncated: boolean;
  git_head: string | null;
}

export const archScan = (root: string) => invoke<ScanResult>("arch_scan", { root });
export const archLoad = (root: string) => invoke<string | null>("arch_load", { root });
export const archSave = (root: string, json: string) => invoke<void>("arch_save", { root, json });
export const archReadFiles = (root: string, paths: string[]) =>
  invoke<[string, string][]>("arch_read_files", { root, paths });
```

- [ ] **Step 2: 寫失敗測試**

```ts
// src/components/ArchitectureView/model.test.ts
import { describe, expect, it } from "vitest";
import { buildBaseGraph, guessLayer, moduleIdOf } from "./model";
import type { ScanResult } from "../../ipc/architecture";

const scan = (files: string[], edges: [string, string][] = []): ScanResult => ({
  root: "/p",
  files: files.map((path) => ({ path, lang: "typescript", lines: 10 })),
  edges: edges.map(([from, to]) => ({ from, to })),
  deps: [],
  truncated: false,
  git_head: null,
});

describe("moduleIdOf", () => {
  it("取前兩層目錄；根目錄檔案歸 (root)", () => {
    expect(moduleIdOf("src/components/A.tsx")).toBe("src/components");
    expect(moduleIdOf("src/App.tsx")).toBe("src");
    expect(moduleIdOf("main.py")).toBe("(root)");
  });
});

describe("guessLayer", () => {
  it("依路徑關鍵字猜分層", () => {
    expect(guessLayer("src/components")).toBe("frontend");
    expect(guessLayer("src-tauri/src")).toBe("backend");
    expect(guessLayer("app/models")).toBe("data");
    expect(guessLayer("scripts")).toBe("other");
  });
});

describe("buildBaseGraph", () => {
  it("把檔案 import 彙整成模組間的 scan 連線，忽略模組內部連線", () => {
    const g = buildBaseGraph(scan(
      ["src/components/A.tsx", "src/components/B.tsx", "src/ipc/x.ts"],
      [["src/components/A.tsx", "src/ipc/x.ts"], ["src/components/A.tsx", "src/components/B.tsx"]],
    ));
    expect(g.modules.map((m) => m.id).sort()).toEqual(["src/components", "src/ipc"]);
    expect(g.edges).toHaveLength(1);
    expect(g.edges[0]).toMatchObject({ from: "src/components", to: "src/ipc", kind: "import", source: "scan" });
    expect(g.modules.find((m) => m.id === "src/components")!.files).toHaveLength(2);
  });
});
```

- [ ] **Step 3: 確認失敗**

Run: `npx vitest run src/components/ArchitectureView/model.test.ts`
Expected: FAIL（找不到 `./model`）。

- [ ] **Step 4: 實作**

```ts
// src/components/ArchitectureView/model.ts
import type { ScanResult } from "../../ipc/architecture";

export type EdgeKind = "import" | "ipc" | "http" | "sql";

export interface Layer { id: string; name: string }
export interface Module {
  id: string;
  layer: string;
  name: string;
  files: string[];
  tech: string[];
  summary: string;
  dataflow: string;
}
export interface Edge {
  from: string;
  to: string;
  kind: EdgeKind;
  source: "scan" | "ai";
  confidence: number;
  evidence?: { file: string; line: number };
  note?: string;
}
export interface Architecture {
  layers: Layer[];
  modules: Module[];
  edges: Edge[];
  meta: { scannedAt: string; gitHead: string | null; analyzed: boolean; dropped: number };
}

export const BASE_LAYERS: Layer[] = [
  { id: "frontend", name: "前端" },
  { id: "transport", name: "傳輸 / API" },
  { id: "backend", name: "後端" },
  { id: "data", name: "資料存取" },
  { id: "other", name: "其他" },
];

export function moduleIdOf(path: string): string {
  const segs = path.split("/");
  if (segs.length === 1) return "(root)";
  return segs.slice(0, Math.min(2, segs.length - 1)).join("/");
}

const RULES: [string, RegExp][] = [
  ["data", /(^|\/)(db|database|models?|repo|repositories|migrations?|schema|store|dao|orm)(\/|$)/i],
  ["transport", /(^|\/)(ipc|api|routes?|router|controllers?|rpc|graphql)(\/|$)/i],
  ["frontend", /(^|\/)(components?|pages?|views?|ui|web|client|frontend|app|hooks|styles)(\/|$)|^src$/i],
  ["backend", /(^|\/)(server|backend|services?|commands|src-tauri|core|handlers?)(\/|$)/i],
];

export function guessLayer(moduleId: string): string {
  for (const [layer, re] of RULES) if (re.test(moduleId)) return layer;
  return "other";
}

/** 只用掃描結果產生的預設圖：分層靠路徑關鍵字猜，連線全是實線 import。 */
export function buildBaseGraph(scan: ScanResult): Architecture {
  const byModule = new Map<string, string[]>();
  for (const f of scan.files) {
    const id = moduleIdOf(f.path);
    (byModule.get(id) ?? byModule.set(id, []).get(id)!).push(f.path);
  }
  const modules: Module[] = [...byModule.entries()].map(([id, files]) => ({
    id, layer: guessLayer(id), name: id, files, tech: [], summary: "", dataflow: "",
  }));
  const seen = new Set<string>();
  const edges: Edge[] = [];
  for (const e of scan.edges) {
    const from = moduleIdOf(e.from);
    const to = moduleIdOf(e.to);
    const key = `${from}→${to}`;
    if (from === to || seen.has(key)) continue;
    seen.add(key);
    edges.push({ from, to, kind: "import", source: "scan", confidence: 1 });
  }
  const used = new Set(modules.map((m) => m.layer));
  return {
    layers: BASE_LAYERS.filter((l) => used.has(l.id)),
    modules,
    edges,
    meta: { scannedAt: new Date().toISOString(), gitHead: scan.git_head, analyzed: false, dropped: 0 },
  };
}
```

- [ ] **Step 5: 確認通過**

Run: `npx vitest run src/components/ArchitectureView/model.test.ts`
Expected: 3 suites 全 PASS。

- [ ] **Step 6: Commit**

```bash
git add src/ipc/architecture.ts src/components/ArchitectureView/model.ts src/components/ArchitectureView/model.test.ts
git commit -m "feat(architecture): 前端 IPC 與預設分層模型

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 7: AI 分析——提示、挑檔、驗證

**Files:**
- Create: `src/components/ArchitectureView/analyze.ts`, `src/components/ArchitectureView/analyze.test.ts`

驗證規則（全部要有測試）：模組引用不存在的 layer → 丟；模組展開後沒有任何真實檔案 → 丟；連線端點不是保留下來的模組 → 丟；連線 evidence 的檔案不在掃描結果、或 line 超出該檔行數或 <1 → 丟；confidence 夾在 0..1；kind 不在白名單 → 丟。丟棄數累計到 `meta.dropped`。掃描確認的 import 連線與 AI 的 `import` 連線重複時只留 scan 那條。

- [ ] **Step 1: 寫失敗測試**

```ts
// src/components/ArchitectureView/analyze.test.ts
import { describe, expect, it } from "vitest";
import { parseAnalysis, pickKeyFiles, buildAnalyzeMessage } from "./analyze";
import { buildBaseGraph } from "./model";
import type { ScanResult } from "../../ipc/architecture";

const scan: ScanResult = {
  root: "/p",
  files: [
    { path: "web/App.tsx", lang: "typescript", lines: 50 },
    { path: "web/api.ts", lang: "typescript", lines: 20 },
    { path: "server/routes.py", lang: "python", lines: 80 },
    { path: "server/db.py", lang: "python", lines: 30 },
  ],
  edges: [{ from: "web/App.tsx", to: "web/api.ts" }],
  deps: ["react", "flask"],
  truncated: false,
  git_head: "abc",
};

const good = {
  layers: [{ id: "frontend", name: "前端" }, { id: "backend", name: "後端" }],
  modules: [
    { id: "web", layer: "frontend", name: "網頁", files: ["web/"], tech: ["React"], summary: "UI", dataflow: "呼叫 /api" },
    { id: "server", layer: "backend", name: "服務", files: ["server/routes.py", "server/db.py"], tech: ["Flask"], summary: "API", dataflow: "讀 DB" },
  ],
  edges: [
    { from: "web", to: "server", kind: "http", confidence: 0.9, evidence: { file: "web/api.ts", line: 5 }, note: "fetch" },
  ],
};

describe("parseAnalysis", () => {
  it("接受合法回應並展開目錄前綴為真實檔案", () => {
    const r = parseAnalysis("```json\n" + JSON.stringify(good) + "\n```", scan)!;
    expect(r.modules.find((m) => m.id === "web")!.files.sort()).toEqual(["web/App.tsx", "web/api.ts"]);
    expect(r.edges).toHaveLength(1);
    expect(r.edges[0]).toMatchObject({ source: "ai", kind: "http", confidence: 0.9 });
    expect(r.meta.analyzed).toBe(true);
    expect(r.meta.dropped).toBe(0);
  });

  it("丟掉幻覺：不存在的檔案、超出行數、未知模組、未知 kind、未知 layer", () => {
    const bad = structuredClone(good);
    bad.modules.push({ id: "ghost", layer: "frontend", name: "g", files: ["nope/x.ts"], tech: [], summary: "", dataflow: "" });
    bad.modules.push({ id: "m2", layer: "nolayer", name: "g", files: ["web/App.tsx"], tech: [], summary: "", dataflow: "" });
    bad.edges.push(
      { from: "web", to: "server", kind: "http", confidence: 0.5, evidence: { file: "web/api.ts", line: 999 }, note: "" },
      { from: "web", to: "server", kind: "http", confidence: 0.5, evidence: { file: "web/ghost.ts", line: 1 }, note: "" },
      { from: "web", to: "ghost", kind: "http", confidence: 0.5, evidence: { file: "web/api.ts", line: 1 }, note: "" },
      { from: "web", to: "server", kind: "carrier-pigeon", confidence: 0.5, evidence: { file: "web/api.ts", line: 1 }, note: "" },
    );
    const r = parseAnalysis(JSON.stringify(bad), scan)!;
    expect(r.modules.map((m) => m.id).sort()).toEqual(["server", "web"]);
    expect(r.edges).toHaveLength(1);
    expect(r.meta.dropped).toBe(6);
  });

  it("confidence 夾在 0..1", () => {
    const g = structuredClone(good);
    g.edges[0].confidence = 7;
    expect(parseAnalysis(JSON.stringify(g), scan)!.edges[0].confidence).toBe(1);
  });

  it("不是 JSON 時回傳 null", () => {
    expect(parseAnalysis("抱歉我做不到", scan)).toBeNull();
  });
});

describe("pickKeyFiles", () => {
  it("優先挑入口／路由／資料庫檔案，最多 12 個", () => {
    const files = Array.from({ length: 40 }, (_, i) => ({ path: `pkg/f${i}.ts`, lang: "typescript", lines: 5 }));
    files.push({ path: "server/routes.py", lang: "python", lines: 5 }, { path: "db/schema.sql", lang: "sql", lines: 5 });
    const picked = pickKeyFiles({ ...scan, files });
    expect(picked.length).toBeLessThanOrEqual(12);
    expect(picked).toContain("server/routes.py");
    expect(picked).toContain("db/schema.sql");
  });
});

describe("buildAnalyzeMessage", () => {
  it("包含依賴、模組概況與關鍵檔案內容", () => {
    const msg = buildAnalyzeMessage(scan, buildBaseGraph(scan), [["server/routes.py", "@app.route('/x')"]]);
    expect(msg).toContain("flask");
    expect(msg).toContain("server/routes.py");
    expect(msg).toContain("@app.route('/x')");
  });
});
```

注意 dropped 計算：`ghost`（無檔案）1、`m2`（未知 layer）1、四條壞連線 4 = 6。

- [ ] **Step 2: 確認失敗**

Run: `npx vitest run src/components/ArchitectureView/analyze.test.ts`
Expected: FAIL（找不到 `./analyze`）。

- [ ] **Step 3: 實作**

```ts
// src/components/ArchitectureView/analyze.ts
import { stripFence } from "../TaskBoard/refinePrompts";
import type { ScanResult } from "../../ipc/architecture";
import type { Architecture, Edge, EdgeKind, Module } from "./model";

export const ANALYZE_SYSTEM = [
  "你是軟體架構分析師。根據專案的檔案概況、依賴清單與關鍵檔案內容，把專案切成「前端 → 傳輸（IPC／API）→ 後端 → 資料存取」等層，並找出跨層呼叫。",
  "只輸出一個 JSON 物件，不要程式碼區塊、不要說明。格式：",
  '{"layers":[{"id":"frontend","name":"前端"}],',
  ' "modules":[{"id":"短英文id","layer":"layers 裡的 id","name":"顯示名稱","files":["檔案路徑或以 / 結尾的目錄"],"tech":["技術或框架"],"summary":"一句話用途","dataflow":"資料如何進出，2-3 句"}],',
  ' "edges":[{"from":"模組id","to":"模組id","kind":"ipc|http|sql|import","confidence":0.0到1.0,"evidence":{"file":"檔案路徑","line":行號},"note":"這條連線在做什麼"}]}',
  "規則：",
  "- files 只能使用我提供的檔案路徑或其目錄，不可虛構。",
  "- 每條 edge 必須附 evidence（真實檔案與行號）；沒有把握就不要寫。",
  "- confidence 反映你對這條連線的把握。",
  "- 用繁體中文寫 name／summary／dataflow／note。",
].join("\n");

const KINDS: EdgeKind[] = ["import", "ipc", "http", "sql"];
const KEY_RE = /(main|index|app|server|routes?|router|controller|command|handler|api|db|database|schema|model|store|lib)\b/i;

export function pickKeyFiles(scan: ScanResult, max = 12): string[] {
  const scored = scan.files
    .map((f) => ({ path: f.path, score: (KEY_RE.test(f.path.split("/").pop() ?? "") ? 2 : 0) + (f.lang === "sql" ? 2 : 0) + (f.path.split("/").length <= 2 ? 1 : 0) }))
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score || a.path.localeCompare(b.path));
  return scored.slice(0, max).map((x) => x.path);
}

export function buildAnalyzeMessage(scan: ScanResult, base: Architecture, files: [string, string][]): string {
  const mods = base.modules
    .map((m) => `- ${m.id}（${m.files.length} 檔）：${m.files.slice(0, 4).join(", ")}${m.files.length > 4 ? " …" : ""}`)
    .join("\n");
  const imports = base.edges.map((e) => `${e.from} → ${e.to}`).join("\n") || "（無）";
  const body = files.map(([p, c]) => `### ${p}\n${c}`).join("\n\n");
  return [
    `依賴：${scan.deps.join(", ") || "（無）"}`,
    `檔案概況（目錄分組）：\n${mods}`,
    `已確認的 import 關係（模組層級）：\n${imports}`,
    `關鍵檔案內容：\n${body}`,
  ].join("\n\n");
}

interface RawAnalysis {
  layers?: { id?: unknown; name?: unknown }[];
  modules?: Record<string, unknown>[];
  edges?: Record<string, unknown>[];
}

export function parseAnalysis(reply: string, scan: ScanResult): Architecture | null {
  let raw: RawAnalysis;
  try {
    raw = JSON.parse(stripFence(reply.trim()).trim()) as RawAnalysis;
  } catch {
    return null;
  }
  if (!raw || typeof raw !== "object" || !Array.isArray(raw.modules)) return null;

  let dropped = 0;
  const lineCount = new Map(scan.files.map((f) => [f.path, f.lines]));
  const layers = (raw.layers ?? [])
    .filter((l) => typeof l.id === "string" && typeof l.name === "string")
    .map((l) => ({ id: l.id as string, name: l.name as string }));
  const layerIds = new Set(layers.map((l) => l.id));

  const modules: Module[] = [];
  for (const m of raw.modules) {
    const id = typeof m.id === "string" ? m.id : "";
    const layer = typeof m.layer === "string" ? m.layer : "";
    const patterns = Array.isArray(m.files) ? m.files.filter((x): x is string => typeof x === "string") : [];
    const files = scan.files
      .map((f) => f.path)
      .filter((p) => patterns.some((pat) => (pat.endsWith("/") ? p.startsWith(pat) : p === pat)));
    if (!id || !layerIds.has(layer) || files.length === 0) { dropped++; continue; }
    modules.push({
      id, layer, files,
      name: typeof m.name === "string" ? m.name : id,
      tech: Array.isArray(m.tech) ? m.tech.filter((x): x is string => typeof x === "string") : [],
      summary: typeof m.summary === "string" ? m.summary : "",
      dataflow: typeof m.dataflow === "string" ? m.dataflow : "",
    });
  }
  const moduleIds = new Set(modules.map((m) => m.id));

  const edges: Edge[] = [];
  for (const e of raw.edges ?? []) {
    const ev = e.evidence as { file?: unknown; line?: unknown } | undefined;
    const kind = e.kind as EdgeKind;
    const total = typeof ev?.file === "string" ? lineCount.get(ev.file) : undefined;
    const ok =
      typeof e.from === "string" && typeof e.to === "string" &&
      moduleIds.has(e.from) && moduleIds.has(e.to) &&
      KINDS.includes(kind) &&
      total !== undefined && typeof ev?.line === "number" && ev.line >= 1 && ev.line <= total;
    if (!ok) { dropped++; continue; }
    const c = typeof e.confidence === "number" ? e.confidence : 0.5;
    edges.push({
      from: e.from as string, to: e.to as string, kind, source: "ai",
      confidence: Math.min(1, Math.max(0, c)),
      evidence: { file: ev!.file as string, line: ev!.line as number },
      note: typeof e.note === "string" ? e.note : undefined,
    });
  }

  return {
    layers, modules, edges,
    meta: { scannedAt: new Date().toISOString(), gitHead: scan.git_head, analyzed: true, dropped },
  };
}

/** 把掃描確認的檔案層級 import 映射成 AI 模組間的實線；模組內部的不算。 */
export function mergeScanEdges(ai: Architecture, scan: ScanResult): Architecture {
  const fileToModule = new Map<string, string>();
  for (const m of ai.modules) for (const f of m.files) fileToModule.set(f, m.id);
  const seen = new Set<string>();
  const scanEdges: Edge[] = [];
  for (const e of scan.edges) {
    const from = fileToModule.get(e.from);
    const to = fileToModule.get(e.to);
    if (!from || !to || from === to) continue;
    const key = `${from}→${to}`;
    if (seen.has(key)) continue;
    seen.add(key);
    scanEdges.push({ from, to, kind: "import", source: "scan", confidence: 1 });
  }
  return { ...ai, edges: [...scanEdges, ...ai.edges] };
}
```

- [ ] **Step 4: 確認通過**

Run: `npx vitest run src/components/ArchitectureView/analyze.test.ts`
Expected: parseAnalysis／pickKeyFiles／buildAnalyzeMessage 全 PASS。

- [ ] **Step 5: 為 mergeScanEdges 補測試**

在 `analyze.test.ts` 加：

```ts
import { mergeScanEdges } from "./analyze";

describe("mergeScanEdges", () => {
  it("把檔案層級 import 映射成 AI 模組間的實線，模組內部的不算", () => {
    const ai = parseAnalysis(JSON.stringify(good), scan)!;
    const cross = { ...scan, edges: [{ from: "web/api.ts", to: "server/routes.py" }, { from: "web/App.tsx", to: "web/api.ts" }] };
    const merged = mergeScanEdges(ai, cross);
    const s = merged.edges.filter((e) => e.source === "scan");
    expect(s).toEqual([{ from: "web", to: "server", kind: "import", source: "scan", confidence: 1 }]);
    expect(merged.edges.filter((e) => e.source === "ai")).toHaveLength(1);
  });
});
```

Run: `npx vitest run src/components/ArchitectureView/analyze.test.ts`
Expected: 全 PASS。

- [ ] **Step 6: Commit**

```bash
git add src/components/ArchitectureView/analyze.ts src/components/ArchitectureView/analyze.test.ts
git commit -m "feat(architecture): AI 分析提示、關鍵檔挑選與幻覺驗證

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 8: 分層座標與連線路徑

**Files:**
- Create: `src/components/ArchitectureView/layout.ts`, `src/components/ArchitectureView/layout.test.ts`

- [ ] **Step 1: 寫失敗測試**

```ts
// src/components/ArchitectureView/layout.test.ts
import { describe, expect, it } from "vitest";
import { layoutGraph, edgePath, traceChain, NODE_W, NODE_H, COL_GAP, ROW_GAP, PAD } from "./layout";
import type { Architecture } from "./model";

const arch: Architecture = {
  layers: [{ id: "a", name: "A" }, { id: "b", name: "B" }],
  modules: [
    { id: "m1", layer: "a", name: "m1", files: [], tech: [], summary: "", dataflow: "" },
    { id: "m2", layer: "a", name: "m2", files: [], tech: [], summary: "", dataflow: "" },
    { id: "m3", layer: "b", name: "m3", files: [], tech: [], summary: "", dataflow: "" },
  ],
  edges: [
    { from: "m1", to: "m3", kind: "http", source: "ai", confidence: 0.8 },
    { from: "m2", to: "m3", kind: "import", source: "scan", confidence: 1 },
  ],
  meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
};

describe("layoutGraph", () => {
  it("每層一欄、層內由上往下排列", () => {
    const l = layoutGraph(arch);
    expect(l.nodes.m1).toEqual({ x: PAD, y: PAD + 40 });
    expect(l.nodes.m2).toEqual({ x: PAD, y: PAD + 40 + NODE_H + ROW_GAP });
    expect(l.nodes.m3.x).toBe(PAD + NODE_W + COL_GAP);
    expect(l.columns).toHaveLength(2);
    expect(l.width).toBeGreaterThan(l.nodes.m3.x + NODE_W);
    expect(l.height).toBeGreaterThan(l.nodes.m2.y + NODE_H);
  });
});

describe("edgePath", () => {
  it("從來源右緣到目標左緣畫三次貝茲曲線", () => {
    const l = layoutGraph(arch);
    const d = edgePath(l.nodes.m1, l.nodes.m3);
    expect(d.startsWith(`M ${l.nodes.m1.x + NODE_W} ${l.nodes.m1.y + NODE_H / 2} C`)).toBe(true);
    expect(d.endsWith(`${l.nodes.m3.x} ${l.nodes.m3.y + NODE_H / 2}`)).toBe(true);
  });
});

describe("traceChain", () => {
  it("從選中的模組往上下游走完整條呼叫鏈", () => {
    const chain = traceChain(arch, "m3");
    expect([...chain.modules].sort()).toEqual(["m1", "m2", "m3"]);
    expect(chain.edges).toHaveLength(2);
    expect([...traceChain(arch, "m1").modules].sort()).toEqual(["m1", "m3"]);
  });
});
```

- [ ] **Step 2: 確認失敗**

Run: `npx vitest run src/components/ArchitectureView/layout.test.ts`
Expected: FAIL（找不到 `./layout`）。

- [ ] **Step 3: 實作**

```ts
// src/components/ArchitectureView/layout.ts
import type { Architecture, Edge } from "./model";

export const NODE_W = 180;
export const NODE_H = 56;
export const COL_GAP = 120;
export const ROW_GAP = 20;
export const PAD = 24;
const HEADER_H = 40;

export interface Pt { x: number; y: number }
export interface Layout {
  nodes: Record<string, Pt>;
  columns: { layerId: string; name: string; x: number }[];
  width: number;
  height: number;
}

export function layoutGraph(arch: Architecture): Layout {
  const nodes: Record<string, Pt> = {};
  const columns: Layout["columns"] = [];
  let maxRows = 0;
  arch.layers.forEach((layer, col) => {
    const x = PAD + col * (NODE_W + COL_GAP);
    columns.push({ layerId: layer.id, name: layer.name, x });
    const mods = arch.modules.filter((m) => m.layer === layer.id);
    maxRows = Math.max(maxRows, mods.length);
    mods.forEach((m, row) => {
      nodes[m.id] = { x, y: PAD + HEADER_H + row * (NODE_H + ROW_GAP) };
    });
  });
  return {
    nodes,
    columns,
    width: PAD * 2 + arch.layers.length * NODE_W + Math.max(0, arch.layers.length - 1) * COL_GAP,
    height: PAD * 2 + HEADER_H + maxRows * NODE_H + Math.max(0, maxRows - 1) * ROW_GAP,
  };
}

/** 來源右緣中點 → 目標左緣中點；目標在左邊時也畫得出可讀的曲線。 */
export function edgePath(from: Pt, to: Pt): string {
  const x1 = from.x + NODE_W;
  const y1 = from.y + NODE_H / 2;
  const x2 = to.x;
  const y2 = to.y + NODE_H / 2;
  const dx = Math.max(40, Math.abs(x2 - x1) / 2);
  return `M ${x1} ${y1} C ${x1 + dx} ${y1} ${x2 - dx} ${y2} ${x2} ${y2}`;
}

/** 從某模組出發，往上游與下游各走到底，回傳整條呼叫鏈上的模組與連線。 */
export function traceChain(arch: Architecture, start: string): { modules: Set<string>; edges: Edge[] } {
  const modules = new Set<string>([start]);
  const edges = new Set<Edge>();
  const walk = (dir: "down" | "up") => {
    const queue = [start];
    const visited = new Set<string>([start]);
    while (queue.length) {
      const cur = queue.shift()!;
      for (const e of arch.edges) {
        const [near, far] = dir === "down" ? [e.from, e.to] : [e.to, e.from];
        if (near !== cur) continue;
        edges.add(e);
        modules.add(far);
        if (!visited.has(far)) { visited.add(far); queue.push(far); }
      }
    }
  };
  walk("down");
  walk("up");
  return { modules, edges: [...edges] };
}
```

- [ ] **Step 4: 確認通過**

Run: `npx vitest run src/components/ArchitectureView/layout.test.ts`
Expected: 3 suites PASS。

- [ ] **Step 5: Commit**

```bash
git add src/components/ArchitectureView/layout.ts src/components/ArchitectureView/layout.test.ts
git commit -m "feat(architecture): 分層座標、連線曲線與呼叫鏈追蹤

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 9: i18n 字串與 spec 同步

**Files:**
- Modify: `src/lib/i18n.ts`, `docs/superpowers/specs/2026-10-22-architecture-explorer-design.md`

- [ ] **Step 1: 加字串**

`src/lib/i18n.ts` 有兩處語系區塊（zh-TW 約第 524 行 `agent_canvas_tab` 旁；en 約第 3021 行旁）。**en 是 `{...zhTW,...enRaw}` 合併，必須兩邊都補**（見記憶：i18n 語系漂移）。在兩處 `new_agent_canvas_desc` 之後各加：

zh-TW：
```ts
    arch_tab: "架構導覽",
    new_arch_desc: "選一個專案資料夾，自動畫出前端、後端、資料層的關係，點模組看資料如何流動",
    arch_pick_folder: "選擇專案資料夾",
    arch_rescan: "重新掃描",
    arch_analyze: "AI 分析",
    arch_analyzing: "AI 分析中…",
    arch_scanning: "掃描中…",
    arch_stale: "架構可能已過期（專案有新的提交），重新分析？",
    arch_truncated: "專案檔案很多，只掃描了一部分",
    arch_dropped: "已丟棄 {n} 筆無法驗證的 AI 推論",
    arch_no_ai: "尚未設定 AI，目前只顯示掃描到的 import 關係",
    arch_legend_scan: "實線：掃描確認",
    arch_legend_ai: "虛線：AI 推論",
    arch_detail_summary: "用途",
    arch_detail_tech: "技術",
    arch_detail_dataflow: "資料流",
    arch_detail_upstream: "呼叫它的模組",
    arch_detail_downstream: "它呼叫的模組",
    arch_detail_files: "檔案",
    arch_detail_empty: "點選任一模組查看詳情",
    arch_edge_evidence: "佐證",
    arch_confidence: "信心度",
    arch_error: "分析失敗：{msg}",
```

en：同一組 key，英文值：`"Architecture"`, `"Pick a project folder to map its frontend, backend and data layers; click a module to see how data flows"`, `"Choose project folder"`, `"Rescan"`, `"Analyze with AI"`, `"Analyzing…"`, `"Scanning…"`, `"The architecture may be stale (new commits). Re-analyze?"`, `"Large project: only part of it was scanned"`, `"Dropped {n} unverifiable AI inferences"`, `"AI is not configured; showing scanned imports only"`, `"Solid: confirmed by scan"`, `"Dashed: inferred by AI"`, `"Purpose"`, `"Tech"`, `"Data flow"`, `"Called by"`, `"Calls"`, `"Files"`, `"Click a module to see details"`, `"Evidence"`, `"Confidence"`, `"Analysis failed: {msg}"`。

- [ ] **Step 2: 驗證 i18n 測試與型別**

Run: `npx tsc -b && npx vitest run src/lib`
Expected: 無型別錯誤；i18n 一致性測試 PASS（若有 locale drift 測試）。

- [ ] **Step 3: 更新 spec 的 analyzer 位置**

在 spec 的「後端（Rust）」把 `analyzer` 那一行與 command 行改為：後端只有 `scanner`、`store`，commands 為 `arch_scan`／`arch_load`／`arch_save`／`arch_read_files`；並新增一段「AI 分析在前端（沿用 `ai_complete`），回應以掃描結果驗證」。

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts docs/superpowers/specs/2026-10-22-architecture-explorer-design.md
git commit -m "feat(architecture): i18n 字串並同步 spec 的 analyzer 位置

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 10: 詳情面板元件

**Files:**
- Create: `src/components/ArchitectureView/DetailPanel.tsx`, `src/components/ArchitectureView/DetailPanel.test.tsx`

- [ ] **Step 1: 寫失敗測試**

```tsx
// src/components/ArchitectureView/DetailPanel.test.tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DetailPanel } from "./DetailPanel";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { Architecture } from "./model";

const arch: Architecture = {
  layers: [{ id: "a", name: "前端" }, { id: "b", name: "後端" }],
  modules: [
    { id: "web", layer: "a", name: "網頁", files: ["web/App.tsx"], tech: ["React"], summary: "畫面", dataflow: "呼叫 /api" },
    { id: "srv", layer: "b", name: "服務", files: ["srv/r.py"], tech: ["Flask"], summary: "API", dataflow: "讀 DB" },
  ],
  edges: [{ from: "web", to: "srv", kind: "http", source: "ai", confidence: 0.8, evidence: { file: "web/api.ts", line: 5 }, note: "fetch" }],
  meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
};

function setup(selected: string | null, onSelect = vi.fn()) {
  render(<LocaleProvider><DetailPanel arch={arch} selected={selected} onSelect={onSelect} /></LocaleProvider>);
  return onSelect;
}

describe("DetailPanel", () => {
  it("未選取時顯示提示", () => {
    setup(null);
    expect(screen.getByText(/點選任一模組|Click a module/)).toBeInTheDocument();
  });

  it("顯示技術、資料流、下游模組與佐證，點下游可跳轉", () => {
    const onSelect = setup("web");
    expect(screen.getByText("React")).toBeInTheDocument();
    expect(screen.getByText("呼叫 /api")).toBeInTheDocument();
    expect(screen.getByText(/web\/api\.ts:5/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "服務" }));
    expect(onSelect).toHaveBeenCalledWith("srv");
  });
});
```

（若 `LocaleProvider` 的匯出名稱不同，以 `src/contexts/LocaleContext.tsx` 實際匯出為準，並參考 `AgentCanvas/index.test.tsx` 的包法。）

- [ ] **Step 2: 確認失敗**

Run: `npx vitest run src/components/ArchitectureView/DetailPanel.test.tsx`
Expected: FAIL（找不到 `./DetailPanel`）。

- [ ] **Step 3: 實作**

```tsx
// src/components/ArchitectureView/DetailPanel.tsx
import { useLocale } from "../../contexts/LocaleContext";
import type { Architecture } from "./model";

interface Props {
  arch: Architecture;
  selected: string | null;
  onSelect: (id: string) => void;
}

export function DetailPanel({ arch, selected, onSelect }: Props) {
  const { t } = useLocale();
  const mod = arch.modules.find((m) => m.id === selected);
  if (!mod) return <aside className="arch-detail arch-detail-empty">{t.arch_detail_empty}</aside>;

  const nameOf = (id: string) => arch.modules.find((m) => m.id === id)?.name ?? id;
  const up = arch.edges.filter((e) => e.to === mod.id);
  const down = arch.edges.filter((e) => e.from === mod.id);
  const layerName = arch.layers.find((l) => l.id === mod.layer)?.name ?? mod.layer;

  return (
    <aside className="arch-detail">
      <h3>{mod.name}</h3>
      <div className="arch-detail-layer">{layerName}</div>
      {mod.summary && <section><h4>{t.arch_detail_summary}</h4><p>{mod.summary}</p></section>}
      {mod.tech.length > 0 && (
        <section><h4>{t.arch_detail_tech}</h4>
          <div className="arch-chips">{mod.tech.map((x) => <span key={x} className="arch-chip">{x}</span>)}</div>
        </section>
      )}
      {mod.dataflow && <section><h4>{t.arch_detail_dataflow}</h4><p>{mod.dataflow}</p></section>}
      {([[t.arch_detail_upstream, up, "from"], [t.arch_detail_downstream, down, "to"]] as const).map(([title, list, side]) =>
        list.length > 0 && (
          <section key={title}><h4>{title}</h4>
            <ul>
              {list.map((e, i) => (
                <li key={`${e.from}-${e.to}-${i}`}>
                  <button type="button" onClick={() => onSelect(e[side])}>{nameOf(e[side])}</button>
                  <span className={`arch-kind arch-kind-${e.source}`}>{e.kind}</span>
                  {e.source === "ai" && <span className="arch-conf">{t.arch_confidence} {Math.round(e.confidence * 100)}%</span>}
                  {e.note && <div className="arch-note">{e.note}</div>}
                  {e.evidence && <code className="arch-evidence">{t.arch_edge_evidence}：{e.evidence.file}:{e.evidence.line}</code>}
                </li>
              ))}
            </ul>
          </section>
        ),
      )}
      <section><h4>{t.arch_detail_files} ({mod.files.length})</h4>
        <ul className="arch-files">{mod.files.slice(0, 50).map((f) => <li key={f}><code>{f}</code></li>)}</ul>
      </section>
    </aside>
  );
}
```

- [ ] **Step 4: 確認通過**

Run: `npx vitest run src/components/ArchitectureView/DetailPanel.test.tsx`
Expected: 2 PASS。

- [ ] **Step 5: Commit**

```bash
git add src/components/ArchitectureView/DetailPanel.tsx src/components/ArchitectureView/DetailPanel.test.tsx
git commit -m "feat(architecture): 模組詳情面板

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 11: 分頁主體（掃描→分析→圖）

**Files:**
- Create: `src/components/ArchitectureView/index.tsx`, `index.css`, `index.test.tsx`

流程：選資料夾（`open({ directory: true })` from `@tauri-apps/plugin-dialog`）→ `archLoad` 有快取就載入並比對 `git_head` → 無快取則 `archScan` + `buildBaseGraph` → 「AI 分析」按鈕：`pickKeyFiles` → `archReadFiles` → `invokeAiComplete([{role:"user",content}], ANALYZE_SYSTEM, nanoid())` → `parseAnalysis` → `mergeScanEdges` → `archSave`。AI 錯誤（`not_configured` 等）時保留掃描圖並顯示 `arch_no_ai` 或 `formatAiError`。`isActive` 為 false 時不做任何事（分頁隱藏不卸載）。

- [ ] **Step 1: 寫失敗測試**

```tsx
// src/components/ArchitectureView/index.test.tsx
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LocaleProvider } from "../../contexts/LocaleContext";

const open = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => open(...a) }));

const archScan = vi.fn();
const archLoad = vi.fn();
const archSave = vi.fn();
const archReadFiles = vi.fn();
vi.mock("../../ipc/architecture", () => ({
  archScan: (...a: unknown[]) => archScan(...a),
  archLoad: (...a: unknown[]) => archLoad(...a),
  archSave: (...a: unknown[]) => archSave(...a),
  archReadFiles: (...a: unknown[]) => archReadFiles(...a),
}));

const invokeAiComplete = vi.fn();
vi.mock("../../ipc/ai", async (orig) => ({
  ...(await orig<typeof import("../../ipc/ai")>()),
  invokeAiComplete: (...a: unknown[]) => invokeAiComplete(...a),
}));

import { ArchitectureView } from "./index";

const SCAN = {
  root: "/p",
  files: [
    { path: "web/App.tsx", lang: "typescript", lines: 50 },
    { path: "server/routes.py", lang: "python", lines: 80 },
  ],
  edges: [],
  deps: [],
  truncated: false,
  git_head: "h1",
};

beforeEach(() => {
  vi.clearAllMocks();
  open.mockResolvedValue("/p");
  archLoad.mockResolvedValue(null);
  archScan.mockResolvedValue(SCAN);
  archReadFiles.mockResolvedValue([]);
});

const renderView = () => render(<LocaleProvider><ArchitectureView isActive /></LocaleProvider>);

describe("ArchitectureView", () => {
  it("選資料夾後掃描並顯示模組方塊；點方塊開詳情", async () => {
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    const web = await screen.findByRole("button", { name: /web/ });
    fireEvent.click(web);
    expect(await screen.findByRole("heading", { name: "web" })).toBeInTheDocument();
  });

  it("AI 分析成功後以分析結果取代預設分層並存檔", async () => {
    invokeAiComplete.mockResolvedValue({
      content: JSON.stringify({
        layers: [{ id: "frontend", name: "前端" }, { id: "backend", name: "後端" }],
        modules: [
          { id: "ui", layer: "frontend", name: "介面", files: ["web/"], tech: ["React"], summary: "s", dataflow: "d" },
          { id: "api", layer: "backend", name: "介面服務", files: ["server/"], tech: [], summary: "", dataflow: "" },
        ],
        edges: [],
      }),
      tool_calls: [], tool_calling_unsupported: false,
    });
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: /web/ });
    fireEvent.click(screen.getByRole("button", { name: /AI 分析|Analyze with AI/ }));
    expect(await screen.findByRole("button", { name: "介面" })).toBeInTheDocument();
    await waitFor(() => expect(archSave).toHaveBeenCalledTimes(1));
  });

  it("AI 未設定時保留掃描圖並提示", async () => {
    invokeAiComplete.mockRejectedValue({ kind: "not_configured" });
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: /web/ });
    fireEvent.click(screen.getByRole("button", { name: /AI 分析|Analyze with AI/ }));
    expect(await screen.findByText(/尚未設定 AI|AI is not configured/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /web/ })).toBeInTheDocument();
  });

  it("快取的 git_head 與目前不同時顯示過期提示", async () => {
    archLoad.mockResolvedValue(JSON.stringify({
      layers: [{ id: "frontend", name: "前端" }],
      modules: [{ id: "ui", layer: "frontend", name: "介面", files: ["web/App.tsx"], tech: [], summary: "", dataflow: "" }],
      edges: [], meta: { scannedAt: "", gitHead: "OLD", analyzed: true, dropped: 0 },
    }));
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    expect(await screen.findByText(/可能已過期|may be stale/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 確認失敗**

Run: `npx vitest run src/components/ArchitectureView/index.test.tsx`
Expected: FAIL（找不到 `./index`）。

- [ ] **Step 3: 實作**

```tsx
// src/components/ArchitectureView/index.tsx
import { useCallback, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { nanoid } from "nanoid";
import { useLocale } from "../../contexts/LocaleContext";
import { archLoad, archReadFiles, archSave, archScan, type ScanResult } from "../../ipc/architecture";
import { formatAiError, invokeAiComplete, type AiError } from "../../ipc/ai";
import { ANALYZE_SYSTEM, buildAnalyzeMessage, mergeScanEdges, parseAnalysis, pickKeyFiles } from "./analyze";
import { buildBaseGraph, type Architecture } from "./model";
import { edgePath, layoutGraph, NODE_H, NODE_W, traceChain } from "./layout";
import { DetailPanel } from "./DetailPanel";
import "./index.css";

type Status = { kind: "idle" } | { kind: "busy"; msg: string } | { kind: "error"; msg: string } | { kind: "notice"; msg: string };

export function ArchitectureView({ isActive }: { isActive: boolean }) {
  const { t } = useLocale();
  const [root, setRoot] = useState<string | null>(null);
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [arch, setArch] = useState<Architecture | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [stale, setStale] = useState(false);
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  const load = useCallback(async (dir: string) => {
    setRoot(dir);
    setSelected(null);
    setStatus({ kind: "busy", msg: t.arch_scanning });
    try {
      const s = await archScan(dir);
      setScan(s);
      const cached = await archLoad(dir);
      if (cached) {
        const parsed = JSON.parse(cached) as Architecture;
        setArch(parsed);
        setStale(parsed.meta.gitHead !== s.git_head);
      } else {
        setArch(buildBaseGraph(s));
        setStale(false);
      }
      setStatus(s.truncated ? { kind: "notice", msg: t.arch_truncated } : { kind: "idle" });
    } catch (e) {
      setStatus({ kind: "error", msg: t.arch_error.replace("{msg}", String(e)) });
    }
  }, [t]);

  const pick = useCallback(async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") await load(dir);
  }, [load]);

  const analyze = useCallback(async () => {
    if (!root || !scan) return;
    setStatus({ kind: "busy", msg: t.arch_analyzing });
    try {
      const base = buildBaseGraph(scan);
      const files = await archReadFiles(root, pickKeyFiles(scan));
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildAnalyzeMessage(scan, base, files) }],
        ANALYZE_SYSTEM,
        nanoid(),
      );
      const parsed = reply.content ? parseAnalysis(reply.content, scan) : null;
      if (!parsed) {
        setStatus({ kind: "error", msg: t.arch_error.replace("{msg}", "AI 回應不是有效的架構 JSON") });
        return;
      }
      const merged = mergeScanEdges(parsed, scan);
      setArch(merged);
      setStale(false);
      await archSave(root, JSON.stringify(merged));
      setStatus(parsed.meta.dropped > 0 ? { kind: "notice", msg: t.arch_dropped.replace("{n}", String(parsed.meta.dropped)) } : { kind: "idle" });
    } catch (e) {
      const err = e as AiError;
      setStatus(err?.kind === "not_configured"
        ? { kind: "notice", msg: t.arch_no_ai }
        : { kind: "error", msg: t.arch_error.replace("{msg}", err?.kind ? formatAiError(err) : String(e)) });
    }
  }, [root, scan, t]);

  const layout = useMemo(() => (arch ? layoutGraph(arch) : null), [arch]);
  const chain = useMemo(() => (arch && selected ? traceChain(arch, selected) : null), [arch, selected]);

  return (
    <div className="arch-view" style={{ display: isActive ? undefined : "none" }}>
      <header className="arch-toolbar">
        <button type="button" onClick={pick}>{t.arch_pick_folder}</button>
        {root && <span className="arch-root">{root}</span>}
        {root && <button type="button" onClick={() => load(root)}>{t.arch_rescan}</button>}
        {scan && <button type="button" onClick={analyze} disabled={status.kind === "busy"}>{t.arch_analyze}</button>}
        <span className="arch-legend"><i className="arch-line-solid" />{t.arch_legend_scan}<i className="arch-line-dash" />{t.arch_legend_ai}</span>
      </header>
      {status.kind !== "idle" && <div className={`arch-status arch-status-${status.kind}`}>{status.msg}</div>}
      {stale && <div className="arch-status arch-status-notice">{t.arch_stale}</div>}
      <div className="arch-body">
        <div className="arch-canvas">
          {arch && layout && (
            <div className="arch-stage" style={{ width: layout.width, height: layout.height }}>
              <svg width={layout.width} height={layout.height} className="arch-edges">
                {arch.edges.map((e, i) => {
                  const a = layout.nodes[e.from];
                  const b = layout.nodes[e.to];
                  if (!a || !b) return null;
                  const on = chain ? chain.edges.includes(e) : false;
                  return (
                    <path key={i} d={edgePath(a, b)} fill="none"
                      className={`arch-edge ${e.source === "ai" ? "arch-edge-ai" : ""} ${on ? "arch-edge-on" : ""} ${chain && !on ? "arch-edge-dim" : ""}`}>
                      <title>{e.note ?? e.kind}</title>
                    </path>
                  );
                })}
              </svg>
              {layout.columns.map((c) => (
                <div key={c.layerId} className="arch-col-title" style={{ left: c.x, width: NODE_W }}>{c.name}</div>
              ))}
              {arch.modules.map((m) => {
                const p = layout.nodes[m.id];
                if (!p) return null;
                const dim = chain && !chain.modules.has(m.id);
                return (
                  <button key={m.id} type="button" title={m.id}
                    className={`arch-node ${selected === m.id ? "arch-node-sel" : ""} ${dim ? "arch-node-dim" : ""}`}
                    style={{ left: p.x, top: p.y, width: NODE_W, height: NODE_H }}
                    onClick={() => setSelected(m.id)}>
                    {m.name}
                  </button>
                );
              })}
            </div>
          )}
        </div>
        {arch && <DetailPanel arch={arch} selected={selected} onSelect={setSelected} />}
      </div>
    </div>
  );
}
```

`index.css`（沿用專案 CSS 變數；實作時對照 `AgentCanvas/index.css` 取用現有 `--` 變數名稱）：

```css
.arch-view { display: flex; flex-direction: column; height: 100%; min-height: 0; }
.arch-toolbar { display: flex; align-items: center; gap: 8px; padding: 8px 12px; flex-wrap: wrap; }
.arch-root { opacity: 0.7; font-size: 12px; overflow: hidden; text-overflow: ellipsis; max-width: 40ch; }
.arch-legend { margin-left: auto; display: flex; align-items: center; gap: 6px; font-size: 12px; opacity: 0.8; }
.arch-line-solid, .arch-line-dash { display: inline-block; width: 24px; height: 0; border-top: 2px solid currentColor; }
.arch-line-dash { border-top-style: dashed; margin-left: 8px; }
.arch-status { padding: 6px 12px; font-size: 13px; }
.arch-status-error { color: #f87171; }
.arch-status-notice { color: #fbbf24; }
.arch-body { display: flex; flex: 1; min-height: 0; }
.arch-canvas { flex: 1; overflow: auto; }
.arch-stage { position: relative; }
.arch-edges { position: absolute; inset: 0; pointer-events: none; }
.arch-edge { stroke: currentColor; stroke-width: 1.5; opacity: 0.55; }
.arch-edge-ai { stroke-dasharray: 6 4; }
.arch-edge-on { opacity: 1; stroke-width: 2.5; stroke: #38bdf8; }
.arch-edge-dim { opacity: 0.12; }
.arch-col-title { position: absolute; top: 24px; text-align: center; font-weight: 600; font-size: 13px; opacity: 0.8; }
.arch-node { position: absolute; border: 1px solid rgba(128,128,128,.5); border-radius: 8px; background: rgba(128,128,128,.12); color: inherit; cursor: pointer; padding: 4px 8px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.arch-node:hover { border-color: #38bdf8; }
.arch-node-sel { border-color: #38bdf8; background: rgba(56,189,248,.18); }
.arch-node-dim { opacity: 0.3; }
.arch-detail { width: 320px; flex-shrink: 0; overflow: auto; padding: 12px 16px; border-left: 1px solid rgba(128,128,128,.3); font-size: 13px; }
.arch-detail-empty { opacity: 0.6; display: flex; align-items: center; justify-content: center; }
.arch-detail h3 { margin: 0 0 2px; }
.arch-detail h4 { margin: 12px 0 4px; font-size: 12px; opacity: 0.7; }
.arch-detail-layer { font-size: 12px; opacity: 0.6; }
.arch-chips { display: flex; flex-wrap: wrap; gap: 4px; }
.arch-chip, .arch-kind { padding: 1px 8px; border-radius: 10px; background: rgba(128,128,128,.2); font-size: 12px; }
.arch-kind { margin-left: 6px; }
.arch-conf { margin-left: 6px; font-size: 11px; opacity: 0.7; }
.arch-note { opacity: 0.8; margin-top: 2px; }
.arch-evidence { display: block; font-size: 11px; opacity: 0.7; word-break: break-all; }
.arch-detail ul { list-style: none; margin: 0; padding: 0; }
.arch-detail li { margin-bottom: 8px; }
.arch-detail li button { background: none; border: none; color: #38bdf8; cursor: pointer; padding: 0; font: inherit; }
.arch-files code { font-size: 11px; word-break: break-all; }
```

- [ ] **Step 4: 確認通過**

Run: `npx vitest run src/components/ArchitectureView`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add src/components/ArchitectureView/index.tsx src/components/ArchitectureView/index.css src/components/ArchitectureView/index.test.tsx
git commit -m "feat(architecture): 分頁主體（掃描、AI 分析、分層圖）

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 12: 分頁型別接線

**Files:**
- Modify: `src/components/TabBar/index.tsx:31,151`、`src/components/Icons.tsx`、`src/components/NewTabPicker/tabCatalog.tsx:89`、`src/components/NewTabPicker/tabCatalog.test.tsx:15`、`src/components/TerminalApp.tsx`（import、約 373 行 title 鏈、約 911 行 render 分支）、`src/components/tabTypeCoverage.test.ts`

先讀 `tabTypeCoverage.test.ts` 的 agent-canvas 區塊與 `TerminalApp.tsx` 其他會對 `TabType` 分支處：`grep -n "agent-canvas" -r src`，**每個出現 `"agent-canvas"` 的檔案都要檢查是否需要對 `"architecture"` 做同樣處理**（關閉確認、忙碌探針等；本分頁無長任務，預期只需 title／icon／render／catalog）。

- [ ] **Step 1: 先寫覆蓋測試（會紅）**

在 `tabTypeCoverage.test.ts` 末尾加：

```ts
describe("architecture tab type coverage", () => {
  it("is a member of the TabType union", () => {
    expect(tabBarSrc).toMatch(/export type TabType =[^;]*"architecture"/s);
  });

  it("has an icon in the tab bar", () => {
    const iconFn = tabBarSrc.slice(tabBarSrc.indexOf("function getTabIcon"));
    expect(iconFn).toContain("architecture");
  });

  it("gets a title and a render branch", () => {
    expect(terminalAppSrc).toContain("t.arch_tab");
    expect(terminalAppSrc).toContain('tab.type === "architecture"');
    expect(terminalAppSrc).toContain("ArchitectureView");
  });
});
```

並在 `tabCatalog.test.tsx` 的 `ALL_TYPES` 加 `"architecture"`。

Run: `npx vitest run src/components/tabTypeCoverage.test.ts src/components/NewTabPicker`
Expected: FAIL。

- [ ] **Step 2: 接線**

- `TabBar/index.tsx:31`：union 末尾加 `| "architecture"`；`getTabIcon` 加 `case "architecture": return <LayersIcon size={18} />;`，並 import。
- `Icons.tsx`：仿 `WorkflowIcon`（第 714 行）的寫法新增 `LayersIcon`（三層疊起來的菱形：三個 `<polygon>`／`<polyline>`，stroke 風格與其他圖示一致）。
- `tabCatalog.tsx`：在 `agent-canvas` 那行後加 `{ id: "architecture", type: "architecture", icon: <LayersIcon size={18} />, labelKey: "arch_tab", descKey: "new_arch_desc", color: "#34d399" },`。
- `TerminalApp.tsx`：`import { ArchitectureView } from "./ArchitectureView";`；title 鏈加 `if (type === "architecture") title = t.arch_tab;`；render 鏈在 `agent-canvas` 分支前加 `) : tab.type === "architecture" ? ( <ArchitectureView isActive={isActive} />`。

- [ ] **Step 3: 驗證**

Run: `npx tsc -b && npm run lint && npm run test 2>&1 | tail -15`
Expected: tsc／lint 無錯誤；測試全綠（`MailView` 為已知記憶體吃緊才紅的 flaky，若僅它紅需註明並單獨重跑）。

- [ ] **Step 4: Commit**

```bash
git add -A src
git status --short   # 確認只有預期檔案，不要把 graphify-out/ 掃進來
git commit -m "feat(architecture): 註冊架構導覽分頁型別

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 13: 實機驗收與收尾

- [ ] **Step 1: 全量驗證**（`superpowers:verification-before-completion`）

Run:
```bash
npx tsc -b && npm run lint && npm run test 2>&1 | tail -15
cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep "test result"
```
Expected: 每個 `test result:` 行 failed 為 0；前端全綠。

- [ ] **Step 2: 實機（對象：AITerm 自己的 repo）**

用 `npm run tauri:dev`（macOS 另需 `env -u APPLE_SIGNING_IDENTITY`，見記憶）：新增「架構導覽」分頁 → 選本 repo → 確認出現模組方塊與實線 → 按「AI 分析」→ 確認出現前端／傳輸／後端／資料分層、至少一條 `ipc` 虛線附 evidence → 點模組確認右側詳情與呼叫鏈高亮 → 關閉重開同資料夾確認讀到快取 → 製造新 commit 後重開確認出現「已過期」提示。以 osascript／截圖留證（見記憶：Mac UI 自動化）。**驗不了的（Windows／Linux、不同專案語言）在回報中明講沒驗。**

- [ ] **Step 3: 自我審查與收尾**

`superpowers:requesting-code-review` 做結構化自審；寫 CHANGELOG 段落（寫給使用者看，只寫實機看過的行為）；**不要自行打 tag**（未經同意不推 tag）。
