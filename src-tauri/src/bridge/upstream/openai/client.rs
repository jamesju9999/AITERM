//! OpenAI chat.completions 上游。

use async_trait::async_trait;
use futures_util::StreamExt;

use super::{request::build_body, stream::StreamParser};
use crate::ai::sse::{find_line_end, separator_len};
use crate::ai::AiError;
use crate::bridge::anthropic::request::MessagesRequest;
use crate::bridge::upstream::{BridgeUpstream, UpstreamEvent, UpstreamResponse};

pub struct OpenAiUpstream {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    /// 部分供應商（如 GitHub Copilot）要求每個請求都帶固定的額外標頭。
    extra_headers: Vec<(String, String)>,
}

impl OpenAiUpstream {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self::with_extra_headers(base_url, api_key, Vec::new())
    }

    pub fn with_extra_headers(base_url: String, api_key: String, extra_headers: Vec<(String, String)>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            client: reqwest::Client::new(),
            extra_headers,
        }
    }

    /// 使用者填的 base_url 有人帶 `/v1` 有人不帶，兩種都要能用。
    fn completions_url(&self) -> String {
        if self.base_url.ends_with("/v1") {
            format!("{}/chat/completions", self.base_url)
        } else {
            format!("{}/v1/chat/completions", self.base_url)
        }
    }
}

#[async_trait]
impl BridgeUpstream for OpenAiUpstream {
    async fn send(
        &self,
        req: &MessagesRequest,
        model: &str,
    ) -> Result<UpstreamResponse, AiError> {
        let mut request_builder = self
            .client
            .post(self.completions_url())
            .bearer_auth(&self.api_key);
        for (key, value) in &self.extra_headers {
            request_builder = request_builder.header(key, value);
        }
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
/// 用 `unfold` 而非 `async_stream`：後者要多一個依賴，而狀態機只有三個欄位。
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
