//! Claude Code 橋接的端到端整合測試：真的 TCP listener、真的 HTTP 請求、
//! 真的（wiremock）上游、真的 SSE 輸出。
//!
//! 既有整合測試都是分段的：`bridge_server.rs` 用 tower oneshot 直接打
//! router（不啟動真的 listener），`bridge_openai_upstream.rs` /
//! `bridge_anthropic_upstream.rs` 只測單一 adapter。這支測試補上「一條龍」
//! 的涵蓋：`BridgeState::start()` 啟動的真 server → `reqwest` 發真請求 →
//! wiremock 扮演上游 → 逐 byte 讀回真的 SSE frame。
//!
//! 計畫（`docs/superpowers/plans/`）的最後一步驗收是手動的（需要 GUI 開分頁
//! 跑真的 `claude` CLI），這支測試把其中能自動化的部分（工具呼叫序列、
//! tool_result 回送、授權、映射錯誤、上游錯誤、ping keepalive）收斂成
//! CI 能跑的斷言。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::BridgeState;
use aiterm_lib::config::types::{ProviderConfig, ProviderType, TierMapping};
use aiterm_lib::config::ConfigStore;
use aiterm_lib::secret::SecretStore;

const TOKEN: &str = "e2e-test-token";

// ── 測試骨架 ────────────────────────────────────────────────────────────

/// 借一個目前沒人用的埠再立刻釋放。
///
/// `BridgeState::start` 收固定的 `u16`（見 `src/bridge/mod.rs` 的註解：埠
/// 若會漂移，已開的分頁會指向死位址，所以刻意不做「被占用就換一個」），
/// 測試若寫死埠會在平行執行（`cargo test` 預設多執行緒跑各測試函式）時
/// 互撞。改成「先 bind `:0` 問 OS 要一個空埠、立刻釋放、再讓
/// `BridgeState` 綁上同一個埠」：釋放到重新綁定之間有極小的 race window
/// 理論上可能被别的行程搶走，但這只是本機/CI 測試環境，機率低到可以接受
/// ——比起寫死一批高位埠去賭它們沒被別的服務占用，這個做法更不脆弱。
async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("borrow a free port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// 啟動一個真的橋接 server。`sonnet_upstream` 給 `Some(wiremock_uri)` 時會把
/// `aiterm:sonnet` 映射到一個指向該 URI 的 OpenAI 型供應商；給 `None` 時
/// 保留 `claude_bridge` 為預設值（不映射任何層級），用於「未映射層級」
/// 與「授權」測試 —— 這兩種情境走不到上游，不需要真的假供應商。
///
/// `SecretStore::new()` 是真的 OS keychain wrapper，但這裡只會呼叫
/// `get()`：`factory::build` 對找不到的 key 做 `.ok().flatten().unwrap_or_default()`，
/// 讀不到就退回空字串，OpenAI 路徑的假上游本來就不驗金鑰。全程沒有任何
/// `set`/`delete` 呼叫，不會在使用者的鑰匙圈裡留下或動到任何東西。
async fn start_bridge(dir: &tempfile::TempDir, sonnet_upstream: Option<&str>) -> (BridgeState, String) {
    let config = ConfigStore::new_at(dir.path().join("config.toml"));
    if let Some(base) = sonnet_upstream {
        config
            .update(|cfg| {
                cfg.claude_bridge.sonnet = Some(TierMapping {
                    provider_id: "wm-openai".into(),
                    model: "gpt-4o-mini".into(),
                });
                cfg.providers.push(ProviderConfig {
                    id: "wm-openai".into(),
                    display_name: "Wiremock OpenAI".into(),
                    provider_type: ProviderType::Openai,
                    base_url: Some(base.to_string()),
                    oauth_client_id: None,
                    model: "gpt-4o-mini".into(),
                    supports_json_mode: true,
                    auth_method: None,
                });
            })
            .expect("寫入暫存 config 不應失敗");
    }

    let bridge = BridgeState::new();
    let port = free_port().await;
    bridge
        .start(Arc::new(config), Arc::new(SecretStore::new()), TOKEN.to_string(), port)
        .await
        .expect("bridge 啟動不應失敗");
    (bridge, format!("http://127.0.0.1:{port}"))
}

fn simple_request() -> Value {
    json!({
        "model": "aiterm:sonnet",
        "stream": true,
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "列出這個目錄的檔案"}]
    })
}

async fn post_messages(base: &str, token: Option<&str>, body: &Value) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("{base}/v1/messages"))
        .json(body);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.expect("請求應該送達（連線層級失敗才會是 Err）")
}

