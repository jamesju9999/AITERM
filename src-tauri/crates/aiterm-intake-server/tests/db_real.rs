//! 真實 Claude Code 驗證資料庫技能（server 內建 MCP 端點）。需要本機已登入的 `claude`，預設不跑：
//!   cargo test -p aiterm-intake-server --test db_real -- --ignored --nocapture --test-threads=1
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, Change, NewAgent, NewCard};
use aiterm_intake_server::orchestrator;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};
use std::process::Command;
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
    db: std::sync::Arc<dyn aiterm_db::adapter::DbAdapter>,
}

/// 一個有 SQLite 資料庫（users 表裡有一筆只在資料庫裡的暗號）、一位裝了資料庫技能的工程師的專案。
async fn rig(mode: &str) -> Rig {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let pid = srv.project("資料庫驗證").await;
    let work = tempfile::tempdir().unwrap();
    sh(work.path(), "git init -q -b main && git config user.email a@b.c && git config user.name t && echo v1 > app.txt && git add -A && git commit -qm init");
    sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(work.path().to_string_lossy().to_string()).bind(&pid).execute(&srv.state().pools.server).await.unwrap();
    admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "claude", "max_concurrent": 1, "timeout_secs": 300, "max_turns": 25}))).await.unwrap();
    let dbfile = srv.dir.path().join("shop.db").to_string_lossy().to_string();
    std::fs::File::create(&dbfile).unwrap();
    let conn = admin.request(Method::POST, "/v1/db/connections", Some(json!({"name": "商店資料庫", "db_type": "sqlite", "host": dbfile}))).await.unwrap();
    let db = aiterm_intake_server::dbconn::adapter_for(srv.state(), conn["id"].as_str().unwrap()).await.unwrap();
    db.execute("CREATE TABLE users (id INTEGER, name TEXT)").await.unwrap();
    db.execute("INSERT INTO users VALUES (1, 'NARWHAL-4421')").await.unwrap();
    let pool = &srv.state().pools.intake;
    let coder = bs::create_agent(pool, NewAgent { project_id: &pid, kind: "coder", name: "工程師1", role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: Some("haiku"), cli: "claude", created_by: "t" }).await.unwrap().unwrap();
    admin.request(Method::PUT, &format!("/v1/board/agents/{}/db-skill", coder.id), Some(json!({"skill": {"connections": [conn["id"]], "mode": mode}}))).await.unwrap();
    admin.request(Method::PUT, &format!("/v1/board/projects/{}/orchestration", pid), Some(json!({"enabled": true}))).await.unwrap();
    Rig { srv, admin, pid, work, coder, db }
}

