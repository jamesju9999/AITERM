//! `aiterm-host` 的端到端測試：真的起一個帶 `HostAuth` 的分享 server、真的
//! 跑一個 shell，用真的 TLS + WebSocket 連線驗證整條金鑰認證路徑。
//!
//! 跟 `src-tauri/tests/share_end_to_end.rs` 同一個理由：這條路徑的價值全在
//! 「真的接得起來」，用假的 PTY 或假的 TLS 握手測等於什麼都沒測。

use std::sync::Arc;
use std::time::Duration;

use aiterm_core::pty::PtyManager;
use aiterm_core::share::auth;
use aiterm_core::share::events::SilentEvents;
use aiterm_core::share::protocol::{ServerMessage, WireAccessMode};
use aiterm_core::share::registry::AccessMode;
use aiterm_core::share::server::HostAuth;
use aiterm_core::share::viewer::{connect_and_handshake, run_viewer_stream, ViewerEvent};
use aiterm_core::share::{tls, ShareServerState};
use futures_util::{SinkExt, StreamExt};
use portable_pty::PtySize;
use tokio_tungstenite::tungstenite::Message;

const SIZE: PtySize = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };

/// 起一個真的分享 server：真的 shell、真的 TLS、真的 `HostAuth`。
///
/// **一定要把回傳的 `Arc<ShareServerState>` 留在呼叫端的作用域裡**——它一
/// 掉，`running` 裡存的 `shutdown` sender 就跟著掉，`serve_tls` 的 accept
/// 迴圈會在下一次 `select!` 就看到 sender 已斷而收工，連線瞬間死光。
async fn start_host(
    mode: AccessMode,
) -> (Arc<PtyManager>, Arc<ShareServerState>, String, u16, Vec<u8>) {
    aiterm_core::share::ensure_crypto_provider();

    let pty = Arc::new(PtyManager::new());
    let tab_id = "cli".to_string();
    // 用跟 `aiterm-host` 主程式一樣的入口——這才是實機會走的路徑，包含
    // shell 整合腳本的注入（OSC 133 標記就是靠它，見 shell.rs）。
    pty.create_with_callback_and_id(SIZE, tab_id.clone(), None, Vec::new(), Vec::new(), |_| {})
        .expect("spawn shell");

    let key = auth::generate_key().to_vec();
    let state = Arc::new(ShareServerState::new());
    let code = state.registry.start_share(tab_id.clone());
    let host_auth = Arc::new(HostAuth { key: key.clone(), code, mode });

    let port = state
        .start_if_needed_on_with_auth(
            Arc::clone(&pty),
            std::net::Ipv4Addr::LOCALHOST,
            0,
            Arc::new(SilentEvents),
            Some(host_auth),
        )
        .await
        .expect("start share server");

    (pty, state, tab_id, port, key)
}

/// 連線並開始串流，回傳按鍵通道與事件通道。
///
/// **把「握手階段就被拒絕」跟「串流階段才結束」統一成同一種事件。**
/// `decide_join` 的 `Reject` 分支（金鑰錯誤／缺少金鑰）發生在 server 送出
/// `SasCommit` 之前（見 `server.rs` 的 `handle_share`），所以
/// `connect_and_handshake` 在這兩種情況下永遠拿不到 `ViewerHandshake`，只
/// 能回傳 `Err`——它連把 ws 交給 `run_viewer_stream` 的機會都沒有。這跟真正
/// 的 GUI（`viewer_manager::connect`）目前的行為一致：金鑰錯誤在那裡會變成
/// 一次失敗的 IPC 呼叫，不是一個 `Ended` 事件。
///
/// 這裡把那個 `Err`（已知在這兩種情境下只會是 `EndReason::Denied` 的
/// Display 字串）轉成跟正常流程一樣的 `ViewerEvent::Ended`，好讓所有測試能
/// 用同一套「收事件、斷言」的寫法，不用為了兩種被拒絕的方式分岔成兩套邏輯。
/// 若錯誤不是預期的 Denied，直接 panic——不要把非預期的失敗誤標成
/// "denied"，那會讓一個真正壞掉的握手看起來像是測試通過。
async fn connect(
    port: u16,
    key: Option<&[u8]>,
) -> (
    tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    tokio::sync::mpsc::UnboundedReceiver<ViewerEvent>,
) {
    let (keys_tx, keys_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel::<ViewerEvent>();

    match connect_and_handshake("127.0.0.1", port, "", "TestViewer", key).await {
        Ok(handshake) => {
            tokio::spawn(run_viewer_stream(
                handshake.ws,
                events_tx,
                keys_rx,
                handshake.key,
                handshake.auth_exporter,
            ));
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Denied") {
                let _ = events_tx.send(ViewerEvent::Ended { reason: "denied".to_string() });
            } else {
                panic!("connect_and_handshake failed for an unexpected reason: {msg}");
            }
        }
    }

    (keys_tx, events_rx)
}

