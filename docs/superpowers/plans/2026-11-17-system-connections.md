# 系統連線（網頁系統）第一期 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 AI 能自主操作使用者設定的外部網頁系統（正式環境），寫入動作被瀏覽器網路層攔截並依每個系統的權限模式放行／拒絕／等核准，使用者能即時看到、停止、接手。

**Architecture:** AITerm 啟動獨立有頭 Chrome（每系統一個設定檔、127.0.0.1 隨機偵錯埠）。Playwright MCP 子行程與 AITerm 的 CDP 閘門各自連上同一個 Chrome；閘門用 `Fetch.requestPaused` 攔非 GET 請求。AITerm 對外提供一個動態的 MCP 端點（`sys_*` 工具＝代理 Playwright MCP 的工具＋`system`／`intent` 參數），Ask AI 迴圈與 Claude Code 都把它當一般 MCP 伺服器用。規格：`docs/superpowers/specs/2026-11-17-system-connections-design.md`（含已實測的探勘結果，腳本在 scratchpad `probe/probe.mjs`）。

**Tech Stack:** Rust（tokio、tokio-tungstenite 0.29 已在依賴、sqlx sqlite、axum、serde_json）、React 19＋TypeScript、Vitest、`@playwright/mcp@0.0.83`（`tasks::ask::PLAYWRIGHT_MCP`）。

**工作方式：** 開 worktree／分支 `feat/system-connections`（`superpowers:using-git-worktrees`）；每個 Task 先寫紅燈測試、證明會紅再實作；commit 前 `git status`，只 add 明確路徑；Rust 驗證一律 `cd src-tauri && cargo test --workspace --no-fail-fast` 並看每一行 `test result:`；前端 `npm run test` 與 `npx tsc -b`；新檔案 CRLF／LF 跟隨同資料夾既有檔案（改完看 `git diff --stat`）。不打 tag、不 push 到 master 前先問使用者。

---

## 檔案結構

新增（後端，`src-tauri/src/systems/`）：
- `mod.rs`：模組宣告、`SystemsState`（執行中的瀏覽器、等待中的核准、接手旗標）。
- `types.rs`：`SystemConnection`、`Mode`、`Rule`、`Decision` 等資料型別。
- `gate.rs`：純函式 `decide()` 與 `glob_match()`（所有判定規則都在這裡，最重要的測試目標）。
- `audit.rs`：`systems.db` 與審計記錄。
- `browser.rs`：找 Chrome、啟動、設定檔路徑、偵錯埠、關閉。
- `cdp.rs`：CDP 連線與 `Fetch` 攔截迴圈，呼叫 `gate::decide`，需要核准時走 `approvals`。
- `approvals.rs`：等待中的核准（`oneshot` 表）、逾時、決定。
- `proxy.rs`：Playwright MCP 子行程（用既有 `mcp::transport::StdioTransport`）、工具清單改寫、`intent` 檢查、逾時改寫、拒絕結果改寫。
- `endpoint.rs`：axum JSON-RPC 的 MCP 端點（`initialize`／`tools/list`／`tools/call`）。

新增（其他）：
- `src-tauri/src/commands/systems.rs`：Tauri 指令（連線 CRUD、開啟登入視窗、核准決定、停止／接手／交還、停用、查審計）。
- `src/ipc/systems.ts`、`src/components/SystemsView/`（`index.tsx`、`ConnectionForm.tsx`、`ApprovalCards.tsx`、`AuditLog.tsx`、`index.css`、各自的 `.test.tsx`）。

修改：
- `src-tauri/src/config/types.rs`：`AppConfig.system_connections`。
- `src-tauri/src/lib.rs`：`mod systems;`、`.manage(...)`、註冊指令、啟動端點。
- `src-tauri/src/commands/mod.rs`、`src/lib/i18n.ts`（zh-TW 與 en 兩份）、`src/components/TerminalApp.tsx`（側邊欄入口）。

---

### Task 1: 資料型別與閘門判定（純函式）

**Files:**
- Create: `src-tauri/src/systems/mod.rs`、`src-tauri/src/systems/types.rs`、`src-tauri/src/systems/gate.rs`
- Modify: `src-tauri/src/lib.rs`（加 `mod systems;`）

- [ ] **Step 1: 建骨架與型別**

`src-tauri/src/systems/mod.rs`：
```rust
//! 系統連線：讓 AI 自主操作外部網頁系統。見
//! docs/superpowers/specs/2026-11-17-system-connections-design.md。

pub mod gate;
pub mod types;
```

`src-tauri/src/systems/types.rs`：
```rust
use serde::{Deserialize, Serialize};

/// 寫入權限模式（每個系統各自一個）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// 非 GET 一律拒絕。
    #[default]
    ReadOnly,
    /// 每個非 GET 都等使用者核准。
    ApproveEach,
    /// 依 `rules` 判定，沒命中的等核准。
    Rules,
    /// 全部放行，只寫審計。
    Autonomous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Allow,
    Ask,
}

/// `rules` 模式的一條規則。`method` 為 `*` 代表任何方法；`url_pattern` 支援 `*` 萬用字元。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub method: String,
    pub url_pattern: String,
    pub action: RuleAction,
}

/// 存在 AppConfig 裡的一個系統連線。不含任何憑證。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemConnection {
    pub id: String,
    pub name: String,
    pub start_url: String,
    /// 允許頂層導覽的網域。子網域也算（`example.com` 允許 `app.example.com`）。
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    Disabled,
    ReadOnly,
    HostNotAllowed,
}

/// 閘門對一個請求的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", content = "reason", rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny(DenyReason),
    /// 暫停請求，等使用者核准。
    Ask,
}
```

