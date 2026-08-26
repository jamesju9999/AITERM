//! 共享連線的端到端測試：連線 → 待審 → 核准 → 重播 → 即時串流 →
//! 輸入授權 → 結束。
//!
//! 用真的 PTY（跑一個 shell）與真的 ws 連線，不是 mock——這條路徑的價值
//! 全在「真的接得起來」，用假的 PTY 測等於什麼都沒測。

use std::sync::Arc;
use std::time::Duration;

use aiterm_lib::pty::manager::PtyManager;
use aiterm_lib::share::registry::{AccessMode, ShareRegistry};
use aiterm_lib::share::protocol::{ClientMessage, EndReason, ServerMessage, WireAccessMode};
use futures_util::{SinkExt, StreamExt};
use portable_pty::PtySize;
use tokio_tungstenite::tungstenite::Message;

const SIZE: PtySize = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };

/// 起一個綁 127.0.0.1:0 的共享 server，回傳它的實際 port。
///
/// 這條路徑刻意**不走 TLS**——這幾個測試驗的是握手、授權與串流邏輯，TLS 與
/// SAS 由 Task 8 的 `both_ends_of_a_real_tls_connection_derive_the_same_sas`
/// 單獨驗。正式路徑的 `ConnectionSas` 由 TLS accept 迴圈注入，這裡手動補一個
/// 佔位值；**不要**把 handler 的 extractor 改成 `Option`，那等於讓正式路徑
/// 在 TLS 沒接上時無聲降級成沒有身分保證的連線。
async fn start_test_server(
    pty: Arc<PtyManager>,
    registry: Arc<ShareRegistry>,
) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = aiterm_lib::share::server::router(pty, registry)
        .layer(axum::Extension(aiterm_lib::share::protocol::ConnectionSas(
            "0000".to_string(),
        )));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
}

/// 讀到下一則 JSON 控制訊息，跳過中間的二進位 frame。
async fn next_control<S>(ws: &mut S) -> ServerMessage
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    for _ in 0..200 {
        match tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                return serde_json::from_str(&t).expect("server sent malformed JSON")
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => panic!("ws error: {e}"),
            Ok(None) => panic!("ws closed while waiting for a control message"),
            Err(_) => continue,
        }
    }
    panic!("timed out waiting for a control message");
}

/// 累積二進位 frame 直到看到 `marker`，或逾時。
async fn collect_binary_until<S>(ws: &mut S, marker: &[u8]) -> Vec<u8>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut acc = Vec::new();
    for _ in 0..200 {
        match tokio::time::timeout(Duration::from_millis(200), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                acc.extend_from_slice(&b);
                if acc.windows(marker.len()).any(|w| w == marker) {
                    return acc;
                }
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(e))) => panic!("ws error: {e}"),
            Ok(None) => break,
            Err(_) => continue,
        }
    }
    acc
}

#[tokio::test]
async fn an_unknown_code_is_refused_without_reaching_the_host() {
    let pty = Arc::new(PtyManager::new());
    let registry = Arc::new(ShareRegistry::new());
    let port = start_test_server(pty, Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            protocol_version: aiterm_lib::share::protocol::PROTOCOL_VERSION,
            code: "000000".to_string(),
            display_name: "Mallory".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Ended { reason: EndReason::InvalidCode }
    );
}

