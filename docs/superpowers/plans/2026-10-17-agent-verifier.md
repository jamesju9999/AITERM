# Agent 驗收者 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 加入「驗收者」角色：協調者合併後在整合結果上實際建置、測試、操作（網頁用瀏覽器），通過才更新程式庫，不通過依模組退回工程師。

**Architecture:** 新 Agent 種類 `verifier` 與卡片種類 `verify`。`workflow::after_card_finished` 在合併成功後建驗收卡，驗收卡在 detached worktree 跑（同審查卡），判定寫 `verify.json`；通過才快轉程式庫，失敗依 `module_id` 開修正卡，修正卡的 `review_of` 指向模組鏈尾端，讓既有的模組鏈邏輯（加兩條規則）自然接上。瀏覽器透過 Playwright MCP（版本釘死）掛在驗收卡的 `mcp.json`。

**Tech Stack:** Rust（sqlx、tokio、tauri 2）、React 19 + Vitest、Playwright MCP 0.0.83。

Spec：`docs/superpowers/specs/2026-10-17-agent-verifier-design.md`

**驗證指令**（每個 Task 結束都跑相關的；最後全跑）：
- Rust：`cd src-tauri && CARGO_TARGET_DIR=/Users/jamesju/Documents/GitHub/AITERM-wt-quota/src-tauri/target cargo test --workspace --no-fail-fast`（看每一行 `test result:`）
- 前端：`npx vitest run`、`npx tsc -b`、`npx eslint <改過的檔案>`
- 每個新測試都要做突變檢查證明會紅；備份用 `mktemp`。

---

## File Structure

| 檔案 | 責任 |
|---|---|
| `src-tauri/src/tasks/agents.rs` | `verifier` 種類、每專案一位 |
| `src-tauri/src/tasks/workflow.rs` | `KIND_VERIFY`、模組鏈新規則、`verify.json` 解析、驗收卡／驗收修正卡內文、合併後與驗收後的交接、放行 |
| `src-tauri/src/tasks/verify_ports.rs`（新） | 找空閒埠、結束監聽那些埠的行程 |
| `src-tauri/src/tasks/ask.rs` | `mcp.json` 可以同時放 aiterm 與 playwright |
| `src-tauri/src/tasks/scheduler.rs` | 派驗收卡（唯讀 worktree、埠、MCP）、`finalize_verify_card`、驗收事件 |
| `src-tauri/src/commands/agents.rs` | `agents_verify_report`、`agents_accept_verification` |
| `src/ipc/agents.ts` | 型別與 IPC |
| `src/components/AgentCanvas/*` | 角色池、卡片、連線、頂列、驗收報告對話框 |
| `src/lib/i18n.ts` | 字串 |

---

### Task 1: `verifier` Agent 種類

**Files:** Modify `src-tauri/src/tasks/agents.rs:64-80`; Test 同檔 tests 模組

- [ ] **Step 1: 失敗測試**（加在既有 `create_agent` 單例測試旁，`mk(kind)` 是同檔既有 helper）

```rust
#[tokio::test]
async fn one_verifier_per_project() {
    let pool = mem_pool().await;
    create_agent(&pool, &mk("verifier")).await.unwrap();
    let err = create_agent(&pool, &mk("verifier")).await.unwrap_err().to_string();
    assert!(err.contains("驗收者"), "{err}");
}
```

- [ ] **Step 2:** 跑 `cargo test --lib one_verifier_per_project`，預期 FAIL（未知的 Agent 種類）。
- [ ] **Step 3: 實作**

```rust
"reviewer" | "orchestrator" | "verifier" => {
    // …既有 COUNT 查詢…
    if n > 0 {
        let label = match a.kind { "reviewer" => "審核者", "orchestrator" => "協調者", _ => "驗收者" };
        return Err(sqlx::Error::Protocol(format!("每個專案最多一個{label}")));
    }
}
```

