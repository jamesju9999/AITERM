//! 即時預覽與遠端操作的連線：桌面版開一條到 server 的 WebSocket（釘住憑證指紋），
//! server 傳來的訊息原樣用事件推給前端（`live://msg/<handle>`），前端的輸入用 `send` 轉過去。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, Notify};

use aiterm_intake_client::api::Api;

/// 一條連線：輸出訊息的通道與背景工作。
struct Conn {
    tx: mpsc::UnboundedSender<String>,
    task: tokio::task::JoinHandle<()>,
    /// 前端註冊好監聽之後才開始轉送訊息：事件在前端訂閱之前送出就永遠遺失（這個 repo 踩過兩次）。
    ready: Arc<Notify>,
}

#[derive(Default)]
pub struct LiveState {
    next: AtomicU64,
    conns: Mutex<HashMap<u64, Conn>>,
}

impl LiveState {
    /// 連上 `path`，回傳 handle。之後 server 的每一則文字訊息都會 emit `live://msg/<handle>`，連線結束 emit `live://end/<handle>`。
    pub async fn open(&self, api: &Api, path: &str, app: AppHandle) -> Result<u64, String> {
        let mut ws = api.ws(path).await.map_err(|e| e.to_string())?;
        let handle = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let (msg_ev, end_ev) = (format!("live://msg/{handle}"), format!("live://end/{handle}"));
        let ready = Arc::new(Notify::new());
        let wait = ready.clone();
        let task = tokio::spawn(async move {
            wait.notified().await;
            loop {
                tokio::select! {
                    got = ws.next_text() => match got {
                        Some(t) => { let _ = app.emit(&msg_ev, t); }
                        None => break,
                    },
                    out = rx.recv() => match out {
                        Some(t) => { if ws.send_text(&t).await.is_err() { break } }
                        None => break,
                    },
                }
            }
            ws.close().await;
            let _ = app.emit(&end_ev, ());
        });
        self.conns.lock().insert(handle, Conn { tx, task, ready });
        Ok(handle)
    }

    /// 前端已經註冊好監聽：開始轉送訊息。
    pub fn start(&self, handle: u64) {
        if let Some(c) = self.conns.lock().get(&handle) {
            c.ready.notify_one();
        }
    }

    pub fn send(&self, handle: u64, text: String) -> Result<(), String> {
        let conns = self.conns.lock();
        let c = conns.get(&handle).ok_or("這條連線已經結束了")?;
        c.tx.send(text).map_err(|_| "這條連線已經結束了".to_string())
    }

    pub fn close(&self, handle: u64) {
        if let Some(c) = self.conns.lock().remove(&handle) {
            c.task.abort();
        }
    }
}
