//! 觀看端客戶端的端到端測試：用自己的 client 連自己的 server。
//!
//! 這比 mock 有價值得多——它同時驗證兩端的協定實作真的對得起來，而不是
//! 各自符合我對協定的想像。

use std::sync::Arc;
use std::time::Duration;

use aiterm_lib::pty::manager::PtyManager;
use aiterm_lib::share::registry::AccessMode;
use aiterm_lib::share::viewer::{connect_and_handshake, ViewerHandshake};
use aiterm_lib::share::ShareServerState;
use portable_pty::PtySize;

const SIZE: PtySize = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };

/// 起一個真的分享 server，回傳 (state, tab_id, code, port)。
async fn start_host() -> (Arc<ShareServerState>, String, String, u16) {
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let state = Arc::new(ShareServerState::new());
    let code = state.registry.start_share(tab_id.clone());
    let port = state
        .start_if_needed(Arc::clone(&pty), None)
        .await
        .expect("start share server");
    (state, tab_id, code, port)
}

#[tokio::test]
async fn the_viewer_derives_the_same_sas_as_the_host() {
    // 這是人工核對能成立的前提：兩端各自從自己那條 TLS 連線導出，結果一致。
    let (state, tab_id, code, port) = start_host().await;

    let ViewerHandshake { sas, .. } =
        connect_and_handshake("127.0.0.1", port, &code, "Alice")
            .await
            .expect("handshake should succeed");

    assert_eq!(sas.len(), 4, "SAS should be four digits, got {sas:?}");

    let pending = state.registry.pending(&tab_id);
    assert_eq!(pending.len(), 1, "the host should see exactly one pending request");
    assert_eq!(
        pending[0].sas, sas,
        "both ends of the same TLS connection must derive the same SAS"
    );
}

#[tokio::test]
async fn an_unknown_code_is_refused_before_any_handshake_completes() {
    let (_state, _tab_id, code, port) = start_host().await;

    // 從真短碼推一個保證不同的出來——短碼是亂數，不能寫死一個「一定不存在」的值。
    let bogus: String = code
        .chars()
        .map(|c| if c == '0' { '1' } else { '0' })
        .collect();
    assert_ne!(bogus, code);

    let result = connect_and_handshake("127.0.0.1", port, &bogus, "Mallory").await;
    assert!(result.is_err(), "an unknown code must not produce a usable connection");
}

#[tokio::test]
async fn a_tampered_commit_aborts_the_handshake() {
    // 這是承諾流程的整個價值所在：主控端揭曉的 nonce 必須跟先前的承諾對得上。
    // 對不上就是中間人的跡象，客戶端要中止而不是繼續。
    //
    // 這裡直接測驗證函式而不是架一個假 server——重點是「驗證邏輯會不會放行
    // 一個不匹配的 nonce」，架 server 只會讓測試變慢變脆。
    use aiterm_lib::share::tls::commit_for;
    use aiterm_lib::share::viewer::commit_matches;

    let real = [0xAAu8; 32];
    let other = [0xBBu8; 32];

    assert!(commit_matches(&commit_for(&real), &real));
    assert!(
        !commit_matches(&commit_for(&other), &real),
        "a commit from a different nonce must not validate"
    );
    assert!(
        !commit_matches("not-hex-at-all", &real),
        "a malformed commit must not validate"
    );
}

#[tokio::test]
async fn an_approved_viewer_receives_the_hosts_screen() {
    use aiterm_lib::share::viewer::{run_viewer_stream, ViewerEvent};

    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let state = Arc::new(ShareServerState::new());
    let code = state.registry.start_share(tab_id.clone());
    let port = state
        .start_if_needed(Arc::clone(&pty), None)
        .await
        .expect("start share server");

    // 分享前先在分頁裡留下歷史，之後要驗證它有被重播。
    #[cfg(windows)]
    pty.write(&tab_id, b"echo VIEWED\r\n").unwrap();
    #[cfg(not(windows))]
    pty.write(&tab_id, b"printf 'VIEWED\\n'\n").unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let handshake = connect_and_handshake("127.0.0.1", port, &code, "Alice")
        .await
        .expect("handshake");

    // 主控端核准（模擬使用者按下「只能看」）。
    let request_id = state.registry.pending(&tab_id)[0].request_id.clone();
    state
        .registry
        .approve(&request_id, AccessMode::ReadOnly)
        .expect("approve");

    // 串流迴圈把事件送進 channel，測試在這裡收。
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ViewerEvent>();
    let (_keys_tx, keys_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(run_viewer_stream(handshake.ws, tx, keys_rx));

    // 先收到 Granted，然後是重播。
    let mut granted = false;
    let mut screen = Vec::new();
    for _ in 0..60 {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(ViewerEvent::Granted { cols, rows, .. })) => {
                assert_eq!((cols, rows), (80, 24), "viewer must use the host's size");
                granted = true;
            }
            Ok(Some(ViewerEvent::Data(bytes))) => {
                screen.extend_from_slice(&bytes);
                if screen.windows(6).any(|w| w == b"VIEWED") {
                    break;
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(_) => continue,
        }
    }

    assert!(granted, "never received Granted");
    assert!(
        screen.windows(6).any(|w| w == b"VIEWED"),
        "the replay did not contain the history the host had before we connected"
    );
}

#[tokio::test]
async fn the_viewer_is_told_why_the_connection_ended() {
    use aiterm_lib::share::viewer::{run_viewer_stream, ViewerEvent};

    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let state = Arc::new(ShareServerState::new());
    let code = state.registry.start_share(tab_id.clone());
    let port = state.start_if_needed(Arc::clone(&pty), None).await.expect("start");

    let handshake = connect_and_handshake("127.0.0.1", port, &code, "Alice")
        .await
        .expect("handshake");
    let request_id = state.registry.pending(&tab_id)[0].request_id.clone();
    state.registry.approve(&request_id, AccessMode::ReadOnly).expect("approve");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ViewerEvent>();
    let (_keys_tx, keys_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(run_viewer_stream(handshake.ws, tx, keys_rx));

    // 主控端停止分享——觀看端要收到有意義的原因，不是無聲斷線。
    tokio::time::sleep(Duration::from_millis(300)).await;
    state.registry.stop_share(&tab_id);

    let mut reason = None;
    for _ in 0..60 {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(ViewerEvent::Ended { reason: r })) => {
                reason = Some(r);
                break;
            }
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(_) => continue,
        }
    }

    assert_eq!(
        reason.as_deref(),
        Some("host_stopped_sharing"),
        "the viewer must learn why, not just see the socket close"
    );
}
