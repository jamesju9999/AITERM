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
