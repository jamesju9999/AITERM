//! Codex（ChatGPT 訂閱）：本機 1455 埠回呼＋PKCE 登入、續期、一次性呼叫、模型清單。
use crate::{client, detail, sse_data_lines, Pkce, Tokens};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub const REDIRECT_PORT: u16 = 1455;
pub const API_BASE: &str = "https://chatgpt.com";
pub const CLIENT_VERSION: &str = "0.144.1";
pub const USER_AGENT: &str = "codex-cli/0.144.1 (Windows 10.0.26200; x64)";
/// 提早多久續期（秒）。
pub const REFRESH_SKEW_SECS: u64 = 300;
/// 模型清單取不到時的備援。
pub const FALLBACK_MODELS: &[&str] = &["gpt-5.1-codex", "gpt-5.1-codex-mini"];

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub token_url: String,
    pub api_base: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self { token_url: TOKEN_URL.into(), api_base: API_BASE.into() }
    }
}

/// 要在瀏覽器開啟的授權網址。`prompt=login` 強制重新登入：否則沿用瀏覽器裡既有的 session，
/// 會讓這台電腦自己的那組登入被擠掉（同一個 client_id 的 session takeover）。
pub fn auth_url(p: &Pkce) -> String {
    format!(
        "{AUTH_URL}?response_type=code&client_id={CLIENT_ID}&redirect_uri={REDIRECT_URI}&scope=openid+profile+email+offline_access&code_challenge={}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=codex_cli_rs&prompt=login&state={}",
        p.challenge, p.state
    )
}

/// 從 id_token（JWT）取 `chatgpt_account_id`，之後每個請求都要帶 `chatgpt-account-id` 標頭。
/// 只讀 claim，不驗簽章：token 是直接從供應商的 token 端點經 HTTPS 拿到的。
pub fn account_id_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let v: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    v.get("https://api.openai.com/auth")?.get("chatgpt_account_id")?.as_str().map(str::to_string)
}

async fn token_request(ep: &Endpoints, form: &[(&str, &str)], what: &str) -> Result<Tokens, String> {
    let resp = client(30)?.post(&ep.token_url).header("Accept", "application/json").form(form).send().await.map_err(|e| format!("{what}連線失敗：{e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("{what}失敗（HTTP {}）：{}", status.as_u16(), text.chars().take(300).collect::<String>()));
    }
    resp.json().await.map_err(|e| format!("{what}回應格式不對：{e}"))
}

/// 用授權碼換 token。
pub async fn exchange(ep: &Endpoints, code: &str, verifier: &str) -> Result<Tokens, String> {
    token_request(ep, &[("grant_type", "authorization_code"), ("client_id", CLIENT_ID), ("code", code), ("redirect_uri", REDIRECT_URI), ("code_verifier", verifier)], "交換授權碼").await
}

/// 續期。**續期憑證是一次性的**：成功後舊的立刻失效，新的 `refresh` 必須存下來。
/// 刻意不帶 `scope`：帶了會被當成重新授權，可能讓同一個 client_id 的其他登入失效。
pub async fn refresh(ep: &Endpoints, refresh_token: &str) -> Result<Tokens, String> {
    token_request(ep, &[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", CLIENT_ID)], "續期").await
}

fn apply_headers(b: reqwest::RequestBuilder, access: &str, account_id: Option<&str>) -> reqwest::RequestBuilder {
    let b = b
        .bearer_auth(access)
        .header("originator", "codex_cli_rs")
        .header("User-Agent", USER_AGENT)
        .header("Version", CLIENT_VERSION)
        .header("Openai-Beta", "responses=experimental")
        .header("X-Codex-Beta-Features", "responses_websockets");
    match account_id {
        Some(id) if !id.is_empty() => b.header("chatgpt-account-id", id),
        _ => b,
    }
}

/// 把 Responses API 的串流事件接成完整文字；`response.failed` 當錯誤。
pub fn parse_sse(body: &str) -> Result<String, String> {
    let mut out = String::new();
    for data in sse_data_lines(body) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
        match v["type"].as_str() {
            Some("response.output_text.delta") => out.push_str(v["delta"].as_str().unwrap_or("")),
            Some("response.failed") => {
                let why = v["response"]["error"].to_string();
                return Err(format!("Codex 回應失敗：{}", why.chars().take(200).collect::<String>()));
            }
            _ => {}
        }
    }
    if out.is_empty() { Err("AI 回應裡找不到文字".into()) } else { Ok(out) }
}

/// 一次性呼叫。端點強制 `stream: true`、`store: false`，且一定要有 `instructions`。
pub async fn complete(ep: &Endpoints, access: &str, account_id: Option<&str>, model: &str, system: &str, user: &str) -> Result<String, String> {
    let instructions = if system.trim().is_empty() { "You are a helpful assistant." } else { system };
    let body = serde_json::json!({
        "model": model,
        "instructions": instructions,
        "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": user }] }],
        "stream": true,
        "store": false,
    });
    let resp = apply_headers(client(90)?.post(format!("{}/backend-api/codex/responses", ep.api_base.trim_end_matches('/'))), access, account_id)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI 呼叫失敗：{e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("AI 回應讀取失敗：{e}"))?;
    if !status.is_success() {
        let d = serde_json::from_str::<serde_json::Value>(&text).map(|v| detail(&v)).unwrap_or_default();
        let d = if d.is_empty() { format!("：{}", text.chars().take(200).collect::<String>()) } else { d };
        return Err(format!("AI 回應 HTTP {status}{d}"));
    }
    parse_sse(&text)
}

