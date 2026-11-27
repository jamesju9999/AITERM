//! 真實 Claude Code 驗證系統連線技能（server）：用測試站確認 Agent 帶著分享的 session 操作、
//! 唯讀擋下寫入、每次核准要等管理員。需要本機已登入的 `claude`、Chrome 與 Node.js，預設不跑：
//!   cargo test -p aiterm-intake-server --test sys_real -- --ignored --nocapture --test-threads=1
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, Change, NewAgent, NewCard};
use aiterm_intake_server::orchestrator;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn sh(dir: &std::path::Path, cmd: &str) -> String {
    let o = Command::new("sh").arg("-c").arg(cmd).current_dir(dir).output().unwrap();
    assert!(o.status.success(), "{cmd}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).to_string()
}

struct Rig {
    srv: TestServer,
    admin: aiterm_intake_client::api::Api,
    pid: String,
    work: tempfile::TempDir,
    coder: bs::BoardAgent,
    posts: Arc<AtomicUsize>,
    site: String,
    system_id: String,
}

async fn rig(mode: &str) -> Rig {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let pid = srv.project("系統驗證").await;
    let work = tempfile::tempdir().unwrap();
    sh(work.path(), "git init -q -b main && git config user.email a@b.c && git config user.name t && echo v1 > app.txt && git add -A && git commit -qm init");
    sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(work.path().to_string_lossy().to_string()).bind(&pid).execute(&srv.state().pools.server).await.unwrap();
    admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "claude", "max_concurrent": 1, "timeout_secs": 300, "max_turns": 40}))).await.unwrap();
    let (port, posts) = aiterm_systems::testsite::fake_site().await;
    let site = format!("http://127.0.0.1:{port}");
    let system = admin.request(Method::POST, "/v1/systems", Some(json!({"name": "測試站", "start_url": format!("{site}/"), "mode": mode}))).await.unwrap();
    let system_id = system["id"].as_str().unwrap().to_string();
    admin.request(Method::PUT, &format!("/v1/systems/{system_id}/session"), Some(json!({"cookies": [{"name": "sid", "value": "abc123", "domain": "127.0.0.1", "path": "/"}]}))).await.unwrap();
    let pool = &srv.state().pools.intake;
    let coder = bs::create_agent(pool, NewAgent { project_id: &pid, kind: "coder", name: "工程師1", role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: Some("haiku"), cli: "claude", created_by: "t" }).await.unwrap().unwrap();
    admin.request(Method::PUT, &format!("/v1/board/agents/{}/systems", coder.id), Some(json!({"ids": [system_id]}))).await.unwrap();
    admin.request(Method::PUT, &format!("/v1/board/projects/{pid}/orchestration"), Some(json!({"enabled": true}))).await.unwrap();
    Rig { srv, admin, pid, work, coder, posts, site, system_id }
}

