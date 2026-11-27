//! 即時預覽與遠端操作：用 CDP 把一個瀏覽器分頁的畫面（`Page.startScreencast`）串流出去，並把滑鼠鍵盤轉成 `Input.dispatch*`。
//! 桌面版與 server 共用同一份訊息格式；座標用正規化（0～1），由這裡依最近一張畫面的實際尺寸換算。

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::cdp::Cdp;

/// server → client。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    /// 一張畫面（base64 的 JPEG）與它的像素尺寸。
    Frame { data: String, w: u32, h: u32 },
    /// 狀態：Agent 執行中是否被接手、目前網址、這條連線能不能操作、種類（`agent`｜`login`）。
    State { manual: bool, url: String, can_operate: bool, mode: String },
    Closed { why: String },
    Error { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseKind {
    Move,
    Down,
    Up,
    Wheel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Down,
    Up,
}

/// client → server（只有管理員的會被執行）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Mouse {
        #[serde(rename = "type")]
        kind: MouseKind,
        x: f64,
        y: f64,
        #[serde(default)]
        button: Option<String>,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    Key {
        #[serde(rename = "type")]
        kind: KeyKind,
        key: String,
        #[serde(default)]
        code: String,
        /// 這個按鍵會輸入的字元（可輸入的字元才有；Enter 是 `\r`）。
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        vk: i64,
        /// 修飾鍵位元（CDP 的定義）：Alt=1、Ctrl=2、Meta=4、Shift=8。
        #[serde(default)]
        mods: i64,
    },
    /// 直接輸入一段文字（輸入法組好的字、貼上）。
    Text { text: String },
    /// Agent 執行中：接手或交還。
    Takeover { on: bool },
    /// 遠端登入：完成，存 session。
    Done,
    /// 上一頁。
    Back,
}

/// 最近一張畫面的 CSS 像素尺寸（座標換算用）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameMeta {
    pub w: f64,
    pub h: f64,
}

impl Default for FrameMeta {
    fn default() -> Self {
        Self { w: 1280.0, h: 800.0 }
    }
}

fn clamp01(v: f64) -> f64 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

fn button_name(b: &Option<String>) -> &'static str {
    match b.as_deref() {
        Some("right") => "right",
        Some("middle") => "middle",
        Some("none") => "none",
        _ => "left",
    }
}

fn buttons_mask(name: &str) -> i64 {
    match name {
        "left" => 1,
        "right" => 2,
        "middle" => 4,
        _ => 0,
    }
}

/// 滑鼠訊息 → `Input.dispatchMouseEvent` 的參數；不是滑鼠訊息回 `None`。
pub fn mouse_params(msg: &ClientMsg, meta: FrameMeta) -> Option<Value> {
    let ClientMsg::Mouse { kind, x, y, button, dx, dy } = msg else { return None };
    let (px, py) = (clamp01(*x) * meta.w, clamp01(*y) * meta.h);
    let name = button_name(button);
    Some(match kind {
        MouseKind::Move => json!({ "type": "mouseMoved", "x": px, "y": py, "button": "none", "buttons": 0 }),
        MouseKind::Down => json!({ "type": "mousePressed", "x": px, "y": py, "button": name, "buttons": buttons_mask(name), "clickCount": 1 }),
        MouseKind::Up => json!({ "type": "mouseReleased", "x": px, "y": py, "button": name, "buttons": 0, "clickCount": 1 }),
        MouseKind::Wheel => json!({ "type": "mouseWheel", "x": px, "y": py, "deltaX": if dx.is_finite() { *dx } else { 0.0 }, "deltaY": if dy.is_finite() { *dy } else { 0.0 } }),
    })
}

/// 鍵盤訊息 → `Input.dispatchKeyEvent` 的參數。能輸入字元的按下用 `keyDown`（帶 `text`），其餘（方向鍵、Backspace…）用 `rawKeyDown`。
pub fn key_params(msg: &ClientMsg) -> Option<Value> {
    let ClientMsg::Key { kind, key, code, text, vk, mods } = msg else { return None };
    // 只收單一字元、而且不是控制字元（Enter 的 `\r` 例外）。
    let text = text.as_deref().filter(|t| t.chars().count() == 1 && t.chars().all(|c| !c.is_control() || c == '\r'));
    Some(match kind {
        KeyKind::Down => {
            let mut p = json!({ "type": if text.is_some() { "keyDown" } else { "rawKeyDown" }, "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk, "modifiers": mods });
            if let Some(t) = text {
                p["text"] = json!(t);
                p["unmodifiedText"] = json!(t);
            }
            p
        }
        KeyKind::Up => json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk, "modifiers": mods }),
    })
}

