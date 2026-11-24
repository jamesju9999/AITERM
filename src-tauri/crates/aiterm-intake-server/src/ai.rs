//! Server 自己的 AI 設定與一次性補全（題目分類、群組「是不是新問題」用）。
//! 只支援用 API 金鑰的供應商；訂閱 OAuth 需要瀏覽器登入，無頭 server 做不到。
use aiterm_intake_core::host::{AiComplete, Secrets};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;

pub const SETTING_KEY: &str = "server_ai";
pub const API_KEY_SECRET: &str = "ai_api_key";

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct AiSettings {
    /// `openai`｜`anthropic`｜`openai_compat`｜`ollama`；空字串＝未設定。
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// 空字串用供應商的預設位址。
    #[serde(default)]
    pub base_url: String,
}

pub const PROVIDERS: [&str; 4] = ["openai", "anthropic", "openai_compat", "ollama"];

pub async fn load(pool: &SqlitePool) -> AiSettings {
    aiterm_intake_core::store::get_setting(pool, SETTING_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub async fn save(pool: &SqlitePool, s: &AiSettings) -> Result<(), String> {
    if !s.provider.is_empty() && !PROVIDERS.contains(&s.provider.as_str()) {
        return Err(format!("不支援的 AI 供應商：{}（可用：{}）", s.provider, PROVIDERS.join("、")));
    }
    aiterm_intake_core::store::set_setting(pool, SETTING_KEY, &serde_json::to_string(s).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())
}

/// 已設定供應商、模型，且（除了 Ollama）有金鑰才算可用。
pub fn configured(s: &AiSettings, secrets: &dyn Secrets) -> bool {
    !s.provider.is_empty() && !s.model.is_empty() && (s.provider == "ollama" || secrets.get(API_KEY_SECRET).is_some())
}

pub struct ServerAi {
    pool: SqlitePool,
    secrets: Arc<dyn Secrets>,
}

impl ServerAi {
    pub fn new(pool: SqlitePool, secrets: Arc<dyn Secrets>) -> Self {
        Self { pool, secrets }
    }
}

fn join(base: &str, default: &str, path: &str) -> String {
    let b = if base.trim().is_empty() { default } else { base.trim() };
    format!("{}/{}", b.trim_end_matches('/'), path.trim_start_matches('/'))
}

#[async_trait::async_trait]
impl AiComplete for ServerAi {
    async fn complete(&self, system: &str, user: &str) -> Result<String, String> {
        let s = load(&self.pool).await;
        if !configured(&s, self.secrets.as_ref()) {
            return Err("尚未設定 AI".into());
        }
        let key = self.secrets.get(API_KEY_SECRET).unwrap_or_default();
        complete_with(&s, &key, system, user).await
    }
}

/// 實際呼叫（與設定儲存分開，方便測試）。
pub async fn complete_with(s: &AiSettings, key: &str, system: &str, user: &str) -> Result<String, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(60)).build().map_err(|e| e.to_string())?;
    let (req, pick): (reqwest::RequestBuilder, fn(&serde_json::Value) -> Option<String>) = match s.provider.as_str() {
        "anthropic" => (
            client
                .post(join(&s.base_url, "https://api.anthropic.com", "v1/messages"))
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .json(&serde_json::json!({ "model": s.model, "max_tokens": 1024, "system": system, "messages": [{ "role": "user", "content": user }] })),
            |v| v["content"][0]["text"].as_str().map(str::to_string),
        ),
        "ollama" => (
            client
                .post(join(&s.base_url, "http://127.0.0.1:11434", "api/chat"))
                .json(&serde_json::json!({ "model": s.model, "stream": false, "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }] })),
            |v| v["message"]["content"].as_str().map(str::to_string),
        ),
        "openai" | "openai_compat" => (
            client
                .post(join(&s.base_url, "https://api.openai.com", "v1/chat/completions"))
                .bearer_auth(key)
                .json(&serde_json::json!({ "model": s.model, "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }] })),
            |v| v["choices"][0]["message"]["content"].as_str().map(str::to_string),
        ),
        other => return Err(format!("不支援的 AI 供應商：{other}")),
    };
    let res = req.send().await.map_err(|e| format!("AI 呼叫失敗：{e}"))?;
    let status = res.status();
    let v: serde_json::Value = res.json().await.map_err(|e| format!("AI 回應不是 JSON：{e}"))?;
    if !status.is_success() {
        return Err(format!("AI 回應 HTTP {status}"));
    }
    pick(&v).ok_or_else(|| "AI 回應裡找不到文字".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn settings(provider: &str, base: &str) -> AiSettings {
        AiSettings { provider: provider.into(), model: "m".into(), base_url: base.into() }
    }

    #[tokio::test]
    async fn openai_style_request_and_response() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer K"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"choices":[{"message":{"content":"{\"kind\":\"question\"}"}}]})))
            .mount(&srv)
            .await;
        for p in ["openai", "openai_compat"] {
            assert_eq!(complete_with(&settings(p, &srv.uri()), "K", "sys", "hi").await.unwrap(), "{\"kind\":\"question\"}");
        }
    }

    #[tokio::test]
    async fn anthropic_style_request_and_response() {
        let srv = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "K"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"content":[{"type":"text","text":"ok"}]})))
            .mount(&srv)
            .await;
        assert_eq!(complete_with(&settings("anthropic", &srv.uri()), "K", "s", "u").await.unwrap(), "ok");
    }

    #[tokio::test]
    async fn ollama_needs_no_key() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/chat")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"message":{"content":"hey"}}))).mount(&srv).await;
        assert_eq!(complete_with(&settings("ollama", &srv.uri()), "", "s", "u").await.unwrap(), "hey");
    }

    #[tokio::test]
    async fn http_errors_and_odd_bodies_are_errors() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({"error":"no"}))).mount(&srv).await;
        assert!(complete_with(&settings("openai", &srv.uri()), "bad", "s", "u").await.unwrap_err().contains("401"));
        let srv2 = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"x":1}))).mount(&srv2).await;
        assert!(complete_with(&settings("openai", &srv2.uri()), "k", "s", "u").await.is_err());
        assert!(complete_with(&settings("nope", ""), "k", "s", "u").await.is_err());
    }

    struct Mem(parking_lot::Mutex<std::collections::HashMap<String, String>>);
    impl Secrets for Mem {
        fn get(&self, id: &str) -> Option<String> {
            self.0.lock().get(id).cloned()
        }
        fn set(&self, id: &str, v: &str) -> Result<(), String> {
            self.0.lock().insert(id.into(), v.into());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.0.lock().remove(id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn unconfigured_ai_is_an_error_so_callers_use_their_defaults() {
        let pools = crate::db::open_memory().await.unwrap();
        let secrets: Arc<dyn Secrets> = Arc::new(Mem(Default::default()));
        let ai = ServerAi::new(pools.intake.clone(), secrets.clone());
        assert_eq!(ai.complete("s", "u").await.unwrap_err(), "尚未設定 AI");
        save(&pools.intake, &settings("openai", "")).await.unwrap();
        assert!(!configured(&load(&pools.intake).await, secrets.as_ref()), "沒有金鑰仍算未設定");
        secrets.set(API_KEY_SECRET, "K").unwrap();
        assert!(configured(&load(&pools.intake).await, secrets.as_ref()));
        assert!(save(&pools.intake, &settings("banana", "")).await.is_err());
    }
}
