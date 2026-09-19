# 讓 AITerm 被作業系統當成終端機（地基＋macOS＋Linux）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 從 Finder／檔案管理員開資料夾或 `.command`／`.sh`、以及 Linux 上的 `x-terminal-emulator -e … / --working-directory=…`，都會在執行中的 AITerm 開一個新分頁。

**Architecture:** Rust 端新增 `launch` 模組：純函式 `parse_args` 把 argv 解析成 `LaunchRequest`；冷啟動 argv、`tauri-plugin-single-instance`、macOS `RunEvent::Opened` 三個入口都收斂進同一個 `LaunchQueue`，前端 `TerminalApp` 先訂閱事件、再呼叫 `take_launch_requests()` 排空（避開「事件先於訂閱」的舊坑）。前端用既有的 `handlePickerSelect("terminal", {initialCwd})` 開分頁；要跑的指令由 `TerminalView` 內的「就緒注入器」在 shell 就緒後送出；`.command`／`.sh` 要先過應用內確認。系統註冊靠 macOS `Info.plist` 合併檔與 Linux `.desktop` 樣板＋deb 維護腳本。

**Tech Stack:** Rust（Tauri 2.10、`tauri-plugin-single-instance`、`url`、`tempfile` 測試）、React 19 + Vitest、shell 維護腳本。

**Spec:** `docs/superpowers/specs/2026-09-19-open-as-terminal-mac-linux-design.md`

**與 spec 的一處刻意差異：** spec 寫「`script` 請求先開分頁、再跳確認」。實作改成「先確認、再開分頁」——因為 `TerminalView` 的 `initialCommand` 只在掛載時讀取，先開分頁就得另外做「事後送指令」的通道。使用者看到的結果相同：確認 → 開在該目錄並執行；取消 → 開在該目錄不執行。Task 12 會把 spec 這一行改成與實作一致。

**全域注意事項（每個 Task 都適用）：**
- 所有 Rust 指令都在 `src-tauri/` 下跑；`build.rs` 會檢查 `externalBin` 檔案存在，這台 macOS 已有 `binaries/uv-aarch64-apple-darwin`，若缺就先跑 `scripts/setup-uv-mac.sh`。
- `git add` 一律只加明確路徑，**不要** `git add -A`（工作樹有 `.claude/settings.local.json` 的未提交修改，與本計畫無關）。
- `docs/superpowers/` 被 `.gitignore` 忽略，這個目錄下的檔案要 `git add -f`。
- commit 訊息結尾加 `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>`。
- 每個新測試都必須先看到它失敗（紅）再寫實作；若某個測試一開始就綠，代表它沒測到東西，先停下來查。

---

## File Structure

| 檔案 | 動作 | 職責 |
|------|------|------|
| `src-tauri/src/launch/mod.rs` | 建立 | 匯出型別；`take_launch_requests` command；`enqueue_and_notify`；單例／Opened 入口函式 |
| `src-tauri/src/launch/parse.rs` | 建立 | `LaunchRequest`、`parse_args`、`args_from_file_urls`（純函式，可單元測試） |
| `src-tauri/src/launch/queue.rs` | 建立 | `LaunchQueue`（push／take） |
| `src-tauri/src/lib.rs` | 修改 | `pub mod launch;`、註冊單例外掛、`.manage(LaunchQueue)`、setup 冷啟動、`RunEvent::Opened`、註冊 command |
| `src-tauri/Cargo.toml` | 修改 | 加 `tauri-plugin-single-instance` |
| `src/ipc/launch.ts` | 建立 | `LaunchRequest` 型別、`takeLaunchRequests`、`onLaunchRequestPending` |
| `src/lib/launchRequest.ts` | 建立 | `planLaunch`、`quoteArg`（純函式） |
| `src/lib/startupCommand.ts` | 建立 | `createStartupInjector`：shell 就緒後送出指令 |
| `src/components/TabBar/index.tsx` | 修改 | `Tab` 加 `initialCommand?` |
| `src/components/NewTabPicker/tabCatalog.ts` | 修改 | `TabOpenOpts` 加 `initialCommand?` |
| `src/components/TerminalView.tsx` | 修改 | 新 prop `initialCommand`，掛上注入器 |
| `src/components/LaunchScriptConfirm/index.tsx` + `index.css` | 建立 | 「要執行這個腳本嗎？」確認框 |
| `src/lib/i18n.ts` | 修改 | 四個 `launch_script_*` 字串（zh-TW、en 兩邊） |
| `src/components/TerminalApp.tsx` | 修改 | 訂閱＋排空、處理請求、掛確認框 |
| `src-tauri/Info.plist` | 建立 | macOS 文件類型宣告（資料夾、`.command`、`.sh`） |
| `src-tauri/linux/aiterm.desktop` | 建立 | `.desktop` 樣板 |
| `src-tauri/linux/postinst.sh`、`prerm.sh` | 建立 | deb 的 `update-alternatives` 註冊／移除 |
| `src-tauri/tauri.linux.conf.json` | 修改 | 指向上述三個檔案 |
| `src-tauri/tests/os_registration.rs` | 建立 | 驗證 Info.plist、`.desktop`、deb 腳本行為 |
| 各 `*.test.ts(x)` | 建立 | 見各 Task |

---

### Task 1: `parse_args`（Rust 純函式）

**Files:**
- Create: `src-tauri/src/launch/mod.rs`
- Create: `src-tauri/src/launch/parse.rs`
- Modify: `src-tauri/src/lib.rs`（頂部 `pub mod` 清單）

- [ ] **Step 1: 建立模組骨架與失敗的測試**

`src-tauri/src/launch/mod.rs`：

```rust
//! 讓 AITerm 被當成終端機使用：把「開這個資料夾／跑這個指令」的請求，
//! 從冷啟動 argv、第二次啟動、macOS Opened 三個入口收斂成同一個佇列。

pub mod parse;

pub use parse::{args_from_file_urls, parse_args, LaunchRequest};
```

`src-tauri/src/launch/parse.rs`（先只放型別、空實作與測試）：

```rust
use serde::Serialize;
use std::path::Path;

/// 一次「開新分頁」的請求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaunchRequest {
    /// 新分頁的起始目錄。
    pub cwd: Option<String>,
    /// `.command`／`.sh` 檔的路徑。前端要先讓使用者確認才會執行。
    pub script: Option<String>,
    /// `-e` 之後的 argv，shell 就緒後直接送進去。
    pub command: Option<Vec<String>>,
}

pub fn parse_args(_argv: &[String], _invoking_cwd: Option<&Path>) -> Vec<LaunchRequest> {
    Vec::new()
}

pub fn args_from_file_urls(_urls: &[url::Url]) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn argv(parts: &[&str]) -> Vec<String> {
        std::iter::once("aiterm").chain(parts.iter().copied()).map(String::from).collect()
    }

    fn p(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_directory_argument_becomes_a_cwd_request() {
        let dir = tempfile::tempdir().unwrap();
        let got = parse_args(&argv(&[&p(dir.path())]), None);
        assert_eq!(
            got,
            vec![LaunchRequest { cwd: Some(p(dir.path())), script: None, command: None }]
        );
    }

    #[test]
    fn a_command_script_opens_its_parent_dir_and_records_the_script() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("deploy.command");
        fs::write(&script, "#!/bin/sh\n").unwrap();
        let got = parse_args(&argv(&[&p(&script)]), None);
        assert_eq!(
            got,
            vec![LaunchRequest { cwd: Some(p(dir.path())), script: Some(p(&script)), command: None }]
        );
    }

    #[test]
    fn a_sh_script_is_treated_like_a_command_script() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("run.SH"); // 副檔名大小寫不分
        fs::write(&script, "").unwrap();
        let got = parse_args(&argv(&[&p(&script)]), None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].script, Some(p(&script)));
    }

    #[test]
    fn a_plain_file_that_is_not_a_script_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        fs::write(&file, "").unwrap();
        assert!(parse_args(&argv(&[&p(&file)]), None).is_empty());
    }

    #[test]
    fn a_missing_path_is_ignored() {
        assert!(parse_args(&argv(&["/definitely/not/here/aiterm-test"]), None).is_empty());
    }

    #[test]
    fn working_directory_accepts_both_spellings() {
        let dir = tempfile::tempdir().unwrap();
        let d = p(dir.path());
        let eq = parse_args(&argv(&[&format!("--working-directory={d}")]), None);
        let sp = parse_args(&argv(&["--working-directory", &d]), None);
        let want = vec![LaunchRequest { cwd: Some(d.clone()), script: None, command: None }];
        assert_eq!(eq, want);
        assert_eq!(sp, want);
    }

    #[test]
    fn dash_e_swallows_every_following_argument_as_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let d = p(dir.path());
        // `-e` 之後的 `--working-directory` 是指令自己的參數，不是我們的旗標。
        let got = parse_args(&argv(&["-e", "ls", "-la", "--working-directory", &d]), Some(dir.path()));
        assert_eq!(
            got,
            vec![LaunchRequest {
                cwd: Some(d.clone()), // 沒給 --working-directory 時退回呼叫端的 cwd
                script: None,
                command: Some(vec!["ls".into(), "-la".into(), "--working-directory".into(), d]),
            }]
        );
    }

    #[test]
    fn working_directory_and_dash_e_combine_into_one_request() {
        let wd = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let got = parse_args(
            &argv(&["--working-directory", &p(wd.path()), "-e", "htop"]),
            Some(other.path()),
        );
        assert_eq!(
            got,
            vec![LaunchRequest {
                cwd: Some(p(wd.path())),
                script: None,
                command: Some(vec!["htop".into()]),
            }]
        );
    }

    #[test]
    fn a_dash_e_with_nothing_after_it_produces_no_request() {
        assert!(parse_args(&argv(&["-e"]), None).is_empty());
    }

    #[test]
    fn relative_paths_resolve_against_the_invoking_cwd() {
        let base = tempfile::tempdir().unwrap();
        fs::create_dir(base.path().join("proj")).unwrap();
        let got = parse_args(&argv(&["proj"]), Some(base.path()));
        assert_eq!(got[0].cwd, Some(p(&base.path().join("proj"))));
        // "." 要被正規化掉，不能留下 `/x/.`
        let dot = parse_args(&argv(&["."]), Some(base.path()));
        assert_eq!(dot[0].cwd, Some(p(base.path())));
    }

    #[test]
    fn unknown_flags_and_headless_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let got = parse_args(&argv(&["--headless", "-psn_0_12345", &p(dir.path())]), None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cwd, Some(p(dir.path())));
    }

    #[test]
    fn multiple_paths_make_multiple_requests_in_order() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let got = parse_args(&argv(&[&p(a.path()), &p(b.path())]), None);
        assert_eq!(got.iter().map(|r| r.cwd.clone().unwrap()).collect::<Vec<_>>(), vec![p(a.path()), p(b.path())]);
    }

    #[cfg(unix)]
    #[test]
    fn file_urls_become_decoded_path_arguments() {
        let urls = vec![url::Url::parse("file:///tmp/a%20b").unwrap()];
        assert_eq!(args_from_file_urls(&urls), vec!["aiterm".to_string(), "/tmp/a b".to_string()]);
    }

    #[test]
    fn non_file_urls_are_dropped() {
        let urls = vec![url::Url::parse("https://example.com/x").unwrap()];
        assert_eq!(args_from_file_urls(&urls), vec!["aiterm".to_string()]);
    }
}
```

