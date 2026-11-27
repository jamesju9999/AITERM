//! 即時預覽、人工接手與遠端登入（server）：權限（成員唯讀、管理員才能操作）與用真的 Chrome 驗證串流、接手、遠端登入（預設不跑）。
//!   cargo test -p aiterm-intake-server --test sys_live -- --ignored --nocapture --test-threads=1
use aiterm_intake_client::api::{Api, WsConn};
use aiterm_intake_core::api::Role;
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
    async fn system(&self, name: &str, url: &str, mode: &str) -> Value {
        self.admin.request(Method::POST, "/v1/systems", Some(json!({"name": name, "start_url": url, "allowed_hosts": ["127.0.0.1"], "mode": mode}))).await.unwrap()
    }
    fn mcp(&self, system_ids: Vec<String>) -> (String, dbmcp::McpGuard) {
        let ctx = McpCtx { card_id: "card1".into(), agent_id: Some("a1".into()), project_id: self.pid.clone(), skill: None, systems: system_ids };
        let guard = dbmcp::register(self.srv.state(), ctx, &self.srv.dir.path().join("run/card1")).expect("MCP 端點要已啟動");
        let cfg: Value = serde_json::from_str(&std::fs::read_to_string(&guard.config_path).unwrap()).unwrap();
        (cfg["mcpServers"]["aiterm-systems"]["url"].as_str().unwrap().to_string(), guard)
    }
}

async fn call(url: &str, tool: &str, args: Value) -> (String, bool) {
    let r = reqwest::Client::new().post(url).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}})).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    let text = v["result"]["content"].as_array().map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
    (text, v["result"]["isError"].as_bool().unwrap_or(false))
}

/// 讀訊息直到符合條件（逾時就 panic）。
async fn until(ws: &mut WsConn, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let end = tokio::time::Instant::now() + Duration::from_secs(25);
    loop {
        let left = end.saturating_duration_since(tokio::time::Instant::now());
        let t = tokio::time::timeout(left, ws.next_text()).await.unwrap_or_else(|_| panic!("等不到：{what}")).unwrap_or_else(|| panic!("連線結束了，等不到：{what}"));
        let v: Value = serde_json::from_str(&t).unwrap();
        if pred(&v) {
            return v;
        }
    }
}

async fn send(ws: &mut WsConn, v: Value) {
    ws.send_text(&v.to_string()).await.unwrap();
}

fn err(e: aiterm_intake_client::ClientError) -> String {
    e.to_string()
}

// ---- 不需要瀏覽器的權限檢查 ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn login_start_is_admin_only_and_unknown_or_disabled_systems_and_missing_streams_are_refused() {
    let r = rig().await;
    let s = r.system("ERP", "https://erp.example.com/", "read_only").await;
    let id = s["id"].as_str().unwrap().to_string();
    assert!(r.member.request(Method::POST, &format!("/v1/systems/{id}/login/start"), None).await.is_err(), "成員不能開遠端登入");
    assert!(r.member.request(Method::POST, &format!("/v1/systems/{id}/login/cancel"), None).await.is_err());
    let e = err(r.admin.request(Method::POST, "/v1/systems/不存在/login/start", None).await.unwrap_err());
    assert!(e.contains("找不到"), "{e}");
    r.admin.request(Method::POST, "/v1/systems", Some(json!({"name": "ERP", "start_url": "https://erp.example.com/", "mode": "read_only", "enabled": false}))).await.unwrap();
    let e = err(r.admin.request(Method::POST, &format!("/v1/systems/{id}/login/start"), None).await.unwrap_err());
    assert!(e.contains("已停用"), "{e}");
    // 沒有進行中的登入／沒有在跑的瀏覽器：連不上這些串流（成員連登入畫面也進不去）。
    assert!(r.admin.ws(&format!("/v1/systems/{id}/login/live")).await.is_err());
    assert!(r.member.ws(&format!("/v1/systems/{id}/login/live")).await.is_err());
    assert!(r.member.ws("/v1/board/sys/live/card-x/sys-x").await.is_err());
    let list = r.member.request(Method::GET, &format!("/v1/board/projects/{}/sys/live", r.pid), None).await.unwrap();
    assert_eq!(list, json!([]), "沒有在跑的瀏覽器");
}