- [ ] **Step 2: 寫紅燈測試**（`gate.rs` 先只放測試與空的 `decide` 簽名會編譯失敗，這就是紅燈；或先放 `todo!()`）

`src-tauri/src/systems/gate.rs`：
```rust
use super::types::{Decision, DenyReason, Mode, Rule, RuleAction, SystemConnection};

pub fn decide(_conn: &SystemConnection, _method: &str, _url: &str, _is_navigation: bool) -> Decision {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(mode: Mode) -> SystemConnection {
        SystemConnection {
            id: "s".into(),
            name: "S".into(),
            start_url: "https://app.example.com/".into(),
            allowed_hosts: vec!["example.com".into()],
            mode,
            rules: vec![],
            enabled: true,
        }
    }

    const U: &str = "https://app.example.com/orders";

    #[test]
    fn reads_are_always_allowed_on_allowed_hosts() {
        for m in ["GET", "HEAD", "OPTIONS", "get"] {
            for mode in [Mode::ReadOnly, Mode::ApproveEach, Mode::Rules, Mode::Autonomous] {
                assert_eq!(decide(&conn(mode), m, U, false), Decision::Allow, "{m} {mode:?}");
            }
        }
    }

    #[test]
    fn writes_follow_the_mode() {
        assert_eq!(decide(&conn(Mode::ReadOnly), "POST", U, false), Decision::Deny(DenyReason::ReadOnly));
        assert_eq!(decide(&conn(Mode::ApproveEach), "DELETE", U, false), Decision::Ask);
        assert_eq!(decide(&conn(Mode::Autonomous), "PUT", U, false), Decision::Allow);
        assert_eq!(decide(&conn(Mode::Rules), "PATCH", U, false), Decision::Ask, "沒命中規則的一律問人");
    }

    #[test]
    fn rules_first_match_wins_and_method_is_case_insensitive() {
        let mut c = conn(Mode::Rules);
        c.rules = vec![
            Rule { method: "post".into(), url_pattern: "https://app.example.com/drafts/*".into(), action: RuleAction::Allow },
            Rule { method: "*".into(), url_pattern: "https://app.example.com/*".into(), action: RuleAction::Ask },
        ];
        assert_eq!(decide(&c, "POST", "https://app.example.com/drafts/new", false), Decision::Allow);
        assert_eq!(decide(&c, "POST", "https://app.example.com/orders", false), Decision::Ask);
        assert_eq!(decide(&c, "DELETE", "https://app.example.com/drafts/9", false), Decision::Ask, "方法不同不命中第一條");
    }

    #[test]
    fn navigation_outside_allowed_hosts_is_denied_even_for_get() {
        assert_eq!(
            decide(&conn(Mode::Autonomous), "GET", "https://evil.test/", true),
            Decision::Deny(DenyReason::HostNotAllowed)
        );
        // 子資源（圖片、腳本）的 GET 可以去 CDN。
        assert_eq!(decide(&conn(Mode::ReadOnly), "GET", "https://cdn.other.test/a.js", false), Decision::Allow);
    }

    #[test]
    fn writes_to_other_hosts_are_denied_in_every_mode() {
        for mode in [Mode::ReadOnly, Mode::ApproveEach, Mode::Rules, Mode::Autonomous] {
            assert_eq!(
                decide(&conn(mode), "POST", "https://evil.test/collect", false),
                Decision::Deny(DenyReason::HostNotAllowed),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn host_match_is_a_suffix_on_a_dot_boundary() {
        let c = conn(Mode::Autonomous);
        assert_eq!(decide(&c, "POST", "https://example.com/x", false), Decision::Allow);
        assert_eq!(decide(&c, "POST", "https://a.b.example.com/x", false), Decision::Allow);
        assert_eq!(
            decide(&c, "POST", "https://notexample.com/x", false),
            Decision::Deny(DenyReason::HostNotAllowed),
            "notexample.com 不是 example.com 的子網域"
        );
        assert_eq!(
            decide(&c, "POST", "https://example.com.evil.test/x", false),
            Decision::Deny(DenyReason::HostNotAllowed)
        );
    }

    #[test]
    fn a_disabled_connection_denies_everything() {
        let mut c = conn(Mode::Autonomous);
        c.enabled = false;
        assert_eq!(decide(&c, "GET", U, false), Decision::Deny(DenyReason::Disabled));
    }

    #[test]
    fn glob_match_handles_wildcards() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*c", "abbbc"));
        assert!(glob_match("a*c*e", "abcde"));
        assert!(!glob_match("a*c", "abd"));
        assert!(glob_match("abc", "abc"));
        assert!(!glob_match("abc", "abcd"));
    }
}
```
並在 `lib.rs` 的模組宣告區加 `mod systems;`（位置跟其他 `mod tasks;` 並列）。

