#![cfg(unix)] // 假的 claude 是 shell 腳本，Windows 無法當成程式執行。
//! Server 驗收者：用假的 `claude` 驅動「全部模組通過 → 驗收（整合 worktree）→ 通過才有合併卡／退回就開修正卡」整條路。
#![allow(dead_code)]
use aiterm_intake_client::api::Api;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, BoardCard, Change, NewAgent, NewCard};
use aiterm_intake_server::orchestrator;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn sh(dir: &Path, cmd: &str) -> String {
    let o = Command::new("sh").arg("-c").arg(cmd).current_dir(dir).output().unwrap();
    assert!(o.status.success(), "{cmd}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    sh(d.path(), "git init -q -b main && git config user.email a@b.c && git config user.name t && echo v1 > app.txt && git add -A && git commit -qm init");
    d
}

/// 假 claude：看 `--allowedTools` 有沒有 Edit 判斷是 Coder 還是 Reviewer。
/// Coder 會在工作資料夾寫一個檔案；Reviewer 從 `verdicts` 檔依序取結論。
struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    fn new(verdicts: &[&str], coder_secs: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("verdicts"), verdicts.iter().map(|v| format!("{v}\n")).collect::<String>()).unwrap();
        let script = r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo '9.9.9 (Fake Claude)'; exit 0; fi
D='@D@'
ALLOWED=''; prev=''; for a in "$@"; do if [ "$prev" = "--allowedTools" ]; then ALLOWED="$a"; fi; prev="$a"; done
cat > "$D/last_prompt.txt"
case "$ALLOWED" in
  *Edit*)
    k=$(cat "$D/k" 2>/dev/null || echo 0); k=$((k+1)); echo $k > "$D/k"
    printf '%s\n' "$@" > "$D/call_args_$k.txt"; cp "$D/last_prompt.txt" "$D/call_prompt_$k.txt"
    if [ -s "$D/lost" ] && printf '%s\n' "$@" | grep -qx -- '--resume'; then
      rm "$D/lost"; echo "lost-session" >> "$D/log.txt"
      echo "No conversation found with session ID: gone" >&2; exit 1
    fi
    if [ -s "$D/quota" ]; then
      rm "$D/quota"; echo "quota-hit" >> "$D/log.txt"
      printf '{"type":"result","is_error":true,"result":"Claude AI usage limit reached|%s"}' "$(( $(date +%s) + 2 ))"; exit 1
    fi
    n=$(cat "$D/n" 2>/dev/null || echo 0); n=$((n+1)); echo $n > "$D/n"
    echo "coder $n start $(pwd)" >> "$D/log.txt"
    cp "$D/last_prompt.txt" "$D/coder_prompt_$n.txt"
    ls > "$D/files_$n.txt"
    printf '%s\n' "$@" > "$D/coder_args_$n.txt"
    prev=''; for a in "$@"; do if [ "$prev" = "--add-dir" ]; then cp "$a/SKILL.md" "$D/skill_seen_$n.txt" 2>/dev/null; echo tampered >> "$a/SKILL.md"; fi; prev="$a"; done
    prev=''; for a in "$@"; do if [ "$prev" = "--mcp-config" ]; then cp "$a" "$D/mcp_seen_$n.json" 2>/dev/null; fi; prev="$a"; done
    echo "$MCP_TOOL_TIMEOUT" > "$D/mcp_env_$n.txt"
    sleep @SECS@
    echo "change $n" >> "work-$n.txt"
    echo "change $n pid $$" > shared.txt
    echo "coder $n end" >> "$D/log.txt"
    printf '{"type":"result","is_error":false,"result":"做完了"}'
    ;;
  *Bash*)
    k=$(cat "$D/vk" 2>/dev/null || echo 0); k=$((k+1)); echo $k > "$D/vk"
    echo "verify $k $(pwd)" >> "$D/log.txt"
    printf '%s\n' "$@" > "$D/verify_args_$k.txt"
    cp "$D/last_prompt.txt" "$D/verify_prompt_$k.txt"
    ls > "$D/verify_files_$k.txt"
    JSON=$(grep -o '：/[^ ]*verify\.json' "$D/last_prompt.txt" | head -1 | sed 's/^：//')
    PORT=$(grep -o '伺服器只能用這些埠：[0-9]*' "$D/last_prompt.txt" | grep -o '[0-9]*$')
    if [ -s "$D/listen" ]; then
      python3 -c "import socket,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',$PORT));s.listen(1);time.sleep(120)" >/dev/null 2>&1 &
      echo $! > "$D/listener_pid"; echo "$PORT" > "$D/listener_port"; sleep 0.6
    fi
    if [ -f "$D/vj_$k.json" ]; then mkdir -p "$(dirname "$JSON")"; cp "$D/vj_$k.json" "$JSON"; fi
    if [ -s "$D/denied" ]; then printf '{"type":"result","is_error":false,"result":"驗收完成","permission_denials":[{"tool_name":"Bash","tool_input":{"command":"git push origin main"}}]}'; else printf '{"type":"result","is_error":false,"result":"驗收完成"}'; fi
    ;;
  *)
    if grep -q '拆解成可以分頭實作' "$D/last_prompt.txt"; then
      echo "planner $(pwd)" >> "$D/log.txt"
      cp "$D/last_prompt.txt" "$D/plan_prompt.txt"
      PLAN=$(cat "$D/plan.json" 2>/dev/null | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ')
      printf '{"type":"result","is_error":false,"result":"%s"}' "$PLAN"
      exit 0
    fi
    m=$(cat "$D/m" 2>/dev/null || echo 0); m=$((m+1)); echo $m > "$D/m"
    echo "reviewer $m $(pwd) $ALLOWED" >> "$D/log.txt"
    cp "$D/last_prompt.txt" "$D/review_prompt_$m.txt"
    printf '%s\n' "$@" > "$D/review_args_$m.txt"
    prev=''; for a in "$@"; do if [ "$prev" = "--add-dir" ]; then cp "$a/SKILL.md" "$D/skill_seen_review_$m.txt" 2>/dev/null; fi; prev="$a"; done
    v=$(head -1 "$D/verdicts"); sed -i.bak 1d "$D/verdicts"
    printf '{"type":"result","is_error":false,"result":"{\\"verdict\\":\\"%s\\",\\"reasons\\":[\\"缺測試\\"]}"}' "$v"
    ;;
esac
"#
        .replace("@D@", &dir.path().display().to_string())
        .replace("@SECS@", coder_secs);
        std::fs::write(dir.path().join("claude"), script).unwrap();
        sh(dir.path(), "chmod +x claude");
        Self { dir }
    }
    fn path(&self) -> String {
        self.dir.path().join("claude").to_string_lossy().to_string()
    }
    fn read(&self, f: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(f)).unwrap_or_default()
    }
    /// 讓下一次「接續（--resume）」的 Coder 呼叫回「找不到那段對話」（真實 Claude Code 只寫在 stderr、stdout 是空的）。
    fn lose_session_once(&self) {
        std::fs::write(self.dir.path().join("lost"), "1").unwrap();
    }
    /// 第 k 次 Coder 呼叫（含失敗的）收到的參數。
    fn coder_call_args(&self, k: usize) -> Vec<String> {
        self.read(&format!("call_args_{k}.txt")).lines().map(str::to_string).collect()
    }
    /// 第 k 次驗收要寫出的 verify.json（沒設就不寫，模擬驗收者忘了寫）。
    fn verify_json(&self, k: usize, json: &str) {
        std::fs::write(self.dir.path().join(format!("vj_{k}.json")), json).unwrap();
    }
    /// 驗收時在被指派的埠上啟動一個伺服器並留著不關（驗收結束後要被清掉）。
    fn leave_a_listener(&self) {
        std::fs::write(self.dir.path().join("listen"), "1").unwrap();
    }
    /// 驗收的輸出帶 `permission_denials`（模擬被禁止清單擋下 `git push`）。
    fn report_a_denial(&self) {
        std::fs::write(self.dir.path().join("denied"), "1").unwrap();
    }
    fn verify_args(&self, k: usize) -> Vec<String> {
        self.read(&format!("verify_args_{k}.txt")).lines().map(str::to_string).collect()
    }
    /// 讓下一次 Coder 呼叫回「額度用完」（重置時間 2 秒後）。
    fn hit_quota_once(&self) {
        std::fs::write(self.dir.path().join("quota"), "1").unwrap();
    }
    fn set_plan(&self, json: &str) {
        std::fs::write(self.dir.path().join("plan.json"), json).unwrap();
    }
    fn log(&self) -> Vec<String> {
        self.read("log.txt").lines().map(str::to_string).collect()
    }
}