`src-tauri/src/lib.rs`：在頂部 `pub mod knowledge_base;` 後面加一行 `pub mod launch;`（保持字母順序）。

- [ ] **Step 2: 跑測試確認全部失敗**

Run: `cd src-tauri && cargo test --lib launch::parse -- --nocapture`
Expected: 編譯成功，多數測試 FAIL（`parse_args` 目前回傳空 Vec；`non_file_urls_are_dropped` 因為空實作回傳 `[]` 而不是 `["aiterm"]` 也會 FAIL）。若有測試一開始就綠，停下來檢查。

> 註：這裡用 `--lib` 只是為了在開發中快速迭代；最後驗證（Task 12）必須跑完整的 `cargo test --workspace --no-fail-fast`。

- [ ] **Step 3: 實作**

把 `parse.rs` 中兩個函式換成（型別與 `tests` 模組不動）：

```rust
use std::path::PathBuf;

const SCRIPT_EXTENSIONS: [&str; 2] = ["command", "sh"];

fn resolve(raw: &str, invoking_cwd: Option<&Path>) -> PathBuf {
    let path = Path::new(raw);
    let joined = match invoking_cwd {
        Some(base) if path.is_relative() => base.join(path),
        _ => path.to_path_buf(),
    };
    // components() 會吃掉中間的 `.`，避免出現 `/x/.`。
    joined.components().collect()
}

fn existing_dir(raw: &str, invoking_cwd: Option<&Path>) -> Option<String> {
    let path = resolve(raw, invoking_cwd);
    path.is_dir().then(|| path.to_string_lossy().into_owned())
}

fn positional(raw: &str, invoking_cwd: Option<&Path>) -> Option<LaunchRequest> {
    let path = resolve(raw, invoking_cwd);
    if path.is_dir() {
        return Some(LaunchRequest { cwd: Some(path.to_string_lossy().into_owned()), script: None, command: None });
    }
    let is_script = path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SCRIPT_EXTENSIONS.iter().any(|s| s.eq_ignore_ascii_case(e)));
    if !is_script {
        return None;
    }
    let parent = path.parent()?.to_string_lossy().into_owned();
    Some(LaunchRequest {
        cwd: Some(parent),
        script: Some(path.to_string_lossy().into_owned()),
        command: None,
    })
}

/// 把命令列參數解析成開分頁請求。`argv[0]` 是程式本身，會被略過。
///
/// - 位置參數：資料夾 → 開在該處；`.command`／`.sh` → 開在其父目錄並記下腳本路徑。
/// - `--working-directory=X`／`--working-directory X`：起始目錄。
/// - `-e`／`--command`／`-x`：其後**所有** argv 都是要跑的指令（x-terminal-emulator 慣例）。
/// - 不存在的路徑、無法辨識的旗標一律略過。
pub fn parse_args(argv: &[String], invoking_cwd: Option<&Path>) -> Vec<LaunchRequest> {
    let mut requests = Vec::new();
    let mut working_dir: Option<String> = None;
    let mut command: Option<Vec<String>> = None;

    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].as_str();
        match arg {
            "-e" | "--command" | "-x" => {
                let rest = &argv[i + 1..];
                if !rest.is_empty() {
                    command = Some(rest.to_vec());
                }
                break;
            }
            "--working-directory" => {
                if let Some(value) = argv.get(i + 1) {
                    working_dir = existing_dir(value, invoking_cwd);
                    i += 1;
                }
            }
            _ if arg.starts_with("--working-directory=") => {
                working_dir = existing_dir(&arg["--working-directory=".len()..], invoking_cwd);
            }
            _ if arg.starts_with('-') => {}
            _ => {
                if let Some(request) = positional(arg, invoking_cwd) {
                    requests.push(request);
                }
            }
        }
        i += 1;
    }

    if command.is_some() || working_dir.is_some() {
        // 只有 `-e` 沒給目錄時，退回呼叫端的 cwd——檔案管理員叫終端機時，
        // 那才是使用者當下所在的位置。
        let cwd = if command.is_some() {
            working_dir.or_else(|| invoking_cwd.map(|p| p.to_string_lossy().into_owned()))
        } else {
            working_dir
        };
        requests.push(LaunchRequest { cwd, script: None, command });
    }
    requests
}

/// macOS `RunEvent::Opened` 給的是 `file://` URL；轉成路徑後餵給 `parse_args`。
pub fn args_from_file_urls(urls: &[url::Url]) -> Vec<String> {
    let mut argv = vec![String::from("aiterm")];
    argv.extend(
        urls.iter()
            .filter_map(|u| u.to_file_path().ok())
            .map(|p| p.to_string_lossy().into_owned()),
    );
    argv
}
```

- [ ] **Step 4: 跑測試確認全綠**

Run: `cd src-tauri && cargo test --lib launch::parse`
Expected: 全部 PASS。

- [ ] **Step 5: 突變檢查（證明測試真的咬得到）**

暫時把 `parse_args` 裡 `"-e" | "--command" | "-x"` 分支的 `break;` 刪掉，跑 `cargo test --lib launch::parse`。
Expected: `dash_e_swallows_every_following_argument_as_the_command` FAIL。看到紅之後還原 `break;`，再跑一次確認全綠。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/launch/mod.rs src-tauri/src/launch/parse.rs src-tauri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(launch): parse terminal-style launch arguments into open-tab requests

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `LaunchQueue` 與 `take_launch_requests`

**Files:**
- Create: `src-tauri/src/launch/queue.rs`
- Modify: `src-tauri/src/launch/mod.rs`

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/launch/queue.rs`：

```rust
use super::parse::LaunchRequest;
use std::sync::Mutex;

/// 還沒被前端取走的請求。請求先存在這裡、再通知前端，前端訂閱之前入列的
/// 請求不會遺失——事件本身不承載資料。
#[derive(Default)]
pub struct LaunchQueue(Mutex<Vec<LaunchRequest>>);

impl LaunchQueue {
    pub fn push(&self, _requests: Vec<LaunchRequest>) {}

    /// 取走並清空。
    pub fn take(&self) -> Vec<LaunchRequest> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(cwd: &str) -> LaunchRequest {
        LaunchRequest { cwd: Some(cwd.into()), script: None, command: None }
    }

    #[test]
    fn take_returns_pushed_requests_in_order_and_empties_the_queue() {
        let q = LaunchQueue::default();
        q.push(vec![req("/a")]);
        q.push(vec![req("/b"), req("/c")]);
        assert_eq!(q.take(), vec![req("/a"), req("/b"), req("/c")]);
        assert!(q.take().is_empty(), "second take must find nothing");
    }

    #[test]
    fn take_on_an_untouched_queue_is_empty() {
        assert!(LaunchQueue::default().take().is_empty());
    }
}
```

`mod.rs` 改成：

```rust
pub mod parse;
pub mod queue;

pub use parse::{args_from_file_urls, parse_args, LaunchRequest};
pub use queue::LaunchQueue;
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd src-tauri && cargo test --lib launch::queue`
Expected: `take_returns_pushed_requests_in_order_and_empties_the_queue` FAIL（回傳空 Vec）。

- [ ] **Step 3: 實作**

把 `push`／`take` 換成：

```rust
    pub fn push(&self, requests: Vec<LaunchRequest>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).extend(requests);
    }

    /// 取走並清空。
    pub fn take(&self) -> Vec<LaunchRequest> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
```

- [ ] **Step 4: 加上 Tauri 端點（`mod.rs`）**

在 `mod.rs` 檔尾加入：

```rust
use tauri::{AppHandle, Emitter, Manager, State};

/// 有新請求入列時發給前端的事件（無酬載——前端收到後自己 `take_launch_requests`）。
pub const PENDING_EVENT: &str = "launch-request-pending";

/// 前端取走所有待處理請求。
#[tauri::command]
pub fn take_launch_requests(queue: State<'_, LaunchQueue>) -> Vec<LaunchRequest> {
    queue.take()
}

/// 入列並通知前端。沒有請求就什麼都不做。
pub fn enqueue_and_notify(app: &AppHandle, requests: Vec<LaunchRequest>) {
    if requests.is_empty() {
        return;
    }
    app.state::<LaunchQueue>().push(requests);
    if let Err(e) = app.emit(PENDING_EVENT, ()) {
        log::warn!("emit {PENDING_EVENT} failed: {e}");
    }
}

/// 第二次啟動（single-instance 外掛的 callback）：解析、入列、把視窗拉到前景。
pub fn on_second_instance(app: &AppHandle, argv: Vec<String>, cwd: String) {
    let requests = parse_args(&argv, Some(std::path::Path::new(&cwd)));
    enqueue_and_notify(app, requests);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// macOS「用 AITerm 開啟」／拖到 Dock 圖示。其它平台沒有這個事件。
pub fn on_run_event(app: &AppHandle, event: &tauri::RunEvent) {
    #[cfg(target_os = "macos")]
    {
        if let tauri::RunEvent::Opened { urls } = event {
            let argv = args_from_file_urls(urls);
            enqueue_and_notify(app, parse_args(&argv, None));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, event);
    }
}
```

