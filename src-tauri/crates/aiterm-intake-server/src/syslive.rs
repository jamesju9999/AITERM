//! 即時預覽、人工接手與遠端登入的 WebSocket 服務。
//!
//! 一個 `LiveSource` 對應一個瀏覽器（Agent 執行中的某張卡＋系統，或一個遠端登入）。第一位觀看者出現才開始串流，
//! 最後一位離開就停；每個來源最多 3 位觀看者。成員只能看，管理員才能操作；Agent 執行中要先「接手」才能輸入，
//! 接手者斷線就自動交還。協定見 `aiterm_systems::live`。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aiterm_systems::live::{ClientMsg, Live, ServerMsg};
use axum::extract::ws::{Message, WebSocket};
use tokio::sync::broadcast;

use crate::members::{self, Me, Role};
use crate::state::AppState;

/// 每個來源最多幾位觀看者。
pub const MAX_VIEWERS: usize = 3;
/// 多久檢查一次成員是不是還有效（撤銷後最慢這麼久斷線）。
const REVOCATION_CHECK: Duration = Duration::from_secs(2);
/// 多久同步一次網址與接手狀態。
const STATE_POLL: Duration = Duration::from_millis(500);

#[derive(Default)]
struct Inner {
    live: Option<Arc<Live>>,
    pump: Option<tokio::task::JoinHandle<()>>,
    last_frame: Option<ServerMsg>,
}

/// 一個可被觀看的瀏覽器。
pub struct LiveSource {
    ws_url: String,
    viewport: Option<(u32, u32)>,
    /// `agent`｜`login`。
    mode: &'static str,
    /// Agent 執行中：接手旗標（遠端登入沒有）。
    manual: Option<Arc<AtomicBool>>,
    inner: tokio::sync::Mutex<Inner>,
    tx: broadcast::Sender<ServerMsg>,
    subs: AtomicUsize,
    closed: AtomicBool,
}

/// 觀看者的訂閱；被丟掉就離開（最後一位離開時停止串流）。
pub struct Subscription {
    pub rx: broadcast::Receiver<ServerMsg>,
    /// 一加入就先給的狀態與最近一張畫面。
    pub snapshot: Vec<ServerMsg>,
    src: Arc<LiveSource>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let src = self.src.clone();
        let left = src.subs.fetch_sub(1, Ordering::SeqCst) - 1;
        if left == 0 {
            if let Ok(h) = tokio::runtime::Handle::try_current() {
                h.spawn(async move {
                    // 已經有人又加入就不停。
                    if src.subs.load(Ordering::SeqCst) == 0 {
                        src.stop_stream().await;
                    }
                });
            }
        }
    }
}

impl LiveSource {
    pub fn new(ws_url: String, viewport: Option<(u32, u32)>, mode: &'static str, manual: Option<Arc<AtomicBool>>) -> Arc<Self> {
        let (tx, _) = broadcast::channel(8);
        Arc::new(Self { ws_url, viewport, mode, manual, inner: Default::default(), tx, subs: AtomicUsize::new(0), closed: AtomicBool::new(false) })
    }

