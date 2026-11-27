//! 中途問人（server）：`ask_human` 掛起、成員回答、管理員取消、權限、卡結束與重啟的收尾。
use aiterm_intake_client::api::Api;
use aiterm_intake_core::api::Role;
use aiterm_intake_core::board_questions as bq;
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
}

async fn rig() -> R {
    let srv = TestServer::start().await;
    let (admin, member) = (srv.paired("小華", Role::Admin).await, srv.paired("小明", Role::Member).await);
    let pid = srv.project("範例科技").await;
    R { srv, admin, member, pid }
}

impl R {
    /// 為一張假想的卡註冊 MCP token，回傳（問人工具的網址, guard）。
    fn mcp(&self, card: &str) -> (String, dbmcp::McpGuard) {
        let ctx = McpCtx { card_id: card.into(), agent_id: Some("a1".into()), project_id: self.pid.clone(), skill: None, systems: vec![] };
        let guard = dbmcp::register(self.srv.state(), ctx, &self.srv.dir.path().join(format!("run/{card}"))).expect("MCP 端點要已啟動");
        let cfg: Value = serde_json::from_str(&std::fs::read_to_string(&guard.config_path).unwrap()).unwrap();
        (cfg["mcpServers"]["aiterm-ask"]["url"].as_str().expect("每張卡都有問人端點").to_string(), guard)
    }
    async fn open(&self) -> Vec<Value> {
        self.member.request(Method::GET, &format!("/v1/board/projects/{}/questions", self.pid), None).await.unwrap()["open"].as_array().unwrap().clone()
    }
    async fn wait_open(&self) -> Value {
        for _ in 0..40 {
            if let Some(q) = self.open().await.first() {
                return q.clone();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("問題沒有出現在待回答清單");
    }
}

async fn call(url: &str, args: Value) -> (String, bool) {
    let r = reqwest::Client::new().post(url).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "ask_human", "arguments": args}})).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"].as_bool().unwrap_or(false))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_ask_endpoint_lists_the_tool_and_every_card_gets_it() {
    let r = rig().await;
    let (url, _g) = r.mcp("c1");
    let v: Value = reqwest::Client::new().post(&url).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["result"]["tools"][0]["name"], "ask_human");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_member_answer_wakes_the_hanging_call_and_becomes_the_tool_result() {
    let r = rig().await;
    let (url, _g) = r.mcp("c1");
    let task = tokio::spawn(async move { call(&url, json!({"question": "用 A 還是 B？", "options": ["A", "B"], "recommended": "A"})).await });
    let q = r.wait_open().await;
    assert_eq!((q["question"].as_str(), q["options"].as_str(), q["card_id"].as_str()), (Some("用 A 還是 B？"), Some("[\"A\",\"B\"]"), Some("c1")));
    assert!(!task.is_finished(), "沒人回答前工具呼叫要一直掛著");
    let id = q["id"].as_i64().unwrap();
    r.member.request(Method::POST, &format!("/v1/board/questions/{id}/answer"), Some(json!({"answer": "B"}))).await.unwrap();
    let (text, is_err) = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert!(!is_err, "{text}");
    assert!(text.contains("B"), "{text}");
    assert!(r.open().await.is_empty());
    // 第二個人再答：已經答過了。
    let again = r.admin.request(Method::POST, &format!("/v1/board/questions/{id}/answer"), Some(json!({"answer": "A"}))).await;
    assert!(again.is_err(), "已回答的問題不能再被回答");
    let row = bq::get(&r.srv.state().pools.intake, id).await.unwrap().unwrap();
    assert_eq!((row.answer.as_deref(), row.answered_by.as_deref(), row.answered_via.as_deref()), (Some("B"), Some("小明"), Some("app")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_an_admin_can_cancel_and_the_call_returns_a_cancelled_notice() {
    let r = rig().await;
    let (url, _g) = r.mcp("c1");
    let task = tokio::spawn(async move { call(&url, json!({"question": "要繼續嗎？"})).await });
    let id = r.wait_open().await["id"].as_i64().unwrap();
    assert!(r.member.request(Method::POST, &format!("/v1/board/questions/{id}/cancel"), None).await.is_err(), "成員不能取消");
    r.admin.request(Method::POST, &format!("/v1/board/questions/{id}/cancel"), None).await.unwrap();
    let (text, is_err) = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert!(is_err && text.contains("取消"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_card_that_ends_cancels_its_waiting_question_and_the_call_returns() {
    let r = rig().await;
    let (url, guard) = r.mcp("c1");
    let task = tokio::spawn(async move { call(&url, json!({"question": "還在嗎？"})).await });
    let id = r.wait_open().await["id"].as_i64().unwrap();
    drop(guard);
    let (text, is_err) = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert!(is_err, "{text}");
    for _ in 0..30 {
        if bq::get(&r.srv.state().pools.intake, id).await.unwrap().unwrap().status == bq::CANCELLED {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("卡結束後問題要被標成取消");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_questions_are_refused_and_a_card_cannot_have_two_open_questions() {
    let r = rig().await;
    let (url, _g) = r.mcp("c1");
    let (text, is_err) = call(&url, json!({"question": "  "})).await;
    assert!(is_err, "{text}");
    let (u2, _g2) = (url.clone(), ());
    let first = tokio::spawn(async move { call(&u2, json!({"question": "第一個"})).await });
    r.wait_open().await;
    let (text, is_err) = call(&url, json!({"question": "第二個"})).await;
    assert!(is_err && text.contains("已經有一個"), "{text}");
    first.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_answers_are_refused_and_admin_settings_validate_the_email() {
    let r = rig().await;
    let (url, _g) = r.mcp("c1");
    let t = tokio::spawn(async move { call(&url, json!({"question": "問"})).await });
    let id = r.wait_open().await["id"].as_i64().unwrap();
    assert!(r.member.request(Method::POST, &format!("/v1/board/questions/{id}/answer"), Some(json!({"answer": "  "}))).await.is_err());
    t.abort();
    assert!(r.member.request(Method::GET, "/v1/ask/settings", None).await.is_err(), "設定只有管理員能看");
    assert!(r.admin.request(Method::PUT, "/v1/ask/settings", Some(json!({"notify_email": "不是信箱"}))).await.is_err());
    r.admin.request(Method::PUT, "/v1/ask/settings", Some(json!({"notify_email": "boss@example.com"}))).await.unwrap();
    assert_eq!(r.admin.request(Method::GET, "/v1/ask/settings", None).await.unwrap()["notify_email"], "boss@example.com");
}

// ── 通知：Telegram（假 API）、Email（mail_tap）、提醒 ─────────────────────────
mod notify {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TOKEN: &str = "123:ABC";

    async fn eventually<T, F: std::future::Future<Output = Option<T>>>(mut f: impl FnMut() -> F) -> T {
        for _ in 0..100 {
            if let Some(v) = f().await {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("等不到結果");
    }

    async fn requests_to(tg: &MockServer, api: &str) -> Vec<Value> {
        let want = format!("/bot{TOKEN}/{api}");
        tg.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == want).map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null)).collect()
    }

    async fn rig_tg(tg: &MockServer) -> R {
        let r = rig().await;
        *r.srv.state().telegram_api_base.lock() = tg.uri();
        r.admin.request(Method::PUT, "/v1/telegram", Some(json!({"owner_chat_id": "111"}))).await.unwrap();
        r.admin.request(Method::PUT, "/v1/secrets/telegram_bot_token", Some(json!({"value": TOKEN}))).await.unwrap();
        Mock::given(method("POST")).and(path(format!("/bot{TOKEN}/sendMessage"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": {"message_id": 900}}))).mount(tg).await;
        Mock::given(method("POST")).and(path(format!("/bot{TOKEN}/editMessageText"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": true}))).mount(tg).await;
        Mock::given(method("POST")).and(path(format!("/bot{TOKEN}/answerCallbackQuery"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": true}))).mount(tg).await;
        r
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_new_question_is_sent_to_the_owner_with_option_buttons_and_the_message_is_remembered() {
        let tg = MockServer::start().await;
        let r = rig_tg(&tg).await;
        let (url, _g) = r.mcp("c1");
        let t = tokio::spawn(async move { call(&url, json!({"question": "用哪個？", "options": ["甲", "乙"]})).await });
        let q = r.wait_open().await;
        let sent = eventually(|| async { Some(requests_to(&tg, "sendMessage").await).filter(|v| !v.is_empty()) }).await;
        assert_eq!(sent[0]["chat_id"], 111);
        assert!(sent[0]["text"].as_str().unwrap().contains("用哪個？"));
        assert_eq!(sent[0]["reply_markup"]["inline_keyboard"][1][0]["text"], "乙");
        let id = q["id"].as_i64().unwrap();
        eventually(|| async { bq::get(&r.srv.state().pools.intake, id).await.unwrap().unwrap().telegram_message_id.filter(|m| *m == 900) }).await;
        t.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_owner_answers_with_a_button_press_and_strangers_cannot() {
        let tg = MockServer::start().await;
        let r = rig_tg(&tg).await;
        let (url, _g) = r.mcp("c1");
        let t = tokio::spawn(async move { call(&url, json!({"question": "用哪個？", "options": ["甲", "乙"]})).await });
        let id = r.wait_open().await["id"].as_i64().unwrap();
        let st = r.srv.state().clone();
        let cb = |chat: i64| json!({"id": "cb1", "data": format!("q|{id}|1"), "message": {"message_id": 900, "chat": {"id": chat}}});
        assert!(aiterm_intake_server::asknotify::handle_callback(&st, &tg.uri(), TOKEN, Some(111), &cb(999)).await);
        assert_eq!(bq::get(&st.pools.intake, id).await.unwrap().unwrap().status, bq::OPEN, "陌生聊天室按按鈕沒有效果");
        assert!(aiterm_intake_server::asknotify::handle_callback(&st, &tg.uri(), TOKEN, Some(111), &cb(111)).await);
        let (text, is_err) = tokio::time::timeout(Duration::from_secs(5), t).await.unwrap().unwrap();
        assert!(!is_err && text.contains("乙"), "{text}");
        let row = bq::get(&st.pools.intake, id).await.unwrap().unwrap();
        assert_eq!((row.answered_via.as_deref(), row.answered_by.as_deref()), (Some("telegram"), Some("Telegram（主人）")));
        let edits = eventually(|| async { Some(requests_to(&tg, "editMessageText").await).filter(|v| !v.is_empty()) }).await;
        assert!(edits[0]["text"].as_str().unwrap().contains("已回答"), "按完後訊息改成結果");
        assert!(!aiterm_intake_server::asknotify::handle_callback(&st, &tg.uri(), TOKEN, Some(111), &json!({"id": "x", "data": "別人的資料"})).await, "不是問題的按鈕不處理");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reply_to_the_question_message_answers_it_but_only_from_the_owner() {
        let tg = MockServer::start().await;
        let r = rig_tg(&tg).await;
        let (url, _g) = r.mcp("c1");
        let t = tokio::spawn(async move { call(&url, json!({"question": "寫什麼名字？"})).await });
        let id = r.wait_open().await["id"].as_i64().unwrap();
        let st = r.srv.state().clone();
        eventually(|| async { bq::get(&st.pools.intake, id).await.unwrap().unwrap().telegram_message_id }).await;
        assert!(aiterm_intake_server::asknotify::handle_reply(&st, Some(111), 999, Some(900), "駭客").await, "是問題的回覆就不當成新題目");
        assert_eq!(bq::get(&st.pools.intake, id).await.unwrap().unwrap().status, bq::OPEN);
        assert!(!aiterm_intake_server::asknotify::handle_reply(&st, Some(111), 111, Some(12345), "不相干的回覆").await, "回覆別的訊息不是答案");
        assert!(aiterm_intake_server::asknotify::handle_reply(&st, Some(111), 111, Some(900), "小花").await);
        let (text, _) = tokio::time::timeout(Duration::from_secs(5), t).await.unwrap().unwrap();
        assert!(text.contains("小花"), "{text}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn email_is_notified_when_an_address_is_set_and_reminders_stop_after_three() {
        let r = rig().await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        *r.srv.state().mail_tap.lock() = Some(tx);
        r.admin.request(Method::PUT, "/v1/ask/settings", Some(json!({"notify_email": "boss@example.com"}))).await.unwrap();
        let (url, _g) = r.mcp("c1");
        let t = tokio::spawn(async move { call(&url, json!({"question": "要不要繼續？"})).await });
        let id = r.wait_open().await["id"].as_i64().unwrap();
        let (to, subject, body) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert_eq!(to, "boss@example.com");
        assert!(subject.contains(&format!("Q-{id}")) && body.contains("要不要繼續？") && body.contains("不能回覆"), "{subject} / {body}");
        // 提醒：剛問完不提醒；把提問時間往前撥 31 分鐘才提醒，最多 3 次。
        let st = r.srv.state().clone();
        assert_eq!(aiterm_intake_server::asknotify::remind_once(&st).await, 0);
        for round in 1..=4 {
            sqlx::query("UPDATE board_questions SET created_at = created_at - 100000, last_reminded_at = CASE WHEN last_reminded_at IS NULL THEN NULL ELSE last_reminded_at - 100000 END WHERE id = ?").bind(id).execute(&st.pools.intake).await.unwrap();
            let n = aiterm_intake_server::asknotify::remind_once(&st).await;
            assert_eq!(n, if round <= 3 { 1 } else { 0 }, "第 {round} 輪");
        }
        t.abort();
    }

    /// 沒有任何專案啟用 Telegram 時，只要設了主人聊天室，輪詢仍然要收按鈕回呼與回覆。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn polling_handles_callbacks_and_replies_even_when_no_project_uses_telegram() {
        let tg = MockServer::start().await;
        let r = rig_tg(&tg).await;
        let (url, _g) = r.mcp("c1");
        let t = tokio::spawn(async move { call(&url, json!({"question": "用哪個？", "options": ["甲", "乙"]})).await });
        let id = r.wait_open().await["id"].as_i64().unwrap();
        let st = r.srv.state().clone();
        eventually(|| async { bq::get(&st.pools.intake, id).await.unwrap().unwrap().telegram_message_id }).await;
        let update = json!({"update_id": 1, "callback_query": {"id": "cb", "data": format!("q|{id}|0"), "message": {"message_id": 900, "chat": {"id": 111}}}});
        Mock::given(method("GET")).and(path(format!("/bot{TOKEN}/getUpdates"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": [update]}))).mount(&tg).await;
        let mut offset = 0;
        aiterm_intake_server::telegram::poll_once(&st, &mut offset).await.unwrap();
        let (text, is_err) = tokio::time::timeout(Duration::from_secs(5), t).await.unwrap().unwrap();
        assert!(!is_err && text.contains("甲"), "{text}");
        let polled = tg.received_requests().await.unwrap().into_iter().find(|r| r.url.path().ends_with("/getUpdates")).unwrap();
        assert!(polled.url.query().unwrap().contains("callback_query"), "要向 Telegram 要 callback_query 這種更新");
    }
}
