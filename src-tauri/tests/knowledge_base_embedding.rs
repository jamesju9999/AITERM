use aiterm_lib::config::types::ProviderType;
use aiterm_lib::knowledge_base::embedding::{Embedder, EmbedderConfig, HttpEmbedder, list_models};
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
    }).expect("client build ok");

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
    }).expect("client build ok");

    let result = embedder.embed(&["a".into(), "b".into()]).await.expect("embed ok");
    // index 0 → first, index 1 → second (regardless of server response order)
    assert_eq!(result, vec![vec![0.1, 0.1], vec![0.9, 0.9]]);
}

#[tokio::test]
async fn openai_compatible_embed_tolerates_missing_index_field() {
    let server = MockServer::start().await;

    // Some self-hosted OpenAI-compatible servers (e.g. local MLX/Qwen gateways)
    // omit `index` and just rely on array order.
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"embedding": [0.1, 0.1]},
                {"embedding": [0.9, 0.9]}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::OpenaiCompatible,
        base_url: server.uri(),
        api_key: None,
        model: "local-embed".into(),
    }).expect("client build ok");

    let result = embedder.embed(&["a".into(), "b".into()]).await
        .expect("missing index field must not fail the whole embed call");
    assert_eq!(result, vec![vec![0.1, 0.1], vec![0.9, 0.9]]);
}

#[tokio::test]
async fn openai_compatible_embed_parse_error_includes_response_body() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"unexpected":"shape"}"#))
        .expect(1)
        .mount(&server)
        .await;

    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::OpenaiCompatible,
        base_url: server.uri(),
        api_key: None,
        model: "local-embed".into(),
    }).expect("client build ok");

    let err = embedder.embed(&["a".into()]).await.unwrap_err();
    assert!(
        err.contains("unexpected"),
        "parse error should include a snippet of the raw response body for diagnosis: {err}"
    );
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
    }).expect("client build ok");

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
    }).expect("client build ok");

    let err = embedder.embed(&["x".into()]).await.unwrap_err();
    assert!(err.contains("Anthropic"), "error should name the unsupported provider: {err}");
}

#[tokio::test]
async fn openai_compatible_list_models_returns_ids() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"id": "nomic-embed-text"},
                {"id": "Qwen3.6-35B-A3B-4bit"}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::OpenaiCompatible,
        base_url: server.uri(),
        api_key: Some("test-key".into()),
        model: "unused".into(),
    };

    let models = list_models(&cfg).await.expect("list ok");
    assert_eq!(models, vec!["nomic-embed-text", "Qwen3.6-35B-A3B-4bit"]);
}

#[tokio::test]
async fn list_models_errors_when_endpoint_missing() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::Openai,
        base_url: server.uri(),
        api_key: None,
        model: "unused".into(),
    };

    let err = list_models(&cfg).await.expect_err("404 should error");
    assert!(err.contains("404"), "error should mention the status: {err}");
}

#[tokio::test]
async fn ollama_list_models_reads_tags_endpoint() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "models": [{"name": "nomic-embed-text"}, {"name": "llama3.1:8b"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::Ollama,
        base_url: server.uri(),
        api_key: None,
        model: "unused".into(),
    };

    let models = list_models(&cfg).await.expect("list ok");
    assert_eq!(models, vec!["nomic-embed-text", "llama3.1:8b"]);
}

#[tokio::test]
async fn list_models_rejects_provider_without_embedding_support() {
    let cfg = EmbedderConfig {
        provider_type: ProviderType::Anthropic,
        base_url: "http://unused".into(),
        api_key: None,
        model: "unused".into(),
    };

    let err = list_models(&cfg).await.expect_err("anthropic has no embedding API");
    assert!(err.contains("不支援 embedding"), "unexpected message: {err}");
}

#[tokio::test]
async fn ollama_list_models_errors_on_http_failure() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::Ollama,
        base_url: server.uri(),
        api_key: None,
        model: "unused".into(),
    };

    let err = list_models(&cfg).await.expect_err("500 should error");
    assert!(err.contains("500"), "error should mention the status: {err}");
}

#[tokio::test]
async fn list_models_reports_parse_error_on_unexpected_shape() {
    let server = MockServer::start().await;

    // A real self-hosted gateway can answer 200 with no `data` key at all.
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::OpenaiCompatible,
        base_url: server.uri(),
        api_key: None,
        model: "unused".into(),
    };

    let err = list_models(&cfg).await.expect_err("missing `data` should error");
    assert!(err.contains("parse"), "error should name the failure mode: {err}");
}

#[tokio::test]
async fn list_models_without_api_key_sends_no_authorization_header() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("accept", "application/json"))
        .and(|req: &wiremock::Request| !req.headers.contains_key("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "local-model"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::OpenaiCompatible,
        base_url: server.uri(),
        api_key: None,
        model: "unused".into(),
    };

    let models = list_models(&cfg).await.expect("list ok");
    assert_eq!(models, vec!["local-model"]);
}

#[tokio::test]
async fn list_models_returns_empty_vec_when_provider_has_no_models() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::Openai,
        base_url: server.uri(),
        api_key: None,
        model: "unused".into(),
    };

    // 空清單不是錯誤——前端 datalist 靠這個契約顯示「這個 provider 沒有模型」。
    let models = list_models(&cfg).await.expect("empty list is not an error");
    assert!(models.is_empty(), "expected no models, got: {models:?}");
}
