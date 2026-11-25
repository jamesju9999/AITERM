#![cfg(unix)] // 假的 claude 是 shell 腳本，Windows 無法當成程式執行；執行器本身在 Windows 要把 claude_path 設成 claude.exe 的完整路徑。
//! Server 自動處理題目：用假的 `claude`（可設定路徑的腳本）驅動，驗證工具參數、模式、worktree、
//! 驗證、核准合併、駁回、衝突、逾時、並行上限與「claude 不可用就不領題」。
use aiterm_intake_client::api::Api;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::config::Config;
use aiterm_intake_core::rules::Level;
use aiterm_intake_core::store::{self, IntakeItem, NewItem};
use aiterm_intake_server::runner;
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

/// 一個有一次提交的 git 專案。
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    sh(d.path(), "git init -q -b main && git config user.email a@b.c && git config user.name t && echo v1 > app.txt && git add -A && git commit -qm init");
    d
}

struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    /// `behavior` 是在工作資料夾裡執行的 shell 片段；`result` 是回給 server 的最終回答。
    fn new(behavior: &str, result: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().display();
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '9.9.9 (Fake Claude)'; exit 0; fi\nprintf '%s\\n' \"$@\" > '{d}/args.txt'\nALLOWED=''; prev=''; for a in \"$@\"; do if [ \"$prev\" = \"--allowedTools\" ]; then ALLOWED=\"$a\"; fi; prev=\"$a\"; done\npwd > '{d}/cwd.txt'\ncat > '{d}/prompt.txt'\n{behavior}\nprintf '{{\"type\":\"result\",\"is_error\":false,\"result\":\"{result}\"}}'\n"
        );
        let path = dir.path().join("claude");
        std::fs::write(&path, script).unwrap();
        sh(dir.path(), "chmod +x claude");
        Self { dir }
    }
    fn path(&self) -> String {
        self.dir.path().join("claude").to_string_lossy().to_string()
    }
    fn read(&self, f: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(f)).unwrap_or_default()
    }
    fn args(&self) -> Vec<String> {
        self.read("args.txt").lines().map(str::to_string).collect()
    }
    fn arg_after(&self, flag: &str) -> String {
        let a = self.args();
        a.iter().position(|x| x == flag).and_then(|i| a.get(i + 1).cloned()).unwrap_or_default()
    }
}

struct Rig {
    srv: TestServer,
    admin: Api,
    pid: String,
    work: tempfile::TempDir,
}

async fn rig(fake: &Fake, auto: bool, verify_command: &str) -> Rig {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let pid = srv.project("範例科技").await;
    let work = repo();
    sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(work.path().to_string_lossy().to_string()).bind(&pid).execute(&srv.state().pools.server).await.unwrap();
    let cfg = Config { enabled: true, auto_process: auto, verify_command: verify_command.into(), ..Config::default() };
    admin.request(Method::PUT, &format!("/v1/projects/{pid}/config"), Some(json!({"config": cfg, "telegram_default": false}))).await.unwrap();
    admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": fake.path(), "max_concurrent": 1, "timeout_secs": 60, "max_turns": 7}))).await.unwrap();
    Rig { srv, admin, pid, work }
}

