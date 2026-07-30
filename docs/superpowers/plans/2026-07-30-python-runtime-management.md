# Python 執行環境管理（uv sidecar + 受管 venv）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 AITerm 不再依賴使用者機器上的 Python —— 內建 uv binary，在首次使用相關功能時自動備好受管 venv 與該功能所需的套件。

**Architecture:** 新增 `src-tauri/src/python_env/` 作為唯一權威，內部以內建的 uv binary 完成「裝 Python → 建 venv → 裝套件」三步，對外只暴露 `ensure(profile) -> interpreter 路徑`。`api_docs/runner.rs` 與 `commands/markitdown.rs` 的偵測段與 pip 段全部移除。安裝過程以 `python-env-log` 事件串流到前端既有的安裝面板樣式。

**Tech Stack:** Rust（tokio process、sha2、serde_json）、Tauri 2 externalBin、uv、React 19 + Vitest

**Spec:** `docs/superpowers/specs/2026-07-30-python-runtime-management-design.md`

---

## File Structure

| 檔案 | 職責 |
|---|---|
| `src-tauri/src/python_env/mod.rs`（新） | 對外 API：`status` / `ensure` / `reset` / `set_interpreter`，以及編排順序 |
| `src-tauri/src/python_env/paths.rs`（新） | 解析 uv binary、venv、runtime 目錄、venv 內 interpreter 的位置 |
| `src-tauri/src/python_env/profiles.rs`（新） | `Profile` 列舉與其 requirements 檔對應 |
| `src-tauri/src/python_env/marker.rs`（新） | `.aiterm-profiles.json` 的讀寫與 `needs_install` 判斷 |
| `src-tauri/src/python_env/commands.rs`（新） | uv 指令組裝的純函式（回傳 program/args/env，不執行） |
| `src-tauri/src/commands/python_env.rs`（新） | Tauri command 層 |
| `scripts/setup-uv-{mac,linux,win}.{sh,sh,ps1}`（新） | 下載官方 uv release 到 `src-tauri/binaries/` |
| `src/ipc/pythonEnv.ts`（新） | 前端 IPC wrapper 與型別 |
| `src/components/PythonEnv/PythonEnvGate.tsx`（新） | 缺環境時的引導卡 + 安裝進度面板 |
| `src-tauri/src/api_docs/{mod,runner}.rs`（改） | 刪除 `find_python`，改用 `python_env::ensure` |
| `src-tauri/src/commands/markitdown.rs`（改） | 刪除 `find_python_for_markitdown` 與 pip 段 |
| `src-tauri/src/config/types.rs`（改） | 新增 `python_interpreter: Option<String>` |
| `src/components/Settings/GeneralPage.tsx`（改） | 「Python 環境」區塊 |
| `src/components/DocConverter/DocConverterView.tsx`（改） | 候選安裝提示 |
| `tools/MarkItDown/requirements{,-media}.txt`（改／新） | extras 分層 |

拆成 5 個小檔而非單一 `python_env.rs`：`paths` 是平台分支最密集處，`commands` 是唯一需要在三平台驗證參數的純邏輯，兩者都要能單獨測試與閱讀。

---

## Task 1: 取得 uv binary 的三平台腳本

**Files:**
- Create: `scripts/setup-uv-mac.sh`, `scripts/setup-uv-linux.sh`, `scripts/setup-uv-win.ps1`
- Modify: `src-tauri/tauri.conf.json:49-51`

uv 官方 release 的資產命名為 `uv-<triple>.tar.gz`（Windows 為 `.zip`）。Tauri 的 `externalBin` 要求檔名帶 target triple 後綴。

- [ ] **Step 1: 寫 macOS 腳本**

```bash
#!/usr/bin/env bash
# Fetch the uv binary used as AITerm's Python environment manager (macOS).
# Run once from the workspace root: bash scripts/setup-uv-mac.sh
set -euo pipefail

UV_VERSION="0.11.19"

if [ "$(uname -m)" = "arm64" ]; then
  TRIPLE="aarch64-apple-darwin"
else
  TRIPLE="x86_64-apple-darwin"
fi

DEST="src-tauri/binaries"
mkdir -p "$DEST"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

URL="https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/uv-${TRIPLE}.tar.gz"
echo "==> Downloading uv ${UV_VERSION} (${TRIPLE})"
curl -L --fail "$URL" -o "$TMP/uv.tar.gz"
tar -xzf "$TMP/uv.tar.gz" -C "$TMP"

# Tauri's externalBin requires the target-triple suffix on the filename.
cp "$TMP/uv-${TRIPLE}/uv" "$DEST/uv-${TRIPLE}"
chmod +x "$DEST/uv-${TRIPLE}"
echo "==> Wrote $DEST/uv-${TRIPLE}"
"$DEST/uv-${TRIPLE}" --version
```

- [ ] **Step 2: 寫 Linux 腳本**

```bash
#!/usr/bin/env bash
# Fetch the uv binary used as AITerm's Python environment manager (Linux).
# Run once from the workspace root: ARCH=x64 bash scripts/setup-uv-linux.sh
set -euo pipefail

UV_VERSION="0.11.19"
ARCH="${ARCH:-x64}"

case "$ARCH" in
  x64)   TRIPLE="x86_64-unknown-linux-gnu" ;;
  arm64) TRIPLE="aarch64-unknown-linux-gnu" ;;
  *) echo "ERROR: unsupported ARCH=$ARCH (use x64 or arm64)"; exit 1 ;;
esac

DEST="src-tauri/binaries"
mkdir -p "$DEST"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

URL="https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/uv-${TRIPLE}.tar.gz"
echo "==> Downloading uv ${UV_VERSION} (${TRIPLE})"
curl -L --fail "$URL" -o "$TMP/uv.tar.gz"
tar -xzf "$TMP/uv.tar.gz" -C "$TMP"

cp "$TMP/uv-${TRIPLE}/uv" "$DEST/uv-${TRIPLE}"
chmod +x "$DEST/uv-${TRIPLE}"
echo "==> Wrote $DEST/uv-${TRIPLE}"
# Smoke-test like the mac/windows scripts and all three setup-db2-* do: Linux
# is the one platform whose triple comes from an env var, so a wrong ARCH
# should fail here rather than at runtime. Skip only if the CI runner turns
# out to be cross-fetching (arm64 binary on an x64 host).
"$DEST/uv-${TRIPLE}" --version
```

- [ ] **Step 3: 寫 Windows 腳本**

```powershell
# Fetch the uv binary used as AITerm's Python environment manager (Windows x64).
# Run once from the workspace root:
#   powershell -ExecutionPolicy Bypass -File scripts\setup-uv-win.ps1

$ErrorActionPreference = "Stop"

$UV_VERSION = "0.11.19"
$TRIPLE = "x86_64-pc-windows-msvc"
$DEST = "src-tauri\binaries"

New-Item $DEST -ItemType Directory -Force | Out-Null

$TMP = New-Item -ItemType Directory -Path ([System.IO.Path]::GetTempPath()) -Name ([guid]::NewGuid())
try {
  $URL = "https://github.com/astral-sh/uv/releases/download/$UV_VERSION/uv-$TRIPLE.zip"
  Write-Host "==> Downloading uv $UV_VERSION ($TRIPLE)"
  Invoke-WebRequest -Uri $URL -OutFile "$TMP\uv.zip"
  Expand-Archive -Path "$TMP\uv.zip" -DestinationPath $TMP -Force

  # Tauri's externalBin requires the target-triple suffix on the filename.
  Copy-Item "$TMP\uv.exe" "$DEST\uv-$TRIPLE.exe" -Force
  Write-Host "==> Wrote $DEST\uv-$TRIPLE.exe"
  & "$DEST\uv-$TRIPLE.exe" --version
} finally {
  Remove-Item $TMP -Recurse -Force
}
```

- [ ] **Step 4: 登記 externalBin（四個檔案，缺一不可）**

`src-tauri/tauri.conf.json` 的 `externalBin` 改為：

```json
    "externalBin": [
      "binaries/db2-sidecar",
      "binaries/uv"
    ]
```

**base config 單獨改是無效的。** `tauri.macos.conf.json:19`、`tauri.windows.conf.json:10`、`tauri.linux.conf.json:3` 三者都設 `"externalBin": []`，而 Tauri 的平台專屬 config 走 JSON Merge Patch（RFC 7396）—— 陣列是**整體取代**而非合併，所以 base 的 `binaries/uv` 在三個平台打包時全被清空，binary 永遠不會進 app bundle。只有 dev 模式（Task 2 會掃 `CARGO_MANIFEST_DIR/binaries`）看起來能用，這種 bug 會潛伏到有人測安裝檔才爆。三個平台 conf 都要改為：

```json
    "externalBin": ["binaries/uv"],
```

