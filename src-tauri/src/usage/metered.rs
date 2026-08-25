//! 包在 `AiProvider` 外的記帳裝飾器。
//!
//! `GenerateChunk` 的消費端散在 6 個檔案（`commands/ai.rs` 一支就有 7 處
//! `rx.recv()` 迴圈），逐一加記帳保證會漏。改在 `AiRouter` 解析出 provider
//! 時包一層，一個接點覆蓋全部呼叫端。
//!
//! ```text
//! 呼叫端 ──tx──▶ MeteredProvider ──tx2──▶ 真正的 provider
//!                      └─ 攔截 usage ─▶ usage.db
//! ```

use crate::ai::{
    AiError, AiProvider, GenerateChunk, GenerateRequest, GenerateWithToolsResult, McpToolDefinition,
    TokenUsage,
};
use crate::usage::store::UsageStore;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct MeteredProvider {
    inner: Arc<dyn AiProvider>,
    store: Arc<UsageStore>,
    provider_id: String,
    model: String,
}

impl MeteredProvider {
    pub fn new(
        inner: Arc<dyn AiProvider>,
        store: Arc<UsageStore>,
        provider_id: String,
        model: String,
    ) -> Self {
        Self { inner, store, provider_id, model }
    }

    /// 建立中繼 channel，回傳「給內層用的 tx」與「轉發任務的 handle」。
    /// handle 完成時給出攔截到的最後一筆 usage。
    fn tap(
        &self,
        out: mpsc::Sender<GenerateChunk>,
    ) -> (mpsc::Sender<GenerateChunk>, tokio::task::JoinHandle<Option<TokenUsage>>) {
        let (tx2, mut rx2) = mpsc::channel::<GenerateChunk>(16);
        let handle = tokio::spawn(async move {
            let mut last: Option<TokenUsage> = None;
            while let Some(chunk) = rx2.recv().await {
                // 任何 chunk 上的 usage 都要收 —— Anthropic 放在 done:false 上。
                if chunk.usage.is_some() {
                    last = chunk.usage;
                }
                // 送不出去代表呼叫端已經走了，繼續收乾內層以免它卡在 send。
                let _ = out.send(chunk).await;
            }
            last
        });
        (tx2, handle)
    }

    /// 等轉發任務結束、拿到 usage 就寫入。**任何失敗都只記 log。**
    async fn finish(&self, handle: tokio::task::JoinHandle<Option<TokenUsage>>) {
        let usage = match handle.await {
            Ok(u) => u,
            Err(e) => {
                log::warn!("用量轉發任務失敗（不影響 AI 流程）: {e}");
                return;
            }
        };
        // 上游沒回 usage 就不寫。不要用估算值補。
        let Some(usage) = usage else { return };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.store.record(&self.provider_id, &self.model, usage, now).await;
    }
}