#[tokio::test]
async fn a_viewer_with_the_right_key_is_granted_without_any_human_approval() {
    // 這是整個里程碑的核心主張：沒有人呼叫 `registry.approve()`。金鑰驗過
    // 就該自動核准，不需要人在旁邊唸 SAS。
    let (_pty, _state, _tab_id, port, key) = start_host(AccessMode::Control).await;

    let (_keys_tx, mut events_rx) = connect(port, Some(&key)).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("timed out waiting for Granted")
        .expect("event channel closed before Granted arrived");
    match ev {
        ViewerEvent::Granted { mode, .. } => {
            assert_eq!(mode, "control", "expected control mode, got {mode:?}")
        }
        other => panic!("expected Granted, got {other:?}"),
    }
}

#[tokio::test]
async fn a_viewer_with_the_wrong_key_is_denied() {
    let (_pty, _state, _tab_id, port, _key) = start_host(AccessMode::Control).await;
    let wrong_key = auth::generate_key().to_vec();

    let (_keys_tx, mut events_rx) = connect(port, Some(&wrong_key)).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("timed out waiting for Ended")
        .expect("event channel closed before Ended arrived");
    match ev {
        ViewerEvent::Ended { reason } => assert_eq!(reason, "denied"),
        other => panic!("expected Ended, got {other:?}"),
    }
}

#[tokio::test]
async fn a_viewer_with_no_key_is_denied() {
    // 舊版觀看端，或短碼模式的觀看端，指向一台 CLI host。
    let (_pty, _state, _tab_id, port, _key) = start_host(AccessMode::Control).await;

    let (_keys_tx, mut events_rx) = connect(port, None).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("timed out waiting for Ended")
        .expect("event channel closed before Ended arrived");
    match ev {
        ViewerEvent::Ended { reason } => assert_eq!(reason, "denied"),
        other => panic!("expected Ended, got {other:?}"),
    }
}

#[tokio::test]
async fn read_only_mode_is_reported_to_the_viewer() {
    let (_pty, _state, _tab_id, port, key) = start_host(AccessMode::ReadOnly).await;

    let (_keys_tx, mut events_rx) = connect(port, Some(&key)).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("timed out waiting for Granted")
        .expect("event channel closed before Granted arrived");
    match ev {
        ViewerEvent::Granted { mode, .. } => {
            assert_eq!(mode, "read_only", "expected read_only mode, got {mode:?}")
        }
        other => panic!("expected Granted, got {other:?}"),
    }
}