- [ ] **Step 3: 跑測試確認紅燈**

Run: `cd src-tauri && cargo test --workspace --no-fail-fast systems::gate 2>&1 | grep -E "test result|panicked|error"`
Expected: FAIL（`todo!()` panic，或 `glob_match` 找不到編譯失敗）。

- [ ] **Step 4: 實作**（取代 `gate.rs` 的 `decide`，並加 `glob_match`、`host_of`、`host_allowed`）
```rust
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?; // 去掉 user:pass@
    let host = authority.split(':').next()?; // 去掉埠
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn host_allowed(allowed: &[String], url: &str) -> bool {
    let Some(host) = host_of(url) else { return false };
    allowed.iter().any(|a| {
        let a = a.trim().trim_start_matches("*.").to_ascii_lowercase();
        !a.is_empty() && (host == a || host.ends_with(&format!(".{a}")))
    })
}

/// 只支援 `*`（任意長度）的萬用字元比對。
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return part.is_empty() || rest.ends_with(part);
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    true
}

/// 閘門判定。`is_navigation`：這是不是頂層頁面導覽（用來套網域白名單；子資源的 GET 可以去 CDN）。
pub fn decide(conn: &SystemConnection, method: &str, url: &str, is_navigation: bool) -> Decision {
    if !conn.enabled {
        return Decision::Deny(DenyReason::Disabled);
    }
    let host_ok = host_allowed(&conn.allowed_hosts, url);
    if is_navigation && !host_ok {
        return Decision::Deny(DenyReason::HostNotAllowed);
    }
    let m = method.to_ascii_uppercase();
    if matches!(m.as_str(), "GET" | "HEAD" | "OPTIONS") {
        return Decision::Allow;
    }
    if !host_ok {
        return Decision::Deny(DenyReason::HostNotAllowed);
    }
    match conn.mode {
        Mode::ReadOnly => Decision::Deny(DenyReason::ReadOnly),
        Mode::ApproveEach => Decision::Ask,
        Mode::Autonomous => Decision::Allow,
        Mode::Rules => conn
            .rules
            .iter()
            .find(|r: &&Rule| (r.method == "*" || r.method.eq_ignore_ascii_case(&m)) && glob_match(&r.url_pattern, url))
            .map(|r| match r.action {
                RuleAction::Allow => Decision::Allow,
                RuleAction::Ask => Decision::Ask,
            })
            .unwrap_or(Decision::Ask),
    }
}
```

- [ ] **Step 5: 跑測試確認全綠**

Run: 同 Step 3。Expected: `test result: ok. 8 passed`（systems::gate）。

- [ ] **Step 6: Commit**
```bash
git add src-tauri/src/systems src-tauri/src/lib.rs
git commit -m "feat(systems): 閘門判定純函式（方法 × 模式 × 規則 × 網域）"
```

---

### Task 2: 設定欄位與連線 CRUD 指令

**Files:**
- Modify: `src-tauri/src/config/types.rs`（`AppConfig` 加欄位）
- Create: `src-tauri/src/commands/systems.rs`
- Modify: `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs`（註冊指令）

- [ ] **Step 1: 紅燈測試**——在 `config/types.rs` 的 tests 加：
```rust
#[test]
fn old_configs_without_system_connections_still_parse_and_default_empty() {
    let cfg: AppConfig = serde_json::from_str("{}").unwrap();
    assert!(cfg.system_connections.is_empty());
}
```
Run: `cd src-tauri && cargo test --workspace --no-fail-fast old_configs_without_system_connections` → 編譯失敗（欄位不存在）。

- [ ] **Step 2: 實作**：`AppConfig` 加
```rust
    /// 系統連線（外部網頁系統）。見 systems/。
    #[serde(default)]
    pub system_connections: Vec<crate::systems::types::SystemConnection>,
```
並在 `Default`（約 402 行 `mcp_servers: vec![]` 旁）加 `system_connections: vec![],`。

- [ ] **Step 3: 指令**。仿 `commands/mcp.rs` 的 `add_mcp_server`／`update`／`remove` 寫法（`config.update(|cfg| ...)` 的實際呼叫形式照該檔 105–115 行），新增：
```rust
#[tauri::command]
pub async fn systems_list(config: State<'_, Arc<ConfigStore>>) -> Result<Vec<SystemConnection>, String> {
    Ok(config.get().system_connections)
}

#[tauri::command]
pub async fn systems_save(conn: SystemConnection, config: State<'_, Arc<ConfigStore>>) -> Result<(), String> {
    validate(&conn)?;
    /* 與 commands/mcp.rs 相同的 update 寫法：id 已存在就取代，否則 push */
}

#[tauri::command]
pub async fn systems_remove(id: String, config: State<'_, Arc<ConfigStore>>) -> Result<(), String> { /* retain */ }

/// 儲存前的檢查：網址要是 http(s)；allowed_hosts 不可為空且必須包含 start_url 的網域。
pub fn validate(conn: &SystemConnection) -> Result<(), String> {
    let host = crate::systems::gate::host_of(&conn.start_url).ok_or("起始網址不正確")?;
    if !(conn.start_url.starts_with("http://") || conn.start_url.starts_with("https://")) {
        return Err("起始網址必須是 http 或 https".into());
    }
    if conn.name.trim().is_empty() {
        return Err("名稱不可空白".into());
    }
    if !conn.allowed_hosts.iter().any(|a| {
        let a = a.trim().trim_start_matches("*.").to_ascii_lowercase();
        !a.is_empty() && (host == a || host.ends_with(&format!(".{a}")))
    }) {
        return Err("允許的網域必須包含起始網址的網域".into());
    }
    Ok(())
}
```
（`gate::host_of` 在 Task 1 已是 `pub`。）`validate` 要有單元測試：空名稱、非 http、網域沒包含起始網址、正常四種。