- [ ] **Step 4:** 測試通過；突變（把 `"verifier"` 從 match 拿掉）會紅。
- [ ] **Step 5:** commit `feat(agents): verifier agent kind`

---

### Task 2: 模組鏈接上驗收退回的修正卡（Rust 與前端同規則）

**Files:** Modify `src-tauri/src/tasks/workflow.rs` `walk()`；`src/components/AgentCanvas/deriveModuleState.ts` `walk()`；Test 兩邊的測試檔

規則（spec §4.4）：
1. 走到 verdict = `pass` 的審查卡 → 找 `card_kind == fix && review_of == 該審查卡` 的修正卡，有就 `path.push(fix); cur = fix; continue`。
2. 找不到審查卡 → 找 `card_kind == fix && review_of == cur` 的修正卡，有就接著走。
3. 這兩種不加 `fails`。

- [ ] **Step 1: Rust 失敗測試**（workflow.rs tests，用同檔 `card(...)` 建 TaskRow 的方式；沒有就寫一個 `row(id, kind, status, outcome, verdict, module_id, review_of)` helper）

```rust
#[test]
fn a_fix_from_verification_reopens_a_passed_module() {
    let m = row("m", KIND_WORK, "done", Some("success"), None, None, None);
    let r = row("r", KIND_REVIEW, "done", Some("success"), Some("pass"), Some("m"), Some("m"));
    let f = row("f", KIND_FIX, "queued", None, None, Some("m"), Some("r"));
    assert_eq!(module_state(&m, &[r.clone()], true), ModuleState::Passed);
    assert_eq!(module_state(&m, &[r.clone(), f.clone()], true), ModuleState::FixQueued);
    let f_done = row("f", KIND_FIX, "done", Some("success"), None, Some("m"), Some("r"));
    assert_eq!(module_state(&m, &[r.clone(), f_done.clone()], true), ModuleState::AwaitingReview);
    let r2 = row("r2", KIND_REVIEW, "done", Some("success"), Some("pass"), Some("m"), Some("f"));
    assert_eq!(module_state(&m, &[r, f_done, r2], true), ModuleState::Passed);
}

#[test]
fn without_a_reviewer_a_verification_fix_hangs_off_the_module() {
    let m = row("m", KIND_WORK, "done", Some("success"), None, None, None);
    let f = row("f", KIND_FIX, "running", None, None, Some("m"), Some("m"));
    assert_eq!(module_state(&m, &[f], false), ModuleState::Coding);
}

#[test]
fn verification_fixes_do_not_count_as_review_rounds() {
    // 審查 pass → 驗收修正 → 審查 fail → 修正 → 審查 fail：只算 2 次審查退回，還沒到上限。
    let m = row("m", KIND_WORK, "done", Some("success"), None, None, None);
    let chain = vec![
        row("r1", KIND_REVIEW, "done", Some("success"), Some("pass"), Some("m"), Some("m")),
        row("f1", KIND_FIX, "done", Some("success"), None, Some("m"), Some("r1")),
        row("r2", KIND_REVIEW, "done", Some("success"), Some("fail"), Some("m"), Some("f1")),
        row("f2", KIND_FIX, "done", Some("success"), None, Some("m"), Some("r2")),
        row("r3", KIND_REVIEW, "done", Some("success"), Some("fail"), Some("m"), Some("f2")),
    ];
    assert_eq!(module_state(&m, &chain, true), ModuleState::FixQueued);
}
```

- [ ] **Step 2:** 跑，前兩條 FAIL（目前停在 Passed）。
- [ ] **Step 3: 實作 Rust `walk`**

