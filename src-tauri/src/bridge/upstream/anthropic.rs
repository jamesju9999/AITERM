//! Anthropic 家族轉發。
//!
//! 上游本來就講 Messages API，解析再重組是純粹的損耗，所以 SSE 原樣 pipe
//! （[`UpstreamResponse::Passthrough`]）。只改三處：模型名、auth 標頭、
//! OAuth 模式的 system 哨兵。

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ai::AiError;
use crate::bridge::anthropic::request::MessagesRequest;
use crate::bridge::upstream::{BridgeUpstream, UpstreamResponse};

/// OAuth（`sk-ant-oat*`）token 要求第一個 system block 必須是這句，否則上游
/// 回一個假的 `rate_limit_error`。與 `ai/anthropic.rs:342` 是同一個常數，
/// 那邊已 `pub`，這裡直接 re-export 而非重打一份。
pub use crate::ai::anthropic::CLAUDE_CODE_SENTINEL;

pub struct AnthropicUpstream {
    base_url: String,
    token: String,
    is_oauth: bool,
    client: reqwest::Client,
}

impl AnthropicUpstream {
    pub fn new(base_url: String, token: String, is_oauth: bool) -> Self {
        Self { base_url, token, is_oauth, client: reqwest::Client::new() }
    }
}

pub fn messages_url(base_url: &str) -> String {
    format!("{}/v1/messages", base_url.trim_end_matches('/'))
}

/// 改寫請求 body。`raw` 是 Claude Code 原封不動的 JSON —— 我們不重建它，
/// 因為任何我們沒解析的欄位（cache_control、未來新增的參數）都要原樣送達。
pub fn rewrite_body(raw: &Value, model: &str, is_oauth: bool) -> Value {
    let mut body = raw.clone();
    body["model"] = json!(model);
    if is_oauth {
        body["system"] = ensure_sentinel(raw.get("system"));
    }
    strip_unsigned_thinking(&mut body);
    body
}

/// 移除歷史訊息裡沒有 `signature` 的 thinking 區塊。
///
/// 三層映射是各自獨立的，使用者可以 sonnet 走 OpenAI、opus 走 Anthropic。
/// 走 OpenAI 那條路徑時我們的序列化器產生的 thinking 區塊沒有簽章（我們簽
/// 不出來），而使用者一旦 /model 切到 opus，這段歷史就會被原樣 POST 到真的
/// api.anthropic.com —— 那邊會驗證 thinking 區塊的簽章。與其賭它會不會 400，
/// 不如轉發前就拿掉。
fn strip_unsigned_thinking(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for m in messages {
        let Some(blocks) = m.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        blocks.retain(|b| {
            let is_thinking = b.get("type").and_then(Value::as_str) == Some("thinking");
            !is_thinking || b.get("signature").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
        });
    }
}

fn ensure_sentinel(system: Option<&Value>) -> Value {
    let sentinel = json!({"type": "text", "text": CLAUDE_CODE_SENTINEL});
    let mut blocks: Vec<Value> = match system {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => vec![json!({"type": "text", "text": s})],
        Some(Value::Array(a)) => a.clone(),
        Some(other) => vec![other.clone()],
    };
    let already = blocks
        .first()
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
        .map(|t| t.starts_with(CLAUDE_CODE_SENTINEL))
        .unwrap_or(false);
    if !already {
        blocks.insert(0, sentinel);
    }
    Value::Array(blocks)
}

#[async_trait]
impl BridgeUpstream for AnthropicUpstream {
    async fn send(
        &self,
        _req: &MessagesRequest,
        _model: &str,
    ) -> Result<UpstreamResponse, AiError> {
        // 這條路徑需要原始 JSON，走 `send_raw`。trait 方法保留是為了讓
        // 工廠能回傳統一的 Box<dyn BridgeUpstream>。
        Err(AiError::ModelError {
            reason: "Anthropic 轉發路徑請呼叫 send_raw".into(),
            raw: String::new(),
        })
    }
}

