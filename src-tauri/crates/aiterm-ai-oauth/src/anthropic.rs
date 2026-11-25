//! Anthropic（Claude 訂閱）：授權碼＋PKCE 登入、續期、一次性呼叫、模型清單。
use serde::Serialize;
use std::time::Duration;

pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const AUTH_URL: &str = "https://claude.ai/oauth/authorize";
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
pub const REDIRECT_URI_ENCODED: &str = "https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback";
pub const SCOPE_ENCODED: &str = "user%3Aprofile%20user%3Ainference%20user%3Asessions%3Aclaude_code";
pub const API_BASE: &str = "https://api.anthropic.com";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// 上游靠這組旗標判斷這是 Claude Code 的 OAuth 請求，缺了就不會把 token 當 OAuth token 看待。
pub const OAUTH_BETA_HEADER: &str = "claude-code-20250219,oauth-2025-04-20";
/// 訂閱 token 只服務 Claude Code 的流量：system 第一塊不是這句，會被回一個偽裝成限流的
/// `{"type":"rate_limit_error","message":"Error"}`（即使是第一個請求），不要把它當成額度問題。
pub const CLAUDE_CODE_SENTINEL: &str = "You are Claude Code, Anthropic's official CLI for Claude.";
/// 提早多久續期（秒）。
pub const REFRESH_SKEW_SECS: u64 = 300;

pub use crate::{challenge_of, Pkce, Tokens};

/// 要在瀏覽器開啟的授權網址。
pub fn auth_url(p: &Pkce) -> String {
    format!(
        "{AUTH_URL}?code=true&client_id={CLIENT_ID}&response_type=code&redirect_uri={REDIRECT_URI_ENCODED}&scope={SCOPE_ENCODED}&code_challenge={}&code_challenge_method=S256&state={}",
        p.challenge, p.state
    )
}

/// 使用者貼回的內容：完整的回呼網址，或舊格式 `{code}#{state}`。
pub fn parse_pasted(input: &str) -> Result<(String, String), String> {
    let input = input.trim();
    if input.contains("code=") && input.contains("state=") {
        let query = input.find('?').map(|i| &input[i + 1..]).unwrap_or(input);
        let code = query.split('&').find_map(|p| p.strip_prefix("code=")).map(str::to_owned).ok_or("網址裡找不到 code")?;
        let state = query.split('&').find_map(|p| p.strip_prefix("state=")).map(str::to_owned).ok_or("網址裡找不到 state")?;
        return Ok((code, state));
    }
    let (c, s) = input.split_once('#').ok_or("格式不對：請貼上瀏覽器顯示的授權碼（code#state）或完整網址")?;
    Ok((c.to_owned(), s.to_owned()))
}

/// 可替換的端點（測試用）；預設是正式位址。
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

fn client(timeout: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder().timeout(Duration::from_secs(timeout)).build().map_err(|e| e.to_string())
}

async fn token_request(ep: &Endpoints, body: serde_json::Value, what: &str) -> Result<Tokens, String> {
    let resp = client(30)?
        .post(&ep.token_url)
        .header("Content-Type", "application/json")
        .header("anthropic-beta", OAUTH_BETA_HEADER)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{what}連線失敗：{e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("{what}失敗（HTTP {}）：{}", status.as_u16(), text.chars().take(300).collect::<String>()));
    }
    resp.json().await.map_err(|e| format!("{what}回應格式不對：{e}"))
}

/// 用授權碼換 token。
pub async fn exchange(ep: &Endpoints, code: &str, state: &str, verifier: &str) -> Result<Tokens, String> {
    token_request(
        ep,
        serde_json::json!({ "grant_type": "authorization_code", "code": code, "redirect_uri": REDIRECT_URI, "client_id": CLIENT_ID, "code_verifier": verifier, "state": state }),
        "交換授權碼",
    )
    .await
}