- [ ] **Step 4: 註冊**：`commands/mod.rs` 加 `pub mod systems;`，`lib.rs` 的 `invoke_handler` 加三個指令。
- [ ] **Step 5: 跑 Rust 全套、確認綠；Commit** `feat(systems): 系統連線設定欄位與 CRUD 指令`。

---

### Task 3: 審計記錄（`systems.db`）

**Files:** Create `src-tauri/src/systems/audit.rs`；Modify `systems/mod.rs`

- [ ] **Step 1: 紅燈測試**（用 `sqlx::SqlitePool::connect("sqlite::memory:")`；寫入三筆、依系統與時間倒序查回、`limit` 生效、`prune_screenshots` 只留最新 N 張的路徑）：
```rust
#[tokio::test]
async fn records_are_listed_newest_first_per_system() {
    let pool = open_memory().await;
    record(&pool, &Entry::new("a", "sys_click", "點儲存", Some("POST"), Some("https://x/y"), Verdict::Approved, "user")).await.unwrap();
    record(&pool, &Entry::new("b", "sys_click", "別的系統", None, None, Verdict::Allow, "rule")).await.unwrap();
    record(&pool, &Entry::new("a", "sys_navigate", "開首頁", Some("GET"), Some("https://x/"), Verdict::Allow, "rule")).await.unwrap();
    let rows = list(&pool, "a", 10).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].tool, "sys_navigate", "最新的在前");
}
```
- [ ] **Step 2: 實作**：`Verdict { Allow, Deny, Approved, Rejected, Timeout, Manual }`（序列化成小寫字串）、`Entry`（`system_id, tool, intent, method, url, verdict, source, screenshot: Option<String>, result: Option<String>, ts: i64`；`Entry::new` 用 `chrono::Utc::now().timestamp_millis()` 或專案已用的時間函式——先 grep 一下 `tasks/store.rs` 取時間的寫法並沿用）、`open(path)`（建表 `CREATE TABLE IF NOT EXISTS system_audit(...)`＋索引 `(system_id, ts)`）、`record`、`list`。`systems.db` 路徑＝`dirs::data_dir()/AITerm/systems.db`（照 `db/knowledge_base.rs` 的資料夾慣例，先讀該檔 `new()` 用的資料夾名稱）。
- [ ] **Step 3: 全綠、Commit** `feat(systems): 審計記錄`。

---

### Task 4: 瀏覽器啟動與設定檔

**Files:** Create `src-tauri/src/systems/browser.rs`；Modify `systems/mod.rs`、`tasks/ask.rs`（把 `detect_browser` 旁新增一個回傳執行檔路徑的函式，供這裡重用，不複製偵測邏輯）

- [ ] **Step 1: 紅燈測試**：
```rust
#[test]
fn profile_dir_is_per_system_and_under_the_app_folder() {
    let a = profile_dir(std::path::Path::new("/data"), "sys-a");
    let b = profile_dir(std::path::Path::new("/data"), "sys-b");
    assert_ne!(a, b);
    assert!(a.starts_with("/data/system-profiles"));
}

#[test]
fn chrome_args_open_a_loopback_debug_port_with_an_isolated_profile() {
    let args = chrome_args(std::path::Path::new("/p"), 19222, "https://x/");
    assert!(args.contains(&"--remote-debugging-port=19222".to_string()));
    assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_string()));
    assert!(args.iter().any(|a| a == "--user-data-dir=/p"));
    assert_eq!(args.last().unwrap(), "https://x/");
    assert!(!args.iter().any(|a| a.contains("headless")), "要有頭：使用者看得到、能接手");
}

#[test]
fn free_port_is_bindable_on_loopback() {
    let p = pick_free_port().unwrap();
    assert!(std::net::TcpListener::bind(("127.0.0.1", p)).is_ok());
}
```
- [ ] **Step 2: 實作**：`profile_dir`、`chrome_args`（`--no-first-run`、`--no-default-browser-check`、`--remote-debugging-address=127.0.0.1`、`--remote-debugging-port=N`、`--user-data-dir=...`、起始網址放最後）、`pick_free_port`（綁 `127.0.0.1:0` 讀埠後放掉）、`chrome_path() -> Option<PathBuf>`（macOS `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`；Windows 沿用 `detect_browser` 的三個環境變數路徑；Linux `/opt/google/chrome/chrome`，再找 PATH 上的 `google-chrome`／`chromium`），`launch(...) -> Result<RunningBrowser, String>`（`tokio::process::Command` spawn，輪詢 `http://127.0.0.1:{port}/json/version` 最多 10 秒取得 `webSocketDebuggerUrl`，`RunningBrowser { child, port, ws_url, profile }`；`close()` 先試 `Browser.close` 失敗再 `kill`）。設定檔目錄建立後在 Unix 設成 `0o700`。
- [ ] **Step 3: 紅燈→綠燈（純函式的三條）；`launch` 另寫一條 `#[ignore]` 的整合測試（需真的 Chrome）：啟動後 `ws_url` 以 `ws://127.0.0.1:` 開頭，`close()` 後埠不再回應。** 手動跑一次：`cargo test --workspace -- --ignored systems::browser` 並貼結果。
- [ ] **Step 4: Commit** `feat(systems): 啟動獨立有頭 Chrome（偵錯埠綁 loopback）`。

