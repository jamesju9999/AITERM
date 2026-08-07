//! OpenAI 上游 adapter 的端到端測試（假上游）。

use std::sync::Arc;

use futures_util::StreamExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::anthropic::request::MessagesRequest;
use aiterm_lib::bridge::tool_meta::ToolMetaCache;
use aiterm_lib::bridge::upstream::openai::client::OpenAiUpstream;
use aiterm_lib::bridge::upstream::{BridgeUpstream, StopReason, UpstreamEvent, UpstreamResponse};

fn req(v: serde_json::Value) -> MessagesRequest {
    serde_json::from_value(v).unwrap()
}

fn empty_cache() -> Arc<ToolMetaCache> {
    Arc::new(ToolMetaCache::new(512))
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
        UpstreamResponse::Passthrough(_) => panic!("OpenAI 路徑不應回 Passthrough"),
    }
}

#[tokio::test]
async fn streams_text_and_tool_calls() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"開始\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"Read\",\"arguments\":\"{}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse, "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = OpenAiUpstream::new(format!("{}/v1/chat/completions", server.uri()), "sk-test".into(), empty_cache());
    let resp = up
        .send(
            &req(serde_json::json!({
                "model": "aiterm:sonnet",
                "messages": [{"role": "user", "content": "hi"}]
            })),
            "qwen",
        )
        .await
        .unwrap();

    let ev = collect(resp).await;
    assert_eq!(ev[0], UpstreamEvent::TextDelta("開始".into()));
    assert_eq!(ev[1], UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Read".into() });
    assert_eq!(ev[2], UpstreamEvent::ToolInputDelta("{}".into()));
    assert_eq!(ev[3], UpstreamEvent::ToolUseEnd);
    assert!(matches!(ev[4], UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }));
}

#[tokio::test]
async fn http_error_is_mapped_to_ai_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("no key"))
        .mount(&server)
        .await;

    let up = OpenAiUpstream::new(format!("{}/v1/chat/completions", server.uri()), "bad".into(), empty_cache());
    let err = up
        .send(&req(serde_json::json!({
            "model": "m", "messages": [{"role":"user","content":"x"}]
        })), "m")
        .await
        .unwrap_err();
    assert!(format!("{err:?}").contains("Auth"), "實際：{err:?}");
}

#[tokio::test]
async fn upstream_uses_the_given_url_verbatim_without_appending_anything() {
    // OpenAiUpstream 不再自己猜端點形狀（那條規則本身就是這次修的
    // bug——GitHub Copilot 沒有版本前綴卻被誤補了 `/v1`）。端點 URL 由呼叫端
    // （`bridge/factory.rs` 透過 `ai::router::openai_chat_url`）依 provider
    // type 算好整個路徑再傳進來，`OpenAiUpstream` 只能原樣打那個 URL，不能
    // 再自作主張加東西。這裡故意傳一個「怪」路徑（不是 `/v1/chat/completions`
    // 也不是 `/chat/completions`）來證明它沒有被猜測邏輯改寫。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/weird/custom/path"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: [DONE]\n\n", "text/event-stream"))
        .mount(&server)
        .await;

    let up = OpenAiUpstream::new(format!("{}/weird/custom/path", server.uri()), "k".into(), empty_cache());
    let resp = up
        .send(&req(serde_json::json!({
            "model": "m", "messages": [{"role":"user","content":"x"}]
        })), "m")
        .await
        .unwrap();
    // 沒有 panic（wiremock 沒收到請求會在 mount 的 mock 未被呼叫時於其他
    // 斷言曝露問題）就代表打到了原樣傳入的 URL，而不是被加工過的版本。
    assert!(matches!(collect(resp).await.last(), Some(UpstreamEvent::Done { .. })));
}
