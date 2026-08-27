# 遠端終端機共享 2B-1：觀看端的 Rust 客戶端 — 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 AITerm 能以觀看端的身分連進別台機器的分享——TLS 握手、SAS 承諾核對、PTY 畫面下行、按鍵上行，全部跑在 Rust，前端只訂閱事件。

**Architecture:** 比照本機終端機的既有模式：`PtySession` 把 PTY bytes 用 `pty://data/{id}` 事件餵給前端，`TerminalView` 只是訂閱者。觀看端同理——`ViewerConnection` 跑在 Rust，用 `share-viewer://data/{id}` 把遠端畫面餵給前端。

**Tech Stack:** Rust / tokio-rustls（自訂憑證驗證器）/ tokio-tungstenite / Tauri commands + events

**Spec:** `docs/superpowers/specs/2026-08-26-remote-terminal-sharing-2-ui-design.md`

---

## 為什麼觀看端的連線必須跑在 Rust

spec 沒有指定觀看端的傳輸層跑在哪，這是規格缺口，在這裡補上。

**JS 不能做這件事**：觀看端要連的是 TLS + **自簽憑證**。Tauri webview 裡的 `new WebSocket("wss://...")` 會被 WKWebView / WebView2 拒絕——它們強制驗證憑證鏈，而且沒有程式化的例外機制。

**Rust 可以**：`rustls` 允許自訂 `ServerCertVerifier`。而自簽憑證本來就沒有憑證鏈可驗——**身分保證來自 SAS 人工核對**，不是憑證。

**現成的參考**：`src-tauri/tests/share_end_to_end.rs` 的 `both_ends_of_a_real_tls_connection_derive_the_same_sas`（約 469-575 行）已經有一個可運作的客戶端：`SasIsTheOnlyIdentityCheck` 驗證器、`TlsConnector`、`client_async`、以及完整的 SAS 承諾握手。當初寫的時候就註明「這是計畫②觀看端要用的那一套」。**這個計畫大致就是把它從測試碼提升成正式程式碼並補上串流。**

---

## 主控端與觀看端在 SAS 上是相反的——兩邊都是刻意的

這一點最容易被後人「為了一致性」改壞，所以先寫清楚：

| | 主控端（2A 已完成） | 觀看端（本計畫） |
|---|---|---|
| 自己算出的 SAS | **絕不送到前端**——回傳結構沒有 sas 欄位，比對在 Rust 的 `decide()` 裡做 | **必須送到前端**——那是要唸給對方聽的東西 |
| 為什麼 | 看得到就會照抄而不問對方，人工核對變成自欺 | 唸不出來就沒得核對 |

**不要把觀看端的 SAS 也藏起來**。兩邊不對稱不是疏漏，是這個設計能成立的原因：一方持有答案並唸出來，另一方沒有答案只能輸入聽到的。

---

## 檔案結構

| 檔案 | 責任 | 動作 |
|---|---|---|
| `src-tauri/src/share/viewer.rs` | 一條觀看連線：TLS、握手、SAS、串流迴圈 | 新增 |
| `src-tauri/src/share/viewer_manager.rs` | 連線的生命週期管理（比照 `PtyManager`） | 新增 |
| `src-tauri/src/share/mod.rs` | 宣告兩個新模組 | 修改 |
| `src-tauri/src/commands/share_viewer.rs` | 三個 Tauri command | 新增 |
| `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs` | 註冊 | 修改 |
| `src/ipc/shareViewer.ts` | 型別化 IPC ＋ 事件訂閱 | 新增 |
| `src-tauri/tests/share_viewer.rs` | 端到端：自己的 client 連自己的 server | 新增 |

---

## 事件契約

觀看端的事件比照 `pty://data/{session_id}` 的既有形狀，每條連線一個 id：

