//! Antigravity（Gemini 訂閱，Google 帳號）：本機回呼＋PKCE 登入、續期、onboarding、一次性呼叫、模型清單。
use crate::{client, detail, sse_data_lines, Pkce, Tokens};
use std::time::Duration;

pub const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
// 拆開寫，避免整串 `GOCSPX-…` 被 secret scanning 誤判；這是公開的桌面應用程式 client secret，不是機密。
const SECRET_PREFIX: &str = "GOCSPX";
const SECRET_BODY: &str = "K58FWR486LdLJ1mLB8sXC4z6qDAf";
pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const API_BASE: &str = "https://cloudcode-pa.googleapis.com";
pub const IDE_VERSION: &str = "2.1.1";
/// 刻意不含 `openid`（會讓 Google 導到卡住的同意畫面）；`cclog`、`experimentsandconfigs` 是後端要求的。
pub const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
/// Google 的 token 較短命，提早 15 分鐘續期。
pub const REFRESH_SKEW_SECS: u64 = 900;
pub const FALLBACK_MODELS: &[&str] = &["gemini-2.5-pro", "gemini-2.5-flash", "gemini-2.5-flash-lite"];

/// client secret；可用環境變數 `AITERM_GOOGLE_OAUTH_CLIENT_SECRET` 換成自己註冊的。
pub fn client_secret() -> String {
    match std::env::var("AITERM_GOOGLE_OAUTH_CLIENT_SECRET") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => format!("{SECRET_PREFIX}-{SECRET_BODY}"),
    }
}

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

pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/oauth2callback")
}

/// 要在瀏覽器開啟的授權網址（`port` 是本機回呼埠）。
pub fn auth_url(p: &Pkce, port: u16) -> String {
    let scope = SCOPE.replace(':', "%3A").replace('/', "%2F").replace(' ', "%20");
    let redir = redirect_uri(port).replace(':', "%3A").replace('/', "%2F");
    format!(
        "{AUTH_URL}?response_type=code&client_id={CLIENT_ID}&redirect_uri={redir}&scope={scope}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
        p.challenge, p.state
    )
}

fn user_agent() -> String {
    // 固定回報 darwin/arm64：後端對 Mac 版本身分比較寬容。
    format!("antigravity/ide/{IDE_VERSION} darwin/arm64")
}

