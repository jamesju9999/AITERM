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
/// SAS 導出由 Task 8 的 `both_ends_of_a_real_tls_connection_derive_the_same_sas`
/// 單獨驗。正式路徑的 `ConnectionExporter` 由 TLS accept 迴圈注入，這裡手動
/// 補一個佔位值；**不要**把 handler 的 extractor 改成 `Option`，那等於讓正式
/// 路徑在 TLS 沒接上時無聲降級成沒有身分保證的連線。
async fn start_test_server(
    pty: Arc<PtyManager>,
    registry: Arc<ShareRegistry>,
) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = aiterm_lib::share::server::router(pty, registry, None).layer(axum::Extension(
        aiterm_lib::share::protocol::ConnectionExporter(
            [0u8; aiterm_lib::share::tls::SAS_MATERIAL_LEN],
        ),
    ));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
}

/// 送出 `Join`，然後走完 SAS 承諾流程：收 `SasCommit`、回一個隨機
/// `SasNonce`、等 `AwaitingApproval` 揭曉主控端的 nonce。
///
/// 回傳 `(host_nonce, viewer_nonce)`——呼叫端接下來會收到
/// `AwaitingApproval`（已經在這裡被消費掉了），可以用這兩個 nonce 加上
/// `start_test_server` 灌的全零 exporter 自己算一份 SAS 來核對。
async fn join_and_do_sas_handshake<S>(ws: &mut S, code: &str, name: &str) -> (Vec<u8>, Vec<u8>)
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error>
        + StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::Join {
            protocol_version: aiterm_lib::share::protocol::PROTOCOL_VERSION,
            code: code.to_string(),
            display_name: name.to_string(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    // 承諾必須先於揭曉——這個 helper 只消費它並檢查形狀；那個順序本身由
    // `the_host_commits_before_it_can_see_the_viewer_nonce` 單獨守著。
    match next_control(ws).await {
        ServerMessage::SasCommit { commit } => {
            assert_eq!(commit.len(), 64, "commit should be hex sha256");
        }
        other => panic!("expected SasCommit, got {other:?}"),
    }

    let viewer_nonce: [u8; 32] = {
        use rand::RngCore;
        let mut n = [0u8; 32];
        rand::rng().fill_bytes(&mut n);
        n
    };
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::SasNonce {
            nonce: aiterm_lib::share::tls::hex_of(&viewer_nonce),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();

    let host_nonce = match next_control(ws).await {
        ServerMessage::AwaitingApproval { host_nonce } => {
            aiterm_lib::share::tls::decode_hex(&host_nonce).expect("host_nonce is valid hex")
        }
        other => panic!("expected AwaitingApproval, got {other:?}"),
    };

    (host_nonce, viewer_nonce.to_vec())
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
    // 短碼在 SAS 承諾流程*之後*才被查——commit/nonce 交換不需要知道短碼是否
    // 有效，所以這裡不能用 `join_and_do_sas_handshake`（它假設一路走到
    // `AwaitingApproval`）：無效短碼會在揭曉那一步之前就被 `Ended` 擋下。
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

    match next_control(&mut ws).await {
        ServerMessage::SasCommit { .. } => {}
        other => panic!("expected SasCommit even for an unknown code, got {other:?}"),
    }
    ws.send(Message::Text(
        serde_json::to_string(&ClientMessage::SasNonce {
            nonce: aiterm_lib::share::tls::hex_of(&[0u8; 32]),
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
    join_and_do_sas_handshake(&mut ws, &code, "Alice").await;

    // 模擬主控端按下「只能看」。
    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    let viewer_id = registry.approve(&request_id, AccessMode::ReadOnly).expect("approve");

    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Granted {
            mode: WireAccessMode::ReadOnly,
            cols: 80,
            rows: 24,
            host_os: std::env::consts::OS.to_string(),
        }
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
    join_and_do_sas_handshake(&mut ws, &code, "Alice").await;

    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    registry.approve(&request_id, AccessMode::Control).expect("approve");
    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Granted {
            mode: WireAccessMode::Control,
            cols: 80,
            rows: 24,
            host_os: std::env::consts::OS.to_string(),
        }
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
    join_and_do_sas_handshake(&mut ws, &code, "Alice").await;

    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    let viewer_id = registry.approve(&request_id, AccessMode::Control).expect("approve");
    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Granted {
            mode: WireAccessMode::Control,
            cols: 80,
            rows: 24,
            host_os: std::env::consts::OS.to_string(),
        }
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
    join_and_do_sas_handshake(&mut ws, &code, "Alice").await;
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
    join_and_do_sas_handshake(&mut ws, &code, "Alice").await;
    let request_id = registry.pending(&tab_id)[0].request_id.clone();
    registry.approve(&request_id, AccessMode::ReadOnly).expect("approve");
    let _ = next_control(&mut ws).await; // Granted

    registry.stop_share(&tab_id);

    assert_eq!(
        next_control(&mut ws).await,
        ServerMessage::Ended { reason: EndReason::HostStoppedSharing }
    );
}

#[tokio::test]
async fn the_host_commits_before_it_can_see_the_viewer_nonce() {
    // 主控端必須在收到觀看端 nonce **之前**送出承諾。順序反過來的話，
    // 中間人就能在看到觀看端的貢獻後才挑自己的，搜出相同的 4 位數。
    // 這個測試就是在守那個順序：連上、送 Join、然後**什麼都不送**，
    // SasCommit 必須自己來。
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

    // 送完 Join 之後不送 nonce，直接等 —— SasCommit 應該主動抵達
    match next_control(&mut ws).await {
        ServerMessage::SasCommit { commit } => {
            assert_eq!(commit.len(), 64, "commit should be hex sha256");
        }
        other => panic!("host must commit before receiving the viewer nonce, got {other:?}"),
    }
}

#[tokio::test]
async fn both_ends_derive_the_same_sas_from_the_committed_nonces() {
    // 兩端用同樣三份材料各自算，結果必須一致——這是人工核對能成立的前提。
    // `start_test_server` 灌的 exporter material 固定是全零，所以觀看端這裡
    // 能用同一份常數重算，跟主控端存進 `PendingRequest` 的那一份比對。
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let registry = Arc::new(ShareRegistry::new());
    let code = registry.start_share(tab_id.clone());
    let port = start_test_server(Arc::clone(&pty), Arc::clone(&registry)).await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/share"))
        .await
        .expect("connect");
    let (host_nonce, viewer_nonce) = join_and_do_sas_handshake(&mut ws, &code, "Alice").await;

    let host_side_sas = registry.pending(&tab_id)[0].sas.clone();
    let viewer_side_sas = aiterm_lib::share::tls::sas_from_parts(
        &host_nonce,
        &viewer_nonce,
        &[0u8; aiterm_lib::share::tls::SAS_MATERIAL_LEN],
    );

    assert_eq!(
        host_side_sas, viewer_side_sas,
        "host and viewer derived different SASes from the same committed nonces"
    );
}

/// 測試用的客戶端憑證驗證器：接受任何憑證。
///
/// 這在這裡**不是偷懶**——正式的觀看端也必須這樣做。自簽憑證沒有任何憑證鏈
/// 可以驗，身分保證完全來自 SAS 人工核對。把這件事寫成一個有名字的型別，是
/// 為了讓它在程式碼審查時顯眼，而不是藏在一個 `danger` 呼叫裡。
#[derive(Debug)]
struct SasIsTheOnlyIdentityCheck;

impl rustls::client::danger::ServerCertVerifier for SasIsTheOnlyIdentityCheck {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[tokio::test]
async fn both_ends_of_a_real_tls_connection_derive_the_same_sas() {
    // 這是防中間人設計的支點，必須用真的 TLS 握手驗證，不能只測折疊函式：
    // 中間人的兩條連線導出不同的 material，兩邊數字對不起來——但那個保證
    // 只有在「雙方各自從自己的連線導出」時才成立。
    let _ = rustls::crypto::ring::default_provider().install_default();

    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let state = aiterm_lib::share::ShareServerState::new();
    let code = state.registry.start_share(tab_id.clone());
    // 固定 port，不用 start_if_needed 的浮動 port——這個檢查的重點是「另一台
    // 機器連得到嗎」，而對方必須**先知道**要連哪裡。浮動 port 逼人先跑起來
    // 才看得到位址，而這個測試會撐三分鐘，在終端機裡很容易被丟到背景而錯過
    // 輸出（第一次跑就是這樣浪費掉的）。
    let want: u16 = std::env::var("AITERM_PROBE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(47823);
    let port = state
        .start_if_needed_on_port(Arc::clone(&pty), want, None)
        .await
        .unwrap_or_else(|e| panic!("bind 0.0.0.0:{want} failed: {e}（用 AITERM_PROBE_PORT 換一個）"));

    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("tcp connect");

    let mut cfg = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SasIsTheOnlyIdentityCheck))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];

    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let server_name = rustls::pki_types::ServerName::try_from("aiterm-share").unwrap();
    let tls = connector.connect(server_name, tcp).await.expect("tls handshake");

    // 握手完成後、把 stream 交給 tungstenite 之前，先取自己這端的 exporter
    // material。SAS 要等承諾流程走完（拿到兩個 nonce）才算得出來。
    let client_exporter = {
        let (_io, conn) = tls.get_ref();
        aiterm_lib::share::tls::exporter_material(conn).expect("client exporter material")
    };

    let (mut ws, _) = tokio_tungstenite::client_async("ws://aiterm-share/share", tls)
        .await
        .expect("ws handshake");

    // 走完 SAS 承諾流程（Join → SasCommit → SasNonce → AwaitingApproval）。
    // 這個 helper 已經在 Task 7b 的整合測試裡了，直接重用。
    let (host_nonce, viewer_nonce) =
        join_and_do_sas_handshake(&mut ws, &code, "Alice").await;

    // 觀看端用自己的 exporter material ＋ 兩個 nonce 自己算一份，**不從線路
    // 上接收任何 SAS**。主控端那一份存在 PendingRequest 裡給同意視窗顯示。
    // 兩端算出來必須一致——這就是人工核對能抓到中間人的原因。
    let client_sas = aiterm_lib::share::tls::sas_from_parts(
        &host_nonce,
        &viewer_nonce,
        &client_exporter,
    );

    let pending = state.registry.pending(&tab_id);
    assert_eq!(pending.len(), 1, "expected exactly one pending request");
    assert_eq!(
        pending[0].sas, client_sas,
        "both ends of the same TLS connection must derive the same SAS"
    );
    assert_eq!(client_sas.len(), 4);
}

/// 手動的區網連通性檢查。**平常不跑**（`#[ignore]`），因為它會把 server
/// 撐開等人來連。
///
/// 這個測試存在的理由：計畫①的所有自動測試都跑在 `127.0.0.1`，證明了協定
/// 本身是對的，但**完全沒有證明第二台機器連得進來**。綁 `0.0.0.0` 之後真正
/// 的未知數有兩個，兩個都不是程式碼問題：
///
/// 1. **作業系統防火牆**——macOS 首次綁定非 loopback 位址時會跳「是否允許
///    接受連入連線」。使用者按了拒絕（慌張時很常見），功能就靜默失效。
/// 2. **網路層隔離**——公司／訪客 Wi-Fi 的 client isolation 讓同網段裝置
///    互相看不到，這在辦公室並不罕見。
///
/// 跑法：
/// ```text
/// cd src-tauri
/// cargo test --test share_end_to_end lan_reachability -- --ignored --nocapture
/// ```
/// 然後在**另一台機器**上照它印出來的指令試連。對方**不需要裝 Rust**——
/// TLS 與 ws 握手已經被上面那些測試證明過了，這裡要驗的只是「TCP 到得了」。
#[tokio::test]
#[ignore = "manual: holds a LAN port open and waits for a human to try connecting"]
async fn lan_reachability_probe() {
    let pty = Arc::new(PtyManager::new());
    let tab_id = pty.create_with_callback(SIZE, |_| {}).expect("spawn pty");
    let state = aiterm_lib::share::ShareServerState::new();
    let code = state.registry.start_share(tab_id.clone());
    // 固定 port，不用 start_if_needed 的浮動 port——這個檢查的重點是「另一台
    // 機器連得到嗎」，而對方必須**先知道**要連哪裡。浮動 port 逼人先跑起來
    // 才看得到位址，而這個測試會撐三分鐘，在終端機裡很容易被丟到背景而錯過
    // 輸出（第一次跑就是這樣浪費掉的）。
    let want: u16 = std::env::var("AITERM_PROBE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(47823);
    let port = state
        .start_if_needed_on_port(Arc::clone(&pty), want, None)
        .await
        .unwrap_or_else(|e| panic!("bind 0.0.0.0:{want} failed: {e}（用 AITERM_PROBE_PORT 換一個）"));

    // 盡力找出這台機器在區網上的位址。找不到不是錯誤——使用者自己知道 IP。
    let ip = std::process::Command::new("sh")
        .arg("-c")
        .arg("ipconfig getifaddr en0 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}'")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "<這台機器的區網 IP>".to_string());

    println!("\n========================================");
    println!("  共享 server 已啟動並綁在 0.0.0.0:{port}");
    println!("  短碼：{code}");
    println!("========================================");
    println!("\n如果 macOS 跳出防火牆詢問，請按「允許」——按拒絕就是這個檢查要抓的失敗情境之一。\n");
    println!("在另一台機器上跑（擇一）：");
    println!("  Windows PowerShell:  Test-NetConnection {ip} -Port {port}");
    println!("  macOS / Linux:       nc -vz {ip} {port}");
    println!("\n看到 TcpTestSucceeded: True（或 nc 的 succeeded）就代表區網連得到。");
    println!("連不到的話，問題在防火牆或網路隔離，不在這份程式碼裡。\n");
    println!("這個 server 會撐開 3 分鐘，然後自己關掉。\n");

    tokio::time::sleep(Duration::from_secs(180)).await;

    println!("時間到，關閉 server。");
    state.registry.stop_share(&tab_id);
}
