//! ChatGPT 網頁版的 `AiProvider` 實作。傳輸走 `chatgpt_web::session`。
//!
//! 這裡沒有任何憑證處理——登入狀態由 session 的 webview 自己持有，
//! 我們只負責把請求組出來、把 SSE 轉成中性事件。

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::ai::{
    AiError, AiProvider, GenerateChunk, GenerateRequest, GenerateWithToolsResult,
    McpToolDefinition,
};
use crate::chatgpt_web::protocol::{flatten_history, FlatTurn, SseOut, SseParser};
use crate::chatgpt_web::{session, tools};

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

/// 帶工具的 payload。契約放最後、提醒黏在最後一個回合之後——雙位置注入。
///
/// 依據 OmniRoute #7679 的實測：契約 prepend 在巨大 system 區塊開頭時，30K
/// 字元 prompt 下模型會回「tool X is not in my tool set」，成功率 0/3；改成
/// 尾端 + 當前回合提醒的雙位置為 16/17。
pub fn build_payload_with_tools(
    req: &GenerateRequest,
    model: &str,
    tool_defs: &[McpToolDefinition],
    nonce: &str,
) -> serde_json::Value {
    let mut turns: Vec<FlatTurn> = req
        .messages
        .iter()
        .map(|m| {
            if m.role == "assistant" {
                // 不可原樣塞回去：聊天面板寫回歷史的 `<tool_call>` 不帶 _nonce，
                // 原樣攤平等於 few-shot 教模型省略它。
                FlatTurn::Assistant(tools::rewrite_envelopes_as_history_markers(&message_text(m)))
            } else {
                FlatTurn::User(message_text(m))
            }
        })
        .collect();

    let reminder = tools::build_reminder(tool_defs);
    if !reminder.is_empty() {
        // 掛在最後一個回合。這條路徑攤不出 ToolResult，所以最後一個回合實際上
        // 一定是 User；但寫法刻意與 BridgeUpstream 一致——那邊會有只含
        // tool_result 的回合，兩處邏輯若分岔，之後只會修到其中一邊。
        match turns.last_mut() {
            Some(FlatTurn::User(text)) => text.push_str(&reminder),
            _ => turns.push(FlatTurn::User(reminder.trim_start().to_string())),
        }
    }

    let mut text = flatten_history(&req.system_prompt, &turns);
    let contract = tools::build_contract(tool_defs, nonce);
    if !contract.is_empty() {
        text.push_str("\n\n");
        text.push_str(&contract);
    }
    serde_json::json!({ "text": text, "model": model })
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
                    SseOut::Error { message, code } => {
                        return Err(map_stream_error(&message, &code));
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

    async fn generate_with_tools(
        &self,
        req: GenerateRequest,
        tool_defs: Vec<McpToolDefinition>,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<GenerateWithToolsResult, AiError> {
        let s = session::get().ok_or(AiError::NotConfigured)?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let payload = build_payload_with_tools(&req, &self.model, &tool_defs, &nonce).to_string();
        // `_guard` 必須綁具名變數：綁成 `_` 會當場 drop，sink 立刻被移除，
        // 接下來一個 chunk 都收不到。
        let (mut rx, _guard) = s
            .request(payload)
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        // 工具呼叫只能在整段回覆收完後才判斷得出來（封套可能跨 chunk），
        // 所以先收滿再剖析。串流仍照送，使用者看得到進度。
        let mut full = String::new();
        let mut parser = SseParser::default();
        while let Some(raw) = rx.recv().await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    return Err(map_upstream_error(
                        err,
                        v.get("status").and_then(|s| s.as_u64()),
                    ));
                }
            }
            // 餵原始 chunk，不要自己 `raw.lines()`——理由同 `generate`。
            let mut finished = false;
            for out in parser.feed_str(&raw) {
                match out {
                    SseOut::Text(delta) => {
                        full.push_str(&delta);
                        let _ = tx
                            .send(GenerateChunk { delta, done: false, usage: None })
                            .await;
                    }
                    SseOut::Error { message, code } => {
                        return Err(map_stream_error(&message, &code));
                    }
                    // 一定要主動結束，不能等通道關閉。
                    SseOut::Done => finished = true,
                }
            }
            if finished {
                break;
            }
        }
        let _ = tx
            .send(GenerateChunk { delta: String::new(), done: true, usage: None })
            .await;

        let (content, calls) = tools::parse_tool_calls(&full, &nonce);
        Ok(match calls {
            Some(calls) => GenerateWithToolsResult::ToolCalls { calls, raw: None },
            None => GenerateWithToolsResult::Text(content),
        })
    }
}

