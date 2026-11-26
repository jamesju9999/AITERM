# Server 驗收者 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 共用看板專案開啟「由 server 執行驗收」後，全部模組審查通過時，server 先在拋棄式整合 worktree 用真實 Claude Code＋無頭瀏覽器驗收；通過才出現「合併待核准」，退回就自動開修正卡給對應工程師（最多 2 輪），管理員核准仍是唯一的合併關卡。

**Architecture:** 純邏輯（狀態機、`verify.json`、提示詞、簽章、閘門）放 `aiterm-intake-core`，可單元測試；執行面（整合 worktree、驗收模式的 `claude -p`、埠清理、禁止清單、API）放 `aiterm-intake-server`；畫面在 AITerm（`board_remote` ＋ Tauri 指令 ＋ React）。驗收卡是新的 `card_kind = "verify"`，修正卡是 `"vfix"`，不動既有審查鏈。

**Tech Stack:** Rust（sqlx／axum／tokio／sysinfo）、`claude -p`（`--mcp-config`、`--disallowedTools`、`--session-id`／`--resume`）、Playwright MCP 0.0.83、React／Vitest。

**Spec:** `docs/superpowers/specs/2026-11-26-server-verifier-design.md`

**慣例（本 repo）：**
- 測試指令：`cd src-tauri && cargo test -p <crate> <name>`；收尾一定跑 `cargo test --workspace --no-fail-fast`（逐行看 `test result:`）與 `npx vitest run`、`npx tsc -b`。
- `docs/` 被 gitignore，新增檔要 `git add -f`；已追蹤檔用 `git add -u`。commit 前 `git status`，只加明確路徑。
- commit 訊息結尾要有 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- 假 claude 測試是 shell 腳本，檔案頂端要 `#![cfg(unix)]`。
- 不打 tag、不發版；只有使用者明說才 push。

---

## 檔案結構

**新增**
- `src-tauri/crates/aiterm-intake-core/src/verifier.rs` — `VerifyReport`／`VerifyItem`、`parse_verify_json`、`signature`、`gate`、`verify_prompt`、`vfix_body`、`attribute`（失敗項分組到模組）。純函式。
- `src-tauri/crates/aiterm-intake-server/src/verifier.rs` — 整合 worktree、`run_verify`、結束後清理、`finish_verify`（解析、開 vfix、存報告）。
- `src-tauri/crates/aiterm-intake-server/src/verify_env.rs` — 空閒埠、結束佔埠行程、Chrome 偵測、`mcp.json`（由桌面版 `tasks/verify_ports.rs`、`tasks/ask.rs` 複製調整，不改桌面版）。
- `src-tauri/crates/aiterm-intake-server/src/deny.rs` — 禁止清單：預設、全域＋專案覆寫的合併、格式檢查。
- `src-tauri/crates/aiterm-intake-server/src/routes_verify.rs` — 驗收相關 API。
- `src-tauri/crates/aiterm-intake-server/tests/verify.rs` — 假 claude 的整條流程測試。

**修改**
- `aiterm-intake-core/src/orchestration.rs` — `module_state` 認得 `vfix`；`merge_status` 不變。
- `aiterm-intake-core/src/board_store.rs` — 新表 `board_verify`、`board_verify_deny`；存取函式。
- `aiterm-intake-server/src/orchestrator.rs` — `advance`（建 verify 卡、vfix 卡、合併卡閘門）、`tick_once`（可派的卡種類）、`run_inner`（分派到 `verifier::run_verify`）。
- `aiterm-intake-server/src/runner.rs` — `Mode::Verify`、`claude_args_with` 帶禁止清單與 MCP、擷取 `permission_denials`。
- `aiterm-intake-server/src/server.rs` — 啟動時補清驗收留下的佔埠行程。
- `aiterm-intake-server/src/routes.rs`／`lib.rs` — 掛路由與模組。
- `src-tauri/src/intake/board_remote.rs`、`src-tauri/src/intake/commands.rs`、`src-tauri/src/lib.rs` — 桌面 App 端的驗收指令。
- `src/ipc/*.ts`、`src/components/AgentCanvas/*` — 畫面。
- `CHANGELOG.md`、`docs/intake-server.md` — 文件（只寫驗證過的）。

---

# 里程碑 1：後端核心（假 claude 驗證）

### Task 1: 模組狀態認得 `vfix`（先寫測試）

**Files:**
- Modify: `src-tauri/crates/aiterm-intake-core/src/orchestration.rs`（`module_state`，約 38–76 行；測試放同檔 `mod tests`）

- [ ] **Step 1: 寫失敗的測試**

在 `orchestration.rs` 的 `mod tests` 末尾加（先看同檔其他測試用的 `card`／`coder_agent` 輔助函式，沿用；以下以 `mk(kind, id, status, outcome)` 表示建一張卡，實作時改用現成的輔助函式）：

```rust
    /// 模組通過審查後，驗收退回開了 vfix：模組不能還是 Passed，修好後要重新審查。
    #[test]
    fn a_verify_fix_after_a_passed_review_puts_the_module_back_in_progress_and_does_not_use_up_review_rounds() {
        let module = mk("work", "m1", "done", Some("success"));
        let mut review = mk("review", "r1", "done", Some("success"));
        review.verdict = Some("pass".into());
        review.review_of = Some("m1".into());
        // 驗收退回：vfix 接在通過的審查後面（review_of = r1），還沒做完。
        let mut vfix = mk("vfix", "v1", "queued", None);
        vfix.review_of = Some("r1".into());
        assert_eq!(module_state(&module, &[review.clone(), vfix.clone()], true), ModuleState::Waiting, "vfix 還沒做完＝修正中，不是 Passed");
        // vfix 做完：要建審查卡（目標是 vfix 本身）。
        vfix.status = "done".into();
        vfix.outcome = Some("success".into());
        assert_eq!(module_state(&module, &[review.clone(), vfix.clone()], true), ModuleState::NeedsReview { target: "v1".into() });
        // 對 vfix 的審查通過：回到 Passed，sha 是 vfix 的版本。
        vfix.end_sha = Some("sha-v".into());
        let mut review2 = mk("review", "r2", "done", Some("success"));
        review2.verdict = Some("pass".into());
        review2.review_of = Some("v1".into());
        assert_eq!(module_state(&module, &[review.clone(), vfix.clone(), review2.clone()], true), ModuleState::Passed { sha: Some("sha-v".into()) });
        // vfix 不計入審查的退回輪數：先有兩次審查 fail 的 fix（用完次數前），vfix 不該讓它提早 Exhausted。
        let mut fail1 = mk("review", "f1", "done", Some("success"));
        fail1.verdict = Some("fail".into());
        fail1.review_of = Some("m1".into());
        let mut fix1 = mk("fix", "x1", "done", Some("success"));
        fix1.review_of = Some("f1".into());
        let mut pass = mk("review", "p1", "done", Some("success"));
        pass.verdict = Some("pass".into());
        pass.review_of = Some("x1".into());
        let mut vfix2 = mk("vfix", "v2", "done", Some("success"));
        vfix2.review_of = Some("p1".into());
        let mut fail2 = mk("review", "f2", "done", Some("success"));
        fail2.verdict = Some("fail".into());
        fail2.review_of = Some("v2".into());
        // 1 次審查退回（fix）＋1 次 vfix，再被審查退回：審查退回輪數只算 fix 的 1 次，還有一次修正機會。
        assert_eq!(
            module_state(&module, &[fail1, fix1, pass, vfix2, fail2], true),
            ModuleState::NeedsFix { review: "f2".into(), round: 2 },
            "vfix 不算審查退回輪數"
        );
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd src-tauri && cargo test -p aiterm-intake-core a_verify_fix_after_a_passed_review 2>&1 | grep -E "test result|panicked"`
Expected: FAIL（第一個 `assert_eq!` 會得到 `Passed`，因為現在 `next_of` 雖找得到 vfix，但 `tail.card_kind` 不是 `"review"`／`"fix"` 的處理路徑把它當一般卡）

- [ ] **Step 3: 最小實作**

`module_state` 的迴圈改成（`fixes` 只算 `fix`，`vfix` 照樣往下走）：

```rust
    for _ in 0..chain.len() + 1 {
        match next_of(tail, chain) {
            Some(n) => {
                if n.card_kind == "fix" {
                    fixes += 1;
                }
                tail = n;
            }
            None => break,
        }
    }
```

此段程式已經是這樣；真正要改的是最後的 `match tail.card_kind.as_str()`：`vfix` 與 `fix` 一樣屬於「做完要審查」的卡，已由 `_ if !has_reviewer / _ => NeedsReview` 涵蓋。因此若 Step 2 的失敗點是 `chain` 過濾，改 `orchestration.rs` 以外的呼叫端：所有 `matches!(c.card_kind.as_str(), "review" | "fix" | "plan")` 要加 `"vfix"`（見 Task 8 的清單）。先確認這個測試單獨在 core 內能通過，不通過才改 `module_state`。

- [ ] **Step 4: 跑測試確認通過**

