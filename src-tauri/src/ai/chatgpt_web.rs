//! ChatGPT 網頁版的 `AiProvider` 實作。傳輸走 `chatgpt_web::session`。
//!
//! 這裡沒有任何憑證處理——登入狀態由 session 的 webview 自己持有，
//! 我們只負責把請求組出來、把 SSE 轉成中性事件。

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::ai::{AiError, AiProvider, GenerateChunk, GenerateRequest};
use crate::chatgpt_web::protocol::{flatten_history, FlatTurn, SseOut, SseParser};
use crate::chatgpt_web::session;

pub struct ChatgptWebProvider {
    model: String,
}

impl ChatgptWebProvider {
    pub fn new(model: String) -> Self {
        Self { model }
    }
}

/// 把 `GenerateRequest` 組成注入腳本要的 payload。抽成獨立函式才能不啟動
/// webview 就測。
/// `ChatMessage.content` 可能是字串、內容陣列，或 null（帶 tool_calls 時）。
/// 取法與 `ai/codex.rs::build_request_body` 一致——那條路徑已經在線上跑。
fn message_text(m: &crate::ai::ChatMessage) -> String {
    match &m.content {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn build_payload(req: &GenerateRequest, model: &str) -> serde_json::Value {
    let turns: Vec<FlatTurn> = req
        .messages
        .iter()
        .map(|m| {
            if m.role == "assistant" {
                FlatTurn::Assistant(message_text(m))
            } else {
                FlatTurn::User(message_text(m))
            }
        })
        .collect();
    serde_json::json!({
        "text": flatten_history(&req.system_prompt, &turns),
        "model": model,
    })
}

#[async_trait]
impl AiProvider for ChatgptWebProvider {
    fn id(&self) -> &str {
        "chatgpt-web"
    }

    fn display_name(&self) -> &str {
        "ChatGPT Web"
    }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let s = session::get().ok_or(AiError::NotConfigured)?;
        let payload = build_payload(&req, &self.model).to_string();
        // `_guard` 必須綁具名變數：綁成 `_` 會當場 drop，sink 立刻被移除，
        // 接下來一個 chunk 都收不到。
        let (mut rx, _guard) = s
            .request(payload)
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let mut parser = SseParser::default();
        while let Some(raw) = rx.recv().await {
            // 注入腳本用 {"error":…} 回報失敗；其餘是 SSE 原文。
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    return Err(map_upstream_error(
                        err,
                        v.get("status").and_then(|s| s.as_u64()),
                    ));
                }
            }
            // 餵原始 chunk，不要自己 `raw.lines()`——HTTP chunk 不保證切在行
            // 邊界上，切開的半行兩邊都會被靜默丟掉。行緩衝在 SseParser 裡。
            for out in parser.feed_str(&raw) {
                match out {
                    SseOut::Text(delta) => {
                        let _ = tx
                            .send(GenerateChunk { delta, done: false, usage: None })
                            .await;
                    }
                    SseOut::Done => {
                        let _ = tx
                            .send(GenerateChunk {
                                delta: String::new(),
                                done: true,
                                usage: None,
                            })
                            .await;
                        return Ok(());
                    }
                }
            }
        }
        // 通道關閉但沒收到 [DONE]（視窗被關、上游斷線）：仍要補一個 done，
        // 否則呼叫端會一直等。SinkGuard 會讓通道真的關得起來。
        let _ = tx
            .send(GenerateChunk { delta: String::new(), done: true, usage: None })
            .await;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), AiError> {
        let s = session::get().ok_or(AiError::NotConfigured)?;
        // 顯示視窗：health_check 是設定頁「測試連線」按的，這正是引導登入的時機。
        s.ensure_window(true)
            .map_err(|e| AiError::Network { message: e.to_string() })?;
        Ok(())
    }
}

/// 上游錯誤字串 → `AiError`。`not_logged_in` 由注入腳本產生，其餘是上游原文。
pub fn map_upstream_error(err: &str, status: Option<u64>) -> AiError {
    if err.contains("not_logged_in") {
        return AiError::AuthFailed;
    }
    // 注入腳本在 PoW 超時時丟這個。不辨識的話使用者只會看到一串原始字串，
    // 而真正該傳達的是「上游把難度調高了，重試或稍後再試」——這跟網路錯誤
    // 的處置完全不同。
    if err.contains("pow_timeout") {
        return AiError::ModelError {
            reason: "ChatGPT 網頁版的工作量證明超時，請稍後再試".into(),
            raw: err.to_string(),
        };
    }
    match status {
        Some(401) | Some(403) => AiError::ModelError {
            reason: "ChatGPT 網頁版拒絕了請求".into(),
            raw: err.to_string(),
        },
        Some(429) => AiError::RateLimit { retry_after: None, body: Some(err.to_string()) },
        _ => AiError::Network { message: err.to_string() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ChatMessage, EnvSnapshot, QueryMode};

    fn req(system: &str, msgs: &[(&str, &str)]) -> GenerateRequest {
        GenerateRequest {
            system_prompt: system.into(),
            messages: msgs
                .iter()
                .map(|(r, c)| ChatMessage {
                    role: (*r).into(),
                    content: serde_json::Value::String((*c).into()),
                    tool_call_id: None,
                    tool_calls: None,
                })
                .collect(),
            context: EnvSnapshot::default(),
            mode: QueryMode::SingleCommand,
            max_tokens: None,
        }
    }

    #[test]
    fn payload_carries_flattened_text_and_model() {
        let p = build_payload(&req("你是助理", &[("user", "哈囉")]), "gpt-5-5");
        assert_eq!(p["model"], "gpt-5-5");
        let text = p["text"].as_str().unwrap();
        assert!(text.contains("你是助理"));
        assert!(text.contains("哈囉"));
    }

    #[test]
    fn assistant_turns_are_preserved_in_order() {
        let p = build_payload(
            &req("", &[("user", "一"), ("assistant", "二"), ("user", "三")]),
            "gpt-5-5",
        );
        let text = p["text"].as_str().unwrap();
        let (a, b, c) = (
            text.find('一').unwrap(),
            text.find('二').unwrap(),
            text.find('三').unwrap(),
        );
        assert!(a < b && b < c, "順序要保留：{text}");
    }

    /// `not_logged_in` 是注入腳本自己產生的字串，不是上游回的——它要對應到
    /// AuthFailed，前端才會提示「請先登入」而不是「網路錯誤」。
    #[test]
    fn injected_error_strings_map_to_actionable_variants() {
        assert!(matches!(map_upstream_error("not_logged_in", None), AiError::AuthFailed));
        assert!(matches!(
            map_upstream_error("pow_timeout: 工作量證明超時（difficulty=06b931）", None),
            AiError::ModelError { .. }
        ));
        assert!(matches!(
            map_upstream_error("whatever", Some(429)),
            AiError::RateLimit { .. }
        ));
    }
}
