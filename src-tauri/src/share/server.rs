//! 共享連線的 axum router 與 ws handler。
//!
//! 每條 ws 連線對應一位觀看者。連線後的握手順序是固定的：
//! `Join` → `SasCommit`（主控端先承諾自己的 nonce）→ `SasNonce`（觀看端送出
//! 自己的 nonce）→ `AwaitingApproval`（主控端揭曉 nonce）→ （主控端裁決）→
//! `Granted` ＋ 重播 ＋ 串流，或 `Ended`。承諾必須先於揭曉——這是防中間人
//! 保證的關鍵（見 `share::tls` 的說明）。在 `Granted` 之前，觀看端拿不到任何
//! 一個 PTY 位元組。

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::{any, get};
use axum::Router;
use tokio::sync::broadcast::error::RecvError;

use crate::pty::manager::PtyManager;

use super::protocol::{
    ClientMessage, ConnectionExporter, EndReason, PendingRequestEvent, ServerMessage,
    WireAccessMode, PROTOCOL_VERSION,
};
use super::registry::ShareRegistry;
use super::tls;

/// 重播給新連線觀看者的位元組上限。
///
/// 直接取 PTY ring buffer 的容量，不另外寫一個數字：重播的意義就是「把 ring
/// 裡有的全部給他」，兩邊各寫一份 `256 * 1024` 遲早會漂掉——改了 ring 卻忘了
/// 改這裡，症狀會是觀看端悄悄少拿到一段歷史，而且沒有任何東西會報錯。
const REPLAY_MAX_BYTES: usize = crate::pty::session::OUTPUT_RING_CAP;

/// 輪詢主控端裁決與分享是否被停掉的間隔。裁決由人操作，秒級足夠；用輪詢而
/// 不是再開一條通知管道，是因為狀態機刻意不依賴任何非同步執行期。
const DECISION_POLL: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub struct ShareAppState {
    pub pty: Arc<PtyManager>,
    pub registry: Arc<ShareRegistry>,
    /// 用來把「有人要連進來」推播給前端。整合測試不起 Tauri app，所以是
    /// `Option`——`None` 時所有事件發送都是 no-op，其餘行為完全一樣。
    pub app: Option<tauri::AppHandle>,
}

pub fn router(
    pty: Arc<PtyManager>,
    registry: Arc<ShareRegistry>,
    app: Option<tauri::AppHandle>,
) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/share", any(share_upgrade))
        .with_state(ShareAppState { pty, registry, app })
}

/// TLS exporter material 由 request extension 帶進來——Task 8 的 TLS accept
/// 迴圈填真值，測試裡注入佔位值。**刻意用 `Extension<ConnectionExporter>`
/// 而不是 `Option<...>`**：沒有它時應該直接 500 而不是無聲降級成「沒有身分
/// 保證的連線」。
async fn share_upgrade(
    ws: WebSocketUpgrade,
    axum::Extension(exporter): axum::Extension<ConnectionExporter>,
    State(state): State<ShareAppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_share(socket, state, exporter.0))
}

async fn send_control(ws: &mut WebSocket, msg: &ServerMessage) -> bool {
    let Ok(json) = serde_json::to_string(msg) else { return false };
    ws.send(Message::Text(json.into())).await.is_ok()
}

async fn end_with(ws: &mut WebSocket, reason: EndReason) {
    send_control(ws, &ServerMessage::Ended { reason }).await;
    // axum 的 `WebSocket` 只有 `recv`/`send`——沒有 `close()`。要關閉就送一個
    // Close frame（`Sink::close` 也在 trait 上，但那需要額外 import
    // `futures_util::SinkExt`，送 frame 更直接）。
    let _ = ws.send(Message::Close(None)).await;
}