為何不比照 db2：db2-sidecar 是「一整個目錄（JRE + jar）」，才走 `resources` + `scripts/tauri-build.js` 的 cpSync 手動複製。uv 是單一可執行檔，正是 `externalBin` 的標準用法，而且它**必須**落在執行檔旁邊才能被 `paths::uv_binary` 找到 —— 走 resources 會進 macOS 的 `Contents/Resources/`，位置不對。

已知代價（刻意接受）：沒先跑 setup-uv 腳本的貢獻者，`tauri:dev` / `tauri:build` 會因找不到 `binaries/uv-<triple>` 而失敗。專案對 db2 sidecar 已有同樣前提（CLAUDE.md 要求先 build JAR 才能跑 `tauri:dev`）。

- [ ] **Step 5: 本機執行並確認**

Run: `bash scripts/setup-uv-mac.sh`
Expected: 印出 `uv 0.11.19 ...`，且 `ls src-tauri/binaries/uv-*` 有檔案

確認 `git status --short src-tauri/binaries` 為空（`binaries/` 已 gitignored）。

- [ ] **Step 6: Commit**

```bash
git add scripts/setup-uv-mac.sh scripts/setup-uv-linux.sh scripts/setup-uv-win.ps1 src-tauri/tauri.conf.json
git commit -m "build: fetch uv binary as an external sidecar on all three platforms"
```

---

## Task 2: 路徑解析（`paths.rs`）

**Files:**
- Create: `src-tauri/src/python_env/mod.rs`, `src-tauri/src/python_env/paths.rs`
- Modify: `src-tauri/src/lib.rs`（加 `pub mod python_env;`）

Tauri 打包後 externalBin 會落在執行檔同層並去掉 triple 後綴；dev 模式下檔名仍帶後綴。因此以候選清單解析，並用 `read_dir` 尋找 `uv-*` 而不去推導 target triple（Rust 沒有現成的 triple 常數，避免為此加 build script）。

- [ ] **Step 1: 寫失敗測試**

`src-tauri/src/python_env/paths.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn venv_interpreter_uses_the_platform_layout() {
        let venv = std::path::Path::new("/tmp/python-env");
        let py = venv_interpreter(venv);
        if cfg!(windows) {
            assert!(py.ends_with("Scripts/python.exe") || py.ends_with("Scripts\\python.exe"));
        } else {
            assert!(py.ends_with("bin/python"));
        }
    }

    #[test]
    fn finds_a_triple_suffixed_uv_in_a_dev_binaries_dir() {
        let dir = tempdir().unwrap();
        let name = if cfg!(windows) { "uv-x86_64-pc-windows-msvc.exe" } else { "uv-aarch64-apple-darwin" };
        std::fs::write(dir.path().join(name), b"").unwrap();

        let found = find_suffixed_uv(dir.path()).expect("should find the suffixed binary");
        assert_eq!(found.file_name().unwrap().to_str().unwrap(), name);
    }

    #[test]
    fn ignores_unrelated_files_when_looking_for_uv() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("db2sidecar.jar"), b"").unwrap();
        assert!(find_suffixed_uv(dir.path()).is_none());
    }
}
```

- [ ] **Step 2: 執行確認失敗**

Run: `cd src-tauri && cargo test --lib python_env::paths`
Expected: 編譯失敗，`cannot find function venv_interpreter` / `find_suffixed_uv`

- [ ] **Step 3: 實作**

`src-tauri/src/python_env/paths.rs`：

```rust
//! Where the managed Python environment lives.
//!
//! Tauri drops `externalBin` next to the executable and strips the
//! target-triple suffix when bundling, but leaves it in place during `tauri
//! dev`. Rather than reconstruct the triple (Rust exposes no constant for it,
//! and adding a build script just for this isn't worth it), the dev lookup
//! scans for a `uv-*` entry.

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const UV_STEM: &str = "uv";

/// The venv the app manages. Deleting this directory is always safe.
pub fn venv_dir(app: &AppHandle) -> PathBuf {
    app_data(app).join("python-env")
}

/// Where uv installs interpreters. Kept under app data (rather than uv's
/// default `~/.local/share/uv`) so uninstalling the app can clean it up.
pub fn runtime_dir(app: &AppHandle) -> PathBuf {
    app_data(app).join("python-runtimes")
}

/// The interpreter inside a venv.
pub fn venv_interpreter(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// Locate the bundled uv binary, or `None` if it wasn't shipped.
///
/// Takes no `AppHandle`: the binary's location follows from the executable's
/// own path, so requiring app context would only mislead callers.
pub fn uv_binary() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let plain = exe_dir.join(exe_name(UV_STEM));
    if plain.exists() {
        return Some(plain);
    }
    // `tauri dev` and local `cargo run` both leave the suffixed name in place.
    if let Some(found) = find_suffixed_uv(&exe_dir) {
        return Some(found);
    }
    let dev_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    find_suffixed_uv(&dev_dir)
}

/// First `uv-<triple>` entry in `dir`, if any.
///
/// A dev machine can accumulate binaries for several triples, and `read_dir`
/// order is platform- and filesystem-dependent — so prefer the one whose
/// triple carries this machine's architecture (picking another would fail at
/// exec time with an error that says nothing about where it came from), and
/// sort so the remaining case is at least deterministic.
fn find_suffixed_uv(dir: &Path) -> Option<PathBuf> {
    let prefix = format!("{UV_STEM}-");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        })
        .collect();
    candidates.sort();
    candidates
        .iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(std::env::consts::ARCH))
        })
        .cloned()
        .or_else(|| candidates.pop())
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() }
}

fn app_data(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}
```

`src-tauri/src/python_env/mod.rs`（本任務只放 module 宣告）：

```rust
//! Manages the Python environment the app's Python-backed features need.
//!
//! Everything runs through the bundled uv binary: it installs an interpreter,
//! creates a venv under app data, and installs per-profile requirements. No
//! feature touches the user's own Python installation.

pub mod paths;
```

`src-tauri/src/lib.rs` 的 module 宣告區加入：

```rust
pub mod python_env;
```

- [ ] **Step 4: 執行測試**

Run: `cd src-tauri && cargo test --lib python_env::paths`
Expected: 3 passed

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/python_env src-tauri/src/lib.rs
git commit -m "feat(python-env): resolve the bundled uv binary and managed env paths"
```

---

## Task 3: Profile 與 requirements 分層

**Files:**
- Create: `src-tauri/src/python_env/profiles.rs`, `tools/MarkItDown/requirements-media.txt`
- Modify: `tools/MarkItDown/requirements.txt`, `src-tauri/tauri.conf.json`（resources）, `src-tauri/src/python_env/mod.rs`

- [ ] **Step 1: 寫失敗測試**

`src-tauri/src/python_env/profiles.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_profile_has_a_distinct_marker_key() {
        let keys: Vec<&str> = Profile::ALL.iter().map(|p| p.marker_key()).collect();
        let mut deduped = keys.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(keys.len(), deduped.len(), "marker keys must be unique");
    }

    #[test]
    fn media_profile_reads_the_media_requirements_file() {
        assert_eq!(Profile::DocMedia.requirements_file(), "requirements-media.txt");
        assert_eq!(Profile::DocCore.requirements_file(), "requirements.txt");
        assert_eq!(Profile::ApiDocs.requirements_file(), "requirements.txt");
    }

    #[test]
    fn doc_profiles_share_the_markitdown_tool_dir() {
        assert_eq!(Profile::DocCore.tool_dir(), "MarkItDown");
        assert_eq!(Profile::DocMedia.tool_dir(), "MarkItDown");
        assert_eq!(Profile::ApiDocs.tool_dir(), "ApiDocFetcher");
    }

    #[test]
    fn marker_key_matches_the_serialized_form() {
        // These are two representations of one wire format: the marker file keys
        // off marker_key(), while Tauri commands and the frontend type
        // ("api_docs" | "doc_core" | "doc_media") go through serde. Pin them
        // together so changing either one can't silently split them apart.
        for profile in Profile::ALL {
            let serialized = serde_json::to_string(&profile).unwrap();
            assert_eq!(serialized, format!("\"{}\"", profile.marker_key()));
        }
    }
}
```

最後一個測試在初次實作時就是綠燈 —— 它的價值不在當下抓 bug，而在鎖住契約。**驗證它真的有效**：暫時改掉某個 `marker_key()` 的回傳字串，確認測試轉紅，再改回。永遠不會失敗的測試等於沒有測試。

- [ ] **Step 2: 執行確認失敗**

Run: `cd src-tauri && cargo test --lib python_env::profiles`
Expected: 編譯失敗，`cannot find type Profile`

- [ ] **Step 3: 實作 profiles.rs**