/// 解析一個完整 SSE frame（不含結尾 `\n\n`）：`event: X\ndata: {...}`。
fn parse_frame(raw: &str) -> Option<(String, Value)> {
    let mut event = None;
    let mut data = None;
    for line in raw.lines() {
        if let Some(e) = line.strip_prefix("event: ") {
            event = Some(e.to_string());
        } else if let Some(d) = line.strip_prefix("data: ") {
            data = Some(serde_json::from_str(d).unwrap_or(Value::Null));
        }
    }
    Some((event?, data?))
}

/// 同 [`collect_all_frames`]，但濾掉 `ping`。
///
/// ping 在 Anthropic 串流的任何位置都合法（真的 API 在長生成期間就會穿插
/// 送），所以驗「內容 frame 的順序」的測試本來就不該在意它。不濾的話，
/// 機器負載高到讓上游 future 被餓超過 PING_INTERVAL 時，這些測試會偶發
/// 紅燈——那是測試環境的雜訊，不是被測行為出錯。ping 本身由專門的
/// `ping_keepalive_is_sent_before_the_first_real_frame` 驗證。
async fn collect_content_frames(resp: reqwest::Response) -> Vec<(String, Value)> {
    collect_all_frames(resp)
        .await
        .into_iter()
        .filter(|(event, _)| event != "ping")
        .collect()
}

/// 逐 chunk 讀到串流結束，回傳依序收到的 `(event, data)`。
async fn collect_all_frames(mut resp: reqwest::Response) -> Vec<(String, Value)> {
    let mut buf = String::new();
    let mut frames = Vec::new();
    while let Some(chunk) = resp.chunk().await.expect("讀取 SSE 串流失敗") {
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find("\n\n") {
            let raw = buf[..pos].to_string();
            buf.drain(..pos + 2);
            if let Some(f) = parse_frame(&raw) {
                frames.push(f);
            }
        }
    }
    frames
}

// ── 1. 完整的工具呼叫循環：frame 序列必須正確 ──────────────────────────

#[tokio::test]
async fn full_tool_call_round_trip_produces_the_correct_frame_sequence() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"開始列目錄\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",",
        "\"function\":{\"name\":\"Bash\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let (bridge, base) = start_bridge(&dir, Some(&server.uri())).await;

    let resp = post_messages(&base, Some(TOKEN), &simple_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let frames = collect_content_frames(resp).await;
    let names: Vec<&str> = frames.iter().map(|(e, _)| e.as_str()).collect();

    // 順序，不只是「有出現」：文字區塊開→送→關，接著工具區塊開→送→關，
    // 最後收尾。
    assert_eq!(
        names,
        vec![
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
        ],
        "實際收到的 frame 序列：{names:?}"
    );

    let message_start = &frames[0].1;
    let input_tokens = message_start["message"]["usage"]["input_tokens"].as_u64().unwrap();
    assert!(input_tokens > 0, "message_start 應帶非零的 input_tokens，實際：{message_start}");

    let text_start = &frames[1].1;
    assert_eq!(text_start["content_block"]["type"], "text");
    let text_delta = &frames[2].1;
    assert_eq!(text_delta["delta"]["type"], "text_delta");

    let tool_start = &frames[4].1;
    assert_eq!(tool_start["content_block"]["type"], "tool_use");
    assert_eq!(tool_start["content_block"]["name"], "Bash", "tool_use 區塊要帶完整的名稱");
    let tool_delta = &frames[5].1;
    assert_eq!(tool_delta["delta"]["type"], "input_json_delta");

    let message_delta = &frames[7].1;
    assert_eq!(message_delta["delta"]["stop_reason"], "tool_use");

    bridge.stop();
}

// ── 2. tool_result 回送：上游收到的請求要有 role: tool ─────────────────