| 事件 | payload | 意義 |
|---|---|---|
| `share-viewer://sas/{id}` | `{ sas: String }` | **我這端算出的 4 位碼，要唸給對方**。到達後前端顯示「請把這組數字唸給對方」 |
| `share-viewer://granted/{id}` | `{ mode, cols, rows }` | 對方同意了。前端照 cols/rows 建 xterm |
| `share-viewer://data/{id}` | `{ base64: String }` | 遠端 PTY 畫面。跟 `pty://data` 同樣是 base64 |
| `share-viewer://resync/{id}` | `()` | 清空畫面，下一個 data 是全量重播 |
| `share-viewer://control/{id}` | `{ mode }` | 控制權變動 |
| `share-viewer://ended/{id}` | `{ reason: String }` | 連線結束，`reason` 是 `EndReason` 的 snake_case 字串 |

**為什麼 `data` 用 base64**：跟本機 PTY 同一個理由——Tauri 的事件 payload 走 JSON，原始位元組塞不進去。既有的 `PtyDataPayload` 就是這樣做的（`src-tauri/src/pty/events.rs`）。

---

## Task 1: 連線與 SAS 承諾握手

**Files:**
- Create: `src-tauri/src/share/viewer.rs`
- Modify: `src-tauri/src/share/mod.rs`
- Test: `src-tauri/tests/share_viewer.rs`（新增）

- [ ] **Step 1: 寫會紅的測試**

建立 `src-tauri/tests/share_viewer.rs`：

```rust
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
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --test share_viewer`
Expected: **編譯失敗**——`could not find viewer in share`。

- [ ] **Step 3: 把 `tokio-tungstenite` 提為正式依賴**

它目前只在 `[dev-dependencies]` 裡（計畫①只在整合測試用它當客戶端），但
觀看端是**正式程式碼**，所以要移到 `[dependencies]`。

`src-tauri/Cargo.toml` 的 `[dependencies]` 區塊加入（放在 `axum` 附近）：

```toml
# 觀看端的 WebSocket 客戶端。版本必須跟 axum 的 `ws` feature 帶進來的那份
# 一致（0.29），否則相依樹會多出第二份互不相容的拷貝。
tokio-tungstenite = "0.29"
```

`[dev-dependencies]` 裡那一行 `tokio-tungstenite = "0.29"` 連同它上面的註解
**一起刪掉**——正式依賴涵蓋了測試用途，留著會是重複宣告。`futures-util`
兩邊都有宣告，那個是既有狀態，**不要動**。

改完確認沒有拉進第二份：

```bash
cd src-tauri && grep -c '^name = "tokio-tungstenite"' Cargo.lock
```

Expected: `1`。

- [ ] **Step 4: 實作 `viewer.rs` 的連線與握手**

建立 `src-tauri/src/share/viewer.rs`：

```rust
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
```

- [ ] **Step 5: `ensure_crypto_provider` 提為 `pub(crate)`**

`viewer.rs` 要用它。`src-tauri/src/share/mod.rs` 把：

```rust
fn ensure_crypto_provider() {
```

改成：

```rust
pub(crate) fn ensure_crypto_provider() {
```

並在同一個檔案的模組宣告區加入：

```rust
pub mod viewer;
```

- [ ] **Step 6: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --test share_viewer`
Expected: `3 passed`。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/share/ src-tauri/tests/share_viewer.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(share): viewer-side TLS connection and SAS commitment handshake

觀看端的傳輸必須跑在 Rust——Tauri webview 的 WebSocket 會拒絕自簽憑證，
而 rustls 允許自訂驗證器。身分保證來自 SAS 人工核對，不是憑證鏈。

觀看端的 SAS 會送到前端（要唸給對方聽），跟主控端相反——那邊的碼絕不
送到前端。兩邊不對稱是刻意的。"
```

---

## Task 2: 串流迴圈

握手完成後，連線進入串流狀態：PTY 畫面下行、按鍵上行、控制訊息處理。

**Files:**
- Modify: `src-tauri/src/share/viewer.rs`
- Test: `src-tauri/tests/share_viewer.rs`