// ---- 真的 Chrome ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要 Chrome 與 Node.js（Playwright MCP）"]
async fn members_watch_admins_take_over_and_hand_back_and_a_dropped_takeover_returns_control() {
    let r = rig().await;
    let (port, posts) = aiterm_systems::testsite::fake_site().await;
    let site = format!("http://127.0.0.1:{port}");
    let s = r.system("測試站", &format!("{site}/live"), "autonomous").await;
    let id = s["id"].as_str().unwrap().to_string();
    r.admin.request(Method::PUT, &format!("/v1/systems/{id}/session"), Some(json!({"cookies": [{"name": "sid", "value": "abc", "domain": "127.0.0.1", "path": "/"}]}))).await.unwrap();
    let (url, guard) = r.mcp(vec![id.clone()]);
    // 讓 Agent 的瀏覽器跑起來（Playwright 管著視窗）。
    let (t, e) = call(&url, "sys_navigate", json!({"system": id, "url": format!("{site}/live"), "intent": "讀：開頁面"})).await;
    assert!(!e, "{t}");

    let list = r.member.request(Method::GET, &format!("/v1/board/projects/{}/sys/live", r.pid), None).await.unwrap();
    assert_eq!((list[0]["card_id"].clone(), list[0]["manual"].clone(), list[0]["system_name"].clone()), (json!("card1"), json!(false), json!("測試站")), "{list}");

    let path = format!("/v1/board/sys/live/card1/{id}");
    let mut member = r.member.ws(&path).await.unwrap();
    let first = until(&mut member, "成員收到狀態", |m| m["t"] == "state").await;
    assert_eq!((first["can_operate"].clone(), first["manual"].clone(), first["mode"].clone()), (json!(false), json!(false), json!("agent")));
    let frame = until(&mut member, "成員收到畫面", |m| m["t"] == "frame").await;
    assert!(frame["data"].as_str().unwrap().len() > 100);
    let (fw, fh) = (frame["w"].as_f64().unwrap(), frame["h"].as_f64().unwrap());

    // 成員只能看：任何輸入都被擋。
    send(&mut member, json!({"t": "mouse", "type": "down", "x": 0.1, "y": 0.1})).await;
    let m = until(&mut member, "成員被擋", |m| m["t"] == "error").await;
    assert!(m["message"].as_str().unwrap().contains("只有管理員"), "{m}");

    // 管理員：沒接手前不能輸入。
    let mut admin = r.admin.ws(&path).await.unwrap();
    let st = until(&mut admin, "管理員狀態", |m| m["t"] == "state").await;
    assert_eq!(st["can_operate"], json!(true));
    let click = |kind: &str| json!({"t": "mouse", "type": kind, "x": 200.0 / fw, "y": 130.0 / fh, "button": "left"});
    send(&mut admin, click("down")).await;
    let m = until(&mut admin, "要先接手", |m| m["t"] == "error").await;
    assert!(m["message"].as_str().unwrap().contains("請先按「接手」"), "{m}");
    assert_eq!(posts.load(Ordering::SeqCst), 0);

    // 接手：狀態廣播給所有觀看者；Agent 的工具呼叫收到「接手中」；管理員的點擊真的點到按鈕。
    send(&mut admin, json!({"t": "takeover", "on": true})).await;
    until(&mut member, "成員看到接手中", |m| m["t"] == "state" && m["manual"] == json!(true)).await;
    let (t, _) = call(&url, "sys_snapshot", json!({"system": id})).await;
    assert!(t.contains("接手"), "Agent 要被告知使用者接手中：{t}");
    send(&mut admin, click("move")).await;
    send(&mut admin, click("down")).await;
    send(&mut admin, click("up")).await;
    for _ in 0..40 {
        if posts.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(posts.load(Ordering::SeqCst), 1, "接手時管理員的點擊要真的送出");

    // 交還：Agent 恢復。
    send(&mut admin, json!({"t": "takeover", "on": false})).await;
    until(&mut member, "成員看到交還", |m| m["t"] == "state" && m["manual"] == json!(false)).await;
    let (t, e) = call(&url, "sys_snapshot", json!({"system": id})).await;
    assert!(!e && !t.contains("接手中"), "{t}");

    // 接手後斷線：自動交還，不會讓 Agent 永遠卡住。
    send(&mut admin, json!({"t": "takeover", "on": true})).await;
    until(&mut member, "再次接手", |m| m["t"] == "state" && m["manual"] == json!(true)).await;
    admin.close().await;
    drop(admin);
    until(&mut member, "斷線自動交還", |m| m["t"] == "state" && m["manual"] == json!(false)).await;

    // 稽核：看、接手、交還都有紀錄。
    let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM audit WHERE action LIKE 'system.live.%'").fetch_all(&r.srv.state().pools.server).await.unwrap();
    for a in ["system.live.view", "system.live.takeover", "system.live.handback"] {
        assert!(actions.iter().any(|x| x == a), "{a}：{actions:?}");
    }

    // 超過 3 位觀看者：第 4 位被拒絕。
    let mut extra = vec![member];
    for _ in 0..2 {
        let mut w = r.member.ws(&path).await.unwrap();
        until(&mut w, "觀看者", |m| m["t"] == "state").await;
        extra.push(w);
    }
    let mut fourth = r.member.ws(&path).await.unwrap();
    let m = until(&mut fourth, "第 4 位被拒", |m| m["t"] == "error").await;
    assert!(m["message"].as_str().unwrap().contains("3 位"), "{m}");

    // 卡結束：所有觀看者收到結束通知。
    drop(guard);
    let closed = until(&mut extra[0], "收到結束", |m| m["t"] == "closed").await;
    assert!(closed["why"].as_str().unwrap().contains("結束"), "{closed}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要 Chrome"]
async fn remote_login_streams_takes_input_saves_the_session_and_cleans_up() {
    let r = rig().await;
    let (port, _posts) = aiterm_systems::testsite::fake_site().await;
    let site = format!("http://127.0.0.1:{port}");
    let s = r.system("測試站", &format!("{site}/live"), "read_only").await;
    let id = s["id"].as_str().unwrap().to_string();
    r.admin.request(Method::POST, &format!("/v1/systems/{id}/login/start"), None).await.unwrap();
    let e = err(r.admin.request(Method::POST, &format!("/v1/systems/{id}/login/start"), None).await.unwrap_err());
    assert!(e.contains("已經有人"), "同一個系統同時只能有一個遠端登入：{e}");
    assert!(r.member.ws(&format!("/v1/systems/{id}/login/live")).await.is_err(), "登入畫面含帳密輸入，成員進不去");

    let mut ws = r.admin.ws(&format!("/v1/systems/{id}/login/live")).await.unwrap();
    let st = until(&mut ws, "狀態", |m| m["t"] == "state").await;
    assert_eq!((st["mode"].clone(), st["can_operate"].clone()), (json!("login"), json!(true)));
    let frame = until(&mut ws, "畫面", |m| m["t"] == "frame").await;
    assert_eq!((frame["w"].clone(), frame["h"].clone()), (json!(1280), json!(800)), "遠端登入固定 1280×800");

    // 還沒登入就按完成：沒有白名單網域的 cookie → 說明原因，登入保持開著。
    send(&mut ws, json!({"t": "done"})).await;
    let m = until(&mut ws, "沒有 cookie", |m| m["t"] == "error").await;
    assert!(m["message"].as_str().unwrap().contains("先在桌面版登入") || m["message"].as_str().unwrap().contains("cookie"), "{m}");
    assert!(r.admin.ws(&format!("/v1/systems/{id}/login/live")).await.is_ok(), "登入還開著");

    // 點「Signin」（100,200、200×60 → 中心 200,230）：測試站設 cookie；再按完成。
    let click = |kind: &str| json!({"t": "mouse", "type": kind, "x": 200.0 / 1280.0, "y": 230.0 / 800.0, "button": "left"});
    for k in ["move", "down", "up"] {
        send(&mut ws, click(k)).await;
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    send(&mut ws, json!({"t": "done"})).await;
    let closed = until(&mut ws, "登入完成", |m| m["t"] == "closed").await;
    assert!(closed["why"].as_str().unwrap().contains("session 已保存"), "{closed}");

    // session 已加密保存，之後 Agent 就帶著它。
    let list = r.member.request(Method::GET, &format!("/v1/systems"), None).await.unwrap();
    assert_eq!(list[0]["session"]["exported_by"], json!("小華"), "{list}");
    let stored = r.srv.state().secrets.get(&format!("syssession:{id}")).unwrap();
    assert!(stored.contains("real-login"), "要存到登入後的 cookie：{stored}");
    assert!(!std::fs::read_to_string(r.srv.dir.path().join("secrets.json")).unwrap().contains("real-login"), "金鑰檔裡是密文");

    // 收乾淨：登入瀏覽器的暫存資料夾刪了，可以再開一次。
    assert!(!r.srv.dir.path().join(format!("run/login-{id}")).exists());
    r.admin.request(Method::POST, &format!("/v1/systems/{id}/login/start"), None).await.unwrap();
    r.admin.request(Method::POST, &format!("/v1/systems/{id}/login/cancel"), None).await.unwrap();
    assert!(r.admin.ws(&format!("/v1/systems/{id}/login/live")).await.is_err(), "取消後串流結束");
    assert!(!r.srv.dir.path().join(format!("run/login-{id}")).exists());
    let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM audit WHERE action LIKE 'system.login.%'").fetch_all(&r.srv.state().pools.server).await.unwrap();
    for a in ["system.login.start", "system.login.view", "system.login.finish", "system.login.cancel"] {
        assert!(actions.iter().any(|x| x == a), "{a}：{actions:?}");
    }
}