```rust
loop {
    let review = chain.iter().find(|c| c.card_kind == KIND_REVIEW && c.review_of.as_deref() == Some(cur.id.as_str()));
    let Some(review) = review else {
        // 沒有審查者時，驗收退回的修正卡直接接在目前這張後面。
        match chain.iter().find(|c| c.card_kind == KIND_FIX && c.review_of.as_deref() == Some(cur.id.as_str())) {
            Some(fix) => { path.push(fix); cur = fix; continue; }
            None => break,
        }
    };
    path.push(review);
    if review.verdict.as_deref() != Some("fail") {
        // 審查通過後被驗收退回：修正卡接在這張審查卡後面。
        match chain.iter().find(|c| c.card_kind == KIND_FIX && c.review_of.as_deref() == Some(review.id.as_str())) {
            Some(fix) => { path.push(fix); cur = fix; continue; }
            None => break,
        }
    }
    fails += 1;
    // …既有：找 fail 後的 fix…
}
```

- [ ] **Step 4: 前端同樣三條測試**（`deriveModuleState.test.ts`，用該檔既有的 card helper），先紅，再照同規則改 `walk`。
- [ ] **Step 5:** 兩邊測試通過；兩邊各做突變（拿掉規則 1）會紅。
- [ ] **Step 6:** commit `feat(workflow): verification fixes reopen a passed module`

---

### Task 3: `verify.json` 與內文

**Files:** Modify `src-tauri/src/tasks/workflow.rs`

- [ ] **Step 1: 失敗測試**

```rust
#[test]
fn verify_json_items_decide_the_verdict() {
    let ok = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"pass"},{"criterion":"b","status":"manual","notes":"要付款"}]}"#).unwrap();
    assert_eq!(ok.verdict, "pass");
    // 自稱 pass 但有 fail 項 → 以 items 為準。
    let lied = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"fail","module_id":"m"}]}"#).unwrap();
    assert_eq!(lied.verdict, "fail");
    assert!(parse_verify_json(r#"{"verdict":"maybe","items":[]}"#).is_err());
    assert!(parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"skipped"}]}"#).is_err());
    assert!(parse_verify_json("\u{feff}{\"verdict\":\"pass\",\"items\":[]}").is_ok(), "BOM");
}

#[test]
fn the_verify_prompt_lists_modules_ports_evidence_dir_and_project_types() {
    let m = row("m1", KIND_WORK, "done", Some("success"), None, None, None);
    let body = verify_body(&[&m], "", Path::new("/p/tasks/v/verify.json"), Path::new("/p/tasks/v/evidence"));
    for needle in ["m1", "/p/tasks/v/verify.json", "/p/tasks/v/evidence", "網頁前端", "manual", "module_id"] {
        assert!(body.contains(needle), "缺少 {needle}：{body}");
    }
}

#[test]
fn a_verification_fix_carries_the_failed_items() {
    let m = row("m1", KIND_WORK, "done", Some("success"), None, None, None);
    let item = VerifyItem { criterion: "按開始會倒數".into(), status: "fail".into(), module_id: Some("m1".into()),
        steps: Some("1. 按開始".into()), expected: Some("25:00 開始倒數".into()), actual: Some("沒反應".into()),
        evidence: vec!["evidence/start.png".into()], notes: None };
    let body = verify_fix_body(&m, &[&item], "v1", 1);
    for needle in ["按開始會倒數", "1. 按開始", "沒反應", "evidence/start.png", "第 1 輪"] {
        assert!(body.contains(needle), "{needle}");
    }
}
```

- [ ] **Step 2:** FAIL（函式不存在）。
- [ ] **Step 3: 實作**