#[async_trait]
impl AiProvider for MeteredProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn display_name(&self) -> &str {
        self.inner.display_name()
    }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let (tx2, handle) = self.tap(tx);
        // tx2 在 inner.generate 返回時被丟棄，轉發任務隨之結束。
        let result = self.inner.generate(req, tx2).await;
        self.finish(handle).await;
        result
    }

    async fn generate_json(
        &self,
        req: GenerateRequest,
        schema: serde_json::Value,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let (tx2, handle) = self.tap(tx);
        let result = self.inner.generate_json(req, schema, tx2).await;
        self.finish(handle).await;
        result
    }

    async fn health_check(&self) -> Result<(), AiError> {
        self.inner.health_check().await
    }

    async fn generate_with_tools(
        &self,
        req: GenerateRequest,
        tools: Vec<McpToolDefinition>,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<GenerateWithToolsResult, AiError> {
        let (tx2, handle) = self.tap(tx);
        let result = self.inner.generate_with_tools(req, tools, tx2).await;
        self.finish(handle).await;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    /// 假 provider：吐出固定的 chunk 序列，最後一個帶 usage。
    struct FakeProvider {
        chunks: Vec<GenerateChunk>,
    }

    #[async_trait]
    impl AiProvider for FakeProvider {
        fn id(&self) -> &str { "fake" }
        fn display_name(&self) -> &str { "Fake" }
        async fn generate(
            &self,
            _req: GenerateRequest,
            tx: mpsc::Sender<GenerateChunk>,
        ) -> Result<(), AiError> {
            for c in &self.chunks {
                tx.send(c.clone()).await.ok();
            }
            Ok(())
        }
        async fn health_check(&self) -> Result<(), AiError> { Ok(()) }
    }

    /// 欄位名稱取自 `ai/mod.rs:129-135` 與 `:77-87`，不要憑印象改。
    fn req() -> GenerateRequest {
        GenerateRequest {
            system_prompt: String::new(),
            messages: vec![],
            context: EnvSnapshot {
                os: "macos".into(),
                shell: "zsh".into(),
                cwd: PathBuf::from("/"),
                recent_output: None,
                dir_listing: None,
            },
            mode: QueryMode::Chat,
            max_tokens: None,
        }
    }

    async fn store() -> (Arc<UsageStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Arc::new(UsageStore::new_at(dir.path().join("u.db")).await), dir)
    }

    /// The decorator forwards each trait method explicitly, so a method it
    /// forgets silently falls back to the trait default — here that would
    /// drop the schema and disable logit-level enforcement with no error.
    #[tokio::test]
    async fn forwards_the_json_schema_to_the_inner_provider() {
        use std::sync::Mutex;

        struct SchemaSpy {
            seen: Mutex<Option<serde_json::Value>>,
        }

        #[async_trait]
        impl AiProvider for SchemaSpy {
            fn id(&self) -> &str { "spy" }
            fn display_name(&self) -> &str { "Spy" }
            async fn generate(
                &self,
                _req: GenerateRequest,
                tx: mpsc::Sender<GenerateChunk>,
            ) -> Result<(), AiError> {
                tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await.ok();
                Ok(())
            }
            async fn generate_json(
                &self,
                _req: GenerateRequest,
                schema: serde_json::Value,
                tx: mpsc::Sender<GenerateChunk>,
            ) -> Result<(), AiError> {
                *self.seen.lock().unwrap() = Some(schema);
                tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await.ok();
                Ok(())
            }
            async fn health_check(&self) -> Result<(), AiError> { Ok(()) }
        }

        let (store, _d) = store().await;
        let inner = Arc::new(SchemaSpy { seen: Mutex::new(None) });
        let metered = MeteredProvider::new(inner.clone(), store, "p".into(), "m".into());

        let schema = serde_json::json!({"type": "object"});
        let (tx, mut rx) = mpsc::channel(16);
        metered.generate_json(req(), schema.clone(), tx).await.expect("generate_json");
        while rx.recv().await.is_some() {}

        assert_eq!(
            inner.seen.lock().unwrap().clone(),
            Some(schema),
            "MeteredProvider 必須把 schema 轉發下去，否則強制解碼會無聲失效"
        );
    }

    #[tokio::test]
    async fn forwards_every_chunk_unchanged() {
        let (store, _d) = store().await;
        let inner = Arc::new(FakeProvider {
            chunks: vec![
                GenerateChunk { delta: "he".into(), done: false, usage: None },
                GenerateChunk { delta: "llo".into(), done: false, usage: None },
                GenerateChunk { delta: String::new(), done: true, usage: None },
            ],
        });
        let metered = MeteredProvider::new(inner, store, "p".into(), "m".into());

        let (tx, mut rx) = mpsc::channel(16);
        metered.generate(req(), tx).await.expect("generate");

        let mut text = String::new();
        let mut count = 0;
        while let Some(c) = rx.recv().await {
            text.push_str(&c.delta);
            count += 1;
        }
        assert_eq!(text, "hello", "轉發不得改動內容");
        assert_eq!(count, 3, "每個 chunk 都要轉發，一個都不能吞");
    }

    #[tokio::test]
    async fn records_usage_from_final_chunk() {
        let (store, _d) = store().await;
        let inner = Arc::new(FakeProvider {
            chunks: vec![GenerateChunk {
                delta: String::new(),
                done: true,
                usage: Some(TokenUsage {
                    prompt: 22, completion: 7, cache_read: 4096, cache_write: 150,
                }),
            }],
        });
        let metered = MeteredProvider::new(inner, store.clone(), "anthropic-pro".into(),
                                           "claude-sonnet-4-5".into());
        let (tx, mut rx) = mpsc::channel(16);
        metered.generate(req(), tx).await.expect("generate");
        while rx.recv().await.is_some() {}

        let rows = store.summary_since(0).await.expect("summary");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider_id, "anthropic-pro");
        assert_eq!(rows[0].model, "claude-sonnet-4-5");
        assert_eq!(rows[0].prompt_tokens, 22);
        assert_eq!(rows[0].cache_read_tokens, 4096);
    }

    #[tokio::test]
    async fn no_usage_means_no_record() {
        // 上游沒回 usage 就不寫，不要用估算值補 —— 那會讓統計失去可信度。
        let (store, _d) = store().await;
        let inner = Arc::new(FakeProvider {
            chunks: vec![GenerateChunk { delta: "x".into(), done: true, usage: None }],
        });
        let metered = MeteredProvider::new(inner, store.clone(), "p".into(), "m".into());
        let (tx, mut rx) = mpsc::channel(16);
        metered.generate(req(), tx).await.expect("generate");
        while rx.recv().await.is_some() {}

        assert!(store.summary_since(0).await.expect("summary").is_empty());
    }

    #[tokio::test]
    async fn usage_on_non_final_chunk_is_still_captured() {
        // Anthropic 的 usage 出現在 done:false 的 message_delta 上
        // （見 anthropic.rs:496），不是最後那個 chunk。
        let (store, _d) = store().await;
        let inner = Arc::new(FakeProvider {
            chunks: vec![
                GenerateChunk {
                    delta: String::new(),
                    done: false,
                    usage: Some(TokenUsage { prompt: 9, completion: 4, cache_read: 0, cache_write: 0 }),
                },
                GenerateChunk { delta: String::new(), done: true, usage: None },
            ],
        });
        let metered = MeteredProvider::new(inner, store.clone(), "p".into(), "m".into());
        let (tx, mut rx) = mpsc::channel(16);
        metered.generate(req(), tx).await.expect("generate");
        while rx.recv().await.is_some() {}

        let rows = store.summary_since(0).await.expect("summary");
        assert_eq!(rows.len(), 1, "done:false 上的 usage 也必須記到");
        assert_eq!(rows[0].prompt_tokens, 9);
    }
}