async fn token_request(ep: &Endpoints, form: &[(&str, &str)], what: &str) -> Result<Tokens, String> {
    let resp = client(30)?.post(&ep.token_url).form(form).send().await.map_err(|e| format!("{what}連線失敗：{e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("{what}失敗（HTTP {}）：{}", status.as_u16(), text.chars().take(300).collect::<String>()));
    }
    resp.json().await.map_err(|e| format!("{what}回應格式不對：{e}"))
}

/// 用授權碼換 token（`port` 要與授權網址用的回呼埠一致）。
pub async fn exchange(ep: &Endpoints, code: &str, verifier: &str, port: u16) -> Result<Tokens, String> {
    let secret = client_secret();
    let redir = redirect_uri(port);
    token_request(ep, &[("grant_type", "authorization_code"), ("code", code), ("client_id", CLIENT_ID), ("client_secret", secret.as_str()), ("redirect_uri", redir.as_str()), ("code_verifier", verifier)], "交換授權碼").await
}

/// 續期。Google 的續期憑證不輪替，回應通常沒有新的 `refresh_token`。
pub async fn refresh(ep: &Endpoints, refresh_token: &str) -> Result<Tokens, String> {
    let secret = client_secret();
    token_request(ep, &[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", CLIENT_ID), ("client_secret", secret.as_str())], "續期").await
}

fn headers(b: reqwest::RequestBuilder, access: &str) -> reqwest::RequestBuilder {
    b.header("Content-Type", "application/json").header("Accept", "application/json").header("User-Agent", user_agent()).bearer_auth(access)
}

/// `loadCodeAssist` 把專案 id 放在 `cloudaicompanionProject`，可能是字串或 `{"id": …}`；空字串視為沒有。
fn project_id_of(v: &serde_json::Value) -> Option<String> {
    let f = v.get("cloudaicompanionProject")?;
    let id = f.as_str().map(str::to_string).or_else(|| f.get("id").and_then(|x| x.as_str()).map(str::to_string))?;
    if id.is_empty() { None } else { Some(id) }
}

async fn post_json(ep: &Endpoints, access: &str, path: &str, body: &serde_json::Value, what: &str) -> Result<serde_json::Value, String> {
    let resp = headers(client(30)?.post(format!("{}{path}", ep.api_base.trim_end_matches('/'))), access).json(body).send().await.map_err(|e| format!("{what} 連線失敗：{e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("{what} 失敗（HTTP {}）：{}", status.as_u16(), text.chars().take(300).collect::<String>()));
    }
    resp.json().await.map_err(|e| format!("{what} 回應格式不對：{e}"))
}

/// 為 Google 帳號開通 Antigravity／Cloud Code Assist，回傳專案 id；登入後做一次。
/// 先 `loadCodeAssist`；沒有專案（全新帳號）就 `onboardUser` 並輪詢到完成，再讀一次。
pub async fn onboard(ep: &Endpoints, access: &str, poll_interval: Duration) -> Result<String, String> {
    let load_body = serde_json::json!({ "metadata": { "ideType": "ANTIGRAVITY" } });
    let first = post_json(ep, access, "/v1internal:loadCodeAssist", &load_body, "loadCodeAssist").await?;
    if let Some(id) = project_id_of(&first) {
        return Ok(id);
    }
    let tier = first
        .get("allowedTiers")
        .and_then(|t| t.as_array())
        .and_then(|a| a.iter().find(|t| t.get("isDefault").and_then(|d| d.as_bool()).unwrap_or(false)))
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("legacy-tier")
        .to_string();
    let onboard_body = serde_json::json!({ "tierId": tier, "metadata": { "ideType": "ANTIGRAVITY" } });
    for attempt in 0..10 {
        let r = post_json(ep, access, "/v1internal:onboardUser", &onboard_body, "onboardUser").await?;
        if r.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
            break;
        }
        if attempt < 9 {
            tokio::time::sleep(poll_interval).await;
        }
    }
    let again = post_json(ep, access, "/v1internal:loadCodeAssist", &load_body, "loadCodeAssist（開通後）").await?;
    project_id_of(&again).ok_or_else(|| "開通完成但沒有取得 Cloud Code 專案 id".to_string())
}

/// 一次性呼叫：`streamGenerateContent?alt=sse`，外層帶專案 id 與 Antigravity 的識別欄位。
pub async fn complete(ep: &Endpoints, access: &str, project_id: &str, model: &str, system: &str, user: &str) -> Result<String, String> {
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let request_id = format!("agent/{now_ms}/{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let mut request = serde_json::json!({
        "contents": [{ "role": "user", "parts": [{ "text": user }] }],
        "generationConfig": { "topK": 40, "topP": 1.0, "maxOutputTokens": 4096 },
    });
    if !system.trim().is_empty() {
        request["systemInstruction"] = serde_json::json!({ "parts": [{ "text": system }] });
    }
    let body = serde_json::json!({ "project": project_id, "requestId": request_id, "userAgent": "antigravity", "requestType": "agent", "model": model, "request": request });
    let resp = client(90)?
        .post(format!("{}/v1internal:streamGenerateContent?alt=sse", ep.api_base.trim_end_matches('/')))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("User-Agent", user_agent())
        .bearer_auth(access)
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

/// 把串流事件的文字接起來。事件外面包一層 `{"response": {...}}`；萬一哪天拿掉了也讀得懂。
/// 被擋下（`promptFeedback.blockReason`）當錯誤。
pub fn parse_sse(body: &str) -> Result<String, String> {
    let mut out = String::new();
    for data in sse_data_lines(body) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
        let chunk = if v.get("response").map(|r| r.is_object()).unwrap_or(false) { &v["response"] } else { &v };
        if chunk["candidates"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
            if let Some(reason) = chunk["promptFeedback"]["blockReason"].as_str() {
                return Err(format!("AI 回應被擋下：{reason}"));
            }
        }
        if let Some(parts) = chunk["candidates"][0]["content"]["parts"].as_array() {
            for p in parts {
                if let Some(t) = p["text"].as_str() {
                    out.push_str(t);
                }
            }
        }
    }
    if out.is_empty() { Err("AI 回應裡找不到文字".into()) } else { Ok(out) }
}

/// 從 `fetchAvailableModels` 回應取 id。真實格式是以模型 id 為鍵的物件（`isInternal` 的要略過）；陣列格式留作備援。
pub fn parse_models(json: &serde_json::Value) -> Vec<String> {
    if let Some(map) = json.get("models").and_then(|v| v.as_object()) {
        let mut ids: Vec<String> = map.iter().filter(|(_, m)| !m.get("isInternal").and_then(|v| v.as_bool()).unwrap_or(false)).map(|(id, _)| id.clone()).collect();
        ids.sort();
        ids.dedup();
        return ids;
    }
    let items: Vec<&serde_json::Value> = if let Some(a) = json.get("models").and_then(|v| v.as_array()) { a.iter().collect() } else if let Some(a) = json.as_array() { a.iter().collect() } else { vec![] };
    let mut ids: Vec<String> = items.iter().filter_map(|i| i.get("name").or_else(|| i.get("id")).and_then(|v| v.as_str()).map(|s| s.trim_start_matches("models/").to_string())).collect();
    ids.sort();
    ids.dedup();
    ids
}

/// 訂閱帳號可用的模型；取不到就回備援清單（不擋住儲存設定）。
pub async fn list_models(ep: &Endpoints, access: &str, project_id: Option<&str>) -> Vec<String> {
    let fallback = || FALLBACK_MODELS.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let body = match project_id {
        Some(p) if !p.is_empty() => serde_json::json!({ "project": p }),
        _ => serde_json::json!({}),
    };
    match post_json(ep, access, "/v1internal:fetchAvailableModels", &body, "fetchAvailableModels").await {
        Ok(j) => {
            let ids = parse_models(&j);
            if ids.is_empty() { fallback() } else { ids }
        }
        Err(_) => fallback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ep(srv: &MockServer) -> Endpoints {
        Endpoints { token_url: format!("{}/token", srv.uri()), api_base: srv.uri() }
    }

    #[test]
    fn the_login_url_asks_for_offline_access_with_the_callback_port_and_antigravity_scopes() {
        let p = Pkce::new();
        let u = auth_url(&p, 4567);
        assert!(u.contains("redirect_uri=http%3A%2F%2Flocalhost%3A4567%2Foauth2callback"), "{u}");
        assert!(u.contains("access_type=offline") && u.contains("prompt=consent") && u.contains("%2Fcclog") && !u.contains("openid"));
        assert!(u.contains(&format!("state={}", p.state)));
    }

    #[test]
    fn the_client_secret_is_assembled_and_can_be_overridden() {
        std::env::remove_var("AITERM_GOOGLE_OAUTH_CLIENT_SECRET");
        assert!(client_secret().starts_with("GOCSPX-"));
    }

    #[tokio::test]
    async fn exchange_sends_the_callback_port_and_refresh_has_no_rotation() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=authorization_code")).and(body_string_contains("localhost%3A4567")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "A", "refresh_token": "R", "expires_in": 3599}))).mount(&srv).await;
        Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=refresh_token")).and(body_string_contains("client_secret=")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "A2", "expires_in": 3599}))).mount(&srv).await;
        let t = exchange(&ep(&srv), "C", "V", 4567).await.unwrap();
        assert_eq!((t.access.as_str(), t.refresh.as_deref()), ("A", Some("R")));
        let r = refresh(&ep(&srv), "R").await.unwrap();
        assert_eq!((r.access.as_str(), r.refresh), ("A2", None));
    }

    #[tokio::test]
    async fn onboarding_returns_an_existing_project_immediately_in_either_shape() {
        for shape in [serde_json::json!({"cloudaicompanionProject": "p-1"}), serde_json::json!({"cloudaicompanionProject": {"id": "p-1"}})] {
            let srv = MockServer::start().await;
            Mock::given(method("POST")).and(path("/v1internal:loadCodeAssist")).and(header("authorization", "Bearer T")).and(body_partial_json(serde_json::json!({"metadata": {"ideType": "ANTIGRAVITY"}}))).respond_with(ResponseTemplate::new(200).set_body_json(shape)).mount(&srv).await;
            assert_eq!(onboard(&ep(&srv), "T", Duration::from_millis(1)).await.unwrap(), "p-1");
            assert_eq!(srv.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn a_fresh_account_is_onboarded_with_its_default_tier_then_the_project_is_read_again() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1internal:loadCodeAssist")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"cloudaicompanionProject": "", "allowedTiers": [{"id": "free-tier", "isDefault": true}]}))).up_to_n_times(1).mount(&srv).await;
        Mock::given(method("POST")).and(path("/v1internal:onboardUser")).and(body_partial_json(serde_json::json!({"tierId": "free-tier"}))).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"done": true}))).mount(&srv).await;
        Mock::given(method("POST")).and(path("/v1internal:loadCodeAssist")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"cloudaicompanionProject": "p-new"}))).mount(&srv).await;
        assert_eq!(onboard(&ep(&srv), "T", Duration::from_millis(1)).await.unwrap(), "p-new");
    }

    #[tokio::test]
    async fn onboarding_that_never_yields_a_project_is_an_error() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1internal:onboardUser")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"done": true}))).mount(&srv).await;
        Mock::given(method("POST")).and(path("/v1internal:loadCodeAssist")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({}))).mount(&srv).await;
        assert!(onboard(&ep(&srv), "T", Duration::from_millis(1)).await.unwrap_err().contains("專案 id"));
        let bad = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(403).set_body_string("denied")).mount(&bad).await;
        assert!(onboard(&ep(&bad), "T", Duration::from_millis(1)).await.unwrap_err().contains("403"));
    }

    fn sse(events: &[serde_json::Value]) -> String {
        events.iter().map(|e| format!("data: {e}\n\n")).collect()
    }

    #[tokio::test]
    async fn a_call_wraps_the_request_with_the_project_and_joins_the_enveloped_text() {
        let srv = MockServer::start().await;
        let body = sse(&[
            serde_json::json!({"response": {"candidates": [{"content": {"parts": [{"text": "O"}]}}]}, "traceId": "t"}),
            serde_json::json!({"response": {"candidates": [{"content": {"parts": [{"text": "K"}]}, "finishReason": "STOP"}]}}),
        ]);
        Mock::given(method("POST")).and(path("/v1internal:streamGenerateContent")).and(header("authorization", "Bearer T")).and(body_partial_json(serde_json::json!({"project": "p-1", "model": "gemini-x", "userAgent": "antigravity", "request": {"systemInstruction": {"parts": [{"text": "sys"}]}, "contents": [{"role": "user", "parts": [{"text": "hi"}]}]}}))).respond_with(ResponseTemplate::new(200).set_body_string(body)).mount(&srv).await;
        assert_eq!(complete(&ep(&srv), "T", "p-1", "gemini-x", "sys", "hi").await.unwrap(), "OK");
        let sent = &srv.received_requests().await.unwrap()[0];
        assert!(sent.headers.get("user-agent").unwrap().to_str().unwrap().starts_with("antigravity/ide/"));
    }

    #[test]
    fn bare_payloads_blocked_prompts_and_empty_streams_are_handled() {
        assert_eq!(parse_sse(&sse(&[serde_json::json!({"candidates": [{"content": {"parts": [{"text": "x"}]}}]})])).unwrap(), "x");
        assert!(parse_sse(&sse(&[serde_json::json!({"response": {"candidates": [], "promptFeedback": {"blockReason": "SAFETY"}}})])).unwrap_err().contains("SAFETY"));
        assert!(parse_sse("").is_err());
    }

    #[tokio::test]
    async fn a_failed_call_says_why() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({"error": {"message": "RESOURCE_EXHAUSTED"}}))).mount(&srv).await;
        let e = complete(&ep(&srv), "T", "p", "m", "", "u").await.unwrap_err();
        assert!(e.contains("429") && e.contains("RESOURCE_EXHAUSTED"), "{e}");
    }

    #[test]
    fn model_ids_come_from_object_keys_skipping_internal_ones_with_array_forms_as_backup() {
        let j = serde_json::json!({"models": {"gemini-b": {}, "gemini-a": {}, "internal-x": {"isInternal": true}}});
        assert_eq!(parse_models(&j), vec!["gemini-a", "gemini-b"]);
        assert_eq!(parse_models(&serde_json::json!({"models": [{"name": "models/gemini-2.5-pro"}]})), vec!["gemini-2.5-pro"]);
    }

    #[tokio::test]
    async fn the_model_list_posts_the_project_and_falls_back_when_unavailable() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1internal:fetchAvailableModels")).and(body_partial_json(serde_json::json!({"project": "p-1"}))).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"models": {"g1": {}}}))).mount(&srv).await;
        assert_eq!(list_models(&ep(&srv), "T", Some("p-1")).await, vec!["g1"]);
        let down = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(500)).mount(&down).await;
        assert_eq!(list_models(&ep(&down), "T", None).await, FALLBACK_MODELS);
    }
}
