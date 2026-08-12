use wiremock::matchers::{header, headers, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::upstream::anthropic::{AnthropicUpstream, ClientHeaders};
use aiterm_lib::bridge::upstream::UpstreamResponse;

#[tokio::test]
async fn api_key_mode_sets_x_api_key_and_passes_body_through() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({
        "model": "aiterm:sonnet", "stream": true, "messages": [],
        // 我們沒解析的欄位必須原樣送達。
        "metadata": {"user_id": "x"}
    });
    let resp = up.send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default()).await.unwrap();
    match resp {
        UpstreamResponse::Passthrough(r) => {
            let body = r.text().await.unwrap();
            assert!(body.contains("message_stop"));
        }
        _ => panic!("Anthropic 路徑必須回 Passthrough"),
    }
}

#[tokio::test]
async fn oauth_mode_sets_bearer_and_beta_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer sk-ant-oat-x"))
        // wiremock 的 `header()` 比對時，實際請求裡逗號分隔的值會被拆成多個
        // element 再比對，但期望值不會被拆；單一 `header()` 因此永遠比對不到
        // 這個逗號分隔的值，改用 `headers()` 帶拆好的兩個 element。
        .and(headers("anthropic-beta", vec!["claude-code-20250219", "oauth-2025-04-20"]))
        .and(header("x-app", "cli"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: {}\n\n", "text/event-stream"))
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-oat-x".into(), true);
    let resp = up
        .send_raw(&serde_json::json!({"model": "m", "messages": []}), "m", &ClientHeaders::default())
        .await;
    assert!(resp.is_ok());
}

#[tokio::test]
async fn oauth_mode_merges_client_beta_flags_instead_of_overwriting() {
    // 真實 Claude Code CLI 送 context_management 欄位時會在自己的
    // anthropic-beta 裡宣告對應的旗標；轉發時若整組覆蓋，上游會因為看到
    // 沒宣告 beta 卻出現該欄位而 400。必須合併：我們必需的兩個 + 客戶端的。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(headers(
            "anthropic-beta",
            vec!["claude-code-20250219", "oauth-2025-04-20", "context-management-2025-06-27"],
        ))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: {}\n\n", "text/event-stream"))
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-oat-x".into(), true);
    let client = ClientHeaders {
        beta: Some("context-management-2025-06-27".into()),
        version: None,
    };
    let resp = up
        .send_raw(&serde_json::json!({"model": "m", "messages": []}), "m", &client)
        .await;
    assert!(resp.is_ok());
}

#[tokio::test]
async fn client_supplied_anthropic_version_is_kept_verbatim() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("anthropic-version", "2024-01-01"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let client = ClientHeaders { beta: None, version: Some("2024-01-01".into()) };
    let resp = up
        .send_raw(&serde_json::json!({"model": "m", "messages": []}), "m", &client)
        .await;
    assert!(resp.is_ok());
}

// 橋接是原樣轉發 Claude Code 的 JSON，只把 model 換成映射後的真模型。但
// Claude Code 送出的參數是依「它以為自己在用的模型」決定的——使用者 /model
// 選 aiterm:opus 時它會啟用 adaptive thinking，而真正收到請求的可能是 Sonnet，
// 上游就回 400「adaptive thinking is not supported on this model」（實測）。
//
// 用能力表去擋會過期，所以改成：上游明確指出某參數不支援時，拿掉它重試一次。
#[tokio::test]
async fn unsupported_thinking_param_is_dropped_and_retried() {
    let server = MockServer::start().await;

    // 第一次：帶著 thinking → 400
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(wiremock::matchers::body_string_contains("\"thinking\""))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"adaptive thinking is not supported on this model"}}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    // 第二次：thinking 已被移除 → 200
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({
        "model": "aiterm:opus",
        "stream": true,
        "messages": [],
        "thinking": {"type": "adaptive"}
    });
    let resp = up
        .send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default())
        .await
        .expect("移除不支援的參數後應該要成功");

    match resp {
        UpstreamResponse::Passthrough(r) => {
            assert!(r.text().await.unwrap().contains("message_stop"));
        }
        _ => panic!("Anthropic 路徑必須回 Passthrough"),
    }
}

// 不相干的 400 不能被這條規則吞掉，也不可以無謂重試。
#[tokio::test]
async fn unrelated_400_is_returned_without_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages: at least one message is required"}}"#,
        ))
        .expect(1) // 只能送一次
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({"model": "m", "messages": [], "thinking": {"type": "adaptive"}});
    let err = up.send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default()).await;
    assert!(err.is_err(), "不相干的 400 應該照樣是錯誤");
}

// 實測 log：拿掉 thinking 之後，上游改抱怨 effort——Claude Code 會同時送多個
// Opus 級專屬參數。只重試一次的話，整體仍然失敗，只是失敗原因換了一個。
#[tokio::test]
async fn keeps_stripping_until_upstream_stops_complaining() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(wiremock::matchers::body_string_contains("\"thinking\""))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":{"message":"adaptive thinking is not supported on this model"}}"#,
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(wiremock::matchers::body_string_contains("\"effort\""))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":{"message":"This model does not support the effort parameter."}}"#,
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({
        "model": "aiterm:opus",
        "messages": [],
        "thinking": {"type": "adaptive"},
        // 實測到的真實形狀：effort 在 output_config 底下，不是頂層。原本這裡是
        // 我假設的 "effort": "high"，被 log 的欄位輪廓推翻了。
        "output_config": {"effort": "high"}
    });
    let resp = up
        .send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default())
        .await
        .expect("兩個參數都剝掉之後應該要成功");
    assert!(matches!(resp, UpstreamResponse::Passthrough(_)));
    // 三次：原始、剝掉一個、再剝掉一個。
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