- [ ] **Step 5: 跑測試確認全綠、且能編譯**

Run: `cd src-tauri && cargo test --lib launch && cargo check`
Expected: `launch::parse`、`launch::queue` 全 PASS；`cargo check` 成功（尚未註冊 command，會有 dead_code 警告可忽略）。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/launch/queue.rs src-tauri/src/launch/mod.rs
git commit -m "$(cat <<'EOF'
feat(launch): queue launch requests and expose take_launch_requests

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 接進 `lib.rs`（單例外掛、冷啟動、Opened）

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 加依賴**

`src-tauri/Cargo.toml` 的 `[dependencies]`，緊接在 `tauri-plugin-process = "2"` 後面加：

```toml
tauri-plugin-single-instance = "2"
```

- [ ] **Step 2: 註冊外掛（必須是第一個 plugin）**

`src-tauri/src/lib.rs`，在 `tauri::Builder::default()` 後、`.plugin(tauri_plugin_dialog::init())` 前插入：

```rust
        // single-instance 必須是第一個外掛（官方文件要求）。第二次啟動會把
        // argv 與 cwd 轉給這個實例，而不是另開一個行程。
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            launch::on_second_instance(app, argv, cwd);
        }))
```

- [ ] **Step 3: 註冊 state、command**

在 `.manage(Arc::new(PtyManager::new()))` 前面加一行：

```rust
        .manage(launch::LaunchQueue::default())
```

在 `tauri::generate_handler![` 清單最後（`reports_delete,` 之後、`]` 之前）加：

```rust
            // 啟動請求（開資料夾／-e 指令）
            launch::take_launch_requests,
```

- [ ] **Step 4: 冷啟動 argv**

在 `.setup(|app| {` 的第一行（`telegram::init(app.handle());` 之前）加：

```rust
            // 冷啟動時帶進來的參數（`aiterm ~/proj`、`--working-directory=…`）。
            // 只入列不必先通知：前端一掛載就會排空。
            launch::enqueue_and_notify(
                app.handle(),
                launch::parse_args(
                    &std::env::args().collect::<Vec<_>>(),
                    std::env::current_dir().ok().as_deref(),
                ),
            );
```

- [ ] **Step 5: macOS Opened**

`.run(|app_handle, event| {` 的第一行（既有那段長註解之前）加：

```rust
            launch::on_run_event(app_handle, &event);
```

- [ ] **Step 6: 確認可編譯、既有測試不受影響**

Run: `cd src-tauri && cargo check && cargo test --lib launch`
Expected: 成功；`Cargo.lock` 會多出 `tauri-plugin-single-instance` 及其相依。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(launch): route cold-start argv, second instances and macOS Opened into the queue

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 前端 IPC 包裝

**Files:**
- Create: `src/ipc/launch.ts`
- Test: `src/ipc/launch.test.ts`

- [ ] **Step 1: 寫失敗的測試**

`src/ipc/launch.test.ts`：

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));

import { takeLaunchRequests, onLaunchRequestPending } from "./launch";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
});

describe("launch ipc", () => {
  it("takeLaunchRequests invokes take_launch_requests and returns its result", async () => {
    invokeMock.mockResolvedValue([{ cwd: "/a", script: null, command: null }]);
    await expect(takeLaunchRequests()).resolves.toEqual([{ cwd: "/a", script: null, command: null }]);
    expect(invokeMock).toHaveBeenCalledWith("take_launch_requests");
  });

  it("onLaunchRequestPending listens on launch-request-pending and ignores the payload", async () => {
    let handler: ((e: { payload: unknown }) => void) | undefined;
    listenMock.mockImplementation((_name: string, h: (e: { payload: unknown }) => void) => {
      handler = h;
      return Promise.resolve(() => {});
    });
    const cb = vi.fn();
    await onLaunchRequestPending(cb);
    expect(listenMock.mock.calls[0][0]).toBe("launch-request-pending");
    handler?.({ payload: null });
    expect(cb).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `npx vitest run src/ipc/launch.test.ts`
Expected: FAIL（找不到 `./launch`）。

- [ ] **Step 3: 實作**

`src/ipc/launch.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 對應 Rust 端 `launch::LaunchRequest`。 */
export interface LaunchRequest {
  cwd: string | null;
  /** `.command`／`.sh` 檔路徑，要使用者確認才能執行。 */
  script: string | null;
  /** `-e` 之後的 argv。 */
  command: string[] | null;
}

/** 取走並清空後端佇列。 */
export function takeLaunchRequests(): Promise<LaunchRequest[]> {
  return invoke<LaunchRequest[]>("take_launch_requests");
}

/** 後端有新請求入列時觸發（事件本身不帶資料，收到後要自己 `takeLaunchRequests`）。 */
export function onLaunchRequestPending(cb: () => void): Promise<UnlistenFn> {
  return listen("launch-request-pending", () => cb());
}
```

- [ ] **Step 4: 跑測試確認全綠**

Run: `npx vitest run src/ipc/launch.test.ts`
Expected: 2 tests PASS。

- [ ] **Step 5: Commit**

```bash
git add src/ipc/launch.ts src/ipc/launch.test.ts
git commit -m "$(cat <<'EOF'
feat(launch): typed IPC wrappers for launch requests

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `planLaunch` 與引號處理（純函式）

**Files:**
- Create: `src/lib/launchRequest.ts`
- Test: `src/lib/launchRequest.test.ts`

- [ ] **Step 1: 寫失敗的測試**

`src/lib/launchRequest.test.ts`：

```ts
import { describe, it, expect } from "vitest";
import { planLaunch, quoteArg } from "./launchRequest";

describe("quoteArg", () => {
  it("leaves safe POSIX words alone", () => {
    expect(quoteArg("ls", false)).toBe("ls");
    expect(quoteArg("/usr/local/bin/x-1.2", false)).toBe("/usr/local/bin/x-1.2");
  });

  it("single-quotes POSIX arguments containing spaces or shell metacharacters", () => {
    expect(quoteArg("/tmp/a b", false)).toBe("'/tmp/a b'");
    expect(quoteArg("a;rm -rf /", false)).toBe("'a;rm -rf /'");
  });

  it("escapes embedded single quotes the POSIX way", () => {
    expect(quoteArg("it's", false)).toBe("'it'\\''s'");
  });

  it("double-quotes Windows arguments that need it", () => {
    expect(quoteArg("C:\\Program Files\\x", true)).toBe('"C:\\Program Files\\x"');
    expect(quoteArg("C:\\x\\y", true)).toBe("C:\\x\\y");
  });
});

describe("planLaunch", () => {
  it("a cwd-only request just opens a tab there", () => {
    expect(planLaunch({ cwd: "/p", script: null, command: null }, false)).toEqual({
      kind: "open",
      cwd: "/p",
    });
  });

  it("a command request joins argv into one quoted command line", () => {
    expect(planLaunch({ cwd: "/p", script: null, command: ["ls", "-la", "/tmp/a b"] }, false)).toEqual({
      kind: "open",
      cwd: "/p",
      command: "ls -la '/tmp/a b'",
    });
  });

  it("a script request needs confirmation and carries the quoted path as its command", () => {
    expect(planLaunch({ cwd: "/p", script: "/p/my deploy.command", command: null }, false)).toEqual({
      kind: "confirm-script",
      cwd: "/p",
      scriptPath: "/p/my deploy.command",
      command: "'/p/my deploy.command'",
    });
  });

  it("on Windows a script is only opened at its directory, never executed", () => {
    expect(planLaunch({ cwd: "C:\\p", script: "C:\\p\\a.sh", command: null }, true)).toEqual({
      kind: "open",
      cwd: "C:\\p",
    });
  });

  it("null cwd becomes an absent cwd, not the string 'null'", () => {
    const plan = planLaunch({ cwd: null, script: null, command: ["htop"] }, false);
    expect(plan).toEqual({ kind: "open", command: "htop" });
    expect("cwd" in plan).toBe(false);
  });
});
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `npx vitest run src/lib/launchRequest.test.ts`
Expected: FAIL（找不到模組）。

- [ ] **Step 3: 實作**

`src/lib/launchRequest.ts`：

```ts
import type { LaunchRequest } from "../ipc/launch";

export type LaunchPlan =
  | { kind: "open"; cwd?: string; command?: string }
  | { kind: "confirm-script"; cwd?: string; scriptPath: string; command: string };

const SAFE_POSIX = /^[A-Za-z0-9_\/.:=@%+,-]+$/;
const SAFE_WINDOWS = /^[A-Za-z0-9_\\/.:=@%+,-]+$/;

/** 把單一參數變成 shell 安全的字串。POSIX 用單引號，Windows 用雙引號。 */
export function quoteArg(arg: string, isWindows: boolean): string {
  if (isWindows) {
    return SAFE_WINDOWS.test(arg) ? arg : `"${arg.replace(/"/g, '\\"')}"`;
  }
  return SAFE_POSIX.test(arg) ? arg : `'${arg.replace(/'/g, "'\\''")}'`;
}

/**
 * 決定一個啟動請求要怎麼處理：
 * - `-e` 指令：直接開分頁並在 shell 就緒後送出。
 * - 腳本：要先讓使用者確認（雙擊 `.command` 等於執行任意程式）。
 *   Windows 沒有註冊這個入口，且 `.sh` 在那邊無法直接執行，所以只開在該目錄。
 */