```rust
//! The dependency sets features ask for.
//!
//! All profiles share one venv: isolating each would double the disk cost and
//! the bookkeeping for no benefit, since nothing here has conflicting pins.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// API doc scraping (`tools/ApiDocFetcher`).
    ApiDocs,
    /// Document conversion for the formats most people convert.
    DocCore,
    /// Image and audio conversion — installed on demand, since these extras
    /// are the bulk of the download.
    DocMedia,
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::ApiDocs, Profile::DocCore, Profile::DocMedia];

    /// Directory under `tools/` (dev) or the resource bundle (production).
    pub fn tool_dir(&self) -> &'static str {
        match self {
            Profile::ApiDocs => "ApiDocFetcher",
            Profile::DocCore | Profile::DocMedia => "MarkItDown",
        }
    }

    pub fn requirements_file(&self) -> &'static str {
        match self {
            Profile::ApiDocs | Profile::DocCore => "requirements.txt",
            Profile::DocMedia => "requirements-media.txt",
        }
    }

    /// Key under which this profile's installed-hash is recorded.
    pub fn marker_key(&self) -> &'static str {
        match self {
            Profile::ApiDocs => "api_docs",
            Profile::DocCore => "doc_core",
            Profile::DocMedia => "doc_media",
        }
    }
}
```

`mod.rs` 加 `pub mod profiles;`

- [ ] **Step 4: 拆分 requirements**

`tools/MarkItDown/requirements.txt` 改為：

```
markitdown[pdf,docx,pptx,xlsx]>=0.1.0
```

`tools/MarkItDown/requirements-media.txt`（新）：

```
markitdown[image,audio-transcription]>=0.1.0
```

**base `tauri.conf.json` 原本沒有 `resources` key** —— MarkItDown 與 ApiDocFetcher 的條目只存在於三份平台 conf，各自完整重複列出。所以這裡是新增整個 key，只放新項目：

```json
    "resources": {
      "../tools/MarkItDown/requirements-media.txt": "MarkItDown/requirements-media.txt"
    }
```

**為什麼放 base 就夠（已實證，不是推論）**：`externalBin` 是陣列，JSON Merge Patch 對陣列是整體取代，所以 Task 1 必須改四份。但 `resources` 是物件，對物件是**遞迴合併**。實驗方法：在 base 的 resources 放一個不存在的路徑，`cargo check --lib` 會失敗並指名該路徑（`tauri-build` 驗證每個 resources 項目是否存在）—— 即使平台 conf 也定義了完整的 resources 物件，base 的項目依然生效。

這留下一個刻意的不對稱：media 條目在 base，其餘在平台 conf。不把既有重複項目一併搬進 base，是因為那是範圍外的重構；而 media 放 base 反而更穩健 —— `release.yml` 會整檔重寫 `tauri.linux.conf.json`，放在 base 的項目不會被那個重寫波及。

- [ ] **Step 5: 執行測試**

Run: `cd src-tauri && cargo test --lib python_env::profiles`
Expected: 3 passed

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/python_env tools/MarkItDown src-tauri/tauri.conf.json
git commit -m "feat(python-env): split MarkItDown extras into core and on-demand media tiers"
```

---

## Task 4: 安裝標記檔（`marker.rs`）

**Files:**
- Create: `src-tauri/src/python_env/marker.rs`
- Modify: `src-tauri/src/python_env/mod.rs`

目前每次使用都跑一次 pip。改以 requirements 檔的 sha256 判斷是否需要安裝。

- [ ] **Step 1: 寫失敗測試**

`src-tauri/src/python_env/marker.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::python_env::profiles::Profile;
    use tempfile::tempdir;

    fn write_requirements(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("requirements.txt");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn install_is_needed_when_no_marker_exists() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        assert!(needs_install(dir.path(), Profile::DocCore, &req).unwrap());
    }

    #[test]
    fn install_is_skipped_once_recorded() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");

        record_installed(dir.path(), Profile::DocCore, &req).unwrap();

        assert!(!needs_install(dir.path(), Profile::DocCore, &req).unwrap());
    }

    #[test]
    fn install_is_needed_again_after_requirements_change() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        record_installed(dir.path(), Profile::DocCore, &req).unwrap();

        write_requirements(dir.path(), "markitdown>=0.2.0\n");

        assert!(needs_install(dir.path(), Profile::DocCore, &req).unwrap());
    }

    #[test]
    fn a_corrupt_marker_is_treated_as_nothing_installed() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        std::fs::write(dir.path().join(MARKER_FILE), b"{not json").unwrap();

        assert!(needs_install(dir.path(), Profile::DocCore, &req).unwrap());
    }

    #[test]
    fn recording_one_profile_leaves_the_others_untouched() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        record_installed(dir.path(), Profile::DocCore, &req).unwrap();
        record_installed(dir.path(), Profile::DocMedia, &req).unwrap();

        assert!(!needs_install(dir.path(), Profile::DocCore, &req).unwrap());
        assert!(!needs_install(dir.path(), Profile::DocMedia, &req).unwrap());
        assert!(needs_install(dir.path(), Profile::ApiDocs, &req).unwrap());
    }

    #[test]
    fn installed_profiles_lists_only_recorded_ones() {
        let dir = tempdir().unwrap();
        let req = write_requirements(dir.path(), "markitdown>=0.1.0\n");
        record_installed(dir.path(), Profile::DocCore, &req).unwrap();

        assert_eq!(installed_profiles(dir.path()), vec![Profile::DocCore]);
    }
}
```

- [ ] **Step 2: 執行確認失敗**

Run: `cd src-tauri && cargo test --lib python_env::marker`
Expected: 編譯失敗，`cannot find function needs_install`

- [ ] **Step 3: 實作 marker.rs**

```rust
//! Records which profiles are installed in the managed venv.
//!
//! Keyed by the sha256 of the requirements file, so editing a requirements
//! file re-installs that profile and nothing else. A missing or unreadable
//! marker means "nothing is installed" — re-installing is cheap and correct,
//! guessing is not.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use super::profiles::Profile;

pub const MARKER_FILE: &str = ".aiterm-profiles.json";

/// True when `profile`'s requirements differ from what's recorded.
pub fn needs_install(venv: &Path, profile: Profile, requirements: &Path) -> Result<bool> {
    let want = hash_file(requirements)?;
    Ok(read_marker(venv).get(profile.marker_key()) != Some(&want))
}

/// Record `profile` as installed at the requirements file's current contents.
pub fn record_installed(venv: &Path, profile: Profile, requirements: &Path) -> Result<()> {
    let hash = hash_file(requirements)?;
    let mut marker = read_marker(venv);
    marker.insert(profile.marker_key().to_string(), hash);

    std::fs::create_dir_all(venv)
        .with_context(|| format!("creating {}", venv.display()))?;
    let body = serde_json::to_string_pretty(&marker)?;
    std::fs::write(venv.join(MARKER_FILE), body)
        .with_context(|| format!("writing {MARKER_FILE}"))
}

/// Profiles with a recorded hash, whatever it is. Used for status display, so
/// a stale hash still counts as "installed" — `ensure` re-checks properly.
pub fn installed_profiles(venv: &Path) -> Vec<Profile> {
    let marker = read_marker(venv);
    Profile::ALL
        .into_iter()
        .filter(|p| marker.contains_key(p.marker_key()))
        .collect()
}

fn read_marker(venv: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(venv.join(MARKER_FILE))
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default()
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}
```

`mod.rs` 加 `pub mod marker;`

- [ ] **Step 4: 執行測試**

Run: `cd src-tauri && cargo test --lib python_env::marker`
Expected: 6 passed

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/python_env
git commit -m "feat(python-env): track installed profiles by requirements hash"
```

---

## Task 5: uv 指令組裝（`commands.rs`，純函式）

**Files:**
- Create: `src-tauri/src/python_env/commands.rs`
- Modify: `src-tauri/src/python_env/mod.rs`

把指令組裝與執行分離，讓三平台都能在不裝 uv 的情況下驗證參數。

- [ ] **Step 1: 寫失敗測試**

`src-tauri/src/python_env/commands.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn uv() -> PathBuf { PathBuf::from("/opt/aiterm/uv") }

    #[test]
    fn python_install_pins_the_version_and_redirects_the_install_dir() {
        let spec = install_python(&uv(), &PathBuf::from("/data/python-runtimes"));

        assert_eq!(spec.program, uv());
        assert_eq!(spec.args, vec!["python", "install", PYTHON_VERSION]);
        assert_eq!(
            spec.env.get("UV_PYTHON_INSTALL_DIR").map(String::as_str),
            Some("/data/python-runtimes")
        );
    }

    #[test]
    fn venv_creation_uses_the_managed_interpreter_by_default() {
        let spec = create_venv(&uv(), &PathBuf::from("/data/python-env"), &PathBuf::from("/data/rt"), None);

        assert_eq!(spec.args, vec!["venv", "/data/python-env", "--python", PYTHON_VERSION]);
    }

    #[test]
    fn venv_creation_honours_a_user_specified_interpreter() {
        let spec = create_venv(
            &uv(),
            &PathBuf::from("/data/python-env"),
            &PathBuf::from("/data/rt"),
            Some(&PathBuf::from("/usr/local/bin/python3.12")),
        );

        assert_eq!(
            spec.args,
            vec!["venv", "/data/python-env", "--python", "/usr/local/bin/python3.12"]
        );
    }

    #[test]
    fn pip_install_targets_the_venv_interpreter_not_the_system_one() {
        let spec = install_requirements(
            &uv(),
            &PathBuf::from("/data/python-env/bin/python"),
            &PathBuf::from("/tools/MarkItDown/requirements.txt"),
        );

        assert_eq!(
            spec.args,
            vec![
                "pip",
                "install",
                "--python",
                "/data/python-env/bin/python",
                "-r",
                "/tools/MarkItDown/requirements.txt",
            ]
        );
    }

    #[test]
    fn every_spec_disables_uvs_progress_animation() {
        // The log panel shows plain lines; uv's spinner would render as noise.
        let specs = [
            install_python(&uv(), &PathBuf::from("/rt")),
            create_venv(&uv(), &PathBuf::from("/env"), &PathBuf::from("/rt"), None),
            install_requirements(&uv(), &PathBuf::from("/env/bin/python"), &PathBuf::from("/r.txt")),
        ];
        for spec in specs {
            assert_eq!(spec.env.get("UV_NO_PROGRESS").map(String::as_str), Some("1"));
        }
    }
}
```