impl AnthropicUpstream {
    /// Anthropic 路徑專用：吃原始 JSON，回未解析的 HTTP 回應。
    pub async fn send_raw(&self, raw: &Value, model: &str) -> Result<UpstreamResponse, AiError> {
        let body = rewrite_body(raw, model, self.is_oauth);
        let mut rb = self.client.post(messages_url(&self.base_url)).json(&body);
        rb = if self.is_oauth {
            rb.bearer_auth(&self.token)
                .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
                .header("x-app", "cli")
        } else {
            rb.header("x-api-key", &self.token)
        };
        let resp = rb
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(crate::ai::sse::map_http_error(status, resp).await);
        }
        Ok(UpstreamResponse::Passthrough(resp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_is_replaced_with_the_mapped_one() {
        let body = rewrite_body(
            &json!({"model": "aiterm:sonnet", "messages": [], "stream": true}),
            "claude-sonnet-4-5",
            false,
        );
        assert_eq!(body["model"], "claude-sonnet-4-5");
    }

    #[test]
    fn api_key_mode_leaves_system_untouched() {
        let body = rewrite_body(
            &json!({"model": "m", "system": "你是助手", "messages": []}),
            "m",
            false,
        );
        assert_eq!(body["system"], "你是助手");
    }

    #[test]
    fn oauth_mode_prepends_the_sentinel_when_missing() {
        // sk-ant-oat* token 要求第一個 system block 必須是這句，否則上游會回
        // 假的 rate_limit_error（見 ai/anthropic.rs:342）。
        let body = rewrite_body(&json!({"model": "m", "system": "自訂", "messages": []}), "m", true);
        let blocks = body["system"].as_array().expect("OAuth 模式要轉成 block 陣列");
        assert_eq!(blocks[0]["text"], CLAUDE_CODE_SENTINEL);
        assert_eq!(blocks[1]["text"], "自訂");
    }

    #[test]
    fn oauth_mode_does_not_duplicate_an_existing_sentinel() {
        // Claude Code 自己送的 system prompt 第一句正好就是那句，多半天然滿足。
        let body = rewrite_body(
            &json!({
                "model": "m", "messages": [],
                "system": [{"type": "text", "text": CLAUDE_CODE_SENTINEL}]
            }),
            "m",
            true,
        );
        let blocks = body["system"].as_array().unwrap();
        assert_eq!(blocks.len(), 1, "不能重複插入：{blocks:?}");
    }

    #[test]
    fn oauth_mode_handles_absent_system() {
        let body = rewrite_body(&json!({"model": "m", "messages": []}), "m", true);
        let blocks = body["system"].as_array().unwrap();
        assert_eq!(blocks[0]["text"], CLAUDE_CODE_SENTINEL);
    }

    #[test]
    fn unsigned_thinking_blocks_are_stripped_from_history() {
        // 走 OpenAI 路徑產生的 thinking 區塊沒有簽章，使用者切到 Anthropic
        // 層之後這段歷史會被原樣轉發，上游會驗簽。
        let body = rewrite_body(
            &json!({"model": "m", "messages": [{"role": "assistant", "content": [
                {"type": "thinking", "thinking": "沒簽章"},
                {"type": "text", "text": "保留"}
            ]}]}),
            "m",
            false,
        );
        let blocks = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
    }

    #[test]
    fn signed_thinking_blocks_survive() {
        let body = rewrite_body(
            &json!({"model": "m", "messages": [{"role": "assistant", "content": [
                {"type": "thinking", "thinking": "有簽章", "signature": "sig-abc"}
            ]}]}),
            "m",
            false,
        );
        assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn string_content_messages_are_left_alone() {
        let body = rewrite_body(
            &json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]}),
            "m",
            false,
        );
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn messages_url_appends_v1_messages() {
        assert_eq!(messages_url("https://api.anthropic.com"), "https://api.anthropic.com/v1/messages");
        assert_eq!(messages_url("https://x.test/"), "https://x.test/v1/messages");
    }
}
