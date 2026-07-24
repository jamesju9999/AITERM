//! Contract test for `list_openai_style_models` — the shared model-list
//! fetcher used by the OpenRouter/xAI/DeepSeek/Kimi provider commands.
//! All four providers speak the same OpenAI-shaped `{"data":[{"id":...}]}`
//! `/models` response, so this is tested once against a wiremock fake rather
//! than per-provider.

use aiterm_lib::commands::provider::list_openai_style_models;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn parses_openai_shaped_model_list() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"data":[{"id":"model-a"},{"id":"model-b"}]}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let models = list_openai_style_models(&server.uri(), "test-key")
        .await
        .unwrap();
    assert_eq!(models, vec!["model-a".to_string(), "model-b".to_string()]);
}

#[tokio::test]
async fn empty_api_key_errors_without_making_a_request() {
    let server = MockServer::start().await;
    // Deliberately no Mock registered: if the function made an HTTP request
    // despite the empty key, wiremock would return its default 404 and the
    // error message would say "404" instead of "api_key is required",
    // failing the assertion below.
    let err = list_openai_style_models(&server.uri(), "   ")
        .await
        .unwrap_err();
    assert!(err.contains("api_key is required"), "got: {err}");
}

#[tokio::test]
async fn non_success_status_returns_error_with_status_and_body() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let err = list_openai_style_models(&server.uri(), "bad-key")
        .await
        .unwrap_err();
    assert!(err.contains("401"), "got: {err}");
    assert!(err.contains("unauthorized"), "got: {err}");
}

#[tokio::test]
async fn trailing_slash_on_base_url_is_handled() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[{"id":"m1"}]}"#))
        .mount(&server)
        .await;

    let base_with_slash = format!("{}/", server.uri());
    let models = list_openai_style_models(&base_with_slash, "k")
        .await
        .unwrap();
    assert_eq!(models, vec!["m1".to_string()]);
}
