//! Codex 上游 adapter 的端到端測試（假上游）。

use futures_util::StreamExt;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::anthropic::request::MessagesRequest;
use aiterm_lib::bridge::upstream::codex::client::CodexUpstream;
use aiterm_lib::bridge::upstream::{BridgeUpstream, StopReason, UpstreamEvent, UpstreamResponse};

fn req(v: serde_json::Value) -> MessagesRequest {
    serde_json::from_value(v).unwrap()
}

async fn collect(resp: UpstreamResponse) -> Vec<UpstreamEvent> {
    match resp {
        UpstreamResponse::Events(mut s) => {
            let mut out = Vec::new();
            while let Some(item) = s.next().await {
                out.push(item.expect("串流不該出錯"));
            }
            out
        }
        UpstreamResponse::Passthrough(_) => panic!("Codex 路徑不應回 Passthrough"),
    }
}

#[tokio::test]
async fn streams_text_and_tool_calls() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"開始\"}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"status\":\"in_progress\",\"arguments\":\"\",\"call_id\":\"call_1\",\"name\":\"Read\"}}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{}\",\"item_id\":\"fc_1\",\"output_index\":0}\n\n",
        "data: {\"type\":\"response.function_call_arguments.done\",\"arguments\":\"{}\",\"item_id\":\"fc_1\",\"output_index\":0}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":7,\"output_tokens\":3},\"output\":[]}}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/backend-api/codex/responses"))
        .and(header("authorization", "Bearer tok"))
        // 探勘確認的必要欄位。
        .and(body_partial_json(serde_json::json!({"stream": true, "store": false})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let up = CodexUpstream::new(server.uri(), "tok".into(), None);
    let resp = up
        .send(
            &req(serde_json::json!({
                "model": "aiterm:sonnet",
                "messages": [{"role": "user", "content": "hi"}]
            })),
            "gpt-5.6",
        )
        .await
        .unwrap();

    let ev = collect(resp).await;
    assert_eq!(ev[0], UpstreamEvent::TextDelta("開始".into()));
    assert_eq!(ev[1], UpstreamEvent::ToolUseStart { id: "call_1".into(), name: "Read".into() });
    assert_eq!(ev[2], UpstreamEvent::ToolInputDelta("{}".into()));
    assert_eq!(ev[3], UpstreamEvent::ToolUseEnd);
    assert!(matches!(ev[4], UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }));
}

#[tokio::test]
async fn account_id_header_is_sent_when_present() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("chatgpt-account-id", "acct-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("data: {\"type\":\"response.completed\",\"response\":{\"usage\":{}}}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = CodexUpstream::new(server.uri(), "tok".into(), Some("acct-1".into()));
    let resp = up
        .send(&req(serde_json::json!({"model":"m","messages":[{"role":"user","content":"x"}]})), "m")
        .await;
    assert!(resp.is_ok());
}

#[tokio::test]
async fn http_error_is_mapped_to_ai_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("no token"))
        .mount(&server)
        .await;

    let up = CodexUpstream::new(server.uri(), "bad".into(), None);
    let err = up
        .send(&req(serde_json::json!({"model":"m","messages":[{"role":"user","content":"x"}]})), "m")
        .await
        .unwrap_err();
    assert!(format!("{err:?}").contains("401") || format!("{err:?}").contains("Auth"), "實際：{err:?}");
}