impl Rig {
    async fn card(&self, title: &str, body: &str) -> String {
        let pool = &self.srv.state().pools.intake;
        let c = bs::create_card(pool, NewCard { project_id: &self.pid, title, body, parallel_ok: true, interactive: false, label: None, created_by: "小華" }).await.unwrap();
        let Change::Ok(c) = bs::assign_card(pool, &c.id, c.version, Some(&self.coder.id)).await.unwrap() else { panic!("指派失敗") };
        let Change::Ok(c) = bs::move_card(pool, &c.id, c.version, "queued", None, None).await.unwrap() else { panic!("排入失敗") };
        c.id
    }
    async fn run(&self, card_id: &str, mut each: impl AsyncFnMut()) -> bs::BoardCard {
        let pool = &self.srv.state().pools.intake;
        let started = Instant::now();
        loop {
            orchestrator::tick_once(self.srv.state()).await;
            each().await;
            let cur = bs::get_card(pool, card_id).await.unwrap().unwrap();
            if cur.status == "done" {
                println!("card: outcome={:?} error={:?}\nsummary={:?}", cur.outcome, cur.error_message, cur.ai_summary);
                return cur;
            }
            assert!(started.elapsed() < Duration::from_secs(290), "等太久：{:?}", (&cur.status, &cur.outcome));
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }
    fn answer(&self) -> String {
        sh(self.work.path(), &format!("git show {}:answer.txt", orchestrator::agent_branch(&self.coder.id)))
    }
    async fn audit(&self) -> Vec<Value> {
        self.admin.request(Method::GET, &format!("/v1/systems/{}/audit", self.system_id), None).await.unwrap().as_array().unwrap().clone()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真實的 Claude Code、Chrome 與 Node.js"]
async fn a_real_agent_uses_the_shared_session_and_read_only_blocks_its_write() {
    let r = rig("read_only").await;
    let body = format!("用系統連線工具（先 sys_list）操作「測試站」：\n1. 開啟 {site}/whoami，把頁面上顯示的 cookie 值原樣寫進專案根目錄 answer.txt 的第一行。\n2. 再開啟首頁 {site}/ ，按下 Save 按鈕。這是在測試寫入控管，請務必實際按下去（不要自己判斷不做；被擋下是預期的結果）。\n3. 把按下 Save 之後工具回傳的訊息重點寫進 answer.txt 的第二行，然後結束。", site = r.site);
    let id = r.card("操作測試站", &body).await;
    let card = r.run(&id, async || {}).await;
    assert_eq!(card.outcome.as_deref(), Some("success"), "{:?}", card.error_message);
    let answer = r.answer();
    println!("answer.txt:\n{answer}");
    assert!(answer.contains("sid=abc123"), "Agent 要帶著分享的 session 開啟頁面：{answer}");
    assert_eq!(r.posts.load(Ordering::SeqCst), 0, "唯讀：測試站不該收到 POST");
    let audit = r.audit().await;
    assert!(audit.iter().any(|a| a["verdict"] == "deny" && a["method"] == "POST"), "被擋下的寫入有審計記錄：{audit:?}");
    assert!(!r.srv.dir.path().join("run").exists() || std::fs::read_dir(r.srv.dir.path().join("run")).unwrap().next().is_none(), "卡結束後瀏覽器與暫存都收掉");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真實的 Claude Code、Chrome 與 Node.js"]
async fn approve_each_holds_a_real_agents_write_until_the_admin_approves() {
    let r = rig("approve_each").await;
    let body = format!("用系統連線工具（先 sys_list）操作「測試站」：開啟 {site}/ ，按下 Save 按鈕送出表單（寫入；intent 要以「寫：」開頭）。送出成功後把『已送出』寫進專案根目錄 answer.txt，然後結束。", site = r.site);
    let id = r.card("送出表單", &body).await;
    let mut approved = false;
    let (admin, pid, posts) = (r.admin.clone(), r.pid.clone(), r.posts.clone());
    let card = r
        .run(&id, async || {
            if approved {
                return;
            }
            let list = admin.request(Method::GET, &format!("/v1/board/projects/{pid}/sys/approvals"), None).await.unwrap();
            if let Some(q) = list.as_array().and_then(|a| a.first()) {
                println!("待核准：{} {} / {}", q["method"], q["url"], q["intent"]);
                assert_eq!(posts.load(Ordering::SeqCst), 0, "核准前不送出");
                admin.request(Method::POST, &format!("/v1/board/sys/approvals/{}/decide", q["id"].as_str().unwrap()), Some(json!({"approve": true}))).await.unwrap();
                approved = true;
            }
        })
        .await;
    assert_eq!(card.outcome.as_deref(), Some("success"), "{:?}", card.error_message);
    assert!(approved, "Agent 的寫入要出現在待核准清單");
    assert_eq!(r.posts.load(Ordering::SeqCst), 1, "核准後才送出");
    let audit = r.audit().await;
    assert!(audit.iter().any(|a| a["verdict"] == "approved"), "{audit:?}");
}