export function planLaunch(req: LaunchRequest, isWindows: boolean): LaunchPlan {
  const cwd = req.cwd ?? undefined;
  const base = cwd === undefined ? {} : { cwd };

  if (req.command && req.command.length > 0) {
    return { kind: "open", ...base, command: req.command.map((a) => quoteArg(a, isWindows)).join(" ") };
  }
  if (req.script) {
    if (isWindows) return { kind: "open", ...base };
    return { kind: "confirm-script", ...base, scriptPath: req.script, command: quoteArg(req.script, false) };
  }
  return { kind: "open", ...base };
}
```

- [ ] **Step 4: 跑測試確認全綠**

Run: `npx vitest run src/lib/launchRequest.test.ts`
Expected: 全部 PASS。

- [ ] **Step 5: 突變檢查**

暫時把 `quoteArg` 裡 `.replace(/'/g, "'\\''")` 改成 `.replace(/'/g, "")`，跑測試。
Expected: `escapes embedded single quotes the POSIX way` FAIL。還原後再跑確認全綠。

- [ ] **Step 6: Commit**

```bash
git add src/lib/launchRequest.ts src/lib/launchRequest.test.ts
git commit -m "$(cat <<'EOF'
feat(launch): plan how each launch request opens, with shell-safe quoting

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 就緒注入器 `createStartupInjector`

背景：Windows 上 shell 就緒前寫進 PTY 的輸入會被丟掉（ConPTY 先吐自己的序列、shell 約數秒後才啟動），所以不能用固定延遲。可靠訊號是 shell 自己發出的 OSC 133 A（提示字元開始），再等輸出安靜。若一直等不到 133;A（使用者的 shell 沒有整合腳本、或序列剛好被 chunk 切開），退回一個較長的保底逾時。

**Files:**
- Create: `src/lib/startupCommand.ts`
- Test: `src/lib/startupCommand.test.ts`

- [ ] **Step 1: 寫失敗的測試**

`src/lib/startupCommand.test.ts`：

```ts
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createStartupInjector } from "./startupCommand";