```rust
pub const KIND_VERIFY: &str = "verify";

pub fn verify_json_path(project_path: &Path, verify_id: &str) -> PathBuf { task_dir(project_path, verify_id).join("verify.json") }
pub fn evidence_dir(project_path: &Path, verify_id: &str) -> PathBuf { task_dir(project_path, verify_id).join("evidence") }

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct VerifyItem {
    pub criterion: String,
    pub status: String, // pass | fail | manual
    #[serde(default)] pub module_id: Option<String>,
    #[serde(default)] pub evidence: Vec<String>,
    #[serde(default)] pub steps: Option<String>,
    #[serde(default)] pub expected: Option<String>,
    #[serde(default)] pub actual: Option<String>,
    #[serde(default)] pub notes: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct VerifyReport {
    pub verdict: String,
    #[serde(default)] pub project_type: Option<String>,
    #[serde(default)] pub items: Vec<VerifyItem>,
    #[serde(default)] pub notes: Option<String>,
}

pub fn parse_verify_json(text: &str) -> Result<VerifyReport, String> {
    let mut v: VerifyReport = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    if !matches!(v.verdict.as_str(), "pass" | "fail") { return Err(format!("verdict 必須是 pass 或 fail，收到 {}", v.verdict)); }
    if let Some(bad) = v.items.iter().find(|i| !matches!(i.status.as_str(), "pass" | "fail" | "manual")) {
        return Err(format!("status 必須是 pass、fail 或 manual，收到 {}", bad.status));
    }
    if v.items.iter().any(|i| i.status == "fail") { v.verdict = "fail".into(); }
    Ok(v)
}
```

`verify_body(modules, spec_text, json_path, evidence_dir)`：繁中提示，內容依序：你是驗收者、不要修改或 commit 任何檔案；模組清單（`- [<id>] <title>\n  需求：<body 前 2000 字>`）；有規格就附規格全文；spec §5 的類型表；瀏覽器工具（`mcp__playwright__*`）的用法與「截圖存成 `<evidence_dir>/<名稱>.png` 的絕對路徑」；伺服器只能用派工時附上的埠、結束前關掉；工具不能用時把需要它的條件標成 `manual` 寫原因；verify.json 的格式（spec §7）與路徑；`module_id` 用上面清單的 id，找不出來就省略。

`verify_fix_body(module, items, verify_id, round)`：`驗收沒有通過，請修正模組「…」（驗收退回第 N 輪，最多 2 輪）` ＋ 每項的條件、重現步驟、預期、實際、證據路徑（相對於驗收卡資料夾 `tasks/<verify_id>/`）＋ 原始需求。

- [ ] **Step 4:** 通過；突變（拿掉「有 fail 就改 fail」）會紅。
- [ ] **Step 5:** commit `feat(workflow): verify.json and verifier prompts`

---

### Task 4: 合併後與驗收後的交接

**Files:** Modify `src-tauri/src/tasks/workflow.rs` `after_card_finished`、`WorkflowEvent`；新增 `accept_verification`

- [ ] **Step 1: 失敗測試**（`workflow.rs` tests；沿用同檔 `agent(&pool, kind, name)`、建卡 helper。fast-forward 的部分要真的 git repo：沿用同檔既有測 `fast_forward_repo` 的 tempdir repo 寫法）

1. `merge_with_a_verifier_queues_a_verify_card_and_leaves_the_repo_alone`：有 verifier 時合併卡 success → 建一張 `verify` 卡（agent = verifier、`review_of` = 合併卡、`start_sha`/`end_sha` = 合併卡的），合併卡 `verdict` 仍是 NULL，程式庫 HEAD 不變。
2. `merge_without_a_verifier_still_fast_forwards`：既有行為不變（合併卡 verdict = `repo_merged`）。
3. `a_passed_verification_fast_forwards_and_marks_the_merge_card`：驗收卡 verdict=pass、success → 合併卡 verdict = `repo_merged`，程式庫 HEAD = 受驗 sha。
4. `a_failed_verification_sends_fixes_to_the_owning_coders`：兩個模組（不同工程師），items 一項指 m1、一項指 m2、一項沒 module_id → 兩張 fix 卡（各自工程師、`module_id`、`review_of` = 各模組鏈尾），回傳 `WorkflowEvent::VerifyAttention`（因為有找不到模組的項目），驗收卡 verdict 仍 `fail`。
5. `a_failure_nobody_owns_stops_for_the_user`：items 全部沒有有效 module_id（含指向不存在 id 的）→ 不建卡、驗收卡 verdict 改 `attention`、回傳 VerifyAttention。
6. `the_third_failed_verification_in_a_row_stops_for_the_user`：前面已有兩張 fail 驗收卡（finished_at 較早、之間沒有 pass）→ 第三張不建修正卡、verdict 改 `attention`。中間有一張 pass 就重新計算。
7. `accepting_a_verification_fast_forwards`：`accept_verification(pool, verify_id)` 只接受 verdict `fail`／`attention` 的驗收卡；之後 verdict = `pass`、合併卡 verdict = 快轉結果。