- [ ] **Step 2: 執行確認失敗**

Run: `cd src-tauri && cargo test --lib python_env::commands`
Expected: 編譯失敗，`cannot find function install_python`

- [ ] **Step 3: 實作 commands.rs**

```rust
//! uv invocations, as data.
//!
//! Building the command and running it are separate so the arguments can be
//! asserted on every platform without uv present.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Pinned rather than "latest" so a new uv default can't silently move the
/// interpreter under existing installs. MarkItDown needs >= 3.10.
pub const PYTHON_VERSION: &str = "3.12";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

fn spec(program: &Path, args: &[&str], runtime_dir: Option<&Path>) -> CommandSpec {
    let mut env = BTreeMap::new();
    env.insert("UV_NO_PROGRESS".to_string(), "1".to_string());
    if let Some(dir) = runtime_dir {
        env.insert(
            "UV_PYTHON_INSTALL_DIR".to_string(),
            dir.to_string_lossy().into_owned(),
        );
    }
    CommandSpec {
        program: program.to_path_buf(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env,
    }
}

/// Download a managed interpreter into `runtime_dir`.
pub fn install_python(uv: &Path, runtime_dir: &Path) -> CommandSpec {
    spec(uv, &["python", "install", PYTHON_VERSION], Some(runtime_dir))
}

/// Create the managed venv. `interpreter` overrides the managed interpreter
/// when the user pointed the app at their own Python.
pub fn create_venv(
    uv: &Path,
    venv: &Path,
    runtime_dir: &Path,
    interpreter: Option<&Path>,
) -> CommandSpec {
    let venv = venv.to_string_lossy().into_owned();
    let python = match interpreter {
        Some(path) => path.to_string_lossy().into_owned(),
        None => PYTHON_VERSION.to_string(),
    };
    spec(
        uv,
        &["venv", &venv, "--python", &python],
        Some(runtime_dir),
    )
}

/// Install a requirements file into the venv.
pub fn install_requirements(uv: &Path, venv_python: &Path, requirements: &Path) -> CommandSpec {
    let python = venv_python.to_string_lossy().into_owned();
    let req = requirements.to_string_lossy().into_owned();
    spec(
        uv,
        &["pip", "install", "--python", &python, "-r", &req],
        None,
    )
}
```

`mod.rs` 加 `pub mod commands;`

- [ ] **Step 4: 執行測試**

Run: `cd src-tauri && cargo test --lib python_env::commands`
Expected: 5 passed

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/python_env
git commit -m "feat(python-env): build uv invocations as assertable data"
```

---

## Task 6: `ensure()` 編排與事件串流

**Files:**
- Modify: `src-tauri/src/python_env/mod.rs`
- Test: 同檔 `#[cfg(test)]`

- [ ] **Step 1: 寫失敗測試（錯誤訊息與序列化語意）**

`mod.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_uv_names_the_setup_script_for_the_current_platform() {
        let msg = PythonEnvError::UvMissing.to_string();
        assert!(msg.contains("setup-uv"), "should point at the setup script: {msg}");
    }

    #[test]
    fn install_failure_keeps_the_tail_of_the_output() {
        let err = PythonEnvError::InstallFailed {
            profile: "doc_core".into(),
            output: "ERROR: could not build wheel for curl_cffi".into(),
        };
        assert!(err.to_string().contains("curl_cffi"));
    }

    #[test]
    fn compile_failures_are_called_out_as_toolchain_problems() {
        // A missing compiler is the one install failure a user can't fix by
        // retrying, so it must not read like a generic pip error.
        assert!(looks_like_compile_failure("error: command 'cc' failed"));
        assert!(looks_like_compile_failure("Microsoft Visual C++ 14.0 or greater is required"));
        assert!(!looks_like_compile_failure("ERROR: No matching distribution found"));
    }
}
```

- [ ] **Step 2: 執行確認失敗**

Run: `cd src-tauri && cargo test --lib python_env::tests`
Expected: 編譯失敗，`cannot find type PythonEnvError`

- [ ] **Step 3: 實作 mod.rs 本體**

在 `mod.rs` 的 module 宣告之後加入：

```rust
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncBufReadExt;

use commands::CommandSpec;
use profiles::Profile;

/// Serialises environment preparation. Two features can ask at once (knowledge
/// base import and doc conversion), and two uv processes writing the same venv
/// is a corruption waiting to happen.
static ENSURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Serialize)]
pub struct PythonEnvLogEvent {
    pub level: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PythonEnvError {
    #[error("找不到內建的 uv 執行檔。開發環境請先執行 scripts/setup-uv-mac.sh（或對應平台的 setup-uv 腳本）。")]
    UvMissing,

    #[error("無法取得 Python：{0}")]
    PythonUnavailable(String),

    #[error("建立 Python 環境失敗：{0}")]
    VenvFailed(String),

    #[error("安裝 {profile} 相依套件失敗：{output}")]
    InstallFailed { profile: String, output: String },

    #[error("套件需要在本機編譯，但找不到編譯工具鏈：{0}")]
    ToolchainMissing(String),

    #[error("{0}")]
    Io(String),
}

impl From<PythonEnvError> for String {
    fn from(e: PythonEnvError) -> String {
        e.to_string()
    }
}

/// Heuristic for "this failed because there's no compiler", which needs a
/// different remedy than any other install failure.
fn looks_like_compile_failure(output: &str) -> bool {
    const MARKERS: [&str; 4] = [
        "command 'cc' failed",
        "command 'gcc' failed",
        "Microsoft Visual C++",
        "error: linker",
    ];
    MARKERS.iter().any(|m| output.contains(m))
}

/// Prepare the environment for `profile` and return its interpreter.
///
/// Idempotent and cheap once warm: the marker file short-circuits the install
/// step, so the common path is a couple of filesystem checks.
pub async fn ensure(app: &AppHandle, profile: Profile) -> Result<PathBuf, PythonEnvError> {
    let _guard = ENSURE_LOCK.lock().await;

    // `paths::app_data` falls back to "." when app_data_dir() fails, which would
    // silently build the venv in the process's working directory. Failing is
    // better than writing a multi-hundred-MB environment somewhere the user
    // never looks and the app can't find again.
    if app.path().app_data_dir().is_err() {
        return Err(PythonEnvError::Io(
            "無法取得應用程式資料目錄，請確認磁碟權限".to_string(),
        ));
    }

    let uv = paths::uv_binary().ok_or(PythonEnvError::UvMissing)?;
    let venv = paths::venv_dir(app);
    let runtimes = paths::runtime_dir(app);
    let interpreter = user_interpreter(app);

    let mut python = paths::venv_interpreter(&venv);
    if !python.exists() {
        if interpreter.is_none() {
            log(app, "info", "正在取得 Python…");
            run(app, commands::install_python(&uv, &runtimes))
                .await
                .map_err(PythonEnvError::PythonUnavailable)?;
        }
        log(app, "info", "正在建立 Python 環境…");
        run(
            app,
            commands::create_venv(&uv, &venv, &runtimes, interpreter.as_deref()),
        )
        .await
        .map_err(PythonEnvError::VenvFailed)?;
    }

    // A venv can survive on disk but stop working (deleted files, an OS
    // upgrade moving dylibs). Rebuild once before giving up.
    if !interpreter_works(&python).await {
        log(app, "warn", "Python 環境無法執行，正在重建…");
        let _ = tokio::fs::remove_dir_all(&venv).await;
        run(
            app,
            commands::create_venv(&uv, &venv, &runtimes, interpreter.as_deref()),
        )
        .await
        .map_err(PythonEnvError::VenvFailed)?;
        python = paths::venv_interpreter(&venv);
        if !interpreter_works(&python).await {
            return Err(PythonEnvError::VenvFailed(
                "重建後仍無法執行 Python".to_string(),
            ));
        }
    }

    let requirements = requirements_path(app, profile);
    if marker::needs_install(&venv, profile, &requirements).unwrap_or(true) {
        log(app, "info", "正在安裝相依套件（首次使用需要一些時間）…");
        run(app, commands::install_requirements(&uv, &python, &requirements))
            .await
            .map_err(|output| {
                if looks_like_compile_failure(&output) {
                    PythonEnvError::ToolchainMissing(tail(&output))
                } else {
                    PythonEnvError::InstallFailed {
                        profile: profile.marker_key().to_string(),
                        output: tail(&output),
                    }
                }
            })?;
        marker::record_installed(&venv, profile, &requirements)
            .map_err(|e| PythonEnvError::Io(e.to_string()))?;
    }

    Ok(python)
}

/// `tools/<dir>/<file>` in dev, the resource bundle in production.
fn requirements_path(app: &AppHandle, profile: Profile) -> PathBuf {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .join("tools")
        .join(profile.tool_dir())
        .join(profile.requirements_file());
    if dev.exists() {
        return dev;
    }
    app.path()
        .resource_dir()
        .map(|r| r.join(profile.tool_dir()).join(profile.requirements_file()))
        .unwrap_or(dev)
}

async fn interpreter_works(python: &Path) -> bool {
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg("-c").arg("import sys");
    no_window(&mut cmd);
    matches!(cmd.status().await, Ok(s) if s.success())
}

/// Run a spec, streaming both streams to the log panel. On failure returns the
/// combined output so the caller can classify it.
async fn run(app: &AppHandle, spec: CommandSpec) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .envs(&spec.env)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    no_window(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("stdout not piped")?;
    let stderr = child.stderr.take().ok_or("stderr not piped")?;

    // stderr on its own task, matching api_docs/runner.rs:133. A `select!` over
    // both readers would break out of the loop on whichever stream ended first
    // and lose the other's remaining output — which is exactly the output that
    // explains a failure, since uv reports errors on stderr.
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let mut collected = String::new();
    let mut out_lines = tokio::io::BufReader::new(stdout).lines();
    while let Some(line) = out_lines.next_line().await.map_err(|e| e.to_string())? {
        collected.push_str(&line);
        collected.push('\n');
        log(app, "info", &line);
    }

    let stderr_output = stderr_task.await.unwrap_or_default();
    for line in stderr_output.lines() {
        log(app, "warn", line);
    }

    let status = child.wait().await.map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        collected.push_str(&stderr_output);
        Err(collected)
    }
}

fn tail(output: &str) -> String {
    output.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
}

fn log(app: &AppHandle, level: &str, message: &str) {
    let _ = app.emit(
        "python-env-log",
        PythonEnvLogEvent { level: level.into(), message: message.into() },
    );
}

fn no_window(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let _ = cmd;
}
```