// 上游一直抱怨也不能無限重試。
#[tokio::test]
async fn stops_retrying_when_no_rule_applies() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":{"message":"adaptive thinking is not supported on this model"}}"#,
        ))
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    // thinking 只有一個，剝掉之後就沒有規則可套用了 → 第二次 400 直接回錯。
    let raw = serde_json::json!({"model": "m", "messages": [], "thinking": {"type": "adaptive"}});
    assert!(up.send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default()).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 2, "不可以無限重試");
}

// 實測欄位輪廓：model messages system tools metadata{user_id} max_tokens
// output_config{effort} context_management{edits}
// —— 要剝的東西不一定在頂層，規則得吃路徑。
#[tokio::test]
async fn strips_nested_param_and_drops_the_emptied_parent() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(wiremock::matchers::body_string_contains("output_config"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":{"message":"This model does not support the effort parameter."}}"#,
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({
        "model": "aiterm:opus",
        "messages": [],
        "output_config": {"effort": "high"}
    });
    up.send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default())
        .await
        .expect("剝掉 output_config.effort 之後應該要成功");

    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 2);
    let second: serde_json::Value = serde_json::from_slice(&reqs[1].body).unwrap();
    // 只剩空殼的 output_config 也要拿掉——空物件同樣可能被上游拒絕。
    assert!(second.get("output_config").is_none(), "清空後的父層應該一併移除");
}

// 實測：上游回「role 'system' is not supported on this model」。Anthropic 的
// Messages API 不接受 messages 裡有 role:"system"，system 只能放頂層——
// ai/anthropic.rs 那條路徑本來就會濾掉並摺進頂層，但橋接的原樣轉發沒做。
#[tokio::test]
async fn system_role_messages_are_folded_into_the_top_level_system_field() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({
        "model": "aiterm:opus",
        "system": "原本的系統提示",
        "messages": [
            {"role": "system", "content": "被放錯位置的系統訊息"},
            {"role": "user", "content": "你好"}
        ]
    });
    up.send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default()).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();

    let roles: Vec<&str> = sent["messages"].as_array().unwrap().iter()
        .map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, vec!["user"], "messages 裡不可以留下 system 角色");

    let system_text = serde_json::to_string(&sent["system"]).unwrap();
    assert!(system_text.contains("原本的系統提示"), "原本的系統提示不能弄丟");
    assert!(system_text.contains("被放錯位置的系統訊息"), "摺進來的內容不能弄丟");
}

// OAuth 模式下 Claude Code 哨兵必須留在第一塊，摺進來的內容接在後面。
#[tokio::test]
async fn folding_keeps_the_sentinel_first_in_oauth_mode() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-oat-test".into(), true);
    let raw = serde_json::json!({
        "model": "aiterm:opus",
        "messages": [{"role": "system", "content": "折進來的"}, {"role": "user", "content": "hi"}]
    });
    up.send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default()).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let blocks = sent["system"].as_array().expect("OAuth 模式的 system 是 block 陣列");
    assert!(
        blocks[0]["text"].as_str().unwrap().starts_with("You are Claude Code"),
        "哨兵必須是第一塊，否則上游會回假的 rate_limit_error"
    );
    assert!(
        serde_json::to_string(&blocks).unwrap().contains("折進來的"),
        "摺進來的內容不能弄丟"
    );
}

// 剝掉 thinking 會引發連鎖：context_management.edits 裡的 clear_thinking 策略
// 依賴 thinking 存在，上游因此回「`clear_thinking_20251015` strategy requires
// `thinking` to be enabled or adaptive」（實測）。依賴項也要跟著移除。
#[tokio::test]
async fn removes_the_edit_strategy_that_depended_on_the_stripped_thinking() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(wiremock::matchers::body_string_contains("clear_thinking"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":{"message":"`clear_thinking_20251015` strategy requires `thinking` to be enabled or adaptive"}}"#,
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({
        "model": "aiterm:opus",
        "messages": [],
        "context_management": {
            "edits": [
                {"type": "clear_thinking_20251015"},
                {"type": "keep_me_20240101"}
            ]
        }
    });
    up.send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default())
        .await
        .expect("移除依賴 thinking 的策略後應該要成功");

    let reqs = server.received_requests().await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&reqs[1].body).unwrap();
    let edits = sent["context_management"]["edits"].as_array().expect("其他策略要留著");
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["type"], "keep_me_20240101", "只該移除依賴 thinking 的那一個");
}

// 陣列被清空時，連同空殼的父層一起移除。
#[tokio::test]
async fn drops_context_management_when_its_only_edit_is_removed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(wiremock::matchers::body_string_contains("clear_thinking"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":{"message":"`clear_thinking_20251015` strategy requires `thinking` to be enabled or adaptive"}}"#,
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({
        "model": "m",
        "messages": [],
        "context_management": {"edits": [{"type": "clear_thinking_20251015"}]}
    });
    up.send_raw(&raw, "claude-sonnet-4-5", &ClientHeaders::default()).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&reqs[1].body).unwrap();
    assert!(sent.get("context_management").is_none(), "空掉的父層要一併移除");
}