- [ ] **Step 2:** FAIL。
- [ ] **Step 3: 實作**
  - `WorkflowEvent` 加 `VerifyAttention { verify_id: String, unowned: usize }`。scheduler 那邊 `let … = ev;` 改成 `match`（Task 6）。
  - `KIND_MERGE if success`：`find_kind(&all_agents, "verifier")` 有 → `create_verify_card(pool, project_path, &task, verifier)`（插卡後補 body，同審查卡的作法），否則照舊 `fast_forward_repo`。
  - `KIND_VERIFY if success`：
    - `pass` → `ff = fast_forward_repo(...)`，`UPDATE tasks SET verdict = ff WHERE id = review_of`。
    - `fail` → 讀 `verify.json`（`verify_json_path`）。依 `module_id` 分組，只留「是 coder 名下 work 卡」的 id；算輪數（`verify_fail_streak`：同專案 `card_kind='verify'` 依 `finished_at` 由新到舊，數到第一張 pass 為止的 fail/attention 張數，含這張）。`streak > MAX_FIX_ROUNDS` 或沒有任何有效模組 → `UPDATE verdict='attention'`、事件；否則每個模組建修正卡（`agent_id` = 模組工程師、`review_of` = `walk(module, chain).0.last().id`、`sort_order` 插到該工程師佇列最前，同審查退回）；有找不到模組的項目也發事件。
  - `pub async fn accept_verification(pool, project_path?, verify_id)`：檢查卡種與 verdict，`UPDATE verdict='pass'`，再做通過的那段（抽成 `on_verify_passed`）。
- [ ] **Step 4:** 通過；突變：(a) 合併後不判斷 verifier 直接快轉 → 測 1 紅；(b) 修正卡 `review_of` 用驗收卡 id → 測 4 紅（鏈接不上會被 Task 2 規則忽略，斷言 module_state = FixQueued 要放進測 4）；(c) streak 不停在 pass → 測 6 紅。
- [ ] **Step 5:** commit `feat(workflow): verification hand-offs`

---

### Task 5: 埠

**Files:** Create `src-tauri/src/tasks/verify_ports.rs`；Modify `tasks/mod.rs`

- [ ] **Step 1: 失敗測試**

```rust
#[test]
fn free_ports_are_distinct_and_bindable() {
    let ports = pick_free_ports(3);
    assert_eq!(ports.len(), 3);
    assert_eq!(ports.iter().collect::<std::collections::HashSet<_>>().len(), 3);
    for p in &ports { std::net::TcpListener::bind(("127.0.0.1", *p)).unwrap(); }
}

#[cfg(unix)]
#[test]
fn leftover_servers_on_our_ports_are_stopped() {
    let port = pick_free_ports(1)[0];
    let mut child = std::process::Command::new("python3")
        .args(["-c", &format!("import socket,time;s=socket.socket();s.bind(('127.0.0.1',{port}));s.listen();time.sleep(60)")])
        .spawn().unwrap();
    // 等它開始監聽
    for _ in 0..50 { if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() { break; } std::thread::sleep(std::time::Duration::from_millis(100)); }
    let killed = stop_listeners(&[port]);
    assert_eq!(killed, vec![child.id()]);
    let status = child.wait().unwrap();
    assert!(!status.success());
}

#[test]
fn never_kills_ourselves() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    assert!(stop_listeners(&[port]).is_empty());
}
```

