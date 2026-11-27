//! 系統連線技能（server）：設定與 session 的權限、cookie 篩選與加密、Agent 勾選、MCP 端點；
//! 以及用真的 Chrome＋Playwright 對測試站驗證：session 有載入、唯讀擋寫入、核准流程、卡結束收乾淨（預設不跑）。
//!   cargo test -p aiterm-intake-server --test sys_skill -- --ignored --nocapture --test-threads=1
use aiterm_intake_client::api::Api;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_store::{self as bs, NewAgent};
use aiterm_intake_core::host::Secrets;
use aiterm_intake_server::dbmcp::{self, McpCtx};
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::Duration;

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

impl R {
    async fn system(&self, name: &str, url: &str, hosts: Vec<&str>, mode: &str) -> Value {
        self.admin.request(Method::POST, "/v1/systems", Some(json!({"name": name, "start_url": url, "allowed_hosts": hosts, "mode": mode}))).await.unwrap()
    }
    async fn coder(&self) -> bs::BoardAgent {
        bs::create_agent(&self.srv.state().pools.intake, NewAgent { project_id: &self.pid, kind: "coder", name: "工程師1", role: "", prompt_prefix: "", canvas_x: 0.0, canvas_y: 0.0, produces_spec: false, model: None, cli: "claude", created_by: "t" }).await.unwrap().unwrap()
    }
    /// 為一張假想的卡註冊 MCP token（只有系統連線），回傳（系統工具的網址, guard）。
    fn mcp(&self, system_ids: Vec<String>) -> (String, dbmcp::McpGuard) {
        let ctx = McpCtx { card_id: "card1".into(), agent_id: Some("a1".into()), project_id: self.pid.clone(), skill: None, systems: system_ids };
        let guard = dbmcp::register(self.srv.state(), ctx, &self.srv.dir.path().join("run/card1")).expect("MCP 端點要已啟動");
        let cfg: Value = serde_json::from_str(&std::fs::read_to_string(&guard.config_path).unwrap()).unwrap();
        (cfg["mcpServers"]["aiterm-systems"]["url"].as_str().unwrap().to_string(), guard)
    }
}

async fn rpc(url: &str, method: &str, params: Value) -> Value {
    let r = reqwest::Client::new().post(url).json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).send().await.unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    r.json().await.unwrap()
}

async fn call(url: &str, tool: &str, args: Value) -> (String, bool) {
    let v = rpc(url, "tools/call", json!({"name": tool, "arguments": args})).await;
    let text = v["result"]["content"].as_array().map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
    (text, v["result"]["isError"].as_bool().unwrap_or(false))
}

fn err(e: aiterm_intake_client::ClientError) -> String {
    e.to_string()
}

