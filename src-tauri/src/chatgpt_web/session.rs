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
                // 腳本可能還沒注入完（視窗剛建立），所以要防呆取用。
                // watchLogin 自己有去重，重複 eval 不會疊出多個輪詢器。
                let _ = w.eval("window.__aiterm && window.__aiterm.watchLogin()");
            }
            return Ok(w);
        }
        let url = "https://chatgpt.com/".parse().expect("static url");
        let w = WebviewWindowBuilder::new(&self.app, Self::WINDOW_LABEL, WebviewUrl::External(url))
            .title("ChatGPT")
            .inner_size(1100.0, 850.0)
            .visible(visible)
            .initialization_script(include_str!("inject.js"))
            // 視窗剛建立時頁面還沒載入，上面那個分支的 eval 會落空（`window.__aiterm`
            // 還不存在）。登入輪詢要掛在這裡才啟動得了。
            //
            // 每次導覽都會觸發（登入流程本身就是好幾次導覽），而導覽會換掉 JS
            // context、`loginWatcher` 跟著重置——所以重新啟動正是我們要的，
            // 不會疊加。只在視窗可見時啟動：隱藏的傳輸層不需要輪詢。
            .on_page_load(|w, _| {
                if w.is_visible().unwrap_or(false) {
                    let _ = w.eval("window.__aiterm && window.__aiterm.watchLogin()");
                }
            })
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

    /// 呼叫 `window.__aiterm` 上某個具名函式（模型查詢等一次性用途），不經過
    /// pending map——這類請求沒有 payload 要拉取。
    ///
    /// 名稱固定在 `__aiterm` 命名空間下，不接受任意的全域名：整個注入腳本只有
    /// 兩個掛載點（`__aiterm` 與 `__aitermTest`），多一個頂層全域就多一個會被
    /// `pickKey(window)` 抽中送給 OpenAI 的自動化標記。
    pub fn request_raw(
        self: &Arc<Self>,
        js_fn: &str,
    ) -> Result<(mpsc::UnboundedReceiver<String>, SinkGuard), tauri::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        self.sinks.lock().expect("sinks poisoned").insert(id.clone(), tx);
        let guard = SinkGuard { session: Arc::clone(self), id: id.clone() };
        let w = self.ensure_window(false)?;
        w.eval(format!(
            "window.__aiterm.{js_fn}({})",
            serde_json::json!(id)
        ))?;
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

/// 注入腳本偵測到登入完成時呼叫。收起視窗即可——傳輸層繼續存活，登入狀態
/// 存在應用程式層級的 webview 儲存裡，不會因為隱藏而失效。
///
/// **必須是 `async`**：理由同上。
#[tauri::command]
pub async fn chatgpt_web_logged_in() {
    if let Some(s) = get() {
        use tauri::Manager;
        if let Some(w) = s.app.get_webview_window(Session::WINDOW_LABEL) {
            let _ = w.hide();
        }
    }
}

/// 設定頁的模型下拉選單用的一筆。
#[derive(serde::Serialize)]
pub struct ChatgptWebModel {
    pub slug: String,
    pub title: String,
    pub max_tokens: u32,
}

/// 等注入腳本回應的上限。
///
/// 沒有它的話這個 command 會永遠等下去：`SinkGuard` 在函式結束才 drop，所以
/// 在 `recv().await` 期間通道永遠不會關。注入腳本的每條失敗路徑都會回報，
/// 但「eval 根本沒送達」（視窗剛被關掉、頁面正在導向）不在它的掌控範圍內。
const MODELS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 設定頁用：取回該帳號可用的模型清單。
///
/// `/backend-api/models` 回的是「該帳號實際可用」的清單，所以不需要維護方案與
/// 模型的對應表——登入哪個帳號就顯示什麼。
#[tauri::command]
pub async fn chatgpt_web_models() -> Result<Vec<ChatgptWebModel>, String> {
    let s = get().ok_or("session 未初始化")?;
    let (mut rx, _guard) = s.request_raw("models").map_err(|e| e.to_string())?;
    let body = tokio::time::timeout(MODELS_TIMEOUT, rx.recv())
        .await
        .map_err(|_| "等待 ChatGPT 網頁回應逾時".to_string())?
        .ok_or("沒有回應")?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return Err(err.to_string());
    }
    Ok(v.get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    Some(ChatgptWebModel {
                        slug: m.get("slug")?.as_str()?.to_string(),
                        title: m
                            .get("title")
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string(),
                        max_tokens: m.get("max_tokens").and_then(|t| t.as_u64()).unwrap_or(0)
                            as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default())
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