    pub fn mode(&self) -> &'static str {
        self.mode
    }

    pub fn is_manual(&self) -> bool {
        self.manual.as_ref().is_some_and(|m| m.load(Ordering::SeqCst))
    }

    pub fn viewers(&self) -> usize {
        self.subs.load(Ordering::SeqCst)
    }

    /// 目前的 `Live`（串流中才有）。
    pub async fn live(&self) -> Option<Arc<Live>> {
        self.inner.lock().await.live.clone()
    }

    fn state_msg(&self, url: &str) -> ServerMsg {
        ServerMsg::State { manual: self.is_manual(), url: url.to_string(), can_operate: false, mode: self.mode.to_string() }
    }

    /// 加入觀看。第一位會啟動串流；超過上限或來源已關閉回錯誤說明。
    pub async fn subscribe(self: &Arc<Self>) -> Result<Subscription, String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err("這個瀏覽器已經結束了".into());
        }
        if self.subs.fetch_add(1, Ordering::SeqCst) >= MAX_VIEWERS {
            self.subs.fetch_sub(1, Ordering::SeqCst);
            return Err(format!("這個畫面已經有 {MAX_VIEWERS} 位在看了"));
        }
        let mut inner = self.inner.lock().await;
        if inner.live.is_none() {
            match Live::start(&self.ws_url, self.viewport).await {
                Ok((live, mut frames)) => {
                    let me = self.clone();
                    let live2 = live.clone();
                    let pump = tokio::spawn(async move {
                        let mut tick = tokio::time::interval(STATE_POLL);
                        let mut last_state = (false, String::new());
                        loop {
                            tokio::select! {
                                f = frames.recv() => match f {
                                    Some(f) => {
                                        let msg = ServerMsg::Frame { data: f.jpeg_b64, w: f.w, h: f.h };
                                        me.inner.lock().await.last_frame = Some(msg.clone());
                                        let _ = me.tx.send(msg);
                                    }
                                    None => break,
                                },
                                _ = tick.tick() => {
                                    let cur = (me.is_manual(), live2.current_url());
                                    if cur != last_state {
                                        let _ = me.tx.send(me.state_msg(&cur.1));
                                        last_state = cur;
                                    }
                                }
                            }
                        }
                    });
                    inner.live = Some(live);
                    inner.pump = Some(pump);
                }
                Err(e) => {
                    self.subs.fetch_sub(1, Ordering::SeqCst);
                    return Err(format!("無法開始串流：{e}"));
                }
            }
        }
        let url = inner.live.as_ref().map(|l| l.current_url()).unwrap_or_default();
        let mut snapshot = vec![self.state_msg(&url)];
        if let Some(f) = inner.last_frame.clone() {
            snapshot.push(f);
        }
        let rx = self.tx.subscribe();
        drop(inner);
        Ok(Subscription { rx, snapshot, src: self.clone() })
    }

    async fn stop_stream(&self) {
        let mut inner = self.inner.lock().await;
        if let Some(p) = inner.pump.take() {
            p.abort();
        }
        if let Some(l) = inner.live.take() {
            l.stop();
        }
        inner.last_frame = None;
    }

    /// 瀏覽器要結束了：通知所有觀看者並停止串流。
    pub async fn shutdown(&self, why: &str) {
        self.closed.store(true, Ordering::SeqCst);
        let _ = self.tx.send(ServerMsg::Closed { why: why.to_string() });
        self.stop_stream().await;
    }
}

/// 這條 WebSocket 連線是哪一種。
pub enum Kind {
    /// Agent 執行中的某張卡＋系統。
    Agent { card_id: String, system_id: String, system_name: String },
    /// 遠端登入（只有管理員會走到這裡）。
    Login { system_id: String, system_name: String },
}

async fn send(socket: &mut WebSocket, msg: &ServerMsg) -> bool {
    let Ok(text) = serde_json::to_string(msg) else { return false };
    socket.send(Message::Text(text.into())).await.is_ok()
}

fn with_operate(msg: ServerMsg, can: bool) -> ServerMsg {
    match msg {
        ServerMsg::State { manual, url, mode, .. } => ServerMsg::State { manual, url, can_operate: can, mode },
        other => other,
    }
}