#[tokio::test]
async fn a_command_run_through_the_viewer_produces_an_osc_133_d_marker() {
    // 這個測試比看起來重要：觀看端的 AI agent 迴圈完全靠 `OSC 133;D` 判斷
    // 「這一步做完了」。沒有這個標記，每一步都會退化成 60 秒逾時才繼續，
    // 而且不會有任何錯誤訊息——症狀只有「agent 變得超慢」。
    //
    // 若這裡失敗，**不要放寬斷言**。那代表 CLI host 的 shell 沒有拿到
    // AITerm 的 shell 整合腳本，是需要回報的真缺陷，不是測試寫錯。
    let (_pty, _state, _tab_id, port, key) = start_host(AccessMode::Control).await;

    let (keys_tx, mut events_rx) = connect(port, Some(&key)).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("timed out waiting for Granted")
        .expect("event channel closed before Granted arrived");
    match ev {
        ViewerEvent::Granted { mode, .. } => assert_eq!(mode, "control"),
        other => panic!("expected Granted, got {other:?}"),
    }

    // 讓提示字元穩定下來再送指令——太快送指令有機會落在 shell 整合腳本
    // 還沒把 `OSC 133;A` 掛回新提示字元的窗口內。
    tokio::time::sleep(Duration::from_millis(1500)).await;

    keys_tx.send(b"echo aiterm-marker-probe\n".to_vec()).expect("send the probe command");

    let mut seen = Vec::new();
    let marker: &[u8] = b"\x1b]133;D";
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut found = false;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, events_rx.recv()).await {
            Ok(Some(ViewerEvent::Data(bytes))) => {
                seen.extend_from_slice(&bytes);
                if seen.windows(marker.len()).any(|w| w == marker) {
                    found = true;
                    break;
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(_) => break,
        }
    }

    assert!(
        found,
        "never saw an OSC 133;D marker in {} bytes of streamed output: {:?}",
        seen.len(),
        String::from_utf8_lossy(&seen)
    );
}

