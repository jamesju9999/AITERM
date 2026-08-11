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
use crate::bridge::anthropic::response::{error_frame, MessageAggregator};
use crate::bridge::factory::{build, Upstream};
use crate::bridge::upstream::anthropic::ClientHeaders;
use crate::bridge::upstream::{BridgeUpstream, UpstreamResponse};
use crate::bridge::tool_meta::ToolMetaCache;
use crate::bridge::{auth, model_map};
use crate::config::types::TierMapping;
use crate::config::ConfigStore;
use crate::secret::SecretStore;
use futures_util::StreamExt;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub secrets: Arc<SecretStore>,
    pub token: Arc<String>,
    /// 存活期跨單一請求，見 `tool_meta.rs` 的模組說明。
    pub tool_meta: Arc<ToolMetaCache>,
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
    //
    // 例外是 RateLimit：它的 Display 只有一句 "rate limit exceeded"，把上游的
    // body 整個丟掉。但「每分鐘打太多次」和「方案額度用完、還有 17 天才重置」
    // 對使用者的意義完全相反——後者重試毫無意義。實測 Codex 回的是
    //   {"error":{"type":"usage_limit_reached","plan_type":"free",
    //             "resets_in_seconds":1460795,...}}
    // 而我們只轉一句 "rate limit exceeded"，害人往速率限制的方向猜了半天。
    match err {
        AiError::RateLimit { body: Some(body), .. } => match upstream_detail(body) {
            Some(detail) => format!("{err}: {detail}"),
            // 解析不出來就原樣附上（截斷）——看得懂總比一句空話好。
            None => format!("{err}: {}", truncate_chars(body, 300)),
        },
        _ => err.to_string(),
    }
}

/// 從上游錯誤 body 取出人看得懂的說明。
///
/// OpenAI / Anthropic / Codex / Gemini 的錯誤都長成 `{"error":{"message":...}}`，
/// 所以只認這一個形狀；認不出來由呼叫端退回原樣附上。
fn upstream_detail(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let err = v.get("error")?;
    let mut out = err.get("message").and_then(Value::as_str)?.to_string();

    // 免費方案撞到的是額度而不是速率，這個欄位是關鍵線索。
    if let Some(plan) = err.get("plan_type").and_then(Value::as_str) {
        out.push_str(&format!(" (plan: {plan})"));
    }
    // 給「還要等多久」而不是 epoch 秒數：使用者要判斷的是該不該重試。
    if let Some(secs) = err.get("resets_in_seconds").and_then(Value::as_u64) {
        out.push_str(&format!(" (resets in {})", humanize_duration(secs)));
    }
    Some(out)
}

fn humanize_duration(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86400),
    }
}

/// 依字元截斷，不是位元組——上游訊息可能含非 ASCII，切在多位元組字元中間會 panic。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
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
    let cfg = state.config.get();
    // resolve 回傳擁有權（見 model_map.rs 的註解），這裡不需要再 clone。
    let mapping = match model_map::resolve(&cfg.claude_bridge, &req.model) {
        Ok(m) => m,
        Err(msg) => {
            log::warn!("bridge 無法映射模型「{}」：{msg}", req.model);
            return json_error(StatusCode::BAD_REQUEST, "invalid_request_error", &msg);
        }
    };

    // 客戶端那端只看得到最終的錯誤字串，沒有這一行就無從得知請求究竟被
    // 導到哪個供應商 —— 尤其是 Claude Code 送真實型號、靠子字串後備規則
    // 判層級的時候。
    log::info!(
        "bridge 請求 model={} → provider={} model={} stream={}",
        req.model,
        mapping.provider_id,
        mapping.model,
        req.stream == Some(true),
    );

    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let client_headers = ClientHeaders {
        beta: headers.get("anthropic-beta").and_then(|v| v.to_str().ok()).map(String::from),
        version: headers.get("anthropic-version").and_then(|v| v.to_str().ok()).map(String::from),
    };

    if req.stream == Some(true) {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("connection", "keep-alive")
            .body(Body::from_stream(super::stream::run(
                state, mapping, req, raw, message_id, client_headers,
            )))
            .expect("建立 SSE 回應不應失敗")
    } else {
        messages_non_streaming(state, mapping, req, raw, message_id, client_headers).await
    }
}