`Cargo.toml` 確認有 `thiserror`；若無則加入 `thiserror = "2"`。

- [ ] **Step 3b: 實測 markitdown 的版本漂移風險**

拆成 core／media 兩份 requirements 後，兩者都用開放式 `>=0.1.0`，而它們是**分兩次**解析安裝的。風險情境：使用者先觸發 `DocCore` 安裝（當時 markitdown 是 0.1.0），數週後第一次選了 PNG 觸發 `DocMedia`，此時 PyPI 上已是 0.3.0 —— 若解析器不保留已裝版本，markitdown 本體會被靜默升級，使用者只想加圖片支援卻連帶換掉已經跑穩的文件轉換核心。拆分前只有一次解析，不存在這個問題。

這個風險是否真的存在，取決於 uv 的實際行為（是否像 pip 一樣「已滿足約束就不動」），必須實測而非推論：

```bash
# 在暫時的 venv 裡先裝 core，記下版本
uv venv /tmp/drift-probe --python 3.12
uv pip install --python /tmp/drift-probe/bin/python -r tools/MarkItDown/requirements.txt
uv pip list --python /tmp/drift-probe/bin/python | grep -i markitdown

# 再裝 media，看本體版本有沒有變
uv pip install --python /tmp/drift-probe/bin/python -r tools/MarkItDown/requirements-media.txt
uv pip list --python /tmp/drift-probe/bin/python | grep -i markitdown

rm -rf /tmp/drift-probe
```

若版本不變 → 記錄結論、不需處理。若被升級 → 兩份檔案改成相同的上下界（例如 `>=0.1.0,<0.2.0`），確保不論安裝順序或時間差都落在同一版本。**不要在沒實測前就加上界** —— 猜錯當前版本會直接把安裝鎖死。

- [ ] **Step 4: 執行測試**

Run: `cd src-tauri && cargo test --lib python_env`
Expected: 全數 passed（含前面任務的 14 個）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/python_env src-tauri/Cargo.toml
git commit -m "feat(python-env): orchestrate install with progress events and a single lock"
```

### 關於「假 uv 整合測試」的取捨（spec 要求明確記錄，不得默默省略）

**決定：不做，改以純函式測試 + Task 14 的端到端驗證覆蓋。**

理由：要用假冒的 uv 執行檔測 `ensure`，就得在 production 程式碼裡開一個 uv 路徑的注入點（環境變數或參數），而 `paths::uv_binary` 目前刻意只認打包位置。而 `ensure` 的價值幾乎都在「編排順序」，它每一步實際送給 uv 的參數已由 `commands.rs` 的 5 個純函式完整覆蓋；順序本身若錯（例如先裝套件才建 venv），Task 14 的第一次端到端驗證就會立刻失敗。為此在正式程式碼開一個只有測試會用的後門，換到的邊際保障不划算。

若日後 `ensure` 的分支變多（例如加入版本升級、多 venv），再回頭引入注入點與假 uv 測試。

---

## Task 7: `status` / `reset` / `set_interpreter` 與 config 欄位

**Files:**
- Modify: `src-tauri/src/python_env/mod.rs`, `src-tauri/src/config/types.rs`

- [ ] **Step 1: 寫失敗測試**

`src-tauri/src/config/types.rs` 的 tests 模組（若無則建立 `#[cfg(test)] mod python_interpreter_tests`）：

```rust
#[cfg(test)]
mod python_interpreter_tests {
    use super::*;

    #[test]
    fn python_interpreter_defaults_to_none_for_existing_configs() {
        let cfg: AppConfig = toml::from_str("").expect("empty config should parse");
        assert_eq!(cfg.python_interpreter, None);
    }

    #[test]
    fn python_interpreter_round_trips() {
        let cfg: AppConfig =
            toml::from_str("python_interpreter = \"/usr/local/bin/python3.12\"").unwrap();
        assert_eq!(cfg.python_interpreter.as_deref(), Some("/usr/local/bin/python3.12"));
    }
}
```

- [ ] **Step 2: 執行確認失敗**

Run: `cd src-tauri && cargo test --lib python_interpreter_tests`
Expected: 編譯失敗，`no field python_interpreter`

- [ ] **Step 3: 加 config 欄位**

`src-tauri/src/config/types.rs` 的 `AppConfig` 內加入：

```rust
    /// Interpreter the user pointed us at when uv can't fetch one (offline or
    /// behind a proxy). The venv is still created under app data — this only
    /// changes which interpreter it's based on.
    #[serde(default)]
    pub python_interpreter: Option<String>,
```

- [ ] **Step 4: 實作三個 API**

`src-tauri/src/python_env/mod.rs` 加入：

```rust
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvStatus {
    pub uv_available: bool,
    pub python_version: Option<String>,
    pub installed: Vec<Profile>,
    pub venv_path: String,
    pub user_interpreter: Option<String>,
}

/// Snapshot for the settings page and the feature gates. Never runs uv.
pub fn status(app: &AppHandle) -> EnvStatus {
    let venv = paths::venv_dir(app);
    let python = paths::venv_interpreter(&venv);
    let python_version = std::process::Command::new(&python)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let raw = if o.stdout.is_empty() { o.stderr } else { o.stdout };
            String::from_utf8_lossy(&raw).trim().to_string()
        });

    EnvStatus {
        uv_available: paths::uv_binary().is_some(),
        python_version,
        installed: marker::installed_profiles(&venv),
        venv_path: venv.to_string_lossy().into_owned(),
        user_interpreter: user_interpreter(app).map(|p| p.to_string_lossy().into_owned()),
    }
}

/// Delete the venv (and optionally the downloaded interpreters). The next
/// `ensure` rebuilds from scratch.
pub async fn reset(app: &AppHandle, purge_runtimes: bool) -> Result<(), PythonEnvError> {
    let _guard = ENSURE_LOCK.lock().await;
    remove_if_present(&paths::venv_dir(app)).await?;
    if purge_runtimes {
        remove_if_present(&paths::runtime_dir(app)).await?;
    }
    Ok(())
}

async fn remove_if_present(dir: &Path) -> Result<(), PythonEnvError> {
    match tokio::fs::remove_dir_all(dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PythonEnvError::Io(format!("刪除 {} 失敗：{e}", dir.display()))),
    }
}

fn user_interpreter(app: &AppHandle) -> Option<PathBuf> {
    let config = app.state::<std::sync::Arc<crate::config::ConfigStore>>();
    config
        .get()
        .python_interpreter
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}
```

