//! 驗收者的 API：專案開關、報告與證據檔（路徑穿越）、放行與重新驗收、禁止清單（權限與檢查）。
use aiterm_intake_client::api::Api;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, BoardCard, NewAgent, NewWorkflowCard};
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};

struct R {
    srv: TestServer,
    admin: Api,
    member: Api,
    pid: String,
}

async fn rig() -> R {
    let srv = TestServer::start().await;
    let (admin, member) = (srv.paired("小華", Role::Admin).await, srv.paired("小明", Role::Member).await);
    let pid = srv.project("範例科技").await;
    sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(srv.dir.path().to_string_lossy().to_string()).bind(&pid).execute(&srv.state().pools.server).await.unwrap();
    R { srv, admin, member, pid }
}

impl R {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.srv.state().pools.intake
    }
    async fn verifier(&self) -> bs::BoardAgent {
        bs::create_agent(self.pool(), NewAgent { project_id: &self.pid, kind: "verifier", name: "驗收者", role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude", created_by: "t" }).await.unwrap().unwrap()
    }
    /// 直接建一張驗收卡並結案成指定狀態。
    async fn verify_card(&self, sig: &str, outcome: &str, verdict: Option<&str>) -> BoardCard {
        let c = bs::create_workflow_card(self.pool(), NewWorkflowCard { project_id: &self.pid, title: "驗收", body: "", kind: "verify", agent_id: None, module_id: None, review_of: None, status: "queued", depends_on: None, from_spec: None }).await.unwrap();
        bs::set_verify_meta(self.pool(), &c.id, sig, &[4001, 4002, 4003]).await.unwrap();
        bs::complete_card(self.pool(), &c.id, outcome, "x").await.unwrap();
        if let Some(v) = verdict {
            bs::set_verdict(self.pool(), &c.id, v, &[]).await.unwrap();
        }
        bs::get_card(self.pool(), &c.id).await.unwrap().unwrap()
    }
    fn evidence_dir(&self, card_id: &str) -> std::path::PathBuf {
        let d = aiterm_intake_server::verifier::card_dir(self.srv.state(), card_id).join("evidence");
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}

fn err(e: aiterm_intake_client::ClientError) -> String {
    e.to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_an_admin_turns_verification_on_and_it_needs_a_folder_and_a_verifier() {
    let r = rig().await;
    let url = format!("/v1/board/projects/{}/verify", r.pid);
    let before = r.member.request(Method::GET, &url, None).await.unwrap();
    assert_eq!((before["enabled"].clone(), before["has_verifier"].clone()), (json!(false), json!(false)), "成員看得到狀態");
    assert!(r.member.request(Method::PUT, &url, Some(json!({"enabled": true}))).await.is_err(), "成員不能改");
    let e = err(r.admin.request(Method::PUT, &url, Some(json!({"enabled": true}))).await.unwrap_err());
    assert!(e.contains("驗收者"), "沒有驗收者不能開：{e}");
    r.verifier().await;
    sqlx::query("UPDATE projects SET work_dir = '' WHERE id = ?").bind(&r.pid).execute(&r.srv.state().pools.server).await.unwrap();
    assert!(err(r.admin.request(Method::PUT, &url, Some(json!({"enabled": true}))).await.unwrap_err()).contains("work_dir"));
    sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(r.srv.dir.path().to_string_lossy().to_string()).bind(&r.pid).execute(&r.srv.state().pools.server).await.unwrap();
    assert_eq!(r.admin.request(Method::PUT, &url, Some(json!({"enabled": true}))).await.unwrap()["enabled"], json!(true));
    assert_eq!(r.member.request(Method::GET, &url, None).await.unwrap()["enabled"], json!(true));
    assert!(aiterm_intake_server::orchestrator::verify_enabled(r.pool(), &r.pid).await);
    r.admin.request(Method::PUT, &url, Some(json!({"enabled": false}))).await.unwrap();
    assert!(!aiterm_intake_server::orchestrator::verify_enabled(r.pool(), &r.pid).await);
    let audited: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit WHERE action IN ('board.verify_on','board.verify_off')").fetch_one(&r.srv.state().pools.server).await.unwrap();
    assert_eq!(audited, 2, "開關寫稽核紀錄");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn members_read_the_report_and_evidence_but_names_cannot_escape_the_evidence_folder() {
    let r = rig().await;
    let c = r.verify_card("s#0", "success", Some("fail")).await;
    bs::save_verify_report(r.pool(), &c.id, &json!({"verdict": "fail", "items": [{"criterion": "按開始", "status": "fail"}]}).to_string(), &["git push origin main".to_string()], 0).await.unwrap();
    let dir = r.evidence_dir(&c.id);
    std::fs::write(dir.join("start.png"), b"\x89PNG-fake").unwrap();
    std::fs::write(dir.join("out.txt"), "npm test: ok").unwrap();
    // 放一個指到證據資料夾外面的連結與一個機密檔，確認讀不到。
    let secret = r.srv.dir.path().join("secret.txt");
    std::fs::write(&secret, "TOP SECRET").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, dir.join("link.txt")).unwrap();
    std::fs::write(dir.join(".hidden"), "x").unwrap();

    let v = r.member.request(Method::GET, &format!("/v1/board/cards/{}/verify", c.id), None).await.unwrap();
    assert_eq!(v["verdict"], json!("fail"));
    assert_eq!(v["report"]["items"][0]["criterion"], json!("按開始"));
    assert_eq!(v["denied"], json!(["git push origin main"]));
    let names: Vec<String> = v["evidence"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"start.png".to_string()) && names.contains(&"out.txt".to_string()));
    assert_eq!(r.member.download(&format!("/v1/board/cards/{}/verify/files/start.png", c.id)).await.unwrap(), b"\x89PNG-fake");
    assert_eq!(r.member.download(&format!("/v1/board/cards/{}/verify/files/out.txt", c.id)).await.unwrap(), b"npm test: ok");
    for bad in ["..%2F..%2Fsecret.txt", "%2Fetc%2Fpasswd", ".hidden", "..", "a%5Cb", "link.txt", "不存在.png"] {
        assert!(r.member.download(&format!("/v1/board/cards/{}/verify/files/{bad}", c.id)).await.is_err(), "應該讀不到：{bad}");
    }
    // 不是驗收卡、不存在的卡：404。
    let normal = bs::create_card(r.pool(), bs::NewCard { project_id: &r.pid, title: "x", body: "", parallel_ok: true, interactive: false, label: None, created_by: "t" }).await.unwrap();
    assert!(r.member.request(Method::GET, &format!("/v1/board/cards/{}/verify", normal.id), None).await.is_err());
    assert!(r.member.request(Method::GET, "/v1/board/cards/不存在/verify", None).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_admin_can_override_a_stuck_verification_and_retry_makes_a_fresh_one_but_not_twice() {
    let r = rig().await;
    r.verifier().await;
    let stuck = r.verify_card("base#0", "success", Some("attention")).await;
    let url = |id: &str, what: &str| format!("/v1/board/cards/{id}/verify/{what}");
    assert!(r.member.request(Method::POST, &url(&stuck.id, "override"), None).await.is_err(), "成員不能放行");
    assert!(r.member.request(Method::POST, &url(&stuck.id, "retry"), None).await.is_err(), "成員不能重新驗收");
    // 重新驗收：建一張新的、簽章加序號、排進待派送；已經有一張在進行就不能再建。
    let new = r.admin.request(Method::POST, &url(&stuck.id, "retry"), None).await.unwrap();
    assert_eq!(new["card_kind"], json!("verify"));
    assert_eq!(bs::get_verify(r.pool(), new["id"].as_str().unwrap()).await.unwrap().unwrap().signature, "base#1");
    assert!(err(r.admin.request(Method::POST, &url(&stuck.id, "retry"), None).await.unwrap_err()).contains("進行中"));
    // 放行：卡片改成通過並記下是誰。
    r.admin.request(Method::POST, &url(&stuck.id, "override"), None).await.unwrap();
    let after = bs::get_card(r.pool(), &stuck.id).await.unwrap().unwrap();
    assert_eq!((after.verdict.as_deref(), after.outcome.as_deref()), (Some("pass"), Some("success")));
    assert_eq!(bs::get_verify(r.pool(), &stuck.id).await.unwrap().unwrap().overridden_by.as_deref(), Some("小華"));
    // 已經通過的不需要放行也不能重新驗收。
    assert!(r.admin.request(Method::POST, &url(&stuck.id, "override"), None).await.is_err());
    assert!(r.admin.request(Method::POST, &url(&stuck.id, "retry"), None).await.is_err());
    // 驗收卡自己失敗（例如合併衝突）也能放行。
    let failed = r.verify_card("x#0", "failed", None).await;
    r.admin.request(Method::POST, &url(&failed.id, "override"), None).await.unwrap();
    assert_eq!(bs::get_card(r.pool(), &failed.id).await.unwrap().unwrap().outcome.as_deref(), Some("success"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_deny_list_is_admin_only_validated_audited_and_resettable() {
    let r = rig().await;
    assert!(r.member.request(Method::GET, "/v1/verify/deny", None).await.is_err(), "成員看不到也改不了");
    let defaults: Value = r.admin.request(Method::GET, "/v1/verify/deny", None).await.unwrap();
    assert!(defaults.as_array().unwrap().iter().any(|x| x["rule"] == json!("Bash(git push:*)") && x["enabled"] == json!(true)));
    // 不合法的規則被拒，並指出是哪一條。
    let e = err(r.admin.request(Method::PUT, "/v1/verify/deny", Some(json!({"rules": [{"rule": "Read(x)", "note": "n", "enabled": true}]}))).await.unwrap_err());
    assert!(e.contains("Read(x)") && e.contains("Bash("), "{e}");
    // 合法的存進去、讀回來；停用一條。
    r.admin.request(Method::PUT, "/v1/verify/deny", Some(json!({"rules": [{"rule": "Bash(sudo:*)", "note": "提權", "enabled": true}, {"rule": "Bash(ssh:*)", "note": "遠端", "enabled": false}]}))).await.unwrap();
    assert_eq!(r.admin.request(Method::GET, "/v1/verify/deny", None).await.unwrap().as_array().unwrap().len(), 2);
    assert_eq!(aiterm_intake_server::deny::effective_for(r.pool(), &r.pid).await, vec!["Bash(sudo:*)".to_string()], "停用的不算");
    // 專案覆寫：新增一條、移除一條全域的；再整份換掉（沒列到的會被刪）。
    let purl = format!("/v1/board/projects/{}/verify/deny", r.pid);
    let p = r.admin.request(Method::PUT, &purl, Some(json!({"overrides": [{"rule": "Bash(make deploy:*)", "kind": "add"}, {"rule": "Bash(sudo:*)", "kind": "remove"}]}))).await.unwrap();
    assert_eq!(p["effective"], json!(["Bash(make deploy:*)"]));
    let p2 = r.admin.request(Method::PUT, &purl, Some(json!({"overrides": [{"rule": "Bash(make deploy:*)", "kind": "add"}]}))).await.unwrap();
    assert_eq!(p2["overrides"].as_array().unwrap().len(), 1);
    assert_eq!(p2["effective"], json!(["Bash(sudo:*)", "Bash(make deploy:*)"]));
    assert!(err(r.admin.request(Method::PUT, &purl, Some(json!({"overrides": [{"rule": "Bash(a:*)", "kind": "maybe"}]}))).await.unwrap_err()).contains("kind"));
    assert!(r.member.request(Method::GET, &purl, None).await.is_err());
    // 還原預設。
    let back = r.admin.request(Method::POST, "/v1/verify/deny/reset", None).await.unwrap();
    assert!(back.as_array().unwrap().len() > 10);
    let audited: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit WHERE action = 'board.verify_deny'").fetch_one(&r.srv.state().pools.server).await.unwrap();
    assert!(audited >= 4, "每次異動都寫稽核：{audited}");
}
