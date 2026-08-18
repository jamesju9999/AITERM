use aiterm_lib::vcs::git::GitClient;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn create_pr_sends_draft_flag_and_returns_number_and_url() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/repos/acme/widget/pulls"))
        .and(header("authorization", "Bearer test-token"))
        .and(body_json(serde_json::json!({
            "title": "登入頁優化",
            "head": "feature/login-optimize",
            "base": "main",
            "body": "",
            "draft": true,
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 42,
            "html_url": "https://github.com/acme/widget/pull/42",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .args(["init", "-q"]).current_dir(dir.path()).status().unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/acme/widget.git"])
        .current_dir(dir.path()).status().unwrap();

    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let (number, url) = client
        .create_pr("登入頁優化", "feature/login-optimize", "main", None, true)
        .await
        .expect("create_pr should succeed");

    assert_eq!(number, 42);
    assert_eq!(url, "https://github.com/acme/widget/pull/42");
}

async fn init_repo_with_origin(dir: &std::path::Path) {
    std::process::Command::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/acme/widget.git"])
        .current_dir(dir).status().unwrap();
}

#[tokio::test]
async fn list_active_features_combines_pr_list_and_per_pr_files() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 7,
                "title": "登入頁優化",
                "user": {"login": "alice"},
                "draft": true,
                "html_url": "https://github.com/acme/widget/pull/7",
                "updated_at": "2026-08-17T00:00:00Z",
                "head": {"ref": "feature/login-optimize"}
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls/7/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"filename": "src/Login.tsx"},
            {"filename": "src/api/auth.ts"}
        ])))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let features = client.list_active_features().await.expect("should succeed");

    assert_eq!(features.len(), 1);
    assert_eq!(features[0].number, 7);
    assert_eq!(features[0].author, "alice");
    assert!(features[0].draft);
    assert_eq!(features[0].head_ref, "feature/login-optimize");
    assert_eq!(features[0].files, vec!["src/Login.tsx", "src/api/auth.ts"]);
}

#[tokio::test]
async fn list_active_features_returns_empty_when_no_open_prs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let features = client.list_active_features().await.expect("should succeed");
    assert!(features.is_empty());
}

#[tokio::test]
async fn list_active_features_degrades_gracefully_when_one_pr_files_fetch_fails() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 7,
                "title": "登入頁優化",
                "user": {"login": "alice"},
                "draft": true,
                "html_url": "https://github.com/acme/widget/pull/7",
                "updated_at": "2026-08-17T00:00:00Z",
                "head": {"ref": "feature/login-optimize"}
            },
            {
                "number": 8,
                "title": "支付流程重構",
                "user": {"login": "bob"},
                "draft": false,
                "html_url": "https://github.com/acme/widget/pull/8",
                "updated_at": "2026-08-17T01:00:00Z",
                "head": {"ref": "feature/payment-refactor"}
            }
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls/7/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"filename": "src/Login.tsx"},
            {"filename": "src/api/auth.ts"}
        ])))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls/8/files"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let features = client.list_active_features().await.expect("should still succeed overall");

    assert_eq!(features.len(), 2);
    assert_eq!(features[0].number, 7);
    assert_eq!(features[0].files, vec!["src/Login.tsx", "src/api/auth.ts"]);
    assert_eq!(features[1].number, 8);
    assert!(features[1].files.is_empty());
}

#[tokio::test]
async fn mark_pr_ready_fetches_node_id_then_calls_graphql_mutation() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "node_id": "PR_kwABC123"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_json(serde_json::json!({
            "query": "mutation($id: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $id}) { pullRequest { id } } }",
            "variables": { "id": "PR_kwABC123" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "markPullRequestReadyForReview": { "pullRequest": { "id": "PR_kwABC123" } } }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    client.mark_pr_ready(7).await.expect("should succeed");
}

#[tokio::test]
async fn mark_pr_ready_surfaces_graphql_errors_even_though_http_status_is_200() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/pulls/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "node_id": "PR_kwABC123"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": null,
            "errors": [{"message": "Pull request is already ready for review"}]
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let err = client.mark_pr_ready(7).await.unwrap_err();
    assert!(err.contains("already ready for review"), "unexpected error: {err}");
}
