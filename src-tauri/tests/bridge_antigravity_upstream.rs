//! Antigravity 上游 adapter 的端到端測試（假上游）。

use std::sync::Arc;

use futures_util::StreamExt;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::anthropic::request::MessagesRequest;
use aiterm_lib::bridge::tool_meta::ToolMetaCache;
use aiterm_lib::bridge::upstream::antigravity::client::AntigravityUpstream;
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
        UpstreamResponse::Passthrough(_) => panic!("Antigravity 路徑不應回 Passthrough"),
    }
}

#[tokio::test]
async fn streams_text_and_tool_calls_and_caches_thought_signature() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"你好\"}]}}]}}\n\n",
        "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"thoughtSignature\":\"SIG\",\"functionCall\":{\"name\":\"Read\",\"args\":{\"p\":1},\"id\":\"c1\"}}]}}]}}\n\n",
        "data: {\"response\":{\"candidates\":[{\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":3}}}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1internal:streamGenerateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let tool_meta = Arc::new(ToolMetaCache::new(512));
    let up = AntigravityUpstream::new(server.uri(), "tok".into(), "proj-1".into(), tool_meta.clone());
    let resp = up
        .send(
            &req(serde_json::json!({
                "model": "aiterm:sonnet",
                "messages": [{"role": "user", "content": "hi"}]
            })),
            "gemini-2.5-pro",
        )
        .await
        .unwrap();

    let ev = collect(resp).await;
    assert_eq!(ev[0], UpstreamEvent::TextDelta("你好".into()));
    assert_eq!(ev[1], UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Read".into() });
    assert_eq!(ev[2], UpstreamEvent::ToolInputDelta("{\"p\":1}".into()));
    assert_eq!(ev[3], UpstreamEvent::ToolUseEnd);
    assert!(matches!(ev[4], UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }));
    assert_eq!(tool_meta.get("c1"), Some(serde_json::json!("SIG")));
}

#[tokio::test]
async fn auth_header_and_project_are_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1internal:streamGenerateContent"))
        .and(header("authorization", "Bearer tok"))
        .and(body_partial_json(serde_json::json!({"project": "proj-1"})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"response\":{\"candidates\":[{\"finishReason\":\"STOP\"}],\"usageMetadata\":{}}}\n\n",
            "text/event-stream",
        ))
        .mount(&server)
        .await;

    let up = AntigravityUpstream::new(
        server.uri(),
        "tok".into(),
        "proj-1".into(),
        Arc::new(ToolMetaCache::new(512)),
    );
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

    let up = AntigravityUpstream::new(
        server.uri(),
        "bad".into(),
        "proj-1".into(),
        Arc::new(ToolMetaCache::new(512)),
    );
    let err = up
        .send(&req(serde_json::json!({"model":"m","messages":[{"role":"user","content":"x"}]})), "m")
        .await
        .unwrap_err();
    assert!(format!("{err:?}").contains("401") || format!("{err:?}").contains("Auth"), "實際：{err:?}");
}
