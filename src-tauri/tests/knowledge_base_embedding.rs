use aiterm_lib::config::types::ProviderType;
use aiterm_lib::knowledge_base::embedding::{Embedder, EmbedderConfig, HttpEmbedder};
use wiremock::matchers::{method, path, header};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn ollama_embed_returns_vectors_in_order() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::Ollama,
        base_url: server.uri(),
        api_key: None,
        model: "nomic-embed-text".into(),
    });

    let result = embedder.embed(&["hello".into(), "world".into()]).await.expect("embed ok");
    assert_eq!(result, vec![vec![0.1, 0.2, 0.3], vec![0.4, 0.5, 0.6]]);
}

#[tokio::test]
async fn openai_compatible_embed_sorts_by_index_and_sends_bearer_token() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"embedding": [0.9, 0.9], "index": 1},
                {"embedding": [0.1, 0.1], "index": 0}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::Openai,
        base_url: server.uri(),
        api_key: Some("test-key".into()),
        model: "text-embedding-3-small".into(),
    });

    let result = embedder.embed(&["a".into(), "b".into()]).await.expect("embed ok");
    // index 0 → first, index 1 → second (regardless of server response order)
    assert_eq!(result, vec![vec![0.1, 0.1], vec![0.9, 0.9]]);
}

#[tokio::test]
async fn http_error_becomes_readable_error_message() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(500).set_body_string("model not found"))
        .mount(&server)
        .await;

    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::Ollama,
        base_url: server.uri(),
        api_key: None,
        model: "does-not-exist".into(),
    });

    let err = embedder.embed(&["x".into()]).await.unwrap_err();
    assert!(err.contains("500"), "error should mention status code: {err}");
}

#[tokio::test]
async fn unsupported_provider_type_returns_error_without_http_call() {
    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::Anthropic,
        base_url: "http://localhost:9".into(), // never actually called
        api_key: None,
        model: "claude".into(),
    });

    let err = embedder.embed(&["x".into()]).await.unwrap_err();
    assert!(err.contains("Anthropic"), "error should name the unsupported provider: {err}");
}