/// 非串流路徑：等上游整段回應收完再一次回傳一個 JSON `Message`，而不是 SSE。
///
/// OpenAI 家族仍然對上游用串流請求（`build_body` 照舊送 `stream: true`），
/// 只是把收到的 `UpstreamEvent` 聚合成一個完整的 Message JSON 再回——這樣
/// 請求翻譯、SSE 解析、tool_calls 累積器全部原封不動重用。Anthropic 家族
/// 的上游本來看到 `stream: false` 就會回一個完整 JSON 物件（不是 SSE），
/// 原樣轉發即可，不解析。
async fn messages_non_streaming(
    state: AppState,
    mapping: TierMapping,
    req: MessagesRequest,
    raw: Value,
    message_id: String,
    client_headers: ClientHeaders,
) -> Response {
    let up = match build(&state.config, &state.secrets, &state.tool_meta, &mapping.provider_id).await {
        Ok(u) => u,
        Err(e) => return ai_error_response(&e),
    };

    match up {
        Upstream::Anthropic(a) => match a.send_raw(&raw, &mapping.model, &client_headers).await {
            Ok(UpstreamResponse::Passthrough(resp)) => passthrough_response(resp).await,
            // send_raw 只會回 Passthrough，這條分支留著只為了讓 match 窮盡。
            Ok(UpstreamResponse::Events(_)) => {
                json_error(StatusCode::INTERNAL_SERVER_ERROR, "api_error", "Anthropic 上游不應回串流事件。")
            }
            Err(e) => ai_error_response(&e),
        },
        // ChatgptWeb 與 OpenAi 走同一種收斂方式：都回 Events，靠
        // MessageAggregator 聚成一則非串流回應。
        Upstream::ChatgptWeb(c) => match c.send(&req, &mapping.model).await {
            Ok(UpstreamResponse::Events(mut events)) => {
                let mut agg = MessageAggregator::new(
                    message_id,
                    req.model.clone(),
                    estimate_input_tokens(&req),
                );
                while let Some(item) = events.next().await {
                    match item {
                        Ok(ev) => agg.push(ev),
                        Err(e) => return ai_error_response(&e),
                    }
                }
                Json(agg.finish()).into_response()
            }
            Ok(UpstreamResponse::Passthrough(_)) => json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_error",
                "ChatGPT Web 上游不應回 passthrough。",
            ),
            Err(e) => ai_error_response(&e),
        },
        Upstream::OpenAi(o) => match o.send(&req, &mapping.model).await {
            Ok(UpstreamResponse::Events(mut events)) => {
                let mut agg =
                    MessageAggregator::new(message_id, req.model.clone(), estimate_input_tokens(&req));
                while let Some(item) = events.next().await {
                    match item {
                        Ok(ev) => agg.push(ev),
                        Err(e) => return ai_error_response(&e),
                    }
                }
                Json(agg.finish()).into_response()
            }
            Ok(UpstreamResponse::Passthrough(_)) => {
                json_error(StatusCode::INTERNAL_SERVER_ERROR, "api_error", "OpenAI 上游不應回 passthrough。")
            }
            Err(e) => ai_error_response(&e),
        },
        Upstream::Codex(c) => match c.send(&req, &mapping.model).await {
            Ok(UpstreamResponse::Events(mut events)) => {
                let mut agg =
                    MessageAggregator::new(message_id, req.model.clone(), estimate_input_tokens(&req));
                while let Some(item) = events.next().await {
                    match item {
                        Ok(ev) => agg.push(ev),
                        Err(e) => return ai_error_response(&e),
                    }
                }
                Json(agg.finish()).into_response()
            }
            Ok(UpstreamResponse::Passthrough(_)) => {
                json_error(StatusCode::INTERNAL_SERVER_ERROR, "api_error", "Codex 上游不應回 passthrough。")
            }
            Err(e) => ai_error_response(&e),
        },
        Upstream::Antigravity(a) => match a.send(&req, &mapping.model).await {
            Ok(UpstreamResponse::Events(mut events)) => {
                let mut agg =
                    MessageAggregator::new(message_id, req.model.clone(), estimate_input_tokens(&req));
                while let Some(item) = events.next().await {
                    match item {
                        Ok(ev) => agg.push(ev),
                        Err(e) => return ai_error_response(&e),
                    }
                }
                Json(agg.finish()).into_response()
            }
            Ok(UpstreamResponse::Passthrough(_)) => {
                json_error(StatusCode::INTERNAL_SERVER_ERROR, "api_error", "Antigravity 上游不應回 passthrough。")
            }
            Err(e) => ai_error_response(&e),
        },
    }
}