/// 一張畫面。
#[derive(Debug, Clone)]
pub struct Frame {
    pub jpeg_b64: String,
    pub w: u32,
    pub h: u32,
}

fn pick_page(targets: &[Value]) -> Option<String> {
    let pages: Vec<&Value> = targets.iter().filter(|t| t["type"] == "page" && !t["url"].as_str().unwrap_or("").starts_with("devtools://")).collect();
    // 偏好有內容的分頁（不是空白頁）；都空白就用第一個。
    pages
        .iter()
        .rev()
        .find(|t| !matches!(t["url"].as_str().unwrap_or(""), "" | "about:blank"))
        .or_else(|| pages.first())
        .and_then(|t| t["targetId"].as_str().map(String::from))
}

/// 一個正在串流的瀏覽器：畫面從 `start` 回傳的接收端出來；輸入用 `mouse`／`key`／`text`。
pub struct Live {
    cdp: Arc<Cdp>,
    session: Arc<Mutex<Option<String>>>,
    meta: Arc<Mutex<FrameMeta>>,
    url: Arc<Mutex<String>>,
    task: tokio::task::JoinHandle<()>,
}

impl Live {
    /// 連上瀏覽器（`ws_url` 是瀏覽器層級的 CDP 位址）、開始串流。`viewport` 指定就強制那個大小（遠端登入用）；
    /// Agent 執行中由 Playwright 管視窗大小，傳 `None`。
    pub async fn start(ws_url: &str, viewport: Option<(u32, u32)>) -> Result<(Arc<Live>, mpsc::UnboundedReceiver<Frame>), String> {
        let (cdp, mut events) = Cdp::connect(ws_url).await?;
        cdp.call("Target.setDiscoverTargets", json!({ "discover": true }), None).await?;
        let (frame_tx, frame_rx) = mpsc::unbounded_channel::<Frame>();
        let session: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let meta = Arc::new(Mutex::new(FrameMeta::default()));
        let url = Arc::new(Mutex::new(String::new()));

        // 找一個頁面（剛開的瀏覽器可能還沒有，最多等 5 秒）。
        let mut target = None;
        for _ in 0..25 {
            let t = cdp.call("Target.getTargets", json!({}), None).await?;
            if let Some(id) = pick_page(t["targetInfos"].as_array().map(Vec::as_slice).unwrap_or(&[])) {
                target = Some(id);
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let target = target.ok_or("瀏覽器裡找不到可以預覽的分頁")?;
        attach(&cdp, &target, &session, viewport).await?;

        let (cdp2, session2, meta2, url2) = (cdp.clone(), session.clone(), meta.clone(), url.clone());
        let task = tokio::spawn(async move {
            let mut current = target;
            while let Some(ev) = events.recv().await {
                let method = ev["method"].as_str().unwrap_or("");
                match method {
                    "Page.screencastFrame" => {
                        let p = &ev["params"];
                        let (Some(data), Some(ack)) = (p["data"].as_str(), p["sessionId"].as_i64()) else { continue };
                        let (w, h) = (p["metadata"]["deviceWidth"].as_f64().unwrap_or(1280.0), p["metadata"]["deviceHeight"].as_f64().unwrap_or(800.0));
                        *meta2.lock() = FrameMeta { w, h };
                        let sid = session2.lock().clone();
                        cdp2.fire("Page.screencastFrameAck", json!({ "sessionId": ack }), sid.as_deref());
                        if frame_tx.send(Frame { jpeg_b64: data.to_string(), w: w as u32, h: h as u32 }).is_err() {
                            break;
                        }
                    }
                    // 新開的分頁（彈出視窗）或目前分頁被關了：切到最合適的那個。
                    "Target.targetCreated" | "Target.targetDestroyed" => {
                        let gone = method == "Target.targetDestroyed" && ev["params"]["targetId"] == json!(current);
                        let created = method == "Target.targetCreated" && ev["params"]["targetInfo"]["type"] == "page";
                        if gone || created {
                            if let Ok(t) = cdp2.call("Target.getTargets", json!({}), None).await {
                                let infos = t["targetInfos"].as_array().cloned().unwrap_or_default();
                                let want = if created { ev["params"]["targetInfo"]["targetId"].as_str().map(String::from) } else { pick_page(&infos) };
                                if let Some(w) = want.filter(|w| *w != current) {
                                    if attach(&cdp2, &w, &session2, viewport).await.is_ok() {
                                        current = w;
                                    }
                                }
                            }
                        }
                    }
                    "Target.targetInfoChanged" => {
                        let i = &ev["params"]["targetInfo"];
                        if i["targetId"] == json!(current) {
                            *url2.lock() = i["url"].as_str().unwrap_or("").to_string();
                        }
                    }
                    _ => {}
                }
            }
        });
        Ok((Arc::new(Live { cdp, session, meta, url, task }), frame_rx))
    }

    fn sid(&self) -> Option<String> {
        self.session.lock().clone()
    }

    /// 目前分頁的網址。
    pub fn current_url(&self) -> String {
        self.url.lock().clone()
    }

    pub async fn mouse(&self, msg: &ClientMsg) -> Result<(), String> {
        let Some(p) = mouse_params(msg, *self.meta.lock()) else { return Ok(()) };
        self.cdp.call("Input.dispatchMouseEvent", p, self.sid().as_deref()).await.map(|_| ())
    }

    pub async fn key(&self, msg: &ClientMsg) -> Result<(), String> {
        let Some(p) = key_params(msg) else { return Ok(()) };
        self.cdp.call("Input.dispatchKeyEvent", p, self.sid().as_deref()).await.map(|_| ())
    }

    pub async fn text(&self, text: &str) -> Result<(), String> {
        self.cdp.call("Input.insertText", json!({ "text": text }), self.sid().as_deref()).await.map(|_| ())
    }

    pub async fn back(&self) -> Result<(), String> {
        let sid = self.sid();
        let h = self.cdp.call("Page.getNavigationHistory", json!({}), sid.as_deref()).await?;
        let idx = h["currentIndex"].as_i64().unwrap_or(0);
        if idx > 0 {
            if let Some(id) = h["entries"][(idx - 1) as usize]["id"].as_i64() {
                self.cdp.call("Page.navigateToHistoryEntry", json!({ "entryId": id }), sid.as_deref()).await?;
            }
        }
        Ok(())
    }

    /// 瀏覽器目前所有的 cookie（遠端登入完成後存 session 用）。
    pub async fn cookies(&self) -> Result<Vec<Value>, String> {
        let v = self.cdp.call("Storage.getCookies", json!({}), None).await?;
        Ok(v["cookies"].as_array().cloned().unwrap_or_default())
    }

    pub fn stop(&self) {
        self.task.abort();
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 連到某個分頁、開始串流（取代原本的）。
async fn attach(cdp: &Arc<Cdp>, target: &str, session: &Arc<Mutex<Option<String>>>, viewport: Option<(u32, u32)>) -> Result<(), String> {
    let old = session.lock().take();
    if let Some(o) = old {
        let _ = cdp.call("Page.stopScreencast", json!({}), Some(&o)).await;
    }
    let r = cdp.call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None).await?;
    let sid = r["sessionId"].as_str().ok_or("attach 沒有回 sessionId")?.to_string();
    cdp.call("Page.enable", json!({}), Some(&sid)).await?;
    if let Some((w, h)) = viewport {
        cdp.call("Emulation.setDeviceMetricsOverride", json!({ "width": w, "height": h, "deviceScaleFactor": 1, "mobile": false }), Some(&sid)).await?;
    }
    cdp.call("Page.startScreencast", json!({ "format": "jpeg", "quality": 60, "maxWidth": 1280, "maxHeight": 800, "everyNthFrame": 1 }), Some(&sid)).await?;
    *session.lock() = Some(sid);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> FrameMeta {
        FrameMeta { w: 1000.0, h: 500.0 }
    }

    fn parse(s: &str) -> ClientMsg {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn normalized_coordinates_are_scaled_to_the_last_frame_size_and_clamped() {
        let p = mouse_params(&parse(r#"{"t":"mouse","type":"down","x":0.25,"y":0.5,"button":"left"}"#), meta()).unwrap();
        assert_eq!((p["type"].clone(), p["x"].clone(), p["y"].clone(), p["buttons"].clone(), p["clickCount"].clone()), (json!("mousePressed"), json!(250.0), json!(250.0), json!(1), json!(1)));
        let out = mouse_params(&parse(r#"{"t":"mouse","type":"move","x":5,"y":-3}"#), meta()).unwrap();
        assert_eq!((out["x"].clone(), out["y"].clone()), (json!(1000.0), json!(0.0)), "超出範圍夾回邊界");
        let nan = mouse_params(&ClientMsg::Mouse { kind: MouseKind::Move, x: f64::NAN, y: f64::INFINITY, button: None, dx: 0.0, dy: 0.0 }, meta()).unwrap();
        assert_eq!((nan["x"].clone(), nan["y"].clone()), (json!(0.0), json!(0.0)), "不是有限數字的當 0，不會把壞值送進瀏覽器");
    }

    #[test]
    fn mouse_buttons_release_and_wheel_map_to_the_right_cdp_events() {
        let up = mouse_params(&parse(r#"{"t":"mouse","type":"up","x":0.1,"y":0.1,"button":"right"}"#), meta()).unwrap();
        assert_eq!((up["type"].clone(), up["button"].clone(), up["buttons"].clone()), (json!("mouseReleased"), json!("right"), json!(0)));
        let w = mouse_params(&parse(r#"{"t":"mouse","type":"wheel","x":0.5,"y":0.5,"dx":0,"dy":120}"#), meta()).unwrap();
        assert_eq!((w["type"].clone(), w["deltaY"].clone()), (json!("mouseWheel"), json!(120.0)));
        assert!(mouse_params(&parse(r#"{"t":"back"}"#), meta()).is_none(), "不是滑鼠訊息");
    }

    #[test]
    fn printable_keys_carry_text_and_special_keys_are_raw() {
        let a = key_params(&parse(r#"{"t":"key","type":"down","key":"a","code":"KeyA","text":"a","vk":65,"mods":0}"#)).unwrap();
        assert_eq!((a["type"].clone(), a["text"].clone(), a["windowsVirtualKeyCode"].clone()), (json!("keyDown"), json!("a"), json!(65)));
        let enter = key_params(&parse(r#"{"t":"key","type":"down","key":"Enter","code":"Enter","text":"\r","vk":13}"#)).unwrap();
        assert_eq!((enter["type"].clone(), enter["text"].clone()), (json!("keyDown"), json!("\r")));
        let arrow = key_params(&parse(r#"{"t":"key","type":"down","key":"ArrowLeft","code":"ArrowLeft","vk":37}"#)).unwrap();
        assert_eq!(arrow["type"], json!("rawKeyDown"));
        assert!(arrow.get("text").is_none());
        let bad = key_params(&parse(r#"{"t":"key","type":"down","key":"x","text":"abc"}"#)).unwrap();
        assert!(bad.get("text").is_none(), "不是單一字元的 text 不收");
        let ctrl = key_params(&parse("{\"t\":\"key\",\"type\":\"down\",\"key\":\"x\",\"text\":\"\\u0007\"}")).unwrap();
        assert!(ctrl.get("text").is_none(), "控制字元不收");
        let up = key_params(&parse(r#"{"t":"key","type":"up","key":"a","code":"KeyA","vk":65,"mods":8}"#)).unwrap();
        assert_eq!((up["type"].clone(), up["modifiers"].clone()), (json!("keyUp"), json!(8)));
    }

    #[test]
    fn messages_round_trip_as_tagged_json() {
        let s = serde_json::to_value(ServerMsg::State { manual: true, url: "https://x/".into(), can_operate: false, mode: "agent".into() }).unwrap();
        assert_eq!(s, json!({"t": "state", "manual": true, "url": "https://x/", "can_operate": false, "mode": "agent"}));
        assert_eq!(serde_json::to_value(ServerMsg::Closed { why: "結束".into() }).unwrap()["t"], json!("closed"));
        assert_eq!(parse(r#"{"t":"takeover","on":true}"#), ClientMsg::Takeover { on: true });
        assert_eq!(parse(r#"{"t":"done"}"#), ClientMsg::Done);
        assert!(serde_json::from_str::<ClientMsg>(r#"{"t":"rm -rf"}"#).is_err(), "不認得的訊息不會被接受");
    }

    #[test]
    fn the_page_picker_prefers_real_content_and_skips_devtools() {
        let t = vec![
            json!({"targetId": "a", "type": "page", "url": "about:blank"}),
            json!({"targetId": "d", "type": "page", "url": "devtools://x"}),
            json!({"targetId": "b", "type": "page", "url": "https://x/"}),
            json!({"targetId": "w", "type": "service_worker", "url": "https://x/sw.js"}),
        ];
        assert_eq!(pick_page(&t).as_deref(), Some("b"));
        assert_eq!(pick_page(&t[..2]).as_deref(), Some("a"), "都空白就用第一個");
        assert_eq!(pick_page(&[]), None);
    }

    /// 需要真的 Chrome：`cargo test -p aiterm-systems --lib -- --ignored live::`。
    /// 串流收得到畫面；正規化座標的點擊真的點到按鈕（測試站收到 POST）；打字進得了輸入框；登入按鈕設的 cookie 取得到。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn streaming_and_remote_input_work_against_a_real_headless_chrome() {
        use crate::browser;
        use crate::testsite::fake_site;
        use std::sync::atomic::Ordering;
        let (port, posts) = fake_site().await;
        let dir = tempfile::tempdir().unwrap();
        let mut b = browser::launch_with(dir.path(), "live", &format!("http://127.0.0.1:{port}/live"), true).await.unwrap();
        let (live, mut frames) = Live::start(&b.ws_url, Some((1280, 800))).await.unwrap();
        let first = tokio::time::timeout(Duration::from_secs(10), frames.recv()).await.expect("要收到第一張畫面").unwrap();
        assert!(first.jpeg_b64.len() > 100 && (first.w, first.h) == (1280, 800), "{}x{}", first.w, first.h);

        // 點「Go」（左上 100,100、200×60 → 中心 200,130）。
        let click = |k: MouseKind| ClientMsg::Mouse { kind: k, x: 200.0 / 1280.0, y: 130.0 / 800.0, button: Some("left".into()), dx: 0.0, dy: 0.0 };
        live.mouse(&click(MouseKind::Move)).await.unwrap();
        live.mouse(&click(MouseKind::Down)).await.unwrap();
        live.mouse(&click(MouseKind::Up)).await.unwrap();
        for _ in 0..30 {
            if posts.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(posts.load(Ordering::SeqCst), 1, "遠端點擊要真的點到按鈕");

        // 點輸入框、打字（text）；再按下登入，cookie 取得到。
        let at = |y: f64| ClientMsg::Mouse { kind: MouseKind::Down, x: 200.0 / 1280.0, y: y / 800.0, button: Some("left".into()), dx: 0.0, dy: 0.0 };
        live.mouse(&at(320.0)).await.unwrap();
        live.mouse(&ClientMsg::Mouse { kind: MouseKind::Up, x: 200.0 / 1280.0, y: 320.0 / 800.0, button: Some("left".into()), dx: 0.0, dy: 0.0 }).await.unwrap();
        live.text("你好abc").await.unwrap();
        live.mouse(&at(230.0)).await.unwrap();
        live.mouse(&ClientMsg::Mouse { kind: MouseKind::Up, x: 200.0 / 1280.0, y: 230.0 / 800.0, button: Some("left".into()), dx: 0.0, dy: 0.0 }).await.unwrap();
        let mut got = false;
        for _ in 0..30 {
            if live.cookies().await.unwrap().iter().any(|c| c["name"] == "sid" && c["value"] == "real-login") {
                got = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(got, "登入按鈕設的 cookie 要取得到");
        assert!(live.current_url().contains("/signin") || !live.current_url().is_empty());
        live.stop();
        b.close().await;
    }
}