#[tokio::test]
async fn tool_result_block_is_forwarded_as_a_tool_role_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: [DONE]\n\n", "text/event-stream"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let (bridge, base) = start_bridge(&dir, Some(&server.uri())).await;

    let body = json!({
        "model": "aiterm:sonnet",
        "stream": true,
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": "跑 ls"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_echo_1", "name": "Bash", "input": {"command": "ls"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_echo_1", "content": "a.txt\nb.txt"}
            ]}
        ]
    });

    let resp = post_messages(&base, Some(TOKEN), &body).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    // 把串流讀完，確保上游請求已經真的送出（handler 內的 spawn 是在請求
    // 一開始就發出的，但保險起見等到 body 結束再檢查記錄的請求）。
    let _ = collect_all_frames(resp).await;

    let received = server.received_requests().await.expect("wiremock 應該有記錄請求");
    assert_eq!(received.len(), 1);
    let sent: Value = serde_json::from_slice(&received[0].body).unwrap();
    let messages = sent["messages"].as_array().expect("上游請求要有 messages 陣列");

    let tool_msg = messages
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap_or_else(|| panic!("上游請求裡沒有 role:tool 的訊息：{messages:?}"));
    assert_eq!(tool_msg["tool_call_id"], "toolu_echo_1");
    assert_eq!(tool_msg["content"], "a.txt\nb.txt");

    bridge.stop();
}

// ── 2b. extra_content 往返：第二輪要把第一輪快取下來的不透明資料原樣送回 ──

#[tokio::test]
async fn second_turn_forwards_the_cached_extra_content_to_the_upstream() {
    // 模擬 Gemini 的 OpenAI 相容端點：tool_call 片段夾帶
    // extra_content.google.thought_signature。第二輪請求若沒有原樣帶回，
    // 真的 Gemini 端點會回 400（"missing a thought_signature"）——這支測試
    // 驗證的就是橋接有沒有把這段不透明資料存起來、在下一輪原樣送回上游。
    let server = MockServer::start().await;
    let sse = format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices": [{"delta": {"content": "開始列目錄"}}]}),
        json!({"choices": [{"delta": {"tool_calls": [{
            "index": 0, "id": "call_1",
            "extra_content": {"google": {"thought_signature": "sig-abc"}},
            "function": {"name": "Bash", "arguments": "{\"command\":\"ls -la /tmp\"}"}
        }]}}]}),
        json!({"choices": [{"finish_reason": "tool_calls"}]}),
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let (bridge, base) = start_bridge(&dir, Some(&server.uri())).await;

    // 第一輪：假上游回一個帶 extra_content 的 tool_call，橋接應該把它快取
    // 起來（鍵是 tool_call 的 id `call_1`）。
    let first = post_messages(&base, Some(TOKEN), &simple_request()).await;
    assert_eq!(first.status(), reqwest::StatusCode::OK);
    let _ = collect_all_frames(first).await;

    // 第二輪：client（Claude Code）把 assistant 的 tool_use 與對應的
    // tool_result 送回，跟真實對話流程一致——client 端完全不知道
    // extra_content 這件事，它只認得 Anthropic 的 tool_use 區塊。
    let second_body = json!({
        "model": "aiterm:sonnet",
        "stream": true,
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": "列出這個目錄的檔案"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "call_1", "name": "Bash", "input": {"command": "ls -la /tmp"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call_1", "content": "a.txt\nb.txt"}
            ]}
        ]
    });
    let second = post_messages(&base, Some(TOKEN), &second_body).await;
    assert_eq!(second.status(), reqwest::StatusCode::OK);
    let _ = collect_all_frames(second).await;

    let received = server.received_requests().await.expect("wiremock 應該有記錄請求");
    assert_eq!(received.len(), 2, "應該有兩輪對話各送出一個上游請求");

    let second_sent: Value = serde_json::from_slice(&received[1].body).unwrap();
    let messages = second_sent["messages"].as_array().expect("上游請求要有 messages 陣列");
    let assistant_tool_call = messages
        .iter()
        .find_map(|m| m.get("tool_calls").and_then(|tc| tc.as_array()))
        .and_then(|tcs| tcs.first())
        .unwrap_or_else(|| panic!("上游第二個請求裡沒有 assistant tool_calls：{messages:?}"));

    assert_eq!(
        assistant_tool_call["extra_content"],
        json!({"google": {"thought_signature": "sig-abc"}}),
        "第二輪送給上游的 tool_call 必須帶著第一輪快取下來的 extra_content，實際請求：{second_sent}"
    );

    bridge.stop();
}

// ── 3. 授權：沒帶 token / 帶錯 token → 401 ──────────────────────────────