fn cookie(name: &str, value: &str, domain: &str) -> Value {
    json!({"name": name, "value": value, "domain": domain, "path": "/", "httpOnly": true, "secure": false, "expires": 1900000000.0, "sameSite": "Lax", "size": 10})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_an_admin_manages_systems_and_members_do_not_see_the_whitelist() {
    let r = rig().await;
    let body = json!({"name": "ERP", "start_url": "https://erp.example.com/", "allowed_hosts": ["sso.example.com"], "mode": "approve_each"});
    assert!(r.member.request(Method::POST, "/v1/systems", Some(body.clone())).await.is_err(), "成員不能新增");
    let s = r.admin.request(Method::POST, "/v1/systems", Some(body)).await.unwrap();
    assert_eq!(s["mode"], json!("approve_each"));
    let a = r.admin.request(Method::GET, "/v1/systems", None).await.unwrap();
    assert_eq!(a[0]["allowed_hosts"], json!(["erp.example.com", "sso.example.com"]), "起始網域一定在白名單裡");
    let m = r.member.request(Method::GET, "/v1/systems", None).await.unwrap();
    assert!(m[0].get("allowed_hosts").is_none() && m[0]["name"] == json!("ERP"), "成員看得到有哪些系統，白名單只給管理員");
    // 同名再新增＝取代設定、id 不變。
    let again = r.system("ERP", "https://erp.example.com/", vec![], "read_only").await;
    assert_eq!((again["id"].clone(), again["mode"].clone()), (s["id"].clone(), json!("read_only")));
    // 輸入檢查。
    for (why, b) in [
        ("規則模式", json!({"name": "x", "start_url": "https://x/", "mode": "rules"})),
        ("非 http", json!({"name": "x", "start_url": "ftp://x/"})),
        ("空名稱", json!({"name": " ", "start_url": "https://x/"})),
    ] {
        assert!(r.admin.request(Method::POST, "/v1/systems", Some(b)).await.is_err(), "{why}");
    }
    let id = s["id"].as_str().unwrap();
    assert!(r.member.request(Method::DELETE, &format!("/v1/systems/{id}"), None).await.is_err());
    r.admin.request(Method::DELETE, &format!("/v1/systems/{id}"), None).await.unwrap();
    assert!(r.admin.request(Method::GET, "/v1/systems", None).await.unwrap().as_array().unwrap().is_empty());
    let audited: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit WHERE action LIKE 'system.%'").fetch_one(&r.srv.state().pools.server).await.unwrap();
    assert_eq!(audited, 3, "新增、取代、刪除都寫稽核");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shared_session_keeps_only_whitelisted_cookies_is_encrypted_and_never_returned() {
    let r = rig().await;
    let s = r.system("ERP", "https://erp.example.com/", vec!["sso.example.com"], "read_only").await;
    let id = s["id"].as_str().unwrap().to_string();
    let url = format!("/v1/systems/{id}/session");
    let cookies = json!({"cookies": [cookie("sid", "超級機密的SESSION值", "erp.example.com"), cookie("sso", "tok", ".sso.example.com"), cookie("track", "t", "evil.com"), cookie("other", "o", "example.org")]});
    assert!(r.member.request(Method::PUT, &url, Some(cookies.clone())).await.is_err(), "成員不能分享 session");
    let ok = r.admin.request(Method::PUT, &url, Some(cookies)).await.unwrap();
    assert_eq!(ok["cookie_count"], json!(2), "只留白名單網域的 cookie");
    let stored: Vec<Value> = serde_json::from_str(&r.srv.state().secrets.get(&format!("syssession:{id}")).unwrap()).unwrap();
    assert_eq!(stored.iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["sid", "sso"]);
    assert!(stored[0].get("size").is_none(), "只保留載入需要的欄位");
    // cookie 內容只在加密的金鑰檔：資料庫檔、API 回應、金鑰檔明文都找不到。
    assert!(!String::from_utf8_lossy(&std::fs::read(r.srv.dir.path().join("intake.db")).unwrap()).contains("超級機密的SESSION值"));
    assert!(!std::fs::read_to_string(r.srv.dir.path().join("secrets.json")).unwrap().contains("超級機密的SESSION值"));
    let list = r.member.request(Method::GET, "/v1/systems", None).await.unwrap();
    assert!(!list.to_string().contains("超級機密的SESSION值") && list[0]["session"]["cookie_count"] == json!(2) && list[0]["session"]["exported_by"] == json!("小華"), "{list}");
    // 沒有任何白名單網域的 cookie：拒絕並說明。
    let e = err(r.admin.request(Method::PUT, &url, Some(json!({"cookies": [cookie("x", "y", "evil.com")]}))).await.unwrap_err());
    assert!(e.contains("先在桌面版登入"), "{e}");
    assert_eq!(r.srv.state().secrets.get(&format!("syssession:{id}")).map(|s| s.contains("sid")), Some(true), "失敗的分享不會蓋掉原本的 session");
    // 刪除 session、刪除系統連同 session。
    assert!(r.member.request(Method::DELETE, &url, None).await.is_err());
    r.admin.request(Method::DELETE, &url, None).await.unwrap();
    assert_eq!(r.srv.state().secrets.get(&format!("syssession:{id}")), None);
    assert!(r.admin.request(Method::GET, "/v1/systems", None).await.unwrap()[0]["session"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_an_admin_picks_an_agents_systems_and_they_must_exist() {
    let r = rig().await;
    let agent = r.coder().await;
    let s = r.system("ERP", "https://erp.example.com/", vec![], "read_only").await;
    let url = format!("/v1/board/agents/{}/systems", agent.id);
    assert!(r.member.request(Method::PUT, &url, Some(json!({"ids": [s["id"]]}))).await.is_err());
    let e = err(r.admin.request(Method::PUT, &url, Some(json!({"ids": ["不存在"]}))).await.unwrap_err());
    assert!(e.contains("沒有這個系統連線"), "{e}");
    let a = r.admin.request(Method::PUT, &url, Some(json!({"ids": [s["id"]]}))).await.unwrap();
    assert_eq!(a["version"], json!(agent.version + 1));
    let listed = r.member.request(Method::GET, &format!("/v1/board/projects/{}/agents", r.pid), None).await.unwrap();
    assert!(listed[0]["systems"].as_str().unwrap().contains(s["id"].as_str().unwrap()));
    assert_eq!(r.admin.request(Method::PUT, &url, Some(json!({"ids": []}))).await.unwrap()["systems"], Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_systems_endpoint_lists_only_granted_systems_and_refuses_ones_without_a_session() {
    let r = rig().await;
    let a = r.system("ERP", "https://erp.example.com/", vec![], "read_only").await;
    let b = r.system("別的", "https://other.example.com/", vec![], "read_only").await;
    r.srv.state().sys.set_catalog(vec![json!({"name": "sys_snapshot"}), json!({"name": "sys_click"})]);
    let (url, guard) = r.mcp(vec![a["id"].as_str().unwrap().to_string()]);
    assert!(url.ends_with("/systems"));
    let init = rpc(&url, "initialize", json!({})).await;
    assert_eq!(init["result"]["serverInfo"]["name"], json!("aiterm-systems"));
    let list = rpc(&url, "tools/list", json!({})).await;
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["sys_list", "sys_snapshot", "sys_click"]);
    let (t, e) = call(&url, "sys_list", json!({})).await;
    assert!(!e && t.contains("ERP") && !t.contains("別的") && t.contains("\"has_session\": false"), "只列被授權的：{t}");
    let (t, e) = call(&url, "sys_snapshot", json!({"system": b["id"]})).await;
    assert!(e && t.contains("沒有被授權"), "{t}");
    let (t, e) = call(&url, "sys_snapshot", json!({"system": a["id"]})).await;
    assert!(e && t.contains("還沒有分享登入 session"), "沒有 session 就不啟動瀏覽器：{t}");
    let (t, e) = call(&url, "sys_snapshot", json!({})).await;
    assert!(e && t.contains("缺少 system"), "{t}");
    drop(guard);
    let resp = reqwest::Client::new().post(&url).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).send().await.unwrap();
    assert_eq!(resp.status(), 401, "卡結束後 token 作廢");
}

// ---- 真的 Chrome＋Playwright（預設不跑） ----

fn ref_of(snapshot: &str, label: &str) -> String {
    // 形如 `button "Save" [active] [ref=f1e4]`：標籤後面可能夾著 [active] 之類的狀態。
    let after = snapshot.split(label).nth(1).unwrap_or_else(|| panic!("找不到 {label}：{snapshot}"));
    after.split("[ref=").nth(1).and_then(|s| s.split(']').next()).unwrap_or_else(|| panic!("{label} 沒有 ref：{snapshot}")).to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要 Chrome 與 Node.js（Playwright MCP）"]
async fn a_real_browser_gets_the_shared_session_blocks_writes_in_read_only_and_waits_for_approval() {
    let r = rig().await;
    let (port, posts) = aiterm_systems::testsite::fake_site().await;
    let site = format!("http://127.0.0.1:{port}");
    let s = r.system("測試站", &format!("{site}/"), vec![], "read_only").await;
    let id = s["id"].as_str().unwrap().to_string();
    r.admin.request(Method::PUT, &format!("/v1/systems/{id}/session"), Some(json!({"cookies": [cookie("sid", "abc123", "127.0.0.1")]}))).await.unwrap();
    let (url, guard) = r.mcp(vec![id.clone()]);

    // 真的工具目錄（啟動 Playwright MCP 讀 tools/list）。
    let list = rpc(&url, "tools/list", json!({})).await;
    let names: Vec<String> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"sys_list".into()) && names.contains(&"sys_click".into()) && names.contains(&"sys_snapshot".into()), "{names:?}");
    assert!(!names.iter().any(|n| n == "sys_evaluate" || n == "sys_run_code"), "能繞過畫面操作的工具不開放");

    // session 有載進這個隔離的瀏覽器：測試站回報收到的 Cookie。
    let (t, e) = call(&url, "sys_navigate", json!({"system": id, "url": format!("{site}/whoami"), "intent": "讀：確認登入狀態"})).await;
    assert!(!e, "{t}");
    let (snap, _) = call(&url, "sys_snapshot", json!({"system": id})).await;
    assert!(snap.contains("sid=abc123"), "session 要載進去：{snap}");

    // 唯讀：寫入（送出表單）被閘門擋下，伺服器收不到 POST。
    call(&url, "sys_navigate", json!({"system": id, "url": format!("{site}/"), "intent": "讀：開首頁"})).await;
    let (snap, _) = call(&url, "sys_snapshot", json!({"system": id})).await;
    let save = ref_of(&snap, "button \"Save\"");
    let (t, _) = call(&url, "sys_click", json!({"system": id, "target": save, "intent": "寫：按儲存"})).await;
    assert!(t.contains("被閘門拒絕"), "{t}");
    assert_eq!(posts.load(Ordering::SeqCst), 0, "唯讀：伺服器不該收到 POST");

    // 改成每次核准：寫入停住，待核准清單有它；核准後才送出。
    r.system("測試站", &format!("{site}/"), vec![], "approve_each").await;
    // 被擋下的送出之後頁面狀態變了，舊的 ref 會失效：重新導覽再取快照。
    call(&url, "sys_navigate", json!({"system": id, "url": format!("{site}/"), "intent": "讀：重新開首頁"})).await;
    let (snap, _) = call(&url, "sys_snapshot", json!({"system": id})).await;
    let save = ref_of(&snap, "button \"Save\"");
    let (u2, i2, s2) = (url.clone(), id.clone(), save.clone());
    let mut click = tokio::spawn(async move { call(&u2, "sys_click", json!({"system": i2, "target": s2, "intent": "寫：按儲存"})).await });
    let approvals = format!("/v1/board/projects/{}/sys/approvals", r.pid);
    let mut pending = Value::Null;
    for _ in 0..60 {
        if let Some(p) = r.member.request(Method::GET, &approvals, None).await.unwrap().as_array().and_then(|a| a.first()) {
            pending = p.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    if pending.is_null() {
        let done = tokio::time::timeout(Duration::from_secs(1), &mut click).await;
        panic!("沒有待核准。點擊的結果：{done:?}；伺服器收到的 POST：{}", posts.load(Ordering::SeqCst));
    }
    assert_eq!((pending["method"].clone(), pending["system_name"].clone(), pending["intent"].clone()), (json!("POST"), json!("測試站"), json!("寫：按儲存")), "{pending}");
    assert_eq!(posts.load(Ordering::SeqCst), 0, "核准前不送出");
    let decide = format!("/v1/board/sys/approvals/{}/decide", pending["id"].as_str().unwrap());
    assert!(r.member.request(Method::POST, &decide, Some(json!({"approve": true}))).await.is_err(), "成員不能核准");
    r.admin.request(Method::POST, &decide, Some(json!({"approve": true}))).await.unwrap();
    let (t, _) = tokio::time::timeout(Duration::from_secs(30), click).await.expect("核准後要完成").unwrap();
    println!("click: {t}");
    assert_eq!(posts.load(Ordering::SeqCst), 1, "核准後送出了");
    assert!(r.admin.request(Method::POST, &decide, Some(json!({"approve": false}))).await.is_err(), "已決定的不能再改");

    // 審計有記錄（讀、被擋、核准）。
    let audit = r.member.request(Method::GET, &format!("/v1/systems/{id}/audit"), None).await.unwrap();
    let verdicts: Vec<&str> = audit.as_array().unwrap().iter().filter_map(|a| a["verdict"].as_str()).collect();
    assert!(verdicts.contains(&"deny") && verdicts.contains(&"approved"), "{verdicts:?}");

    // 卡結束：瀏覽器、代理、瀏覽器的暫存資料夾都收掉（`run/<卡>` 本身由 SkillCtx 負責刪）。
    let sys_dir = r.srv.dir.path().join("run/card1/sys");
    assert!(sys_dir.exists(), "執行中有瀏覽器的設定檔資料夾");
    drop(guard);
    for _ in 0..40 {
        if !sys_dir.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(!sys_dir.exists(), "卡結束後瀏覽器的暫存資料夾要刪掉");
    assert!(r.srv.state().sys.approvals.list().is_empty(), "沒有留下等待中的核准");
}
