//! 配對的端到端測試：真的 TLS、真的互證。
use aiterm_core::share::auth;
use aiterm_intake_client::pair::{pair, pair_with_timeout};
use aiterm_intake_client::ClientError;
use aiterm_intake_core::api::invite;
use aiterm_intake_server::members::Role;
use aiterm_intake_server::testkit::TestServer;
use std::time::Duration;

#[tokio::test]
async fn a_valid_invite_pairs_once_and_yields_a_working_token() {
    let srv = TestServer::start().await;
    let code = srv.invite("小明", Role::Member).await;
    let paired = pair(&srv.addr(), &code).await.expect("配對成功");
    assert_eq!(paired.response.me.name, "小明");
    assert_eq!(paired.response.me.role, aiterm_intake_core::api::Role::Member);
    assert_eq!(paired.response.fingerprint, srv.state().fingerprint, "釘住的是 server 真的憑證");
    // 同一個邀請不能再用。
    assert_eq!(pair(&srv.addr(), &code).await.unwrap_err(), ClientError::PairingRefused);
}

#[tokio::test]
async fn a_wrong_secret_is_refused_and_burns_the_invite_after_repeated_guesses() {
    let srv = TestServer::start().await;
    let code = srv.invite("小明", Role::Member).await;
    let (id, _) = invite::parse_code(&code).unwrap();
    let wrong = invite::format_code(&id, "AAAAAAAAAAAAAAAA");
    assert_eq!(pair(&srv.addr(), &wrong).await.unwrap_err(), ClientError::PairingRefused);
    // 正確的碼在還沒被猜爆之前仍可用。
    assert!(pair(&srv.addr(), &code).await.is_ok());
}

#[tokio::test]
async fn an_unknown_invite_is_refused_like_a_wrong_one() {
    let srv = TestServer::start().await;
    let ghost = invite::format_code("ZZZZZZ", "AAAAAAAAAAAAAAAA");
    assert_eq!(pair(&srv.addr(), &ghost).await.unwrap_err(), ClientError::PairingRefused);
}

#[tokio::test]
async fn a_server_that_does_not_know_the_secret_is_not_trusted() {
    // 假 server：能完成 TLS，也回 200，但算不出正確的 host proof（它沒有邀請碼）。
    use aiterm_intake_server::tls;
    use axum::routing::post;
    let dir = tempfile::tempdir().unwrap();
    let identity = tls::load_or_generate(dir.path(), vec!["127.0.0.1".into()]).unwrap();
    let app = axum::Router::new().route(
        "/v1/pair",
        post(|| async {
            let body = serde_json::json!({"token":"x","me":{"id":"1","name":"n","role":"member"},"fingerprint":"00"});
            ([("x-aiterm-host-proof", "deadbeef")], axum::Json(body))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(tls::serve(listener, app, identity, rx));
    let code = invite::format_code("ABCDEF", "GHJKMNPQRSTUVWXY");
    let err = pair_with_timeout(&format!("127.0.0.1:{port}"), &code, Duration::from_secs(5)).await.unwrap_err();
    assert_eq!(err, ClientError::ServerNotProven, "沒有邀請碼就算不出證明，Client 必須拒絕");
}

#[tokio::test]
async fn a_proof_computed_on_another_connection_is_rejected() {
    // 模擬中間人：拿別條連線上算出的證明（exporter 不同）送給真的 server。
    let srv = TestServer::start().await;
    let code = srv.invite("小明", Role::Member).await;
    let (id, secret) = invite::parse_code(&code).unwrap();
    let key = invite::invite_key(&secret);
    let stale = auth::viewer_proof(&key, &[7u8; 32]);
    let verifier = aiterm_intake_client::pin::PinVerifier::new(None);
    let cfg = aiterm_intake_client::pin::client_config(verifier);
    let client = reqwest::Client::builder().use_preconfigured_tls(cfg).build().unwrap();
    let res = client
        .post(format!("https://{}/v1/pair", srv.addr()))
        .header("x-aiterm-proof", stale)
        .json(&serde_json::json!({ "invite_id": id }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn unauthenticated_requests_to_protected_routes_are_401_and_hello_is_public() {
    let srv = TestServer::start().await;
    let verifier = aiterm_intake_client::pin::PinVerifier::new(None);
    let client = reqwest::Client::builder().use_preconfigured_tls(aiterm_intake_client::pin::client_config(verifier)).build().unwrap();
    let base = format!("https://{}", srv.addr());
    assert_eq!(client.get(format!("{base}/v1/hello")).send().await.unwrap().status(), 200);
    assert_eq!(client.get(format!("{base}/v1/me")).send().await.unwrap().status(), 401);
    assert_eq!(client.get(format!("{base}/v1/me")).bearer_auth("0".repeat(64)).send().await.unwrap().status(), 401);
}