- [ ] **Step 5: 執行測試**

Run: `cd src-tauri && cargo test --lib` 
Expected: 全數 passed

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/python_env src-tauri/src/config/types.rs
git commit -m "feat(python-env): expose status, reset and a user-specified interpreter"
```

---

## Task 8: Tauri commands 與 IPC wrapper

**Files:**
- Create: `src-tauri/src/commands/python_env.rs`, `src/ipc/pythonEnv.ts`
- Modify: `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs`

- [ ] **Step 1: 實作 command 層**

`src-tauri/src/commands/python_env.rs`：

```rust
//! Tauri surface for the managed Python environment.

use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::config::ConfigStore;
use crate::python_env::{self, profiles::Profile, EnvStatus};

#[tauri::command]
pub fn python_env_status(app: AppHandle) -> EnvStatus {
    python_env::status(&app)
}

#[tauri::command]
pub async fn python_env_ensure(app: AppHandle, profile: Profile) -> Result<(), String> {
    python_env::ensure(&app, profile).await.map(|_| ()).map_err(String::from)
}

#[tauri::command]
pub async fn python_env_reset(app: AppHandle, purge_runtimes: bool) -> Result<(), String> {
    python_env::reset(&app, purge_runtimes).await.map_err(String::from)
}

#[tauri::command]
pub fn python_env_set_interpreter(
    path: Option<String>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<(), String> {
    config
        .update(|cfg| {
            cfg.python_interpreter = path.as_ref().map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        })
        .map_err(|e| format!("儲存設定失敗：{e}"))
}
```

`src-tauri/src/commands/mod.rs` 加 `pub mod python_env;`

`src-tauri/src/lib.rs` 的 `generate_handler!` 清單加入（緊接在 markitdown 相關 command 之後，維持分組註解風格）：

```rust
            // Python environment
            python_env_status,
            python_env_ensure,
            python_env_reset,
            python_env_set_interpreter,
```

並在 use 區塊引入這四個 command。

- [ ] **Step 2: 寫前端 IPC wrapper**

`src/ipc/pythonEnv.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";

export type PythonProfile = "api_docs" | "doc_core" | "doc_media";

export interface PythonEnvStatus {
  uvAvailable: boolean;
  pythonVersion: string | null;
  installed: PythonProfile[];
  venvPath: string;
  userInterpreter: string | null;
}

export interface PythonEnvLogEvent {
  level: "info" | "warn" | "error";
  message: string;
}

export const pythonEnvStatus = (): Promise<PythonEnvStatus> =>
  invoke("python_env_status");

export const pythonEnvEnsure = (profile: PythonProfile): Promise<void> =>
  invoke("python_env_ensure", { profile });

/** `purgeRuntimes` also deletes downloaded interpreters, not just the venv. */
export const pythonEnvReset = (purgeRuntimes: boolean): Promise<void> =>
  invoke("python_env_reset", { purgeRuntimes });

export const pythonEnvSetInterpreter = (path: string | null): Promise<void> =>
  invoke("python_env_set_interpreter", { path });
```

- [ ] **Step 3: 型別檢查與編譯**

Run: `npx tsc -b && cd src-tauri && cargo build --lib`
Expected: 皆無錯誤

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands src-tauri/src/lib.rs src/ipc/pythonEnv.ts
git commit -m "feat(python-env): add IPC commands and their typed wrappers"
```

---

## Task 9: 改寫 `api_docs` 使用受管環境

**Files:**
- Modify: `src-tauri/src/api_docs/mod.rs:8-15`（刪除 `find_python`）, `src-tauri/src/api_docs/runner.rs:59-109`

- [ ] **Step 1: 刪除假偵測**

移除 `src-tauri/src/api_docs/mod.rs` 中整個 `find_python` 函式（含其 doc comment）。它宣稱會探測 interpreter，實際只回傳硬寫的名稱，在沒有 Python 的 Windows 上會命中 WindowsApps stub。

- [ ] **Step 2: 改寫 runner.rs**

`run_fetcher` 開頭的 `let python = super::find_python();` 與其後整段 pip install 區塊（`let req_file = ...` 到該 `if` 結束）替換為：

```rust
    let python = crate::python_env::ensure(app, crate::python_env::profiles::Profile::ApiDocs)
        .await
        .map_err(String::from)?;
```

`let mut fetch_cmd = Command::new(python);` 改為 `Command::new(&python)`。

- [ ] **Step 3: 編譯並確認舊符號已消失**

Run: `cd src-tauri && cargo build --lib 2>&1 | grep -c "find_python"`
Expected: `0`

Run: `grep -rn "find_python" src-tauri/src | wc -l`
Expected: `0`

- [ ] **Step 4: 執行測試**

Run: `cd src-tauri && cargo test`
Expected: 全數 passed

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/api_docs
git commit -m "refactor(api-docs): use the managed Python env, drop the fake interpreter probe"
```

---

## Task 10: 改寫 `markitdown` 使用受管環境

**Files:**
- Modify: `src-tauri/src/commands/markitdown.rs:10-54`（刪除 `find_python_for_markitdown`）, `:128-182`

- [ ] **Step 1: 刪除偵測函式**

移除 `find_python_for_markitdown` 整個函式。其候選清單與版本驗證的職責已由受管 venv 取代；其中的中文安裝指引文案移到前端引導卡（Task 11）。

- [ ] **Step 2: 改寫 `markitdown_convert`**

`let python = find_python_for_markitdown()?;` 與其後整段 pip install 區塊（`let req_file = ...` 到該 `if` 結束）替換為：

```rust
    let python = crate::python_env::ensure(&app, crate::python_env::profiles::Profile::DocCore)
        .await
        .map_err(String::from)?;
```

保留 `let script_dir = script.parent().unwrap_or(script.as_path());`（後續 `current_dir` 仍需要），移除已無用的 `req_file`。

- [ ] **Step 3: 編譯並確認**

Run: `cd src-tauri && cargo build --lib 2>&1 | grep -E "^warning: unused|^error" | head`
Expected: 無 `error`，無新的 unused 警告

- [ ] **Step 4: 執行測試**

Run: `cd src-tauri && cargo test && npm run test --prefix ..`
Expected: 全數 passed

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/markitdown.rs
git commit -m "refactor(markitdown): use the managed Python env instead of the user's Python"
```

---

## Task 11: 前端引導卡與安裝進度面板

**Files:**
- Create: `src/components/PythonEnv/PythonEnvGate.tsx`, `src/components/PythonEnv/PythonEnvGate.test.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 加 i18n key**

`src/lib/i18n.ts` 的 `zhTW` 與 `en` 各加入（放在檔案既有的分組註解之後，新增一組 `// Python environment`）：

```ts
    // Python environment
    python_env_title: "Python 環境",
    python_env_preparing: "正在準備 Python 環境…",
    python_env_missing_title: "需要 Python 環境",
    python_env_missing_body: "這項功能需要 Python。AITerm 可以自行安裝一份，不會影響你系統上的 Python。",
    python_env_install: "幫我安裝",
    python_env_recheck: "我自己裝好了，重新偵測",
    python_env_pick_interpreter: "手動指定路徑",
    python_env_rebuild: "重建環境",
    python_env_purge: "完全刪除",
    python_env_legacy_note: "舊版曾以 pip --user 將套件裝進你自己的 Python；那些套件不再被使用，可自行移除。",
    python_env_media_prompt: "這個檔案需要影像／語音支援，要現在安裝嗎？",
```

英文對應（同樣的 key）：

```ts
    // Python environment
    python_env_title: "Python environment",
    python_env_preparing: "Preparing the Python environment…",
    python_env_missing_title: "Python environment needed",
    python_env_missing_body: "This feature needs Python. AITerm can install its own copy without touching the Python on your system.",
    python_env_install: "Install it for me",
    python_env_recheck: "I installed it myself — check again",
    python_env_pick_interpreter: "Choose an interpreter",
    python_env_rebuild: "Rebuild environment",
    python_env_purge: "Delete everything",
    python_env_legacy_note: "Older versions installed packages into your own Python with pip --user. Those are no longer used and can be removed.",
    python_env_media_prompt: "This file needs image/audio support. Install it now?",
```

- [ ] **Step 2: 寫失敗測試**

`src/components/PythonEnv/PythonEnvGate.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { PythonEnvGate } from "./PythonEnvGate";

vi.mock("../../ipc/pythonEnv", () => ({
  pythonEnvStatus: vi.fn(),
  pythonEnvEnsure: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

describe("PythonEnvGate", () => {
  beforeEach(() => vi.clearAllMocks());

  it("shows the log lines it received while installing", () => {
    render(<PythonEnvGate state="installing" lines={[{ text: "Resolved 12 packages", isError: false }]} onInstall={() => {}} onRecheck={() => {}} />);
    expect(screen.getByText("Resolved 12 packages")).toBeTruthy();
  });

  it("offers all three escape hatches when Python is missing", () => {
    render(
      <PythonEnvGate
        state="missing"
        lines={[]}
        onInstall={() => {}}
        onRecheck={() => {}}
        onPickInterpreter={() => {}}
      />,
    );
    expect(screen.getByText(/Install it for me|幫我安裝/)).toBeTruthy();
    expect(screen.getByText(/check again|重新偵測/)).toBeTruthy();
    expect(screen.getByText(/interpreter|手動指定/)).toBeTruthy();
  });

  it("renders nothing once the environment is ready", () => {
    const { container } = render(<PythonEnvGate state="ready" lines={[]} onInstall={() => {}} onRecheck={() => {}} />);
    expect(container.firstChild).toBeNull();
  });
});
```

- [ ] **Step 3: 執行確認失敗**

Run: `npm run test -- PythonEnvGate`
Expected: FAIL — 找不到模組 `./PythonEnvGate`

- [ ] **Step 4: 實作元件**

`src/components/PythonEnv/PythonEnvGate.tsx`。呈現沿用既有 `Settings/McpInstallTerminal.tsx` 的 log 面板寫法（`InstallLogLine` 形狀相同、底部滑出、自動捲到底），差別只在標題與按鈕。

```tsx
import { useEffect, useRef } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { InstallLogLine } from "../Settings/McpInstallTerminal";

export type GateState = "ready" | "installing" | "missing" | "failed";

interface Props {
  state: GateState;
  lines: InstallLogLine[];
  error?: string;
  onInstall: () => void;
  onRecheck: () => void;
  onPickInterpreter?: () => void;
}

export function PythonEnvGate({ state, lines, error, onInstall, onRecheck, onPickInterpreter }: Props) {
  const { t } = useLocale();
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [lines]);

  if (state === "ready") return null;

  return (
    <div className="aiterm-python-gate">
      <h3>{state === "installing" ? t("python_env_preparing") : t("python_env_missing_title")}</h3>

      {state !== "installing" && <p>{t("python_env_missing_body")}</p>}
      {error && <p className="aiterm-python-gate-error">{error}</p>}

      {state !== "installing" && (
        <div className="aiterm-python-gate-actions">
          <button onClick={onInstall}>{t("python_env_install")}</button>
          <button onClick={onRecheck}>{t("python_env_recheck")}</button>
          {onPickInterpreter && (
            <button onClick={onPickInterpreter}>{t("python_env_pick_interpreter")}</button>
          )}
        </div>
      )}

      {lines.length > 0 && (
        <pre className="aiterm-python-gate-log">
          {lines.map((line, i) => (
            <div key={i} className={line.isError ? "is-error" : undefined}>
              {line.text}
            </div>
          ))}
          <div ref={bottomRef} />
        </pre>
      )}
    </div>
  );
}
```

`McpInstallTerminal.tsx` 的 `InstallLogLine` 已 `export`，直接沿用，不另定義形狀。

- [ ] **Step 5: 執行測試**

Run: `npm run test -- PythonEnvGate && npx tsc -b`
Expected: 3 passed，型別檢查通過

- [ ] **Step 6: Commit**

```bash
git add src/components/PythonEnv src/lib/i18n.ts
git commit -m "feat(python-env): add the setup gate with install progress and escape hatches"
```

---

## Task 12: 設定頁「Python 環境」區塊

**Files:**
- Modify: `src/components/Settings/GeneralPage.tsx`
- Test: `src/components/Settings/GeneralPage.pythonEnv.test.tsx`（新）

- [ ] **Step 1: 寫失敗測試**

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { GeneralPage } from "./GeneralPage";

const status = vi.fn();
const reset = vi.fn();

vi.mock("../../ipc/pythonEnv", () => ({
  pythonEnvStatus: () => status(),
  pythonEnvReset: (purge: boolean) => reset(purge),
  pythonEnvSetInterpreter: vi.fn(),
}));

describe("GeneralPage — Python environment", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    status.mockResolvedValue({
      uvAvailable: true,
      pythonVersion: "Python 3.12.7",
      installed: ["doc_core"],
      venvPath: "/data/python-env",
      userInterpreter: null,
    });
  });

  it("shows the resolved Python version and venv path", async () => {
    render(<GeneralPage />);
    await waitFor(() => expect(screen.getByText(/Python 3.12.7/)).toBeTruthy());
    expect(screen.getByText(/\/data\/python-env/)).toBeTruthy();
  });

  it("rebuild keeps downloaded runtimes, delete-everything purges them", async () => {
    render(<GeneralPage />);
    await waitFor(() => expect(status).toHaveBeenCalled());

    await userEvent.click(screen.getByText(/Rebuild environment|重建環境/));
    expect(reset).toHaveBeenCalledWith(false);

    await userEvent.click(screen.getByText(/Delete everything|完全刪除/));
    expect(reset).toHaveBeenCalledWith(true);
  });

  it("explains that the old pip --user packages are no longer used", async () => {
    render(<GeneralPage />);
    await waitFor(() => expect(screen.getByText(/pip --user/)).toBeTruthy());
  });
});
```

- [ ] **Step 2: 執行確認失敗**

Run: `npm run test -- GeneralPage.pythonEnv`
Expected: FAIL — 找不到「重建環境」等文字

- [ ] **Step 3: 實作區塊**

在 `GeneralPage.tsx` 既有設定區塊的樣式與排版慣例下新增一節。狀態在掛載時取得，重建／刪除後重新取得：

```tsx
const [pyEnv, setPyEnv] = useState<PythonEnvStatus | null>(null);