---

### Task 5: CDP 閘門（攔截、核准、審計）

**Files:** Create `src-tauri/src/systems/cdp.rs`、`src-tauri/src/systems/approvals.rs`；Modify `systems/mod.rs`

這一步把探勘腳本的邏輯搬進 Rust。探勘已證實：`Target.setAutoAttach{autoAttach:true,waitForDebuggerOnStart:true,flatten:true}` ＋ 對 `page` 類型的 target 發 `Fetch.enable{patterns:[{urlPattern:"*"}]}` ＋ `Runtime.runIfWaitingForDebugger`；對既有分頁要主動 `Target.attachToTarget{flatten:true}`；**每個 target 只能附加一次**（用 `HashSet<TargetId>` 去重）。

- [ ] **Step 1: `approvals.rs` 紅燈測試**：
```rust
#[tokio::test]
async fn a_pending_approval_resolves_with_the_users_decision() {
    let a = Approvals::new();
    let (id, rx) = a.open(Pending::sample("s1"));
    assert!(a.decide(&id, true));
    assert_eq!(rx.await.unwrap(), Outcome::Approved);
}

#[tokio::test]
async fn an_unanswered_approval_times_out_as_rejected() {
    let a = Approvals::new();
    let (_id, rx) = a.open(Pending::sample("s1"));
    let out = Approvals::wait(rx, std::time::Duration::from_millis(30)).await;
    assert_eq!(out, Outcome::TimedOut);
}

#[tokio::test]
async fn stop_rejects_everything_pending_for_that_system() {
    let a = Approvals::new();
    let (_i1, r1) = a.open(Pending::sample("s1"));
    let (_i2, r2) = a.open(Pending::sample("s2"));
    a.reject_all("s1");
    assert_eq!(r1.await.unwrap(), Outcome::Rejected);
    assert!(a.has_pending("s2"));
    drop(r2);
}
```
- [ ] **Step 2: 實作 `approvals.rs`**：`Pending { system_id, intent, method, url, tool, screenshot: Option<String>, declared_read: bool }`、`Outcome { Approved, Rejected, TimedOut }`、`Approvals`（`Mutex<HashMap<String, (Pending, oneshot::Sender<Outcome>)>>`；`open`→`(id, Receiver)`；`decide(id, approve)`；`reject_all(system_id)`；`has_pending(system_id)`；`list()` 給前端；`wait(rx, timeout)`）。id 用 `uuid`（已在依賴）。
- [ ] **Step 3: `cdp.rs` 實作** `run_gate(ws_url, conn: Arc<Mutex<SystemConnection>>, approvals: Arc<Approvals>, audit: SqlitePool, intent: Arc<Mutex<Option<String>>>, emit: impl Fn(GateEvent)+Send+Sync+'static, manual: Arc<AtomicBool>)`：
  - `tokio_tungstenite::connect_async(ws_url)`，把 sink 與 stream 拆開（`futures_util::StreamExt/SinkExt`）；一個 `mpsc` 讓其他 task 送出 CDP 指令；`id` 遞增，回應用 `HashMap<u64, oneshot>`。
  - 啟動時送 `Target.setDiscoverTargets`、`Target.setAutoAttach`、對現有 `page` 做 `Target.attachToTarget`＋`Fetch.enable`；`Target.attachedToTarget` 事件對 `page` 做同樣處理（去重）。
  - `Fetch.requestPaused`：取 `request.method`、`request.url`、`resourceType`（`Document` 且 `frameId` 為頂層時 `is_navigation = true`）。`manual` 旗標為真（使用者接手中）→ 直接 `continueRequest` 並審計 `Manual`。否則呼叫 `gate::decide`：
    - `Allow` → `Fetch.continueRequest`；非 GET 時寫審計 `Allow`（GET 不逐筆寫，免得審計被子資源淹沒）。
    - `Deny(r)` → `Fetch.failRequest{errorReason:"BlockedByClient"}`；審計 `Deny`；`emit(GateEvent::Denied{..})`。
    - `Ask` → `approvals.open(Pending{ intent: 目前 intent, .. })`，`emit(GateEvent::NeedApproval{id,..})`，**另開 task** 等 `Approvals::wait(rx, 5分鐘)`，依結果 `continueRequest`／`failRequest`，審計 `Approved`／`Rejected`／`Timeout`。不可在讀取迴圈裡直接 await，否則會卡住後續所有事件。
  - 宣告為讀（`intent` 以「讀」開頭或 `declared_read=true`）卻到了非 GET → `Pending.declared_read = true` 且核准卡標警示。
