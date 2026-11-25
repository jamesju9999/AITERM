#![cfg(unix)] // 假的 claude 是 shell 腳本，Windows 無法當成程式執行。
//! Server 無頭編排：用假的 `claude` 驅動「Coder 做 → Reviewer 審 → 退回修正 → 通過 → 待核准合併」整條路，
//! 驗證每個 Coder 的隔離 worktree、審查的唯讀模式與 diff、修正輪數上限、永不自動合併、成員不能搶 server 的卡。
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
    n=$(cat "$D/n" 2>/dev/null || echo 0); n=$((n+1)); echo $n > "$D/n"
    echo "coder $n start $(pwd)" >> "$D/log.txt"
    cp "$D/last_prompt.txt" "$D/coder_prompt_$n.txt"
    ls > "$D/files_$n.txt"
    printf '%s\n' "$@" > "$D/coder_args_$n.txt"
    sleep @SECS@
    echo "change $n" >> "work-$n.txt"
    echo "coder $n end" >> "$D/log.txt"
    printf '{"type":"result","is_error":false,"result":"做完了"}'
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_module_is_built_in_the_coders_own_worktree_reviewed_read_only_and_waits_for_approval_before_merging() {
    let fake = Fake::new(&["pass"], "0");
    let r = rig(&fake).await;
    r.enable().await;
    let m = r.module("登入頁", "做一個登入頁").await;
    r.pump("合併待核准的卡出現", |c| c.iter().any(|x| x.card_kind == "merge")).await;

    // Coder：在自己專屬的 worktree 裡做（不是專案資料夾），提示詞有角色前綴與卡片內容。
    let log = fake.log();
    assert!(log[0].starts_with("coder 1 start ") && log[0].contains(&format!("worktrees/agent-{}", r.coder.id)), "{log:?}");
    assert!(!log[0].ends_with(&r.work_path().display().to_string()));
    let prompt = fake.read("coder_prompt_1.txt");
    assert!(prompt.starts_with("你是資深工程師") && prompt.contains("# 登入頁") && prompt.contains("做一個登入頁"));
    let module = bs::get_card(r.pool(), &m.id).await.unwrap().unwrap();
    assert_eq!((module.status.as_str(), module.outcome.as_deref()), ("done", Some("success")));
    assert!(module.start_sha.is_some() && module.end_sha.is_some() && module.start_sha != module.end_sha, "有記下做之前與之後的版本：{:?}", (&module.start_sha, &module.end_sha));
    assert_eq!(sh(&r.work_path(), &format!("git log --format=%s {}", r.branch())).lines().next().unwrap(), "工程師1：登入頁");

    // Reviewer：唯讀工具、看到 diff、在一次性 worktree（不是專案資料夾也不是 Coder 的）。
    let review = r.kind("review").await.remove(0);
    assert_eq!((review.verdict.as_deref(), review.review_of.as_deref(), review.module_id.as_deref()), (Some("pass"), Some(m.id.as_str()), Some(m.id.as_str())));
    let rv = log.iter().find(|l| l.starts_with("reviewer 1 ")).unwrap();
    // 模型設定要真的傳給 claude：Coder 沒設就不帶 --model，Reviewer 設了 sonnet 就帶。
    assert!(!fake.read("coder_args_1.txt").contains("--model"), "沒設模型就沿用預設");
    assert!(fake.read("review_args_1.txt").lines().collect::<Vec<_>>().windows(2).any(|w| w == ["--model", "sonnet"]), "{}", fake.read("review_args_1.txt"));
    assert!(rv.ends_with("Read,Grep,Glob") && !rv.contains("Edit"), "審查只給讀檔工具：{rv}");
    assert!(rv.contains("worktrees/review-") && !rv.contains("agent-"), "{rv}");
    let rp = fake.read("review_prompt_1.txt");
    assert!(rp.starts_with("嚴格審查") && rp.contains("做一個登入頁") && rp.contains("+change 1"), "{rp}");
    assert!(!r.srv.state().data_dir.join("worktrees").join(format!("review-{}", review.id)).exists(), "審查完一次性 worktree 要清掉");

    // 通過之後：只建合併待核准的卡，專案資料夾一個字都沒動。
    let merge = r.kind("merge").await.remove(0);
    assert_eq!((merge.status.as_str(), merge.agent_id.as_deref()), ("planning", None));
    assert!(merge.body.contains("工程師1") && merge.body.contains(&r.branch()));
    assert!(!r.work_path().join("work-1.txt").exists());
    assert_eq!(sh(&r.work_path(), "git log --format=%s").trim(), "init", "沒有自動合併");

    // 管理員核准 → 才合併進專案，並清掉 worktree 與分支。
    r.admin.request(Method::POST, &format!("/v1/board/cards/{}/merge", merge.id), None).await.unwrap();
    assert!(r.work_path().join("work-1.txt").exists());
    assert!(sh(&r.work_path(), "git log --format=%s").contains("合併 Agent「工程師1」的成果"));
    let after = bs::get_card(r.pool(), &merge.id).await.unwrap().unwrap();
    assert_eq!((after.status.as_str(), after.outcome.as_deref()), ("done", Some("success")));
    assert!(!r.srv.state().data_dir.join("worktrees").join(format!("agent-{}", r.coder.id)).exists());
    assert!(sh(&r.work_path(), "git branch --list 'agent/*'").trim().is_empty());
    // 核准過的卡不能再核准一次。
    assert!(r.admin.request(Method::POST, &format!("/v1/board/cards/{}/merge", merge.id), None).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_review_sends_the_reasons_back_to_the_coder_and_the_second_review_checks_the_fix() {
    let fake = Fake::new(&["fail", "pass"], "0");
    let r = rig(&fake).await;
    r.enable().await;
    let m = r.module("登入頁", "做一個登入頁").await;
    r.pump("合併待核准", |c| c.iter().any(|x| x.card_kind == "merge")).await;

    let fix = r.kind("fix").await.remove(0);
    let first_review = r.kind("review").await.into_iter().find(|c| c.review_of.as_deref() == Some(m.id.as_str())).unwrap();
    assert_eq!((first_review.verdict.as_deref(), first_review.review_reasons.as_deref()), (Some("fail"), Some("[\"缺測試\"]")));
    assert_eq!((fix.review_of.as_deref(), fix.module_id.as_deref(), fix.agent_id.as_deref()), (Some(first_review.id.as_str()), Some(m.id.as_str()), Some(r.coder.id.as_str())));
    assert!(fix.title.contains("第 1 次") && fix.body.contains("第 1 次修正") && fix.body.contains("缺測試") && fix.body.contains("做一個登入頁"));
    assert!(fake.read("coder_prompt_2.txt").contains("缺測試"), "Coder 第二次拿到的是審查的理由");

    // 第二次審查審的是修正那版，而且被提醒上一次的問題。
    let second = r.kind("review").await.into_iter().find(|c| c.review_of.as_deref() == Some(fix.id.as_str())).unwrap();
    assert_eq!(second.verdict.as_deref(), Some("pass"));
    let rp = fake.read("review_prompt_2.txt");
    assert!(rp.contains("上一次審查提出的問題") && rp.contains("缺測試") && rp.contains("+change 2") && !rp.contains("+change 1"), "{rp}");
    // 兩次修改累積在同一條分支上。
    assert_eq!(sh(&r.work_path(), &format!("git rev-list --count main..{}", r.branch())).trim(), "2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_two_fix_rounds_it_stops_and_waits_for_a_person_instead_of_looping_or_merging() {
    let fake = Fake::new(&["fail", "fail", "fail", "fail"], "0");
    let r = rig(&fake).await;
    r.enable().await;
    r.module("登入頁", "做一個登入頁").await;
    r.pump("第三次審查完成", |c| done(c, "review", 3)).await;
    for _ in 0..8 {
        orchestrator::tick_once(r.srv.state()).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    assert_eq!((r.kind("fix").await.len(), r.kind("review").await.len()), (2, 3), "修正 2 輪、審查 3 次就停");
    assert!(r.kind("merge").await.is_empty(), "沒通過就不建合併卡");
    assert_eq!(fake.read("n").trim(), "3", "Coder 只做了 1 次實作＋2 次修正");
    assert!(!r.work_path().join("work-1.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_coder_works_one_card_at_a_time_and_modules_pile_up_on_the_same_branch() {
    let fake = Fake::new(&["pass", "pass"], "1");
    let r = rig(&fake).await;
    r.enable().await;
    r.module("模組甲", "做甲").await;
    r.module("模組乙", "做乙").await;
    r.pump("合併待核准", |c| c.iter().any(|x| x.card_kind == "merge")).await;
    // 兩次實作不重疊：每個 start 後面緊接著它自己的 end。
    let coder: Vec<String> = fake.log().into_iter().filter(|l| l.starts_with("coder")).collect();
    assert_eq!(coder.len(), 4);
    assert!(coder[0].contains("1 start") && coder[1] == "coder 1 end" && coder[2].contains("2 start") && coder[3] == "coder 2 end", "{coder:?}");
    assert_eq!(sh(&r.work_path(), &format!("git rev-list --count main..{}", r.branch())).trim(), "2");
    assert_eq!(r.kind("merge").await.len(), 1, "兩個模組都過才建一張合併卡");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn members_cannot_grab_a_card_the_server_runs_but_can_when_orchestration_is_off() {
    let fake = Fake::new(&["pass"], "0");
    let r = rig(&fake).await;
    let member = r.srv.paired("小明", Role::Member).await;
    let m = r.module("登入頁", "做一個登入頁").await;
    r.enable().await;
    let err = member.request(Method::POST, &format!("/v1/board/cards/{}/claim", m.id), None).await.unwrap_err();
    assert!(err.to_string().contains("server 的編排"), "{err}");
    // 一般卡片（沒指派 Agent）照舊可以領。
    let plain = bs::create_card(r.pool(), NewCard { project_id: &r.pid, title: "一般", body: "", parallel_ok: true, interactive: false, label: None, created_by: "小華" }).await.unwrap();
    let Change::Ok(plain) = bs::move_card(r.pool(), &plain.id, plain.version, "queued", None, None).await.unwrap() else { panic!() };
    member.request(Method::POST, &format!("/v1/board/cards/{}/claim", plain.id), None).await.unwrap();
    // 關掉之後，指派過 Agent 的卡也回到成員可領。
    r.admin.request(Method::PUT, &format!("/v1/board/projects/{}/orchestration", r.pid), Some(json!({"enabled": false}))).await.unwrap();
    member.request(Method::POST, &format!("/v1/board/cards/{}/claim", m.id), None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_admins_can_turn_it_on_and_it_needs_a_project_folder_and_a_working_claude() {
    let fake = Fake::new(&["pass"], "0");
    let r = rig(&fake).await;
    let member = r.srv.paired("小明", Role::Member).await;
    let url = format!("/v1/board/projects/{}/orchestration", r.pid);
    assert!(member.request(Method::PUT, &url, Some(json!({"enabled": true}))).await.is_err(), "成員不能開");
    // 沒設工作資料夾不能開。
    sqlx::query("UPDATE projects SET work_dir = '' WHERE id = ?").bind(&r.pid).execute(&r.srv.state().pools.server).await.unwrap();
    assert!(r.admin.request(Method::PUT, &url, Some(json!({"enabled": true}))).await.is_err());
    sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(r.work_path().to_string_lossy().to_string()).bind(&r.pid).execute(&r.srv.state().pools.server).await.unwrap();
    r.enable().await;
    let on = r.admin.request(Method::GET, &url, None).await.unwrap();
    assert_eq!(on["enabled"], true);
    // claude 不能用：卡片原封不動留在待派送，不會被領走。
    r.admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "/不存在/claude", "max_concurrent": 1, "timeout_secs": 60, "max_turns": 7}))).await.unwrap();
    let m = r.module("登入頁", "做一個登入頁").await;
    for _ in 0..3 {
        assert_eq!(orchestrator::tick_once(r.srv.state()).await, 0);
    }
    let card = bs::get_card(r.pool(), &m.id).await.unwrap().unwrap();
    assert_eq!((card.status.as_str(), card.claimed_by), ("queued", None));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_no_reviewer_a_finished_module_counts_as_passed_and_other_projects_are_left_alone() {
    let fake = Fake::new(&[], "0");
    let r = rig(&fake).await;
    bs::delete_agent(r.pool(), &r.reviewer.id).await.unwrap().unwrap();
    r.enable().await;
    // 另一個沒開編排的專案：卡片不會被碰。
    let other = r.srv.project("別的專案").await;
    let oc = bs::create_card(r.pool(), NewCard { project_id: &other, title: "別人的", body: "", parallel_ok: true, interactive: false, label: None, created_by: "x" }).await.unwrap();
    bs::move_card(r.pool(), &oc.id, oc.version, "queued", None, None).await.unwrap();
    r.module("登入頁", "做一個登入頁").await;
    r.pump("合併待核准", |c| c.iter().any(|x| x.card_kind == "merge")).await;
    assert!(r.kind("review").await.is_empty());
    assert_eq!(bs::get_card(r.pool(), &oc.id).await.unwrap().unwrap().status, "queued");
}


// ---- S4：Orchestrator 把通過審查的規格拆成實作卡 ----

struct Rig4 {
    r: Rig,
    writer: bs::BoardAgent,
    impl_b: bs::BoardAgent,
    orchestrator: bs::BoardAgent,
}

async fn rig4(fake: &Fake) -> Rig4 {
    let r = rig(fake).await;
    let writer = bs::create_agent(r.pool(), NewAgent { project_id: &r.pid, kind: "coder", name: "規格站", role: "規格", prompt_prefix: "你寫規格", canvas_x: 0.0, canvas_y: 0.0, produces_spec: true, model: None, cli: "claude", created_by: "t" }).await.unwrap().unwrap();
    let impl_b = agent(&r.srv, &r.pid, "coder", "工程師2", "你是工程師2").await;
    let orchestrator = agent(&r.srv, &r.pid, "orchestrator", "調度者", "你是調度者").await;
    Rig4 { r, writer, impl_b, orchestrator }
}

impl Rig4 {
    /// 建一張指派給「規格站」的卡並排進待派送：指派時它就成為規格卡。
    async fn spec(&self, title: &str) -> BoardCard {
        let c = bs::create_card(self.r.pool(), NewCard { project_id: &self.r.pid, title, body: "做一個記帳功能", parallel_ok: true, interactive: false, label: None, created_by: "小華" }).await.unwrap();
        let Change::Ok(c) = bs::assign_card(self.r.pool(), &c.id, c.version, Some(&self.writer.id)).await.unwrap() else { panic!("指派失敗") };
        let Change::Ok(c) = bs::move_card(self.r.pool(), &c.id, c.version, "queued", None, None).await.unwrap() else { panic!("排入失敗") };
        assert!(c.is_spec, "指派給產出規格的 Coder，這張就是規格卡");
        *c
    }
    fn impl_cards<'a>(cards: &'a [BoardCard], spec: &BoardCard) -> Vec<&'a BoardCard> {
        cards.iter().filter(|c| c.from_spec.as_deref() == Some(spec.id.as_str())).collect()
    }
}

const PLAN3: &str = r#"{"interfaces":"User{id:int}","tasks":[{"title":"資料層","body":"做 db"},{"title":"介面層","body":"做 ui","depends_on":[0]},{"title":"文件","body":"寫文件"}]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reviewed_spec_is_split_by_the_orchestrator_into_cards_for_the_idlest_coders_and_dependents_get_the_approved_code() {
    let fake = Fake::new(&["pass", "pass", "pass", "pass"], "0");
    fake.set_plan(PLAN3);
    let x = rig4(&fake).await;
    x.r.enable().await;
    let spec = x.spec("記帳規格").await;
    x.r.pump("合併待核准", |c| c.iter().any(|k| k.card_kind == "merge")).await;
    let cards = x.r.cards().await;

    // 拆解卡：給 Orchestrator、讀規格那一版、唯讀；提示詞帶了需求、工程師名單與規格全文。
    let plan = cards.iter().find(|c| c.card_kind == "plan").unwrap();
    assert_eq!((plan.agent_id.as_deref(), plan.module_id.as_deref(), plan.outcome.as_deref(), plan.ai_summary.as_deref()), (Some(x.orchestrator.id.as_str()), Some(spec.id.as_str()), Some("success"), Some("拆成 3 張實作卡")));
    let pl = fake.log().into_iter().find(|l| l.starts_with("planner ")).unwrap();
    assert!(pl.contains("worktrees/review-"), "在一次性 worktree 裡拆解：{pl}");
    let pp = fake.read("plan_prompt.txt");
    assert!(pp.starts_with("你是調度者") && pp.contains("做一個記帳功能") && pp.contains("工程師1") && pp.contains("工程師2") && !pp.contains("規格站（"), "名單只有能實作的工程師，不含規格站：{pp}");
    assert!(pp.contains("work-1.txt") && pp.contains("change 1"), "附上規格全文（規格站改動的檔案）：{pp}");

    // 實作卡：來源是這份規格、照最閒的分、相依換成卡 id、內文帶介面約定與規格。
    let impls = Rig4::impl_cards(&cards, &spec);
    assert_eq!(impls.len(), 3);
    let by_title = |t: &str| impls.iter().find(|c| c.title == t).copied().unwrap();
    let (db, ui, doc) = (by_title("資料層"), by_title("介面層"), by_title("文件"));
    // 同樣閒的時候誰先拿不固定；但第一張佔了一位之後，第二張一定給另一位，而且都不會派給規格站。
    let implementers = [x.r.coder.id.as_str(), x.impl_b.id.as_str()];
    assert!([db, ui, doc].iter().all(|c| implementers.contains(&c.agent_id.as_deref().unwrap())), "只派給能實作的工程師");
    assert_ne!(db.agent_id, ui.agent_id, "資料層與介面層分給不同的人（才能平行、才會測到跨人的前置）");
    assert_eq!(orchestrator_deps(ui), vec![db.id.clone()]);
    assert!(db.depends_on.is_none() && doc.depends_on.is_none());
    assert!(ui.body.starts_with("做 ui") && ui.body.contains("User{id:int}") && ui.body.contains("從規格「記帳規格」拆出來") && ui.body.contains("change 1"));
    assert!(impls.iter().all(|c| c.card_kind == "work" && !c.is_spec));

    // 相依：介面層的 Coder 開工時，資料層「通過審查的版本」已經在它的 worktree 裡。
    let n_of = |needle: &str| (1..=8).find(|n| fake.read(&format!("coder_prompt_{n}.txt")).contains(needle)).unwrap();
    let (n_db, n_ui) = (n_of("做 db"), n_of("做 ui"));
    assert!(fake.read(&format!("files_{n_ui}.txt")).contains(&format!("work-{n_db}.txt")), "介面層開工時要看得到資料層的程式：{}", fake.read(&format!("files_{n_ui}.txt")));
    // 審查看到的 diff 只有這張卡自己的改動，沒有被預先合併進來的前置。
    assert!(!fake.read("review_prompt_3.txt").contains(&format!("+change {n_db}\n+change {n_ui}")));

    // 規格＋三張實作卡全部通過後才有一張合併卡；專案資料夾在核准前沒被動到。
    assert_eq!(x.r.kind("merge").await.len(), 1);
    assert_eq!(sh(&x.r.work_path(), "git log --format=%s").trim(), "init");
    let merge = x.r.kind("merge").await.remove(0);
    x.r.admin.request(Method::POST, &format!("/v1/board/cards/{}/merge", merge.id), None).await.unwrap();
    for n in 1..=4 {
        assert!(x.r.work_path().join(format!("work-{n}.txt")).exists(), "work-{n}.txt 合併進專案了");
    }
}

fn orchestrator_deps(c: &BoardCard) -> Vec<String> {
    aiterm_intake_core::orchestration::deps_of(c)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dependent_card_waits_for_the_fixed_version_when_the_first_review_fails() {
    // 規格過；資料層第一次審查退回、修正後才過。介面層必須拿到「修正後」的版本，不是退回的那版。
    let fake = Fake::new(&["pass", "fail", "pass", "pass", "pass"], "0");
    fake.set_plan(r#"{"tasks":[{"title":"資料層","body":"做 db"},{"title":"介面層","body":"做 ui","depends_on":[0]}]}"#);
    let x = rig4(&fake).await;
    x.r.enable().await;
    let spec = x.spec("記帳規格").await;
    x.r.pump("合併待核准", |c| c.iter().any(|k| k.card_kind == "merge")).await;
    let cards = x.r.cards().await;
    let db = Rig4::impl_cards(&cards, &spec).into_iter().find(|c| c.title == "資料層").unwrap().clone();
    let fix = cards.iter().find(|c| c.card_kind == "fix" && c.module_id.as_deref() == Some(db.id.as_str())).expect("資料層被退回後有修正卡");
    let ui_n = (1..=8).find(|n| fake.read(&format!("coder_prompt_{n}.txt")).contains("做 ui")).unwrap();
    let fix_n = (1..=8).find(|n| fake.read(&format!("coder_prompt_{n}.txt")).contains("缺測試")).unwrap();
    assert!(ui_n > fix_n, "介面層排在資料層修正之後才開工（序 {ui_n} > {fix_n}）");
    assert!(fake.read(&format!("files_{ui_n}.txt")).contains(&format!("work-{fix_n}.txt")), "拿到的是修正後通過的版本：{}", fake.read(&format!("files_{ui_n}.txt")));
    assert_eq!(fix.review_of.is_some(), true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_plan_fails_the_plan_card_creates_nothing_and_blocks_the_merge() {
    let fake = Fake::new(&["pass"], "0");
    fake.set_plan(r#"{"tasks":[{"title":"甲","depends_on":[3]}]}"#);
    let x = rig4(&fake).await;
    x.r.enable().await;
    let spec = x.spec("記帳規格").await;
    x.r.pump("拆解卡失敗", |c| c.iter().any(|k| k.card_kind == "plan" && k.status == "done")).await;
    for _ in 0..6 {
        orchestrator::tick_once(x.r.srv.state()).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    let cards = x.r.cards().await;
    let plan = cards.iter().find(|c| c.card_kind == "plan").unwrap();
    assert_eq!(plan.outcome.as_deref(), Some("failed"));
    assert!(plan.error_message.clone().unwrap().contains("只能指向排在它前面"), "{:?}", plan.error_message);
    assert!(Rig4::impl_cards(&cards, &spec).is_empty(), "壞的計畫不建任何卡");
    assert_eq!(cards.iter().filter(|c| c.card_kind == "plan").count(), 1, "不會一直重試");
    assert!(cards.iter().all(|c| c.card_kind != "merge"), "規格沒拆成功就不能合併");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_an_implementer_the_plan_fails_with_a_clear_reason_and_without_an_orchestrator_a_spec_is_just_a_module() {
    let fake = Fake::new(&["pass", "pass"], "0");
    fake.set_plan(PLAN3);
    let x = rig4(&fake).await;
    // 只留規格站：沒有人可以實作。
    bs::delete_agent(x.r.pool(), &x.r.coder.id).await.unwrap().unwrap();
    bs::delete_agent(x.r.pool(), &x.impl_b.id).await.unwrap().unwrap();
    x.r.enable().await;
    x.spec("記帳規格").await;
    x.r.pump("拆解卡失敗", |c| c.iter().any(|k| k.card_kind == "plan" && k.status == "done")).await;
    let plan = x.r.kind("plan").await.remove(0);
    assert!(plan.error_message.unwrap().contains("沒有可以接手實作的工程師"));

    // 沒有 Orchestrator：規格審查通過就照一般模組算，直接有合併卡。
    let fake2 = Fake::new(&["pass"], "0");
    let y = rig4(&fake2).await;
    bs::delete_agent(y.r.pool(), &y.orchestrator.id).await.unwrap().unwrap();
    y.r.enable().await;
    y.spec("記帳規格").await;
    y.r.pump("合併待核准", |c| c.iter().any(|k| k.card_kind == "merge")).await;
    assert!(y.r.kind("plan").await.is_empty());
}

/// 真實 Claude Code 的端到端演練（會用掉一點額度）：
/// `cargo test -p aiterm-intake-server --test orchestrate -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真實的 Claude Code（已登入）"]
async fn real_claude_code_builds_a_module_a_real_reviewer_judges_it_and_the_merge_waits_for_approval() {
    let fake = Fake::new(&[], "0"); // 只借它的資料夾結構；實際用真的 claude
    let r = rig(&fake).await;
    r.admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "claude", "max_concurrent": 2, "timeout_secs": 300, "max_turns": 12}))).await.unwrap();
    let view: serde_json::Value = r.admin.get("/v1/runner").await.unwrap();
    assert_eq!(view["availability"]["available"], true, "{view}");
    r.enable().await;
    r.module("加法函式", "在專案根目錄建立 add.py，裡面有 def add(a, b): 回傳 a + b。只做這件事，不要建其他檔案。").await;
    r.pump("審查結論出來", |c| done(c, "review", 1)).await;
    for _ in 0..600 {
        orchestrator::tick_once(r.srv.state()).await;
        let cards = r.cards().await;
        if cards.iter().any(|c| c.card_kind == "merge") || cards.iter().any(|c| c.outcome.as_deref() == Some("failed")) || done(&cards, "fix", 2) && done(&cards, "review", 3) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    for c in r.cards().await {
        println!("[{}] {} status={} outcome={:?} verdict={:?} reasons={:?} err={:?}\n   摘要：{:?}", c.card_kind, c.title, c.status, c.outcome, c.verdict, c.review_reasons, c.error_message, c.ai_summary);
    }
    let module = r.cards().await.into_iter().find(|c| c.card_kind == "work").unwrap();
    assert_eq!(module.outcome.as_deref(), Some("success"), "{:?}", module.error_message);
    let diff = sh(&r.work_path(), &format!("git diff main..{}", r.branch()));
    println!("分支的改動：\n{diff}");
    assert!(diff.contains("add.py") && diff.contains("def add"), "Coder 真的在自己的分支裡寫了檔案");
    assert!(!r.work_path().join("add.py").exists(), "專案資料夾沒被動到");
    let reviews = r.kind("review").await;
    assert!(reviews.iter().all(|c| matches!(c.verdict.as_deref(), Some("pass") | Some("fail"))), "真實 Reviewer 回的結論要能被解析：{reviews:#?}");
    if let Some(merge) = r.kind("merge").await.first() {
        r.admin.request(Method::POST, &format!("/v1/board/cards/{}/merge", merge.id), None).await.unwrap();
        assert!(r.work_path().join("add.py").exists(), "核准後才合併進專案");
    }
}

/// 真實的「退回修正」：Reviewer 被要求「沒有 test_add.py 就不通過」，Coder 第一次不會做，審查退回後要補上。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真實的 Claude Code（已登入）"]
async fn real_claude_code_reviewer_sends_it_back_and_the_coder_fixes_it() {
    let fake = Fake::new(&[], "0");
    let r = rig(&fake).await;
    sqlx::query("UPDATE board_agents SET prompt_prefix = ? WHERE id = ?")
        .bind("審查規則：專案根目錄一定要有 test_add.py（測 add 函式）。沒有這個檔案就一律 fail，並在 reasons 寫明缺什麼；有而且內容合理才 pass。")
        .bind(&r.reviewer.id)
        .execute(r.pool())
        .await
        .unwrap();
    r.admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "claude", "max_concurrent": 2, "timeout_secs": 300, "max_turns": 12}))).await.unwrap();
    r.enable().await;
    r.module("加法函式", "在專案根目錄建立 add.py，裡面有 def add(a, b): 回傳 a + b。只做這件事，不要建其他檔案。").await;
    for _ in 0..1200 {
        orchestrator::tick_once(r.srv.state()).await;
        let cards = r.cards().await;
        if cards.iter().any(|c| c.card_kind == "merge") || cards.iter().any(|c| c.outcome.as_deref() == Some("failed")) || done(&cards, "review", 3) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    for c in r.cards().await {
        println!("[{}] {} status={} outcome={:?} verdict={:?} reasons={:?} err={:?}", c.card_kind, c.title, c.status, c.outcome, c.verdict, c.review_reasons, c.error_message);
    }
    let reviews = r.kind("review").await;
    assert!(reviews.iter().any(|c| c.verdict.as_deref() == Some("fail")), "第一次審查應該因為缺測試檔而退回");
    assert!(!r.kind("fix").await.is_empty(), "退回之後要有修正卡");
    let files = sh(&r.work_path(), &format!("git ls-tree -r --name-only {}", r.branch()));
    println!("分支上的檔案：\n{files}");
    assert!(files.contains("test_add.py"), "修正之後 Coder 補上了測試檔");
    assert!(r.kind("merge").await.len() == 1, "最後通過、建出待核准的合併卡");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_module_finished_after_an_approved_merge_gets_its_own_merge_card_and_approval_waits_while_work_is_unfinished() {
    let fake = Fake::new(&["pass", "pass"], "0");
    let r = rig(&fake).await;
    r.enable().await;
    r.module("模組甲", "做甲").await;
    r.pump("第一張合併卡", |c| c.iter().any(|x| x.card_kind == "merge")).await;
    let first = r.kind("merge").await.remove(0);
    r.admin.request(Method::POST, &format!("/v1/board/cards/{}/merge", first.id), None).await.unwrap();
    // 合併完、沒有新成果：不會憑空再冒出一張合併卡。
    for _ in 0..6 {
        orchestrator::tick_once(r.srv.state()).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    assert_eq!(r.kind("merge").await.len(), 1);
    // 隔一秒再做第二個模組（時間戳是秒），審查通過後要有第二張待核准的合併卡。
    tokio::time::sleep(Duration::from_millis(1100)).await;
    r.module("模組乙", "做乙").await;
    r.pump("第二張合併卡", |c| c.iter().filter(|x| x.card_kind == "merge").count() == 2).await;
    let merges = r.kind("merge").await;
    let second = merges.iter().find(|c| c.id != first.id).unwrap();
    assert_eq!((second.status.as_str(), first.id.as_str() != second.id.as_str()), ("planning", true));
    r.admin.request(Method::POST, &format!("/v1/board/cards/{}/merge", second.id), None).await.unwrap();
    assert!(r.work_path().join("work-1.txt").exists() && r.work_path().join("work-2.txt").exists(), "兩個模組的成果都合併進專案了");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approving_a_merge_while_another_module_is_still_being_worked_on_is_refused_and_destroys_nothing() {
    let fake = Fake::new(&["pass", "pass"], "2");
    let r = rig(&fake).await;
    r.enable().await;
    r.module("模組甲", "做甲").await;
    r.pump("第一張合併卡", |c| c.iter().any(|x| x.card_kind == "merge")).await;
    let merge = r.kind("merge").await.remove(0);
    // 第二個模組開始做（假 claude 要 2 秒），這時核准要被擋下。
    r.module("模組乙", "做乙").await;
    for _ in 0..200 {
        orchestrator::tick_once(r.srv.state()).await;
        if r.cards().await.iter().any(|c| c.title == "模組乙" && c.status == "running") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let err = r.admin.request(Method::POST, &format!("/v1/board/cards/{}/merge", merge.id), None).await.unwrap_err();
    assert!(err.to_string().contains("還沒審查通過") || err.to_string().contains("正在處理"), "{err}");
    assert!(r.srv.state().data_dir.join("worktrees").join(format!("agent-{}", r.coder.id)).exists(), "Coder 的 worktree 還在");
    assert_eq!(bs::get_card(r.pool(), &merge.id).await.unwrap().unwrap().status, "planning", "合併卡還在等");
    assert!(!r.work_path().join("work-1.txt").exists(), "什麼都沒合併");
    // 等第二個模組做完、審查通過後，核准就可以了。
    r.pump("模組乙通過", |c| c.iter().filter(|x| x.card_kind == "review" && x.status == "done").count() == 2).await;
    r.admin.request(Method::POST, &format!("/v1/board/cards/{}/merge", merge.id), None).await.unwrap();
    assert!(r.work_path().join("work-1.txt").exists() && r.work_path().join("work-2.txt").exists());
}

/// 真實 Claude Code 的 S4 端到端：規格站寫規格 → 審查 → Orchestrator 拆解 → 兩位工程師依相依實作 → 審查 → 待核准合併。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真實的 Claude Code（已登入）"]
async fn real_claude_code_writes_a_spec_the_orchestrator_splits_it_and_coders_build_it_in_dependency_order() {
    let fake = Fake::new(&[], "0");
    let x = rig4(&fake).await;
    let r = &x.r;
    r.admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "claude", "max_concurrent": 2, "timeout_secs": 300, "max_turns": 15}))).await.unwrap();
    sqlx::query("UPDATE board_agents SET prompt_prefix = ? WHERE id = ?").bind("你是規格撰寫師：只寫文件，不要寫任何程式碼。").bind(&x.writer.id).execute(r.pool()).await.unwrap();
    r.enable().await;
    let c = bs::create_card(r.pool(), NewCard { project_id: &r.pid, title: "迷你數學套件規格", body: "請在專案根目錄寫一份 SPEC.md（只寫文件）。內容：要做兩個很小的 Python 模組放在 mathx 資料夾：1) mathx/add.py 提供 def add(a, b) 回傳 a + b；2) mathx/calc.py 提供 def double(x)，它必須呼叫 add 來算 add(x, x)。每個模組都要列出函式簽名與驗收條件。", parallel_ok: true, interactive: false, label: None, created_by: "小華" }).await.unwrap();
    let Change::Ok(c) = bs::assign_card(r.pool(), &c.id, c.version, Some(&x.writer.id)).await.unwrap() else { panic!() };
    bs::move_card(r.pool(), &c.id, c.version, "queued", None, None).await.unwrap();
    for _ in 0..1500 {
        orchestrator::tick_once(r.srv.state()).await;
        let cards = r.cards().await;
        if cards.iter().any(|k| k.card_kind == "merge") || cards.iter().any(|k| k.outcome.as_deref() == Some("failed")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let cards = r.cards().await;
    for c in &cards {
        println!("[{}] {} agent={} status={} outcome={:?} verdict={:?} deps={:?} err={:?}", c.card_kind, c.title, c.agent_id.as_deref().map(|a| &a[..4]).unwrap_or("-"), c.status, c.outcome, c.verdict, c.depends_on, c.error_message);
        if c.card_kind == "plan" {
            println!("   拆解摘要：{:?}", c.ai_summary);
        }
    }
    let spec = cards.iter().find(|k| k.is_spec).unwrap();
    let impls = Rig4::impl_cards(&cards, spec);
    assert!(impls.len() >= 2, "Orchestrator 至少拆出 2 張：{}", impls.len());
    assert!(impls.iter().any(|c| c.depends_on.is_some()), "double 依賴 add，至少有一張帶相依");
    assert!(cards.iter().all(|c| c.outcome.as_deref() != Some("failed")), "沒有卡失敗");
    let merge = cards.iter().find(|k| k.card_kind == "merge").expect("全部通過後有合併卡");
    r.admin.request(Method::POST, &format!("/v1/board/cards/{}/merge", merge.id), None).await.unwrap();
    println!("合併後的檔案：\n{}", sh(&r.work_path(), "git ls-files"));
    assert!(r.work_path().join("mathx/add.py").exists() && r.work_path().join("mathx/calc.py").exists());
    println!("{}", sh(&r.work_path(), "cat mathx/add.py mathx/calc.py"));
}