const PROMPT = "\x1b]133;A\x07";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("createStartupInjector", () => {
  it("does not write on ConPTY-style startup noise, only after a prompt and a quiet moment", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.feed("\x1b[?9001h\x1b[?1004h"); // 不是 shell 的輸出
    vi.advanceTimersByTime(5_000);
    expect(write).not.toHaveBeenCalled();
    inj.feed(`${PROMPT}user@host % `);
    vi.advanceTimersByTime(249);
    expect(write).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(write).toHaveBeenCalledTimes(1);
    expect(write).toHaveBeenCalledWith("ls\r");
  });

  it("keeps waiting while output is still arriving after the prompt", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.feed(PROMPT);
    vi.advanceTimersByTime(200);
    inj.feed("more output"); // 重新計時
    vi.advanceTimersByTime(200);
    expect(write).not.toHaveBeenCalled();
    vi.advanceTimersByTime(50);
    expect(write).toHaveBeenCalledTimes(1);
  });

  it("falls back to sending anyway when no prompt marker ever shows up", () => {
    const write = vi.fn();
    createStartupInjector({ command: "ls", write });
    vi.advanceTimersByTime(9_999);
    expect(write).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(write).toHaveBeenCalledWith("ls\r");
  });

  it("sends at most once even if more prompts follow", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.feed(PROMPT);
    vi.advanceTimersByTime(250);
    inj.feed(PROMPT);
    vi.advanceTimersByTime(20_000);
    expect(write).toHaveBeenCalledTimes(1);
  });

  it("dispose cancels everything", () => {
    const write = vi.fn();
    const inj = createStartupInjector({ command: "ls", write });
    inj.feed(PROMPT);
    inj.dispose();
    vi.advanceTimersByTime(20_000);
    expect(write).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `npx vitest run src/lib/startupCommand.test.ts`
Expected: FAIL（找不到模組）。

- [ ] **Step 3: 實作**

`src/lib/startupCommand.ts`：

```ts
export interface StartupInjector {
  /** 把 PTY 輸出餵進來。 */
  feed(chunk: string): void;
  dispose(): void;
}

interface Options {
  command: string;
  write: (data: string) => void;
  /** 看到提示字元之後，安靜多久才送。 */
  quietMs?: number;
  /** 一直看不到提示字元標記時的保底逾時。 */
  fallbackMs?: number;
}

const PROMPT_START = "\x1b]133;A";

/**
 * 在 shell 就緒後把 `command` 送進 PTY，只送一次。
 *
 * 「收到第一個輸出 chunk」不算就緒（Windows 的第一個 chunk 是 ConPTY 自己的
 * 序列，shell 還沒啟動，此時寫入的輸入會被丟掉）。可靠訊號是 shell 自己發的
 * OSC 133 A，再加上輸出安靜下來。
 */
export function createStartupInjector({
  command,
  write,
  quietMs = 250,
  fallbackMs = 10_000,
}: Options): StartupInjector {
  let sawPrompt = false;
  let finished = false;
  let quietTimer: ReturnType<typeof setTimeout> | undefined;

  const finish = () => {
    clearTimeout(quietTimer);
    clearTimeout(fallbackTimer);
    finished = true;
  };
  const send = () => {
    if (finished) return;
    finish();
    write(`${command}\r`);
  };
  const fallbackTimer = setTimeout(send, fallbackMs);

  return {
    feed(chunk) {
      if (finished) return;
      if (chunk.includes(PROMPT_START)) sawPrompt = true;
      if (sawPrompt) {
        clearTimeout(quietTimer);
        quietTimer = setTimeout(send, quietMs);
      }
    },
    dispose: finish,
  };
}
```

- [ ] **Step 4: 跑測試確認全綠**

Run: `npx vitest run src/lib/startupCommand.test.ts`
Expected: 5 tests PASS。

- [ ] **Step 5: 突變檢查**

暫時把 `feed` 裡 `if (sawPrompt) {` 改成 `if (true) {`（讓任何輸出都觸發安靜計時），跑測試。
Expected: 第一個測試（ConPTY 雜訊）FAIL。還原後再跑確認全綠。

- [ ] **Step 6: Commit**

```bash
git add src/lib/startupCommand.ts src/lib/startupCommand.test.ts
git commit -m "$(cat <<'EOF'
feat(launch): startup injector that sends a command once the shell is ready

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: `initialCommand` 貫穿 Tab → TerminalView

**Files:**
- Modify: `src/components/TabBar/index.tsx`（`Tab` 介面，約 line 38）
- Modify: `src/components/NewTabPicker/tabCatalog.ts`（`TabOpenOpts`，約 line 11-18）
- Modify: `src/components/TerminalView.tsx`（props 約 line 107；建立 PTY 約 line 1342；`onPtyData` 約 line 1373；cleanup 約 line 1816）
- Modify: `src/components/TerminalApp.tsx`（`handlePickerSelect` 建 tab 處，約 line 340；`<TerminalView …>` 約 line 778）
- Test: `src/lib/sessionTabs.initialCommand.test.ts`

- [ ] **Step 1: 寫失敗的回歸測試（`initialCommand` 不可被持久化）**

`initialCommand` 若被存進 localStorage，下次開 AITerm 還原分頁時會**再跑一次**指令。`saveSessionTabs` 用白名單所以現在就不會存；這個測試把它釘住。

`src/lib/sessionTabs.initialCommand.test.ts`：

```ts
import { describe, it, expect, beforeEach } from "vitest";
import { saveSessionTabs, restoreSessionTabs } from "./sessionTabs";
import type { Tab } from "../components/TabBar";

beforeEach(() => localStorage.clear());

describe("session tab persistence", () => {
  it("never persists initialCommand, so a restart cannot re-run a launch command", () => {
    const tab: Tab = {
      id: "t1",
      title: "Terminal",
      type: "terminal",
      cwd: "/proj",
      initialCommand: "rm -rf build",
    };
    saveSessionTabs([tab]);
    // 不依賴 storage key 的名字：掃過 localStorage 裡所有的值。
    const everythingStored = Object.keys(localStorage)
      .map((k) => localStorage.getItem(k))
      .join("\n");
    expect(everythingStored).toContain("/proj"); // 確認真的有存東西，否則下一行是空斷言
    expect(everythingStored).not.toContain("rm -rf build");
    expect(restoreSessionTabs()?.[0].initialCommand).toBeUndefined();
  });
});
```

- [ ] **Step 2: 跑測試確認失敗（型別錯誤）**

Run: `npx vitest run src/lib/sessionTabs.initialCommand.test.ts && npx tsc -b`
Expected: `tsc -b` 報 `initialCommand` 不存在於 `Tab`（測試檔本身也會因型別而無法通過型別檢查）。

- [ ] **Step 3: 型別與傳遞**

`src/components/TabBar/index.tsx`，在 `initialCwd?: string;`（約 line 38）下面加：

```ts
  /** 建立分頁時要在 shell 就緒後送出的指令（啟動請求的 `-e`／已確認的腳本）。
   *  只在 TerminalView 掛載時讀一次，且**絕不持久化**——見 sessionTabs.ts 的白名單。 */
  initialCommand?: string;
```

`src/components/NewTabPicker/tabCatalog.ts`，在 `TabOpenOpts` 的 `initialCwd?: string;` 下面加：

```ts
  /** 終端機分頁 shell 就緒後要送出的指令。啟動請求（開資料夾／-e／腳本）用它。 */
  initialCommand?: string;
```

`src/components/TerminalApp.tsx` 的 `handlePickerSelect` 建 tab 處，把

```ts
      initialCwd: opts?.initialCwd,
      initialMission: opts?.initialMission,
```

改成

```ts
      initialCwd: opts?.initialCwd,
      initialCommand: opts?.initialCommand,
      initialMission: opts?.initialMission,
```

同檔 `<TerminalView` 的 props（約 line 778），在 `initialCwd={tab.initialCwd}` 下面加：

```tsx
                  initialCommand={tab.initialCommand}
```

`src/components/TerminalView.tsx`：

1. 在檔頂 import 區加：`import { createStartupInjector, type StartupInjector } from "../lib/startupCommand";`
2. `TerminalViewProps` 的 `initialCwd?: string;`（約 line 107）下面加：

```ts
  /** 若有，PTY 就緒後把它當成一行指令送出（只送一次）。 */
  initialCommand?: string;
```

3. 元件函式簽名（約 line 165）的解構參數在 `initialCwd,` 後面加上 `initialCommand,`。
4. 在元件內其它 `useRef` 宣告附近加：

```ts
  const startupRef = useRef<StartupInjector | null>(null);
```

5. 建立 PTY 那段，`sessionRef.current = id;`（約 line 1361）下一行加：

```ts
        if (initialCommand) {
          startupRef.current = createStartupInjector({
            command: initialCommand,
            write: (data) => {
              writePty(id, data).catch(console.error);
            },
          });
        }
```

6. `onPtyData` 回呼裡，`const text = decoder.decode(bytes, { stream: true });`（約 line 1373）下一行加：

```ts
          startupRef.current?.feed(text);
```

7. 同一個 effect 的 cleanup 裡，`cancelled = true;`（約 line 1816）下一行加：

```ts
      startupRef.current?.dispose();
      startupRef.current = null;
```

- [ ] **Step 4: 型別檢查與測試**

Run: `npx tsc -b && npx vitest run src/lib/sessionTabs.initialCommand.test.ts`
Expected: `tsc -b` 無錯誤；測試 PASS。

- [ ] **Step 5: 突變檢查**

暫時把 `src/lib/sessionTabs.ts` 的 `saveSessionTabs` 白名單解構改成把整個 tab 存進去（例如 `const toSave = tabs as unknown as SavedTab[];`），跑該測試。
Expected: FAIL（字串 `rm -rf build` 出現在儲存內容）。還原後再跑確認 PASS。

- [ ] **Step 6: 跑既有 TerminalView／TerminalApp 測試確認沒壞**

Run: `npx vitest run src/components/TerminalView src/components/TerminalApp`
Expected: 既有測試全 PASS（新增的 prop 是選用的）。

- [ ] **Step 7: Commit**

```bash
git add src/components/TabBar/index.tsx src/components/NewTabPicker/tabCatalog.ts src/components/TerminalView.tsx src/components/TerminalApp.tsx src/lib/sessionTabs.initialCommand.test.ts
git commit -m "$(cat <<'EOF'
feat(launch): let a terminal tab run a command once its shell is ready

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: 腳本確認框與 i18n

**Files:**
- Modify: `src/lib/i18n.ts`（zhTW 約 line 210 旁；enRaw 約 line 1802 旁）
- Create: `src/components/LaunchScriptConfirm/index.tsx`
- Create: `src/components/LaunchScriptConfirm/index.css`
- Test: `src/components/LaunchScriptConfirm/index.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

`src/components/LaunchScriptConfirm/index.test.tsx`：

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LaunchScriptConfirm } from ".";
import { LocaleProvider } from "../../contexts/LocaleContext";

function mount(props: Partial<React.ComponentProps<typeof LaunchScriptConfirm>> = {}) {
  const onRun = vi.fn();
  const onSkip = vi.fn();
  render(
    <LocaleProvider>
      <LaunchScriptConfirm scriptPath="/proj/deploy.command" onRun={onRun} onSkip={onSkip} {...props} />
    </LocaleProvider>,
  );
  return { onRun, onSkip };
}

describe("LaunchScriptConfirm", () => {
  it("shows the full script path so the user knows exactly what would run", () => {
    mount({ scriptPath: "/proj/my folder/deploy.command" });
    expect(screen.getByRole("dialog")).toHaveTextContent("/proj/my folder/deploy.command");
  });

  it("Run and Skip call their own handlers and nothing else", async () => {
    const { onRun, onSkip } = mount();
    await userEvent.click(screen.getByTestId("launch-script-run"));
    expect(onRun).toHaveBeenCalledTimes(1);
    expect(onSkip).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTestId("launch-script-skip"));
    expect(onSkip).toHaveBeenCalledTimes(1);
    expect(onRun).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `npx vitest run src/components/LaunchScriptConfirm`
Expected: FAIL（找不到元件）。

- [ ] **Step 3: i18n 字串**

`src/lib/i18n.ts`：在 zhTW 字典的 `terminal_tab: "終端機",` 下面加：

```ts
    launch_script_title: "要執行這個腳本嗎？",
    launch_script_body: (path: string) => `有人要求用 AITerm 開啟並執行這個腳本：\n${path}\n\n只有你信任這個檔案時才執行。`,
    launch_script_run: "執行",
    launch_script_skip: "只開啟資料夾",
```

在 enRaw 字典的 `terminal_tab: "Terminal",` 下面加：

```ts
    launch_script_title: "Run this script?",
    launch_script_body: (path: string) => `AITerm was asked to open and run this script:\n${path}\n\nOnly run it if you trust the file.`,
    launch_script_run: "Run",
    launch_script_skip: "Just open the folder",
```

- [ ] **Step 4: 元件**

`src/components/LaunchScriptConfirm/index.tsx`：

```tsx
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  scriptPath: string;
  onRun: () => void;
  onSkip: () => void;
}

/**
 * 雙擊 `.command`／`.sh` 等於執行任意程式，所以執行前要使用者確認，並且
 * 顯示完整路徑。**必須掛在 `TerminalApp` 層**：非作用中的分頁是
 * `visibility: hidden` + `pointer-events: none`，掛在分頁裡會點不到。
 * 不用 `window.confirm`（Tauri 內有已知問題）。
 */
export function LaunchScriptConfirm({ scriptPath, onRun, onSkip }: Props) {
  const { t } = useLocale();
  return (
    <div className="aiterm-launch-confirm__backdrop">
      <div className="aiterm-launch-confirm" role="dialog" aria-modal="true">
        <div className="aiterm-launch-confirm__title">{t.launch_script_title}</div>
        <div className="aiterm-launch-confirm__body">{t.launch_script_body(scriptPath)}</div>
        <div className="aiterm-launch-confirm__actions">
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            data-testid="launch-script-skip"
            onClick={onSkip}
          >
            {t.launch_script_skip}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            data-testid="launch-script-run"
            autoFocus
            onClick={onRun}
          >
            {t.launch_script_run}
          </button>
        </div>
      </div>
    </div>
  );
}
```

`src/components/LaunchScriptConfirm/index.css`：

```css
.aiterm-launch-confirm__backdrop {
  position: fixed;
  inset: 0;
  z-index: 200;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #000a;
}

.aiterm-launch-confirm {
  width: 420px;
  max-width: calc(100vw - 32px);
  padding: 20px;
  border-radius: 10px;
  background: var(--aiterm-surface-2, #1e293b);
  border: 1px solid var(--aiterm-border, #334155);
  box-shadow: 0 12px 40px #000c;
  box-sizing: border-box;
}

.aiterm-launch-confirm__title {
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 8px;
}

.aiterm-launch-confirm__body {
  font-size: 13px;
  line-height: 1.5;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--aiterm-text-muted, #94a3b8);
  margin-bottom: 16px;
}

.aiterm-launch-confirm__actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
```

- [ ] **Step 5: 跑測試與型別／i18n 對齊測試**

Run: `npx vitest run src/components/LaunchScriptConfirm src/lib/i18n && npx tsc -b`
Expected: 全 PASS。`i18n.test.ts` 的 key 對齊測試會確認兩個語系都有這四個 key。

- [ ] **Step 6: 突變檢查（i18n 漂移）**

暫時只刪掉 enRaw 裡的 `launch_script_skip` 那一行，跑 `npx vitest run src/lib/i18n.test.ts`。
Expected: FAIL（en 缺 key）。還原後再跑確認 PASS。

- [ ] **Step 7: Commit**

```bash
git add src/lib/i18n.ts src/components/LaunchScriptConfirm
git commit -m "$(cat <<'EOF'
feat(launch): in-app confirmation before running a launched script

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: `TerminalApp` 訂閱、排空、處理請求

**Files:**
- Modify: `src/components/TerminalApp.tsx`
- Test: `src/components/TerminalApp.launchRequest.test.tsx`

設計要點（寫進程式註解）：
- **先 `listen`、再 `take`**：訂閱完成後才排空，之後的入列一定會有事件通知；訂閱之前入列的請求由第一次排空取到。
- `take_launch_requests` 在 Rust 端是原子的，多次呼叫（StrictMode 雙掛載、殘留的舊 listener）取到的集合互不相交，**不會**重複開分頁。
- 排空結果的處理**不能**檢查 `disposed`：StrictMode 開發模式下第一個 effect 實例被 cleanup 後，它的 `await take` 才回來，此時請求已經從佇列取走，若丟掉就永久遺失。改用穩定的 ref 呼叫最新的 handler。

- [ ] **Step 1: 寫失敗的測試**

`src/components/TerminalApp.launchRequest.test.tsx`（把 `TerminalView` 換成探針，這樣不需要 xterm，斷言直接看傳進去的 props）：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

type LaunchReq = { cwd: string | null; script: string | null; command: string[] | null };
const pending: LaunchReq[] = [];
const listeners = new Map<string, () => void>();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "take_launch_requests") return Promise.resolve(pending.splice(0));
    return new Promise(() => {});
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: () => void) => {
    listeners.set(name, cb);
    return Promise.resolve(() => listeners.delete(name));
  }),
}));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn(() => Promise.resolve("/home/test")) }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFocused: () => Promise.resolve(true),
    onFocusChanged: () => Promise.resolve(() => {}),
    isMaximized: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
    maximize: () => Promise.resolve(),
    unmaximize: () => Promise.resolve(),
    minimize: () => Promise.resolve(),
    close: () => Promise.resolve(),
    startDragging: () => Promise.resolve(),
  }),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({ sendNotification: vi.fn() }));

// TerminalView 換成探針：這個檔案要驗的是「請求怎麼變成分頁」，不是 xterm。
vi.mock("./TerminalView", () => ({
  TerminalView: (p: { initialCwd?: string; initialCommand?: string }) => (
    <div data-testid="tv" data-cwd={p.initialCwd ?? ""} data-cmd={p.initialCommand ?? ""} />
  ),
}));

Element.prototype.scrollTo = Element.prototype.scrollTo || (() => {});
if (!window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}
class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;

import { TerminalApp } from "./TerminalApp";
import { LocaleProvider } from "../contexts/LocaleContext";

function mountApp() {
  return render(
    <MemoryRouter>
      <LocaleProvider>
        <TerminalApp />
      </LocaleProvider>
    </MemoryRouter>,
  );
}