- [ ] **Step 4: 端到端驗證（`#[ignore]`，需 Chrome）**：寫 `src-tauri/tests/systems_gate_e2e.rs`：本機 `tokio` TCP 假網站（`GET /` 回含 form 的 HTML；`POST /submit` 把 `AtomicUsize` +1），用 `browser::launch` 開 Chrome，`run_gate` 以 `read_only` 模式；用 CDP `Runtime.evaluate`（測試自己的第三個用戶端）或直接導覽後 `document.forms[0].submit()` 觸發 POST。斷言：**`AtomicUsize == 0`**；改 `autonomous` 後再試 → `== 1`；`approve_each` 下先 `decide(id,false)` → `== 0`、`decide(id,true)` → `== 1`。這條是整個功能最重要的測試，必須親眼看到紅（先把 `decide` 暫時改成永遠 Allow，確認計數器變 1 而測試失敗）再還原。
- [ ] **Step 5: Commit** `feat(systems): CDP 寫入閘門與核准佇列`。

---

### Task 6: Playwright MCP 代理（intent、逾時改寫、拒絕改寫）

**Files:** Create `src-tauri/src/systems/proxy.rs`；Modify `systems/mod.rs`

- [ ] **Step 1: 紅燈測試（純函式部分）**：
```rust
#[test]
fn only_whitelisted_tools_are_exposed_and_get_system_and_intent_params() {
    let upstream = serde_json::json!([
        {"name":"browser_click","description":"Click","inputSchema":{"type":"object","properties":{"target":{"type":"string"}},"required":["target"]}},
        {"name":"browser_evaluate","description":"Eval","inputSchema":{"type":"object","properties":{}}},
        {"name":"browser_snapshot","description":"Snap","inputSchema":{"type":"object","properties":{}}}
    ]);
    let tools = expose_tools(upstream.as_array().unwrap());
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["sys_click", "sys_snapshot"], "evaluate 不開放");
    let click = &tools[0];
    assert_eq!(click["inputSchema"]["required"], serde_json::json!(["target", "system", "intent"]));
    let snap = &tools[1];
    assert_eq!(snap["inputSchema"]["required"], serde_json::json!(["system"]), "唯讀的 snapshot 不必 intent");
}

#[test]
fn acting_tools_without_intent_are_rejected() {
    let err = check_intent("sys_click", &serde_json::json!({"system":"s","target":"e4"})).unwrap_err();
    assert!(err.contains("intent"));
    assert!(check_intent("sys_click", &serde_json::json!({"system":"s","target":"e4","intent":"讀：點查詢"})).is_ok());
    assert!(check_intent("sys_snapshot", &serde_json::json!({"system":"s"})).is_ok());
}

#[test]
fn a_timeout_while_an_approval_is_pending_is_not_shown_to_the_ai() {
    let timeout = serde_json::json!({"content":[{"type":"text","text":"### Error\nTimeoutError: browserBackend.callTool: Timeout 5000ms exceeded."}],"isError":true});
    assert!(is_playwright_timeout(&timeout));
    assert!(!is_playwright_timeout(&serde_json::json!({"content":[{"type":"text","text":"ok"}]})));
}

#[test]
fn a_gate_blocked_page_is_rewritten_for_the_ai() {
    let blocked = serde_json::json!({"content":[{"type":"text","text":"### Page\n- Page URL: chrome-error://chromewebdata/\n"}]});
    assert!(is_blocked_page(&blocked));
    let msg = blocked_message("POST /orders");
    assert!(msg.contains("被閘門拒絕") && msg.contains("不可重試"));
}
```
動作類（需 intent）工具：`browser_click`、`_type`、`_fill_form`、`_press_key`、`_select_option`、`_drag`、`_navigate`、`_handle_dialog`；唯讀類：`_snapshot`、`_take_screenshot`、`_find`、`_wait_for`、`_tabs`、`_navigate_back`、`_hover`、`_console_messages`、`_network_requests`、`_network_request`、`_resize`。不開放：`_evaluate`、`_run_code_unsafe`、`_file_upload`、`_drop`、`_close`、`_emulate_media`。
- [ ] **Step 2: 實作純函式**：`expose_tools`（改名加 `sys_`、`inputSchema.properties` 加 `system`（string）與動作類加 `intent`（string，描述寫明「一句話說明要做什麼、是讀還是寫」）、`required` 依上面測試的順序追加）、`check_intent`、`is_playwright_timeout`、`is_blocked_page`、`blocked_message`。
- [ ] **Step 3: 實作代理本體** `Proxy`：用 `mcp::transport::StdioTransport::spawn` 以 `npx -y {PLAYWRIGHT_MCP} --cdp-endpoint http://127.0.0.1:{port} --output-dir {該系統的截圖資料夾}`（Windows 要用 `npx.cmd` 的處理照 `StdioTransport::spawn` 既有做法，先讀 `mcp/transport.rs` 38–60 行）啟動；`initialize`＋`notifications/initialized`＋`tools/list` 快取。`call(tool, args)`：
  1. `check_intent`；記下 `intent` 到共享的 `Arc<Mutex<Option<String>>>`（給 Task 5 的閘門讀）。
  2. 去掉 `system`／`intent` 後轉給子行程的 `browser_*`。
  3. 結果若是 `is_playwright_timeout` 且 `approvals.has_pending(system)` → 不回給 AI，改 `await` 該核准解決（上限同核准逾時），之後再呼叫一次 `browser_snapshot` 回「已核准／已拒絕」加最新快照。
  4. 結果若 `is_blocked_page` → 呼叫 `browser_navigate_back`，回 `blocked_message(...)`。
  5. 接手中（`manual` 旗標）→ 直接回「使用者接手中，請稍候」不轉發。
