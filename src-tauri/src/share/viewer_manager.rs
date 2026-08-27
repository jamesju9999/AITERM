//! 觀看連線的生命週期管理。比照 `PtyManager`：用 id 記住每條連線，
//! 提供 write / close。
//!
//! 跟 `PtyManager` 一樣把事件發送的責任放在這裡（而不是 `viewer.rs`），
//! 讓協定那一層能在不起 Tauri app 的情況下測試。

use std::collections::HashMap;

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
