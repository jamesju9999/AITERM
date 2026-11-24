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

/// 最近一次實際呼叫的結果（記憶體內；重啟就清空）。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Health {
    /// 最近一次成功的時間（Unix 秒）。
    pub last_ok: Option<i64>,
    /// 最近一次成功呼叫花的毫秒數。
    pub last_ms: Option<u64>,
    /// 最近一次呼叫的錯誤；成功後清除。
    pub error: Option<String>,
}

pub struct ServerAi {
    pool: SqlitePool,
    secrets: Arc<dyn Secrets>,
    health: parking_lot::Mutex<Health>,
}

impl ServerAi {
    pub fn new(pool: SqlitePool, secrets: Arc<dyn Secrets>) -> Self {
        Self { pool, secrets, health: Default::default() }
    }

    pub fn health(&self) -> Health {
        self.health.lock().clone()
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
        let began = std::time::Instant::now();
        let out = complete_with(&s, &key, system, user).await;
        let mut h = self.health.lock();
        match &out {
            Ok(_) => *h = Health { last_ok: Some(crate::db::now()), last_ms: Some(began.elapsed().as_millis() as u64), error: None },
            Err(e) => h.error = Some(e.clone()),
        }
        out
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
    let v: serde_json::Value = res.json().await.map_err(|e| format!("AI 回應不是 JSON（HTTP {status}）：{e}"))?;
    if !status.is_success() {
        return Err(format!("AI 回應 HTTP {status}{}", error_detail(&v)));
    }
    pick(&v).ok_or_else(|| "AI 回應裡找不到文字".to_string())
}

/// 供應商回的錯誤原因（金鑰錯、模型不存在…），截短後附在錯誤訊息後面。
fn error_detail(v: &serde_json::Value) -> String {
    let msg = v["error"]["message"].as_str().or_else(|| v["error"].as_str()).or_else(|| v["message"].as_str()).unwrap_or("");
    if msg.trim().is_empty() {
        String::new()
    } else {
        format!("：{}", msg.trim().chars().take(200).collect::<String>())
    }
}

/// 問供應商有哪些模型，排序後回傳。
pub async fn list_models_with(s: &AiSettings, key: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let (req, field): (reqwest::RequestBuilder, &str) = match s.provider.as_str() {
        "anthropic" => (client.get(join(&s.base_url, "https://api.anthropic.com", "v1/models?limit=100")).header("x-api-key", key).header("anthropic-version", "2023-06-01"), "data"),
        "ollama" => (client.get(join(&s.base_url, "http://127.0.0.1:11434", "api/tags")), "models"),
        "openai" => (client.get(join(&s.base_url, "https://api.openai.com", "v1/models")).bearer_auth(key), "data"),
        "openai_compat" => {
            if s.base_url.trim().is_empty() {
                return Err("OpenAI 相容的供應商需要填位址".into());
            }
            // 相容伺服器的位址常常已經含 /v1；兩種都試一次。
            let base = s.base_url.trim().trim_end_matches('/');
            let url = if base.ends_with("/v1") { format!("{base}/models") } else { format!("{base}/v1/models") };
            (client.get(url).bearer_auth(key), "data")
        }
        other => return Err(format!("不支援的 AI 供應商：{other}")),
    };
    let res = req.send().await.map_err(|e| format!("取得模型清單失敗：{e}"))?;
    let status = res.status();
    let v: serde_json::Value = res.json().await.map_err(|e| format!("模型清單不是 JSON（HTTP {status}）：{e}"))?;
    if !status.is_success() {
        return Err(format!("取得模型清單失敗 HTTP {status}{}", error_detail(&v)));
    }
    let mut ids: Vec<String> = v[field].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str()).or_else(|| m["model"].as_str()).map(str::to_string)).collect()).unwrap_or_default();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// 「測試連線」的結果。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TestResult {
    pub ok: bool,
    pub ms: u64,
    /// 成功時模型的回覆（截短）；失敗時是錯誤原因。
    pub message: String,
}