const tabsWith = (attr: "data-cwd" | "data-cmd", value: string) =>
  screen.queryAllByTestId("tv").filter((el) => el.getAttribute(attr) === value);

beforeEach(() => {
  pending.length = 0;
  listeners.clear();
  localStorage.clear();
});

describe("TerminalApp launch requests", () => {
  it("sanity: with no request, no tab has a starting directory", async () => {
    mountApp();
    await waitFor(() => expect(screen.queryAllByTestId("tv").length).toBeGreaterThan(0));
    expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(0);
  });

  it("a request queued BEFORE the app mounted still opens a tab at that directory", async () => {
    pending.push({ cwd: "/tmp/proj", script: null, command: null });
    mountApp();
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1));
  });

  it("a request queued AFTER mount opens a tab when the pending event fires", async () => {
    mountApp();
    await waitFor(() => expect(listeners.has("launch-request-pending")).toBe(true));
    pending.push({ cwd: "/tmp/later", script: null, command: null });
    listeners.get("launch-request-pending")!();
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/later")).toHaveLength(1));
  });

  it("firing the event twice does not open the same request twice", async () => {
    mountApp();
    await waitFor(() => expect(listeners.has("launch-request-pending")).toBe(true));
    pending.push({ cwd: "/tmp/once", script: null, command: null });
    listeners.get("launch-request-pending")!();
    listeners.get("launch-request-pending")!();
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/once")).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 30));
    expect(tabsWith("data-cwd", "/tmp/once")).toHaveLength(1);
  });

  it("a -e request opens a tab carrying the quoted command line", async () => {
    pending.push({ cwd: "/tmp/proj", script: null, command: ["ls", "-la", "/tmp/a b"] });
    mountApp();
    await waitFor(() => expect(tabsWith("data-cmd", "ls -la '/tmp/a b'")).toHaveLength(1));
  });

  it("a script asks first: nothing runs until the user confirms", async () => {
    pending.push({ cwd: "/tmp/proj", script: "/tmp/proj/go.command", command: null });
    mountApp();
    await screen.findByRole("dialog");
    expect(screen.getByRole("dialog")).toHaveTextContent("/tmp/proj/go.command");
    expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(0);
    await userEvent.click(screen.getByTestId("launch-script-run"));
    await waitFor(() => expect(tabsWith("data-cmd", "/tmp/proj/go.command")).toHaveLength(1));
    expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("skipping a script still opens the directory but runs nothing", async () => {
    pending.push({ cwd: "/tmp/proj", script: "/tmp/proj/go.command", command: null });
    mountApp();
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByTestId("launch-script-skip"));
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1));
    expect(tabsWith("data-cmd", "/tmp/proj/go.command")).toHaveLength(0);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("two scripts are confirmed one at a time, in order", async () => {
    pending.push(
      { cwd: "/a", script: "/a/one.command", command: null },
      { cwd: "/b", script: "/b/two.command", command: null },
    );
    mountApp();
    const first = await screen.findByRole("dialog");
    expect(first).toHaveTextContent("/a/one.command");
    await userEvent.click(screen.getByTestId("launch-script-skip"));
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("/b/two.command"));
  });
});
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `npx vitest run src/components/TerminalApp.launchRequest.test.tsx`
Expected: 除第一個 sanity 測試外全部 FAIL（`TerminalApp` 還沒訂閱／排空）。sanity 若 FAIL，代表測試環境本身有問題（探針沒渲染），先解決再往下。

- [ ] **Step 3: 實作**

`src/components/TerminalApp.tsx`：

1. import 區加：

```ts
import { takeLaunchRequests, onLaunchRequestPending, type LaunchRequest } from "../ipc/launch";
import { planLaunch } from "../lib/launchRequest";
import { LaunchScriptConfirm } from "./LaunchScriptConfirm";
```

2. 在 `handlePickerSelect` 的 `useCallback` 之後（約 line 350 之後，`handleAiRouted` 之前）加入：

```ts
  // ── 啟動請求（從 Finder／檔案管理員開資料夾、x-terminal-emulator -e …）──
  // 等待使用者確認的腳本請求，一次顯示一個。
  const [scriptQueue, setScriptQueue] = useState<Array<{ cwd?: string; scriptPath: string; command: string }>>([]);

  const handleLaunchRequest = useCallback((req: LaunchRequest) => {
    const isWindows = navigator.platform.toLowerCase().startsWith("win");
    const plan = planLaunch(req, isWindows);
    if (plan.kind === "open") {
      handlePickerSelect("terminal", { initialCwd: plan.cwd, initialCommand: plan.command });
    } else {
      setScriptQueue((q) => [...q, { cwd: plan.cwd, scriptPath: plan.scriptPath, command: plan.command }]);
    }
  }, [handlePickerSelect]);
  // 用 ref 呼叫最新的 handler，讓下面那個 effect 不必因為 handler 換了而重新訂閱。
  const launchHandlerRef = useRef(handleLaunchRequest);
  launchHandlerRef.current = handleLaunchRequest;

  useEffect(() => {
    const drain = async () => {
      try {
        const requests = await takeLaunchRequests();
        // 注意：這裡**不能**檢查「effect 已被 cleanup」。請求一旦從後端佇列取走
        // 就只存在這個陣列裡；StrictMode 開發模式下第一個 effect 實例被 cleanup
        // 後它的 await 才回來，若在此丟掉，那些請求就永久遺失。ref 永遠指向
        // 最新的 handler，元件真的卸載時 setState 是 no-op。
        for (const r of requests) launchHandlerRef.current(r);
      } catch (e) {
        console.warn("take_launch_requests 失敗:", e);
      }
    };
    // **先訂閱、再排空**：訂閱完成之後入列的請求一定有事件通知；訂閱之前入列的
    // （冷啟動 argv、macOS 的 Opened）由這次排空取到。後端 take 是原子的，
    // 重複呼叫取到的集合互不相交，不會重複開分頁。
    const pendingUnlisten = onLaunchRequestPending(() => void drain());
    void pendingUnlisten.then(() => drain()).catch(() => {});
    return unlistenOnCleanup(pendingUnlisten, "launch-request-pending");
  }, []);

  const currentScript = scriptQueue[0];
  // 不可以在 setScriptQueue 的 updater 裡呼叫 handlePickerSelect：updater 必須是
  // 純的，StrictMode 會把它跑兩次，而那會開出兩個分頁。先讀目前值、再各自 set。
  const finishScript = useCallback((run: boolean) => {
    const head = scriptQueue[0];
    if (!head) return;
    setScriptQueue((q) => q.slice(1));
    handlePickerSelect("terminal", {
      initialCwd: head.cwd,
      initialCommand: run ? head.command : undefined,
    });
  }, [scriptQueue, handlePickerSelect]);
```

3. 在 JSX 中與其它全域對話框（`<ConsentDialog … />` 那一帶）並列處加：

```tsx
      {currentScript && (
        <LaunchScriptConfirm
          scriptPath={currentScript.scriptPath}
          onRun={() => finishScript(true)}
          onSkip={() => finishScript(false)}
        />
      )}
```

- [ ] **Step 4: 跑測試確認全綠**

Run: `npx vitest run src/components/TerminalApp.launchRequest.test.tsx`
Expected: 8 tests PASS。

- [ ] **Step 5: 突變檢查（兩個關鍵不變量）**

(a) 把 `void pendingUnlisten.then(() => drain()).catch(() => {});` 那行註解掉，跑測試。
Expected: `a request queued BEFORE the app mounted…` FAIL（冷啟動請求永遠沒人取）。還原。

(b) 暫時把 `for (const r of requests) launchHandlerRef.current(r);` 那行註解掉，跑測試。
Expected: 除 sanity 外全部 FAIL，證明每個測試都確實走過「排空 → handler → 開分頁」這條路徑。還原。

（StrictMode「cleanup 後才回來的排空結果不能丟」這條不變量，測試環境沒開 StrictMode，無法自動驗證；靠上面的程式註解與 Task 12 的實機驗收把關。）

再跑一次完整檔案確認全綠。

- [ ] **Step 6: 既有 TerminalApp 測試不受影響**

Run: `npx vitest run src/components/TerminalApp`
Expected: 既有的 `taskBoard`、`routeHintCloseGuard`、`autoCloseSkipGuard`、`remoteReconnect` 測試全 PASS。（這些測試的 `invoke` 永不 resolve，所以 `takeLaunchRequests` 只會卡住，不影響它們。）

- [ ] **Step 7: 型別、lint**

Run: `npx tsc -b && npm run lint`
Expected: 無錯誤。

- [ ] **Step 8: Commit**

```bash
git add src/components/TerminalApp.tsx src/components/TerminalApp.launchRequest.test.tsx
git commit -m "$(cat <<'EOF'
feat(launch): open a tab for each launch request, confirming scripts first

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: macOS 系統註冊（Info.plist）

**Files:**
- Create: `src-tauri/Info.plist`
- Create: `src-tauri/tests/os_registration.rs`

Tauri 會自動把與 `tauri.conf.json` 同目錄的 `Info.plist` 合併進 macOS bundle。UTI：`public.folder`（資料夾）、`com.apple.terminal.shell-script`（`.command`，Terminal.app 宣告的型別）、`public.shell-script`（`.sh`）。`LSHandlerRank = Alternate` 表示只出現在「打開方式」，不搶走現有預設。

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/tests/os_registration.rs`：

```rust
//! 驗證「讓作業系統認得 AITerm 是終端機」的靜態註冊檔：macOS Info.plist、
//! Linux .desktop 樣板與 deb 維護腳本。這些檔案只在打包時才被讀到，
//! 一旦寫壞，要到裝了套件才會發現，所以在這裡用測試釘住。

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("讀不到 {rel}: {e}"))
}

#[test]
fn info_plist_declares_folders_and_shell_scripts_as_openable_without_stealing_defaults() {
    let plist = read("Info.plist");
    for uti in ["public.folder", "com.apple.terminal.shell-script", "public.shell-script"] {
        assert!(plist.contains(uti), "Info.plist 缺少 {uti}");
    }
    assert!(plist.contains("CFBundleDocumentTypes"));
    assert!(plist.contains("<string>Alternate</string>"), "必須是 Alternate，不能搶預設");
    assert!(!plist.contains("<string>Owner</string>"), "不可宣告成 Owner");
}
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd src-tauri && cargo test --test os_registration`
Expected: FAIL（`讀不到 Info.plist`）。

