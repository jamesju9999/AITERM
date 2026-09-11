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
    /// 前端訂閱好之後用這個放行事件 pump。見 `connect` 裡的說明。
    ///
    /// `Option` 是因為只能放行一次：`mark_ready` 取走它，重複呼叫是 no-op。
    ready: Option<tokio::sync::oneshot::Sender<()>>,
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
        key: Option<String>,
    ) -> anyhow::Result<Connected> {
        let key_bytes = match key {
            Some(k) => Some(
                aiterm_core::share::tls::decode_hex(&k)
                    .ok_or_else(|| anyhow::anyhow!("金鑰不是合法的 hex"))?,
            ),
            None => None,
        };
        let handshake =
            connect_and_handshake(&host, port, &code, &display_name, key_bytes.as_deref()).await?;
        let id = Uuid::new_v4().to_string();
        let sas = handshake.sas;

        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel::<ViewerEvent>();
        let (keys_tx, keys_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

        self.connections
            .lock()
            .insert(id.clone(), Connection { keys: keys_tx, ready: Some(ready_tx) });

        tokio::spawn(run_viewer_stream(
            handshake.ws,
            events_tx,
            keys_rx,
            handshake.key,
            handshake.auth_exporter,
        ));

        let id_for_pump = id.clone();
        tokio::spawn(async move {
            // **等前端說「我在聽了」才開始送事件。**
            //
            // 這不是保險，是修一個實機抓到的 bug：金鑰模式（CLI host）的核准
            // 是瞬間的，所以 `Granted` 連同它後面那批畫面重播，會在前端拿到
            // conn_id、開好分頁、元件掛載並訂閱**之前**就發出去。Tauri 事件
            // 不重播，那些事件直接消失，畫面永遠停在「等待對方同意」。
            //
            // 短碼模式踩不到只是因為人要花好幾秒才按同意——同一個 race 一直
            // 都在，只是被人類的反應時間蓋住了。這個檔案開頭關於 SAS 為什麼
            // 走回傳值而不走事件的註解，講的就是同一件事。
            //
            // 事件不會因為等待而遺失：`events_rx` 是 unbounded channel，
            // 在這裡等的期間它會把東西排好。
            //
            // `Err` 代表 sender 被丟掉了——連線在前端還沒準備好之前就結束。
            // 那就什麼都不用送了。
            if ready_rx.await.is_err() {
                return;
            }
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

    /// 前端已經訂閱好所有事件，可以開始送了。
    ///
    /// 找不到這條連線、或已經放行過，都是安靜的 no-op——前端重複呼叫（例如
    /// effect 重跑）不該變成錯誤。
    pub fn mark_ready(&self, id: &str) {
        let mut conns = self.connections.lock();
        let Some(conn) = conns.get_mut(id) else { return };
        if let Some(tx) = conn.ready.take() {
            let _ = tx.send(());
        }
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