Run: 同 Step 2。Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/aiterm-intake-core/src/orchestration.rs
git commit -m "test(board): 驗收退回的修正卡（vfix）不計入審查退回輪數，模組回到修正中"
```

### Task 2: core 純邏輯 `verifier.rs`

**Files:**
- Create: `src-tauri/crates/aiterm-intake-core/src/verifier.rs`
- Modify: `src-tauri/crates/aiterm-intake-core/src/lib.rs`（加 `pub mod verifier;`）
- Test: 同檔 `mod tests`

- [ ] **Step 1: 寫型別、解析與簽章的失敗測試**

`verifier.rs` 開頭先放型別與函式簽名（`todo!()` 不可留；先寫測試，再寫實作）。測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_failed_item_makes_the_verdict_fail_whatever_the_file_says() {
        let r = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"pass","evidence":["x.png"]},{"criterion":"b","status":"fail","steps":"s","expected":"e","actual":"a"}]}"#).unwrap();
        assert_eq!(r.verdict, "fail");
    }

    #[test]
    fn bad_shapes_are_refused_with_a_reason() {
        assert!(parse_verify_json("not json").is_err());
        assert!(parse_verify_json(r#"{"verdict":"maybe","items":[]}"#).unwrap_err().contains("pass 或 fail"));
        assert!(parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"ok"}]}"#).unwrap_err().contains("pass、fail 或 manual"));
        assert!(parse_verify_json("\u{feff}{\"verdict\":\"pass\",\"items\":[]}").is_ok(), "容忍 BOM");
    }

    #[test]
    fn a_manual_item_needs_a_reason_and_a_pass_needs_evidence() {
        let no_reason = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"a","status":"manual"}]}"#).unwrap();
        assert_eq!(check_quality(&no_reason).unwrap_err(), vec!["「a」標成 manual 卻沒有寫原因".to_string()]);
        let no_evidence = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"b","status":"pass"}]}"#).unwrap();
        assert_eq!(check_quality(&no_evidence).unwrap_err(), vec!["「b」通過但沒有附證據".to_string()]);
        let ok = parse_verify_json(r#"{"verdict":"pass","items":[{"criterion":"c","status":"pass","evidence":["npm test: 12 passed"]},{"criterion":"d","status":"manual","notes":"原生畫面"}]}"#).unwrap();
        assert!(check_quality(&ok).is_ok());
        assert_eq!(manual_count(&ok), 1);
    }

    #[test]
    fn the_signature_follows_the_passed_versions_and_ignores_order() {
        assert_eq!(signature(&[("b".into(), Some("2".into())), ("a".into(), Some("1".into()))], 0), signature(&[("a".into(), Some("1".into())), ("b".into(), Some("2".into()))], 0));
        assert_ne!(signature(&[("a".into(), Some("1".into()))], 0), signature(&[("a".into(), Some("9".into()))], 0), "版本變了簽章就變");
        assert_ne!(signature(&[("a".into(), Some("1".into()))], 0), signature(&[("a".into(), Some("1".into()))], 1), "重新驗收的序號也算");
    }

    #[test]
    fn failed_items_are_grouped_by_module_and_unknown_ids_are_unattributed() {
        let r = parse_verify_json(r#"{"verdict":"fail","items":[
            {"criterion":"a","status":"fail","module_id":"m1","steps":"s","expected":"e","actual":"x"},
            {"criterion":"b","status":"fail","module_id":"m1"},
            {"criterion":"c","status":"fail","module_id":"ghost"},
            {"criterion":"d","status":"fail"},
            {"criterion":"e","status":"pass","module_id":"m2","evidence":["o"]}]}"#).unwrap();
        let a = attribute(&r, &["m1".to_string(), "m2".to_string()]);
        assert_eq!(a.by_module.len(), 1);
        assert_eq!(a.by_module[0].0, "m1");
        assert_eq!(a.by_module[0].1.len(), 2);
        assert_eq!(a.unattributed.len(), 2, "ghost 與沒寫 module_id 的都找不到模組");
    }

    #[test]
    fn the_gate_decides_what_to_create_next() {
        // 沒有 verify 卡：要建一張。
        assert_eq!(gate(&[], "sig1"), Gate::NeedsVerify);
        // 有同簽章、還沒做完：等。
        assert_eq!(gate(&[vc("v1", "sig1", "queued", None, None)], "sig1"), Gate::Waiting);
        // 同簽章通過：可以建合併卡。
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("success"), Some("pass"))], "sig1"), Gate::Passed);
        // 簽章變了（修正卡做完）：舊的通過不算，要重新驗。
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("success"), Some("pass"))], "sig2"), Gate::NeedsVerify);
        // 同簽章失敗：輪數沒滿 → 等 vfix 做完（簽章會變）；這裡回 Failed 讓呼叫端開 vfix。
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("success"), Some("fail"))], "sig1"), Gate::Failed { rounds: 1 });
        // 同簽章 attention：停住。
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("success"), Some("attention"))], "sig1"), Gate::NeedsHuman);
        // 驗收卡自己失敗（outcome=failed，如合併衝突、verify.json 壞）：停住等管理員。
        assert_eq!(gate(&[vc("v1", "sig1", "done", Some("failed"), None)], "sig1"), Gate::NeedsHuman);
        // 連續兩次 fail 之後（輪數用完）：停住。
        assert_eq!(
            gate(&[vc("v1", "s1", "done", Some("success"), Some("fail")), vc("v2", "s2", "done", Some("success"), Some("fail"))], "s2"),
            Gate::NeedsHuman
        );
    }

    fn vc(id: &str, sig: &str, status: &str, outcome: Option<&str>, verdict: Option<&str>) -> VerifyCardView {
        VerifyCardView { id: id.into(), signature: sig.into(), status: status.into(), outcome: outcome.map(Into::into), verdict: verdict.map(Into::into), created_at: 0 }
    }
}
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd src-tauri && cargo test -p aiterm-intake-core verifier 2>&1 | grep -E "^error|test result" | head`
Expected: 編譯失敗（型別與函式未定義）

- [ ] **Step 3: 實作**

`verifier.rs`（`VerifyItem`／`VerifyReport` 與 `parse_verify_json` 取自桌面版 `tasks/workflow.rs:931–976`，加 `check_quality`、`manual_count`、`signature`、`attribute`、`gate`）：

```rust
//! 驗收者的純邏輯（桌面版與 server 的格式一致）：verify.json、簽章、失敗項分組、合併卡閘門。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct VerifyItem {
    pub criterion: String,
    pub status: String,
    #[serde(default)]
    pub module_id: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub steps: Option<String>,
    #[serde(default)]
    pub expected: Option<String>,
    #[serde(default)]
    pub actual: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct VerifyReport {
    pub verdict: String,
    #[serde(default)]
    pub project_type: Option<String>,
    #[serde(default)]
    pub items: Vec<VerifyItem>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// 解析 verify.json。有任何 `fail` 項就是 `fail`，不管它自己寫什麼。
pub fn parse_verify_json(text: &str) -> Result<VerifyReport, String> {
    let mut v: VerifyReport = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    if !matches!(v.verdict.as_str(), "pass" | "fail") {
        return Err(format!("verdict 必須是 pass 或 fail，收到 {}", v.verdict));
    }
    if let Some(bad) = v.items.iter().find(|i| !matches!(i.status.as_str(), "pass" | "fail" | "manual")) {
        return Err(format!("status 必須是 pass、fail 或 manual，收到 {}", bad.status));
    }
    if v.items.iter().any(|i| i.status == "fail") {
        v.verdict = "fail".into();
    }
    Ok(v)
}

/// 品質檢查：`manual` 要寫原因（`notes`）；`pass` 要附證據。不合格回清單（驗收卡當成「需處理」）。
pub fn check_quality(r: &VerifyReport) -> Result<(), Vec<String>> {
    let mut bad = Vec::new();
    for i in &r.items {
        if i.status == "manual" && i.notes.as_deref().map_or(true, |n| n.trim().is_empty()) {
            bad.push(format!("「{}」標成 manual 卻沒有寫原因", i.criterion));
        }
        if i.status == "pass" && i.evidence.iter().all(|e| e.trim().is_empty()) {
            bad.push(format!("「{}」通過但沒有附證據", i.criterion));
        }
    }
    if bad.is_empty() { Ok(()) } else { Err(bad) }
}

pub fn manual_count(r: &VerifyReport) -> usize {
    r.items.iter().filter(|i| i.status == "manual").count()
}

/// 驗收簽章：各模組通過的版本（`(模組 id, sha)`，順序不拘）加重試序號。簽章不同＝受驗內容變了，要重新驗。
pub fn signature(passed: &[(String, Option<String>)], retry: u32) -> String {
    let mut parts: Vec<String> = passed.iter().map(|(m, s)| format!("{m}={}", s.as_deref().unwrap_or(""))).collect();
    parts.sort();
    let mut h = Sha256::new();
    h.update(parts.join("\n"));
    h.update(format!("\nretry={retry}"));
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// 失敗項分組的結果。
#[derive(Debug, PartialEq)]
pub struct Attribution<'a> {
    /// 有對應模組的失敗項，依模組分組（模組順序依首次出現）。
    pub by_module: Vec<(String, Vec<&'a VerifyItem>)>,
    /// `module_id` 沒寫、或不是任何已知模組的失敗項。
    pub unattributed: Vec<&'a VerifyItem>,
}

pub fn attribute<'a>(r: &'a VerifyReport, module_ids: &[String]) -> Attribution<'a> {
    let mut by_module: Vec<(String, Vec<&VerifyItem>)> = Vec::new();
    let mut unattributed = Vec::new();
    for i in r.items.iter().filter(|i| i.status == "fail") {
        match i.module_id.as_deref().filter(|m| module_ids.iter().any(|k| k == m)) {
            Some(m) => match by_module.iter_mut().find(|(k, _)| k == m) {
                Some((_, v)) => v.push(i),
                None => by_module.push((m.to_string(), vec![i])),
            },
            None => unattributed.push(i),
        }
    }
    Attribution { by_module, unattributed }
}

/// 閘門判讀用的驗收卡摘要（server 從 `board_cards` ＋ `board_verify` 組出來，按建立時間由舊到新）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyCardView {
    pub id: String,
    pub signature: String,
    pub status: String,
    pub outcome: Option<String>,
    pub verdict: Option<String>,
    pub created_at: i64,
}

/// 驗收不通過最多自動退回幾輪（與審查相同）。
pub use crate::orchestration::MAX_FIX_ROUNDS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// 目前這組版本還沒有驗收卡：要建一張。
    NeedsVerify,
    /// 驗收卡還在跑或排隊。
    Waiting,
    /// 同簽章通過：可以建合併卡。
    Passed,
    /// 同簽章不通過、輪數沒滿：呼叫端要依失敗項開 vfix 卡。
    Failed { rounds: usize },
    /// 停住等管理員：attention、驗收卡自己失敗、或輪數用完。
    NeedsHuman,
}

/// 依目前簽章與驗收卡歷史（舊→新）判斷下一步。
/// 輪數 = 從最近一次 `pass` 之後，連續 `fail`／`attention` 的驗收卡張數。
pub fn gate(cards: &[VerifyCardView], current: &str) -> Gate {
    let done = |c: &&VerifyCardView| c.status == "done";
    let tail_rounds = cards
        .iter()
        .rev()
        .take_while(|c| !(c.status == "done" && c.verdict.as_deref() == Some("pass")))
        .filter(done)
        .filter(|c| matches!(c.verdict.as_deref(), Some("fail") | Some("attention")))
        .count();
    let Some(last) = cards.iter().rev().find(|c| c.signature == current) else { return Gate::NeedsVerify };
    if last.status != "done" {
        return Gate::Waiting;
    }
    if last.outcome.as_deref() != Some("success") {
        return Gate::NeedsHuman;
    }
    match last.verdict.as_deref() {
        Some("pass") => Gate::Passed,
        Some("fail") if tail_rounds >= MAX_FIX_ROUNDS => Gate::NeedsHuman,
        Some("fail") => Gate::Failed { rounds: tail_rounds },
        _ => Gate::NeedsHuman,
    }
}
```