impl Rig {
    async fn card(&self, title: &str, body: &str) -> String {
        let pool = &self.srv.state().pools.intake;
        let c = bs::create_card(pool, NewCard { project_id: &self.pid, title, body, parallel_ok: true, interactive: false, label: None, created_by: "小華" }).await.unwrap();
        let Change::Ok(c) = bs::assign_card(pool, &c.id, c.version, Some(&self.coder.id)).await.unwrap() else { panic!("指派失敗") };
        let Change::Ok(c) = bs::move_card(pool, &c.id, c.version, "queued", None, None).await.unwrap() else { panic!("排入失敗") };
        c.id
    }
    /// 反覆觸發編排直到卡結案；`each` 每輪呼叫一次（用來在等核准時扮演管理員）。
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
            assert!(started.elapsed() < Duration::from_secs(280), "等太久：{:?}", (&cur.status, &cur.outcome));
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }
    fn answer(&self) -> String {
        sh(self.work.path(), &format!("git show {}:answer.txt", orchestrator::agent_branch(&self.coder.id)))
    }
    async fn queries(&self) -> Vec<Value> {
        self.admin.request(Method::GET, &format!("/v1/board/projects/{}/db/queries", self.pid), None).await.unwrap().as_array().unwrap().clone()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真實的 Claude Code（已登入）"]
async fn a_real_coder_queries_the_database_through_the_builtin_mcp_endpoint() {
    let r = rig("read").await;
    let id = r.card("查資料庫寫 answer.txt", "用資料庫工具查 users 表裡 id=1 那一列的 name，把查到的 name 原樣寫進專案根目錄的 answer.txt（只寫那個值），然後結束。").await;
    let card = r.run(&id, async || {}).await;
    assert_eq!(card.outcome.as_deref(), Some("success"), "{:?}", card.error_message);
    let answer = r.answer();
    println!("answer.txt: {answer}");
    assert!(answer.contains("NARWHAL-4421"), "要從資料庫查到：{answer}");
    let q = r.queries().await;
    assert!(q.iter().any(|x| x["decision"] == "allowed" && x["sql"].as_str().unwrap_or("").to_uppercase().contains("SELECT")), "查詢有記錄：{q:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真實的 Claude Code（已登入）"]
async fn read_only_mode_blocks_a_real_agents_write_and_the_data_stays_intact() {
    let r = rig("read").await;
    let id = r.card("改資料庫", "這是在測試資料庫工具的權限控管：請務必實際呼叫 db_query 對 users 表執行 UPDATE users SET name = 'HACKED' WHERE id = 1（不要自己判斷要不能做，被擋下是預期的結果）。然後把工具回傳的訊息原樣寫進專案根目錄的 answer.txt，結束。").await;
    let card = r.run(&id, async || {}).await;
    assert_eq!(card.outcome.as_deref(), Some("success"), "{:?}", card.error_message);
    let rows = r.db.execute("SELECT name FROM users WHERE id = 1").await.unwrap().rows;
    assert_eq!(rows[0][0], json!("NARWHAL-4421"), "資料不能被改");
    let q = r.queries().await;
    assert!(q.iter().any(|x| x["decision"] == "denied" && x["sql"].as_str().unwrap_or("").to_uppercase().contains("UPDATE")), "被擋下的寫入有記錄：{q:?}");
    println!("answer.txt: {}", r.answer());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真實的 Claude Code（已登入）"]
async fn approve_mode_hangs_a_real_agents_write_until_the_admin_approves() {
    let r = rig("approve").await;
    let id = r.card("加一筆資料", "用資料庫工具在 users 表新增一列：id=2、name='APPROVED-ROW'。寫入成功後，把『已新增』寫進專案根目錄的 answer.txt，然後結束。").await;
    let mut saw_pending_while_unwritten = false;
    let mut approved = false;
    let admin = r.admin.clone();
    let pid = r.pid.clone();
    let db = r.db.clone();
    let card = r
        .run(&id, async || {
            if approved {
                return;
            }
            let list = admin.request(Method::GET, &format!("/v1/board/projects/{pid}/db/approvals"), None).await.unwrap();
            if let Some(q) = list.as_array().and_then(|a| a.first()) {
                // 等核准期間：資料還沒寫進去。
                let n = db.execute("SELECT * FROM users WHERE id = 2").await.unwrap().rows.len();
                saw_pending_while_unwritten = n == 0;
                println!("待核准：{} / {}", q["connection_name"], q["sql"]);
                admin.request(Method::POST, &format!("/v1/board/db/approvals/{}/decide", q["id"]), Some(json!({"approve": true}))).await.unwrap();
                approved = true;
            }
        })
        .await;
    assert_eq!(card.outcome.as_deref(), Some("success"), "{:?}", card.error_message);
    assert!(approved, "Agent 的寫入要出現在待核准清單");
    assert!(saw_pending_while_unwritten, "核准前資料不能先寫進去");
    let rows = r.db.execute("SELECT name FROM users WHERE id = 2").await.unwrap().rows;
    assert_eq!(rows.first().map(|x| x[0].clone()), Some(json!("APPROVED-ROW")), "核准後寫入了");
    let q = r.queries().await;
    assert!(q.iter().any(|x| x["decision"] == "approved" && x["decided_by"] == "小華"), "{q:?}");
}
