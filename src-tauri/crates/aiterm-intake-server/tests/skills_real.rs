//! 真實 Claude Code 驗證：技能裡的暗號要真的被讀到並照做。需要本機已登入的 `claude`，所以預設不跑：
//!   cargo test -p aiterm-intake-server --test skills_real -- --ignored --nocapture
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, Change, NewAgent, NewCard};
use aiterm_intake_server::orchestrator;
use aiterm_intake_server::testkit::TestServer;
use base64::Engine;
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
#[ignore = "需要真實的 Claude Code（已登入）"]
async fn a_real_coder_reads_the_skill_and_its_reference_file_and_follows_it() {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let pid = srv.project("技能驗證").await;
    let work = tempfile::tempdir().unwrap();
    sh(work.path(), "git init -q -b main && git config user.email a@b.c && git config user.name t && echo v1 > app.txt && git add -A && git commit -qm init");
    sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(work.path().to_string_lossy().to_string()).bind(&pid).execute(&srv.state().pools.server).await.unwrap();
    admin.request(Method::PUT, "/v1/runner", Some(json!({"claude_path": "claude", "max_concurrent": 1, "timeout_secs": 300, "max_turns": 20}))).await.unwrap();
    let pool = &srv.state().pools.intake;
    let coder = bs::create_agent(pool, NewAgent { project_id: &pid, kind: "coder", name: "工程師1", role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: Some("haiku"), cli: "claude", created_by: "t" }).await.unwrap().unwrap();

    // 技能：暗號一在 SKILL.md，暗號二在附屬檔 data/code.txt（要照指示去讀）。
    let b64 = |t: &str| base64::engine::general_purpose::STANDARD.encode(t.as_bytes());
    let skill = admin.request(Method::POST, "/v1/skills", Some(json!({"folder_name": "暗號技能", "files": [
        {"path": "SKILL.md", "data": b64("---\nname: 暗號技能\ndescription: 被要求「依暗號技能寫 answer.txt」時使用\n---\n依暗號技能寫 answer.txt 的做法：\n1. 先讀同資料夾的 data/code.txt。\n2. 在專案根目錄建立 answer.txt，第一行寫「暗號一：SKILL-ZEBRA-7731」，第二行寫 data/code.txt 的內容。\n")},
        {"path": "data/code.txt", "data": b64("暗號二：REF-OTTER-5520")}
    ]}))).await.unwrap();
    admin.request(Method::PUT, &format!("/v1/board/agents/{}/skills", coder.id), Some(json!({"ids": [skill["id"]]}))).await.unwrap();
    admin.request(Method::PUT, &format!("/v1/board/projects/{}/orchestration", pid), Some(json!({"enabled": true}))).await.unwrap();

    let c = bs::create_card(pool, NewCard { project_id: &pid, title: "依暗號技能寫 answer.txt", body: "請依暗號技能寫 answer.txt，寫好就結束。", parallel_ok: true, interactive: false, label: None, created_by: "小華" }).await.unwrap();
    let Change::Ok(c) = bs::assign_card(pool, &c.id, c.version, Some(&coder.id)).await.unwrap() else { panic!("指派失敗") };
    let Change::Ok(c) = bs::move_card(pool, &c.id, c.version, "queued", None, None).await.unwrap() else { panic!("排入失敗") };

    let started = Instant::now();
    loop {
        orchestrator::tick_once(srv.state()).await;
        let cur = bs::get_card(pool, &c.id).await.unwrap().unwrap();
        if cur.status == "done" {
            println!("card: outcome={:?} error={:?} summary={:?}", cur.outcome, cur.error_message, cur.ai_summary);
            assert_eq!(cur.outcome.as_deref(), Some("success"), "{:?}", cur.error_message);
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(280), "等太久：{:?}", (&cur.status, &cur.outcome));
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let branch = orchestrator::agent_branch(&coder.id);
    let answer = sh(work.path(), &format!("git show {branch}:answer.txt"));
    println!("answer.txt:\n{answer}");
    assert!(answer.contains("SKILL-ZEBRA-7731"), "要讀到 SKILL.md 並照做：{answer}");
    assert!(answer.contains("REF-OTTER-5520"), "要照 SKILL.md 的指示去讀附屬檔：{answer}");
    // 副本用完就刪，共用技能庫沒被動到。
    assert!(!srv.dir.path().join("run").exists() || std::fs::read_dir(srv.dir.path().join("run")).unwrap().next().is_none());
    let lib = std::fs::read_to_string(srv.dir.path().join("skills").join(skill["id"].as_str().unwrap()).join("SKILL.md")).unwrap();
    assert!(lib.contains("SKILL-ZEBRA-7731"));
}