/// 送一個極短的請求，確認供應商、模型與金鑰都能用。
pub async fn test_with(s: &AiSettings, key: &str) -> TestResult {
    let began = std::time::Instant::now();
    let out = if s.provider.is_empty() || s.model.trim().is_empty() {
        Err("請先選供應商並填模型".to_string())
    } else if s.provider != "ollama" && key.trim().is_empty() {
        Err("尚未設定 API 金鑰".to_string())
    } else {
        complete_with(s, key, "You are a connectivity check. Reply with the single word OK.", "ping").await
    };
    let ms = began.elapsed().as_millis() as u64;
    match out {
        Ok(text) => TestResult { ok: true, ms, message: text.trim().chars().take(80).collect() },
        Err(e) => TestResult { ok: false, ms, message: e },
    }
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

    #[tokio::test]
    async fn model_lists_are_read_from_each_providers_own_shape_sorted_and_deduplicated() {
        let srv = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v1/models")).and(header("authorization", "Bearer K")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[{"id":"gpt-b"},{"id":"gpt-a"},{"id":"gpt-b"}]}))).mount(&srv).await;
        Mock::given(method("GET")).and(path("/v1/models")).and(header("x-api-key", "K")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[{"id":"claude-x"}]}))).mount(&srv).await;
        Mock::given(method("GET")).and(path("/api/tags")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"models":[{"name":"llama3"},{"name":"qwen"}]}))).mount(&srv).await;
        assert_eq!(list_models_with(&settings("openai", &srv.uri()), "K").await.unwrap(), vec!["gpt-a", "gpt-b"]);
        assert_eq!(list_models_with(&settings("openai_compat", &srv.uri()), "K").await.unwrap(), vec!["gpt-a", "gpt-b"]);
        assert_eq!(list_models_with(&settings("anthropic", &srv.uri()), "K").await.unwrap(), vec!["claude-x"]);
        assert_eq!(list_models_with(&settings("ollama", &srv.uri()), "").await.unwrap(), vec!["llama3", "qwen"]);
        assert!(list_models_with(&settings("openai_compat", ""), "K").await.unwrap_err().contains("位址"), "相容供應商沒填位址要明講");
    }

    #[tokio::test]
    async fn a_failed_model_list_says_why() {
        let srv = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({"error":{"message":"Incorrect API key provided"}}))).mount(&srv).await;
        let e = list_models_with(&settings("openai", &srv.uri()), "bad").await.unwrap_err();
        assert!(e.contains("401") && e.contains("Incorrect API key"), "{e}");
    }

    #[tokio::test]
    async fn the_connection_test_reports_success_with_timing_and_failure_with_the_providers_reason() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).and(header("authorization", "Bearer good")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"choices":[{"message":{"content":" OK \n"}}]}))).mount(&srv).await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({"error":{"message":"The model `nope` does not exist"}}))).mount(&srv).await;
        let ok = test_with(&settings("openai", &srv.uri()), "good").await;
        assert_eq!((ok.ok, ok.message.as_str()), (true, "OK"));
        let bad = test_with(&AiSettings { model: "nope".into(), ..settings("openai", &srv.uri()) }, "wrong").await;
        assert!(!bad.ok && bad.message.contains("404") && bad.message.contains("does not exist"), "{}", bad.message);
        assert!(test_with(&settings("openai", &srv.uri()), "").await.message.contains("金鑰"));
        assert!(test_with(&AiSettings { model: String::new(), ..settings("openai", "") }, "k").await.message.contains("模型"));
    }

    #[tokio::test]
    async fn real_calls_leave_a_health_record_and_a_later_failure_keeps_the_last_success_time() {
        let srv = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"choices":[{"message":{"content":"x"}}]}))).up_to_n_times(1).mount(&srv).await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({"error":{"message":"boom"}}))).mount(&srv).await;
        let pools = crate::db::open_memory().await.unwrap();
        let secrets: Arc<dyn Secrets> = Arc::new(Mem(Default::default()));
        secrets.set(API_KEY_SECRET, "K").unwrap();
        save(&pools.intake, &settings("openai", &srv.uri())).await.unwrap();
        let ai = ServerAi::new(pools.intake.clone(), secrets);
        assert_eq!(ai.health(), Health::default());
        ai.complete("s", "u").await.unwrap();
        let h1 = ai.health();
        assert!(h1.last_ok.is_some() && h1.error.is_none());
        ai.complete("s", "u").await.unwrap_err();
        let h2 = ai.health();
        assert_eq!(h2.last_ok, h1.last_ok, "後來失敗不會抹掉最近一次成功的時間");
        assert!(h2.error.as_deref().unwrap_or("").contains("boom"));
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