- [ ] **Step 4: 整合測試（`#[ignore]`）**：真 Chrome＋真 Playwright MCP＋Task 5 的假網站，走完「導覽→snapshot→click Save」在 `read_only` 下：AI 收到的是 `blocked_message` 而不是 Chrome 錯誤頁，計數器 0；`approve_each` 下 click 回傳前先被核准、之後回「已核准」。
- [ ] **Step 5: Commit** `feat(systems): Playwright MCP 代理（intent、逾時與拒絕改寫）`。

---

### Task 7: 動態 MCP 端點

**Files:** Create `src-tauri/src/systems/endpoint.rs`；Modify `systems/mod.rs`、`lib.rs`

現有的 `mcp_server/` 用 `rmcp` 的 `#[tool]` 靜態方法，無法動態代理工具清單，所以這裡仿 `tasks/ask.rs` 手寫 JSON-RPC 的作法（`initialize`／`tools/list`／`tools/call`，回 JSON，不用串流），掛在 axum 上、用 `bridge::auth` 的 bearer token 驗證（金鑰鑰匙圈 key：`systems-endpoint:token`，沿用 `MCP_TOOL_SERVER_TOKEN_KEY` 的產生方式）。

- [ ] **Step 1: 紅燈測試**：`handle_rpc(request_json) -> Option<Value>` 純函式分派：`initialize` 回 `protocolVersion`＋`serverInfo`；`notifications/initialized` 回 `None`；`tools/list` 回代理的工具清單；未知方法回 JSON-RPC error `-32601`；`tools/call` 對未知 `system`、已停用系統回 `isError:true` 與明確文字。
- [ ] **Step 2: 實作**：axum `Router` `POST /systems/mcp`（只綁 127.0.0.1，埠＝固定預設 8320，被占用就報錯不漂移，與 `McpToolServerState::start` 同理）。
- [ ] **Step 3: 接到兩個消費端**：
  - Ask AI 迴圈：在 `AppConfig.mcp_servers` 自動維護一筆 `id = "aiterm-systems"`、`transport = http`、`url = http://127.0.0.1:8320/systems/mcp` 的項目（有任一個已啟用的系統連線才存在；沒有就移除）。**先讀 `mcp/transport.rs` 的 `HttpTransport` 怎麼帶 bearer header，不支援就補上 headers 欄位。**
  - Claude Code：`mcp-config` 的 `{"type":"http","url":..., "headers":{"Authorization":"Bearer ..."}}`，提供一個指令回傳可貼的 JSON，設定頁顯示。
- [ ] **Step 4: Commit** `feat(systems): 動態 MCP 端點，Ask AI 與 Claude Code 共用`。

---

### Task 8: 狀態、事件與控制指令（停止／接手／停用／登入）

**Files:** Modify `systems/mod.rs`（`SystemsState`）、`commands/systems.rs`、`lib.rs`

- [ ] **Step 1: 紅燈測試**：`SystemsState` 的純邏輯——`set_manual(id,true)` 後 `is_manual(id)`；`stop(id)` 會 `reject_all`；`disable(id)` 會設 `enabled=false`、`reject_all`、並回傳「需要關閉瀏覽器」。
- [ ] **Step 2: 實作 `SystemsState`**：每個系統一個 `Running { browser, gate_task, proxy, manual: Arc<AtomicBool> }`；`ensure_running(conn)`（沒開就 `browser::launch`＋`run_gate`＋`Proxy::start`，已開就重用）；`close(id)`。
- [ ] **Step 3: 指令**：`systems_open_login(id)`（`ensure_running`，視窗已開就只回狀態；使用者在可見視窗登入）、`systems_pending()`（核准清單）、`systems_decide(approval_id, approve)`、`systems_stop(id)`（`reject_all`＋通知取消目前工具呼叫）、`systems_takeover(id)`／`systems_handback(id)`、`systems_set_enabled(id, enabled)`（停用＝關瀏覽器＋拒絕後續）、`systems_audit(id, limit)`。事件（`app.emit`）：`systems://need-approval`、`systems://denied`、`systems://state`。
- [ ] **Step 4: session 過期偵測**：代理在每次 `snapshot`／導覽結果的頁面網址不屬於 `start_url` 的登入頁特徵（網域不在白名單、或網址路徑含 `login|signin|sso`）時，回覆「登入已失效，請在視窗重新登入後告訴我」並發 `systems://state {needs_login:true}`。
- [ ] **Step 5: 全綠、Commit** `feat(systems): 執行狀態、停止／接手／停用與登入指令`。

---

### Task 9: 前端

