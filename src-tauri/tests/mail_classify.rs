// src-tauri/tests/mail_classify.rs
use aiterm_lib::ai::{AiError, AiProvider, GenerateChunk, GenerateRequest};
use aiterm_lib::mail::classify::{classify_message, build_mail_classify_prompt};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

struct MockProvider {
    chunks: Vec<&'static str>,
}

#[async_trait]
impl AiProvider for MockProvider {
    fn id(&self) -> &str { "mock" }
    fn display_name(&self) -> &str { "Mock" }

    async fn generate(
        &self,
        _req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        for (i, c) in self.chunks.iter().enumerate() {
            let done = i + 1 == self.chunks.len();
            let _ = tx.send(GenerateChunk { delta: c.to_string(), done, usage: None }).await;
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), AiError> { Ok(()) }
}

#[tokio::test]
async fn parses_valid_classification_json() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec![r#"{"summary":"老闆問你今天能不能開會","is_important":true,"is_promotional":false}"#],
    });
    let result = classify_message(provider, "boss@example.com", "Quick meeting?", "Can we meet today?")
        .await
        .expect("classify ok");
    assert_eq!(result.summary, "老闆問你今天能不能開會");
    assert!(result.is_important);
    assert!(!result.is_promotional);
}

#[tokio::test]
async fn strips_markdown_fence_before_parsing() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec!["```json\n", r#"{"summary":"週年慶特賣","is_important":false,"is_promotional":true}"#, "\n```"],
    });
    let result = classify_message(provider, "deals@shop.com", "50% off everything!", "Sale sale sale")
        .await
        .expect("classify ok");
    assert!(result.is_promotional);
    assert!(!result.is_important);
}

#[tokio::test]
async fn missing_fields_default_to_false_and_empty_summary() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec![r#"{}"#],
    });
    let result = classify_message(provider, "a@b.com", "subj", "body").await.expect("classify ok");
    assert_eq!(result.summary, "");
    assert!(!result.is_important);
    assert!(!result.is_promotional);
}

#[tokio::test]
async fn malformed_json_is_an_error_not_a_panic() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec!["not json at all"],
    });
    let result = classify_message(provider, "a@b.com", "subj", "body").await;
    assert!(result.is_err());
}

#[test]
fn prompt_is_deterministic_and_forbids_dual_classification() {
    let a = build_mail_classify_prompt();
    let b = build_mail_classify_prompt();
    assert_eq!(a, b);
    assert!(a.contains("is_important"));
    assert!(a.contains("is_promotional"));
}