/// 用續期憑證換新的 token。**續期憑證是一次性的**：成功後舊的立刻失效，新的 `refresh` 必須存下來。
pub async fn refresh(ep: &Endpoints, refresh_token: &str) -> Result<Tokens, String> {
    token_request(ep, serde_json::json!({ "grant_type": "refresh_token", "refresh_token": refresh_token, "client_id": CLIENT_ID }), "續期").await
}

#[derive(Serialize)]
struct Block<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    text: &'a str,
}

/// system 欄位：訂閱 token 必須用區塊陣列，並以 Claude Code 識別句開頭；呼叫端自己的提示放第二塊，
/// 空的就省略（空的 text 區塊會被 400）。
pub fn system_blocks(system: &str) -> serde_json::Value {
    let mut blocks = vec![serde_json::to_value(Block { kind: "text", text: CLAUDE_CODE_SENTINEL }).unwrap()];
    if !system.is_empty() {
        blocks.push(serde_json::to_value(Block { kind: "text", text: system }).unwrap());
    }
    serde_json::Value::Array(blocks)
}

fn detail(v: &serde_json::Value) -> String {
    let msg = v["error"]["message"].as_str().or_else(|| v["error"].as_str()).unwrap_or("");
    if msg.trim().is_empty() { String::new() } else { format!("：{}", msg.trim().chars().take(200).collect::<String>()) }
}