/// 一個假扮主控端的 TLS + WebSocket endpoint：走完承諾流程的訊息序列，但
/// `Granted` 帶的 `host_auth` 是用**錯的**金鑰算的。
///
/// 鏡像 `share::mod::serve_tls` 的 TLS 接線與 `share::server::handle_share`
/// 前半段的訊息序列，但完全不碰 registry、不碰 PTY——它不需要真的核准
/// 任何東西，只需要讓觀看端相信自己在走正常流程，直到 `Granted` 那一刻。
async fn run_fake_mitm_host(listener: tokio::net::TcpListener, wrong_key: Vec<u8>) {
    let (stream, _peer) = listener.accept().await.expect("accept a connection");

    // 1. TLS 握手——跟 `serve_tls`同一招：每次分享一組臨時自簽憑證。
    let identity = tls::ShareIdentity::generate().expect("generate a TLS identity");
    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![identity.cert_der.clone()], identity.key_der.clone_key())
        .expect("build a TLS server config");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let tls_stream = acceptor.accept(stream).await.expect("TLS handshake with the viewer");

    // 2. 導出認證用 material——用跟真正的 CLI host 一樣的 label,這條連線的
    // exporter 是真的（TLS 握手是真的），只有等一下拿它算證明的金鑰是假的。
    let auth_material = {
        let (_io, conn) = tls_stream.get_ref();
        tls::exporter_material_with_label(conn, tls::AUTH_EXPORTER_LABEL)
            .expect("export auth material")
    };

    // 3. 升級成 WebSocket。
    let mut ws = tokio_tungstenite::accept_async(tls_stream).await.expect("WebSocket handshake");

    // 4. 承諾一個 nonce——內容是什麼不重要，金鑰模式的觀看端根本不會顯示
    // SAS，這裡只是要走完觀看端期待的訊息序列。
    let host_nonce = tls::fresh_nonce();
    let commit = tls::commit_for(&host_nonce);
    ws.send(Message::Text(
        serde_json::to_string(&ServerMessage::SasCommit { commit }).unwrap().into(),
    ))
    .await
    .expect("send SasCommit");

    // 5. 收 Join——內容整個忽略。假主控端驗不了裡面的 auth 欄位，也不在乎。
    ws.next().await.expect("stream ended").expect("read Join");

    // 6. 收 SasNonce——同樣忽略內容，算 SAS 用不到它。
    ws.next().await.expect("stream ended").expect("read SasNonce");

    // 7. 揭曉——host_nonce 必須跟上面的承諾對得上，否則觀看端會用
    // `sas_commit_mismatch`／`sas_handshake_failed` 中止，而不是我們要測的
    // `host_auth_failed`。
    ws.send(Message::Text(
        serde_json::to_string(&ServerMessage::AwaitingApproval {
            host_nonce: tls::hex_of(&host_nonce),
        })
        .unwrap()
        .into(),
    ))
    .await
    .expect("send AwaitingApproval");

    // 8. 核准——但 host_auth 是用錯的金鑰算的。這是整個攻擊的重點：中間人
    // 不需要偽造觀看端的證明，它可以直接扮演主控端自己發 Granted。
    ws.send(Message::Text(
        serde_json::to_string(&ServerMessage::Granted {
            mode: WireAccessMode::Control,
            cols: 80,
            rows: 24,
            host_os: "linux".to_string(),
            host_auth: Some(auth::host_proof(&wrong_key, &auth_material)),
        })
        .unwrap()
        .into(),
    ))
    .await
    .expect("send Granted");

    // 一份假畫面。如果觀看端在驗 host_auth 失敗後沒有 `break`，這就是中間人
    // 用來餵假畫面／收走按鍵的東西——收到它本身就是傷害，不用等到內容被誤用。
    let _ = ws.send(Message::Binary(b"FAKE-SCREEN-DATA".to_vec().into())).await;

    // 撐著連線一小段時間，讓測試那邊的「之後不該再收到任何東西」有東西可以
    // 不收到；撐太久沒意義，撐 0 秒的話 socket 立刻斷線，測試會把「連線斷了
    // 所以收不到」跟「驗證擋住了所以收不到」搞混。
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn a_host_that_cannot_prove_the_key_never_gets_to_stream() {
    // 這個測試守的是防中間人設計裡唯一沒被任何既有測試踩到的分支：
    // `host_proof_acceptable` 回傳 `false` 之後那條路。全部既有測試都走
    // 短碼模式（`key = None`），那個分支在金鑰模式（`key = Some`）才可能
    // 走到——Task 11 的突變測試證實刪掉它後面的 `break;`，1576 個測試裡
    // 沒有任何一個變紅。
    //
    // 觀看端刻意不驗 TLS 憑證（`SasIsTheOnlyIdentityCheck`），金鑰模式又
    // 沒有人工 SAS 核對，所以中間人根本不用僞造任何東西：它終止 TLS、自己
    // 回 `Granted`、餵假畫面、收走每一個按鍵。能擋住它的只有觀看端對
    // `host_auth` 的驗證。
    aiterm_core::share::ensure_crypto_provider();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let viewer_key = auth::generate_key().to_vec();
    let wrong_key = auth::generate_key().to_vec();
    assert_ne!(viewer_key, wrong_key);

    tokio::spawn(run_fake_mitm_host(listener, wrong_key));

    // 假主控端會完整走完承諾流程（`connect_and_handshake` 到這裡都會成功
    // ——它只檢查 commit 跟 nonce 對不對得上，不驗 host_auth），所以直接
    // 用跟其他測試一樣的 `connect` helper 就好，不用重複組 handshake。
    let (_keys_tx, mut events_rx) = connect(port, Some(&viewer_key)).await;

    let ev = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .expect("timed out waiting for Ended")
        .expect("event channel closed before Ended arrived");
    match ev {
        ViewerEvent::Ended { reason } => {
            // 斷言精確字串，不能只驗「有 Ended 就好」——假主控端若哪裡壞了
            // 提早死掉，觀看端會用別的理由（例如 `session_closed`）結束，
            // 一個寬鬆的斷言會讓那種情況看起來像測試通過了。
            assert_eq!(
                reason, "host_auth_failed",
                "wrong Ended reason (a broken fake host would end for a different reason \
                 and a loose assertion would hide that)"
            );
        }
        other => panic!("expected Ended, got {other:?}"),
    }

    // 而且之後不能再收到任何東西——收到一份假畫面本身就是傷害，不用等到
    // 內容被誤用才算數。
    let after = tokio::time::timeout(Duration::from_millis(500), events_rx.recv()).await;
    match after {
        Err(_) => {} // timed out — nothing arrived, as expected
        Ok(None) => {} // channel closed — also fine
        Ok(Some(ev)) => panic!(
            "the viewer kept streaming after a failed host-auth check; got {ev:?}"
        ),
    }
}