#[tokio::test]
async fn missing_token_is_rejected_with_401() {
    let dir = tempfile::tempdir().unwrap();
    let (bridge, base) = start_bridge(&dir, None).await;

    let resp = post_messages(&base, None, &simple_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    bridge.stop();
}

#[tokio::test]
async fn wrong_token_is_rejected_with_401() {
    let dir = tempfile::tempdir().unwrap();
    let (bridge, base) = start_bridge(&dir, None).await;

    let resp = post_messages(&base, Some("not-the-right-token"), &simple_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    bridge.stop();
}

// ── 4. 未映射的層級 → 400，訊息含「設定」 ───────────────────────────────

#[tokio::test]
async fn unmapped_tier_returns_400_pointing_at_settings() {
    let dir = tempfile::tempdir().unwrap();
    // sonnet_upstream: None → claude_bridge 維持預設值，opus/sonnet/haiku
    // 都沒有映射任何供應商。
    let (bridge, base) = start_bridge(&dir, None).await;

    let resp = post_messages(&base, Some(TOKEN), &simple_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    let body = resp.text().await.unwrap();
    assert!(body.contains("設定"), "錯誤訊息要指向設定頁：{body}");

    bridge.stop();
}

// ── 5. 上游錯誤 → Anthropic 格式的 error frame，不是原始上游回應 ────────

#[tokio::test]
async fn upstream_500_becomes_an_anthropic_shaped_error_frame() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream proxy misconfigured, raw dump"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let (bridge, base) = start_bridge(&dir, Some(&server.uri())).await;

    let resp = post_messages(&base, Some(TOKEN), &simple_request()).await;
    // 串流已經開始（headers 早就送出），所以整個 HTTP 回應仍然是 200；
    // 錯誤是用 SSE `event: error` frame 表達的，不是把狀態碼改成 500。
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let frames = collect_content_frames(resp).await;
    assert!(!frames.is_empty(), "上游立即失敗，至少要收到一個 error frame");
    let (event, data) = &frames[0];
    assert_eq!(event, "error", "第一個 frame 應該就是 error（沒有機會先送 message_start）");
    assert_eq!(data["type"], "error");
    // 訊息要是我們自己包過的句子（帶 http 狀態碼前綴），不是上游原始回應
    // 內容被逐字轉發給客戶端。
    let message = data["error"]["message"].as_str().unwrap();
    assert!(message.contains("http 500"), "實際：{message}");

    bridge.stop();
}

// ── 6. ping keepalive：上游延遲時，真內容之前要先收到 ping ──────────────

#[tokio::test]
async fn ping_keepalive_is_sent_before_the_first_real_frame() {
    // 上游延遲 4 秒才回應（模擬本地模型冷啟動），PING_INTERVAL 是 3 秒，
    // 所以這個測試至少要花 4 秒才會結束 —— 這是刻意的，不是卡住。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"慢慢來\"}}]}\n\ndata: [DONE]\n\n",
                    "text/event-stream",
                )
                .set_delay(Duration::from_secs(4)),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let (bridge, base) = start_bridge(&dir, Some(&server.uri())).await;

    let mut resp = post_messages(&base, Some(TOKEN), &simple_request()).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let mut buf = String::new();
    let mut saw_ping = false;
    let mut first_real_event: Option<String> = None;
    'outer: while let Some(chunk) = resp.chunk().await.expect("讀取 SSE 串流失敗") {
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find("\n\n") {
            let raw = buf[..pos].to_string();
            buf.drain(..pos + 2);
            let Some((event, _)) = parse_frame(&raw) else { continue };
            if event == "ping" {
                saw_ping = true;
            } else {
                first_real_event = Some(event);
                break 'outer;
            }
        }
    }

    assert!(saw_ping, "在真的內容之前應該先收到至少一個 ping frame");
    assert!(first_real_event.is_some(), "ping 之後應該還是收到了真的事件");

    bridge.stop();
}

// ── 7. 非串流：OpenAI 上游仍以串流方式收，我們聚合成一個 JSON Message ──

#[tokio::test]
async fn non_streaming_request_returns_an_aggregated_json_message() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"開始列目錄\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",",
        "\"function\":{\"name\":\"Bash\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let (bridge, base) = start_bridge(&dir, Some(&server.uri())).await;

    let body = json!({
        "model": "aiterm:sonnet",
        "stream": false,
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "列出這個目錄的檔案"}]
    });
    let resp = post_messages(&base, Some(TOKEN), &body).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        content_type.starts_with("application/json"),
        "非串流回應要是完整的 JSON，不是 SSE：{content_type}"
    );

    let msg: Value = resp.json().await.expect("回應要能解析成 JSON Message");
    assert_eq!(msg["type"], "message");
    assert_eq!(msg["role"], "assistant");

    let content = msg["content"].as_array().expect("content 要是陣列");
    let text_block = content.iter().find(|b| b["type"] == "text").expect("要有 text block");
    assert_eq!(text_block["text"], "開始列目錄");

    let tool_block = content.iter().find(|b| b["type"] == "tool_use").expect("要有 tool_use block");
    assert_eq!(tool_block["name"], "Bash");
    assert!(
        tool_block["input"].is_object(),
        "input 必須是解析後的物件，不是字串：{}",
        tool_block["input"]
    );
    assert_eq!(tool_block["input"], json!({"command": "ls"}));

    assert_eq!(msg["stop_reason"], "tool_use");

    bridge.stop();
}

