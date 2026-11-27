//! 資料庫技能（server）：連線管理與權限、密碼不外洩、Agent 技能設定、內建 MCP 端點（工具、權限、核准流程）。
use aiterm_intake_client::api::Api;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, NewAgent};
use aiterm_intake_core::host::Secrets;
use aiterm_intake_server::dbmcp::{self, McpCtx};
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;

struct R {
    srv: TestServer,
    admin: Api,
    member: Api,
    pid: String,
    /// SQLite 資料庫檔（在 server 主機上）。
    db_path: String,
}

async fn rig() -> R {
    let srv = TestServer::start().await;
    let (admin, member) = (srv.paired("小華", Role::Admin).await, srv.paired("小明", Role::Member).await);
    let pid = srv.project("範例科技").await;
    let db_path = srv.dir.path().join("app.db").to_string_lossy().to_string();
    std::fs::File::create(&db_path).unwrap();
    R { srv, admin, member, pid, db_path }
}

impl R {
    async fn share(&self, name: &str, password: Option<&str>) -> Value {
        let mut body = json!({"name": name, "db_type": "sqlite", "host": self.db_path, "database": "app", "username": "alice"});
        if let Some(p) = password {
            body["password"] = json!(p);
        }
        self.admin.request(Method::POST, "/v1/db/connections", Some(body)).await.unwrap()
    }
    async fn coder(&self) -> bs::BoardAgent {
        bs::create_agent(&self.srv.state().pools.intake, NewAgent { project_id: &self.pid, kind: "coder", name: "工程師1", role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude", created_by: "t" }).await.unwrap().unwrap()
    }
    /// 為一張假想的卡註冊 MCP token，回傳（MCP 網址, guard）。
    fn mcp(&self, conn_id: &str, mode: &str) -> (String, dbmcp::McpGuard) {
        let skill = serde_json::from_value(json!({"connections": [conn_id], "mode": mode})).unwrap();
        let ctx = McpCtx { card_id: "card1".into(), agent_id: Some("a1".into()), project_id: self.pid.clone(), skill: Some(skill), systems: vec![] };
        let guard = dbmcp::register(self.srv.state(), ctx, &self.srv.dir.path().join("run/card1")).expect("MCP 端點要已啟動");
        let cfg: Value = serde_json::from_str(&std::fs::read_to_string(&guard.config_path).unwrap()).unwrap();
        (cfg["mcpServers"]["aiterm-db"]["url"].as_str().unwrap().to_string(), guard)
    }
}

async fn rpc(url: &str, method: &str, params: Value) -> Value {
    let r = reqwest::Client::new().post(url).json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).send().await.unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    r.json().await.unwrap()
}

async fn call(url: &str, tool: &str, args: Value) -> (String, bool) {
    let v = rpc(url, "tools/call", json!({"name": tool, "arguments": args})).await;
    (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"].as_bool().unwrap_or(false))
}

fn err(e: aiterm_intake_client::ClientError) -> String {
    e.to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn members_see_only_names_admins_manage_and_the_password_never_leaks() {
    let r = rig().await;
    assert!(r.member.request(Method::POST, "/v1/db/connections", Some(json!({"name": "x", "db_type": "sqlite", "host": "/x"}))).await.is_err(), "成員不能分享連線");
    let c = r.share("正式機", Some("超級機密密碼")).await;
    let id = c["id"].as_str().unwrap().to_string();
    assert!(!c.to_string().contains("超級機密密碼"), "回應不含密碼");
    // 成員只看得到能選連線所需的資訊；管理員看得到主機與帳號。
    let m = r.member.request(Method::GET, "/v1/db/connections", None).await.unwrap();
    assert_eq!(m[0], json!({"id": id, "name": "正式機", "db_type": "sqlite", "database": "app"}));
    let a = r.admin.request(Method::GET, "/v1/db/connections", None).await.unwrap();
    assert_eq!((a[0]["host"].clone(), a[0]["username"].clone()), (json!(r.db_path), json!("alice")));
    assert!(!a.to_string().contains("超級機密密碼"));
    // 密碼只在加密金鑰檔：資料庫檔與 API 都找不到明文。
    let secret = r.srv.state().secrets.get(&format!("dbconn:{id}")).unwrap();
    assert_eq!(secret, "超級機密密碼");
    let on_disk = std::fs::read(r.srv.dir.path().join("intake.db")).unwrap();
    assert!(!String::from_utf8_lossy(&on_disk).contains("超級機密密碼"));
    let sealed = std::fs::read_to_string(r.srv.dir.path().join("secrets.json")).unwrap();
    assert!(!sealed.contains("超級機密密碼"), "金鑰檔裡是密文");
    // 同名再分享（沒給密碼）：id 不變、沿用密碼。
    let again = r.share("正式機", None).await;
    assert_eq!(again["id"], c["id"]);
    assert_eq!(r.srv.state().secrets.get(&format!("dbconn:{id}")).unwrap(), "超級機密密碼");
    // 測試連線、刪除連同密碼。
    assert_eq!(r.admin.request(Method::POST, &format!("/v1/db/connections/{id}/test"), None).await.unwrap()["ok"], json!(true));
    assert!(r.member.request(Method::POST, &format!("/v1/db/connections/{id}/test"), None).await.is_err());
    assert!(r.member.request(Method::DELETE, &format!("/v1/db/connections/{id}"), None).await.is_err());
    r.admin.request(Method::DELETE, &format!("/v1/db/connections/{id}"), None).await.unwrap();
    assert_eq!(r.srv.state().secrets.get(&format!("dbconn:{id}")), None, "密碼也刪了");
    let audited: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit WHERE action LIKE 'db.connection.%'").fetch_one(&r.srv.state().pools.server).await.unwrap();
    assert_eq!(audited, 3, "分享×2、刪除都寫稽核");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_databases_and_a_dead_connection_are_refused_with_a_reason() {
    let r = rig().await;
    let e = err(r.admin.request(Method::POST, "/v1/db/connections", Some(json!({"name": "x", "db_type": "mssql", "host": "h", "port": 1433}))).await.unwrap_err());
    assert!(e.contains("不支援") && e.contains("桌面版"), "{e}");
    let e = err(r.admin.request(Method::POST, "/v1/db/connections", Some(json!({"name": "x", "db_type": "postgresql", "host": "h", "port": 0}))).await.unwrap_err());
    assert!(e.contains("埠"), "{e}");
    let ghost = r.srv.dir.path().join("沒有這個目錄/x.db").to_string_lossy().to_string();
    let c = r.admin.request(Method::POST, "/v1/db/connections", Some(json!({"name": "壞的", "db_type": "sqlite", "host": ghost}))).await.unwrap();
    let e = err(r.admin.request(Method::POST, &format!("/v1/db/connections/{}/test", c["id"].as_str().unwrap()), None).await.unwrap_err());
    assert!(e.contains("連不上"), "{e}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_an_admin_sets_an_agents_db_skill_and_the_connections_must_exist() {
    let r = rig().await;
    let agent = r.coder().await;
    let c = r.share("正式機", None).await;
    let url = format!("/v1/board/agents/{}/db-skill", agent.id);
    let skill = json!({"skill": {"connections": [c["id"]], "mode": "approve"}});
    assert!(r.member.request(Method::PUT, &url, Some(skill.clone())).await.is_err(), "成員不能設定");
    let e = err(r.admin.request(Method::PUT, &url, Some(json!({"skill": {"connections": ["不存在"], "mode": "read"}}))).await.unwrap_err());
    assert!(e.contains("沒有這個資料庫連線"), "{e}");
    assert!(r.admin.request(Method::PUT, &url, Some(json!({"skill": {"connections": [c["id"]], "mode": "亂寫"}}))).await.is_err(), "模式要是 read／approve／write");
    let a = r.admin.request(Method::PUT, &url, Some(skill)).await.unwrap();
    assert_eq!(a["version"], json!(agent.version + 1));
    let listed = r.member.request(Method::GET, &format!("/v1/board/projects/{}/agents", r.pid), None).await.unwrap();
    assert!(listed[0]["db_skill"].as_str().unwrap().contains("approve"), "Agent 的 JSON 帶著技能");
    let cleared = r.admin.request(Method::PUT, &url, Some(json!({"skill": null}))).await.unwrap();
    assert_eq!(cleared["db_skill"], Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_mcp_endpoint_speaks_json_rpc_lists_tools_and_rejects_bad_or_revoked_tokens() {
    let r = rig().await;
    let c = r.share("正式機", None).await;
    let (url, guard) = r.mcp(c["id"].as_str().unwrap(), "read");
    let init = rpc(&url, "initialize", json!({"protocolVersion": "2025-06-18"})).await;
    assert_eq!((init["result"]["serverInfo"]["name"].clone(), init["result"]["protocolVersion"].clone()), (json!("aiterm-db"), json!("2025-06-18")));
    let list = rpc(&url, "tools/list", json!({})).await;
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["db_list_connections", "db_list_schemas", "db_list_tables", "db_table_schema", "db_query"]);
    // 通知（沒有 id）回 202，不回內容。
    let note = reqwest::Client::new().post(&url).json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).send().await.unwrap();
    assert_eq!(note.status(), 202);
    assert_eq!(rpc(&url, "no/such", json!({})).await["error"]["code"], json!(-32601));
    // 壞 token 與作廢後的 token 都是 401。
    let bad = url.rsplit_once('/').unwrap().0.to_string() + "/不存在";
    let resp = reqwest::Client::new().post(&bad).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).send().await.unwrap();
    assert_eq!(resp.status(), 401);
    drop(guard);
    let resp = reqwest::Client::new().post(&url).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).send().await.unwrap();
    assert_eq!(resp.status(), 401, "卡結束後 token 作廢");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_only_mode_runs_selects_and_metadata_but_blocks_writes_and_unlisted_connections() {
    let r = rig().await;
    let c = r.share("正式機", None).await;
    let cid = c["id"].as_str().unwrap().to_string();
    let other = r.share("別的庫", None).await;
    // 預先建表（直接用適配器，模擬資料庫裡本來就有資料）。
    let a = aiterm_intake_server::dbconn::adapter_for(r.srv.state(), &cid).await.unwrap();
    a.execute("CREATE TABLE users (id INTEGER, name TEXT)").await.unwrap();
    a.execute("INSERT INTO users VALUES (1, '小明')").await.unwrap();
    let (url, _g) = r.mcp(&cid, "read");

    let (t, e) = call(&url, "db_list_connections", json!({})).await;
    assert!(!e && t.contains("正式機") && !t.contains("別的庫"), "只列被允許的連線：{t}");
    let (t, e) = call(&url, "db_list_tables", json!({"connection_id": cid, "schema": "main"})).await;
    assert!(!e && t.contains("users"), "{t}");
    let (t, e) = call(&url, "db_table_schema", json!({"connection_id": cid, "schema": "main", "table": "users"})).await;
    assert!(!e && t.contains("name"), "{t}");
    let (t, e) = call(&url, "db_query", json!({"connection_id": cid, "sql": "SELECT * FROM users"})).await;
    assert!(!e && t.contains("小明"), "{t}");
    let (t, e) = call(&url, "db_query", json!({"connection_id": cid, "sql": "DELETE FROM users"})).await;
    assert!(e && t.contains("唯讀"), "唯讀擋下寫入：{t}");
    let (t, e) = call(&url, "db_query", json!({"connection_id": cid, "sql": "SELECT 1; DROP TABLE users"})).await;
    assert!(e, "夾帶的第二句也擋：{t}");
    let (t, e) = call(&url, "db_query", json!({"connection_id": other["id"], "sql": "SELECT 1"})).await;
    assert!(e && t.contains("沒有開放給你"), "{t}");
    let (t, e) = call(&url, "db_list_tables", json!({"connection_id": other["id"], "schema": "main"})).await;
    assert!(e && t.contains("沒有開放給你"), "結構查詢也受連線限制：{t}");
    let (t, e) = call(&url, "db_query", json!({"connection_id": cid, "sql": "SELECT * FROM nope"})).await;
    assert!(e && t.to_lowercase().contains("no such table"), "資料庫錯誤原樣回給 Agent：{t}");
    // 資料沒被動到，查詢記錄有了（含被擋下的）。
    let (t, _) = call(&url, "db_query", json!({"connection_id": cid, "sql": "SELECT count(*) AS n FROM users"})).await;
    assert!(t.contains("1"), "{t}");
    let log = r.member.request(Method::GET, &format!("/v1/board/projects/{}/db/queries", r.pid), None).await.unwrap();
    let rows = log.as_array().unwrap();
    assert!(rows.iter().any(|q| q["sql"] == "DELETE FROM users" && q["decision"] == "denied"), "{log}");
    assert!(rows.iter().any(|q| q["sql"] == "SELECT * FROM users" && q["decision"] == "allowed" && q["rows"] == json!(1)), "{log}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_mode_runs_writes_directly() {
    let r = rig().await;
    let cid = r.share("正式機", None).await["id"].as_str().unwrap().to_string();
    let a = aiterm_intake_server::dbconn::adapter_for(r.srv.state(), &cid).await.unwrap();
    a.execute("CREATE TABLE t (id INTEGER)").await.unwrap();
    let (url, _g) = r.mcp(&cid, "write");
    let (t, e) = call(&url, "db_query", json!({"connection_id": cid, "sql": "INSERT INTO t VALUES (7)"})).await;
    assert!(!e, "{t}");
    assert!(a.execute("SELECT id FROM t").await.unwrap().rows[0][0] == json!(7));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approve_mode_hangs_a_write_until_an_admin_decides_and_only_an_admin_can() {
    let r = rig().await;
    let cid = r.share("正式機", None).await["id"].as_str().unwrap().to_string();
    let a = aiterm_intake_server::dbconn::adapter_for(r.srv.state(), &cid).await.unwrap();
    a.execute("CREATE TABLE t (id INTEGER)").await.unwrap();
    let (url, _g) = r.mcp(&cid, "approve");
    // 唯讀查詢不用核准。
    assert!(!call(&url, "db_query", json!({"connection_id": cid, "sql": "SELECT 1"})).await.1);

    // 寫入：呼叫掛著，畫布的待核准清單出現完整 SQL。
    let (u2, c2) = (url.clone(), cid.clone());
    let pending = tokio::spawn(async move { call(&u2, "db_query", json!({"connection_id": c2, "sql": "INSERT INTO t VALUES (1)"})).await });
    let approvals_url = format!("/v1/board/projects/{}/db/approvals", r.pid);
    let mut found = Value::Null;
    for _ in 0..40 {
        let list = r.member.request(Method::GET, &approvals_url, None).await.unwrap();
        if let Some(q) = list.as_array().and_then(|a| a.first()) {
            found = q.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!((found["sql"].clone(), found["connection_name"].clone(), found["decision"].clone()), (json!("INSERT INTO t VALUES (1)"), json!("正式機"), json!("pending")), "{found}");
    assert!(!pending.is_finished(), "還沒決定，工具呼叫要掛著");
    assert_eq!(a.execute("SELECT * FROM t").await.unwrap().rows.len(), 0, "核准前沒有執行");

    let decide = format!("/v1/board/db/approvals/{}/decide", found["id"]);
    assert!(r.member.request(Method::POST, &decide, Some(json!({"approve": true}))).await.is_err(), "成員不能決定");
    r.admin.request(Method::POST, &decide, Some(json!({"approve": true}))).await.unwrap();
    let (t, e) = tokio::time::timeout(Duration::from_secs(10), pending).await.expect("核准後要完成").unwrap();
    assert!(!e, "{t}");
    assert_eq!(a.execute("SELECT * FROM t").await.unwrap().rows.len(), 1, "核准後執行了");
    assert!(r.admin.request(Method::POST, &decide, Some(json!({"approve": false}))).await.is_err(), "已決定的不能再改");
    assert!(r.member.request(Method::GET, &approvals_url, None).await.unwrap().as_array().unwrap().is_empty());
    let q = r.member.request(Method::GET, &format!("/v1/board/projects/{}/db/queries", r.pid), None).await.unwrap();
    assert!(q.as_array().unwrap().iter().any(|x| x["sql"] == "INSERT INTO t VALUES (1)" && x["decision"] == "approved" && x["decided_by"] == "小華" && x["rows"] == json!(1)), "{q}");

    // 拒絕：Agent 收到拒絕、資料沒被動。
    let (u3, c3) = (url.clone(), cid.clone());
    let denied = tokio::spawn(async move { call(&u3, "db_query", json!({"connection_id": c3, "sql": "DELETE FROM t"})).await });
    let mut id = Value::Null;
    for _ in 0..40 {
        if let Some(q) = r.admin.request(Method::GET, &approvals_url, None).await.unwrap().as_array().and_then(|a| a.first()) {
            id = q["id"].clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    r.admin.request(Method::POST, &format!("/v1/board/db/approvals/{id}/decide"), Some(json!({"approve": false}))).await.unwrap();
    let (t, e) = tokio::time::timeout(Duration::from_secs(10), denied).await.expect("拒絕後要回來").unwrap();
    assert!(e && t.contains("拒絕"), "{t}");
    assert_eq!(a.execute("SELECT * FROM t").await.unwrap().rows.len(), 1, "被拒絕的寫入沒有執行");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_card_that_ends_while_a_write_waits_denies_it_and_the_call_returns() {
    let r = rig().await;
    let cid = r.share("正式機", None).await["id"].as_str().unwrap().to_string();
    let a = aiterm_intake_server::dbconn::adapter_for(r.srv.state(), &cid).await.unwrap();
    a.execute("CREATE TABLE t (id INTEGER)").await.unwrap();
    let (url, guard) = r.mcp(&cid, "approve");
    let (u2, c2) = (url.clone(), cid.clone());
    let pending = tokio::spawn(async move { call(&u2, "db_query", json!({"connection_id": c2, "sql": "INSERT INTO t VALUES (1)"})).await });
    let approvals_url = format!("/v1/board/projects/{}/db/approvals", r.pid);
    for _ in 0..40 {
        if !r.admin.request(Method::GET, &approvals_url, None).await.unwrap().as_array().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    drop(guard);
    // 卡結束：token 作廢、等核准的記成拒絕，掛著的工具呼叫也會結束（回錯誤，不是卡到天荒地老）。
    let (t, e) = tokio::time::timeout(Duration::from_secs(10), pending).await.expect("token 作廢後工具呼叫要回來").unwrap_or_else(|_| (String::new(), true));
    assert!(e || t.is_empty(), "{t}");
    assert!(r.admin.request(Method::GET, &approvals_url, None).await.unwrap().as_array().unwrap().is_empty(), "不再是待核准");
    assert_eq!(a.execute("SELECT * FROM t").await.unwrap().rows.len(), 0, "沒有執行");
}