#[tokio::test]
async fn a_read_only_viewer_sees_output_but_cannot_type() {
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    // 分享前先在分頁裡留下一點歷史，之後要驗證它有被重播。
    #[cfg(windows)]
    pty.write(&tab_id, b"echo BEFORE\r\n").unwrap();
    #[cfg(not(windows))]
    pty.write(&tab_id, b"printf 'BEFORE\\n'\n").unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            protocol_version: aiterm_lib::share::protocol::PROTOCOL_VERSION,
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    assert_eq!(next_control(&mut ws).await, ServerMessage::AwaitingApproval);

    // 模擬主控端按下「只能看」。
    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    let viewer_id = registry.approve(&request_id, AccessMode::ReadOnly).expect("approve");

    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Granted { mode: WireAccessMode::ReadOnly, cols: 80, rows: 24 }
    );

    // 重播必須包含分享前就存在的歷史。
    let replayed = collect_binary_until(&mut ws, b"BEFORE").await;
    assert!(
        replayed.windows(6).any(|w| w == b"BEFORE"),
        "expected the pre-share history to be replayed"
    );

    // 唯讀端送輸入必須不會進到 PTY。
    ws.send(Message::Binary(b"echo HACKED\n".to_vec().into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let seen = pty.get_recent_output(&tab_id, 64 * 1024).unwrap_or_default();
    assert!(
        !seen.contains("HACKED"),
        "a read-only viewer's input reached the PTY: {seen}"
    );
    assert!(!registry.may_send_input(&tab_id, &viewer_id));
}

#[tokio::test]
async fn a_controlling_viewer_can_type_and_sees_the_result() {
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            protocol_version: aiterm_lib::share::protocol::PROTOCOL_VERSION,
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_control(&mut ws).await, ServerMessage::AwaitingApproval);

    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    registry.approve(&request_id, AccessMode::Control).expect("approve");
    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Granted { mode: WireAccessMode::Control, cols: 80, rows: 24 }
    );

    // 遠端打字，PTY 應該真的收到並回顯。
    #[cfg(windows)]
    ws.send(Message::Binary(b"echo REMOTE\r\n".to_vec().into())).await.unwrap();
    #[cfg(not(windows))]
    ws.send(Message::Binary(b"printf 'REMOTE\\n'\n".to_vec().into())).await.unwrap();

    let echoed = collect_binary_until(&mut ws, b"REMOTE").await;
    assert!(
        echoed.windows(6).any(|w| w == b"REMOTE"),
        "the controlling viewer's input never came back as output"
    );
}

#[tokio::test]
async fn a_mismatched_protocol_version_is_refused_at_the_handshake() {
    // 「同事的 AITerm 沒更新」在區網分享裡是常態。沒有這道檢查的話，症狀會是
    // 後續某則訊息解析失敗、連線莫名其妙斷掉，使用者完全無從得知原因。
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            protocol_version: aiterm_lib::share::protocol::PROTOCOL_VERSION + 1,
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Ended { reason: EndReason::VersionMismatch }
    );
    // 版本不符時不該留下待審請求打擾主控端。
    assert_eq!(registry.pending(&tab_id).len(), 0);
}

#[tokio::test]
async fn revoking_control_tells_the_viewer_it_can_no_longer_type() {
    // 唯讀的人不知道自己被降級，就會白打一堆字進黑洞。
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            protocol_version: aiterm_lib::share::protocol::PROTOCOL_VERSION,
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_control(&mut ws).await, ServerMessage::AwaitingApproval);

    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    let viewer_id = registry.approve(&request_id, AccessMode::Control).expect("approve");
    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Granted { mode: WireAccessMode::Control, cols: 80, rows: 24 }
    );

    // 主控端收回控制權——觀看端要在下一次輪詢時被告知。
    registry.revoke_control(&tab_id);
    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::ControlChanged { mode: WireAccessMode::ReadOnly }
    );
    assert!(!registry.may_send_input(&tab_id, &viewer_id));
}

#[tokio::test]
async fn a_viewer_that_gives_up_waiting_stops_pestering_the_host() {
    // 觀看端在等待同意期間關掉連線（等不下去了）。那筆待審請求必須跟著消失
    // ——否則主控端的同意視窗上會掛著一個永遠不會有下文的「OOO 想連進來」，
    // 而 server 這邊還有一個永遠空轉的 task。
    //
    // 這條路徑刻意**不**能靠逾時解決：逾時會讓「使用者只是走開了」變成自動
    // 拒絕，那是 spec 明確排除的行為。要偵測的是連線本身斷了。
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            protocol_version: aiterm_lib::share::protocol::PROTOCOL_VERSION,
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_control(&mut ws).await, ServerMessage::AwaitingApproval);
    assert_eq!(
        registry.pending(&tab_id).len(),
        1,
        "the host should see the request while the viewer is still waiting"
    );

    // 觀看端放棄，直接關掉連線。
    drop(ws);

    // server 端要在幾個輪詢間隔內注意到並收掉那筆請求。
    let mut cleared = false;
    for _ in 0..50 {
        if registry.pending(&tab_id).is_empty() {
            cleared = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        cleared,
        "a viewer that closed its connection left a pending request behind: {:?}",
        registry.pending(&tab_id)
    );
}

#[tokio::test]
async fn stopping_the_share_ends_the_connection_with_a_reason() {
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            protocol_version: aiterm_lib::share::protocol::PROTOCOL_VERSION,
            code: code.clone(),
            display_name: "Alice".to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_control(&mut ws).await, ServerMessage::AwaitingApproval);
    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    registry.approve(&request_id, AccessMode::ReadOnly).expect("approve");
    let _ = next_control(&mut ws).await; // Granted

    registry.stop_share(&tab_id);

    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Ended { reason: EndReason::HostStoppedSharing }
    );
}