- [ ] **Step 2:** FAIL。
- [ ] **Step 3: 實作**
  - `pick_free_ports(n)`：同時綁 n 個 `127.0.0.1:0`、讀埠、全部放掉。
  - `pids_listening(port) -> Vec<u32>`：unix 跑 `lsof -nP -iTCP:<port> -sTCP:LISTEN -t`；windows 跑 `netstat -ano -p TCP`，取 `LISTENING` 而且本地位址結尾是 `:<port>` 的最後一欄 PID。
  - `stop_listeners(ports) -> Vec<u32>`：去掉自己的 pid（`std::process::id()`），unix `kill -TERM`，等 1 秒還在就 `kill -KILL`；windows `taskkill /PID <pid> /T /F`。回傳結束的 pid。
  - `save_ports(task_dir, &ports)` / `load_ports(task_dir)`：`ports.json`。
- [ ] **Step 4:** 通過；突變（不排除自己）→ 第三條紅。
- [ ] **Step 5:** commit `feat(tasks): verifier ports and leftover server cleanup`

---

### Task 6: 派工與結案

**Files:** Modify `src-tauri/src/tasks/ask.rs`、`src-tauri/src/tasks/scheduler.rs`

- [ ] **Step 1: 失敗測試**
  - `ask.rs`：`mcp_config_json_with(url: Option<&str>, playwright_out: Option<&Path>)` 產生的 JSON：兩個都有時 `mcpServers` 有 `aiterm` 與 `playwright`；playwright 的 args 等於 `["-y","@playwright/mcp@0.0.83","--headless","--isolated","--output-dir",<path>]`；`allowed_tools(ask, browser)` 回 `"mcp__aiterm__ask_human mcp__playwright"` 等組合。
  - `scheduler.rs`：`finalize_verify_card`（仿 `finalize_review_card` 的測試）：有效 verify.json → Success、verdict 寫入、`review_reasons` 存失敗項的條件清單；沒有檔案 → Failed、驗收者暫停；worktree 被移除；`ports.json` 裡的埠上有監聽的行程會被結束（unix）。
- [ ] **Step 2:** FAIL。
- [ ] **Step 3: 實作**
  - `PLAYWRIGHT_MCP: &str = "@playwright/mcp@0.0.83"`。
  - `RealDispatcher::dispatch`：`readonly` 加上 `KIND_VERIFY`（用 `prepare_review_worktree`，訊息裡的「審查」不影響）。驗收卡：`pick_free_ports(3)`、`save_ports`、建 `evidence` 資料夾、提示後面附「可用的埠：a、b、c」；`mcp.json` 改用 `mcp_config_json_with(ask_url, Some(evidence))`，`--allowedTools` 用 `allowed_tools(can_ask, true)`。非驗收卡行為不變（`mcp_config_json_with(Some(url), None)` 必須等於原本的 `mcp_config_json(url)`，測試鎖住）。
  - watch closure：`KIND_VERIFY` → `finalize_verify_card`。
  - `finalize_verify_card`：讀 `verify.json`→`parse_verify_json`→`set_verdict`（reasons = fail 項的 criterion），失敗就 Failed 並暫停驗收者；`stop_listeners(load_ports)`；移除 worktree。
  - 事件：`WorkflowEvent::VerifyAttention` → emit `agent-verify-attention { project_id, project_name, verify_id, unowned }`。
  - `commit_message_for` 不需要（驗收卡不 commit）。
- [ ] **Step 4:** 通過；突變：驗收卡不加 playwright → 測紅；finalize 不清埠 → 測紅。
- [ ] **Step 5:** commit `feat(scheduler): dispatch and finish verify cards`

---

### Task 7: 指令

**Files:** Modify `src-tauri/src/commands/agents.rs`、`src-tauri/src/lib.rs`

