//! 觀看端：連進別台機器分享出來的終端機。
//!
//! **為什麼這一整層跑在 Rust 而不是前端 JS**：要連的是 TLS ＋ 自簽憑證，
//! 而 Tauri webview 的 `new WebSocket("wss://...")` 會被 WKWebView /
//! WebView2 拒絕——它們強制驗證憑證鏈且沒有程式化例外。`rustls` 則允許
//! 自訂驗證器。自簽憑證本來就沒有鏈可驗，**身分保證來自 SAS 人工核對**。
//!
//! 前端只訂閱事件，比照本機 PTY 用 `pty://data/{id}` 餵畫面的既有模式。

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use super::protocol::{ClientMessage, ServerMessage, PROTOCOL_VERSION};
use super::tls;

/// 觀看端的憑證驗證器：接受任何憑證。
///
/// **這不是偷懶。** 自簽憑證沒有任何憑證鏈可以驗——觀看端沒有先驗資訊能
/// 判斷該信任哪一張。身分保證完全來自 SAS 人工核對（見 `share::tls`）。
/// 寫成一個有名字的型別是為了讓這件事在程式碼審查時顯眼，而不是藏在一個
/// `dangerous()` 呼叫裡。
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

/// 一條完成握手、等待對方裁決的觀看連線。
pub struct ViewerHandshake {
    /// **這一端算出的 4 位驗證碼，要唸給對方聽。**
    ///
    /// 跟主控端相反：主控端的碼絕不送到前端（看得到就會照抄而不問對方），
    /// 觀看端的碼**必須**送到前端，因為那正是要唸出來的東西。兩邊不對稱
    /// 是刻意的，不要「為了一致性」把這個藏起來。
    pub sas: String,
    /// 已完成握手的 ws，交給串流迴圈用（Task 2）。
    pub ws: WebSocketStream<TlsStream<TcpStream>>,
}

/// 主控端揭曉的 nonce 跟先前的承諾對不對得上。
///
/// 對不上就是中間人的跡象——它必須在看到觀看端的 nonce 之前就承諾自己的，
/// 若它中途換了一個，這裡會抓到。格式錯誤（非 hex）也回 `false`，那同樣
/// 是不該放行的狀況。
pub fn commit_matches(commit: &str, host_nonce: &[u8]) -> bool {
    tls::commit_for(host_nonce) == commit
}

/// 連線並走完 SAS 承諾握手。回傳時對方尚未裁決。
pub async fn connect_and_handshake(
    host: &str,
    port: u16,
    code: &str,
    display_name: &str,
) -> Result<ViewerHandshake> {
    super::ensure_crypto_provider();

    let tcp = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("連不上 {host}:{port}"))?;

    let mut cfg = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SasIsTheOnlyIdentityCheck))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];

    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    // 憑證的 CN 沒有意義（我們不驗它），但 rustls 的 API 要求一個名字。
    let server_name = rustls::pki_types::ServerName::try_from("aiterm-share")
        .map_err(|e| anyhow!("server name: {e}"))?;
    let tls = connector.connect(server_name, tcp).await.context("TLS 握手失敗")?;

    // 握手完成後、交給 tungstenite 之前先取 exporter material。SAS 要等
    // 承諾流程走完（拿到兩個 nonce）才算得出來。
    let exporter = {
        let (_io, conn) = tls.get_ref();
        tls::exporter_material(conn).context("導出金鑰 material 失敗")?
    };

    let (mut ws, _) = tokio_tungstenite::client_async("ws://aiterm-share/share", tls)
        .await
        .context("WebSocket 握手失敗")?;

    // 1. Join
    send_json(
        &mut ws,
        &ClientMessage::Join {
            protocol_version: PROTOCOL_VERSION,
            code: code.to_string(),
            display_name: display_name.to_string(),
        },
    )
    .await?;

    // 2. 收主控端的承諾
    let commit = match recv_json(&mut ws).await? {
        ServerMessage::SasCommit { commit } => commit,
        ServerMessage::Ended { reason } => return Err(anyhow!("連線被拒：{reason:?}")),
        other => return Err(anyhow!("預期 SasCommit，收到 {other:?}")),
    };

    // 3. 送出自己的 nonce
    let viewer_nonce = tls::fresh_nonce();
    send_json(&mut ws, &ClientMessage::SasNonce { nonce: tls::hex_of(&viewer_nonce) }).await?;

    // 4. 收主控端揭曉的 nonce
    let host_nonce_hex = match recv_json(&mut ws).await? {
        ServerMessage::AwaitingApproval { host_nonce } => host_nonce,
        ServerMessage::Ended { reason } => return Err(anyhow!("連線被拒：{reason:?}")),
        other => return Err(anyhow!("預期 AwaitingApproval，收到 {other:?}")),
    };
    let host_nonce = tls::decode_hex(&host_nonce_hex)
        .ok_or_else(|| anyhow!("主控端送來的 nonce 不是合法的 hex"))?;

    // 5. 驗證承諾。對不上就中止——那是中間人的跡象。
    if !commit_matches(&commit, &host_nonce) {
        return Err(anyhow!(
            "驗證失敗：對方揭曉的值跟先前的承諾對不上，可能有人在中間攔截"
        ));
    }

    let sas = tls::sas_from_parts(&host_nonce, &viewer_nonce, &exporter);
    Ok(ViewerHandshake { sas, ws })
}

