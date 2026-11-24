//! 資料端點的端到端測試：真的 TLS、真的配對、用 Client 呼叫。
use aiterm_intake_client::ClientError;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::config::{Config, Handling};
use aiterm_intake_core::ops::{ConfigView, SenderDto};
use aiterm_intake_core::rules::Level;
use aiterm_intake_core::store::{self, IntakeItem, NewItem};
use aiterm_intake_server::testkit::TestServer;
use reqwest::Method;
use serde_json::json;
use std::time::Duration;

async fn add_item(srv: &TestServer, pid: &str, source: &str, sender: &str, subject: &str, status: &str) -> IntakeItem {
    store::insert_item(
        &srv.state().pools.intake,
        NewItem { project_id: pid, id: None, source, sender, level: Level::External, subject, body: "內容", message_id: &uuid::Uuid::new_v4().to_string(), status },
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn members_can_read_but_only_admins_can_change_settings() {
    let srv = TestServer::start().await;
    let pid = srv.project("範例專案").await;
    let member = srv.paired("小明", Role::Member).await;
    let admin = srv.paired("小華", Role::Admin).await;
    let path = format!("/v1/projects/{pid}/config");
    let cfg = Config { tag: "[ABC]".into(), ..Config::default() };
    let body = json!({ "config": cfg, "telegram_default": false });
    assert_eq!(member.request(Method::PUT, &path, Some(body.clone())).await.unwrap_err(), ClientError::Forbidden);
    admin.request(Method::PUT, &path, Some(body)).await.unwrap();
    let seen: ConfigView = member.get(&path).await.unwrap();
    assert_eq!(seen.config.tag, "[ABC]", "成員看得到管理員存的設定");
    assert!(seen.saved);
}

#[tokio::test]
async fn every_config_field_survives_a_round_trip() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let admin = srv.paired("小華", Role::Admin).await;
    let path = format!("/v1/projects/{pid}/config");
    // 守門：沒有任何欄位被 serde 跳過（新增欄位時會自動納入）。
    let default_keys: Vec<String> = serde_json::to_value(Config::default()).unwrap().as_object().unwrap().keys().cloned().collect();
    let got: serde_json::Value = admin.get(&path).await.unwrap();
    for k in &default_keys {
        assert!(got["config"].get(k).is_some(), "設定欄位 {k} 沒有出現在 API 回應裡");
    }
    let mut cfg = Config { enabled: true, tag: "[T]".into(), line_enabled: true, line_test_mode: true, line_auto_open: false, line_port: 9191, line_retention_days: 3, ..Config::default() };
    cfg.reference_folders.shared = vec!["/data/faq".into()];
    cfg.reference_folders.internal = vec!["/data/internal".into()];
    cfg.channel_handling.line = Handling::Edit;
    admin.request(Method::PUT, &path, Some(json!({ "config": cfg.clone(), "telegram_default": false }))).await.unwrap();
    let back: ConfigView = admin.get(&path).await.unwrap();
    assert_eq!(serde_json::to_value(&back.config).unwrap(), serde_json::to_value(&cfg).unwrap());
}

#[tokio::test]
async fn secrets_are_write_only_and_never_appear_in_any_response() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let admin = srv.paired("小華", Role::Admin).await;
    let member = srv.paired("小明", Role::Member).await;
    assert_eq!(member.request(Method::PUT, "/v1/secrets/line_channel_secret", Some(json!({"value":"x"}))).await.unwrap_err(), ClientError::Forbidden);
    assert!(admin.request(Method::PUT, "/v1/secrets/whatever", Some(json!({"value":"x"}))).await.is_err(), "只能設定白名單裡的祕密");
    for id in ["line_channel_secret", "line_channel_access_token", "ai_api_key"] {
        admin.request(Method::PUT, &format!("/v1/secrets/{id}"), Some(json!({"value":"SECRET-XYZ-123"}))).await.unwrap();
    }
    let mut all = String::new();
    for p in ["/v1/status", "/v1/ai", &format!("/v1/projects/{pid}/config"), "/v1/projects", "/v1/audit", &format!("/v1/projects/{pid}/senders")] {
        all += &admin.request(Method::GET, p, None).await.unwrap().to_string();
    }
    assert!(!all.contains("SECRET-XYZ-123"), "祕密外洩：{all}");
    let status: serde_json::Value = admin.get("/v1/status").await.unwrap();
    assert_eq!((status["has_line_secret"].as_bool(), status["has_line_token"].as_bool()), (Some(true), Some(true)));
    admin.request(Method::DELETE, "/v1/secrets/line_channel_secret", None).await.unwrap();
    let status: serde_json::Value = admin.get("/v1/status").await.unwrap();
    assert_eq!(status["has_line_secret"].as_bool(), Some(false));
}