/// 一條觀看／操作連線的整個生命週期。
pub async fn serve(st: Arc<AppState>, me: Me, mut socket: WebSocket, src: Arc<LiveSource>, kind: Kind) {
    let admin = me.role == Role::Admin;
    let sub = match src.subscribe().await {
        Ok(s) => s,
        Err(why) => {
            let _ = send(&mut socket, &ServerMsg::Error { message: why }).await;
            let _ = send(&mut socket, &ServerMsg::Closed { why: "無法觀看".into() }).await;
            return;
        }
    };
    let mut sub = sub;
    let snapshot = std::mem::take(&mut sub.snapshot);
    let detail = match &kind {
        Kind::Agent { system_name, card_id, .. } => format!("{system_name} 卡片 {card_id}"),
        Kind::Login { system_name, .. } => system_name.clone(),
    };
    crate::audit::record(&st.pools.server, &me.name, if matches!(kind, Kind::Login { .. }) { "system.login.view" } else { "system.live.view" }, &detail).await;
    for m in snapshot {
        if !send(&mut socket, &with_operate(m, admin)).await {
            return;
        }
    }
    // 這條連線是不是自己把 Agent 接手了（斷線時要交還）。
    let mut took_over = false;
    let mut tick = tokio::time::interval(REVOCATION_CHECK);
    let handback = |took: bool, st: Arc<AppState>, who: String, detail: String, src: Arc<LiveSource>| async move {
        if took {
            if let Some(m) = &src.manual {
                m.store(false, Ordering::SeqCst);
            }
            crate::audit::record(&st.pools.server, &who, "system.live.handback", &detail).await;
        }
    };
    loop {
        tokio::select! {
            got = sub.rx.recv() => match got {
                Ok(m) => {
                    let closing = matches!(m, ServerMsg::Closed { .. });
                    if !send(&mut socket, &with_operate(m, admin)).await { break }
                    if closing { break }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            },
            _ = tick.tick() => {
                if !members::is_active(&st.pools.server, &me.id).await {
                    let _ = send(&mut socket, &ServerMsg::Closed { why: "你的連線已被撤銷".into() }).await;
                    break;
                }
            }
            msg = socket.recv() => match msg {
                Some(Ok(Message::Text(t))) => {
                    let Ok(cm) = serde_json::from_str::<ClientMsg>(t.as_str()) else {
                        let _ = send(&mut socket, &ServerMsg::Error { message: "不認得的訊息".into() }).await;
                        continue;
                    };
                    if !admin {
                        let _ = send(&mut socket, &ServerMsg::Error { message: "只有管理員能操作".into() }).await;
                        continue;
                    }
                    match handle_input(&st, &me, &src, &kind, &cm, &mut took_over).await {
                        Ok(true) => {
                            // 遠端登入完成：通知並結束。
                            let _ = send(&mut socket, &ServerMsg::Closed { why: "登入完成，session 已保存".into() }).await;
                            break;
                        }
                        Ok(false) => {}
                        Err(e) => {
                            let _ = send(&mut socket, &ServerMsg::Error { message: e }).await;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
    handback(took_over, st, me.name.clone(), detail, src).await;
}

/// 處理管理員的一則輸入。回 `Ok(true)` 表示這條連線該結束了（遠端登入完成）。
async fn handle_input(st: &Arc<AppState>, me: &Me, src: &Arc<LiveSource>, kind: &Kind, msg: &ClientMsg, took_over: &mut bool) -> Result<bool, String> {
    match msg {
        ClientMsg::Takeover { on } => {
            let Some(m) = &src.manual else { return Err("遠端登入不需要接手".into()) };
            if *on == m.load(Ordering::SeqCst) {
                return Ok(false);
            }
            if *on && m.load(Ordering::SeqCst) {
                return Err("已經有人接手了".into());
            }
            m.store(*on, Ordering::SeqCst);
            *took_over = *on;
            let detail = match kind {
                Kind::Agent { system_name, card_id, .. } => format!("{system_name} 卡片 {card_id}"),
                Kind::Login { system_name, .. } => system_name.clone(),
            };
            crate::audit::record(&st.pools.server, &me.name, if *on { "system.live.takeover" } else { "system.live.handback" }, &detail).await;
            Ok(false)
        }
        ClientMsg::Done => match kind {
            Kind::Login { system_id, .. } => {
                crate::syscore::finish_login(st, system_id, &me.name).await?;
                Ok(true)
            }
            Kind::Agent { .. } => Err("這不是遠端登入".into()),
        },
        input => {
            // Agent 執行中要先接手才能輸入，免得跟 Agent 搶著操作。
            if matches!(kind, Kind::Agent { .. }) && !src.is_manual() {
                return Err("請先按「接手」再操作".into());
            }
            let live = src.live().await.ok_or("串流還沒就緒")?;
            match input {
                ClientMsg::Mouse { .. } => live.mouse(input).await?,
                ClientMsg::Key { .. } => live.key(input).await?,
                ClientMsg::Text { text } => live.text(text).await?,
                ClientMsg::Back => live.back().await?,
                _ => {}
            }
            Ok(false)
        }
    }
}
