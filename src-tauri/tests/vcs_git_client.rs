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