#[tokio::test]
async fn items_are_listed_actioned_and_scoped_to_their_project() {
    let srv = TestServer::start().await;
    let (a, b) = (srv.project("甲").await, srv.project("乙").await);
    let member = srv.paired("小明", Role::Member).await;
    let item = add_item(&srv, &a, "line", "line:u0123456789abcdef0123456789abcdef", "電腦壞了", store::PENDING).await;
    let other = add_item(&srv, &b, "line", "line:u0123456789abcdef0123456789abcdee", "別的專案", store::PENDING).await;

    let list: Vec<IntakeItem> = member.get(&format!("/v1/projects/{a}/items")).await.unwrap();
    assert_eq!(list.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec![item.id.as_str()], "只看得到自己專案的題目");
    let list_b: Vec<IntakeItem> = member.get(&format!("/v1/projects/{b}/items")).await.unwrap();
    assert_eq!(list_b.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec![other.id.as_str()]);
    // 不存在的題目一律 404。
    let wrong = member.request(Method::POST, "/v1/items/no-such-item/approve", None).await;
    assert_eq!(wrong.unwrap_err(), ClientError::Server("not found".into()));

    member.request(Method::POST, &format!("/v1/items/{}/approve", item.id), None).await.unwrap();
    let after: IntakeItem = member.get(&format!("/v1/items/{}", item.id)).await.unwrap();
    assert_ne!(after.status, store::PENDING, "核准後不再是待核准");

    let audit: Vec<serde_json::Value> = srv.paired("管理", Role::Admin).await.get("/v1/audit").await.unwrap();
    assert!(audit.iter().any(|e| e["action"] == "item.approve" && e["member"] == "小明" && e["target"] == item.id.as_str()), "{audit:?}");
}

#[tokio::test]
async fn replying_to_a_line_item_pushes_plain_text_and_marks_it_sent() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let line = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v2/bot/message/push")).and(header("authorization", "Bearer TOKEN")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).expect(1).mount(&line).await;
    let srv = TestServer::start().await;
    *srv.state().line_api_base.lock() = line.uri();
    let pid = srv.project("甲").await;
    let admin = srv.paired("小華", Role::Admin).await;
    admin.request(Method::PUT, "/v1/secrets/line_channel_access_token", Some(json!({"value":"TOKEN"}))).await.unwrap();
    let item = add_item(&srv, &pid, "line", "line:u0123456789abcdef0123456789abcdef", "問題", store::DRAFT).await;
    // 還沒有草稿：不能寄。
    assert!(admin.request(Method::POST, &format!("/v1/items/{}/reply", item.id), None).await.is_err());
    admin.request(Method::PUT, &format!("/v1/items/{}/draft", item.id), Some(json!({"draft":"請按 **確定**"}))).await.unwrap();
    admin.request(Method::POST, &format!("/v1/items/{}/reply", item.id), None).await.unwrap();
    let sent = &line.received_requests().await.unwrap()[0];
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(body["messages"][0]["text"], "請按 確定", "Markdown 已轉成純文字");
    let after: IntakeItem = admin.get(&format!("/v1/items/{}", item.id)).await.unwrap();
    assert_eq!(after.status, store::DONE);
}

#[tokio::test]
async fn a_websocket_client_hears_about_changes_and_is_cut_off_when_revoked() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let member = srv.paired("小明", Role::Member).await;
    let admin = srv.paired("小華", Role::Admin).await;
    let mut ws = member.events().await.expect("連上");
    let first = tokio::time::timeout(Duration::from_secs(3), ws.next()).await.unwrap().expect("連上先收到目前 seq");
    assert_eq!(first.r#type, "changed");
    admin.request(Method::POST, "/v1/projects", Some(json!({"name":"新專案"}))).await.unwrap();
    let ev = tokio::time::timeout(Duration::from_secs(3), ws.next()).await.unwrap().expect("資料變動有推播");
    assert!(ev.seq > first.seq);
    let _ = pid;
    // 撤銷成員：連線在幾秒內被關掉，之後的請求也 401。
    aiterm_intake_server::members::revoke(&srv.state().pools.server, "小明", aiterm_intake_server::db::now()).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(6), async { while ws.next().await.is_some() {} }).await;
    assert!(closed.is_ok(), "撤銷後 WebSocket 應該被關閉");
    assert_eq!(member.request(Method::GET, "/v1/me", None).await.unwrap_err(), ClientError::Revoked);
}