- [ ] **Step 1: 寫會紅的測試**

加到 `src-tauri/tests/share_viewer.rs`：

```rust
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
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `cd src-tauri && cargo test --test share_viewer`
Expected: **編譯失敗**——`cannot find function run_viewer_stream`。

- [ ] **Step 3: 實作串流迴圈**

加到 `src-tauri/src/share/viewer.rs`：

```rust
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
```

- [ ] **Step 4: 跑測試確認轉綠**

Run: `cd src-tauri && cargo test --test share_viewer`
Expected: `5 passed`。

連跑 5 次確認不 flaky（真 PTY ＋ 真 TLS ＋ 真 ws）：

```bash
cd src-tauri
for i in $(seq 1 5); do
  cargo test --test share_viewer 2>&1 | grep -E "^test result" || echo "RUN $i FAILED"
done
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/share/viewer.rs src-tauri/tests/share_viewer.rs
git commit -m "feat(share): viewer streaming loop

下行 PTY 畫面、上行按鍵、控制訊息。用 channel 而不是直接發 Tauri 事件，
讓這一層能在不起 Tauri app 的情況下測試。

end_reason_str 沒有萬用分支——新增 EndReason 時會編譯失敗，提醒前端也要
加一句對應的人話。"
```

---

## Task 3: 連線管理與 Tauri commands

**Files:**
- Create: `src-tauri/src/share/viewer_manager.rs`
- Create: `src-tauri/src/commands/share_viewer.rs`
- Modify: `src-tauri/src/share/mod.rs`、`src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs`

- [ ] **Step 1: 實作 `ViewerManager`**

建立 `src-tauri/src/share/viewer_manager.rs`：

```rust
//! 觀看連線的生命週期管理。比照 `PtyManager`：用 id 記住每條連線，
//! 提供 write / close。
//!
//! 跟 `PtyManager` 一樣把事件發送的責任放在這裡（而不是 `viewer.rs`），
//! 讓協定那一層能在不起 Tauri app 的情況下測試。