/// 從模型清單回應取 id；容許已知的幾種形狀。
pub fn parse_models(json: &serde_json::Value) -> Vec<String> {
    let items: Vec<&serde_json::Value> = if let Some(a) = json.get("models").and_then(|v| v.as_array()) {
        a.iter().collect()
    } else if let Some(a) = json.get("data").and_then(|v| v.as_array()) {
        a.iter().collect()
    } else if let Some(a) = json.as_array() {
        a.iter().collect()
    } else if let Some(o) = json.as_object() {
        o.values().collect()
    } else {
        vec![]
    };
    let mut ids: Vec<String> = items.iter().filter_map(|i| i.get("slug").or_else(|| i.get("id")).or_else(|| i.get("model")).and_then(|v| v.as_str()).map(str::to_string)).collect();
    ids.sort();
    ids.dedup();
    ids
}

/// 訂閱帳號可用的模型；取不到就回備援清單（不擋住儲存設定）。
pub async fn list_models(ep: &Endpoints, access: &str, account_id: Option<&str>) -> Vec<String> {
    let fallback = || FALLBACK_MODELS.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let Ok(c) = client(20) else { return fallback() };
    let req = apply_headers(c.get(format!("{}/backend-api/codex/models?client_version={CLIENT_VERSION}", ep.api_base.trim_end_matches('/'))), access, account_id);
    match req.send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(j) => {
                let ids = parse_models(&j);
                if ids.is_empty() { fallback() } else { ids }
            }
            Err(_) => fallback(),
        },
        _ => fallback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ep(srv: &MockServer) -> Endpoints {
        Endpoints { token_url: format!("{}/oauth/token", srv.uri()), api_base: srv.uri() }
    }

    fn jwt(payload: serde_json::Value) -> String {
        format!("h.{}.s", URL_SAFE_NO_PAD.encode(payload.to_string()))
    }

    #[test]
    fn the_account_id_is_read_from_the_nested_claim_and_bad_tokens_give_none() {
        let t = jwt(serde_json::json!({"https://api.openai.com/auth": {"chatgpt_account_id": "acct-1"}}));
        assert_eq!(account_id_from_id_token(&t).as_deref(), Some("acct-1"));
        assert_eq!(account_id_from_id_token(&jwt(serde_json::json!({"sub": "x"}))), None);
        assert_eq!(account_id_from_id_token("not-a-jwt"), None);
    }

    #[test]
    fn the_login_url_forces_a_fresh_login_so_the_local_session_is_not_taken_over() {
        let p = Pkce::new();
        let u = auth_url(&p);
        assert!(u.contains("prompt=login") && u.contains("originator=codex_cli_rs") && u.contains(&format!("state={}", p.state)) && u.contains(&format!("code_challenge={}", p.challenge)));
    }

    #[tokio::test]
    async fn exchange_and_refresh_post_forms_and_refresh_has_no_scope() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).and(path("/oauth/token")).and(body_string_contains("grant_type=authorization_code")).and(body_string_contains("code_verifier=V")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "A", "refresh_token": "R", "expires_in": 3600, "id_token": "I"}))).mount(&srv).await;
        Mock::given(method("POST")).and(path("/oauth/token")).and(body_string_contains("grant_type=refresh_token")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "A2", "refresh_token": "R2", "expires_in": 3600}))).mount(&srv).await;
        let t = exchange(&ep(&srv), "C", "V").await.unwrap();
        assert_eq!((t.access.as_str(), t.refresh.as_deref(), t.id_token.as_deref()), ("A", Some("R"), Some("I")));
        let r = refresh(&ep(&srv), "R").await.unwrap();
        assert_eq!(r.refresh.as_deref(), Some("R2"));
        let sent = srv.received_requests().await.unwrap();
        assert!(!String::from_utf8_lossy(&sent[1].body).contains("scope"), "續期不帶 scope");
    }

    #[tokio::test]
    async fn a_rejected_refresh_reports_the_http_status() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({"error": "refresh_token_reused"}))).mount(&srv).await;
        let e = refresh(&ep(&srv), "old").await.unwrap_err();
        assert!(e.contains("HTTP 400") && e.contains("refresh_token_reused"), "{e}");
    }

    fn sse(events: &[serde_json::Value]) -> String {
        events.iter().map(|e| format!("event: x\ndata: {e}\n\n")).collect()
    }

    #[tokio::test]
    async fn a_call_sends_the_codex_identity_and_joins_the_streamed_text() {
        let srv = MockServer::start().await;
        let body = sse(&[serde_json::json!({"type": "response.created"}), serde_json::json!({"type": "response.output_text.delta", "delta": "O"}), serde_json::json!({"type": "response.output_text.delta", "delta": "K"}), serde_json::json!({"type": "response.completed", "response": {}})]);
        Mock::given(method("POST")).and(path("/backend-api/codex/responses")).and(header("authorization", "Bearer TOK")).and(header("chatgpt-account-id", "acct-1")).and(header("originator", "codex_cli_rs")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(body)).mount(&srv).await;
        assert_eq!(complete(&ep(&srv), "TOK", Some("acct-1"), "gpt-x", "sys", "hi").await.unwrap(), "OK");
        let sent: serde_json::Value = serde_json::from_slice(&srv.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!((sent["stream"].as_bool(), sent["store"].as_bool(), sent["instructions"].as_str()), (Some(true), Some(false), Some("sys")));
        assert_eq!(sent["input"][0]["content"][0]["type"], "input_text");
    }

    #[tokio::test]
    async fn empty_instructions_are_replaced_because_the_endpoint_rejects_them() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_string(sse(&[serde_json::json!({"type": "response.output_text.delta", "delta": "x"})]))).mount(&srv).await;
        complete(&ep(&srv), "T", None, "m", "", "hi").await.unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&srv.received_requests().await.unwrap()[0].body).unwrap();
        assert!(!sent["instructions"].as_str().unwrap().is_empty());
        assert!(srv.received_requests().await.unwrap()[0].headers.get("chatgpt-account-id").is_none(), "沒有帳號 id 就不帶標頭");
    }

    #[tokio::test]
    async fn failures_say_why() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).and(path("/backend-api/codex/responses")).respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({"detail": "The 'x' model is not supported"}))).mount(&srv).await;
        let e = complete(&ep(&srv), "T", None, "x", "s", "u").await.unwrap_err();
        assert!(e.contains("400") && e.contains("not supported"), "{e}");
        let failed = sse(&[serde_json::json!({"type": "response.failed", "response": {"error": {"message": "quota"}}})]);
        assert!(parse_sse(&failed).unwrap_err().contains("quota"));
        assert!(parse_sse("data: {\"type\":\"response.created\"}\n").is_err(), "沒有任何文字也是錯誤");
    }

    #[tokio::test]
    async fn models_come_from_the_catalog_or_fall_back_to_the_builtin_list() {
        let srv = MockServer::start().await;
        Mock::given(method("GET")).and(path("/backend-api/codex/models")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"models": [{"slug": "b"}, {"slug": "a"}, {"id": "a"}]}))).mount(&srv).await;
        assert_eq!(list_models(&ep(&srv), "T", None).await, vec!["a", "b"]);
        let down = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(500)).mount(&down).await;
        assert_eq!(list_models(&ep(&down), "T", None).await, FALLBACK_MODELS);
    }
}