impl Rig {
    async fn item(&self, subject: &str, source: &str, level: Level) -> IntakeItem {
        store::insert_item(&self.srv.state().pools.intake, NewItem { project_id: &self.pid, id: None, source, sender: "line:u0123456789abcdef0123456789abcdef", level, subject, body: "請幫我看", message_id: subject, status: store::QUEUED })
            .await
            .unwrap()
            .unwrap()
    }
    async fn get(&self, id: &str) -> IntakeItem {
        store::get_item(&self.srv.state().pools.intake, id).await.unwrap().unwrap()
    }
    /// 等題目離開「處理中」。
    async fn settle(&self, id: &str) -> IntakeItem {
        for _ in 0..200 {
            let it = self.get(id).await;
            if it.status != store::RUNNING && it.status != store::QUEUED {
                return it;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("等不到結果：{:?}", self.get(id).await.status);
    }
    fn work_path(&self) -> PathBuf {
        self.work.path().to_path_buf()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_only_item_runs_claude_with_only_read_tools_in_the_project_folder_and_becomes_a_redacted_draft() {
    let fake = Fake::new("", "請看 /Users/someone/proj 與 sk-abcdefghijklmnop1234，重開機再試");
    let r = rig(&fake, true, "").await;
    let it = r.item("電腦很慢", "line", Level::Internal).await;
    assert_eq!(runner::tick_once(r.srv.state()).await, 1);
    let done = r.settle(&it.id).await;
    assert_eq!((done.status.as_str(), done.claimed_by.as_deref()), (store::DRAFT, Some("server")));
    let draft = done.reply_draft.unwrap();
    assert!(draft.contains("重開機再試") && !draft.contains("someone") && !draft.contains("sk-abcdef"), "回覆要去敏：{draft}");
    assert_eq!(done.branch, None);

    assert_eq!(fake.arg_after("--allowedTools"), "Read,Grep,Glob", "唯讀只給讀檔與搜尋");
    let denied = fake.arg_after("--disallowedTools");
    assert!(["Bash", "Edit", "Write"].iter().all(|t| denied.split(',').any(|d| d == *t)), "要明確禁用改檔案與執行指令：{denied}");
    assert_eq!((fake.arg_after("--permission-mode").as_str(), fake.arg_after("--max-turns").as_str()), ("dontAsk", "7"));
    assert!(fake.args().contains(&"-p".to_string()) && fake.args().contains(&"--no-session-persistence".to_string()));
    assert_eq!(std::fs::canonicalize(fake.read("cwd.txt").trim()).unwrap(), std::fs::canonicalize(r.work_path()).unwrap(), "在專案的工作資料夾執行");
    assert!(fake.read("prompt.txt").contains("電腦很慢"), "提示詞走標準輸入，不放在參數裡");
    assert!(!fake.args().iter().any(|a| a.contains("電腦很慢")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_customers_unclassified_items_and_diagnose_only_channels_are_always_read_only() {
    // 假的 Claude 像真的一樣只在工具白名單含 Edit 時才改檔案：唯讀模式下它根本改不了。
    let fake = Fake::new("case \",$ALLOWED,\" in *,Edit,*) echo changed > app.txt;; esac", "好");
    let r = rig(&fake, true, "true").await;
    // email 預設可改檔案：外部客戶的題目、以及沒標種類又沒設 AI 的內部題目，仍然唯讀。
    let ext = r.item("外部客戶", "email", Level::External).await;
    store::set_kind(&r.srv.state().pools.intake, &ext.id, "code_change", None).await.unwrap();
    let unknown = r.item("沒標種類", "email", Level::Internal).await;
    let line = r.item("LINE 只診斷", "line", Level::Internal).await;
    store::set_kind(&r.srv.state().pools.intake, &line.id, "code_change", None).await.unwrap();
    for it in [&ext, &unknown, &line] {
        assert_eq!(runner::tick_once(r.srv.state()).await, 1);
        let done = r.settle(&it.id).await;
        assert_eq!((done.status.as_str(), done.branch.as_deref()), (store::DRAFT, None), "{}", it.subject);
        assert_eq!(fake.arg_after("--allowedTools"), "Read,Grep,Glob", "{} 必須唯讀", it.subject);
    }
    assert_eq!(sh(&r.work_path(), "cat app.txt").trim(), "v1", "唯讀模式不可能改到專案檔案");
    assert!(!r.srv.dir.path().join("worktrees").exists(), "唯讀不開 worktree");
}

async fn edit_item(r: &Rig) -> IntakeItem {
    let it = r.item("程式有 bug", "email", Level::Internal).await;
    store::set_kind(&r.srv.state().pools.intake, &it.id, "code_change", None).await.unwrap();
    it
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_code_change_is_edited_in_an_isolated_worktree_verified_and_waits_for_approval_without_touching_the_project() {
    let fake = Fake::new("echo changed > app.txt", "已修好");
    let r = rig(&fake, true, "true").await;
    let it = edit_item(&r).await;
    assert_eq!(runner::tick_once(r.srv.state()).await, 1);
    let done = r.settle(&it.id).await;
    assert_eq!((done.status.as_str(), done.merge_state.as_deref(), done.verified), (store::DRAFT, Some("pending"), Some(true)), "{:?}", done.error);
    let branch = done.branch.clone().unwrap();
    assert!(done.reply_draft.as_deref().unwrap().contains("已修好") && done.verification.as_deref().unwrap().contains("app.txt"));

    let allowed = fake.arg_after("--allowedTools");
    assert!(allowed.contains("Edit") && allowed.contains("Write") && allowed.contains("Bash(true)"), "可改檔案，且只多開放驗證指令：{allowed}");
    assert!(!allowed.split(',').any(|t| t == "Bash"), "不開放任意 Bash：{allowed}");
    let wt = r.srv.dir.path().join("worktrees").join(&it.id);
    assert_eq!(std::fs::canonicalize(fake.read("cwd.txt").trim()).unwrap(), std::fs::canonicalize(&wt).unwrap(), "在隔離的 worktree 執行");
    assert_eq!(sh(&r.work_path(), "cat app.txt").trim(), "v1", "核准之前專案本身不會被改");
    assert!(sh(&r.work_path(), &format!("git log {branch} --oneline")).contains("問題處理"), "改動 commit 在專屬分支上");
    assert!(sh(&r.work_path(), "git log main --oneline").lines().count() == 1, "主分支沒有新提交");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_an_admin_can_approve_and_approval_merges_the_branch_then_cleans_up() {
    let fake = Fake::new("echo changed > app.txt", "好");
    let r = rig(&fake, true, "true").await;
    let it = edit_item(&r).await;
    runner::tick_once(r.srv.state()).await;
    let done = r.settle(&it.id).await;
    let branch = done.branch.clone().unwrap();
    let member = r.srv.paired("小明", Role::Member).await;
    let path = format!("/v1/items/{}/merge", it.id);
    assert!(member.request(Method::POST, &path, Some(json!({"approve": true}))).await.is_err(), "成員不能核准合併");
    assert_eq!(sh(&r.work_path(), "cat app.txt").trim(), "v1");

    r.admin.request(Method::POST, &path, Some(json!({"approve": true}))).await.unwrap();
    assert_eq!(sh(&r.work_path(), "cat app.txt").trim(), "changed", "核准後合併進專案");
    let g = r.get(&it.id).await;
    assert_eq!(g.merge_state.as_deref(), Some("merged"));
    assert!(!r.srv.dir.path().join("worktrees").join(&it.id).exists(), "worktree 清掉");
    assert!(sh(&r.work_path(), "git branch --list").lines().all(|l| !l.contains(&branch)), "分支清掉");
    assert!(r.admin.request(Method::POST, &path, Some(json!({"approve": true}))).await.is_err(), "不能重複核准");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejecting_throws_the_change_away() {
    let fake = Fake::new("echo changed > app.txt", "好");
    let r = rig(&fake, true, "true").await;
    let it = edit_item(&r).await;
    runner::tick_once(r.srv.state()).await;
    let branch = r.settle(&it.id).await.branch.unwrap();
    r.admin.request(Method::POST, &format!("/v1/items/{}/merge", it.id), Some(json!({"approve": false}))).await.unwrap();
    assert_eq!(r.get(&it.id).await.merge_state.as_deref(), Some("rejected"));
    assert_eq!(sh(&r.work_path(), "cat app.txt").trim(), "v1");
    assert!(sh(&r.work_path(), "git branch --list").lines().all(|l| !l.contains(&branch)));
    assert!(!r.srv.dir.path().join("worktrees").join(&it.id).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conflicting_merge_is_aborted_cleanly_reported_and_can_be_retried() {
    let fake = Fake::new("echo agent > app.txt", "好");
    let r = rig(&fake, true, "true").await;
    let it = edit_item(&r).await;
    runner::tick_once(r.srv.state()).await;
    r.settle(&it.id).await;
    // 審稿的這段時間，主分支上有人改了同一行。
    sh(&r.work_path(), "echo human > app.txt && git commit -qam human");
    let res = r.admin.request(Method::POST, &format!("/v1/items/{}/merge", it.id), Some(json!({"approve": true}))).await;
    assert!(res.is_err(), "衝突要回報");
    let g = r.get(&it.id).await;
    assert_eq!(g.merge_state.as_deref(), Some("pending"), "退回等待核准，可以處理後再試");
    assert!(g.error.as_deref().unwrap_or("").contains("合併失敗"), "{:?}", g.error);
    assert_eq!(sh(&r.work_path(), "cat app.txt").trim(), "human", "專案維持原樣");
    assert!(sh(&r.work_path(), "git status --porcelain").trim().is_empty(), "沒有留下合併到一半的狀態");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_code_change_that_changes_nothing_is_a_draft_with_a_failed_verification_and_no_branch() {
    let fake = Fake::new("", "我看了但沒改");
    let r = rig(&fake, true, "true").await;
    let it = edit_item(&r).await;
    runner::tick_once(r.srv.state()).await;
    let done = r.settle(&it.id).await;
    assert_eq!((done.status.as_str(), done.verified, done.branch.as_deref()), (store::DRAFT, Some(false), None));
    assert!(done.verification.unwrap().contains("沒有偵測到任何檔案變更"));
    assert!(!r.srv.dir.path().join("worktrees").join(&it.id).exists(), "沒有改動就不留 worktree");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn at_most_the_configured_number_of_items_run_at_once() {
    let fake = Fake::new("sleep 2", "好");
    let r = rig(&fake, true, "").await;
    let (a, b) = (r.item("一", "line", Level::Internal).await, r.item("二", "line", Level::Internal).await);
    assert_eq!(runner::tick_once(r.srv.state()).await, 1, "並行上限是 1");
    assert_eq!(r.srv.state().runner.running(), 1);
    assert_eq!(runner::tick_once(r.srv.state()).await, 0, "還在處理第一題，不領第二題");
    assert_eq!(r.get(&b.id).await.status, store::QUEUED);
    r.settle(&a.id).await;
    for _ in 0..40 {
        if r.srv.state().runner.running() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(runner::tick_once(r.srv.state()).await, 1, "第一題做完才輪到第二題");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_is_claimed_when_auto_process_is_off_or_claude_is_unavailable() {
    let fake = Fake::new("", "好");
    let off = rig(&fake, false, "").await;
    let it = off.item("沒開自動處理", "line", Level::Internal).await;
    assert_eq!(runner::tick_once(off.srv.state()).await, 0);
    assert_eq!(off.get(&it.id).await.status, store::QUEUED, "沒開自動處理，題目留給成員領取");

    let r = rig(&fake, true, "").await;
    r.admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "/不存在/claude", "max_concurrent": 1, "timeout_secs": 60, "max_turns": 7}))).await.unwrap();
    let it = r.item("claude 壞了", "line", Level::Internal).await;
    assert_eq!(runner::tick_once(r.srv.state()).await, 0);
    assert_eq!(r.get(&it.id).await.status, store::QUEUED, "claude 不能用就不領，題目留給成員");
    let view: serde_json::Value = r.admin.get("/v1/runner").await.unwrap();
    assert_eq!(view["availability"]["available"], false);
    assert!(view["availability"]["error"].as_str().unwrap().contains("/不存在/claude"), "{view}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failures_go_back_to_pending_with_the_reason() {
    let fake = Fake::new("exit 3", "不會用到");
    let r = rig(&fake, true, "").await;
    let a = r.item("會失敗", "line", Level::Internal).await;
    runner::tick_once(r.srv.state()).await;
    let done = r.settle(&a.id).await;
    assert_eq!(done.status, store::PENDING);
    assert!(done.error.unwrap().contains("Claude Code 失敗"));

    // 專案沒有工作資料夾。
    sqlx::query("UPDATE projects SET work_dir = '' WHERE id = ?").bind(&r.pid).execute(&r.srv.state().pools.server).await.unwrap();
    for _ in 0..40 {
        if r.srv.state().runner.running() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let b = r.item("沒有資料夾", "line", Level::Internal).await;
    runner::tick_once(r.srv.state()).await;
    let done = r.settle(&b.id).await;
    assert!(done.error.unwrap().contains("work_dir"), "要明講是缺工作資料夾");
}

#[tokio::test]
async fn a_run_past_its_timeout_is_killed_and_odd_outputs_are_errors() {
    let fake = Fake::new("sleep 30", "不會用到");
    let s = runner::RunnerSettings { claude_path: fake.path(), timeout_secs: 1, ..Default::default() };
    let began = std::time::Instant::now();
    let e = runner::exec_claude(&s, fake.dir.path(), &runner::Mode::ReadOnly, &[], "hi").await.unwrap_err();
    assert!(e.contains("逾時") && began.elapsed() < Duration::from_secs(10), "{e}");

    assert_eq!(runner::parse_output(r#"{"type":"result","is_error":false,"result":" 答案 "}"#).unwrap(), " 答案 ");
    assert!(runner::parse_output(r#"{"is_error":true,"result":"額度用完"}"#).unwrap_err().contains("額度用完"));
    assert!(runner::parse_output(r#"{"is_error":false,"result":"  "}"#).is_err(), "空回答不算成功");
    assert!(runner::parse_output("這不是 JSON").unwrap_err().contains("不是預期的格式"));
    assert_eq!(runner::parse_output("log line\n{\"result\":\"尾行才是 JSON\"}").unwrap(), "尾行才是 JSON");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_settings_are_validated_and_only_admins_change_them() {
    let fake = Fake::new("", "好");
    let r = rig(&fake, true, "").await;
    let member = r.srv.paired("小明", Role::Member).await;
    let ok = json!({"claude_path": fake.path(), "max_concurrent": 2, "timeout_secs": 100, "max_turns": 10});
    assert!(member.request(Method::PUT, "/v1/runner", Some(ok.clone())).await.is_err());
    for bad in [json!({"claude_path": "", "max_concurrent": 1, "timeout_secs": 100, "max_turns": 10}), json!({"claude_path": "c", "max_concurrent": 0, "timeout_secs": 100, "max_turns": 10}), json!({"claude_path": "c", "max_concurrent": 1, "timeout_secs": 900, "max_turns": 10}), json!({"claude_path": "c", "max_concurrent": 1, "timeout_secs": 5, "max_turns": 10})] {
        assert!(r.admin.request(Method::PUT, "/v1/runner", Some(bad.clone())).await.is_err(), "{bad}");
    }
    let v: serde_json::Value = r.admin.typed(Method::PUT, "/v1/runner", Some(ok)).await.unwrap();
    assert_eq!((v["settings"]["max_concurrent"].as_i64(), v["availability"]["available"].as_bool(), v["availability"]["version"].as_str()), (Some(2), Some(true), Some("9.9.9 (Fake Claude)")));
    let seen: serde_json::Value = member.get("/v1/runner").await.unwrap();
    assert_eq!(seen["settings"]["timeout_secs"], 100, "成員也看得到目前的設定與狀態");
}

#[test]
fn the_children_get_a_minimal_environment_args_never_carry_the_prompt_and_read_only_has_no_bash() {
    let s = runner::RunnerSettings::default();
    let args = runner::claude_args(&s, &runner::Mode::ReadOnly, &["/refs".to_string()]);
    assert_eq!(args.iter().filter(|a| *a == "--add-dir").count(), 1);
    assert!(args.windows(2).any(|w| w[0] == "--add-dir" && w[1] == "/refs"));
    let edit = runner::claude_args(&s, &runner::Mode::Edit { verify_command: "npm test".into() }, &[]);
    let allowed = edit[edit.iter().position(|a| a == "--allowedTools").unwrap() + 1].clone();
    assert!(allowed.contains("Bash(npm test)") && !allowed.split(',').any(|t| t == "Bash"));
    let none = runner::claude_args(&s, &runner::Mode::Edit { verify_command: "  ".into() }, &[]);
    assert!(!none.iter().any(|a| a.contains("Bash(")), "沒有驗證指令就不開 Bash");
}

/// 真實 Claude Code 的端到端演練（會用掉一點額度）：`cargo test -p aiterm-intake-server --test auto_process -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真實的 Claude Code（已登入）"]
async fn real_claude_code_answers_read_only_and_edits_in_a_worktree_then_waits_for_approval() {
    let fake = Fake::new("", "unused");
    let r = rig(&fake, true, "grep -q v2 app.txt").await;
    r.admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "claude", "max_concurrent": 1, "timeout_secs": 300, "max_turns": 12}))).await.unwrap();
    let view: serde_json::Value = r.admin.get("/v1/runner").await.unwrap();
    println!("claude: {}", view["availability"]);
    assert_eq!(view["availability"]["available"], true, "{view}");

    // 1. 唯讀：問專案裡的事，它只能讀檔。
    let q = r.item("app.txt 的內容是什麼？", "line", Level::Internal).await;
    assert_eq!(runner::tick_once(r.srv.state()).await, 1);
    let done = r.settle(&q.id).await;
    println!("唯讀回覆：{:?}\n錯誤：{:?}", done.reply_draft, done.error);
    assert_eq!(done.status, store::DRAFT, "{:?}", done.error);
    assert!(done.reply_draft.unwrap().contains("v1"), "它應該讀得到檔案內容 v1");
    assert_eq!(sh(&r.work_path(), "cat app.txt").trim(), "v1");

    // 2. 可改檔案：在 worktree 裡改，驗證通過，等人核准。
    for _ in 0..40 {
        if r.srv.state().runner.running() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut e = r.item("請把 app.txt 的內容從 v1 改成 v2", "email", Level::Internal).await;
    e.kind = Some("code_change".into());
    store::set_kind(&r.srv.state().pools.intake, &e.id, "code_change", None).await.unwrap();
    assert_eq!(runner::tick_once(r.srv.state()).await, 1);
    let done = r.settle(&e.id).await;
    println!("改檔回覆：{:?}\n驗證：{:?}\n分支：{:?}\n錯誤：{:?}", done.reply_draft, done.verification, done.branch, done.error);
    assert_eq!((done.status.as_str(), done.merge_state.as_deref(), done.verified), (store::DRAFT, Some("pending"), Some(true)), "{:?}", done.error);
    assert_eq!(sh(&r.work_path(), "cat app.txt").trim(), "v1", "核准前專案不變");
    r.admin.request(Method::POST, &format!("/v1/items/{}/merge", e.id), Some(json!({"approve": true}))).await.unwrap();
    assert_eq!(sh(&r.work_path(), "cat app.txt").trim(), "v2", "核准後合併進專案");
}

#[test]
fn only_safe_model_names_reach_the_claude_command_line() {
    for ok in ["sonnet", "opus", "claude-sonnet-5-5", "claude-opus-4-1[1m]", "a.b_c:d"] {
        assert!(runner::valid_model(ok), "{ok}");
    }
    for bad in ["", "-p", "--dangerously-skip-permissions", "a b", "a;b", "$(x)", "模型", &"x".repeat(81)] {
        assert!(!runner::valid_model(bad), "{bad}");
    }
}
