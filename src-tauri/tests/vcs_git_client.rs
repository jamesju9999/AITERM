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
                "head": {"ref": "feature/login-optimize"},
                "base": {"ref": "main"}
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
    assert_eq!(features[0].base_ref, "main");
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
                "head": {"ref": "feature/login-optimize"},
                "base": {"ref": "main"}
            },
            {
                "number": 8,
                "title": "支付流程重構",
                "user": {"login": "bob"},
                "draft": false,
                "html_url": "https://github.com/acme/widget/pull/8",
                "updated_at": "2026-08-17T01:00:00Z",
                "head": {"ref": "feature/payment-refactor"},
                "base": {"ref": "main"}
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
    assert_eq!(features[0].base_ref, "main");
    assert_eq!(features[0].files, vec!["src/Login.tsx", "src/api/auth.ts"]);
    assert_eq!(features[1].number, 8);
    assert_eq!(features[1].base_ref, "main");
    assert!(features[1].files.is_empty());
}

#[tokio::test]
async fn get_default_branch_extracts_default_branch_from_repo_response() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widget"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 123456,
            "name": "widget",
            "full_name": "acme/widget",
            "private": false,
            "default_branch": "master"
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

    let branch = client.get_default_branch().await.expect("should succeed");
    assert_eq!(branch, "master");
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

#[tokio::test]
async fn delete_remote_branch_sends_delete_to_the_correct_github_api_path() {
    let server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/repos/acme/widget/git/refs/heads/feature/login-optimize"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(204))
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

    client
        .delete_remote_branch("feature/login-optimize")
        .await
        .expect("should succeed");
}

#[tokio::test]
async fn pr_diff_requests_diff_media_type_and_returns_raw_text() {
    let server = MockServer::start().await;

    let diff_body = "diff --git a/src/Login.tsx b/src/Login.tsx\n+added line\n";
    Mock::given(method("GET"))
        .and(path("/repos/acme/widget/compare/main...feature/login-optimize"))
        .and(header("accept", "application/vnd.github.v3.diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string(diff_body))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;
    let client = GitClient::new_with_api_base(
        dir.path().to_string_lossy().to_string(),
        Some("test-token".to_string()),
        server.uri(),
    );

    let result = client.pr_diff("main", "feature/login-optimize").await.expect("should succeed");
    match result {
        aiterm_lib::vcs::VcsResult::Diff { content, .. } => assert_eq!(content, diff_body),
        other => panic!("expected Diff variant, got {other:?}"),
    }
}

#[tokio::test]
async fn merge_pr_sends_put_to_the_correct_merge_endpoint() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/repos/acme/widget/pulls/7/merge"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "sha": "abc123",
            "merged": true,
            "message": "Pull Request successfully merged"
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

    client.merge_pr(7).await.expect("should succeed");
}

#[tokio::test]
async fn merge_pr_surfaces_not_mergeable_error() {
    let server = MockServer::start().await;

    Mock::given(method("PUT"))
        .and(path("/repos/acme/widget/pulls/7/merge"))
        .respond_with(ResponseTemplate::new(405).set_body_json(serde_json::json!({
            "message": "Pull Request is not mergeable"
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

    let err = client.merge_pr(7).await.unwrap_err();
    assert!(err.contains("not mergeable"), "unexpected error: {err}");
}

/// Sets up two real git repos: a bare "remote" and a normal working repo
/// whose `origin` points at the bare repo's filesystem path. This lets tests
/// exercise a genuine `git push` without any network dependency — wiremock
/// only intercepts HTTP calls, not raw git subprocess I/O.
async fn init_repo_with_local_remote(dir: &std::path::Path) -> std::path::PathBuf {
    let bare_dir = dir.join("origin.git");
    std::process::Command::new("git")
        .args(["init", "--bare", "-q"])
        .arg(&bare_dir)
        .status()
        .unwrap();

    let work_dir = dir.join("work");
    std::fs::create_dir(&work_dir).unwrap();
    std::process::Command::new("git").args(["init", "-q"]).current_dir(&work_dir).status().unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&work_dir).status().unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&work_dir).status().unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin"])
        .arg(&bare_dir)
        .current_dir(&work_dir).status().unwrap();

    // Need at least one commit on the initial branch before it can be pushed
    // and before a feature branch can meaningfully be created "from" it.
    std::fs::write(work_dir.join("README.md"), "init").unwrap();
    std::process::Command::new("git").args(["add", "."]).current_dir(&work_dir).status().unwrap();
    std::process::Command::new("git").args(["commit", "-q", "-m", "init"]).current_dir(&work_dir).status().unwrap();
    // Ensure the default branch is named "main" regardless of the test machine's git config.
    std::process::Command::new("git").args(["branch", "-M", "main"]).current_dir(&work_dir).status().unwrap();
    std::process::Command::new("git").args(["push", "-q", "origin", "main"]).current_dir(&work_dir).status().unwrap();

    work_dir
}

#[tokio::test]
async fn push_branch_pushes_the_current_branch_to_origin_with_upstream() {
    let dir = tempfile::tempdir().unwrap();
    let work_dir = init_repo_with_local_remote(dir.path()).await;

    let client = GitClient::new(work_dir.to_string_lossy().to_string(), None);
    client.create_branch("feature/test-push", Some("main")).await.expect("create_branch should succeed");

    client.push_branch("feature/test-push").await.expect("push_branch should succeed");

    // Verify the branch genuinely exists on the "remote" (the bare repo) now.
    let bare_dir = dir.path().join("origin.git");
    let out = std::process::Command::new("git")
        .args(["branch", "--list", "feature/test-push"])
        .current_dir(&bare_dir)
        .output()
        .unwrap();
    let branches = String::from_utf8_lossy(&out.stdout);
    assert!(branches.contains("feature/test-push"), "expected branch to exist on remote, got: {branches}");
}

#[tokio::test]
async fn commit_empty_creates_a_commit_with_no_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let work_dir = init_repo_with_local_remote(dir.path()).await;

    let client = GitClient::new(work_dir.to_string_lossy().to_string(), None);
    client.create_branch("feature/test-commit", Some("main")).await.expect("create_branch should succeed");

    client.commit_empty("Start feature: test").await.expect("commit_empty should succeed");

    // Verify there is now a commit on this branch that main doesn't have.
    let out = std::process::Command::new("git")
        .args(["rev-list", "--count", "main..feature/test-commit"])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    let count: i32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    assert_eq!(count, 1, "expected exactly one new commit ahead of main");

    // And that commit genuinely changes no files.
    let diff_out = std::process::Command::new("git")
        .args(["diff", "--stat", "main..feature/test-commit"])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&diff_out.stdout).trim().is_empty(), "empty commit should produce an empty diff");
}