struct Rig {
    srv: TestServer,
    admin: Api,
    pid: String,
    work: tempfile::TempDir,
    coder: bs::BoardAgent,
    reviewer: bs::BoardAgent,
}

async fn agent(srv: &TestServer, pid: &str, kind: &str, name: &str, prefix: &str) -> bs::BoardAgent {
    bs::create_agent(&srv.state().pools.intake, NewAgent { project_id: pid, kind, name, role: "", prompt_prefix: prefix, canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude", created_by: "t" })
        .await
        .unwrap()
        .unwrap()
}

async fn rig(fake: &Fake) -> Rig {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let pid = srv.project("範例科技").await;
    let work = repo();
    sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(work.path().to_string_lossy().to_string()).bind(&pid).execute(&srv.state().pools.server).await.unwrap();
    admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": fake.path(), "max_concurrent": 2, "timeout_secs": 60, "max_turns": 7}))).await.unwrap();
    let coder = agent(&srv, &pid, "coder", "工程師1", "你是資深工程師").await;
    let mut reviewer = agent(&srv, &pid, "reviewer", "審核者", "嚴格審查").await;
    sqlx::query("UPDATE board_agents SET model = 'sonnet' WHERE id = ?").bind(&reviewer.id).execute(&srv.state().pools.intake).await.unwrap();
    reviewer.model = Some("sonnet".into());
    Rig { srv, admin, pid, work, coder, reviewer }
}

impl Rig {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.srv.state().pools.intake
    }
    async fn enable(&self) {
        self.admin.request(Method::PUT, &format!("/v1/board/projects/{}/orchestration", self.pid), Some(json!({"enabled": true}))).await.unwrap();
    }
    /// 建一張指派給 Coder 的模組卡並排進待派送。
    async fn module(&self, title: &str, body: &str) -> BoardCard {
        let c = bs::create_card(self.pool(), NewCard { project_id: &self.pid, title, body, parallel_ok: true, interactive: false, label: None, created_by: "小華" }).await.unwrap();
        let Change::Ok(c) = bs::assign_card(self.pool(), &c.id, c.version, Some(&self.coder.id)).await.unwrap() else { panic!("指派失敗") };
        let Change::Ok(c) = bs::move_card(self.pool(), &c.id, c.version, "queued", None, None).await.unwrap() else { panic!("排入失敗") };
        *c
    }
    async fn cards(&self) -> Vec<BoardCard> {
        bs::project_cards(self.pool(), &self.pid).await.unwrap()
    }
    async fn kind(&self, kind: &str) -> Vec<BoardCard> {
        self.cards().await.into_iter().filter(|c| c.card_kind == kind).collect()
    }
    /// 反覆觸發編排，直到條件成立。
    async fn pump(&self, what: &str, until: impl Fn(&[BoardCard]) -> bool) {
        for _ in 0..600 {
            orchestrator::tick_once(self.srv.state()).await;
            if until(&self.cards().await) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("等不到「{what}」：{:#?}", self.cards().await.iter().map(|c| (&c.card_kind, &c.status, &c.outcome, &c.verdict, &c.error_message)).collect::<Vec<_>>());
    }
    fn branch(&self) -> String {
        orchestrator::agent_branch(&self.coder.id)
    }
    fn work_path(&self) -> PathBuf {
        self.work.path().to_path_buf()
    }
}

fn done(cards: &[BoardCard], kind: &str, n: usize) -> bool {
    cards.iter().filter(|c| c.card_kind == kind && c.status == "done").count() >= n
}

// ---- 驗收者專用的輔助 ----

/// 有驗收者、並且開了「由 server 執行驗收」的專案。
async fn rig_v(fake: &Fake) -> (Rig, bs::BoardAgent) {
    let r = rig(fake).await;
    r.enable().await;
    let verifier = agent(&r.srv, &r.pid, "verifier", "驗收者", "").await;
    orchestrator::set_verify_enabled(r.pool(), &r.pid, true).await.unwrap();
    (r, verifier)
}

fn alive(pid: &str) -> bool {
    Command::new("kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status().unwrap().success()
}

fn pass_json(module_id: &str) -> String {
    json!({"verdict": "pass", "project_type": "web", "items": [
        {"criterion": "主要入口可用", "status": "pass", "evidence": ["evidence/home.png"]},
        {"criterion": "按開始倒數", "status": "pass", "module_id": module_id, "evidence": ["npm test: 12 passed"]}
    ]})
    .to_string()
}

fn fail_json(module_id: &str) -> String {
    json!({"verdict": "fail", "project_type": "web", "items": [
        {"criterion": "按開始倒數", "status": "fail", "module_id": module_id, "steps": "按下開始", "expected": "25:00", "actual": "24:58"}
    ]})
    .to_string()
}

fn manual_json(module_id: &str) -> String {
    json!({"verdict": "pass", "items": [
        {"criterion": "按開始倒數", "status": "pass", "module_id": module_id, "evidence": ["x"]},
        {"criterion": "原生通知", "status": "manual", "notes": "需要真的桌面通知，無頭環境看不到"}
    ]})
    .to_string()
}

impl Rig {
    async fn verify_cards(&self) -> Vec<BoardCard> {
        bs::verify_cards(self.pool(), &self.pid).await.unwrap()
    }
    /// 反覆觸發編排，直到第 n 張驗收卡結案。
    async fn pump_verify(&self, n: usize) {
        for _ in 0..800 {
            orchestrator::tick_once(self.srv.state()).await;
            if self.verify_cards().await.iter().filter(|c| c.status == "done").count() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        panic!("等不到第 {n} 張驗收卡結案：{:#?}", self.cards().await.iter().map(|c| (&c.card_kind, &c.status, &c.outcome, &c.verdict, &c.error_message)).collect::<Vec<_>>());
    }
    /// 再多掃幾次，確認「沒有」發生某事（例如沒有出現合併卡）。
    async fn settle(&self) {
        for _ in 0..12 {
            orchestrator::tick_once(self.srv.state()).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
    fn integration_worktrees(&self) -> Vec<String> {
        std::fs::read_dir(self.srv.state().data_dir.join("worktrees")).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with("verify-")).collect()).unwrap_or_default()
    }
}

// ---- 測試 ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_verification_on_the_merge_card_appears_only_after_a_passing_verification() {
    let fake = Fake::new(&["pass"], "0");
    let (r, verifier) = rig_v(&fake).await;
    let m = r.module("登入頁", "做一個登入頁").await;
    fake.verify_json(1, &manual_json(&m.id));
    r.pump_verify(1).await;
    // 驗收進行當下與剛結案時：還沒有合併卡（要等下一個 tick 看到通過才建）。
    r.pump("合併待核准的卡出現", |c| c.iter().any(|x| x.card_kind == "merge")).await;

    let v = r.verify_cards().await.remove(0);
    assert_eq!((v.agent_id.as_deref(), v.status.as_str(), v.outcome.as_deref(), v.verdict.as_deref()), (Some(verifier.id.as_str()), "done", Some("success"), Some("pass")));
    // 驗收模式：Bash 但沒有編輯工具、在整合 worktree（不是專案資料夾、不是 Coder 的）、禁止清單帶進命令列。
    let args = fake.verify_args(1);
    let after = |f: &str| args.iter().position(|a| a == f).map(|i| args[i + 1].clone()).unwrap_or_default();
    assert!(after("--allowedTools").starts_with("Read,Grep,Glob,Bash"), "{args:?}");
    assert!(!after("--allowedTools").contains("Edit"));
    let disallowed = after("--disallowedTools");
    assert!(disallowed.contains("Edit") && disallowed.contains("Bash(git push:*)") && disallowed.contains("Bash(sudo:*)"), "{disallowed}");
    let log = fake.log();
    let line = log.iter().find(|l| l.starts_with("verify 1 ")).unwrap();
    assert!(line.contains("worktrees/verify-") && !line.contains("agent-") && !line.ends_with(&r.work_path().display().to_string()), "{line}");
    // 整合 worktree 裡真的有工程師的成果（各分支併進來了）。
    assert!(fake.read("verify_files_1.txt").contains("work-1.txt"), "{}", fake.read("verify_files_1.txt"));
    // 提示詞：模組 id 與標題、不提 ask_human。
    let p = fake.read("verify_prompt_1.txt");
    assert!(p.contains(&format!("[{}] 登入頁", m.id)) && p.contains("做一個登入頁") && !p.contains("ask_human"), "{p}");
    // 報告存進資料庫；N 項需確認寫在合併卡上。
    let meta = bs::get_verify(r.pool(), &v.id).await.unwrap().unwrap();
    assert_eq!(meta.manual_count, 1);
    assert!(meta.report.unwrap().contains("原生通知"));
    assert_eq!(meta.ports.len(), 3);
    let merge = r.kind("merge").await.remove(0);
    assert_eq!(merge.status, "planning");
    assert!(merge.body.contains("也通過了驗收（1 項需要你確認）"), "{}", merge.body);
    // 專案資料夾一個字都沒動，整合 worktree 清掉了。
    assert_eq!(sh(&r.work_path(), "git log --format=%s").trim(), "init");
    assert!(r.integration_worktrees().is_empty(), "{:?}", r.integration_worktrees());
    assert_eq!(bs::get_session(r.pool(), &v.id).await.unwrap(), None, "對話記錄清掉");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_verification_sends_the_failing_module_back_and_verifies_again_after_the_fix() {
    let fake = Fake::new(&["pass", "pass"], "0");
    let (r, _v) = rig_v(&fake).await;
    let m = r.module("計時器", "按開始倒數 25 分鐘").await;
    fake.verify_json(1, &fail_json(&m.id));
    fake.verify_json(2, &pass_json(&m.id));
    r.pump("合併待核准的卡出現", |c| c.iter().any(|x| x.card_kind == "merge")).await;

    // 退回：vfix 卡給原工程師，掛在通過的審查後面，內文帶重現步驟。
    let vfix = r.kind("vfix").await;
    assert_eq!(vfix.len(), 1, "只開一張");
    let (review, vf) = (r.kind("review").await.remove(0), &vfix[0]);
    assert_eq!((vf.agent_id.as_deref(), vf.module_id.as_deref(), vf.review_of.as_deref(), vf.status.as_str()), (Some(r.coder.id.as_str()), Some(m.id.as_str()), Some(review.id.as_str()), "done"));
    assert!(vf.body.contains("按下開始") && vf.body.contains("25:00") && vf.body.contains("24:58") && vf.body.contains("第 1 輪"), "{}", vf.body);
    // 工程師真的拿到那段提示詞，用的是有編輯工具的路徑。
    assert!(fake.read("call_prompt_2.txt").contains("驗收沒有通過") && fake.read("call_args_2.txt").contains("Edit"));
    // 修好後重新審查、重新驗收（第二張驗收卡），這次通過才有合併卡。
    assert_eq!(r.verify_cards().await.len(), 2);
    assert_eq!(r.kind("review").await.len(), 2, "vfix 也要審查");
    let vs = r.verify_cards().await;
    assert_eq!((vs[0].verdict.as_deref(), vs[1].verdict.as_deref()), (Some("fail"), Some("pass")));
    assert_eq!(r.kind("merge").await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failures_without_a_known_module_stop_for_a_human_and_open_no_fix_card() {
    let fake = Fake::new(&["pass"], "0");
    let (r, _v) = rig_v(&fake).await;
    let _m = r.module("計時器", "x").await;
    fake.verify_json(1, &fail_json("不存在的模組"));
    r.pump_verify(1).await;
    r.settle().await;
    let v = r.verify_cards().await.remove(0);
    assert_eq!(v.verdict.as_deref(), Some("attention"), "找不到模組：停住等管理員");
    assert!(r.kind("vfix").await.is_empty() && r.kind("merge").await.is_empty());
    assert_eq!(r.verify_cards().await.len(), 1, "不會一直重建驗收卡");
    // 管理員放行：視為通過，這時才有合併卡。
    r.admin.request(Method::POST, &format!("/v1/board/cards/{}/verify/override", v.id), None).await.unwrap();
    r.pump("放行後合併待核准的卡出現", |c| c.iter().any(|x| x.card_kind == "merge")).await;
    let merge = r.kind("merge").await.remove(0);
    assert!(merge.body.contains("驗收由管理員「小華」放行"), "{}", merge.body);
}

/// 全部條件都標成 manual（驗收者什麼都沒能實際驗證）不能算通過：停下來等管理員，不會有合併卡。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_report_where_nothing_was_really_verified_needs_a_human_instead_of_passing() {
    let fake = Fake::new(&["pass"], "0");
    let (r, _v) = rig_v(&fake).await;
    let m = r.module("計時器", "x").await;
    fake.verify_json(1, &json!({"verdict": "pass", "items": [{"criterion": "按開始倒數", "status": "manual", "module_id": m.id, "notes": "伺服器被禁止清單擋下，無法啟動"}]}).to_string());
    r.pump_verify(1).await;
    r.settle().await;
    let v = r.verify_cards().await.remove(0);
    assert_eq!((v.outcome.as_deref(), v.verdict.as_deref()), (Some("success"), Some("attention")));
    assert!(v.ai_summary.as_deref().unwrap_or("").contains("沒能實際驗證任何條件"), "{:?}", v.ai_summary);
    assert!(r.kind("merge").await.is_empty() && r.kind("vfix").await.is_empty(), "不能放行，也沒有東西可退回");
    let meta = bs::get_verify(r.pool(), &v.id).await.unwrap().unwrap();
    assert_eq!(meta.manual_count, 1);
    assert!(meta.report.is_some(), "報告照樣存下來，管理員才看得到 manual 的原因");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retried_verification_runs_again_and_a_pass_then_opens_the_merge_card() {
    let fake = Fake::new(&["pass"], "0");
    let (r, _v) = rig_v(&fake).await;
    let m = r.module("計時器", "x").await;
    // 第一次驗收者忘了寫 verify.json → 失敗；管理員重新驗收，第二次寫好了。
    r.pump_verify(1).await;
    let first = r.verify_cards().await.remove(0);
    assert_eq!(first.outcome.as_deref(), Some("failed"));
    r.settle().await;
    assert!(r.kind("merge").await.is_empty());
    fake.verify_json(2, &pass_json(&m.id));
    r.admin.request(Method::POST, &format!("/v1/board/cards/{}/verify/retry", first.id), None).await.unwrap();
    r.pump("重新驗收通過後合併待核准的卡出現", |c| c.iter().any(|x| x.card_kind == "merge")).await;
    let vs = r.verify_cards().await;
    assert_eq!(vs.len(), 2);
    assert_eq!((vs[1].outcome.as_deref(), vs[1].verdict.as_deref()), (Some("success"), Some("pass")));
}

/// server 被強制結束時，驗收啟動的伺服器還佔著埠：重啟時依記錄的埠收掉，別的行程不動。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_cleanup_stops_leftover_verification_servers_on_the_recorded_ports_only() {
    let fake = Fake::new(&["pass"], "0");
    let r = rig(&fake).await;
    let listen = |port: u16| {
        Command::new("python3")
            .args(["-c", &format!("import socket,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',{port}));s.listen(1);time.sleep(60)")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let ports = aiterm_intake_server::verify_env::pick_free_ports(2);
    let (mut recorded, mut other) = (listen(ports[0]), listen(ports[1]));
    let c = bs::create_workflow_card(r.pool(), bs::NewWorkflowCard { project_id: &r.pid, title: "驗收", body: "", kind: "verify", agent_id: None, module_id: None, review_of: None, status: "running", depends_on: None, from_spec: None }).await.unwrap();
    bs::set_verify_meta(r.pool(), &c.id, "s#0", &[ports[0]]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    aiterm_intake_server::verifier::cleanup_stale(r.srv.state()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(recorded.try_wait().unwrap().is_some(), "記錄過的埠上的行程被結束");
    assert!(other.try_wait().unwrap().is_none(), "沒記錄的埠上的行程不動");
    other.kill().ok();
    other.wait().ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_two_fix_rounds_a_third_failure_stops_for_a_human() {
    let fake = Fake::new(&["pass", "pass", "pass"], "0");
    let (r, _v) = rig_v(&fake).await;
    let m = r.module("計時器", "x").await;
    for k in 1..=3 {
        fake.verify_json(k, &fail_json(&m.id));
    }
    r.pump_verify(3).await;
    r.settle().await;
    assert_eq!(r.kind("vfix").await.len(), 2, "最多自動修正兩輪");
    let vfix = r.kind("vfix").await;
    assert!(vfix.iter().any(|c| c.body.contains("第 1 輪")) && vfix.iter().any(|c| c.body.contains("第 2 輪")));
    assert!(r.kind("merge").await.is_empty(), "沒有通過就不會有合併卡");
    assert_eq!(r.verify_cards().await.len(), 3, "第三次失敗後不再建新的");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_merge_conflict_in_the_integration_worktree_fails_the_verification_with_the_reason_and_touches_nothing() {
    let fake = Fake::new(&["pass", "pass"], "0");
    let (r, _v) = rig_v(&fake).await;
    // 第二位工程師改同一個檔案（假 claude 每次都寫 shared.txt）→ 兩條分支合併必衝突。
    let coder2 = agent(&r.srv, &r.pid, "coder", "工程師2", "").await;
    let _m1 = r.module("甲", "x").await;
    let c = bs::create_card(r.pool(), NewCard { project_id: &r.pid, title: "乙", body: "y", parallel_ok: true, interactive: false, label: None, created_by: "t" }).await.unwrap();
    let Change::Ok(c) = bs::assign_card(r.pool(), &c.id, c.version, Some(&coder2.id)).await.unwrap() else { panic!() };
    let Change::Ok(_) = bs::move_card(r.pool(), &c.id, c.version, "queued", None, None).await.unwrap() else { panic!() };
    r.pump_verify(1).await;
    r.settle().await;
    let v = r.verify_cards().await.remove(0);
    assert_eq!((v.outcome.as_deref(), v.status.as_str()), (Some("failed"), "done"));
    assert!(v.error_message.as_deref().unwrap_or("").contains("合併衝突"), "{:?}", v.error_message);
    assert!(r.kind("merge").await.is_empty() && r.kind("vfix").await.is_empty());
    assert_eq!(sh(&r.work_path(), "git log --format=%s").trim(), "init", "專案資料夾沒動");
    assert!(!sh(&r.work_path(), "git branch --list 'agent/*'").trim().is_empty(), "工程師的分支還在");
    assert!(r.integration_worktrees().is_empty());
    assert_eq!(fake.read("vk").trim(), "", "衝突時根本沒有叫 Claude");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_broken_or_evidence_free_verify_json_fails_the_card_and_never_passes() {
    let fake = Fake::new(&["pass"], "0");
    let (r, _v) = rig_v(&fake).await;
    let m = r.module("計時器", "x").await;
    // 第一次：沒寫檔案。
    r.pump_verify(1).await;
    let v1 = r.verify_cards().await.remove(0);
    assert_eq!(v1.outcome.as_deref(), Some("failed"));
    assert!(v1.error_message.as_deref().unwrap_or("").contains("沒有寫出 verify.json"), "{:?}", v1.error_message);
    assert!(r.kind("merge").await.is_empty());
    // 壞掉的 JSON 與沒有證據的 pass，也都是失敗：用另外兩個專案各驗一次太重，直接驗 core 的判定已涵蓋；這裡再確認 pass 沒證據走到 server 也失敗。
    let _ = m;
    let fake2 = Fake::new(&["pass"], "0");
    let (r2, _v2) = rig_v(&fake2).await;
    let m2 = r2.module("計時器", "x").await;
    fake2.verify_json(1, &json!({"verdict": "pass", "items": [{"criterion": "按開始倒數", "status": "pass", "module_id": m2.id}]}).to_string());
    r2.pump_verify(1).await;
    let v = r2.verify_cards().await.remove(0);
    assert_eq!(v.outcome.as_deref(), Some("failed"));
    assert!(v.error_message.as_deref().unwrap_or("").contains("沒有附證據"), "{:?}", v.error_message);
    assert!(r2.kind("merge").await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_deny_list_and_ports_reach_the_command_line_denials_are_reported_and_listeners_are_cleaned() {
    let fake = Fake::new(&["pass"], "0");
    fake.leave_a_listener();
    fake.report_a_denial();
    let (r, _v) = rig_v(&fake).await;
    // 專案覆寫：新增一條、移除一條預設。
    bs::put_deny_override(r.pool(), &r.pid, "Bash(make deploy:*)", "add", true).await.unwrap();
    bs::put_deny_override(r.pool(), &r.pid, "Bash(docker push:*)", "remove", true).await.unwrap();
    let m = r.module("計時器", "x").await;
    fake.verify_json(1, &pass_json(&m.id));
    r.pump_verify(1).await;
    let args = fake.verify_args(1);
    let disallowed = args.iter().position(|a| a == "--disallowedTools").map(|i| args[i + 1].clone()).unwrap();
    assert!(disallowed.contains("Bash(make deploy:*)") && !disallowed.contains("Bash(docker push:*)") && disallowed.contains("Bash(git push:*)"), "{disallowed}");
    // 被擋下的命令記在報告上，給管理員判斷是不是誤擋。
    let v = r.verify_cards().await.remove(0);
    let meta = bs::get_verify(r.pool(), &v.id).await.unwrap().unwrap();
    assert_eq!(meta.denied, vec!["git push origin main".to_string()]);
    // 驗收者留下的伺服器：就在被指派的埠上，驗收結束後被結束；別的行程不動。
    let port: u16 = fake.read("listener_port").trim().parse().unwrap();
    assert!(meta.ports.contains(&port), "伺服器用的是 server 指派的埠：{port} {:?}", meta.ports);
    let pid = fake.read("listener_pid").trim().to_string();
    for _ in 0..40 {
        if !alive(&pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!alive(&pid), "佔埠的行程被結束了");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_verification_enabled_or_without_a_verifier_the_flow_is_unchanged() {
    // 有驗收者但沒開驗收。
    let fake = Fake::new(&["pass"], "0");
    let r = rig(&fake).await;
    r.enable().await;
    let _verifier = agent(&r.srv, &r.pid, "verifier", "驗收者", "").await;
    let _m = r.module("登入頁", "x").await;
    r.pump("合併待核准的卡出現", |c| c.iter().any(|x| x.card_kind == "merge")).await;
    assert!(r.verify_cards().await.is_empty() && fake.read("vk").trim().is_empty());
    assert!(!r.kind("merge").await[0].body.contains("驗收"), "沒驗收就不提驗收");
    // 開了驗收但畫布上沒有驗收者。
    let fake2 = Fake::new(&["pass"], "0");
    let r2 = rig(&fake2).await;
    r2.enable().await;
    orchestrator::set_verify_enabled(r2.pool(), &r2.pid, true).await.unwrap();
    let _m2 = r2.module("登入頁", "x").await;
    r2.pump("合併待核准的卡出現", |c| c.iter().any(|x| x.card_kind == "merge")).await;
    assert!(r2.verify_cards().await.is_empty());
}

// ---- 技能 ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agents_skills_reach_claude_as_a_throwaway_copy_and_never_touch_the_shared_library() {
    use base64::Engine;
    let fake = Fake::new(&["pass"], "0");
    let r = rig(&fake).await;
    r.enable().await;
    let b64 = |t: &str| base64::engine::general_purpose::STANDARD.encode(t.as_bytes());
    let skill = r.admin.request(Method::POST, "/v1/skills", Some(json!({"folder_name": "x", "files": [
        {"path": "SKILL.md", "data": b64("---\nname: 寫測試\ndescription: 先寫紅燈再實作\n---\n暗號：香蕉")},
        {"path": "scripts/check.sh", "data": b64("echo ok")}
    ]}))).await.unwrap();
    let ids = json!({"ids": [skill["id"]]});
    r.admin.request(Method::PUT, &format!("/v1/board/agents/{}/skills", r.coder.id), Some(ids.clone())).await.unwrap();
    r.admin.request(Method::PUT, &format!("/v1/board/agents/{}/skills", r.reviewer.id), Some(ids)).await.unwrap();
    r.module("登入頁", "做一個登入頁").await;
    r.pump("審查結案", |c| done(c, "review", 1)).await;

    // 工程師（work 卡）：--add-dir 指到一次性副本，提示詞附清單，Claude 看得到技能內容。
    let args: Vec<String> = fake.read("coder_args_1.txt").lines().map(str::to_string).collect();
    let dir = args.iter().position(|a| a == "--add-dir").map(|i| args[i + 1].clone()).expect("要有 --add-dir");
    assert!(dir.contains("/run/") && dir.ends_with("/skills/寫測試"), "{dir}");
    let prompt = fake.read("coder_prompt_1.txt");
    assert!(prompt.contains("寫測試") && prompt.contains("先寫紅燈再實作") && prompt.contains(&format!("{dir}/SKILL.md")), "{prompt}");
    assert!(fake.read("skill_seen_1.txt").contains("暗號：香蕉"), "Claude 讀得到技能內容");
    // 審查者（唯讀卡）也一樣。
    assert!(fake.read("review_prompt_1.txt").contains("寫測試"), "審查提示詞也有清單");
    assert!(fake.read("skill_seen_review_1.txt").contains("暗號：香蕉"));
    // Agent 改了副本（假 claude 故意亂寫）：共用技能庫的檔案不受影響；卡結束後副本整個刪掉。
    let lib = r.srv.dir.path().join("skills").join(skill["id"].as_str().unwrap()).join("SKILL.md");
    let text = std::fs::read_to_string(&lib).unwrap();
    assert!(text.contains("暗號：香蕉") && !text.contains("tampered"), "{text}");
    assert!(!r.srv.dir.path().join("run").exists() || std::fs::read_dir(r.srv.dir.path().join("run")).unwrap().next().is_none(), "副本要刪掉");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_without_skills_gets_no_add_dir_and_no_skill_note() {
    let fake = Fake::new(&["pass"], "0");
    let r = rig(&fake).await;
    r.enable().await;
    r.module("登入頁", "做一個登入頁").await;
    r.pump("審查結案", |c| done(c, "review", 1)).await;
    assert!(!fake.read("coder_args_1.txt").lines().any(|a| a == "--add-dir"));
    assert!(!fake.read("coder_prompt_1.txt").contains("你有這些技能可以用"));
    assert!(!fake.read("coder_args_1.txt").lines().any(|a| a == "--mcp-config" || a == "--strict-mcp-config"), "沒裝資料庫技能就沒有 MCP 設定");
    assert!(!fake.read("coder_args_1.txt").contains("mcp__aiterm-db"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_db_skill_gives_claude_a_per_card_mcp_endpoint_that_dies_with_the_card() {
    let fake = Fake::new(&["pass"], "0");
    let r = rig(&fake).await;
    r.enable().await;
    let db = r.srv.dir.path().join("app.db").to_string_lossy().to_string();
    std::fs::File::create(&db).unwrap();
    let conn = r.admin.request(Method::POST, "/v1/db/connections", Some(json!({"name": "正式機", "db_type": "sqlite", "host": db}))).await.unwrap();
    r.admin.request(Method::PUT, &format!("/v1/board/agents/{}/db-skill", r.coder.id), Some(json!({"skill": {"connections": [conn["id"]], "mode": "approve"}}))).await.unwrap();
    r.module("登入頁", "做一個登入頁").await;
    r.pump("審查結案", |c| done(c, "review", 1)).await;

    let args: Vec<String> = fake.read("coder_args_1.txt").lines().map(str::to_string).collect();
    let after = |f: &str| args.iter().position(|a| a == f).map(|i| args[i + 1].clone()).unwrap_or_default();
    assert!(after("--allowedTools").contains("mcp__aiterm-db") && after("--allowedTools").starts_with("Read,Grep,Glob,Edit"), "{args:?}");
    assert!(args.iter().any(|a| a == "--strict-mcp-config"), "不載入使用者自己的 MCP 設定：{args:?}");
    assert_eq!(fake.read("mcp_env_1.txt").trim(), "86400000", "等核准時工具呼叫才不會逾時");
    // 設定檔指向本機的 MCP 端點；提示詞說明可用的連線與權限。
    let cfg: serde_json::Value = serde_json::from_str(&fake.read("mcp_seen_1.json")).unwrap();
    let url = cfg["mcpServers"]["aiterm-db"]["url"].as_str().unwrap().to_string();
    assert!(url.starts_with("http://127.0.0.1:") && url.contains("/mcp/"), "{url}");
    let prompt = fake.read("coder_prompt_1.txt");
    assert!(prompt.contains("db_query") && prompt.contains("正式機") && prompt.contains("核准"), "{prompt}");
    // 卡結束後 token 作廢；設定檔隨一次性資料夾刪除。
    let resp = reqwest::Client::new().post(&url).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).send().await.unwrap();
    assert_eq!(resp.status(), 401);
    assert!(!r.srv.dir.path().join("run").exists() || std::fs::read_dir(r.srv.dir.path().join("run")).unwrap().next().is_none());
}