/// 上游 [`AiError`] → 非串流的 JSON 錯誤回應（HTTP 狀態碼 + body），
/// 跟串流路徑用的 `error_frame_for`（SSE error frame）是同一份資訊的兩種
/// 輸出形態。
fn ai_error_response(err: &AiError) -> Response {
    // 串流路徑在 stream.rs 有對應的 log::warn!，非串流這條原本沒有——
    // 於是 Claude Code 用非串流請求撞牆時，server 端一片空白，只能靠
    // 客戶端那個被截斷的錯誤字串猜。這是實際排錯時吃過的虧。
    log::warn!("bridge 非串流請求失敗：{err:?}");
    json_error(
        StatusCode::from_u16(status_for(err)).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        error_kind(err),
        &error_text(err),
    )
}

/// Anthropic 家族的非串流回應：原樣轉發上游的 body 與 content-type，不解析。
async fn passthrough_response(resp: reqwest::Response) -> Response {
    let status = resp.status();
    let content_type = resp.headers().get("content-type").cloned();
    match resp.bytes().await {
        Ok(bytes) => {
            let mut builder = Response::builder().status(status);
            if let Some(ct) = content_type {
                builder = builder.header("content-type", ct);
            }
            builder.body(Body::from(bytes)).expect("建立 passthrough 回應不應失敗")
        }
        Err(e) => json_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            &format!("讀取上游回應失敗：{e}"),
        ),
    }
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

    /// 實測從 Codex 收到的 body（見 AITerm.log）。免費方案額度用完、17 天後重置，
    /// 這跟「每分鐘打太多次」完全是兩回事，訊息必須說得出差別。
    #[test]
    fn rate_limit_surfaces_codex_usage_limit_body() {
        let err = AiError::RateLimit {
            retry_after: None,
            body: Some(
                r#"{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached","plan_type":"free","resets_at":1787744844,"eligible_promo":null,"resets_in_seconds":1460795}}"#
                    .to_string(),
            ),
        };
        let text = error_text(&err);
        assert!(text.contains("The usage limit has been reached"), "實際：{text}");
        assert!(text.contains("plan: free"), "實際：{text}");
        assert!(text.contains("resets in 16d"), "實際：{text}");
    }

    #[test]
    fn rate_limit_without_body_keeps_plain_display() {
        let err = AiError::RateLimit { retry_after: None, body: None };
        assert_eq!(error_text(&err), "rate limit exceeded");
    }

    /// 認不出形狀就原樣附上——看得懂總比一句空話好。
    #[test]
    fn rate_limit_with_unparseable_body_falls_back_to_raw() {
        let err = AiError::RateLimit {
            retry_after: None,
            body: Some("<html>502 Bad Gateway</html>".to_string()),
        };
        assert!(error_text(&err).contains("502 Bad Gateway"));
    }

    /// 上游訊息可能含非 ASCII，依位元組截斷會切在多位元組字元中間而 panic。
    #[test]
    fn truncate_is_char_safe() {
        let long = "額".repeat(400);
        let out = truncate_chars(&long, 300);
        assert_eq!(out.chars().count(), 301); // 300 個字 + 省略號
    }

    #[test]
    fn humanize_duration_picks_a_readable_unit() {
        assert_eq!(humanize_duration(45), "45s");
        assert_eq!(humanize_duration(600), "10m");
        assert_eq!(humanize_duration(7200), "2h");
        assert_eq!(humanize_duration(1460795), "16d");
    }

    /// 其他變體不受影響。
    #[test]
    fn other_variants_keep_display_text() {
        assert_eq!(error_text(&AiError::AuthFailed), "authentication failed (check your API key)");
    }
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
