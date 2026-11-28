//! 編制切換（server）：只有管理員能改、成員可讀；切換重建 Agent、進行中的卡擋住、做完再換由排程套用。
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, NewAgent, NewCard};
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};

struct R {
    srv: TestServer,
    admin: aiterm_intake_client::api::Api,
    member: aiterm_intake_client::api::Api,
    pid: String,
}

async fn rig() -> R {
    let srv = TestServer::start().await;
    let (admin, member) = (srv.paired("小華", Role::Admin).await, srv.paired("小明", Role::Member).await);
    let pid = srv.project("範例科技").await;
    R { srv, admin, member, pid }
}

impl R {
    fn path(&self, tail: &str) -> String {
        format!("/v1/board/projects/{}/formations{tail}", self.pid)
    }
    async fn agents(&self) -> Vec<bs::BoardAgent> {
        bs::list_agents(&self.srv.state().pools.intake, &self.pid).await.unwrap()
    }
}

fn fa(kind: &str, name: &str) -> Value {
    json!({"kind": kind, "name": name, "role": "", "dx": 10.0, "dy": 20.0})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_admins_change_formations_and_everyone_can_read_them() {
    let r = rig().await;
    assert!(r.member.request(Method::POST, &r.path(""), Some(json!({"name": "盤點", "agents": []}))).await.is_err(), "成員不能建立");
    let id = r.admin.request(Method::POST, &r.path(""), Some(json!({"name": "盤點", "agents": [fa("coder", "工程師1")]}))).await.unwrap()["id"].as_str().unwrap().to_string();
    let st = r.member.request(Method::GET, &r.path(""), None).await.unwrap();
    assert_eq!(st["formations"][0]["name"], "盤點");
    for (m, tail, body) in [(Method::POST, format!("/{id}/switch"), None), (Method::PUT, format!("/{id}"), Some(json!({"name": "x"}))), (Method::DELETE, format!("/{id}"), None), (Method::POST, "/save-as".to_string(), Some(json!({"name": "x"}))), (Method::POST, format!("/{id}/request-switch"), None), (Method::POST, "/cancel-switch".to_string(), None)] {
        assert!(r.member.request(m.clone(), &r.path(&tail), body).await.is_err(), "成員不能 {m} {tail}");
    }
    assert!(r.admin.request(Method::GET, "/v1/board/projects/不存在/formations", None).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_rebuilds_agents_with_their_skills_and_old_agents_become_retired() {
    let r = rig().await;
    // 舊專案：桌布上已有 Agent、還沒有編制 → 第一次讀取建立「預設」。
    bs::create_agent(&r.srv.state().pools.intake, NewAgent { project_id: &r.pid, kind: "coder", name: "舊工程師", role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude", created_by: "t" }).await.unwrap().unwrap();
    let st = r.admin.request(Method::GET, &format!("{}?default_name=預設", r.path("")), None).await.unwrap();
    assert_eq!((st["formations"][0]["name"].as_str(), st["formations"][0]["agent_count"].as_i64()), (Some("預設"), Some(1)));
    assert_eq!(st["active"], st["formations"][0]["id"]);
    let b = r.admin.request(Method::POST, &r.path(""), Some(json!({"name": "轉換", "agents": [{"kind": "coder", "name": "新工程師", "role": "", "dx": 1.0, "dy": 2.0, "systems": "[\"s1\"]", "model": "sonnet"}, fa("reviewer", "新審核者")]}))).await.unwrap()["id"].as_str().unwrap().to_string();
    let out = r.admin.request(Method::POST, &r.path(&format!("/{b}/switch")), None).await.unwrap();
    assert_eq!(out["status"], "switched");
    let agents = r.agents().await;
    // 同一秒建立的 Agent 順序不固定，所以依名字找。
    let mut names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["新審核者", "新工程師"]);
    let coder = agents.iter().find(|a| a.name == "新工程師").unwrap();
    assert_eq!((coder.systems.as_deref(), coder.model.as_deref()), (Some("[\"s1\"]"), Some("sonnet")));
    let retired = r.member.request(Method::GET, &r.path("/retired"), None).await.unwrap();
    assert_eq!(retired[0]["name"], "舊工程師");
    let st = r.member.request(Method::GET, &r.path(""), None).await.unwrap();
    assert_eq!(st["log"][0]["to_name"], "轉換");
    assert!(r.admin.request(Method::DELETE, &r.path(&format!("/{b}")), None).await.is_err(), "不能刪除目前的編制");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_card_blocks_the_switch_and_the_scheduler_applies_the_pending_one_when_it_ends() {
    let r = rig().await;
    let a = r.admin.request(Method::POST, &r.path(""), Some(json!({"name": "盤點", "agents": [fa("coder", "工程師1")]}))).await.unwrap()["id"].as_str().unwrap().to_string();
    let b = r.admin.request(Method::POST, &r.path(""), Some(json!({"name": "轉換", "agents": [fa("coder", "轉換工程師")]}))).await.unwrap()["id"].as_str().unwrap().to_string();
    r.admin.request(Method::POST, &r.path(&format!("/{a}/switch")), None).await.unwrap();
    let pool = r.srv.state().pools.intake.clone();
    let agent = r.agents().await.remove(0);
    let card = bs::create_card(&pool, NewCard { project_id: &r.pid, title: "工作", body: "", parallel_ok: true, interactive: false, label: None, created_by: "t" }).await.unwrap();
    sqlx::query("UPDATE board_cards SET agent_id = ?, status = 'running' WHERE id = ?").bind(&agent.id).bind(&card.id).execute(&pool).await.unwrap();
    let out = r.admin.request(Method::POST, &r.path(&format!("/{b}/switch")), None).await.unwrap();
    assert_eq!((out["status"].as_str(), out["names"][0].as_str()), (Some("busy"), Some("工程師1")));
    assert_eq!(r.agents().await.len(), 1);
    r.admin.request(Method::POST, &r.path(&format!("/{b}/request-switch")), None).await.unwrap();
    assert_eq!(r.admin.request(Method::GET, &r.path(""), None).await.unwrap()["pending"], json!(b));
    aiterm_intake_server::orchestrator::tick_once(r.srv.state()).await;
    assert_eq!(r.agents().await[0].name, "工程師1", "還在工作就不換");
    sqlx::query("UPDATE board_cards SET status = 'done' WHERE id = ?").bind(&card.id).execute(&pool).await.unwrap();
    aiterm_intake_server::orchestrator::tick_once(r.srv.state()).await;
    assert_eq!(r.agents().await[0].name, "轉換工程師", "做完後排程自動切換");
    assert_eq!(r.admin.request(Method::GET, &r.path(""), None).await.unwrap()["pending"], Value::Null);
    // 取消等待中的切換。
    r.admin.request(Method::POST, &r.path(&format!("/{a}/request-switch")), None).await.unwrap();
    r.admin.request(Method::POST, &r.path("/cancel-switch"), None).await.unwrap();
    assert_eq!(r.admin.request(Method::GET, &r.path(""), None).await.unwrap()["pending"], Value::Null);
}