- [ ] **Step 3: 建立 `src-tauri/Info.plist`**

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key>
      <string>Folder</string>
      <key>CFBundleTypeRole</key>
      <string>Viewer</string>
      <key>LSHandlerRank</key>
      <string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>public.folder</string>
      </array>
    </dict>
    <dict>
      <key>CFBundleTypeName</key>
      <string>Shell Script</string>
      <key>CFBundleTypeRole</key>
      <string>Shell</string>
      <key>LSHandlerRank</key>
      <string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>com.apple.terminal.shell-script</string>
        <string>public.shell-script</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
```

- [ ] **Step 4: 跑測試確認全綠，並驗證 plist 語法**

Run: `cd src-tauri && cargo test --test os_registration && plutil -lint Info.plist`
Expected: 測試 PASS；`plutil` 輸出 `Info.plist: OK`。

- [ ] **Step 5: 突變檢查**

把 `Info.plist` 裡第一個 `<string>Alternate</string>` 改成 `<string>Owner</string>`，跑 `cargo test --test os_registration`。
Expected: FAIL。還原後再跑確認 PASS。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Info.plist src-tauri/tests/os_registration.rs
git commit -m "$(cat <<'EOF'
feat(launch): declare folders and shell scripts as openable on macOS

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 11: Linux 系統註冊（.desktop、deb 腳本）

**Files:**
- Create: `src-tauri/linux/aiterm.desktop`
- Create: `src-tauri/linux/postinst.sh`
- Create: `src-tauri/linux/prerm.sh`
- Modify: `src-tauri/tauri.linux.conf.json`
- Modify: `src-tauri/tests/os_registration.rs`

不寫死執行檔路徑：Tauri 打包後的 `Exec=` 值才是真正的執行檔名稱，所以維護腳本從套件已安裝的 `.desktop` 讀出 `Exec=`，避免猜錯（`/usr/bin/` 下還有 `uv` 等 sidecar，不能取第一個檔案）。`postinst` 只在 `configure` 時註冊；`prerm` 只在 `remove`／`deconfigure` 時移除，**升級不移除**。priority 40 低於多數發行版預設，不搶既有設定。

- [ ] **Step 1: 在 `os_registration.rs` 追加失敗的測試**

檔尾追加：

```rust
#[test]
fn desktop_template_is_a_terminal_emulator_that_opens_directories() {
    let d = read("linux/aiterm.desktop");
    assert!(d.contains("TerminalEmulator"), "缺 TerminalEmulator 類別");
    assert!(d.contains("inode/directory"), "缺 MimeType inode/directory");
    assert!(d.contains("Exec={{exec}} %F"), "Exec 要接 %F 才會收到檔案管理員給的路徑");
    for var in ["{{name}}", "{{icon}}"] {
        assert!(d.contains(var), "樣板缺少變數 {var}");
    }
}

#[test]
fn linux_conf_points_at_the_template_and_both_scripts() {
    let conf = read("tauri.linux.conf.json");
    for needle in [
        "\"desktopTemplate\": \"linux/aiterm.desktop\"",
        "\"postInstallScript\": \"linux/postinst.sh\"",
        "\"preRemoveScript\": \"linux/prerm.sh\"",
    ] {
        assert!(conf.contains(needle), "tauri.linux.conf.json 缺少 {needle}");
    }
}

#[cfg(unix)]
mod maintainer_scripts {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    /// 建一個假的 PATH：`dpkg -L` 回報一個裝好的 .desktop，
    /// `update-alternatives` 只把收到的參數記到 log。
    fn stub_env() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let desktop = dir.path().join("AITerm.desktop");
        fs::write(&desktop, "[Desktop Entry]\nExec=AITerm %F\nName=AITerm\n").unwrap();
        let log = dir.path().join("alternatives.log");