const refreshPyEnv = useCallback(() => {
  pythonEnvStatus().then(setPyEnv).catch(() => setPyEnv(null));
}, []);

useEffect(refreshPyEnv, [refreshPyEnv]);

const handleReset = async (purge: boolean) => {
  // Purging removes downloaded interpreters too, so it's the one that needs a
  // confirmation — a rebuild is cheap and recoverable.
  if (purge && !window.confirm(t("python_env_purge_confirm"))) return;
  await pythonEnvReset(purge);
  refreshPyEnv();
};
```

區塊本體（沿用該檔既有的 section/label class 命名）：

```tsx
<section className="settings-section">
  <h3>{t("python_env_title")}</h3>
  <dl>
    <dt>{t("python_env_version_label")}</dt>
    <dd>{pyEnv?.pythonVersion ?? t("python_env_not_created")}</dd>
    <dt>{t("python_env_path_label")}</dt>
    <dd><code>{pyEnv?.venvPath ?? "—"}</code></dd>
    <dt>{t("python_env_source_label")}</dt>
    <dd>{pyEnv?.userInterpreter ?? t("python_env_source_bundled")}</dd>
    <dt>{t("python_env_installed_label")}</dt>
    <dd>{pyEnv?.installed.join(", ") || t("python_env_none_installed")}</dd>
  </dl>
  <div className="settings-actions">
    <button onClick={() => handleReset(false)}>{t("python_env_rebuild")}</button>
    <button onClick={() => handleReset(true)}>{t("python_env_purge")}</button>
  </div>
  <p className="settings-hint">{t("python_env_legacy_note")}</p>
</section>
```

Task 11 的 i18n 清單再補這六個 key（zhTW／en 各一份）：

```ts
    python_env_version_label: "Python 版本",          // "Python version"
    python_env_path_label: "環境路徑",                // "Environment path"
    python_env_source_label: "Interpreter 來源",       // "Interpreter source"
    python_env_source_bundled: "內建",                // "Bundled"
    python_env_installed_label: "已安裝套件組",        // "Installed profiles"
    python_env_none_installed: "尚未安裝",            // "None yet"
    python_env_not_created: "尚未建立",               // "Not created yet"
    python_env_purge_confirm: "確定要刪除整個 Python 環境嗎？下次使用需要重新下載。",
    // "Delete the whole Python environment? The next use will download it again."
```

- [ ] **Step 4: 執行測試**

Run: `npm run test -- GeneralPage && npx tsc -b`
Expected: 全數 passed

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings
git commit -m "feat(settings): surface Python environment state with rebuild and purge"
```

---

## Task 13: DocConverter 的候選安裝

**Files:**
- Modify: `src/components/DocConverter/DocConverterView.tsx`
- Test: `src/components/DocConverter/DocConverterView.mediaProfile.test.tsx`（新）

- [ ] **Step 1: 寫失敗測試**

```tsx
import { describe, it, expect } from "vitest";
import { needsMediaProfile } from "./DocConverterView";

describe("needsMediaProfile", () => {
  it("is true for image and audio files", () => {
    for (const name of ["a.png", "b.JPG", "c.jpeg", "d.gif", "e.bmp", "f.webp", "g.mp3", "h.wav", "i.m4a", "j.flac"]) {
      expect(needsMediaProfile(name)).toBe(true);
    }
  });

  it("is false for the document formats the core profile covers", () => {
    for (const name of ["a.pdf", "b.docx", "c.pptx", "d.xlsx", "e.txt"]) {
      expect(needsMediaProfile(name)).toBe(false);
    }
  });

  it("is false when there is no extension", () => {
    expect(needsMediaProfile("README")).toBe(false);
  });
});
```

