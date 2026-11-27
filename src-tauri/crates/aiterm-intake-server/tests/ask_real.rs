//! 真實 Claude Code 驗證中途問人（`ask_human`）。需要本機已登入的 `claude`，預設不跑：
//!   cargo test -p aiterm-intake-server --test ask_real -- --ignored --nocapture --test-threads=1
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_questions as bq;
use aiterm_intake_core::board_store::{self as bs, Change, NewAgent, NewCard};
use aiterm_intake_server::orchestrator;
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::json;
use std::process::Command;
use std::time::{Duration, Instant};

fn sh(dir: &std::path::Path, cmd: &str) -> String {
    let o = Command::new("sh").arg("-c").arg(cmd).current_dir(dir).output().unwrap();
    assert!(o.status.success(), "{cmd}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn a_real_claude_asks_a_question_waits_for_a_member_and_uses_the_answer() {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let member = srv.paired("小明", Role::Member).await;
    let pid = srv.project("問人驗證").await;
    let work = tempfile::tempdir().unwrap();
    sh(work.path(), "git init -q -b main && git config user.email a@b.c && git config user.name t && echo v1 > app.txt && git add -A && git commit -qm init");
    sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(work.path().to_string_lossy().to_string()).bind(&pid).execute(&srv.state().pools.server).await.unwrap();
    admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "claude", "max_concurrent": 1, "timeout_secs": 300, "max_turns": 25}))).await.unwrap();
    let pool = &srv.state().pools.intake;
    let coder = bs::create_agent(pool, NewAgent { project_id: &pid, kind: "coder", name: "工程師1", role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: Some("haiku"), cli: "claude", created_by: "t" }).await.unwrap().unwrap();
    admin.request(Method::PUT, &format!("/v1/board/projects/{pid}/orchestration"), Some(json!({"enabled": true}))).await.unwrap();
    let body = "這張卡的規格故意留白：我沒有告訴你要寫什麼暗號。請先用 ask_human 工具問我「answer.txt 要寫什麼暗號」（不要自己猜），拿到回答後把那串暗號原樣寫進專案根目錄的 answer.txt（只寫暗號本身），然後 git add 並 commit。";
    let c = bs::create_card(pool, NewCard { project_id: &pid, title: "問人驗證", body, parallel_ok: true, interactive: false, label: None, created_by: "小華" }).await.unwrap();
    let Change::Ok(c) = bs::assign_card(pool, &c.id, c.version, Some(&coder.id)).await.unwrap() else { panic!("指派失敗") };
    let Change::Ok(c) = bs::move_card(pool, &c.id, c.version, "queued", None, None).await.unwrap() else { panic!("排入失敗") };

    let started = Instant::now();
    let mut asked = false;
    loop {
        orchestrator::tick_once(srv.state()).await;
        // 扮演成員：有問題就回答。
        let open = member.request(Method::GET, &format!("/v1/board/projects/{pid}/questions"), None).await.unwrap();
        if let Some(q) = open["open"].as_array().and_then(|a| a.first()) {
            println!("Agent 問：{}", q["question"]);
            asked = true;
            member.request(Method::POST, &format!("/v1/board/questions/{}/answer", q["id"]), Some(json!({"answer": "PELICAN-7788"}))).await.unwrap();
        }
        let cur = bs::get_card(pool, &c.id).await.unwrap().unwrap();
        if cur.status == "done" {
            println!("card: outcome={:?} error={:?}\nsummary={:?}", cur.outcome, cur.error_message, cur.ai_summary);
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(280), "等太久：{:?}", (&cur.status, &cur.outcome));
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    assert!(asked, "Agent 要真的呼叫 ask_human");
    let answer = sh(work.path(), &format!("git show {}:answer.txt", orchestrator::agent_branch(&coder.id)));
    assert!(answer.contains("PELICAN-7788"), "Agent 要照人的回答寫：{answer}");
    let rows = bq::recent_closed(pool, &pid, 10).await.unwrap();
    assert_eq!((rows[0].status.as_str(), rows[0].answered_by.as_deref()), ("answered", Some("小明")));
}
