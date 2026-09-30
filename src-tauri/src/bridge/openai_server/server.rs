//! OpenAI 相容 server 的 router 與 handlers。

use std::collections::VecDeque;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use serde_json::{json, Value};

use super::request::{translate, Translated};
use super::response::{
    error_body, error_chunk, ChunkEncoder, CompletionAggregator, DONE_FRAME,
};
use super::{alias_map, anthropic_events};
use crate::ai::AiError;
use crate::bridge::factory::{build, Upstream};
use crate::bridge::server::{error_text, status_for};
use crate::bridge::tool_meta::ToolMetaCache;
use crate::bridge::upstream::anthropic::ClientHeaders;
use crate::bridge::upstream::{BridgeUpstream, UpstreamEvent, UpstreamResponse};
use crate::bridge::auth;
use crate::config::types::TierMapping;
use crate::config::ConfigStore;
use crate::secret::SecretStore;

/// 請求 body 上限。含 base64 圖片的多輪對話可能很大，但不該無上限。
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub secrets: Arc<SecretStore>,
    pub token: Arc<String>,
    pub tool_meta: Arc<ToolMetaCache>,
}

type EventStream = BoxStream<'static, Result<UpstreamEvent, AiError>>;

pub fn router(state: AppState, allow_lan: bool) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/models", get(models).options(preflight))
        .route("/v1/chat/completions", post(chat_completions).options(preflight))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        // 只有開放區網時才加 CORS 標頭。綁 127.0.0.1 時不加：本機的非瀏覽器客戶端
        // 用不到，而且這樣任意網頁的 JS 都讀不到回應，擋掉「惡意網頁打本機
        // server」。區網模式下 Bearer key 本身就是憑證、不靠 cookie，`*` 不會
        // 擴大攻擊面。
        .layer(axum::middleware::map_response(move |mut resp: Response| async move {
            if allow_lan {
                let h = resp.headers_mut();
                h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
                h.insert(
                    "access-control-allow-headers",
                    HeaderValue::from_static("authorization, content-type"),
                );
                h.insert(
                    "access-control-allow-methods",
                    HeaderValue::from_static("GET, POST, OPTIONS"),
                );
            }
            resp
        }))
        .with_state(state)
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

fn json_error(status: StatusCode, kind: &str, code: Option<&str>, message: &str) -> Response {
    (status, Json(error_body(kind, code, message))).into_response()
}

/// 只認 `Authorization: Bearer`（OpenAI 慣例）；不吃 `x-api-key`。
fn authorize(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let authorization = headers.get("authorization").and_then(|v| v.to_str().ok());
    match auth::extract_token(authorization, None) {
        Some(provided) if auth::token_matches(&state.token, &provided) => None,
        _ => Some(json_error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            Some("invalid_api_key"),
            "API key 不正確。請使用 AITerm 設定頁顯示的 key（Authorization: Bearer <key>）。",
        )),
    }
}

async fn models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(deny) = authorize(&state, &headers) {
        return deny;
    }
    let cfg = state.config.get();
    Json(models_body(&cfg.openai_server)).into_response()
}

pub fn models_body(cfg: &crate::config::types::OpenAiServerConfig) -> Value {
    let data: Vec<Value> = alias_map::list(cfg)
        .iter()
        .map(|a| json!({"id": a.alias, "object": "model", "created": 0, "owned_by": "aiterm"}))
        .collect();
    json!({"object": "list", "data": data})
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(deny) = authorize(&state, &headers) {
        return deny;
    }
    let Ok(raw) = serde_json::from_slice::<Value>(&body) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_request_error", None, "請求不是合法的 JSON。");
    };
    let translated = match translate(&raw) {
        Ok(t) => t,
        Err(msg) => return json_error(StatusCode::BAD_REQUEST, "invalid_request_error", None, &msg),
    };
    let client_model = translated.request.model.clone();
    let mapping = match alias_map::resolve(&state.config.get().openai_server, &client_model) {
        Some(m) => m,
        None => {
            log::warn!("openai-server 找不到模型別名「{client_model}」");
            return json_error(
                StatusCode::NOT_FOUND,
                "invalid_request_error",
                Some("model_not_found"),
                &format!("The model `{client_model}` does not exist. 請到 AITerm 設定 → OpenAI 相容伺服器 新增這個別名。"),
            );
        }
    };
    // 客戶端只看得到最終的錯誤字串；沒有這一行就無從得知請求被導到哪個供應商。
    log::info!(
        "openai-server 請求 model={client_model} → provider={} model={} stream={}",
        mapping.provider_id,
        mapping.model,
        translated.stream,
    );

    // 先開上游、再決定回應形態：上游的 401／429 要以真正的 HTTP 狀態碼回給客戶端，
    // 而不是先送 200 再塞一個 error frame。
    let events = match open_events(&state, &mapping, &translated).await {
        Ok(e) => e,
        Err(e) => return ai_error_response(&e),
    };
    let id = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());

    if translated.stream {
        let enc = ChunkEncoder::new(id, client_model, translated.include_usage);
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("connection", "keep-alive")
            .body(Body::from_stream(sse_stream(events, enc)))
            .expect("建立 SSE 回應不應失敗")
    } else {
        aggregate(events, id, client_model).await
    }
}

