use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::config::types::ProviderType;

/// 列模型是使用者打開下拉選單時觸發的，必須感覺得到即時；連不上就早點放棄。
const LIST_MODELS_TIMEOUT: Duration = Duration::from_secs(15);
/// Embedding 一次送一整批文字，本來就慢，逾時要給得比列模型寬。
///
/// 必須嚴格大於 `ingest::EMBED_TIMEOUT`（目前 120s）：那邊用
/// `tokio::time::timeout` 包住 `embedder.embed()` 想給使用者「Embedding
/// request timed out after 120s」這種看得懂的訊息；如果這裡的 client 層
/// timeout 比它短或相等，reqwest 會先掐斷連線，外層那句好懂的訊息永遠
/// 沒機會出現，使用者只會看到一句「error decoding response body」之類、
/// 看不出是逾時的原始 reqwest 錯誤。兩邊改動時要一起調整。
const EMBED_TIMEOUT: Duration = Duration::from_secs(150);

fn build_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("建立 HTTP client 失敗: {e}"))
}

/// 一個 provider 類型該用哪一套 embedding HTTP API。
///
/// 這是「哪些 provider 類型能做 embedding」的唯一來源。`embed`、`list_models`
/// 與 `commands::knowledge_base::resolve_embedder_config` 都是對這個 enum 做
/// **沒有 catch-all** 的 exhaustive match，所以日後新增一種能做 embedding 的
/// provider 時，compiler 會逼著把三邊都補齊，不會只改一邊就編得過、
/// 害 UI 列出實際上用不了的模型。
///
/// `OpenAi` 與 `OpenAiCompatible` 走同一套 HTTP API（前兩處用 or-pattern 合併），
/// 分成兩個 variant 是因為 `resolve_embedder_config` 要給它們不同的預設 base_url。
pub(crate) enum EmbeddingApi {
    Ollama,
    OpenAi,
    OpenAiCompatible,
}

pub(crate) fn embedding_api(provider_type: ProviderType) -> Result<EmbeddingApi, String> {
    match provider_type {
        ProviderType::Ollama => Ok(EmbeddingApi::Ollama),
        ProviderType::Openai => Ok(EmbeddingApi::OpenAi),
        ProviderType::OpenaiCompatible => Ok(EmbeddingApi::OpenAiCompatible),
        other => Err(format!("{other} 不支援 embedding")),
    }
}

#[derive(Clone)]
pub struct EmbedderConfig {
    pub provider_type: ProviderType,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

impl std::fmt::Debug for EmbedderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbedderConfig")
            .field("provider_type", &self.provider_type)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("model", &self.model)
            .finish()
    }
}

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
}

pub struct HttpEmbedder {
    pub config: EmbedderConfig,
    client: reqwest::Client,
}

impl HttpEmbedder {
    pub fn new(config: EmbedderConfig) -> Result<Self, String> {
        let client = build_client(EMBED_TIMEOUT)?;
        Ok(Self { config, client })
    }
}

#[async_trait]
impl Embedder for HttpEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        match embedding_api(self.config.provider_type)? {
            EmbeddingApi::Ollama => embed_ollama(&self.client, &self.config, texts).await,
            EmbeddingApi::OpenAi | EmbeddingApi::OpenAiCompatible => {
                embed_openai_compatible(&self.client, &self.config, texts).await
            }
        }
    }
}

/// 列出某個 embedding provider 可用的模型。
pub async fn list_models(cfg: &EmbedderConfig) -> Result<Vec<String>, String> {
    // 先分類再建 client：不支援的 provider 連 client 都不用做。
    let api = embedding_api(cfg.provider_type)?;
    let client = build_client(LIST_MODELS_TIMEOUT)?;

    match api {
        EmbeddingApi::Ollama => list_ollama(&client, cfg).await,
        EmbeddingApi::OpenAi | EmbeddingApi::OpenAiCompatible => {
            list_openai_compatible(&client, cfg).await
        }
    }
}

async fn list_ollama(
    client: &reqwest::Client,
    cfg: &EmbedderConfig,
) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", cfg.base_url.trim_end_matches('/'));
    let resp = client.get(&url)
        .header("Accept", "application/json")
        .send().await
        .map_err(|e| format!("Ollama tags request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Ollama tags HTTP {}", resp.status()));
    }

    #[derive(Deserialize)]
    struct TagsResponse { models: Vec<TagItem> }
    #[derive(Deserialize)]
    struct TagItem { name: String }

    let parsed: TagsResponse = resp.json().await
        .map_err(|e| format!("Ollama tags parse error: {e}"))?;
    Ok(parsed.models.into_iter().map(|m| m.name).collect())
}