- [ ] **Step 2: 執行確認失敗**

Run: `npm run test -- DocConverterView.mediaProfile`
Expected: FAIL — `needsMediaProfile` is not exported

- [ ] **Step 3: 實作**

`DocConverterView.tsx` 加入並 export：

```tsx
/** Extensions that need the on-demand media profile. Mirrors the formats
 *  converter.py handles via markitdown's image/audio extras. */
const MEDIA_EXTENSIONS = new Set([
  "png", "jpg", "jpeg", "gif", "bmp", "webp",
  "mp3", "wav", "m4a", "flac",
]);

export function needsMediaProfile(fileName: string): boolean {
  const dot = fileName.lastIndexOf(".");
  if (dot < 0) return false;
  return MEDIA_EXTENSIONS.has(fileName.slice(dot + 1).toLowerCase());
}
```

轉換流程中，選定檔案後先確保 media profile：

```tsx
/** Returns false when the user declined the extra download, in which case the
 *  conversion is skipped rather than run and fail inside converter.py. */
const ensureMediaProfileIfNeeded = async (filePath: string): Promise<boolean> => {
  if (!needsMediaProfile(filePath)) return true;

  const status = await pythonEnvStatus();
  if (status.installed.includes("doc_media")) return true;

  if (!window.confirm(t("python_env_media_prompt"))) return false;

  setGateState("installing");
  try {
    await pythonEnvEnsure("doc_media");
    return true;
  } finally {
    setGateState("ready");
  }
};
```

呼叫端（既有的轉換 handler 開頭）：

```tsx
if (!(await ensureMediaProfileIfNeeded(path))) return;
```

安裝過程的 log 行由 `python-env-log` 事件累積進 `lines` state 並交給 `PythonEnvGate` 顯示：

```tsx
useEffect(() => {
  const unlisten = listen<PythonEnvLogEvent>("python-env-log", (e) => {
    setLines((prev) => [...prev, { text: e.payload.message, isError: e.payload.level === "error" }]);
  });
  return () => { unlisten.then((off) => off()); };
}, []);
```

- [ ] **Step 4: 執行測試**

Run: `npm run test -- DocConverterView && npx tsc -b`
Expected: 全數 passed

- [ ] **Step 5: Commit**

```bash
git add src/components/DocConverter
git commit -m "feat(doc-converter): offer the media profile only when the file needs it"
```

---

## Task 14: CI 打包 uv 與手動驗證

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: 加入 CI 步驟**

在既有 DB2 sidecar 的三個平台步驟（`release.yml:234`、`:242`、`:250`）之後，各加一個對應的 uv 步驟，沿用同樣的條件式與 shell：

```yaml
      - name: Fetch uv (macOS)
        if: matrix.platform == 'macos'
        run: bash scripts/setup-uv-mac.sh

      - name: Fetch uv (Windows)
        if: matrix.platform == 'windows'
        run: pwsh -ExecutionPolicy Bypass -File scripts\setup-uv-win.ps1

      - name: Fetch uv (Linux)
        if: matrix.platform == 'linux'
        run: ARCH=${{ matrix.db2_arch }} bash scripts/setup-uv-linux.sh
```

（`matrix.db2_arch` 是 matrix 裡既有的 x64／arm64 值，直接沿用，不新增 matrix 變數。）

**Linux 的條件式不可照抄 db2 的。** db2 的 Linux 步驟只跑在 `.deb` 兩條 leg（`ubuntu-24.04` / `ubuntu-24.04-arm`），因為 AppImage 刻意跳過 DB2。但 uv 不同：`tauri.linux.conf.json` 的 `externalBin` 對**全部 4 條** Linux leg 都是 `["binaries/uv"]`（AppImage 兩條 leg 用的是 repo 內的版本，不會被 CI 覆寫），所以 AppImage 的 `tauri build` 同樣需要 `binaries/uv-<triple>` 存在，否則會撞上與 `.deb` 相同的 resource-path 驗證失敗。uv 的 Linux fetch 步驟必須涵蓋 `ubuntu-22.04`、`ubuntu-22.04-arm`、`ubuntu-24.04`、`ubuntu-24.04-arm` 全部四條。

**為什麼這裡必須用真的 fetch 而不是 placeholder**：`ci.yml` 的 warm-cache job 只編譯不打包，所以 0-byte placeholder 足夠（實測：缺檔 `cargo check` 失敗、0-byte 通過）；但 release.yml 會真的 bundle，空檔會產出一個帶著 0-byte uv 的安裝檔，執行期才爆。兩者不可混用。

- [ ] **Step 1b: 修正 Linux conf 的重新生成（否則前面全部白做）**

`release.yml:252-269` 的「Patch tauri.linux.conf.json」步驟會用 python 重新生成整個 `tauri.linux.conf.json`，**覆蓋掉 repo 裡的版本**。它目前寫死 `'externalBin': []`，會把 Task 1 加的 uv 登記清掉；而且它的 `resources` 只列 ApiDocFetcher，**漏了 MarkItDown 的兩個條目**（repo 版本 `tauri.linux.conf.json:8-9` 有）——這是既有缺陷，代表現在打包出來的 Linux `.deb` 根本沒有 `converter.py`，文件轉換與知識庫匯入在 Linux 上必定失敗。順手一起修，因為本任務正要往同一份 resources 再加一個檔案。

該步驟的 python heredoc 改為：

```python
          conf = {
            'bundle': {
              'externalBin': ['binaries/uv'],
              'resources': {
                '${{ matrix.db2_sidecar_dir }}': 'db2-sidecar',
                '../tools/ApiDocFetcher/*.py': 'ApiDocFetcher/',
                '../tools/ApiDocFetcher/strategies/*.py': 'ApiDocFetcher/strategies/',
                '../tools/ApiDocFetcher/requirements.txt': 'ApiDocFetcher/requirements.txt',
                '../tools/MarkItDown/converter.py': 'MarkItDown/converter.py',
                '../tools/MarkItDown/requirements.txt': 'MarkItDown/requirements.txt'
              }
            }
          }
```

（`converter.py` 與 `requirements.txt` 這兩行已由 master 上的 hotfix `27b280b` 補上，此處只需確認 `externalBin` 帶上 `binaries/uv`。）

**不要在這裡加 `requirements-media.txt`。** Task 3 把它放進 base `tauri.conf.json` 的 `resources`，而 `resources` 是物件、走遞迴合併，所以 base 的項目在每個平台都生效 —— 包含這份被 CI 整檔重寫的 Linux conf（重寫只覆蓋平台 conf，動不到 base）。在這裡重複列出不會壞，但會讓「media 條目歸屬於 base」這個刻意的決定變模糊。

改完後比對一次：這份生成的內容除了 `db2_sidecar_dir` 之外，應與 repo 的 `src-tauri/tauri.linux.conf.json` 完全一致 —— 注意兩者都**不該**含 media 條目，那一項只在 base。

同時確認 macOS／Windows 兩邊：`release.yml:271` 之後的 macOS 注入步驟與 `tauri.windows.conf.json` 若也會覆寫 `externalBin`，要一併帶上 `binaries/uv`。

- [ ] **Step 2: 驗證 workflow 語法**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 3: 本機端到端驗證**

Run: `npm run tauri:dev`

依序確認：

1. 刪掉 `~/Library/Application Support/AITerm/python-env`，開啟文件轉換並選一個 PDF → 應看到進度面板、安裝完成、轉換成功
2. 再轉一次同檔 → 不應再出現安裝步驟（標記檔生效）
3. 選一張 PNG → 應出現候選安裝提示
4. 設定頁 →「Python 環境」顯示版本與路徑，「重建環境」後版本仍可取得
5. 知識庫匯入一個 PDF → 走同一條路徑，不再要求系統 Python

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: ship the uv binary with every platform build"
```

---

## 完成後的手動驗證表（三平台，需真實環境）

| 平台 | 已有合格 Python | 完全沒有 Python |
|---|---|---|
| macOS | 手動指定 → venv 建立 → 轉換成功 | 「幫我安裝」→ uv 取得 Python → 轉換成功 |
| Windows | 同上，並確認不會誤用 WindowsApps stub | 同上，並確認無 UAC 提示 |
| Linux | 同上 | 同上，並確認不需 sudo |

Spec 末尾的三個待驗證假設在此階段確認：uv 對 `markitdown[...]` extras 的解析是否與 pip 一致（`curl_cffi` 可能需就地編譯 → 應觸發 `ToolchainMissing` 而非通用錯誤）、uv binary 對安裝檔的實際增量、`UV_PYTHON_INSTALL_DIR` 在 Windows 的路徑長度表現。
