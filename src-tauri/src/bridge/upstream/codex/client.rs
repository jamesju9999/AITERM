//! Codex（ChatGPT 訂閱）上游。

use async_trait::async_trait;
use futures_util::StreamExt;

use super::{request::build_body, stream::StreamParser};
use crate::ai::codex::CodexClient;
use crate::ai::sse::{find_line_end, separator_len};
use crate::ai::AiError;
use crate::bridge::anthropic::request::MessagesRequest;
use crate::bridge::upstream::{BridgeUpstream, UpstreamEvent, UpstreamResponse};

pub struct CodexUpstream {
    /// 只借用它已驗證過的 `responses_url()`/`apply_headers()`，不透過它
    /// 發送請求——它的 `client` 欄位是私有的，這裡另外持有自己的
    /// `reqwest::Client`。`model` 傳空字串：這裡的 URL/headers 邏輯不吃它。
    inner: CodexClient,
    client: reqwest::Client,
}

impl CodexUpstream {
    pub fn new(base_url: String, access_token: String, chatgpt_account_id: Option<String>) -> Self {
        Self {
            inner: CodexClient::with_base_url(access_token, String::new(), chatgpt_account_id, base_url),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl BridgeUpstream for CodexUpstream {
    async fn send(
        &self,
        req: &MessagesRequest,
        model: &str,
    ) -> Result<UpstreamResponse, AiError> {
        let request_builder =
            self.inner.apply_headers(self.client.post(self.inner.responses_url()));
        let resp = request_builder
            .json(&build_body(req, model))
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(crate::ai::sse::map_http_error(status, resp).await);
        }

        Ok(UpstreamResponse::Events(Box::pin(into_events(resp))))
    }
}

/// 把 HTTP 回應的 byte 串轉成 [`UpstreamEvent`] 串流。
///
/// 跟 `openai/client.rs` 的 `into_events` 同一個骨架，差別是 Codex 用
/// `response.failed` 事件表達錯誤（不是 HTTP 狀態碼），所以每一行要先
/// 用 `take_error` 檢查，有錯就結束串流並回該錯誤，否則才 `feed_line`。
fn into_events(
    resp: reqwest::Response,
) -> impl futures_util::Stream<Item = Result<UpstreamEvent, AiError>> {
    struct State {
        bytes: std::pin::Pin<Box<dyn futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
        buf: Vec<u8>,
        parser: StreamParser,
        queued: std::collections::VecDeque<UpstreamEvent>,
        ended: bool,
    }

    let state = State {
        bytes: Box::pin(resp.bytes_stream()),
        buf: Vec::new(),
        parser: StreamParser::default(),
        queued: std::collections::VecDeque::new(),
        ended: false,
    };

    futures_util::stream::unfold(state, |mut s| async move {
        loop {
            if let Some(ev) = s.queued.pop_front() {
                return Some((Ok(ev), s));
            }
            if s.ended {
                return None;
            }
            // 先把緩衝區裡完整的行處理掉。
            if let Some(pos) = find_line_end(&s.buf) {
                let line_bytes: Vec<u8> = s.buf.drain(..pos).collect();
                let sep = separator_len(&s.buf);
                s.buf.drain(..sep);
                if let Ok(line) = std::str::from_utf8(&line_bytes) {
                    if let Some(err) = s.parser.take_error(line) {
                        s.ended = true;
                        return Some((Err(err), s));
                    }
                    s.queued.extend(s.parser.feed_line(line));
                }
                continue;
            }
            match s.bytes.next().await {
                Some(Ok(chunk)) => s.buf.extend_from_slice(&chunk),
                Some(Err(e)) => {
                    s.ended = true;
                    return Some((Err(AiError::Network { message: e.to_string() }), s));
                }
                None => {
                    s.ended = true;
                    s.queued.extend(s.parser.finish());
                }
            }
        }
    })
}