- `agents_verify_report(project_id, task_id) -> VerifyReportView { report: Option<VerifyReport>, images: HashMap<String, String> }`：讀 verify.json；每個 item 的 `evidence` 若是 `.png/.jpg/.jpeg` 而且解析後在 `tasks/<id>/` 底下、小於 5MB → base64 data URL（防止 `../` 跳出卡片資料夾，測試鎖住）。
- `agents_accept_verification(project_id, task_id)` → `workflow::accept_verification`，之後 emit `tasks-updated`。
- [ ] 測試：純函式 `evidence_images(task_dir, report)`：正常圖片收進來、`../../etc/x.png` 不收、超過大小不收、非圖片不收。
- [ ] commit `feat(commands): verification report and accept`

---

### Task 8: 前端

**Files:** `src/ipc/agents.ts`、`roles.ts`、`deriveAgentStatus.ts`、`edges.ts`、`projectSummary.ts`、`AgentNode.tsx`、`index.tsx`、新增 `VerifyReportDialog.tsx`、`i18n.ts`；測試同名 `.test.ts(x)`

- [ ] **AgentKind** 加 `"verifier"`；`VerifyReport`、`VerifyReportView` 型別；`verifyReport()`、`acceptVerification()`；`onVerifyAttention` 事件。
- [ ] **roles**：`verifier` → 一個內建角色「驗收者」，prefix 說明不改檔、照條件逐條驗。
- [ ] **deriveAgentStatus**：verifier 執行中 → 新狀態 `verifying`（CSS 用紫色）。測試。
- [ ] **edges**：有 orchestrator 與 verifier → `orch → verifier`；verifier 執行中時那條線亮。合併卡完成的光點 orch→verifier；驗收修正卡出現的光點 verifier→coder（反向橘色）。測試。
- [ ] **projectSummary**：有 verifier 時，最後成功的合併卡之後：
  - 沒有該合併的驗收卡、或驗收卡 queued/running → `{kind:"verifying"}`
  - 驗收卡 verdict `fail` → `{kind:"verify_failed", items}`（失敗項數由 `review_reasons` 長度）
  - `attention` → `{kind:"verify_attention"}`
  - `pass` → 照原本看合併卡 verdict（in_repo…）。
  測試涵蓋四種，並確認沒有 verifier 時結果不變。
- [ ] **AgentNode**：verifier 的執行中標題「驗收中」、閒置時顯示最後一張驗收卡結果（✓ 通過（N 項需你確認）／✗ 退回 N 項／⚠ 需要你決定＋「放行」按鈕）；點結果開驗收報告。
- [ ] **index.tsx**：Agent 池加驗收者（單例）；頂列摘要新三種的文字；`VerifyReportDialog` 掛載；`agent-verify-attention` 桌面通知（同 `agent-module-attention` 的作法）。
- [ ] **VerifyReportDialog**：每項狀態圖示、條件、重現步驟、預期／實際、圖片縮圖（點開放大）、`manual` 項目的原因；有 `attention` 時顯示「放行」。測試：渲染三種狀態、圖片、放行呼叫 IPC。
- [ ] **i18n**：zh-TW 與 en 都加（`localeSources` 測試會檢查兩邊一致）。
- [ ] commit `feat(agent-canvas): verifier role, report and summary`

---

### Task 9: 全部驗證與實機

- [ ] Rust 全跑、前端全跑、tsc、eslint。
- [ ] 假 claude 實機（不花額度）：寫假 claude 腳本，依卡片種類行為：work/fix 卡改一個檔案；review 卡寫 pass；merge 卡做 git merge；verify 卡第一次寫 fail（指向模組 A）、第二次寫 pass。確認畫面：驗收中 → 退回 → 修正 → 再審 → 再合併 → 驗收通過 → 程式庫更新。
- [ ] 真 claude 實機：小網頁專案（兩個模組、一個故意的串接 bug），確認：開瀏覽器點擊、截圖出現在報告、退回正確的工程師、伺服器有關（驗收後 `lsof` 那三個埠沒有監聽）。
- [ ] spec §6.3 第 2、3 項：沒有 Chrome／沒有 npx 時的行為，至少在 macOS 用乾淨 HOME 試一次，結果寫回 spec。
- [ ] 更新 memory、回報、問是否合併。