**Files:** Create `src/ipc/systems.ts`、`src/components/SystemsView/{index.tsx,ConnectionForm.tsx,ApprovalCards.tsx,AuditLog.tsx,index.css}` 與各 `.test.tsx`；Modify `src/lib/i18n.ts`（zh-TW 與 en 都要加，並補 locale 漂移測試會比對的鍵）、`src/components/TerminalApp.tsx`、`src/components/TabBar`（側邊欄入口與分頁型別，先讀 `project_tab_types_agent_naming` 提到的陷阱：Agent 相關分頁不是 code-assistant 型別）。

- [ ] **Step 1: `ipc/systems.ts`**：與 Task 2/8 指令一一對應的型別化包裝（`listSystems`、`saveSystem`、`removeSystem`、`openLogin`、`pendingApprovals`、`decideApproval`、`stopSystem`、`takeover`、`handback`、`setEnabled`、`auditLog`、`mcpConfigJson`），事件訂閱 helper 要有 `mountedRef` 防呆（照 `useAiChat.ts`）與**先訂閱再取初始狀態**（`feedback_event_before_subscribe_race`）。
- [ ] **Step 2: 紅燈測試→實作**（每個元件一輪）：
  - `ConnectionForm`：名稱、起始網址、允許網域（預設帶入起始網址的網域）、模式四選一（`rules` 才顯示規則編輯）；`autonomous` 要二次確認文字；儲存時前端也擋空名稱。
  - `ApprovalCards`：顯示 intent、`POST /orders`、截圖縮圖；宣告為讀卻寫入時顯示警示；按鈕「核准／拒絕／拒絕並停用」；多張排隊時依時間排序。
  - `AuditLog`：依系統篩選、顯示判定與來源。
  - `SystemsView`：連線清單＋每個系統的狀態（執行中／等核准／已暫停／接手中／需要登入）、按鈕「開啟並登入」「停止」「我來接手／交還 AI」「停用」；頂部顯示 MCP 設定 JSON（給 Claude Code 貼）。
- [ ] **Step 3: 核准通知**：收到 `systems://need-approval` 時，不論目前在哪個分頁都要看得到（沿用現有的通知／toast 機制，先 grep 既有 toast 元件）；點通知跳到系統頁面。
- [ ] **Step 4: `npm run test`、`npx tsc -b`、`npm run lint` 全綠；Commit** `feat(systems): 系統連線頁面、核准卡與審計`。

---

### Task 10: 實機驗收與收尾

- [ ] **Step 1: 用本機假網站實機走一遍**（不碰使用者的正式系統）：`tauri:dev` 啟動（照 `feedback_verify_running_binary`：乾淨重起、確認跑的是新二進位），新增連線指向假網站，開啟登入視窗，用 Ask AI 下「把表單送出」：`read_only` → 被拒且伺服器計數 0；`approve_each` → 出現核准卡、核准後計數 1；按「停止」「接手」「停用」各驗一次；關掉登入視窗後 AI 回報需要重新登入。每一項記錄實際看到的畫面行為（`feedback_changelog_verify_ui_claims`：只寫真的看過的）。
- [ ] **Step 2: Windows／Linux**：本機只有 macOS，Windows 用 Parallels（`reference_mac_ui_automation`／`project_parallels_shared_repo_risk` 提醒共用 `.git` 風險）驗 Chrome 路徑偵測與偵錯埠；做不到的平台在 PR／回報裡明講「未驗證」。
- [ ] **Step 3: 文件**：在 `CHANGELOG.md` 補一段給使用者看的說明（只寫實測過的行為，不打 tag）；把已知限制（GET 改資料、POST 查詢吵、偵錯埠對本機可見）寫進系統頁面的說明文字與 i18n。
- [ ] **Step 4: 驗收清單（`superpowers:verification-before-completion`）**：`cargo test --workspace --no-fail-fast` 逐行看 `test result:`、`-- --ignored` 的整合測試實跑結果、`npm run test`、`npx tsc -b`、`npm run lint`；然後 `superpowers:requesting-code-review` 自我審查；最後問使用者要不要 push／合併。

---

## 後續期數（各自另開 spec，不在本計畫內）

第二期配方沉澱、第三期定時執行、第四期信件／Telegram 觸發。第一期的 `intent` 與審計記錄是配方沉澱的資料來源。

## Self-Review

- **規格涵蓋**：連線設定與模式（T1、T2）、登入保存與 session 過期（T4、T8）、兩層閘門（T1、T5、T6）、核准卡（T5、T8、T9）、即時預覽／停止／接手（有頭視窗＋T8、T9）、審計（T3）、錯誤處理的逾時與拒絕改寫（T6）、停用開關（T8）、跨平台（T4、T10）、測試（T1、T5、T6、T10）。已知限制寫入 T10。
- **類型一致**：`SystemConnection`／`Mode`／`Rule`／`Decision`／`DenyReason` 全由 T1 定義；`Approvals`／`Pending`／`Outcome` 由 T5 定義，T6、T8 引用同名；`Proxy`、`SystemsState`、`RunningBrowser` 各只定義一處。
- **刻意留給實作者讀碼再決定的細節**（不是留白，是需要先讀的既有碼）：`ConfigStore.update` 的呼叫形式（照 `commands/mcp.rs`）、取時間函式（照 `tasks/store.rs`）、`HttpTransport` 是否能帶 header（T7 Step 3 明寫要先讀）、資料夾名稱（照 `db/knowledge_base.rs`）。