use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::viewer::{connect_and_handshake, run_viewer_stream, ViewerEvent};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SasPayload {
    sas: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GrantedPayload {
    mode: String,
    cols: u16,
    rows: u16,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DataPayload {
    base64: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ModePayload {
    mode: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EndedPayload {
    reason: String,
}

struct Connection {
    keys: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

#[derive(Default)]
pub struct ViewerManager {
    connections: Mutex<HashMap<String, Connection>>,
}

impl ViewerManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 連線並開始串流。回傳連線 id，後續的事件都掛在這個 id 上。
    ///
    /// 握手完成（但對方尚未裁決）時就回傳——此時已經發出
    /// `share-viewer://sas/{id}` 事件，前端可以顯示「請把這組數字唸給對方」。
    pub async fn connect(
        &self,
        app: AppHandle,
        host: String,
        port: u16,
        code: String,
        display_name: String,
    ) -> anyhow::Result<String> {
        let handshake = connect_and_handshake(&host, port, &code, &display_name).await?;
        let id = Uuid::new_v4().to_string();

        // 觀看端的 SAS **要**送給前端——那是要唸給對方聽的。跟主控端相反，
        // 那邊的碼絕不送到前端。見 `viewer::ViewerHandshake::sas`。
        let _ = app.emit(
            &format!("share-viewer://sas/{id}"),
            SasPayload { sas: handshake.sas },
        );

        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel::<ViewerEvent>();
        let (keys_tx, keys_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

        self.connections.lock().insert(id.clone(), Connection { keys: keys_tx });

        tokio::spawn(run_viewer_stream(handshake.ws, events_tx, keys_rx));

        let id_for_pump = id.clone();
        tokio::spawn(async move {
            while let Some(ev) = events_rx.recv().await {
                let id = &id_for_pump;
                match ev {
                    ViewerEvent::Granted { mode, cols, rows } => {
                        let _ = app.emit(
                            &format!("share-viewer://granted/{id}"),
                            GrantedPayload { mode, cols, rows },
                        );
                    }
                    ViewerEvent::Data(bytes) => {
                        let _ = app.emit(
                            &format!("share-viewer://data/{id}"),
                            DataPayload { base64: BASE64.encode(&bytes) },
                        );
                    }
                    ViewerEvent::Resync => {
                        let _ = app.emit(&format!("share-viewer://resync/{id}"), ());
                    }
                    ViewerEvent::ControlChanged { mode } => {
                        let _ = app.emit(
                            &format!("share-viewer://control/{id}"),
                            ModePayload { mode },
                        );
                    }
                    ViewerEvent::Ended { reason } => {
                        let _ = app.emit(
                            &format!("share-viewer://ended/{id}"),
                            EndedPayload { reason },
                        );
                        break;
                    }
                }
            }
        });

        Ok(id)
    }

    /// 把按鍵送給對方。唯讀時上層就不該呼叫——伺服器端還有一道授權檢查。
    pub fn send(&self, id: &str, data: Vec<u8>) -> anyhow::Result<()> {
        let conns = self.connections.lock();
        let conn = conns
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("找不到這條觀看連線：{id}"))?;
        conn.keys
            .send(data)
            .map_err(|_| anyhow::anyhow!("連線已結束"))?;
        Ok(())
    }

    /// 關掉一條連線。丟掉 keys sender 會讓串流迴圈的 select 收到 None 而結束。
    pub fn disconnect(&self, id: &str) {
        self.connections.lock().remove(id);
    }
}
```

- [ ] **Step 2: 三個 Tauri command**

建立 `src-tauri/src/commands/share_viewer.rs`：

```rust
//! 觀看端的 Tauri commands。
//!
//! 傳輸跑在 Rust（見 `share::viewer` 的說明），前端只負責發起連線、
//! 送按鍵、訂閱事件。

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::share::viewer_manager::ViewerManager;

#[tauri::command]
pub async fn share_viewer_connect(
    host: String,
    port: u16,
    code: String,
    display_name: String,
    viewers: State<'_, Arc<ViewerManager>>,
    app: AppHandle,
) -> Result<String, String> {
    viewers
        .connect(app, host, port, code, display_name)
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub async fn share_viewer_send(
    conn_id: String,
    data: String,
    viewers: State<'_, Arc<ViewerManager>>,
) -> Result<(), String> {
    viewers
        .send(&conn_id, data.into_bytes())
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub async fn share_viewer_disconnect(
    conn_id: String,
    viewers: State<'_, Arc<ViewerManager>>,
) -> Result<(), String> {
    viewers.disconnect(&conn_id);
    Ok(())
}
```

- [ ] **Step 3: 註冊**

`src-tauri/src/share/mod.rs` 加：

```rust
pub mod viewer_manager;
```

`src-tauri/src/commands/mod.rs` 加（維持字母序，`share` 之後）：

```rust
pub mod share_viewer;
```

`src-tauri/src/lib.rs`：
1. `.manage(...)` 那一串加入：
```rust
        .manage(Arc::new(share::viewer_manager::ViewerManager::new()))
```
2. commands `use` 區塊加入：
```rust
    share_viewer::{share_viewer_connect, share_viewer_disconnect, share_viewer_send},
```
3. `generate_handler!` 加入那三個名字。

- [ ] **Step 4: 確認三個 command 都註冊兩次**

```bash
cd src-tauri && for c in share_viewer_connect share_viewer_send share_viewer_disconnect; do
  printf "%-28s" "$c"; grep -c "\b$c\b" src/lib.rs
done
```

Expected: 每一行都是 `2`。任何一行是 `1` 就代表漏註冊——那個 command 會在前端呼叫時才於執行期失敗。

- [ ] **Step 5: 編譯確認**

Run: `cd src-tauri && cargo test --test share_viewer && cargo test --lib share::`
Expected: `5 passed` 與 `41 passed`。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/share/ src-tauri/src/commands/ src-tauri/src/lib.rs
git commit -m "feat(share): viewer connection manager and Tauri commands

比照 PtyManager：用 id 記住每條連線，事件發送的責任在 manager 而不是
協定層，讓 viewer.rs 能在不起 Tauri app 的情況下測試。"
```

---

## Task 4: 前端 IPC wrapper

**Files:**
- Create: `src/ipc/shareViewer.ts`
- Test: `src/ipc/shareViewer.test.ts`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/ipc/shareViewer.test.ts`：

```ts
import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));

import {
  shareViewerConnect,
  shareViewerSend,
  onShareViewerSas,
  onShareViewerData,
} from "./shareViewer";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  invokeMock.mockResolvedValue("conn-1");
  listenMock.mockResolvedValue(() => {});
});

describe("share viewer IPC", () => {
  it("passes host, port, code and display name when connecting", async () => {
    const id = await shareViewerConnect("192.168.1.33", 47823, "559207", "Bob");
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_connect", {
      host: "192.168.1.33",
      port: 47823,
      code: "559207",
      displayName: "Bob",
    });
    expect(id).toBe("conn-1");
  });

  it("scopes the sas event to the connection id", async () => {
    // 每條連線一組事件名稱，比照本機 PTY 的 `pty://data/{id}`。同時開兩個
    // 遠端分頁時，兩邊的畫面不能混在一起。
    await onShareViewerSas("conn-1", () => {});
    expect(listenMock).toHaveBeenCalledWith(
      "share-viewer://sas/conn-1",
      expect.any(Function),
    );
  });

  it("scopes the data event to the connection id", async () => {
    await onShareViewerData("conn-2", () => {});
    expect(listenMock).toHaveBeenCalledWith(
      "share-viewer://data/conn-2",
      expect.any(Function),
    );
  });

  it("sends keystrokes as a plain string", async () => {
    invokeMock.mockResolvedValue(undefined);
    await shareViewerSend("conn-1", "ls\n");
    expect(invokeMock).toHaveBeenCalledWith("share_viewer_send", {
      connId: "conn-1",
      data: "ls\n",
    });
  });
});
```

- [ ] **Step 2: 跑測試確認會紅**

Run: `npx vitest run src/ipc/shareViewer.test.ts`
Expected: FAIL——`Failed to resolve import "./shareViewer"`。

- [ ] **Step 3: 實作**

建立 `src/ipc/shareViewer.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * 連進別台機器分享出來的終端機。回傳連線 id——後續所有事件都掛在它上面。
 *
 * 傳輸跑在 Rust，不在這裡：要連的是 TLS ＋ 自簽憑證，而 webview 的
 * `new WebSocket("wss://...")` 會拒絕自簽憑證且沒有程式化例外。
 */
