use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::config::types::ProviderType;

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
    pub fn new(config: EmbedderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self { config, client }
    }
}

#[async_trait]
impl Embedder for HttpEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        match self.config.provider_type {
            ProviderType::Ollama => embed_ollama(&self.client, &self.config, texts).await,
            ProviderType::Openai | ProviderType::OpenaiCompatible => {
                embed_openai_compatible(&self.client, &self.config, texts).await
            }
            other => Err(format!("{other} 不支援 embedding")),
        }
    }
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