/// 開啟上游並統一成 [`UpstreamEvent`] 串流。
///
/// Anthropic 家族一律要求 `stream:true`，非串流客戶端由聚合器收斂——與 Claude
/// Code 橋接對 OpenAI 上游的作法一致，避免維護兩條解析路徑。
async fn open_events(
    state: &AppState,
    mapping: &TierMapping,
    t: &Translated,
) -> Result<EventStream, AiError> {
    let up = build(&state.config, &state.secrets, &state.tool_meta, &mapping.provider_id).await?;
    let resp = match up {
        Upstream::Anthropic(a) => {
            let mut raw = t.raw.clone();
            raw["stream"] = json!(true);
            a.send_raw(&raw, &mapping.model, &ClientHeaders::default()).await?
        }
        Upstream::OpenAi(o) => o.send(&t.request, &mapping.model).await?,
        Upstream::Codex(c) => c.send(&t.request, &mapping.model).await?,
        Upstream::Antigravity(a) => a.send(&t.request, &mapping.model).await?,
        Upstream::ChatgptWeb(c) => c.send(&t.request, &mapping.model).await?,
    };
    Ok(match resp {
        // 只有 Anthropic 家族會回 Passthrough（Anthropic SSE，需要解析）。
        UpstreamResponse::Passthrough(r) => Box::pin(anthropic_events::into_events(r)),
        UpstreamResponse::Events(s) => s,
    })
}

fn sse_stream(
    events: EventStream,
    enc: ChunkEncoder,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    struct S {
        events: EventStream,
        enc: ChunkEncoder,
        queue: VecDeque<String>,
        ended: bool,
    }
    let s = S { events, enc, queue: VecDeque::new(), ended: false };
    futures_util::stream::unfold(s, |mut s| async move {
        loop {
            if let Some(f) = s.queue.pop_front() {
                return Some((Ok(Bytes::from(f)), s));
            }
            if s.ended {
                return None;
            }
            match s.events.next().await {
                Some(Ok(ev)) => s.queue.extend(s.enc.push(ev)),
                Some(Err(e)) => {
                    log::warn!("openai-server 串流中斷：{e:?}");
                    s.ended = true;
                    let (kind, _) = error_kind(&e);
                    s.queue.push_back(error_chunk(kind, &error_message(&e)));
                    s.queue.push_back(DONE_FRAME.to_string());
                }
                None => {
                    s.ended = true;
                    s.queue.extend(s.enc.finish());
                }
            }
        }
    })
}

async fn aggregate(mut events: EventStream, id: String, model: String) -> Response {
    let mut agg = CompletionAggregator::new(id, model);
    while let Some(item) = events.next().await {
        match item {
            Ok(ev) => agg.push(ev),
            Err(e) => return ai_error_response(&e),
        }
    }
    Json(agg.finish()).into_response()
}

fn error_kind(err: &AiError) -> (&'static str, Option<&'static str>) {
    match err {
        AiError::AuthFailed => ("authentication_error", Some("upstream_auth_failed")),
        AiError::RateLimit { .. } => ("rate_limit_error", Some("rate_limit_exceeded")),
        AiError::NotConfigured | AiError::InvalidInput { .. } => ("invalid_request_error", None),
        _ => ("api_error", None),
    }
}

fn error_message(err: &AiError) -> String {
    match err {
        // 客戶端會以為是它自己的 key 錯了，說清楚是上游。
        AiError::AuthFailed => {
            "上游供應商認證失敗（不是你的 AITerm API key）。請到 AITerm 重新登入或檢查該供應商的金鑰。".into()
        }
        _ => error_text(err),
    }
}

fn ai_error_response(err: &AiError) -> Response {
    log::warn!("openai-server 請求失敗：{err:?}");
    let (kind, code) = error_kind(err);
    json_error(
        StatusCode::from_u16(status_for(err)).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        kind,
        code,
        &error_message(err),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{ModelAlias, OpenAiServerConfig};

    #[test]
    fn models_body_lists_aliases_in_openai_shape() {
        let cfg = OpenAiServerConfig {
            aliases: vec![ModelAlias { alias: "gpt-4o".into(), provider_id: "p".into(), model: "m".into() }],
            ..Default::default()
        };
        assert_eq!(
            models_body(&cfg),
            json!({"object": "list", "data": [{"id": "gpt-4o", "object": "model", "created": 0, "owned_by": "aiterm"}]})
        );
    }

    #[test]
    fn models_body_is_empty_list_without_aliases() {
        assert_eq!(models_body(&OpenAiServerConfig::default()), json!({"object": "list", "data": []}));
    }

    #[test]
    fn error_kinds() {
        assert_eq!(error_kind(&AiError::AuthFailed).0, "authentication_error");
        assert_eq!(
            error_kind(&AiError::RateLimit { retry_after: None, body: None }),
            ("rate_limit_error", Some("rate_limit_exceeded"))
        );
        assert_eq!(error_kind(&AiError::NotConfigured).0, "invalid_request_error");
        assert_eq!(error_kind(&AiError::Network { message: "x".into() }).0, "api_error");
    }

    #[test]
    fn upstream_auth_failure_message_says_it_is_upstream() {
        assert!(error_message(&AiError::AuthFailed).contains("上游"));
    }
}
