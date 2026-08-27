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

/// `connect` 的回傳值。
///
/// **SAS 跟著回傳值走，不走事件**——見 `ViewerManager::connect` 的說明。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Connected {
    pub conn_id: String,
    /// 這一端算出的 4 位驗證碼，**要唸給對方聽**。
    ///
    /// 跟主控端相反：那邊的碼絕不送到前端（看得到就會照抄而不問對方）。
    /// 兩邊不對稱是這個設計能成立的原因。
    pub sas: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GrantedPayload {
    mode: String,
    cols: u16,
    rows: u16,
    host_os: String,
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

    /// 連線並開始串流。回傳連線 id 與**這一端算出的 4 位驗證碼**。
    ///
    /// 握手完成（但對方尚未裁決）時就回傳。
    ///
    /// **SAS 用回傳值而不是事件**：它在這裡就已經算出來了，而訂閱者要等
    /// 前端拿到 id、開好分頁、元件掛載之後才存在——用事件送必然遺失，
    /// 因為發出的時候還沒有人在聽。實機測試就是這樣抓到的（觀看端的
    /// 驗證碼永遠是空的）。回傳值沒有這個時間差。
    pub async fn connect(
        &self,
        app: AppHandle,
        host: String,
        port: u16,
        code: String,
        display_name: String,
    ) -> anyhow::Result<Connected> {
        let handshake = connect_and_handshake(&host, port, &code, &display_name).await?;
        let id = Uuid::new_v4().to_string();
        let sas = handshake.sas;

        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel::<ViewerEvent>();
        let (keys_tx, keys_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

        self.connections.lock().insert(id.clone(), Connection { keys: keys_tx });

        tokio::spawn(run_viewer_stream(handshake.ws, events_tx, keys_rx));

        let id_for_pump = id.clone();
        tokio::spawn(async move {
            while let Some(ev) = events_rx.recv().await {
                let id = &id_for_pump;
                match ev {
                    ViewerEvent::Granted { mode, cols, rows, host_os } => {
                        let _ = app.emit(
                            &format!("share-viewer://granted/{id}"),
                            GrantedPayload { mode, cols, rows, host_os },
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

        Ok(Connected { conn_id: id, sas })
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