export function shareViewerConnect(
  host: string,
  port: number,
  code: string,
  displayName: string,
): Promise<string> {
  return invoke<string>("share_viewer_connect", { host, port, code, displayName });
}

/** 把按鍵送給對方。唯讀時不該呼叫——伺服器端還有一道授權檢查。 */
export function shareViewerSend(connId: string, data: string): Promise<void> {
  return invoke<void>("share_viewer_send", { connId, data });
}

export function shareViewerDisconnect(connId: string): Promise<void> {
  return invoke<void>("share_viewer_disconnect", { connId });
}

/**
 * **這一端算出的 4 位驗證碼，要唸給對方聽。**
 *
 * 跟主控端相反：主控端的碼絕不送到前端（看得到就會照抄而不問對方），
 * 觀看端的碼必須顯示，因為那正是要唸出來的東西。兩邊不對稱是刻意的。
 */
export function onShareViewerSas(
  connId: string,
  cb: (sas: string) => void,
): Promise<UnlistenFn> {
  return listen<{ sas: string }>(`share-viewer://sas/${connId}`, (e) => cb(e.payload.sas));
}

export interface ViewerGranted {
  /** `"read_only"` 或 `"control"`。 */
  mode: string;
  /** 主控端的終端機尺寸——xterm 必須照這個建，不能用自己的視窗大小。 */
  cols: number;
  rows: number;
}