（`sha2` 已是 core 的依賴；`Cargo.toml` 若沒有，加 `sha2 = "0.10"`，版本與同 crate 其他地方一致。）

- [ ] **Step 4: 跑測試確認通過**

Run: `cd src-tauri && cargo test -p aiterm-intake-core verifier 2>&1 | grep -E "^error|test result|panicked"`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/aiterm-intake-core/src/verifier.rs src-tauri/crates/aiterm-intake-core/src/lib.rs src-tauri/crates/aiterm-intake-core/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(board): 驗收者的純邏輯（verify.json 解析與品質檢查、簽章、失敗項分組、合併卡閘門）"
```

### Task 3: 驗收提示詞與 vfix 內文（core）

**Files:**
- Modify: `src-tauri/crates/aiterm-intake-core/src/verifier.rs`
- Test: 同檔

- [ ] **Step 1: 失敗的測試**

```rust
    #[test]
    fn the_verify_prompt_lists_modules_ports_paths_and_never_offers_ask_human() {
        let mods = vec![("m1".to_string(), "計時器".to_string(), "按開始倒數 25 分鐘".to_string())];
        let p = verify_prompt(&mods, "規格：倒數 25 分鐘", "/v/verify.json", "/v/evidence", &[4001, 4002, 4003], true);
        for needle in ["[m1] 計時器", "按開始倒數 25 分鐘", "規格：倒數 25 分鐘", "/v/verify.json", "/v/evidence", "4001", "4002", "4003", "mcp__playwright", "manual", "module_id", "install-browser chrome-for-testing"] {
            assert!(p.contains(needle), "缺：{needle}\n{p}");
        }
        assert!(!p.contains("ask_human"), "server 沒有 ask_human");
        assert!(p.contains("寫成 JSON 檔"), "用 Bash 寫 verify.json");
        let no_browser = verify_prompt(&mods, "", "/v/verify.json", "/v/evidence", &[4001, 4002, 4003], false);
        assert!(no_browser.contains("瀏覽器工具不可用"), "沒有 Node 時要求標 manual");
        assert!(no_browser.contains("沒有規格"));
    }

    #[test]
    fn the_verify_fix_body_carries_the_reproduction_and_the_round() {
        let items = vec![VerifyItem { criterion: "按開始倒數".into(), status: "fail".into(), module_id: Some("m1".into()), evidence: vec!["evidence/a.png".into()], steps: Some("按開始".into()), expected: Some("25:00".into()), actual: Some("24:58".into()), notes: None }];
        let refs: Vec<&VerifyItem> = items.iter().collect();
        let b = vfix_body("計時器", "按開始倒數 25 分鐘", &refs, "v-card-1", 1);
        for needle in ["計時器", "第 1 輪", "按開始倒數", "按開始", "25:00", "24:58", "evidence/a.png", "按開始倒數 25 分鐘"] {
            assert!(b.contains(needle), "缺：{needle}\n{b}");
        }
    }
```

- [ ] **Step 2: 確認失敗** — `cargo test -p aiterm-intake-core verify_prompt` → 編譯失敗。

- [ ] **Step 3: 實作**

`verify_prompt`（取自桌面版 `workflow::verify_body`，server 差異：模組以 `(id, 標題, 需求)` 傳入；埠清單；`has_browser` 控制瀏覽器段落；沒有 `ask_human`；`verify.json` 用 Bash 寫）與 `vfix_body`（取自桌面版 `verify_fix_body`）：

```rust
pub const BROWSER_INSTALL: &str = "npx -y @playwright/mcp@0.0.83 install-browser chrome-for-testing";

fn take_chars(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push_str("…（略）");
    }
    out
}

/// 驗收卡的提示。`modules` 是 `(id, 標題, 需求)`；`has_browser` 為假（沒有 Node）時要求相關條件標 manual。
pub fn verify_prompt(modules: &[(String, String, String)], spec_text: &str, json_path: &str, evidence: &str, ports: &[u16], has_browser: bool) -> String {
    let mut s = String::from(
        "你是驗收者。目前目錄是各工程師的成果合併之後的版本（拋棄式的工作資料夾）。請確認它真的能用：實際建置、執行、操作，照驗收條件逐條確認。\n\
         - 不要修改、commit 或 push 任何原始碼；安裝相依套件、建置產生的檔案沒關係。\n\
         - 你只做判定，不修 bug。你沒有編輯檔案的工具，用 Bash 把結果寫成檔案。\n\n\
         模組清單（module_id 要把方括號裡的 id 原樣整串貼上，不要縮短、不要自己編；不屬於任何一個模組的整合類條目就省略 module_id）：\n",
    );
    for (id, title, body) in modules {
        s.push_str(&format!("- [{id}] {title}\n  需求：{}\n", take_chars(body, 2000).replace('\n', "\n  ")));
    }
    if spec_text.trim().is_empty() {
        s.push_str("\n沒有規格：請依各模組需求自己列出驗收條件，每條都要驗。\n");
    } else {
        s.push_str(&format!("\n規格全文（照裡面的驗收條件逐條驗）：\n{spec_text}\n"));
    }
    s.push_str(
        "\n先判斷專案類型，照下面的方式驗：\n\
         - 網頁前端：啟動開發伺服器，用瀏覽器工具（mcp__playwright__ 開頭的工具）照驗收條件實際點擊、輸入、換頁；用 browser_console_messages 檢查主控台錯誤；每條條件用 browser_take_screenshot 截圖。\n\
         - 後端 API：啟動服務，實際呼叫端點，檢查狀態碼與回應內容。\n\
         - 全端：前後端都啟動，從瀏覽器操作，確認資料存進後端、重新整理後讀得回來。\n\
         - 命令列工具：實際執行各種參數，檢查輸出與結束碼。\n\
         - 函式庫：跑測試，再寫一小段程式實際呼叫公開介面（寫在目前目錄以外的暫存位置）。\n\
         - Electron：用瀏覽器工具操作。\n\
         - 其他桌面或手機 App（原生、Tauri、iOS、Android）：只做建置、測試、啟動不當掉；需要操作畫面的條件標成 manual。\n\
         \n專案自己有測試就一定要跑。\n\
         \n不管規格有沒有寫，都要先驗證專案的主要入口能用：網頁就打開根網址，確認畫面有內容、沒有跳錯誤頁、主控台沒有錯誤；後端就呼叫一個最基本的端點；命令列工具就執行一次最基本的指令。這一項用 criterion「主要入口可用」寫進 items（省略 module_id），入口打不開就是 fail。\n",
    );
    s.push_str(&format!("\n截圖一律存成這個資料夾底下的絕對路徑：{evidence}（例如 {evidence}/start.png）。存到別的地方會被拒絕。\n"));
    s.push_str(&format!(
        "伺服器只能用這些埠：{}。驗收結束前把你啟動的伺服器與瀏覽器都關掉（browser_close）。\n",
        ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("、")
    ));
    if has_browser {
        s.push_str(&format!("瀏覽器工具回報瀏覽器沒有安裝（not installed／not found）時，先執行 `{BROWSER_INSTALL}` 再重試一次（第一次要下載約 100MB）。\n瀏覽器工具或其他工具還是不能用時，把需要它的條件標成 manual 並寫原因，不能當成通過。\n"));
    } else {
        s.push_str("瀏覽器工具不可用（這台電腦沒有 Node.js）：需要瀏覽器的條件一律標成 manual 並寫原因，不能當成通過。\n");
    }
    s.push_str(&format!(
        "\n你不能問人，需要人確認的條件一律標成 manual 並寫原因。\n\
         \n完成後用 Bash 把結果寫成 JSON 檔到：{json_path}\n\
         格式：\n\
         {{\"verdict\": \"pass\" 或 \"fail\", \"project_type\": \"web\", \"items\": [\
         {{\"criterion\": \"驗收條件\", \"status\": \"pass\" 或 \"fail\" 或 \"manual\", \
         \"module_id\": \"這條屬於哪個模組的 id（跨模組的全域條件才省略）\", \
         \"evidence\": [\"evidence/start.png\", \"指令輸出摘要\"], \
         \"steps\": \"重現步驟（fail 必填）\", \"expected\": \"預期\", \"actual\": \"實際\", \
         \"notes\": \"manual 的原因或其他說明\"}}], \"notes\": \"其他觀察\"}}\n\
         任何一條 fail，verdict 就是 fail。manual 不算失敗，但一定要寫原因。evidence 裡的檔案路徑寫成相對於 JSON 檔所在資料夾的路徑。\n\
         \n上面清單裡的每個模組都至少要有一條 items（通過的也要寫，用 module_id 標明是哪個模組）；實在驗不了的模組也要寫一條 status 為 manual 的並說明原因。\n\
         \n每一條 status 為 pass 的都必須附證據（evidence）：截圖路徑，或實際跑過的指令與輸出摘要。沒有證據的 pass 視為沒驗；只讀程式碼不算驗過。\n"
    ));
    s
}

