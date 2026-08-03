use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::config::types::ProviderType;

/// 列模型是使用者打開下拉選單時觸發的，必須感覺得到即時；連不上就早點放棄。
const LIST_MODELS_TIMEOUT: Duration = Duration::from_secs(15);
/// Embedding 一次送一整批文字，本來就慢，逾時要給得比列模型寬。
const EMBED_TIMEOUT: Duration = Duration::from_secs(60);

fn build_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("建立 HTTP client 失敗: {e}"))
}

/// 一個 provider 類型該用哪一套 embedding HTTP API。
///
/// 這是「哪些 provider 類型能做 embedding」在本模組的唯一來源。`embed` 與
/// `list_models` 都是對這個 enum 做 **沒有 catch-all** 的 exhaustive match，
/// 所以日後新增一種能做 embedding 的 provider 時，compiler 會逼著把兩邊都補齊，
/// 不會只改一邊就編得過、害 UI 列出實際上用不了的模型。
///
/// 注意 `commands::knowledge_base::resolve_embedder_config` 還有一份同樣的判斷，
/// 但它同時要挑各類型的預設 base_url（Openai 與 OpenaiCompatible 在那裡行為不同，
/// 無法收斂成這裡的同一個 variant），所以沒有共用；改這裡時記得順手看那邊。
enum EmbeddingApi {
    Ollama,
    OpenAiCompatible,
}

fn embedding_api(provider_type: ProviderType) -> Result<EmbeddingApi, String> {
    match provider_type {
        ProviderType::Ollama => Ok(EmbeddingApi::Ollama),
        ProviderType::Openai | ProviderType::OpenaiCompatible => Ok(EmbeddingApi::OpenAiCompatible),
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
            EmbeddingApi::OpenAiCompatible => {
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
        EmbeddingApi::OpenAiCompatible => list_openai_compatible(&client, cfg).await,
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
    index: usize,
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

    let parsed: OpenAiEmbedResponse = resp.json().await
        .map_err(|e| format!("Embedding parse error: {e}"))?;
    let mut items = parsed.data;
    items.sort_by_key(|i| i.index);
    Ok(items.into_iter().map(|i| i.embedding).collect())
}
