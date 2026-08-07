//! 綁 127.0.0.1 的 Anthropic Messages API server。

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::ai::AiError;
use crate::bridge::anthropic::request::{parse_content, system_text, ContentBlock, MessagesRequest};
use crate::bridge::anthropic::response::error_frame;
use crate::bridge::{auth, model_map};
use crate::config::ConfigStore;
use crate::secret::SecretStore;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub secrets: Arc<SecretStore>,
    pub token: Arc<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .with_state(state)
}

fn authorize(state: &AppState, headers: &HeaderMap) -> bool {
    let authorization = headers.get("authorization").and_then(|v| v.to_str().ok());
    let x_api_key = headers.get("x-api-key").and_then(|v| v.to_str().ok());
    match auth::extract_token(authorization, x_api_key) {
        Some(provided) => auth::token_matches(&state.token, &provided),
        None => false,
    }
}

/// 非串流的錯誤回應（連 SSE 都還沒開始時用）。
fn json_error(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"type": "error", "error": {"type": kind, "message": message}})),
    )
        .into_response()
}

pub fn status_for(err: &AiError) -> u16 {
    match err {
        AiError::AuthFailed => 401,
        AiError::RateLimit { .. } => 429,
        AiError::NotConfigured | AiError::InvalidInput { .. } => 400,
        AiError::Network { .. } => 502,
        AiError::ModelError { .. } => 500,
        // 這條路徑理論上不會從橋接的上游 adapter 冒出來（橋接不透過
        // ai::AiProvider trait），但 AiError 是窮舉 match，仍要給個
        // 合理的狀態碼：客戶端請求了目前設定無法滿足的能力。
        AiError::ToolCallingUnsupported => 400,
    }
}

fn error_text(err: &AiError) -> String {
    // AiError 每個變體都用 thiserror 的 #[error(...)] 定義了 Display，
    // 比逐變體反序列化欄位更省事，且新增變體會自動涵蓋（不會漏字串）。
    err.to_string()
}

fn error_kind(err: &AiError) -> &'static str {
    match err {
        AiError::AuthFailed => "authentication_error",
        AiError::RateLimit { .. } => "rate_limit_error",
        AiError::NotConfigured | AiError::InvalidInput { .. } => "invalid_request_error",
        _ => "api_error",
    }
}

async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorize(&state, &headers) {
        return json_error(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "AITerm 橋接的 token 不正確。請確認終端機分頁是由 AITerm 開啟的。",
        );
    }

    let Ok(raw) = serde_json::from_slice::<Value>(&body) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_request_error", "請求不是合法的 JSON。");
    };
    let req: MessagesRequest = match serde_json::from_value(raw.clone()) {
        Ok(r) => r,
        Err(e) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                &format!("無法解析 Messages 請求：{e}"),
            )
        }
    };
    if req.stream != Some(true) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "AITerm 橋接只支援串流請求（stream: true）。",
        );
    }

    let cfg = state.config.get();
    // resolve 回傳擁有權（見 model_map.rs 的註解），這裡不需要再 clone。
    let mapping = match model_map::resolve(&cfg.claude_bridge, &req.model) {
        Ok(m) => m,
        Err(msg) => return json_error(StatusCode::BAD_REQUEST, "invalid_request_error", &msg),
    };

    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let _ = (&mapping, &req, &raw, &message_id); // Task 15 才會用到，先壓掉 unused 警告。
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        // Task 15 會換成 super::stream::run(state, mapping, req, raw, message_id)。
        .body(Body::empty())
        .expect("建立 SSE 回應不應失敗")
}

async fn count_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorize(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "authentication_error", "token 不正確。");
    }
    let Ok(req) = serde_json::from_slice::<MessagesRequest>(&body) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_request_error", "請求格式錯誤。");
    };
    Json(json!({"input_tokens": estimate_input_tokens(&req)})).into_response()
}

/// 粗估 token 數：字元數 ÷ 4。
///
/// Claude Code 用這個端點做 context 管理，不需要精確值。引入 tiktoken 對
/// 非 OpenAI 模型也只是另一種估算，不值得多一個依賴與詞彙表體積。
pub fn estimate_input_tokens(req: &MessagesRequest) -> u32 {
    let mut chars = system_text(req.system.as_ref()).chars().count();
    for m in &req.messages {
        chars += count_blocks(&parse_content(&m.content));
    }
    let est = chars / 4;
    if chars > 0 { est.max(1) as u32 } else { 0 }
}

fn count_blocks(blocks: &[ContentBlock]) -> usize {
    blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Text(t) | ContentBlock::Thinking(t) => t.chars().count(),
            // 圖片按 Anthropic 文件的粗略公式無法在不解碼下算準，
            // 用 base64 長度的固定比例當下限即可。
            ContentBlock::Image { data, .. } => data.len() / 750,
            ContentBlock::ToolUse { name, input, .. } => {
                name.chars().count() + input.to_string().chars().count()
            }
            ContentBlock::ToolResult { content, .. } => count_blocks(content),
        })
        .sum()
}

/// 上游錯誤 → 一則 SSE error frame。串流已經開始時只能用這個回報。
pub fn error_frame_for(err: &AiError) -> String {
    error_frame(error_kind(err), &error_text(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn token_estimate_counts_all_text_fields() {
        let req: crate::bridge::anthropic::request::MessagesRequest =
            serde_json::from_value(json!({
                "model": "m",
                "system": "1234",
                "messages": [{"role": "user", "content": "12345678"}]
            }))
            .unwrap();
        // 4 + 8 = 12 字元 → 12/4 = 3
        assert_eq!(estimate_input_tokens(&req), 3);
    }

    #[test]
    fn token_estimate_never_returns_zero_for_nonempty_input() {
        let req: crate::bridge::anthropic::request::MessagesRequest =
            serde_json::from_value(json!({"model": "m", "messages": [{"role":"user","content":"a"}]}))
                .unwrap();
        assert_eq!(estimate_input_tokens(&req), 1);
    }

    #[test]
    fn token_estimate_includes_tool_result_text() {
        let req: crate::bridge::anthropic::request::MessagesRequest =
            serde_json::from_value(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t", "content": "12345678"}
                ]}]
            }))
            .unwrap();
        assert_eq!(estimate_input_tokens(&req), 2);
    }

    #[test]
    fn error_status_maps_from_ai_error() {
        use crate::ai::AiError;
        assert_eq!(status_for(&AiError::AuthFailed), 401);
        assert_eq!(status_for(&AiError::RateLimit { retry_after: None, body: None }), 429);
        assert_eq!(status_for(&AiError::NotConfigured), 400);
        assert_eq!(status_for(&AiError::Network { message: "x".into() }), 502);
    }
}