/// 驗收退回的修正卡內文。`items` 是屬於這個模組的失敗項。
pub fn vfix_body(module_title: &str, module_body: &str, items: &[&VerifyItem], verify_id: &str, round: usize) -> String {
    let mut s = format!("驗收沒有通過，請修正模組「{module_title}」（驗收退回第 {round} 輪，最多 {MAX_FIX_ROUNDS} 輪；驗收卡 {verify_id}）。\n驗收發現的問題：\n");
    for i in items {
        s.push_str(&format!("- {}\n", i.criterion));
        if let Some(v) = &i.steps { s.push_str(&format!("  重現步驟：{v}\n")); }
        if let Some(v) = &i.expected { s.push_str(&format!("  預期：{v}\n")); }
        if let Some(v) = &i.actual { s.push_str(&format!("  實際：{v}\n")); }
        if !i.evidence.is_empty() { s.push_str(&format!("  證據：{}\n", i.evidence.join("、"))); }
    }
    s.push_str(&format!("\n這個模組原本的需求（供對照）：\n{}\n\n修好後照一般流程交付，會再審查、再驗收。\n", take_chars(module_body, 2000)));
    s
}
```

- [ ] **Step 4: 確認通過** — `cargo test -p aiterm-intake-core verifier`。Expected: PASS

- [ ] **Step 5: Commit** — `git add src-tauri/crates/aiterm-intake-core/src/verifier.rs && git commit -m "feat(board): 驗收提示詞與驗收退回的修正卡內文"`

### Task 4: 資料表與存取（`board_verify`、`board_verify_deny`、`verify_enabled`）

**Files:**
- Modify: `src-tauri/crates/aiterm-intake-core/src/board_store.rs`（`init_schema` 加兩個 `CREATE TABLE IF NOT EXISTS`；新增存取函式；測試同檔）

- [ ] **Step 1: 失敗的測試**（加在 `mod tests`，沿用同檔 `pool()`、`card()` 輔助）

```rust
    #[tokio::test]
    async fn verify_rows_round_trip_and_the_card_view_joins_them() {
        let p = pool().await;
        let c = card(&p, "驗收").await;
        set_verify_meta(&p, &c.id, "sig-1", &[4001, 4002, 4003]).await.unwrap();
        assert_eq!(get_verify(&p, &c.id).await.unwrap().unwrap().signature, "sig-1");
        assert_eq!(get_verify(&p, &c.id).await.unwrap().unwrap().ports, vec![4001, 4002, 4003]);
        save_verify_report(&p, &c.id, r#"{"verdict":"pass","items":[]}"#, &["git push".to_string()], 2).await.unwrap();
        let v = get_verify(&p, &c.id).await.unwrap().unwrap();
        assert_eq!((v.manual_count, v.denied.clone(), v.report.is_some()), (2, vec!["git push".to_string()], true));
        set_verify_override(&p, &c.id, "小華").await.unwrap();
        assert_eq!(get_verify(&p, &c.id).await.unwrap().unwrap().overridden_by.as_deref(), Some("小華"));
        assert!(get_verify(&p, "不存在").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn project_deny_overrides_keep_add_and_remove_separate_and_per_project() {
        let p = pool().await;
        put_deny_override(&p, "P1", "Bash(docker push:*)", "remove", true).await.unwrap();
        put_deny_override(&p, "P1", "Bash(make deploy:*)", "add", true).await.unwrap();
        put_deny_override(&p, "P2", "Bash(foo:*)", "add", true).await.unwrap();
        let p1 = list_deny_overrides(&p, "P1").await.unwrap();
        assert_eq!(p1.len(), 2);
        assert!(p1.iter().any(|o| o.rule == "Bash(docker push:*)" && o.kind == "remove"));
        assert_eq!(list_deny_overrides(&p, "P2").await.unwrap().len(), 1);
        delete_deny_override(&p, "P1", "Bash(make deploy:*)").await.unwrap();
        assert_eq!(list_deny_overrides(&p, "P1").await.unwrap().len(), 1);
    }
```

- [ ] **Step 2: 確認失敗** — 編譯失敗（函式未定義）。

- [ ] **Step 3: 實作**

`init_schema` 加：

```rust
        "CREATE TABLE IF NOT EXISTS board_verify (card_id TEXT PRIMARY KEY NOT NULL, signature TEXT NOT NULL, ports TEXT NOT NULL DEFAULT '[]', report TEXT, denied TEXT NOT NULL DEFAULT '[]', manual_count INTEGER NOT NULL DEFAULT 0, overridden_by TEXT, created_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS board_verify_deny (project_id TEXT NOT NULL, rule TEXT NOT NULL, kind TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, PRIMARY KEY (project_id, rule))",
```

存取函式（`BoardVerify`、`DenyOverride` 兩個 `Serialize` struct；`ports`／`denied` 以 JSON 字串存）：

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct BoardVerify {
    pub card_id: String,
    pub signature: String,
    pub ports: Vec<u16>,
    pub report: Option<String>,
    pub denied: Vec<String>,
    pub manual_count: i64,
    pub overridden_by: Option<String>,
}

pub async fn set_verify_meta(pool: &SqlitePool, card_id: &str, signature: &str, ports: &[u16]) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO board_verify (card_id, signature, ports, created_at) VALUES (?,?,?,?) ON CONFLICT(card_id) DO UPDATE SET signature = excluded.signature, ports = excluded.ports")
        .bind(card_id).bind(signature).bind(serde_json::to_string(ports).unwrap_or_default()).bind(now()).execute(pool).await?;
    Ok(())
}

pub async fn save_verify_report(pool: &SqlitePool, card_id: &str, report_json: &str, denied: &[String], manual_count: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_verify SET report = ?, denied = ?, manual_count = ? WHERE card_id = ?")
        .bind(report_json).bind(serde_json::to_string(denied).unwrap_or_default()).bind(manual_count).bind(card_id).execute(pool).await?;
    Ok(())
}

pub async fn set_verify_override(pool: &SqlitePool, card_id: &str, by: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_verify SET overridden_by = ? WHERE card_id = ?").bind(by).bind(card_id).execute(pool).await?;
    Ok(())
}

pub async fn get_verify(pool: &SqlitePool, card_id: &str) -> Result<Option<BoardVerify>, sqlx::Error> {
    let row: Option<(String, String, String, Option<String>, String, i64, Option<String>)> =
        sqlx::query_as("SELECT card_id, signature, ports, report, denied, manual_count, overridden_by FROM board_verify WHERE card_id = ?").bind(card_id).fetch_optional(pool).await?;
    Ok(row.map(|(card_id, signature, ports, report, denied, manual_count, overridden_by)| BoardVerify {
        card_id, signature, report, manual_count, overridden_by,
        ports: serde_json::from_str(&ports).unwrap_or_default(),
        denied: serde_json::from_str(&denied).unwrap_or_default(),
    }))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DenyOverride {
    pub rule: String,
    /// `add`｜`remove`
    pub kind: String,
    pub enabled: bool,
}

pub async fn put_deny_override(pool: &SqlitePool, project_id: &str, rule: &str, kind: &str, enabled: bool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO board_verify_deny (project_id, rule, kind, enabled) VALUES (?,?,?,?) ON CONFLICT(project_id, rule) DO UPDATE SET kind = excluded.kind, enabled = excluded.enabled")
        .bind(project_id).bind(rule).bind(kind).bind(enabled).execute(pool).await?;
    Ok(())
}

pub async fn delete_deny_override(pool: &SqlitePool, project_id: &str, rule: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM board_verify_deny WHERE project_id = ? AND rule = ?").bind(project_id).bind(rule).execute(pool).await?;
    Ok(())
}

pub async fn list_deny_overrides(pool: &SqlitePool, project_id: &str) -> Result<Vec<DenyOverride>, sqlx::Error> {
    let rows: Vec<(String, String, bool)> = sqlx::query_as("SELECT rule, kind, enabled FROM board_verify_deny WHERE project_id = ? ORDER BY rule").bind(project_id).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|(rule, kind, enabled)| DenyOverride { rule, kind, enabled }).collect())
}
```

- [ ] **Step 4: 確認通過** — `cargo test -p aiterm-intake-core verify_rows project_deny` → PASS
- [ ] **Step 5: Commit** — `git add src-tauri/crates/aiterm-intake-core/src/board_store.rs && git commit -m "feat(board): 驗收報告與專案禁止清單覆寫的資料表"`

### Task 5: 禁止清單 `deny.rs`

**Files:**
- Create: `src-tauri/crates/aiterm-intake-server/src/deny.rs`
- Modify: `src-tauri/crates/aiterm-intake-server/src/lib.rs`（`pub mod deny;`）

- [ ] **Step 1: 失敗的測試**（同檔 `mod tests`）

```rust
    #[test]
    fn defaults_cover_the_categories_and_every_rule_is_well_formed() {
        let d = default_rules();
        for needle in ["Bash(sudo:*)", "Bash(git push:*)", "Bash(npm publish:*)", "Bash(ssh:*)", "Bash(rm -rf /:*)", "Bash(rm -rf ~:*)", "Bash(npm install -g:*)"] {
            assert!(d.iter().any(|r| r.rule == needle), "預設缺：{needle}");
        }
        assert!(d.iter().all(|r| validate_rule(&r.rule).is_ok() && !r.note.is_empty()));
    }

    #[test]
    fn rules_must_be_a_single_bash_pattern() {
        assert!(validate_rule("Bash(foo:*)").is_ok());
        for bad in ["", "foo", "Bash()", "Bash(a:*)\nBash(b:*)", "Read(x)", "Bash(a:*", &format!("Bash({}:*)", "x".repeat(300))] {
            assert!(validate_rule(bad).is_err(), "應該拒絕：{bad:?}");
        }
    }

    #[test]
    fn effective_rules_apply_global_then_project_overrides() {
        let global = vec![
            Rule { rule: "Bash(sudo:*)".into(), note: "x".into(), enabled: true },
            Rule { rule: "Bash(docker push:*)".into(), note: "x".into(), enabled: true },
            Rule { rule: "Bash(ssh:*)".into(), note: "x".into(), enabled: false },
        ];
        let overrides = vec![
            DenyOverrideRow { rule: "Bash(docker push:*)".into(), kind: "remove".into(), enabled: true },
            DenyOverrideRow { rule: "Bash(make deploy:*)".into(), kind: "add".into(), enabled: true },
            DenyOverrideRow { rule: "Bash(rm -rf build:*)".into(), kind: "add".into(), enabled: false },
        ];
        let eff = effective(&global, &overrides);
        assert_eq!(eff, vec!["Bash(sudo:*)".to_string(), "Bash(make deploy:*)".to_string()], "停用的、被專案移除的、停用的新增都不算");
    }
```

- [ ] **Step 2: 確認失敗** — 編譯失敗。

- [ ] **Step 3: 實作**

```rust
//! 驗收者的禁止清單：全域預設（server 一份，管理員編輯）＋專案覆寫（移除某條預設／新增專屬規則）。
//! 不是沙箱：只擋最明顯的誤操作（見 spec §6.3）。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub rule: String,
    pub note: String,
    pub enabled: bool,
}

/// 專案覆寫（對應 `board_store::DenyOverride`；放在這裡讓純函式不依賴資料庫型別）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyOverrideRow {
    pub rule: String,
    pub kind: String,
    pub enabled: bool,
}

pub const MAX_RULE_LEN: usize = 200;

pub fn validate_rule(r: &str) -> Result<(), String> {
    let t = r.trim();
    if !(t.starts_with("Bash(") && t.ends_with(')')) {
        return Err("規則要寫成 Bash(命令:*) 的形式".into());
    }
    if t.contains('\n') || t.contains('\r') {
        return Err("規則不能換行".into());
    }
    if t.len() > MAX_RULE_LEN {
        return Err(format!("規則最長 {MAX_RULE_LEN} 字元"));
    }
    if t["Bash(".len()..t.len() - 1].trim().is_empty() {
        return Err("括號裡不能是空的".into());
    }
    if t[5..t.len() - 1].contains(')') && !t[5..t.len() - 1].ends_with(":*") {
        return Err("規則格式不對".into());
    }
    Ok(())
}

pub fn default_rules() -> Vec<Rule> {
    let r = |rule: &str, note: &str| Rule { rule: rule.into(), note: note.into(), enabled: true };
    vec![
        r("Bash(sudo:*)", "提權"),
        r("Bash(su:*)", "提權"),
        r("Bash(git push:*)", "對外發佈：推到遠端"),
        r("Bash(npm publish:*)", "對外發佈：發佈套件"),
        r("Bash(cargo publish:*)", "對外發佈：發佈套件"),
        r("Bash(gh pr create:*)", "對外發佈：建立 PR"),
        r("Bash(gh release create:*)", "對外發佈：建立 release"),
        r("Bash(docker push:*)", "對外發佈：推映像檔"),
        r("Bash(ssh:*)", "遠端連線"),
        r("Bash(scp:*)", "遠端連線"),
        r("Bash(sftp:*)", "遠端連線"),
        r("Bash(curl:* | sh)", "下載後直接執行"),
        r("Bash(curl:* | bash)", "下載後直接執行"),
        r("Bash(wget:* | sh)", "下載後直接執行"),
        r("Bash(wget:* | bash)", "下載後直接執行"),
        r("Bash(rm -rf /:*)", "破壞性刪除：根目錄"),
        r("Bash(rm -rf ~:*)", "破壞性刪除：家目錄"),
        r("Bash(rm -rf $HOME:*)", "破壞性刪除：家目錄"),
        r("Bash(rm -rf ..:*)", "破壞性刪除：上層目錄"),
        r("Bash(launchctl:*)", "改系統設定"),
        r("Bash(systemctl:*)", "改系統設定"),
        r("Bash(defaults write:*)", "改系統設定"),
        r("Bash(npm install -g:*)", "改全域環境"),
        r("Bash(npm i -g:*)", "改全域環境"),
        r("Bash(pip install --user:*)", "改全域環境"),
    ]
}

/// 有效的禁止規則：全域啟用的，扣掉專案移除的，加上專案啟用的新增。
pub fn effective(global: &[Rule], overrides: &[DenyOverrideRow]) -> Vec<String> {
    let removed: Vec<&str> = overrides.iter().filter(|o| o.kind == "remove" && o.enabled).map(|o| o.rule.as_str()).collect();
    let mut out: Vec<String> = global.iter().filter(|g| g.enabled && !removed.contains(&g.rule.as_str())).map(|g| g.rule.clone()).collect();
    for o in overrides.iter().filter(|o| o.kind == "add" && o.enabled) {
        if !out.contains(&o.rule) {
            out.push(o.rule.clone());
        }
    }
    out
}
```

全域清單的讀寫放 `server.db` 設定（沿用 `store::get_setting`／`set_setting`，鍵 `verify_deny_global`，值為 `Vec<Rule>` 的 JSON；沒有設定就用 `default_rules()`）：

```rust
pub const GLOBAL_KEY: &str = "verify_deny_global";

pub async fn load_global(pool: &sqlx::SqlitePool) -> Vec<Rule> {
    aiterm_intake_core::store::get_setting(pool, GLOBAL_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(default_rules)
}

pub async fn save_global(pool: &sqlx::SqlitePool, rules: &[Rule]) -> Result<(), String> {
    for r in rules {
        validate_rule(&r.rule)?;
    }
    aiterm_intake_core::store::set_setting(pool, GLOBAL_KEY, &serde_json::to_string(rules).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())
}
```

（注意：`get_setting`／`set_setting` 用的是 intake 資料庫的 `intake_settings` 表，與 `orchestrate:<pid>` 同處；管理員設定放那裡一致。）

- [ ] **Step 4: 確認通過** — `cargo test -p aiterm-intake-server --lib deny` → PASS
- [ ] **Step 5: Commit** — `git add src-tauri/crates/aiterm-intake-server/src/deny.rs src-tauri/crates/aiterm-intake-server/src/lib.rs && git commit -m "feat(board): 驗收者的禁止清單（全域預設＋專案覆寫、格式檢查）"`

### Task 6: 驗收的執行環境 `verify_env.rs`（埠、Chrome、mcp.json）

**Files:**
- Create: `src-tauri/crates/aiterm-intake-server/src/verify_env.rs`
- Modify: `lib.rs`（`pub mod verify_env;`）

- [ ] **Step 1: 失敗的測試**

```rust
    #[test]
    fn ports_are_distinct_and_the_mcp_config_only_has_the_pinned_headless_browser() {
        let ports = pick_free_ports(3);
        assert_eq!(ports.len(), 3);
        assert_eq!(ports.iter().collect::<std::collections::HashSet<_>>().len(), 3);
        let json = mcp_config_json(std::path::Path::new("/v/evidence"), Browser::SystemChrome);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["mcpServers"].as_object().unwrap().len(), 1, "只有 playwright，不帶別的 MCP");
        assert_eq!(v["mcpServers"]["playwright"]["args"], serde_json::json!(["-y", "@playwright/mcp@0.0.83", "--headless", "--isolated", "--output-dir", "/v/evidence"]));
        let bundled: serde_json::Value = serde_json::from_str(&mcp_config_json(std::path::Path::new("/v/evidence"), Browser::Bundled)).unwrap();
        assert!(bundled["mcpServers"]["playwright"]["args"].as_array().unwrap().iter().any(|a| a == "chromium"));
    }

    #[cfg(unix)]
    #[test]
    fn only_processes_listening_on_the_given_ports_are_stopped() {
        use std::process::{Command, Stdio};
        let mine = pick_free_ports(1)[0];
        let other = pick_free_ports(1)[0];
        let spawn = |port: u16| Command::new("python3").args(["-c", &format!("import socket,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',{port}));s.listen(1);time.sleep(60)")]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let (mut a, mut b) = (spawn(mine), spawn(other));
        std::thread::sleep(std::time::Duration::from_millis(600));
        let stopped = stop_listeners(&[mine]);
        assert_eq!(stopped.len(), 1, "只結束那個埠上的行程");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(a.try_wait().unwrap().is_some(), "指定埠的行程結束了");
        assert!(b.try_wait().unwrap().is_none(), "別的埠的行程沒動");
        b.kill().ok();
        b.wait().ok();
    }

    #[test]
    fn node_availability_is_detected_by_running_npx_version() {
        // 只驗證函式不會 panic 並回傳布林（這台機器有沒有 Node 不一定）。
        let _ = node_available();
    }
```

- [ ] **Step 2: 確認失敗** — 編譯失敗。

- [ ] **Step 3: 實作**

由桌面版 `tasks/verify_ports.rs`（`pick_free_ports`、`pids_listening`（unix `lsof`／windows `netstat`＋`parse_netstat`）、`terminate`、`stop_listeners`）與 `tasks/ask.rs`（`PLAYWRIGHT_MCP`、`Browser`、`detect_browser`、`playwright_args`）複製，並新增：

```rust
/// `mcp.json` 內容：只放釘死版本的無頭、獨立設定檔瀏覽器。
pub fn mcp_config_json(out: &std::path::Path, browser: Browser) -> String {
    serde_json::json!({ "mcpServers": { "playwright": { "command": "npx", "args": playwright_args(out, browser) } } }).to_string()
}

/// 這台電腦有沒有 `npx`（沒有就沒有瀏覽器工具，驗收者要把需要瀏覽器的條件標 manual）。
pub fn node_available() -> bool {
    std::process::Command::new(if cfg!(windows) { "npx.cmd" } else { "npx" })
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
```

`stop_listeners` 內以 `std::process::id()` 排除 server 自己。其餘照搬（含 Windows 版本的 `cfg`）；模組頂端註明來源檔案與「不改桌面版、兩份各自維護」。

- [ ] **Step 4: 確認通過** — `cargo test -p aiterm-intake-server --lib verify_env`（python3 是 macOS／Linux 內建；Windows 該測試被 `cfg(unix)` 略過）。Expected: PASS
- [ ] **Step 5: Commit** — `git add src-tauri/crates/aiterm-intake-server/src/verify_env.rs src-tauri/crates/aiterm-intake-server/src/lib.rs && git commit -m "feat(board): 驗收用的空閒埠、佔埠行程清理、瀏覽器偵測與 mcp 設定"`

### Task 7: 驗收模式的 `claude -p` 參數與被擋命令擷取

**Files:**
- Modify: `src-tauri/crates/aiterm-intake-server/src/runner.rs`（`Mode`、`claude_args_with`、`exec_claude_session` 回傳型別、`parse_output`）
- Test: `src-tauri/crates/aiterm-intake-server/tests/auto_process.rs`（它已有 `claude_args` 測試）與 `runner.rs` 內 `mod tests`

- [ ] **Step 1: 失敗的測試**（`tests/auto_process.rs` 的參數測試旁新增）

```rust
#[test]
fn verify_mode_gives_bash_and_the_browser_but_no_edit_tools_and_carries_the_deny_list() {
    let s = runner::RunnerSettings::default();
    let mode = runner::Mode::Verify { deny: vec!["Bash(sudo:*)".into(), "Bash(git push:*)".into()], browser: true, mcp_config: Some("/v/mcp.json".into()) };
    let args = runner::claude_args(&s, &mode, &[]);
    let after = |f: &str| args.iter().position(|a| a == f).map(|i| args[i + 1].clone());
    assert_eq!(after("--allowedTools").as_deref(), Some("Read,Grep,Glob,Bash,mcp__playwright"));
    let disallowed = after("--disallowedTools").unwrap();
    for needle in ["Edit", "Write", "MultiEdit", "WebFetch", "WebSearch", "Bash(sudo:*)", "Bash(git push:*)"] {
        assert!(disallowed.contains(needle), "禁止清單缺：{needle}");
    }
    assert_eq!(after("--mcp-config").as_deref(), Some("/v/mcp.json"));
    assert!(args.iter().any(|a| a == "--strict-mcp-config"));
    let no_browser = runner::claude_args(&s, &runner::Mode::Verify { deny: vec![], browser: false, mcp_config: None }, &[]);
    assert_eq!(no_browser.iter().position(|a| a == "--allowedTools").map(|i| no_browser[i + 1].clone()).as_deref(), Some("Read,Grep,Glob,Bash"));
    assert!(!no_browser.iter().any(|a| a == "--mcp-config"));
}

#[test]
fn denied_bash_calls_are_read_from_claude_output() {
    let out = r#"{"type":"result","is_error":false,"result":"ok","permission_denials":[{"tool_name":"Bash","tool_input":{"command":"git push origin main"}},{"tool_name":"Edit","tool_input":{"file_path":"x"}}]}"#;
    assert_eq!(runner::denied_commands(out), vec!["git push origin main".to_string()]);
    assert!(runner::denied_commands("not json").is_empty());
}
```

- [ ] **Step 2: 確認失敗** — 編譯失敗（`Mode::Verify`、`denied_commands` 未定義）。

- [ ] **Step 3: 實作**

`Mode` 加 `Verify { deny: Vec<String>, browser: bool, mcp_config: Option<String> }`；`claude_args_with` 的 `match mode` 加：

```rust
        Mode::Verify { deny, browser, .. } => {
            let allowed = if *browser { "Read,Grep,Glob,Bash,mcp__playwright".to_string() } else { "Read,Grep,Glob,Bash".to_string() };
            let mut dis = vec!["Edit".to_string(), "Write".into(), "MultiEdit".into(), "NotebookEdit".into(), "WebFetch".into(), "WebSearch".into()];
            dis.extend(deny.iter().cloned());
            (allowed, dis.join(","))
        }
```

在 `args.extend(["--max-turns" …])` 之後：

```rust
    if let Mode::Verify { mcp_config: Some(path), .. } = mode {
        args.extend(["--mcp-config".into(), path.clone(), "--strict-mcp-config".into()]);
    }
```

`denied_commands`（`pub fn denied_commands(stdout: &str) -> Vec<String>`）解析 `permission_denials[]` 中 `tool_name == "Bash"` 的 `tool_input.command`。`exec_claude_session` 目前只回 `String`；加 `exec_claude_full(...) -> Result<ClaudeRun, String>`，`ClaudeRun { text: String, denied: Vec<String> }`，`exec_claude_session` 改成呼叫它並丟掉 `denied`（既有呼叫端不變）。實作時把子行程的 `stdout` 先存下來再 `parse_output` 與 `denied_commands`。

- [ ] **Step 4: 確認通過** — `cargo test -p aiterm-intake-server --test auto_process verify_mode denied_bash` 並跑 `--lib runner`。Expected: PASS；既有 `claude_args` 測試仍通過。
- [ ] **Step 5: Commit** — `git add src-tauri/crates/aiterm-intake-server/src/runner.rs src-tauri/crates/aiterm-intake-server/tests/auto_process.rs && git commit -m "feat(board): 驗收模式的 claude 參數（Bash＋禁止清單＋瀏覽器）與被擋命令擷取"`

### Task 8: 編排——建驗收卡、vfix、合併卡閘門

**Files:**
- Modify: `src-tauri/crates/aiterm-intake-server/src/orchestrator.rs`
- Modify: `src-tauri/crates/aiterm-intake-core/src/board_store.rs`（`create_workflow_card` 若白名單了 `kind`，允許 `verify`／`vfix`；`project_cards` 等不變）

要點（逐項寫成測試後實作，測試放 Task 10 的整合測試檔）：

1. **`verify_enabled(pool, pid)`／`set_verify_enabled`**：鍵 `verify_enabled:<pid>`，值 `"1"`（同 `enabled`）。
2. **所有 `matches!(c.card_kind.as_str(), "review" | "fix" | "plan")`** 加 `"vfix"`：`orchestrator.rs`（`advance`、`deps_ready`、`premerge_dependencies`）、`scheduler.rs`（`round_result`）。用 `grep -n '"review" | "fix" | "plan"' -r src-tauri/crates` 全找出來，一處不漏。
3. **`advance` 在建合併卡前**（目前 `if !pending_merge && orch::merge_status(...) == Ready`）改成：

```rust
    let verify_on = verify_enabled(pool, pid).await && agents.iter().any(|a| a.kind == "verifier");
    if merge_status_is_ready && !pending_merge {
        if !verify_on {
            // 現有行為：建合併卡
        } else {
            let sig = current_signature(pool, pid, &mods, &cards, &agents).await?;
            let views = verify_views(pool, &cards).await?; // 由 board_verify 與 verify 卡組 VerifyCardView（舊→新）
            match verifier_core::gate(&views, &sig) {
                Gate::NeedsVerify => create_verify_card(...),   // status QUEUED、agent_id = verifier、kind "verify"，並 set_verify_meta(sig, 空閒埠)
                Gate::Passed => create_merge_card(...),          // 內文附驗收結論摘要
                Gate::Failed { rounds } => open_vfix_cards(...), // 依 attribute 對每個模組建 vfix 卡
                Gate::Waiting | Gate::NeedsHuman => {}
            }
        }
    }
```

`current_signature`：各模組 `module_status` 的 `Passed { sha }` 組成 `(模組 id, sha)`，加上「重新驗收序號」（`retry` = 該簽章前綴下被管理員重試的次數，存在 `board_verify.signature` 後綴不可行；改成：`retry` = 已存在的 verify 卡中「簽章基底相同」的卡數，簽章存成 `基底#序號`；`gate` 比對時用完整簽章）。**實作時先簡化**：簽章基底以 `signature(passed, 0)`；重新驗收（`retry`）時建新卡，簽章用 `signature(passed, n)`（n = 同基底已有的卡數）；`gate` 比對 `current` = 同基底下**最新**那張的完整簽章。此邏輯寫成一個 `latest_signature(views, base_prefix)`。

4. **`open_vfix_cards`**：`attribute(report, module_ids)`；每個有模組的群組建一張 `vfix` 卡（`agent_id` = 模組的 Coder、`module_id` = 模組、`review_of` = 該模組鏈目前的尾端（沿用 `module_state` 的尾端計算——為此在 core 加 `pub fn chain_tail<'a>(module, chain) -> &'a BoardCard`，與 `module_state` 共用）、內文 = `vfix_body`）。未歸屬的失敗項：verify 卡 `verdict` 改為 `attention`（若沒有任何有模組的群組）。
5. **`tick_once` 可派的卡種類**：`matches!(c.card_kind.as_str(), "work" | "fix" | "review" | "plan")` 加 `"vfix" | "verify"`。`vfix` 走 Coder 路徑（`run_inner` 的 work/fix 分支，條件改成 `matches!(kind, "work"|"fix"|"vfix")`）；`verify` 走新的 `verifier::run_verify`。
6. **`merge_approved` 與 `merge_status` 不變**（合併卡只在通過後才建）。

- [ ] **Step 1–5**（每一點：先在 Task 10 的 `tests/verify.rs` 寫對應測試並確認失敗 → 實作 → 通過 → commit）。這一任務的 commit：`feat(board): 啟用驗收時全部通過後先建驗收卡，通過才建合併卡；驗收退回開 vfix 卡`

### Task 9: 執行驗收 `verifier::run_verify`

**Files:**
- Create: `src-tauri/crates/aiterm-intake-server/src/verifier.rs`
- Modify: `lib.rs`；`orchestrator.rs`（`run_inner` 分派、`run_card` 結束時清理）

行為（`pub async fn run_verify(st: &Arc<AppState>, settings: &RunnerSettings, work_dir: &Path, agent: &BoardAgent, card: &BoardCard) -> Result<String, String>`）：

1. 讀 `board_verify`（簽章、埠）；沒有埠就現挑（`pick_free_ports(3)`）並寫回。
2. **整合 worktree** `st.data_dir/worktrees/verify-<card.id 前 12 碼>`：`git worktree add --detach <wt> HEAD`；對每個有分支的 Coder（`agent_branch`，`rev-parse --verify` 成功才合）依序 `git merge --no-ff -m … <branch>`；衝突 → `merge --abort`、回傳 `Err(format!("合併衝突：{}", agent.name))`（驗收卡失敗；`run_card` 會寫 `failed`）。
3. 證據與輸出資料夾：`st.data_dir/verify/<card.id>/{evidence}`；`verify.json` 路徑 `…/verify/<card.id>/verify.json`。建立資料夾。
4. 提示詞：模組清單 `(id, title, body)` 取專案的 `modules()`；`spec_text`（取最後一個 `is_spec` 模組通過的規格，沿用 `spec_text()`；沒有就空）；`verify_prompt(..., has_browser = verify_env::node_available())`。
5. 瀏覽器設定：`has_browser` 時寫 `mcp.json` 到卡片資料夾（`mcp_config_json(evidence, detect_browser())`）。
6. 禁止清單：`deny::effective(&deny::load_global(server pool), &project overrides)`。
7. 執行：`runner::exec_claude_full(settings_with_verify_timeouts, &wt, &Mode::Verify{…}, &[], &prompt, agent.model.as_deref(), &SessionUse::…)`（**接續機制**：驗收卡也用 `board_card_session` 的對話 id；`run_inner` 現有的「prior／resume」邏輯抽成共用函式 `run_with_session(card_id, …)` 供 work/fix/vfix/verify 使用；驗收卡接續時 `start_sha` 為空，用 `filter` 放寬）。
8. **無論成敗都清理**（`finally` 式）：`stop_listeners(ports)`、`git worktree remove --force`＋`remove_dir_all`、`prune`。
9. 解析：讀 `verify.json`；缺或 `parse_verify_json` 失敗 → `Err("驗收者沒有寫出有效的 verify.json：…")`；`check_quality` 不合格 → `Err("驗收報告品質不合格：…")`（驗收卡失敗，可重新驗收）。
10. 存報告：`save_verify_report(report_json, denied（來自 ClaudeRun）, manual_count)`；`bs::set_verdict` 寫 `pass`／`fail`（`attention` 在 `advance` 判定：全部失敗項找不到模組）。回傳摘要字串（「驗收通過（N 項需你確認）」／「驗收退回 N 項」）。

測試（Task 10 的 `tests/verify.rs`）：假 claude 依參數判斷是驗收（`--allowedTools` 含 `Bash` 且不含 `Edit`），在 `$VERIFY_JSON`（從提示詞抓路徑，與桌面版測試同法）寫 verify.json。

- [ ] **Commit**：`feat(board): server 執行驗收（整合 worktree、驗收模式、清理、報告）`

### Task 10: 後端整合測試（假 claude）

**Files:**
- Create: `src-tauri/crates/aiterm-intake-server/tests/verify.rs`（頂端 `#![cfg(unix)]`；沿用 `tests/orchestrate.rs` 的 `Fake`、`rig`、`repo`、`pump`，複製需要的輔助函式——測試檔之間不共用輔助，與現有慣例一致）

必須涵蓋（每個一個 `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`，先寫、先看它失敗）：

1. `with_verification_on_the_merge_card_appears_only_after_a_passing_verification`：開驗收、全部模組通過 → 先出現 `verify` 卡（沒有 `merge` 卡）；假 claude 寫 pass 的 verify.json → 才出現 `merge` 卡，內文含驗收結論。
2. `a_failed_verification_sends_the_failing_module_back_and_verifies_again_after_the_fix`：假 claude 第一次回 fail（`module_id` = 模組）→ 出現 `vfix` 卡給該 Coder → 做完審查通過 → 簽章變了 → 新 verify 卡 → 這次 pass → 合併卡。
3. `failures_without_a_known_module_stop_for_a_human_and_an_admin_can_override`：fail 且 `module_id` 對不上 → verify 卡 `verdict = attention`，沒有 vfix、沒有 merge；管理員 `override` → 合併卡出現。
4. `after_two_rejected_rounds_it_stops_and_retry_makes_a_fresh_verification`：連續兩次 fail → 第三次不再開 vfix（`NeedsHuman`）；管理員 `retry` → 新 verify 卡。
5. `a_merge_conflict_in_the_integration_worktree_fails_the_verification_with_the_reason`：兩位 Coder 改同一個檔案同一行 → verify 卡 `failed`，原因含「合併衝突」；專案資料夾與工程師分支沒被動到。
6. `a_missing_or_broken_verify_json_fails_the_card_and_never_passes`。
7. `verify_json_without_evidence_for_a_pass_is_rejected`。
8. `the_deny_list_and_ports_reach_the_command_line_and_listeners_are_cleaned`：假 claude 啟動一個監聽被指派埠的 python 行程後結束；驗收後該行程被結束、別的埠上的行程沒動；命令列帶了 `--disallowedTools` 含預設規則與專案覆寫。
9. `without_verification_enabled_the_flow_is_unchanged`：沒開驗收（或沒有驗收者）→ 與現在一樣直接出現合併卡（回歸）。
10. `the_verification_worktree_and_session_are_cleaned_up_whatever_the_outcome`。
11. `an_interrupted_verification_resumes_its_conversation`（用額度用完那招，沿用接續測試的寫法）。

- [ ] **跑法**：`cd src-tauri && cargo test -p aiterm-intake-server --test verify 2>&1 | grep -E "^test |test result|panicked"`
- [ ] **Commit**：`test(board): server 驗收者的整條流程（假 claude）`

### Task 11: API 與稽核

**Files:**
- Create: `src-tauri/crates/aiterm-intake-server/src/routes_verify.rs`
- Modify: `routes.rs`（`.merge(routes_verify::routes())`）、`lib.rs`

端點（表見 spec §7.2）。實作要點：

- 角色檢查沿用 `routes_board.rs` 的做法（`Extension(me): Extension<Me>`；管理員檢查的 helper 照現有 `require_admin`）。
- `GET /v1/board/cards/{id}/verify`：回 `{ card, signature, report（已解析的 JSON）, denied, manual_count, overridden_by, evidence: [檔名…] }`；證據檔名只列證據資料夾第一層的檔案。
- `GET …/verify/files/{name}`：`name` 必須通過 `is_safe_name`（不含 `/`、`\`、`..`、不以 `.` 開頭；長度上限）；從 `st.data_dir/verify/<card id>/evidence/` 讀；`Content-Type` 依副檔名（png／jpg／txt）；不在就 404。
- `POST …/override`：驗收卡必須是 `done` 且 `verdict ∈ {fail, attention}` 或 `outcome = failed`；`set_verdict(pass)`、`set_verify_override(by)`、`complete` 已結案的卡不動狀態只改 verdict；寫稽核 `board.verify_override`；`st.events.changed()`。
- `POST …/retry`：同條件；建一張新的 verify 卡（簽章加序號）；稽核 `board.verify_retry`。
- `PUT /v1/board/projects/{pid}/verify {enabled}`：管理員；要求專案有 `work_dir` 與驗收者（否則 400 並說明）；稽核 `board.verify_enable`／`disable`。
- `GET/PUT /v1/verify/deny`、`GET/PUT /v1/board/projects/{pid}/verify/deny`：管理員；PUT 逐條 `validate_rule`，不合法回 400 並指出哪一條；異動寫稽核 `board.verify_deny`（記 diff 摘要）。

測試（加在 `tests/verify.rs` 或新的 `tests/verify_api.rs`）：成員唯讀、非管理員 403；證據檔路徑穿越（`..%2f`、`/etc/passwd`、`.hidden`）被拒；PUT 不合法規則被拒；override／retry 的前置條件；稽核紀錄有寫。

- [ ] **Commit**：`feat(board): 驗收報告、證據檔、放行、重新驗收與禁止清單的 API`

### Task 12: 啟動時清理與里程碑 1 全量驗證

- [ ] `server.rs` 啟動：除了既有的「結束孤兒 claude」，對所有 `board_verify.ports` 再 `stop_listeners`（驗收中 server 被強制結束時留下的伺服器）。測試：寫入一筆 `board_verify` 與一個監聽該埠的假行程，呼叫啟動清理函式 `server::cleanup_stale_verify(&state)`，行程被結束。
- [ ] 跑 `cd src-tauri && cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result|FAILED|panicked|^error"`，逐行確認無失敗。
- [ ] **Commit**：`feat(board): server 啟動時清掉驗收留下的佔埠行程`；里程碑 1 完成。

---

# 里程碑 2：報告與畫面

### Task 13: 桌面 App 端的指令（Rust）

**Files:** `src-tauri/src/intake/board_remote.rs`、`src-tauri/src/intake/commands.rs`、`src-tauri/src/lib.rs`

- `board_remote.rs`：`verify_report(api, card_id)`、`verify_file(api, card_id, name) -> base64 data URL`（server 回原始位元組；沿用 `Api` 取二進位的方法——先看 `attachment` 的做法）、`verify_override`、`verify_retry`、`verify_settings`／`set_verify_enabled`、`deny_global`／`set_deny_global`、`deny_project`／`set_deny_project`。
- `commands.rs`：對應的 `#[tauri::command]`（`intake_verify_report`、`intake_verify_file`、`intake_verify_override`、`intake_verify_retry`、`intake_verify_get`、`intake_verify_set`、`intake_deny_global_get/set`、`intake_deny_project_get/set`），註冊到 `lib.rs` 的 `invoke_handler`。錯誤沿用 `board_remote::fail`。
- 測試：沿用 `board_remote.rs` 現有測試風格（`TestServer`＋`paired`）：取報告、取檔、放行、重試、規則往返。

- [ ] **Commit**：`feat(board): AITerm 端的驗收報告、放行、重新驗收與禁止清單指令`

### Task 14: 前端 IPC 與資料型別

**Files:** `src/ipc/verify.ts`（新）

```ts
export interface VerifyItem { criterion: string; status: "pass" | "fail" | "manual"; module_id?: string | null; evidence: string[]; steps?: string | null; expected?: string | null; actual?: string | null; notes?: string | null }
export interface VerifyReport { verdict: "pass" | "fail"; project_type?: string | null; items: VerifyItem[]; notes?: string | null }
export interface VerifyView { signature: string; report: VerifyReport | null; denied: string[]; manual_count: number; overridden_by: string | null; evidence: string[] }
export interface DenyRule { rule: string; note: string; enabled: boolean }
export interface DenyOverride { rule: string; kind: "add" | "remove"; enabled: boolean }
export const verifyReport = (cardId: string) => invoke<VerifyView>("intake_verify_report", { cardId });
export const verifyFile = (cardId: string, name: string) => invoke<string>("intake_verify_file", { cardId, name });
export const verifyOverride = (cardId: string) => invoke<void>("intake_verify_override", { cardId });
export const verifyRetry = (cardId: string) => invoke<void>("intake_verify_retry", { cardId });
// 專案開關與禁止清單
```

- 測試：沿用既有 `src/ipc/*.test.ts` 的 mock invoke 寫法，驗證每個函式呼叫對的指令名與參數。
- **Commit**：`feat(board): 驗收的前端 IPC`

### Task 15: 驗收者卡片狀態與頂列

**Files:** `src/components/AgentCanvas/*`（先 `grep -n "verifier" src/components/AgentCanvas/*.tsx` 找出桌面版驗收者卡片與頂列摘要的位置，沿用）

- 共用專案的驗收者卡片狀態：驗收中／✓ 通過（N 項需你確認）／✗ 退回 N 項／⚠ 需要你決定，資料來自 verify 卡（`card_kind === "verify"`）的 `status`／`verdict`／`manual_count`。
- 頂列：驗收中 →「驗收中」；退回 →「驗收退回 N 項，修正中」；通過後才可核准合併（沿用現有 `ServerMergeBanner`，驗收中時隱藏核准按鈕並顯示原因）。
- `vfix` 卡在畫布上與 `fix` 同樣呈現（`grep -rn '"fix"' src/` 找出所有判斷處，加 `vfix`）。
- 測試：元件測試各狀態（RTL）。
- **Commit**：`feat(board): 共用畫布的驗收者卡片與頂列顯示驗收狀態`

### Task 16: 驗收報告對話框

**Files:** `src/components/AgentCanvas/VerifyReportDialog.tsx`（新）＋ `.test.tsx`

- 顯示每個條件的狀態徽章、重現步驟、預期與實際、證據（圖片用 `verifyFile` 取 data URL 顯示縮圖，點擊放大；非圖片證據顯示文字）、被禁止清單擋下的命令（附一句「若是誤擋，請管理員到禁止清單放寬」）、manual 項目與原因。
- 管理員（`useIntakeRemote().role === "admin"`）看得到「放行」「重新驗收」；放行前用 `plugin-dialog` 的 `confirm`。
- 入口：點驗收者卡片、合併卡上的「查看驗收報告」。
- 測試：渲染各項目；管理員與成員的按鈕差異；圖片載入失敗時顯示檔名不崩。
- **Commit**：`feat(board): 驗收報告對話框（含截圖縮圖、放行、重新驗收）`

### Task 17: 專案開關與禁止清單編輯

**Files:** 共用畫布的設定區（沿用「由 server 執行」開關所在元件）＋ `src/components/AgentCanvas/DenyListDialog.tsx`（新）

- 開關「由 server 執行驗收」（管理員可改；專案沒有驗收者或沒有工作資料夾時停用並說明原因）。
- 禁止清單對話框：兩個分頁「全域預設」「這個專案」；每條規則有說明、啟用勾選、刪除；新增規則時即時顯示格式錯誤；「還原預設」；儲存失敗顯示 server 回的哪一條不合法。
- i18n：所有字串加到 `src/lib/i18n.ts` 的 zhTW 與 en 兩份（en 是 `{...zhTW,...enRaw}` 合併，要兩邊都補，否則 tsc 抓不到；跑 `npx vitest run src/lib` 確認語系測試）。
- 測試：開關、規則增刪、錯誤顯示。
- **Commit**：`feat(board): 專案的驗收開關與禁止清單編輯`

### Task 18: 里程碑 2 全量驗證

- [ ] `npx tsc -b`、`npx vitest run`、`npx eslint <改過的檔案>`、`cd src-tauri && cargo test --workspace --no-fail-fast`。
- [ ] 啟動 dev 版（`npm run tauri:dev`，用使用者的真實資料前先告知；實機操作用隔離 HOME 失敗過——鑰匙圈授權框無法在測試用鑰匙圈通過，改用真實 AITerm 並事後清理），連到自己起的測試 server，開驗收開關，確認畫面（卡片狀態、報告、設定）。操作要點見記憶 `reference_mac_ui_automation`。

---

# 里程碑 3：真實 Claude 實機驗收與文件

### Task 19: 實機驗收（真實 Claude Code，會用額度）

用與「接續」驗證同樣的方式（`examples/drive.rs` 暫時工具；用完刪除）建立真實 server＋專案：

1. **有串接 bug 的小網頁專案**：兩個模組（頁面＋邏輯）各一位 Coder，故意讓串接處有 bug；跑到驗收 → 確認開了瀏覽器、截圖存在證據資料夾、判定退回、`module_id` 正確、vfix 卡給對的 Coder。
2. 修好後重新驗收通過 → 才出現合併卡 → 管理員核准 → 合併。
3. 驗收進行中 `kill -9` server 再重啟：確認沒有殘留的 claude、瀏覽器、佔埠行程，驗收接續完成。
4. 故意違規：在專案覆寫禁止清單新增一條會被觸發的規則（或要求 `git push` 到本機 bare repo 前先確認預設擋住），確認被擋並顯示在報告。
5. 確認驗收結束後整合 worktree 與對話檔都清掉（`git worktree list`、`~/.claude/projects` 檢查）。

任何一項失敗：照 `superpowers:systematic-debugging` 找根因，修了補自動測試，再重跑該項。

### Task 20: 文件與收尾

- [ ] `docs/intake-server.md` 新增「server 驗收者」一節：啟用條件、流程、禁止清單（含「不是沙箱」的誠實說明）、需要 Node／Chrome、成本提醒、Windows／Linux 未實測。
- [ ] `CHANGELOG.md` 的 `## v1.69.0` 補一條，**只寫驗證過的行為**，未實機的明寫「只有自動測試」。
- [ ] 清掉測試殘留（`examples/drive.rs`、`~/.claude/projects` 裡我的測試資料夾、測試 server 行程）。
- [ ] 全量測試通過後 commit。**不 push、不打 tag**，除非使用者明說。

---

## Self-Review（對照 spec）

- §3 角色與卡片：Task 1、8（`verify`、`vfix` 種類）。
- §4.1–4.5 流程、簽章、輪數、閘門：Task 1、2、8、10。
- §5.1 整合 worktree：Task 9、10（衝突）。§5.2 驗收模式：Task 7、9。§5.3 瀏覽器：Task 6、9。§5.4 verify.json：Task 2、9。§5.5 埠：Task 6、9、12。
- §6 禁止清單（兩層、格式檢查、稽核、被擋命令）：Task 5、7、9、11、17。
- §7 資料與 API：Task 4、11。
- §8 畫面：Task 15–17。
- §9 錯誤處理：Task 9、10、12。§11 測試：Task 10、18、19。§12 里程碑：本計畫三個里程碑。
- 型別一致：`VerifyCardView`、`Gate`、`signature`、`attribute`、`Mode::Verify { deny, browser, mcp_config }`、`ClaudeRun { text, denied }` 在各任務使用一致。
- 已知取捨：驗收的重試簽章以「基底＋序號」處理（Task 8 要點 3），實作時以測試 4 釘死；桌面版 `verify_ports.rs`／`ask.rs` 以複製方式重用，不改桌面版。