async fn handle_share(
    mut ws: WebSocket,
    state: ShareAppState,
    exporter: [u8; tls::SAS_MATERIAL_LEN],
) {
    // 1. 第一則訊息必須是 Join。
    let join = match ws.recv().await {
        Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientMessage>(&t) {
            Ok(ClientMessage::Join { protocol_version, code, display_name }) => {
                (protocol_version, code, display_name)
            }
            Ok(_) | Err(_) => return end_with(&mut ws, EndReason::InvalidCode).await,
        },
        _ => return end_with(&mut ws, EndReason::InvalidCode).await,
    };
    let (protocol_version, code, display_name) = join;

    // 版本落差在握手第一步就擋掉。不這樣做的話，兩端版本不同時的症狀會是
    // 後續某則訊息解析失敗、連線莫名其妙斷掉——而「同事的 AITerm 沒更新」
    // 在區網分享裡是常態，不是邊角案例。
    if protocol_version != PROTOCOL_VERSION {
        return end_with(&mut ws, EndReason::VersionMismatch).await;
    }

    // SAS 承諾流程。**這兩步的順序就是整個防中間人保證的全部價值，不可調換。**
    //
    // 主控端必須在知道觀看端的 nonce 之前就把自己的承諾送出去。否則中間人
    // （對觀看端扮演 TLS server，而 TLS 1.3 的 server 是後手）可以在看到觀看端
    // 的貢獻之後，才在本機反覆試算自己的 nonce，湊出跟另一條連線相同的 4 位數
    // ——搜尋空間只有 10⁴，約一秒就完成。承諾把它的 nonce 提前鎖死，這條路就
    // 斷了。結構取自 ZRTP／RFC 6189。
    //
    // 換更強的雜湊救不了這件事（輸出空間不變）；順序才是關鍵。
    let host_nonce = tls::fresh_nonce();
    if !send_control(
        &mut ws,
        &ServerMessage::SasCommit { commit: tls::commit_for(&host_nonce) },
    )
    .await
    {
        return;
    }

    let viewer_nonce = match ws.recv().await {
        Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientMessage>(&t) {
            Ok(ClientMessage::SasNonce { nonce }) => match tls::decode_hex(&nonce) {
                Some(n) => n,
                None => return end_with(&mut ws, EndReason::SasHandshakeFailed).await,
            },
            _ => return end_with(&mut ws, EndReason::SasHandshakeFailed).await,
        },
        _ => return end_with(&mut ws, EndReason::SasHandshakeFailed).await,
    };

    // 這條連線的 SAS。主控端把它存進 PendingRequest 給同意視窗顯示；
    // 觀看端會用同樣三份材料自己算一份。
    let sas = tls::sas_from_parts(&host_nonce, &viewer_nonce, exporter.as_slice());

    // 2. 短碼換待審請求。短碼無效就到此為止——主控端不會看到任何東西，所以
    //    亂猜短碼連「打擾對方」都做不到。
    let display_name_for_event = display_name.clone();
    let Some(request_id) = state.registry.request_join(&code, display_name, sas.clone()) else {
        return end_with(&mut ws, EndReason::InvalidCode).await;
    };
    let Some(tab_id) = state.registry.tab_for_code(&code) else {
        // `request_join` 成功了，但短碼在這兩行之間失效（主控端剛好停止分享）。
        // 必須把剛建立的待審請求收掉——`stop_share` 的清理早就跑過了，沒有人
        // 會再來清它，留著就是一筆永遠掛在 registry 裡的孤兒。
        state.registry.deny(&request_id);
        return end_with(&mut ws, EndReason::InvalidCode).await;
    };

    // 推播給前端，讓同意視窗跳出來。`None` 時（整合測試）是 no-op。
    if let Some(app) = &state.app {
        use tauri::Emitter;
        let _ = app.emit(
            "share://request-pending",
            PendingRequestEvent {
                request_id: request_id.clone(),
                tab_id: tab_id.clone(),
                display_name: display_name_for_event.clone(),
            },
        );
    }

    if !send_control(
        &mut ws,
        &ServerMessage::AwaitingApproval { host_nonce: tls::hex_of(&host_nonce) },
    )
    .await
    {
        state.registry.deny(&request_id);
        return;
    }

    // 3. 等主控端裁決。刻意不設自動拒絕的逾時——使用者可能只是走開了，自動
    //    拒絕會讓他回來時毫無線索（見 spec 的錯誤處理）。觀看端要放棄的話
    //    自己關掉連線，下面的 select 會偵測到並收掉那筆待審請求。
    let viewer_id = loop {
        // 分享在裁決前被停掉。
        if state.registry.tab_for_code(&code).is_none() {
            state.registry.deny(&request_id);
            return end_with(&mut ws, EndReason::HostStoppedSharing).await;
        }
        // 已經不在待審名單裡：不是被核准（下面查得到 viewer）就是被拒絕。
        let still_pending = state
            .registry
            .pending(&tab_id)
            .iter()
            .any(|p| p.request_id == request_id);
        if !still_pending {
            break match state.registry.viewer_for_request(&tab_id, &request_id) {
                Some(id) => id,
                None => return end_with(&mut ws, EndReason::Denied).await,
            };
        }
        // 等一個輪詢間隔，但同時盯著 ws——觀看端可能等不下去自己關掉了。
        //
        // 沒有這個 select 的話（原本就沒有），放棄的連線會讓這個迴圈**永遠
        // 空轉**，而那筆待審請求會一直掛在主控端的同意視窗上，因為沒有任何
        // 人會來清掉它。刻意不設逾時是為了不讓「使用者走開」變成自動拒絕，
        // 不是為了對觀看端已經離線這件事視而不見。
        //
        // `ws.recv()` 是 cancel-safe 的（底層 tokio-tungstenite 把未完成的
        // frame 狀態存在 stream 裡而不是 future 裡），所以放進 select 不會
        // 因為被取消而漏掉半個訊息。
        tokio::select! {
            _ = tokio::time::sleep(DECISION_POLL) => {}
            incoming = ws.recv() => {
                match incoming {
                    // 串流結束、連線壞掉、或收到 Close frame——對方走了。
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {
                        state.registry.deny(&request_id);
                        return;
                    }
                    // 協定規定觀看端在 `Granted` 之前不送任何東西。收到別的
                    // 訊息就忽略，繼續等裁決。
                    Some(Ok(_)) => {}
                }
            }
        }
    };

    // 4. 已獲准：先送尺寸與模式，再送重播，最後接即時串流。
    let mode: WireAccessMode = state
        .registry
        .viewers(&tab_id)
        .into_iter()
        .find(|v| v.viewer_id == viewer_id)
        .map(|v| v.mode.into())
        .unwrap_or(WireAccessMode::ReadOnly);
    let (cols, rows) = state.pty.size(&tab_id).unwrap_or((80, 24));

    // 記住最後一次告訴觀看端的存取層級，之後靠它偵測變化（見下方 watch tick）。
    let mut announced_mode = mode;

    if !send_control(&mut ws, &ServerMessage::Granted { mode, cols, rows }).await {
        state.registry.remove_viewer(&tab_id, &viewer_id);
        return;
    }

    // **一定要用 `subscribe_with_history`，不要分開呼叫 `subscribe` 與
    // `get_recent_raw`。** 那兩支用的是不同的鎖，不論哪個順序都會留下窗口：
    // 先訂閱會讓中間的 chunk 重複，先取快照會讓它整段消失——而消失可能截斷
    // 一段 ANSI 逃脫序列，畫面從此錯亂且不會自己好。見 Task 3b。
    let Some((history, mut rx)) =
        state.pty.subscribe_with_history(&tab_id, REPLAY_MAX_BYTES)
    else {
        state.registry.remove_viewer(&tab_id, &viewer_id);
        return end_with(&mut ws, EndReason::SessionClosed).await;
    };

    if let Some(history) = history {
        if ws.send(Message::Binary(history.into())).await.is_err() {
            state.registry.remove_viewer(&tab_id, &viewer_id);
            return;
        }
    }

    // 5. 串流迴圈：PTY 輸出下行、按鍵上行、分享狀態監看，三者併行。
    let mut share_watch = tokio::time::interval(DECISION_POLL);
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Ok(chunk) => {
                    if ws.send(Message::Binary(chunk.into())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    // 漏掉的位元組可能截斷 ANSI 逃脫序列——不能當沒事發生。
                    // 叫觀看端清空畫面，重新給他全量重播。
                    //
                    // 重新同步同樣要用 `subscribe_with_history`：這裡的窗口
                    // 跟首次連線時一模一樣，用 `get_recent_raw` 補快照而讓
                    // 舊的 rx 繼續收，中間的 chunk 一樣會重複或消失。取得
                    // 新的 receiver 後直接換掉舊的。
                    if !send_control(&mut ws, &ServerMessage::Resync).await {
                        break;
                    }
                    let Some((history, fresh_rx)) =
                        state.pty.subscribe_with_history(&tab_id, REPLAY_MAX_BYTES)
                    else {
                        end_with(&mut ws, EndReason::SessionClosed).await;
                        break;
                    };
                    rx = fresh_rx;
                    if let Some(history) = history {
                        if ws.send(Message::Binary(history.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Err(RecvError::Closed) => {
                    end_with(&mut ws, EndReason::SessionClosed).await;
                    break;
                }
            },
            incoming = ws.recv() => match incoming {
                Some(Ok(Message::Binary(keys))) => {
                    // 伺服器端授權檢查。唯讀端理應根本不送，但那是對方程式的
                    // 自律，不是安全邊界。
                    if state.registry.may_send_input(&tab_id, &viewer_id) {
                        let _ = state.pty.write(&tab_id, &keys);
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break,
            },
            _ = share_watch.tick() => {
                if state.registry.tab_for_code(&code).is_none() {
                    end_with(&mut ws, EndReason::HostStoppedSharing).await;
                    break;
                }
                let me = state
                    .registry
                    .viewers(&tab_id)
                    .into_iter()
                    .find(|v| v.viewer_id == viewer_id);
                let Some(me) = me else {
                    end_with(&mut ws, EndReason::KickedByHost).await;
                    break;
                };
                // 主控端可能在這期間收回或授予了控制權（`revoke_control` /
                // `grant_control`）。觀看端必須知道自己現在能不能打字——否則
                // 唯讀的人會白打一堆字進黑洞，剛拿到控制權的人則不知道可以動。
                // 用同一個輪詢偵測，跟上面的踢人偵測同一個機制。
                let current: WireAccessMode = me.mode.into();
                if current != announced_mode {
                    announced_mode = current;
                    if !send_control(
                        &mut ws,
                        &ServerMessage::ControlChanged { mode: current },
                    )
                    .await
                    {
                        break;
                    }
                }
            }
        }
    }

    state.registry.remove_viewer(&tab_id, &viewer_id);

    // 觀看者清單變了，讓主控端的面板重新抓一次。這個事件不帶內容——前端
    // 收到就去 `share_viewers` 重讀，避免兩份資料對不上。
    if let Some(app) = &state.app {
        use tauri::Emitter;
        let _ = app.emit("share://viewers-changed", ());
    }
}
