//! 技能庫 API：權限、上傳檢查（壞路徑、超限）、設定 Agent 技能、刪除後的過濾。
use aiterm_intake_client::api::Api;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, NewAgent};
use aiterm_intake_server::testkit::TestServer;
use base64::Engine;
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
    R { srv, admin, member, pid }
}

fn b64(s: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

fn body(name: &str, extra: &[(&str, &[u8])]) -> Value {
    let mut files = vec![json!({"path": "SKILL.md", "data": b64(format!("---\nname: {name}\ndescription: 說明\n---\n內文").as_bytes())})];
    for (p, d) in extra {
        files.push(json!({"path": p, "data": b64(d)}));
    }
    json!({"folder_name": "資料夾", "files": files})
}

fn err(e: aiterm_intake_client::ClientError) -> String {
    e.to_string()
}

impl R {
    async fn coder(&self) -> bs::BoardAgent {
        bs::create_agent(&self.srv.state().pools.intake, NewAgent { project_id: &self.pid, kind: "coder", name: "工程師1", role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude", created_by: "t" }).await.unwrap().unwrap()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_an_admin_uploads_or_deletes_but_every_member_can_list() {
    let r = rig().await;
    assert!(r.member.request(Method::POST, "/v1/skills", Some(body("寫測試", &[]))).await.is_err(), "成員不能上傳");
    let s = r.admin.request(Method::POST, "/v1/skills", Some(body("寫測試", &[("scripts/run.sh", b"echo hi")]))).await.unwrap();
    assert_eq!((s["name"].clone(), s["description"].clone(), s["files"].clone(), s["uploaded_by"].clone()), (json!("寫測試"), json!("說明"), json!(2), json!("小華")));
    let list = r.member.request(Method::GET, "/v1/skills", None).await.unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1, "成員看得到清單");
    let id = s["id"].as_str().unwrap();
    assert!(r.member.request(Method::DELETE, &format!("/v1/skills/{id}"), None).await.is_err(), "成員不能刪");
    r.admin.request(Method::DELETE, &format!("/v1/skills/{id}"), None).await.unwrap();
    assert!(r.member.request(Method::GET, "/v1/skills", None).await.unwrap().as_array().unwrap().is_empty());
    assert!(!r.srv.dir.path().join("skills").join(id).exists(), "檔案也刪了");
    let audited: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit WHERE action IN ('skill.upload','skill.delete')").fetch_one(&r.srv.state().pools.server).await.unwrap();
    assert_eq!(audited, 2, "上傳與刪除都寫稽核");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_uploads_are_refused_with_a_reason() {
    let r = rig().await;
    let e = err(r.admin.request(Method::POST, "/v1/skills", Some(body("x", &[("../evil.txt", b"x")]))).await.unwrap_err());
    assert!(e.contains("路徑不合法"), "{e}");
    let e = err(r.admin.request(Method::POST, "/v1/skills", Some(json!({"folder_name": "x", "files": [{"path": "a.md", "data": b64(b"x")}]}))).await.unwrap_err());
    assert!(e.contains("SKILL.md"), "{e}");
    let e = err(r.admin.request(Method::POST, "/v1/skills", Some(json!({"folder_name": "x", "files": [{"path": "SKILL.md", "data": "!!!不是base64"}]}))).await.unwrap_err());
    assert!(e.contains("base64"), "{e}");
    let big = vec![0u8; aiterm_intake_core::skills::MAX_FILE_BYTES + 1];
    let e = err(r.admin.request(Method::POST, "/v1/skills", Some(body("x", &[("big.bin", &big)]))).await.unwrap_err());
    assert!(e.contains("單檔上限"), "{e}");
    assert!(r.admin.request(Method::GET, "/v1/skills", None).await.unwrap().as_array().unwrap().is_empty(), "拒絕的上傳什麼都沒留下");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_near_the_limit_gets_through_the_body_limit() {
    let r = rig().await;
    // 5 個 ~1.9 MB 的檔（接近 10 MB 總量）：base64 後主體約 13 MB，超過預設的 2 MB 主體上限。
    let chunk = vec![7u8; 1_900_000];
    let extra: Vec<(String, &[u8])> = (0..5).map(|i| (format!("data{i}.bin"), chunk.as_slice())).collect();
    let extra_ref: Vec<(&str, &[u8])> = extra.iter().map(|(p, d)| (p.as_str(), *d)).collect();
    let s = r.admin.request(Method::POST, "/v1/skills", Some(body("大技能", &extra_ref))).await.unwrap();
    assert_eq!(s["files"], json!(6));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_admin_sets_an_agents_skills_and_deleted_skills_drop_out_of_its_list() {
    let r = rig().await;
    let agent = r.coder().await;
    let s1 = r.admin.request(Method::POST, "/v1/skills", Some(body("甲", &[]))).await.unwrap();
    let s2 = r.admin.request(Method::POST, "/v1/skills", Some(body("乙", &[]))).await.unwrap();
    let url = format!("/v1/board/agents/{}/skills", agent.id);
    assert!(r.member.request(Method::PUT, &url, Some(json!({"ids": [s1["id"]]}))).await.is_err(), "成員不能設定");
    let e = err(r.admin.request(Method::PUT, &url, Some(json!({"ids": ["00000000-0000-0000-0000-000000000000"]}))).await.unwrap_err());
    assert!(e.contains("沒有這個技能"), "{e}");
    let a = r.admin.request(Method::PUT, &url, Some(json!({"ids": [s1["id"], s2["id"]]}))).await.unwrap();
    assert_eq!(a["version"], json!(agent.version + 1));
    let listed: Value = r.member.request(Method::GET, &format!("/v1/board/projects/{}/agents", r.pid), None).await.unwrap();
    assert!(listed[0]["custom_skills"].as_str().unwrap().contains(s1["id"].as_str().unwrap()), "Agent 的 JSON 帶著技能 id");
    r.admin.request(Method::DELETE, &format!("/v1/skills/{}", s1["id"].as_str().unwrap()), None).await.unwrap();
    let a = bs::get_agent(&r.srv.state().pools.intake, &agent.id).await.unwrap().unwrap();
    let names: Vec<String> = bs::agent_skills(&r.srv.state().pools.intake, &a).await.unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["乙"], "技能庫刪掉後 Agent 只剩還在的");
    assert!(r.admin.request(Method::PUT, "/v1/board/agents/不存在/skills", Some(json!({"ids": []}))).await.is_err());
}