/// 串流內錯誤 frame → `AiError`。
///
/// 這類錯誤的 HTTP 狀態是 **200**，錯誤藏在 SSE body 裡
/// （`{"message":null,"error":"…","error_code":"usage_limit"}`），所以走不到
/// `map_upstream_error` 那條看 HTTP status 的路徑。實測時「你已達到上限」
/// 就是這樣被漏掉，最後表現成一個跟真正原因無關的「AI 回傳格式錯誤」。
pub fn map_stream_error(message: &str, code: &str) -> AiError {
    match code {
        // 方案用量上限。對應到 RateLimit 前端才會顯示成「稍後再試」而不是
        // 「模型壞了」——這兩種的處置完全不同。
        "usage_limit" | "rate_limit_exceeded" => AiError::RateLimit {
            retry_after: None,
            body: Some(message.to_string()),
        },
        _ => AiError::ModelError {
            reason: message.to_string(),
            raw: format!("error_code={code}"),
        },
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
    // ChatGPT 網頁版對**單則訊息**有長度上限，而且比模型的上下文視窗小得多。
    // 這條路徑把 system prompt 與整段歷史攤平成單一則訊息，所以 Claude Code
    // 那種數萬字元的 prompt 會直接撞牆。實測（2026-08-11）橋接每一發都是
    // 這個錯。不辨識的話它會被歸成 Network，使用者看到「網路錯誤」——跟真正
    // 的原因毫無關係，也不會知道該怎麼辦。
    if err.contains("message_length_exceeds_limit") {
        return AiError::InvalidInput {
            reason: "訊息長度超過 ChatGPT 網頁版的單則上限。這條路徑會把整段歷史攤平成\
                     一則訊息，Claude Code 的 system prompt 通常會超標——請改用其他供應商，\
                     或縮短對話歷史。"
                .into(),
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

    fn tool(name: &str) -> McpToolDefinition {
        McpToolDefinition {
            name: name.into(),
            description: "說明".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    /// 契約要在最後（攤平後落在整段文字尾端），提醒要黏在最後一個回合之後。
    /// 依據 OmniRoute #7679 的實測：prepend 在 30K prompt 下是 0/3。
    #[test]
    fn tool_payload_places_contract_last_and_reminder_after_user() {
        let p = build_payload_with_tools(
            &req("系統指示", &[("user", "請讀檔")]),
            "gpt-5-5",
            &[tool("Read")],
            "n1",
        );
        let text = p["text"].as_str().unwrap();
        let user_at = text.find("請讀檔").unwrap();
        let reminder_at = text.find("Client protocol reminder").unwrap();
        let contract_at = text.find("Available tools").unwrap();
        assert!(user_at < reminder_at, "提醒要在 user 訊息之後");
        assert!(reminder_at < contract_at, "完整契約要在最後");
        assert!(text.contains("n1"), "nonce 要在契約裡");
    }

    #[test]
    fn no_tools_means_no_contract() {
        let p = build_payload_with_tools(&req("", &[("user", "哈囉")]), "gpt-5-5", &[], "n1");
        let text = p["text"].as_str().unwrap();
        assert!(!text.contains("Available tools"));
        assert!(!text.contains("Client protocol reminder"), "沒工具就不該有提醒");
    }

    /// 第二輪起，歷史裡會有聊天面板寫回去的 `<tool_call>`（不帶 `_nonce`，
    /// 見 `src/hooks/useMcpChat.ts:194-197`）。原樣攤回 prompt 等於 few-shot
    /// 教模型省略 nonce，之後每次呼叫都會被 `parse_tool_calls` 拒絕——
    /// 使用者看到的是「第一個工具會動，之後就不動了」。
    ///
    /// 這條測試守的是「`rewrite_envelopes_as_history_markers` 真的有被呼叫」，
    /// 光有那個函式不算數。
    #[test]
    fn history_tool_call_envelopes_never_reach_the_payload() {
        let r = req(
            "系統指示",
            &[
                ("user", "請讀檔"),
                (
                    "assistant",
                    r#"<tool_call>{"name":"fs__read","arguments":{"path":"a"}}</tool_call>"#,
                ),
                ("user", "結果是 hello"),
            ],
        );
        let p = build_payload_with_tools(&r, "gpt-5-5", &[tool("fs__read")], "n1");
        let text = p["text"].as_str().unwrap();
        let history_end = text.find("Available tools").unwrap();
        let history = &text[..history_end];
        assert!(
            !history.contains("<tool_call"),
            "歷史裡不可留下封套形狀，否則等於教模型省略 nonce：{history}"
        );
        assert!(
            history.contains("[[tool_call:fs__read#history]]"),
            "應改寫成界定符：{history}"
        );
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
        // 這是實測依據（2026-08-11）：橋接路徑每一發都撞這個。網頁版對單則
        // 訊息有長度上限，而這條路徑把整段歷史攤成一則。不辨識的話會被歸成
        // Network，使用者看到「網路錯誤」，跟真正的原因毫無關係。
        assert!(matches!(
            map_upstream_error(
                r#"{"detail":{"message":"你提交的訊息太長","code":"message_length_exceeds_limit"}}"#,
                None
            ),
            AiError::InvalidInput { .. }
        ));
    }
}