async fn list_openai_compatible(
    client: &reqwest::Client,
    cfg: &EmbedderConfig,
) -> Result<Vec<String>, String> {
    let url = format!("{}/models", cfg.base_url.trim_end_matches('/'));
    // 有些 gateway 靠 Accept 決定回 JSON 還是一頁 HTML 說明，少了它會變成看不懂的 parse error。
    let mut req = client.get(&url).header("Accept", "application/json");
    if let Some(key) = &cfg.api_key {
        req = req.bearer_auth(key);
    }

    let resp = req.send().await
        .map_err(|e| format!("Models request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Models HTTP {}", resp.status()));
    }

    #[derive(Deserialize)]
    struct ModelsResponse { data: Vec<ModelItem> }
    #[derive(Deserialize)]
    struct ModelItem { id: String }

    let parsed: ModelsResponse = resp.json().await
        .map_err(|e| format!("Models parse error: {e}"))?;
    Ok(parsed.data.into_iter().map(|m| m.id).collect())
}

#[derive(Serialize)]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

async fn embed_ollama(
    client: &reqwest::Client,
    cfg: &EmbedderConfig,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    let url = format!("{}/api/embed", cfg.base_url.trim_end_matches('/'));
    let resp = client.post(&url)
        .json(&OllamaEmbedRequest { model: &cfg.model, input: texts })
        .send().await.map_err(|e| format!("Ollama embed request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama embed HTTP {status}: {body}"));
    }

    let parsed: OllamaEmbedResponse = resp.json().await
        .map_err(|e| format!("Ollama embed parse error: {e}"))?;
    Ok(parsed.embeddings)
}

#[derive(Serialize)]
struct OpenAiEmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OpenAiEmbedResponse {
    data: Vec<OpenAiEmbedItem>,
}

#[derive(Deserialize)]
struct OpenAiEmbedItem {
    embedding: Vec<f32>,
    // Some self-hosted OpenAI-compatible servers (local MLX/Qwen gateways in
    // particular) omit `index` and just return items in request order — a
    // required `usize` here made every item in that response fail to
    // deserialize, killing the whole batch with an opaque "error decoding
    // response body". Optional so those servers still work; see fallback
    // ordering logic below.
    index: Option<usize>,
}

async fn embed_openai_compatible(
    client: &reqwest::Client,
    cfg: &EmbedderConfig,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    let url = format!("{}/embeddings", cfg.base_url.trim_end_matches('/'));
    let mut req = client.post(&url).json(&OpenAiEmbedRequest { model: &cfg.model, input: texts });
    if let Some(key) = &cfg.api_key {
        req = req.bearer_auth(key);
    }

    let resp = req.send().await.map_err(|e| format!("Embedding request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Embedding HTTP {status}: {body}"));
    }

    // Read as text first (rather than resp.json()) so a shape mismatch can report
    // what the server actually sent — "error decoding response body" alone gave no
    // way to tell a wrong endpoint from a different response schema.
    let body = resp.text().await.map_err(|e| format!("Embedding response read error: {e}"))?;
    let parsed: OpenAiEmbedResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Embedding parse error: {e} (response: {})", truncate_for_error(&body)))?;

    let mut items = parsed.data;
    // Only sort when every item actually reports an index; a response missing it
    // entirely relies on array order, and sorting on all-None would be a no-op
    // that masks the real ordering if that ever changes.
    if items.iter().all(|i| i.index.is_some()) {
        items.sort_by_key(|i| i.index.unwrap());
    }
    Ok(items.into_iter().map(|i| i.embedding).collect())
}

/// Truncates `s` to at most `max_chars` characters (char-boundary safe) for
/// inclusion in an error message, appending `…` when it was cut.
fn truncate_for_error(s: &str) -> String {
    const MAX_CHARS: usize = 300;
    let mut truncated: String = s.chars().take(MAX_CHARS).collect();
    if s.chars().count() > MAX_CHARS {
        truncated.push('…');
    }
    truncated
}
