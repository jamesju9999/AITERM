//! WebSocket 推播：`{"type":"changed","seq":N}`。Client 收到就重抓清單。
use crate::members::{self, Me};
use crate::state::AppState;
use aiterm_intake_core::api::Event;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::Extension;
use std::sync::Arc;
use std::time::Duration;

/// 多久檢查一次連線的成員是不是還有效；撤銷後最慢這麼久就斷線。
pub const REVOCATION_CHECK: Duration = Duration::from_secs(2);

pub async fn events(State(st): State<Arc<AppState>>, Extension(me): Extension<Me>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| run(st, me, socket))
}

async fn send(socket: &mut WebSocket, seq: u64) -> bool {
    let Ok(text) = serde_json::to_string(&Event::changed(seq)) else { return false };
    socket.send(Message::Text(text.into())).await.is_ok()
}

async fn run(st: Arc<AppState>, me: Me, mut socket: WebSocket) {
    let mut rx = st.events.subscribe();
    // 連上先送目前的 seq：Client 斷線期間錯過的變動，靠這一則觸發重抓。
    if !send(&mut socket, st.events.seq()).await {
        return;
    }
    let mut tick = tokio::time::interval(REVOCATION_CHECK);
    loop {
        tokio::select! {
            got = rx.recv() => match got {
                Ok(seq) => if !send(&mut socket, seq).await { return },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => if !send(&mut socket, st.events.seq()).await { return },
                Err(_) => return,
            },
            _ = tick.tick() => {
                if !members::is_active(&st.pools.server, &me.id).await {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            }
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            },
        }
    }
}