/// Exercises the same create_branch -> commit_empty -> (failed push) ->
/// rollback sequence that `vcs_start_feature` performs when `push_branch`
/// fails, proving the cleanup (`checkout_branch` back to base +
/// `delete_branch_force`) genuinely undoes the local state left behind by a
/// failed push. `vcs_start_feature` itself is a Tauri command that needs
/// `State` args and can't be unit tested directly, so this tests the same
/// sequence at the `GitClient` level instead, consistent with how this file
/// already handles Tauri-command testability elsewhere.
#[tokio::test]
async fn failed_push_can_be_rolled_back_via_checkout_and_force_delete() {
    let dir = tempfile::tempdir().unwrap();
    let work_dir = dir.path().join("work");
    std::fs::create_dir(&work_dir).unwrap();
    std::process::Command::new("git").args(["init", "-q"]).current_dir(&work_dir).status().unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&work_dir).status().unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&work_dir).status().unwrap();

    // Point origin at a path with no repo there at all, so any push fails
    // deterministically without any network dependency.
    let missing_remote = dir.path().join("does-not-exist.git");
    std::process::Command::new("git")
        .args(["remote", "add", "origin"])
        .arg(&missing_remote)
        .current_dir(&work_dir).status().unwrap();

    std::fs::write(work_dir.join("README.md"), "init").unwrap();
    std::process::Command::new("git").args(["add", "."]).current_dir(&work_dir).status().unwrap();
    std::process::Command::new("git").args(["commit", "-q", "-m", "init"]).current_dir(&work_dir).status().unwrap();
    std::process::Command::new("git").args(["branch", "-M", "main"]).current_dir(&work_dir).status().unwrap();

    let client = GitClient::new(work_dir.to_string_lossy().to_string(), None);
    client.create_branch("feature/test-rollback", Some("main")).await.expect("create_branch should succeed");
    client.commit_empty("Start feature: test").await.expect("commit_empty should succeed");

    let push_result = client.push_branch("feature/test-rollback").await;
    assert!(push_result.is_err(), "push should fail because origin doesn't exist");

    // This mirrors the rollback vcs_start_feature performs on push failure.
    client.checkout_branch("main").await.expect("checkout back to base should succeed");
    client.delete_branch_force("feature/test-rollback").await.expect("force delete should succeed");

    let branch_out = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&branch_out.stdout).trim(), "main", "should be back on base branch");

    let list_out = std::process::Command::new("git")
        .args(["branch", "--list", "feature/test-rollback"])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&list_out.stdout).trim().is_empty(),
        "feature branch should have been deleted locally"
    );
}

#[tokio::test]
async fn create_branch_from_a_freshly_fetched_remote_ref_picks_up_new_remote_commits_not_a_stale_local_branch() {
    let dir = tempfile::tempdir().unwrap();
    let work_dir = init_repo_with_local_remote(dir.path()).await;
    let bare_dir = dir.path().join("origin.git");

    // Simulate a second contributor pushing a new commit to origin/main that
    // this local clone doesn't know about yet (its local `main` is stale).
    let other_clone = dir.path().join("other-clone");
    std::process::Command::new("git")
        .args(["clone", "-q"]).arg(&bare_dir).arg(&other_clone)
        .status().unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "other@test.com"]).current_dir(&other_clone).status().unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Other"]).current_dir(&other_clone).status().unwrap();
    std::fs::write(other_clone.join("new-file.txt"), "new content").unwrap();
    std::process::Command::new("git")
        .args(["add", "."]).current_dir(&other_clone).status().unwrap();
    std::process::Command::new("git")
        .args(["commit", "-q", "-m", "second contributor's commit"]).current_dir(&other_clone).status().unwrap();
    std::process::Command::new("git")
        .args(["push", "-q", "origin", "main"]).current_dir(&other_clone).status().unwrap();

    // The original working repo's local `main` is now stale (doesn't have
    // "second contributor's commit"). Fetch + branch from origin/main should
    // still pick it up correctly.
    let client = GitClient::new(work_dir.to_string_lossy().to_string(), None);
    client.fetch_ref("main").await.expect("fetch_ref should succeed");
    client.create_branch("feature/from-fresh-remote", Some("origin/main")).await.expect("create_branch should succeed");

    // The new branch must contain the second contributor's commit, which the
    // local (stale) `main` never had.
    let out = std::process::Command::new("git")
        .args(["log", "--oneline", "feature/from-fresh-remote"])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stdout);
    assert!(log.contains("second contributor's commit"), "expected the new branch to include the remote's latest commit, got log: {log}");
}
