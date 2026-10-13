//! Wayland：從遠端控制 session 附帶的螢幕串流（ScreenCast，經 PipeWire）直接取畫面。
//!
//! 截圖 portal 每拍一張 GNOME 都會閃一下、而且慢；串流則是授權一次後持續送來的畫面。這裡開一條
//! PipeWire 執行緒，一直保留「最新的一格」（合成器只在畫面有變動時才送新格），要截圖時直接拿。
//! 只收記憶體共享的影格（不宣告 DMA-BUF modifier，合成器就會改用 MemFd），像素轉換見 `waylandmap`。

use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pipewire as pw;
use pw::spa;
use pw::spa::param::video::VideoFormat;

use super::waylandmap::{self as wm, PixelLayout};

/// 串流裡的一格原始影像。
struct RawFrame {
    layout: PixelLayout,
    width: u32,
    height: u32,
    stride: usize,
    bytes: Vec<u8>,
    at: Instant,
}

/// 轉好的一格（RGBA8）。
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Capture {
    latest: Arc<Mutex<Option<RawFrame>>>,
    stop: Mutex<Option<pw::channel::Sender<()>>>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(tx) = self.stop.lock().ok().and_then(|mut g| g.take()) {
            let _ = tx.send(());
        }
    }
}

fn layout_of(f: VideoFormat) -> Option<PixelLayout> {
    if f == VideoFormat::BGRx || f == VideoFormat::BGRA {
        Some(PixelLayout::Bgrx)
    } else if f == VideoFormat::RGBx || f == VideoFormat::RGBA {
        Some(PixelLayout::Rgbx)
    } else if f == VideoFormat::xRGB || f == VideoFormat::ARGB {
        Some(PixelLayout::Xrgb)
    } else if f == VideoFormat::xBGR || f == VideoFormat::ABGR {
        Some(PixelLayout::Xbgr)
    } else {
        None
    }
}

impl Capture {
    /// 連到 portal 給的 PipeWire 遠端，訂閱 `node` 這條螢幕串流。連線與串流都建起來才回傳。
    pub fn start(fd: OwnedFd, node: u32) -> Result<Capture, String> {
        let latest: Arc<Mutex<Option<RawFrame>>> = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let (stop_tx, stop_rx) = pw::channel::channel::<()>();
        let sink = latest.clone();
        std::thread::Builder::new()
            .name("aiterm-pipewire".into())
            .spawn(move || {
                if let Err(e) = run(fd, node, sink, stop_rx, &ready_tx) {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| format!("無法啟動 PipeWire 執行緒：{e}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Capture { latest, stop: Mutex::new(Some(stop_tx)) }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("PipeWire 串流逾時沒有建立".to_string()),
        }
    }

    /// 取最新的一格。`since` 之後若有新格就等它（最多 `wait`）：剛把 AITerm 縮小，畫面會變，
    /// 要拿變動後的那一格；畫面沒變時合成器不會送新格，逾時就用手上最新的。完全沒有影格回 `None`。
    pub async fn frame(&self, since: Instant, wait: Duration) -> Option<Frame> {
        let deadline = Instant::now() + wait;
        loop {
            let fresh = self.latest.lock().ok().and_then(|g| g.as_ref().map(|f| f.at >= since)).unwrap_or(false);
            if fresh || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        let g = self.latest.lock().ok()?;
        let f = g.as_ref()?;
        let rgba = wm::frame_to_rgba(f.layout, f.width, f.height, f.stride, &f.bytes)?;
        Some(Frame { width: f.width, height: f.height, rgba })
    }
}

#[derive(Default)]
struct StreamState {
    format: Option<(PixelLayout, u32, u32)>,
}

fn run(
    fd: OwnedFd,
    node: u32,
    sink: Arc<Mutex<Option<RawFrame>>>,
    stop_rx: pw::channel::Receiver<()>,
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|e| format!("PipeWire main loop：{e}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(|e| format!("PipeWire context：{e}"))?;
    let core = context.connect_fd_rc(fd, None).map_err(|e| format!("連線到 PipeWire 失敗：{e}"))?;

    let _stop = stop_rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });

    let stream = pw::stream::StreamBox::new(
        &core,
        "aiterm-screen",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| format!("建立 PipeWire 串流失敗：{e}"))?;

    let _listener = stream
        .add_local_listener_with_user_data(StreamState::default())
        .param_changed(|_, state, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param) else { return };
            if media_type != spa::param::format::MediaType::Video || media_subtype != spa::param::format::MediaSubtype::Raw {
                return;
            }
            let mut info = spa::param::video::VideoInfoRaw::default();
            if info.parse(param).is_err() {
                return;
            }
            let size = info.size();
            state.format = layout_of(info.format()).map(|l| (l, size.width, size.height));
            if state.format.is_none() {
                log::warn!("PipeWire 串流的像素格式不支援：{:?}", info.format());
            }
        })
        .process(move |stream, state| {
            let Some((layout, width, height)) = state.format else { return };
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            let Some(d) = datas.first_mut() else { return };
            let (offset, size, stride) = {
                let c = d.chunk();
                (c.offset() as usize, c.size() as usize, c.stride())
            };
            let stride = if stride > 0 { stride as usize } else { width as usize * 4 };
            let Some(bytes) = d.data() else { return }; // DMA-BUF 等無法對映的緩衝區略過
            let end = (offset + size).min(bytes.len());
            if offset >= end {
                return;
            }
            let copy = bytes[offset..end].to_vec();
            if let Ok(mut g) = sink.lock() {
                *g = Some(RawFrame { layout, width, height, stride, bytes: copy, at: Instant::now() });
            }
        })
        .register()
        .map_err(|e| format!("註冊 PipeWire 串流回呼失敗：{e}"))?;

    let obj = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(spa::param::format::FormatProperties::MediaType, Id, spa::param::format::MediaType::Video),
        spa::pod::property!(spa::param::format::FormatProperties::MediaSubtype, Id, spa::param::format::MediaSubtype::Raw),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA,
            VideoFormat::xRGB,
            VideoFormat::xBGR,
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle { width: 1920, height: 1080 },
            spa::utils::Rectangle { width: 1, height: 1 },
            spa::utils::Rectangle { width: 8192, height: 8192 }
        ),
        // 只要夠新的畫面，不需要高幀率：合成器每秒最多送 10 格，省 CPU。
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction { num: 5, denom: 1 },
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction { num: 10, denom: 1 }
        ),
    );
    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &spa::pod::Value::Object(obj))
        .map_err(|e| format!("PipeWire 格式參數：{e:?}"))?
        .0
        .into_inner();
    let mut params = [spa::pod::Pod::from_bytes(&values).ok_or("PipeWire 格式參數無效")?];
    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| format!("連到螢幕串流失敗：{e}"))?;

    let _ = ready.send(Ok(()));
    mainloop.run();
    Ok(())
}