        let write_stub = |name: &str, body: String| {
            let path = bin.join(name);
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        };
        write_stub("dpkg", format!("echo '{}'", desktop.display()));
        write_stub("update-alternatives", format!("echo \"$@\" >> '{}'", log.display()));
        (dir, bin, log)
    }

    fn run_script(script: &str, action: &str, bin: &PathBuf) {
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let status = Command::new("sh")
            .arg(root().join(script))
            .arg(action)
            .env("PATH", path)
            .env("DPKG_MAINTSCRIPT_PACKAGE", "aiterm")
            .status()
            .expect("sh 無法執行");
        assert!(status.success(), "{script} {action} 應該成功結束");
    }

    fn logged(log: &PathBuf) -> String {
        fs::read_to_string(log).unwrap_or_default()
    }

    #[test]
    fn scripts_are_valid_shell() {
        for script in ["linux/postinst.sh", "linux/prerm.sh"] {
            let status = Command::new("sh").arg("-n").arg(root().join(script)).status().unwrap();
            assert!(status.success(), "{script} 語法錯誤");
        }
    }

    #[test]
    fn postinst_registers_the_binary_named_in_the_installed_desktop_file() {
        let (_guard, bin, log) = stub_env();
        run_script("linux/postinst.sh", "configure", &bin);
        assert_eq!(
            logged(&log).trim(),
            "--install /usr/bin/x-terminal-emulator x-terminal-emulator /usr/bin/AITerm 40"
        );
    }

    #[test]
    fn postinst_does_nothing_for_other_actions() {
        let (_guard, bin, log) = stub_env();
        run_script("linux/postinst.sh", "abort-upgrade", &bin);
        assert_eq!(logged(&log), "");
    }

    #[test]
    fn prerm_unregisters_on_remove() {
        let (_guard, bin, log) = stub_env();
        run_script("linux/prerm.sh", "remove", &bin);
        assert_eq!(logged(&log).trim(), "--remove x-terminal-emulator /usr/bin/AITerm");
    }

    #[test]
    fn prerm_keeps_the_registration_during_an_upgrade() {
        let (_guard, bin, log) = stub_env();
        run_script("linux/prerm.sh", "upgrade", &bin);
        assert_eq!(logged(&log), "", "升級不可移除註冊，否則使用者選的終端機會被重設");
    }
}
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd src-tauri && cargo test --test os_registration`
Expected: Task 10 的測試 PASS；新增的測試全部 FAIL（讀不到 `linux/aiterm.desktop` 等檔案）。

- [ ] **Step 3: `.desktop` 樣板**

`src-tauri/linux/aiterm.desktop`（Handlebars；可用變數只有 `categories`、`comment`、`exec`、`icon`、`name`）：

```
[Desktop Entry]
Type=Application
Name={{name}}
{{#if comment}}
Comment={{comment}}
{{/if}}
Exec={{exec}} %F
Icon={{icon}}
Terminal=false
Categories=System;TerminalEmulator;Development;
Keywords=terminal;shell;console;
MimeType=inode/directory;
```

- [ ] **Step 4: 維護腳本**

`src-tauri/linux/postinst.sh`：

```sh
#!/bin/sh
# 把 AITerm 登記成 x-terminal-emulator 的候選。priority 40 低於多數發行版預設，
# 不會搶走使用者現有的選擇；要切換用 `update-alternatives --config x-terminal-emulator`。
set -e

[ "$1" = "configure" ] || exit 0

# 執行檔名稱以套件實際安裝的 .desktop 的 Exec= 為準，不寫死
# （/usr/bin 下還有 uv 等 sidecar，不能隨便取第一個檔案）。
DESKTOP=$(dpkg -L "$DPKG_MAINTSCRIPT_PACKAGE" 2>/dev/null | grep '\.desktop$' | head -n 1)
[ -n "$DESKTOP" ] || exit 0
BIN=$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$DESKTOP" | head -n 1)
[ -n "$BIN" ] || exit 0
case "$BIN" in
  /*) ;;
  *) BIN="/usr/bin/$BIN" ;;
esac

update-alternatives --install /usr/bin/x-terminal-emulator x-terminal-emulator "$BIN" 40
```

`src-tauri/linux/prerm.sh`：

```sh
#!/bin/sh
# 移除套件時取消註冊。升級（upgrade）不能取消，否則使用者選的終端機會被重設。
set -e

case "$1" in
  remove|deconfigure) ;;
  *) exit 0 ;;
esac

DESKTOP=$(dpkg -L "$DPKG_MAINTSCRIPT_PACKAGE" 2>/dev/null | grep '\.desktop$' | head -n 1)
[ -n "$DESKTOP" ] || exit 0
BIN=$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$DESKTOP" | head -n 1)
[ -n "$BIN" ] || exit 0
case "$BIN" in
  /*) ;;
  *) BIN="/usr/bin/$BIN" ;;
esac

update-alternatives --remove x-terminal-emulator "$BIN"
```

- [ ] **Step 5: 接進 `tauri.linux.conf.json`**

把 `bundle` 物件改成（保留原有的 `externalBin` 與 `resources`，只新增 `linux`）：

```json
{
  "bundle": {
    "externalBin": ["binaries/uv"],
    "linux": {
      "deb": {
        "desktopTemplate": "linux/aiterm.desktop",
        "postInstallScript": "linux/postinst.sh",
        "preRemoveScript": "linux/prerm.sh"
      }
    },
    "resources": {
      "../tools/ApiDocFetcher/*.py": "ApiDocFetcher/",
      "../tools/ApiDocFetcher/strategies/*.py": "ApiDocFetcher/strategies/",
      "../tools/ApiDocFetcher/requirements.txt": "ApiDocFetcher/requirements.txt",
      "../tools/MarkItDown/converter.py": "MarkItDown/converter.py",
      "../tools/MarkItDown/requirements.txt": "MarkItDown/requirements.txt"
    }
  }
}
```

（編輯時用 Edit 只插入 `"linux": {…},` 這一段，不要整份重寫。）

- [ ] **Step 6: 跑測試確認全綠**

Run: `cd src-tauri && cargo test --test os_registration`
Expected: 全部 PASS（macOS 上會跑 `maintainer_scripts`，因為它是 `cfg(unix)`）。

- [ ] **Step 7: 突變檢查**

(a) 把 `prerm.sh` 的 `remove|deconfigure) ;;` 改成 `remove|deconfigure|upgrade) ;;`，跑測試。
Expected: `prerm_keeps_the_registration_during_an_upgrade` FAIL。還原。

(b) 把 `postinst.sh` 的 `[ "$1" = "configure" ] || exit 0` 刪掉，跑測試。
Expected: `postinst_does_nothing_for_other_actions` FAIL。還原。

再跑一次確認全綠。

- [ ] **Step 8: 確認 conf 是合法 JSON**

Run: `python3 -c "import json; json.load(open('src-tauri/tauri.linux.conf.json'))" && echo OK`
Expected: `OK`。

- [ ] **Step 9: Commit**

```bash
git add src-tauri/linux src-tauri/tauri.linux.conf.json src-tauri/tests/os_registration.rs
git commit -m "$(cat <<'EOF'
feat(launch): register AITerm as a Linux terminal emulator in deb packages

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: 完整驗證、macOS 實機驗收、收尾

**Files:**
- Modify: `docs/superpowers/specs/2026-09-19-open-as-terminal-mac-linux-design.md`（把腳本流程一行改成與實作一致）

- [ ] **Step 1: 完整自動化測試（依 CLAUDE.md，必須 `--workspace --no-fail-fast`）**

Run:
```bash
cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result|FAILED|failed|error"
cd .. && npx tsc -b && npm run lint && npm run test
```
Expected: 每一行 `test result:` 都是 `ok`（要看**每一行**，不是最後一行）；`tsc`、`lint` 無錯誤；Vitest 全綠。已知 `MailView` 有一個只在記憶體吃緊時才紅的 flaky 測試，見記憶「MailView flaky 測試（未解）」——若只有它紅，單獨重跑一次確認即可，不要往 CPU 競爭或檔案內污染的方向查。

若 `aiterm-core` 的 pty 測試因環境性 openpty 競爭而紅，那是既有問題，但要確認 `app` 這個 crate 的測試**確實有執行**（找 `Running tests/os_registration.rs` 與 `launch::` 相關輸出），不能因為前面失敗就沒跑到。

- [ ] **Step 2: 更新 spec 與實作一致**

`docs/superpowers/specs/2026-09-19-open-as-terminal-mac-linux-design.md` 的「前端行為」第 3 點，把
「`script`：先顯示**應用內**確認對話框…取消則保留已開好的分頁，什麼都不執行。」
改成
「`script`：先顯示**應用內**確認對話框（含完整路徑），確認後開分頁並執行；選「只開啟資料夾」則開分頁但不執行。因為 `initialCommand` 只在 `TerminalView` 掛載時讀取，所以是先確認再開分頁，使用者看到的結果與先開後確認相同。」

- [ ] **Step 3: macOS 實機驗收（用正式 build，不是 `tauri:dev`）**

`tauri:dev` 的 watcher 不可靠，且 Info.plist 的文件類型只在打包後的 `.app` 才會登記給 LaunchServices，所以用正式 build：

```bash
npm run tauri:build -- --no-sign
APP="src-tauri/target/release/bundle/macos/AITerm.app"
plutil -p "$APP/Contents/Info.plist" | grep -A6 CFBundleDocumentTypes
```
Expected: 看得到 `public.folder`、`com.apple.terminal.shell-script`、`public.shell-script`，且 `LSHandlerRank => Alternate`。

確認跑的是新二進位（不是舊 build 的殘留）：`ls -l "$APP/Contents/MacOS/"` 的時間戳是剛剛，且先 `pkill -x AITerm` 後再啟動。

驗收情境（每一項都要實際看到結果，用 `screencapture` 或 osascript 取畫面；權限與 IME 注意事項見記憶「用 osascript 實際操作 Mac 上的 AITerm 視窗驗收」）：

```bash
mkdir -p "/tmp/aiterm launch test/sub dir"
printf '#!/bin/sh\necho SCRIPT-RAN-OK\n' > "/tmp/aiterm launch test/hello.command" && chmod +x "/tmp/aiterm launch test/hello.command"
```

1. **冷啟動開資料夾**：`pkill -x AITerm; open -a "$APP" "/tmp/aiterm launch test"` → 新分頁，`pwd` 是 `/tmp/aiterm launch test`。
2. **已在執行時開資料夾**：AITerm 開著，`open -a "$APP" "/tmp/aiterm launch test/sub dir"` → **同一個視窗**多一個分頁，`pwd` 是 `sub dir`，且沒有第二個 AITerm 行程（`pgrep -x AITerm | wc -l` 為 1）。
3. **`.command`**：`open -a "$APP" "/tmp/aiterm launch test/hello.command"` → 出現確認框且顯示完整路徑；按「執行」→ 新分頁印出 `SCRIPT-RAN-OK`；再開一次按「只開啟資料夾」→ 分頁開在該目錄、**沒有**印出。
4. **命令列旗標（單例轉發）**：`"$APP/Contents/MacOS/AITerm" --working-directory="/tmp/aiterm launch test" -e ls` → 既有視窗多一個分頁，列出該目錄內容。
5. **拖到 Dock 圖示**：把資料夾拖到 Dock 的 AITerm 圖示 → 開新分頁。
6. **不搶預設**：Finder 對一個 `.command` 檔按「取得資訊」，預設「打開方式」仍是 Terminal，AITerm 只出現在清單裡。

任何一項失敗：不要猜，依 `superpowers:systematic-debugging` 先找根因。

- [ ] **Step 4: 誠實記錄 Linux 與 Windows 的驗證範圍**

Linux 沒有實機：已驗證的是 `.desktop` 樣板內容、維護腳本的語法與行為（用假的 `dpkg`／`update-alternatives` 驗證呼叫參數）、conf 指向。**尚未驗證**：真實 deb 的 `Exec=` 值是什麼、`update-alternatives` 實際行為、Nautilus「在終端機開啟」是否出現 AITerm。這幾項留待 CI 出 deb 後在 Ubuntu VM 上驗證，在那之前回報時不可說「Linux 已實測」。

Windows 不在本份範圍，但 single-instance 與 `parse_args` 也會在 Windows 生效（`aiterm .` 之類的命令列會開新分頁）；建議在下一輪 Windows spec 開始前，先在 Windows 上簡單確認 `.exe` 不會因為新增 single-instance 而在沒有參數時行為改變。

- [ ] **Step 5: Commit spec 更新**

```bash
git add -f docs/superpowers/specs/2026-09-19-open-as-terminal-mac-linux-design.md
git commit -m "$(cat <<'EOF'
docs: align terminal-registration spec with confirm-then-open script flow

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 6: 收尾**

依專案流程呼叫 `superpowers:verification-before-completion` 與 `superpowers:requesting-code-review`。**不要**自行推 tag（推 `vX.Y.Z` tag 會觸發 release build，必須先問使用者；打 tag 前還要先寫使用者看得懂的 CHANGELOG 段落）。

---

## Self-Review（計畫對照 spec）

**Spec 涵蓋：**
- 啟動請求解析（資料夾、`.command`／`.sh`、`--working-directory` 兩種寫法、`-e` 吞掉其後全部、相對路徑、不存在略過、未知旗標略過、`--headless`）→ Task 1。
- 三個入口收斂（冷啟動、single-instance、macOS Opened）→ Task 2（函式）＋ Task 3（接線）。
- 先入列後通知、前端先訂閱再排空、監聽器在 `TerminalApp`、unlisten → Task 4、Task 9（含「先訂閱後排空」與「重複觸發不重複開分頁」測試）。
- 前端行為：新分頁＋`cwd`、`command` 就緒後送出、`script` 應用內確認、不用 `window.confirm`、i18n 兩語系 → Task 5–9。
- 系統註冊：macOS Info.plist（Alternate、資料夾、`.command`、`.sh`）→ Task 10；Linux `.desktop`（TerminalEmulator、`inode/directory`）、deb `update-alternatives` 註冊／移除 → Task 11；AppImage／rpm 只有 `.desktop` 類別（由樣板自然達成，沒有額外腳本）。
- 錯誤處理：路徑不存在略過（Task 1）；佇列在視窗建立前入列由「先入列後排空」承接（Task 9）。`script` 檔不可讀的「顯示一行提示」**沒有**實作——確認框顯示路徑、執行時 shell 自己會報錯，加額外提示屬於 YAGNI；如需要再補。
- 測試與驗收限制：Rust 單元、前端 Vitest、macOS 實機、Linux 只驗內容並誠實標註 → Task 1–12。

**刻意偏離 spec 並已記錄：** 腳本流程改為「先確認再開分頁」（開頭說明；Task 12 Step 2 更新 spec）。

**型別一致性檢查：** `LaunchRequest`（Rust `cwd/script/command`）↔ `LaunchRequest`（TS `cwd/script/command`，皆可為 null）；`planLaunch` 回傳 `LaunchPlan`，Task 9 依 `kind` 分支；`TabOpenOpts.initialCommand` ↔ `Tab.initialCommand` ↔ `TerminalView.initialCommand`；`createStartupInjector` 的 `feed/dispose` 與 Task 7 的使用一致；事件名 `launch-request-pending`（Rust `PENDING_EVENT`）↔ `ipc/launch.ts` 一致；command 名 `take_launch_requests` 一致。

**已知風險（執行時要留意）：**
1. Task 9 測試假設 `TerminalApp` 在沒有 `LocaleProvider` 之外的額外 provider 下可掛載，這與既有 `TerminalApp.taskBoard.test.tsx` 的做法一致；若掛載失敗，對照該檔的 mock 清單補齊。
2. Task 7 的 `SESSION_TABS_KEY` 實際字串要以原始碼為準（該 Step 已註明）。
3. Task 11 的 `Exec=` 實際值要等真實 deb 才能確認，所以腳本設計成從已安裝的 `.desktop` 讀取而不是寫死。