// ── 8. 省略 stream 欄位時也要走非串流路徑（不是預設拒絕） ────────────────

#[tokio::test]
async fn omitted_stream_field_also_uses_the_non_streaming_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n",
            "text/event-stream",
        ))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let (bridge, base) = start_bridge(&dir, Some(&server.uri())).await;

    let body = json!({
        "model": "aiterm:sonnet",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "hi"}]
    });
    let resp = post_messages(&base, Some(TOKEN), &body).await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let msg: Value = resp.json().await.expect("回應要能解析成 JSON");
    assert_eq!(msg["type"], "message");
    assert_eq!(msg["content"][0]["text"], "hi");

    bridge.stop();
}

// ── 9. Codex 供應商沒有鑰匙圈憑證 → 乾淨的 error frame，不是 panic/卡住 ──
//
// Codex 路徑的 `factory::build` 會呼叫 `get_valid_codex_oauth_token`，那會
// 讀（必要時刷新並回存）使用者真實的 OS 鑰匙圈。這支測試不能像
// `start_bridge` 幫 OpenAI 假上游那樣塞一組假憑證進去：Codex 的 refresh
// token 是會輪替的真實憑證，測試環境沒有、也不該偽造一個寫進使用者的
// 鑰匙圈。所以這裡只驗證「找不到憑證」這條乾淨的錯誤路徑——這仍然有
// 價值：證明了路由到 Codex kind、factory 分派、`AiError::NotConfigured`
// 包裝成 Anthropic 錯誤 frame 全部正確；驗不到的是真的打上游成功的那條
// 路徑，那一層留給 `bridge_codex_upstream.rs` 的 adapter 測試與手動端到端
// 驗收（見 M2 計畫 Task 6）。
//
// provider_id 用一個不會撞到真實供應商設定的字串。`SecretStore::get`
// 對鑰匙圈裡不存在的 key 回 `Ok(None)`（見 secret/mod.rs 的 `NoEntry` 分
// 支），不會 panic 也不會寫入任何東西——整支測試全程只有讀，沒有
// `set`/`delete`。
#[tokio::test]
async fn codex_provider_without_credentials_returns_a_clean_error_frame() {
    let dir = tempfile::tempdir().unwrap();
    let config = ConfigStore::new_at(dir.path().join("config.toml"));
    config
        .update(|cfg| {
            cfg.claude_bridge.sonnet = Some(TierMapping {
                provider_id: "e2e-codex-no-creds".into(),
                model: "gpt-5-codex".into(),
            });
            cfg.providers.push(ProviderConfig {
                id: "e2e-codex-no-creds".into(),
                display_name: "Codex (no creds)".into(),
                provider_type: ProviderType::Codex,
                base_url: None,
                oauth_client_id: None,
                model: "gpt-5-codex".into(),
                supports_json_mode: false,
                auth_method: Some("oauth".into()),
            });
        })
        .expect("寫入暫存 config 不應失敗");

    let bridge = BridgeState::new();
    let port = free_port().await;
    bridge
        .start(Arc::new(config), Arc::new(SecretStore::new()), TOKEN.to_string(), port)
        .await
        .expect("bridge 啟動不應失敗");
    let base = format!("http://127.0.0.1:{port}");

    let resp = post_messages(&base, Some(TOKEN), &simple_request()).await;
    // `build()` 是在 SSE stream 裡才被 await 的（見 stream.rs），headers 早就
    // 送出 200 了，所以跟 `upstream_500_becomes_an_anthropic_shaped_error_frame`
    // 一樣：錯誤要用 SSE error frame 表達，不是把狀態碼改掉。
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let frames = collect_content_frames(resp).await;
    assert!(!frames.is_empty(), "找不到憑證，至少要收到一個 error frame");
    let (event, data) = &frames[0];
    assert_eq!(event, "error", "第一個 frame 應該就是 error（沒有機會先送 message_start）");
    assert_eq!(data["type"], "error");
    assert_eq!(data["error"]["type"], "invalid_request_error");

    bridge.stop();
}