#[tokio::test]
async fn a_client_refuses_a_server_whose_certificate_changed() {
    let srv = TestServer::start().await;
    let code = srv.invite("小明", Role::Member).await;
    let p = aiterm_intake_client::pair::pair(&srv.addr(), &code).await.unwrap();
    let api = aiterm_intake_client::api::Api::new(&srv.addr(), &p.token, Some("00".repeat(32))).unwrap();
    assert_eq!(api.request(Method::GET, "/v1/me", None).await.unwrap_err(), ClientError::FingerprintChanged);
}

#[tokio::test]
async fn the_version_handshake_accepts_this_protocol() {
    let srv = TestServer::start().await;
    let m = srv.paired("小明", Role::Member).await;
    let hello = m.check_version().await.unwrap();
    assert_eq!(hello.protocol, aiterm_intake_core::api::PROTOCOL);
}

#[tokio::test]
async fn senders_labels_and_project_management_round_trip() {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let created: serde_json::Value = admin.typed(Method::POST, "/v1/projects", Some(json!({"name":"範例科技","work_dir":""}))).await.unwrap();
    let pid = created["id"].as_str().unwrap().to_string();
    let s = SenderDto { address: "line:c0123456789abcdef0123456789abcdef".into(), level: Level::External, skip_auth: false, label: "客服群".into() };
    admin.request(Method::PUT, &format!("/v1/projects/{pid}/senders"), Some(serde_json::to_value(&s).unwrap())).await.unwrap();
    let list: Vec<SenderDto> = admin.get(&format!("/v1/projects/{pid}/senders")).await.unwrap();
    assert_eq!(list, vec![s.clone()]);
    let del: bool = admin.typed(Method::DELETE, &format!("/v1/projects/{pid}/senders"), Some(json!({"address": s.address}))).await.unwrap();
    assert!(del);
    admin.request(Method::PUT, &format!("/v1/projects/{pid}"), Some(json!({"name":"改名","work_dir":"/srv/x"}))).await.unwrap();
    let projects: Vec<serde_json::Value> = admin.get("/v1/projects").await.unwrap();
    assert_eq!((projects[0]["name"].as_str(), projects[0]["work_dir"].as_str()), (Some("改名"), Some("/srv/x")));
    admin.request(Method::DELETE, &format!("/v1/projects/{pid}"), None).await.unwrap();
    assert_eq!(admin.get::<Vec<serde_json::Value>>("/v1/projects").await.unwrap().len(), 0);
}

#[tokio::test]
async fn ai_settings_are_admin_only_and_report_whether_ai_is_usable() {
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let member = srv.paired("小明", Role::Member).await;
    let body = json!({"provider":"openai","model":"gpt-x","base_url":""});
    assert_eq!(member.request(Method::PUT, "/v1/ai", Some(body.clone())).await.unwrap_err(), ClientError::Forbidden);
    admin.request(Method::PUT, "/v1/ai", Some(body)).await.unwrap();
    assert_eq!(member.get::<serde_json::Value>("/v1/ai").await.unwrap()["configured"], false, "有供應商但沒金鑰仍是未設定");
    admin.request(Method::PUT, "/v1/secrets/ai_api_key", Some(json!({"value":"K"}))).await.unwrap();
    assert_eq!(member.get::<serde_json::Value>("/v1/ai").await.unwrap()["configured"], true);
    assert_eq!(member.get::<serde_json::Value>("/v1/status").await.unwrap()["ai_configured"], true);
    assert!(admin.request(Method::PUT, "/v1/ai", Some(json!({"provider":"banana","model":"m"}))).await.is_err());
}

#[tokio::test]
async fn no_data_endpoint_answers_without_a_token() {
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let verifier = aiterm_intake_client::pin::PinVerifier::new(None);
    let http = reqwest::Client::builder().use_preconfigured_tls(aiterm_intake_client::pin::client_config(verifier)).build().unwrap();
    let base = format!("https://{}", srv.addr());
    let routes = [
        ("GET", "/v1/me".to_string()),
        ("GET", "/v1/status".into()),
        ("GET", "/v1/audit".into()),
        ("GET", "/v1/ai".into()),
        ("PUT", "/v1/ai".into()),
        ("PUT", "/v1/secrets/ai_api_key".into()),
        ("GET", "/v1/projects".into()),
        ("POST", "/v1/projects".into()),
        ("GET", format!("/v1/projects/{pid}/config")),
        ("PUT", format!("/v1/projects/{pid}/config")),
        ("GET", format!("/v1/projects/{pid}/senders")),
        ("GET", format!("/v1/projects/{pid}/items")),
        ("GET", "/v1/items/x".to_string()),
        ("POST", "/v1/items/x/approve".to_string()),
        ("POST", "/v1/items/x/reply".to_string()),
        ("POST", "/v1/items/x/claim".to_string()),
        ("GET", "/v1/line/unknown".into()),
        ("GET", "/v1/events".into()),
    ];
    for (m, p) in routes {
        let res = http.request(m.parse().unwrap(), format!("{base}{p}")).json(&json!({})).send().await.unwrap();
        assert_eq!(res.status(), 401, "{m} {p} 沒有 token 也回應了");
    }
}