export function onShareViewerGranted(
  connId: string,
  cb: (g: ViewerGranted) => void,
): Promise<UnlistenFn> {
  return listen<ViewerGranted>(`share-viewer://granted/${connId}`, (e) => cb(e.payload));
}

/** 遠端 PTY 畫面，base64 編碼（跟本機 `pty://data` 同樣的形狀）。 */
export function onShareViewerData(
  connId: string,
  cb: (base64: string) => void,
): Promise<UnlistenFn> {
  return listen<{ base64: string }>(`share-viewer://data/${connId}`, (e) =>
    cb(e.payload.base64),
  );
}

/**
 * 落後太多，要清空畫面——下一批 data 是全量重播。
 *
 * 不能忽略：漏掉的位元組可能截斷 ANSI 逃脫序列，帶著壞掉的畫面繼續是不會
 * 自己好的。
 */
export function onShareViewerResync(connId: string, cb: () => void): Promise<UnlistenFn> {
  return listen<unknown>(`share-viewer://resync/${connId}`, () => cb());
}

export function onShareViewerControlChanged(
  connId: string,
  cb: (mode: string) => void,
): Promise<UnlistenFn> {
  return listen<{ mode: string }>(`share-viewer://control/${connId}`, (e) =>
    cb(e.payload.mode),
  );
}

/** 連線結束。`reason` 是 `EndReason` 的 snake_case 字串。 */
export function onShareViewerEnded(
  connId: string,
  cb: (reason: string) => void,
): Promise<UnlistenFn> {
  return listen<{ reason: string }>(`share-viewer://ended/${connId}`, (e) =>
    cb(e.payload.reason),
  );
}
```

- [ ] **Step 4: 跑測試確認轉綠**

Run: `npx vitest run src/ipc/shareViewer.test.ts`
Expected: PASS，4 個測試全過。

- [ ] **Step 5: 型別檢查**

Run: `npx tsc -b`
Expected: 沒有輸出。

**用 `npx tsc -b`，不要用 `tsc --noEmit`**——根 `tsconfig.json` 是 solution file（`"files": []`），`--noEmit` 什麼都不檢查而且永遠 exit 0。

- [ ] **Step 6: Commit**

```bash
git add src/ipc/shareViewer.ts src/ipc/shareViewer.test.ts
git commit -m "feat(share): typed IPC for the viewer side"
```

---

## 完成標準

- [ ] `cd src-tauri && cargo test` 全綠
- [ ] `cargo test --test share_viewer` 連跑 5 次全過（真 PTY ＋ 真 TLS ＋ 真 ws）
- [ ] `npx vitest run src/ipc/shareViewer.test.ts` 全綠、`npx tsc -b` 通過
- [ ] `cargo clippy --lib --tests -- -D warnings 2>&1 | grep -E "src/share/|src/commands/share"` 沒有輸出

  注意：這個 repo 本來就有約 43 個既有 clippy 錯誤（`vcs`、`mcp`、`enterprise`、`pty/cd_parser` 等），**不是這個計畫造成的、不要順手修**。同理 `cargo test --lib` 有一個既有的 flaky 測試（`pty::session::tests::every_subscriber_receives_the_same_output_and_on_data_still_fires`），偶發失敗重跑即可。
- [ ] 三個 viewer command 都在 `lib.rs` 出現兩次（`use` ＋ `generate_handler!`）
- [ ] 觀看端的 SAS **有**送到前端（`share-viewer://sas/{id}` 事件），主控端的 **沒有**——兩邊相反是刻意的

**尚未具備**：任何畫面（2B-2）、mDNS（2C）。2B-1 結束時觀看端能連線並收到畫面，但只有測試看得到。