async fn send_json(
    ws: &mut WebSocketStream<TlsStream<TcpStream>>,
    msg: &ClientMessage,
) -> Result<()> {
    let json = serde_json::to_string(msg)?;
    ws.send(Message::Text(json.into())).await.context("送出訊息失敗")?;
    Ok(())
}

/// 讀到下一則 JSON 控制訊息，跳過二進位 frame。
///
/// 握手階段不該收到二進位（協定規定 `Granted` 之前沒有 PTY 位元組），但
/// 跳過而不是報錯比較穩健——真的收到也只是忽略。
async fn recv_json(ws: &mut WebSocketStream<TlsStream<TcpStream>>) -> Result<ServerMessage> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => {
                return serde_json::from_str(&t).context("對方送來無法解析的訊息")
            }
            Some(Ok(Message::Close(_))) | None => return Err(anyhow!("連線已關閉")),
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(anyhow!("連線錯誤：{e}")),
        }
    }
}

/// 串流階段送給上層的事件。上層（`viewer_manager`）把它們轉成 Tauri 事件。
///
/// 用 channel 而不是直接發 Tauri 事件，是為了讓這一層能在不起 Tauri app
/// 的情況下測試——整合測試直接收 channel 就好。
#[derive(Debug)]
pub enum ViewerEvent {
    /// 對方同意了。`cols`/`rows` 是主控端的終端機尺寸，觀看端必須照這個
    /// 建立 xterm，不能用自己的視窗大小。
    Granted { mode: String, cols: u16, rows: u16 },
    /// 遠端 PTY 的原始位元組。
    Data(Vec<u8>),
    /// 落後太多，清空畫面，下一批 `Data` 是全量重播。
    Resync,
    /// 控制權變動。
    ControlChanged { mode: String },
    /// 連線結束。`reason` 是 `EndReason` 的 snake_case 字串。
    Ended { reason: String },
}

/// 串流迴圈：下行 PTY 畫面、上行按鍵、處理控制訊息。
///
/// `keys` 是上層送進來的按鍵。**唯讀模式下上層就不該送**——伺服器端還有
/// 一道 `may_send_input` 檢查，但那是安全邊界，這裡不重複實作。
pub async fn run_viewer_stream(
    mut ws: WebSocketStream<TlsStream<TcpStream>>,
    events: tokio::sync::mpsc::UnboundedSender<ViewerEvent>,
    mut keys: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
) {
    loop {
        tokio::select! {
            incoming = ws.next() => match incoming {
                Some(Ok(Message::Binary(bytes))) => {
                    if events.send(ViewerEvent::Data(bytes.to_vec())).is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Text(t))) => {
                    let Ok(msg) = serde_json::from_str::<ServerMessage>(&t) else {
                        // 解析不了通常代表對方版本較新。握手時已經擋過版本
                        // 落差，走到這裡還解不了就是真的壞了——結束連線。
                        let _ = events.send(ViewerEvent::Ended {
                            reason: "sas_handshake_failed".to_string(),
                        });
                        break;
                    };
                    match msg {
                        ServerMessage::Granted { mode, cols, rows } => {
                            let _ = events.send(ViewerEvent::Granted {
                                mode: wire_mode_str(mode),
                                cols,
                                rows,
                            });
                        }
                        ServerMessage::Resize { cols, rows } => {
                            // 尺寸變更跟 Granted 用同一個事件——上層只要照著
                            // 重新 fit 即可，不需要區分是初次還是後續。
                            let _ = events.send(ViewerEvent::Granted {
                                mode: String::new(),
                                cols,
                                rows,
                            });
                        }
                        ServerMessage::Resync => {
                            let _ = events.send(ViewerEvent::Resync);
                        }
                        ServerMessage::ControlChanged { mode } => {
                            let _ = events.send(ViewerEvent::ControlChanged {
                                mode: wire_mode_str(mode),
                            });
                        }
                        ServerMessage::Ended { reason } => {
                            let _ = events.send(ViewerEvent::Ended {
                                reason: end_reason_str(reason),
                            });
                            break;
                        }
                        // 握手階段的訊息在串流階段不該再出現，忽略。
                        ServerMessage::SasCommit { .. } | ServerMessage::AwaitingApproval { .. } => {}
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    let _ = events.send(ViewerEvent::Ended {
                        reason: "session_closed".to_string(),
                    });
                    break;
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => {
                    let _ = events.send(ViewerEvent::Ended {
                        reason: "session_closed".to_string(),
                    });
                    break;
                }
            },
            Some(data) = keys.recv() => {
                if ws.send(Message::Binary(data.into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

fn wire_mode_str(mode: super::protocol::WireAccessMode) -> String {
    match mode {
        super::protocol::WireAccessMode::ReadOnly => "read_only".to_string(),
        super::protocol::WireAccessMode::Control => "control".to_string(),
    }
}

fn end_reason_str(reason: super::protocol::EndReason) -> String {
    use super::protocol::EndReason as R;
    // 沒有 `_` 萬用分支：新增 EndReason 變體時這裡會編譯失敗，那是提醒你
    // 前端也要加一句對應的人話（見 spec 的錯誤處理表）。
    match reason {
        R::Denied => "denied",
        R::HostStoppedSharing => "host_stopped_sharing",
        R::SessionClosed => "session_closed",
        R::KickedByHost => "kicked_by_host",
        R::InvalidCode => "invalid_code",
        R::VersionMismatch => "version_mismatch",
        R::SasCommitMismatch => "sas_commit_mismatch",
        R::SasHandshakeFailed => "sas_handshake_failed",
    }
    .to_string()
}