#[tokio::test]
async fn attachments_are_read_by_id_only_from_the_servers_attachment_folder() {
    use aiterm_intake_core::attach::{save_all, Raw};
    let srv = TestServer::start().await;
    let pid = srv.project("甲").await;
    let m = srv.paired("小明", Role::Member).await;
    let item = add_item(&srv, &pid, "line", "line:u0123456789abcdef0123456789abcdef", "附檔", store::PENDING).await;
    let root = srv.dir.path().join("attachments");
    let (saved, _) = save_all(&root, &item.id, vec![Raw { name: "err.log".into(), bytes: "ERROR boom\n".as_bytes().to_vec() }]);
    store::add_attachment(&srv.state().pools.intake, &item.id, &saved[0]).await.unwrap();

    let rows: Vec<store::AttachmentRow> = m.get(&format!("/v1/items/{}/attachments", item.id)).await.unwrap();
    assert_eq!(rows.len(), 1);
    let text: String = m.get(&format!("/v1/attachments/{}/text", rows[0].id)).await.unwrap();
    assert_eq!(text, "ERROR boom\n");
    assert_eq!(m.request(Method::GET, "/v1/attachments/nope/text", None).await.unwrap_err(), ClientError::Server("not found".into()));

    // 資料庫被竄改、指到目錄外的檔案：不能讀。
    let outside = srv.dir.path().join("secrets.key");
    sqlx::query("UPDATE intake_attachments SET path = ? WHERE id = ?").bind(outside.to_string_lossy().as_ref()).bind(&rows[0].id).execute(&srv.state().pools.intake).await.unwrap();
    assert!(m.request(Method::GET, &format!("/v1/attachments/{}/text", rows[0].id), None).await.is_err(), "目錄外的檔案不能讀");
}

#[tokio::test]
async fn a_self_signed_server_gets_its_fingerprint_pinned() {
    let srv = TestServer::start().await;
    let fp = srv.state().fingerprint.clone();
    assert_eq!(aiterm_intake_client::api::decide_pin(&srv.addr(), &fp).await, Some(fp), "自簽憑證過不了一般驗證，所以要釘指紋");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ai_models_and_connection_test_work_on_unsaved_values_and_only_for_admins() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let up = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).and(header("authorization", "Bearer typed-key")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[{"id":"m2"},{"id":"m1"}]}))).mount(&up).await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).and(header("authorization", "Bearer saved-key")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"choices":[{"message":{"content":"OK"}}]}))).mount(&up).await;
    let srv = TestServer::start().await;
    let admin = srv.paired("小華", Role::Admin).await;
    let member = srv.paired("小明", Role::Member).await;
    let probe = |key: &str| serde_json::json!({"provider": "openai", "model": "m1", "base_url": up.uri(), "api_key": key});

    let models: Vec<String> = admin.typed(Method::POST, "/v1/ai/models", Some(probe("typed-key"))).await.unwrap();
    assert_eq!(models, vec!["m1", "m2"], "畫面上剛輸入、還沒儲存的金鑰也能拿來列模型");
    assert!(admin.request(Method::POST, "/v1/ai/models", Some(probe("wrong"))).await.is_err(), "金鑰錯誤要回錯誤");

    admin.request(Method::PUT, "/v1/secrets/ai_api_key", Some(serde_json::json!({"value": "saved-key"}))).await.unwrap();
    let r: serde_json::Value = admin.typed(Method::POST, "/v1/ai/test", Some(probe(""))).await.unwrap();
    assert_eq!((r["ok"].as_bool(), r["message"].as_str()), (Some(true), Some("OK")), "金鑰留空就用已存的");
    let bad: serde_json::Value = admin.typed(Method::POST, "/v1/ai/test", Some(probe("nope"))).await.unwrap();
    assert_eq!(bad["ok"], false);

    assert!(member.request(Method::POST, "/v1/ai/test", Some(probe(""))).await.is_err());
    assert!(member.request(Method::POST, "/v1/ai/models", Some(probe(""))).await.is_err());
    let view: serde_json::Value = admin.get("/v1/ai").await.unwrap();
    assert!(view["health"].is_object());
}