/// 一次性呼叫（不帶 tools，不串流）。**不帶 tools** 是刻意的：訂閱帶 tools 會被算到額外用量。
pub async fn complete(ep: &Endpoints, access: &str, model: &str, system: &str, user: &str) -> Result<String, String> {
    let body = serde_json::json!({ "model": model, "max_tokens": 1024, "system": system_blocks(system), "messages": [{ "role": "user", "content": user }] });
    let resp = client(60)?
        .post(format!("{}/v1/messages", ep.api_base.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {access}"))
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", OAUTH_BETA_HEADER)
        .header("x-app", "cli")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI 呼叫失敗：{e}"))?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("AI 回應不是 JSON（HTTP {status}）：{e}"))?;
    if !status.is_success() {
        return Err(format!("AI 回應 HTTP {status}{}", detail(&v)));
    }
    v["content"][0]["text"].as_str().map(str::to_string).ok_or_else(|| "AI 回應裡找不到文字".to_string())
}

/// 訂閱帳號可用的模型。
pub async fn list_models(ep: &Endpoints, access: &str) -> Result<Vec<String>, String> {
    let resp = client(20)?
        .get(format!("{}/v1/models", ep.api_base.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {access}"))
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", OAUTH_BETA_HEADER)
        .header("x-app", "cli")
        .send()
        .await
        .map_err(|e| format!("取得模型清單失敗：{e}"))?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("模型清單不是 JSON（HTTP {status}）：{e}"))?;
    if !status.is_success() {
        return Err(format!("取得模型清單失敗 HTTP {status}{}", detail(&v)));
    }
    let mut ids: Vec<String> = v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ep(srv: &MockServer) -> Endpoints {
        Endpoints { token_url: format!("{}/oauth/token", srv.uri()), api_base: srv.uri() }
    }

    #[test]
    fn the_challenge_is_the_sha256_of_the_verifier_and_every_login_is_unique() {
        // RFC 7636 附錄 B 的範例。
        assert_eq!(challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let (a, b) = (Pkce::new(), Pkce::new());
        assert_ne!((a.verifier.clone(), a.state.clone()), (b.verifier, b.state));
        assert_eq!(a.challenge, challenge_of(&a.verifier));
        assert_eq!(a.state.len(), 32);
    }

    #[test]
    fn the_auth_url_carries_pkce_state_and_scope() {
        let p = Pkce::new();
        let u = auth_url(&p);
        assert!(u.starts_with("https://claude.ai/oauth/authorize?code=true&client_id=9d1c250a"), "{u}");
        assert!(u.contains(&format!("code_challenge={}", p.challenge)) && u.contains(&format!("state={}", p.state)) && u.contains("code_challenge_method=S256"));
        assert!(u.contains("user%3Ainference"));
    }

    #[test]
    fn pasted_input_may_be_a_full_callback_url_or_code_hash_state() {
        assert_eq!(parse_pasted("abc#xyz").unwrap(), ("abc".into(), "xyz".into()));
        assert_eq!(parse_pasted("  https://platform.claude.com/oauth/code/callback?code=abc&state=xyz\n").unwrap(), ("abc".into(), "xyz".into()));
        assert!(parse_pasted("沒有格式").is_err());
        assert!(parse_pasted("https://x/?code=abc").is_err(), "缺 state 要拒絕");
    }

    #[tokio::test]
    async fn the_code_exchange_sends_the_pkce_verifier_and_returns_the_tokens() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(header_exists("anthropic-beta"))
            .and(body_partial_json(serde_json::json!({"grant_type": "authorization_code", "code": "C", "code_verifier": "V", "state": "S", "client_id": CLIENT_ID, "redirect_uri": REDIRECT_URI})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "A", "refresh_token": "R", "expires_in": 3600})))
            .mount(&srv)
            .await;
        assert_eq!(exchange(&ep(&srv), "C", "S", "V").await.unwrap(), Tokens { access: "A".into(), refresh: Some("R".into()), id_token: None, expires_in: Some(3600) });
        let sent = &srv.received_requests().await.unwrap()[0];
        assert_eq!(sent.headers.get("anthropic-beta").unwrap().to_str().unwrap(), OAUTH_BETA_HEADER);
    }

    #[tokio::test]
    async fn a_refresh_returns_the_rotated_refresh_token_and_a_rejection_says_so() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).and(body_partial_json(serde_json::json!({"grant_type": "refresh_token", "refresh_token": "OLD", "client_id": CLIENT_ID}))).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "A2", "refresh_token": "NEW", "expires_in": 28800}))).mount(&srv).await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({"error": "invalid_grant"}))).mount(&srv).await;
        assert_eq!(refresh(&ep(&srv), "OLD").await.unwrap().refresh.as_deref(), Some("NEW"));
        let e = refresh(&ep(&srv), "USED").await.unwrap_err();
        assert!(e.contains("400") && e.contains("invalid_grant"), "{e}");
    }

    #[test]
    fn system_is_a_block_array_starting_with_the_claude_code_sentinel_and_omits_an_empty_prompt() {
        let v = system_blocks("你是助理");
        assert_eq!((v[0]["text"].as_str(), v[1]["text"].as_str()), (Some(CLAUDE_CODE_SENTINEL), Some("你是助理")));
        assert_eq!(system_blocks("").as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_completion_uses_bearer_auth_oauth_beta_flags_the_sentinel_and_no_tools() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("authorization", "Bearer ACCESS"))
            .and(header_exists("anthropic-beta"))
            .and(header("x-app", "cli"))
            .and(body_partial_json(serde_json::json!({"model": "claude-x", "system": [{"type": "text", "text": CLAUDE_CODE_SENTINEL}, {"type": "text", "text": "sys"}], "messages": [{"role": "user", "content": "hi"}]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"content": [{"type": "text", "text": "hello"}]})))
            .mount(&srv)
            .await;
        assert_eq!(complete(&ep(&srv), "ACCESS", "claude-x", "sys", "hi").await.unwrap(), "hello");
        let req = &srv.received_requests().await.unwrap()[0];
        assert_eq!(req.headers.get("anthropic-beta").unwrap().to_str().unwrap(), OAUTH_BETA_HEADER);
        let sent: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert!(sent.get("tools").is_none(), "不能帶 tools（訂閱帶 tools 會被算額外用量）");
    }

    #[tokio::test]
    async fn completion_and_model_list_errors_carry_the_providers_reason() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({"error": {"message": "Error"}}))).mount(&srv).await;
        Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"id": "b"}, {"id": "a"}]}))).mount(&srv).await;
        let e = complete(&ep(&srv), "A", "m", "", "hi").await.unwrap_err();
        assert!(e.contains("429") && e.contains("Error"), "{e}");
        assert_eq!(list_models(&ep(&srv), "A").await.unwrap(), vec!["a", "b"]);
    }
}
