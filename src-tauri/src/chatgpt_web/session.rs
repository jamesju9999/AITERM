//! webview 生命週期與請求配對。
//!
//! webview **就是傳輸層本身**：`hide()` 保留它、請求照常；`close()` 銷毀它、
//! 下次請求自動重建。登入狀態不受影響——Tauri 的 webview 資料儲存是應用程式
//! 層級，因此**必須用預設儲存，不可用隔離分割區**。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;

/// 待取用的請求 payload。JS 端以 id 反向拉取。
///
/// 為什麼要反向拉取而不是直接把 payload eval 進去：Claude Code 的 system
/// prompt 動輒 30K 字元，拼進 JS 字串會踩上跳脫與長度限制。
#[derive(Default)]
pub struct PendingMap(Mutex<HashMap<String, String>>);

impl PendingMap {
    pub fn insert(&self, id: String, payload: String) {
        self.0.lock().expect("pending map poisoned").insert(id, payload);
    }

    /// 取出並移除。同一個 id 只能被取用一次——重複取用代表 JS 端有重入問題，
    /// 回 None 讓它明確失敗好過送出重複請求。
    pub fn take(&self, id: &str) -> Option<String> {
        self.0.lock().expect("pending map poisoned").remove(id)
    }
}

pub struct Session {
    app: tauri::AppHandle,
    pending: PendingMap,
    sinks: Mutex<HashMap<String, mpsc::UnboundedSender<String>>>,
}

/// 全域存取點。`AiRouter` 沒有 `AppHandle`，橋接也在另一條路徑上，
/// 兩者都需要同一個 Session，因此在 setup 時初始化一次。
static SESSION: OnceLock<Arc<Session>> = OnceLock::new();

pub fn init(app: tauri::AppHandle) {
    let _ = SESSION.set(Arc::new(Session {
        app,
        pending: PendingMap::default(),
        sinks: Mutex::new(HashMap::new()),
    }));
}

pub fn get() -> Option<Arc<Session>> {
    SESSION.get().cloned()
}

impl Session {
    const WINDOW_LABEL: &'static str = "chatgpt-web";

    /// 確保視窗存在。已存在就沿用（保留登入狀態），不存在才建立。
    /// `visible` 為 true 時顯示出來讓使用者登入。
    pub fn ensure_window(&self, visible: bool) -> Result<tauri::WebviewWindow, tauri::Error> {
        use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
        if let Some(w) = self.app.get_webview_window(Self::WINDOW_LABEL) {
            if visible {
                let _ = w.show();
            }
            return Ok(w);
        }
        let url = "https://chatgpt.com/".parse().expect("static url");
        let w = WebviewWindowBuilder::new(&self.app, Self::WINDOW_LABEL, WebviewUrl::External(url))
            .title("ChatGPT")
            .inner_size(1100.0, 850.0)
            .visible(visible)
            .initialization_script(include_str!("inject.js"))
            .build()?;
        Ok(w)
    }

    /// 送出一個請求，回傳接收 chunk 的通道與一個清理守衛。
    ///
    /// **守衛一定要綁在具名變數上**（`let (mut rx, _guard) = …`）。綁成 `_`
    /// 會當場 drop，sink 立刻被移除，接下來收不到任何 chunk。
    pub fn request(
        self: &Arc<Self>,
        payload: String,
    ) -> Result<(mpsc::UnboundedReceiver<String>, SinkGuard), tauri::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        self.pending.insert(id.clone(), payload);
        self.sinks.lock().expect("sinks poisoned").insert(id.clone(), tx);
        let guard = SinkGuard { session: Arc::clone(self), id: id.clone() };
        let w = self.ensure_window(false)?;
        // 只送 id，payload 由 JS 反向拉取。
        w.eval(format!("window.__aiterm.pull({})", serde_json::json!(id)))?;
        Ok((rx, guard))
    }

    /// 呼叫注入腳本上某個具名函式（模型查詢等一次性用途），不經過 pending map
    /// ——這類請求沒有 payload 要拉取。
    pub fn request_raw(
        self: &Arc<Self>,
        js_fn: &str,
    ) -> Result<(mpsc::UnboundedReceiver<String>, SinkGuard), tauri::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        self.sinks.lock().expect("sinks poisoned").insert(id.clone(), tx);
        let guard = SinkGuard { session: Arc::clone(self), id: id.clone() };
        let w = self.ensure_window(false)?;
        w.eval(format!("window.{js_fn}({})", serde_json::json!(id)))?;
        Ok((rx, guard))
    }

    pub fn take_pending(&self, id: &str) -> Option<String> {
        self.pending.take(id)
    }

    pub fn push_chunk(&self, id: &str, data: String) {
        if let Some(tx) = self.sinks.lock().expect("sinks poisoned").get(id) {
            let _ = tx.send(data);
        }
    }
}

/// 請求結束時把 sink 從 map 移掉。
///
/// 沒有它的話 `sinks` 會永遠握著一份 sender clone，`rx.recv().await` 永遠不會
/// 回 `None`——任何靠「通道關閉」收尾的消費端都會永久卡住（UI 一直轉圈、
/// 沒有錯誤訊息）。用 `Drop` 而不是在每個結束點手動移除，是為了同時涵蓋
/// 錯誤路徑與提早 `return`。
pub struct SinkGuard {
    session: Arc<Session>,
    id: String,
}

impl Drop for SinkGuard {
    fn drop(&mut self) {
        self.session.sinks.lock().expect("sinks poisoned").remove(&self.id);
        // JS 若因故沒來拉取，payload 也不該留著。
        let _ = self.session.pending.take(&self.id);
    }
}

/// 注入腳本反向拉取 payload。
///
/// **必須是 `async`**：Tauri 文件記載，在同步 command 內接觸 webview 相關資源
/// 會在 Windows 上死鎖（wry #583）。
#[tauri::command]
pub async fn chatgpt_web_take(id: String) -> Result<Option<String>, String> {
    Ok(get().and_then(|s| s.take_pending(&id)))
}

/// 注入腳本回傳一段原始 SSE chunk（或一個 JSON 錯誤物件）。
///
/// **必須是 `async`**：理由同上。
#[tauri::command]
pub async fn chatgpt_web_chunk(id: String, data: String) {
    if let Some(s) = get() {
        s.push_chunk(&id, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_returns_payload_once_then_none() {
        let p = PendingMap::default();
        p.insert("id1".into(), "payload".into());
        assert_eq!(p.take("id1"), Some("payload".to_string()));
        assert_eq!(p.take("id1"), None, "同一個 id 不可被取用兩次");
    }

    #[test]
    fn unknown_id_takes_as_none() {
        let p = PendingMap::default();
        assert_eq!(p.take("沒看過的"), None);
    }
}
