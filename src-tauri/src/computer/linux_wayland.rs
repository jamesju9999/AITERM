//! Linux（Wayland）後端：全部走 xdg-desktop-portal。
//!
//! - 截圖：Screenshot portal（非互動）。第一次桌面會問是否允許。
//! - 滑鼠鍵盤：RemoteDesktop portal（搭配 ScreenCast 取得螢幕座標系）。第一次桌面會跳授權框；
//!   選「記住」後以 restore token 保存，之後不再詢問。
//! - 文字辨識：沿用 X11 版的 tesseract；圖示偵測沿用 `icons`。
//!
//! Wayland 刻意不讓程式讀其他視窗，所以沒有「App 清單」與「切換前景」：整個桌面當成一個虛擬 App
//! （名稱 `Desktop`／`桌面`），操作對象永遠是整個畫面。沒有無障礙樹，`snapshot` 只回一個全螢幕視窗，
//! 共用邏輯會因此判定資訊很少而引導模型改用 `computer_read_screen`／`computer_click_text`。
//! 純邏輯見 `waylandmap.rs`。
//!
//! **這份程式碼在開發當下只通過型別檢查（x86_64-unknown-linux-gnu），尚未在真實的 Wayland 桌面執行。**

use std::sync::Arc;
use std::time::Duration;

use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::screenshot::Screenshot;
use ashpd::desktop::{PersistMode, Session};
use async_trait::async_trait;

use super::backend::*;
use super::icons::{detect_icons, Region};
use super::linux::{spawn_detached, tesseract_ocr};
use super::{linuxmap, waylandmap as wm, winmap};
use crate::resident::portal::registered_connection;

fn perr(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what}：{e}")
}

/// 一個已啟動的 RemoteDesktop＋ScreenCast session 與它的螢幕座標系。
struct Input {
    remote: RemoteDesktop,
    session: Session<RemoteDesktop>,
    stream: u32,
    pos: (i32, i32),
    size: (i32, i32),
}

static INPUT: tokio::sync::Mutex<Option<Arc<Input>>> = tokio::sync::Mutex::const_new(None);

fn token_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))?;
    Some(base.join("com.aiterm.app").join("wayland_input_token"))
}

fn read_token() -> Option<String> {
    let t = std::fs::read_to_string(token_path()?).ok()?;
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

fn save_token(token: &str) {
    if let Some(p) = token_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, token);
    }
}

async fn start_input() -> Result<Input, String> {
    let conn = registered_connection().await.map_err(|e| perr("無法連線到桌面的 D-Bus", e))?;
    let remote = RemoteDesktop::with_connection(conn.clone()).await.map_err(|e| perr("RemoteDesktop portal 不可用", e))?;
    let screencast = Screencast::with_connection(conn).await.map_err(|e| perr("ScreenCast portal 不可用", e))?;
    let session = remote.create_session(Default::default()).await.map_err(|e| perr("建立 portal session 失敗", e))?;
    let token = read_token();
    remote
        .select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(token.as_deref()),
        )
        .await
        .map_err(|e| perr("select_devices 失敗", e))?
        .response()
        .map_err(|e| perr("select_devices 被拒絕", e))?;
    screencast
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_sources(ashpd::enumflags2::BitFlags::from(SourceType::Monitor))
                .set_multiple(false)
                .set_cursor_mode(CursorMode::Hidden),
        )
        .await
        .map_err(|e| perr("select_sources 失敗", e))?
        .response()
        .map_err(|e| perr("select_sources 被拒絕", e))?;
    let started = remote
        .start(&session, None, Default::default())
        .await
        .map_err(|e| perr("啟動遠端控制失敗", e))?
        .response()
        .map_err(|e| format!("桌面沒有核准電腦操作（授權框被取消或拒絕）：{e}"))?;
    if let Some(t) = started.restore_token() {
        save_token(t);
    }
    let stream = started.streams().first().ok_or("桌面沒有提供螢幕串流，無法定位滑鼠")?;
    let size = stream.size().ok_or("桌面沒有回報螢幕大小，無法定位滑鼠")?;
    Ok(Input {
        remote,
        session,
        stream: stream.pipe_wire_node_id(),
        pos: stream.position().unwrap_or((0, 0)),
        size,
    })
}

async fn input() -> Result<Arc<Input>, String> {
    let mut g = INPUT.lock().await;
    if let Some(i) = g.as_ref() {
        return Ok(i.clone());
    }
    let i = Arc::new(start_input().await.map_err(|e| {
        log::warn!("Wayland 遠端控制 session 建立失敗：{e}");
        e
    })?);
    *g = Some(i.clone());
    Ok(i)
}

async fn reset_input() {
    *INPUT.lock().await = None;
}

/// 執行一個輸入動作；失敗（session 可能已被桌面關掉）就重建 session 重試一次。
async fn with_input<T, Fut>(op: impl Fn(Arc<Input>) -> Fut) -> Result<T, String>
where
    Fut: std::future::Future<Output = Result<T, ashpd::Error>>,
{
    let mut last = String::new();
    for _ in 0..2 {
        let i = input().await?;
        match op(i).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = e.to_string();
                reset_input().await;
            }
        }
    }
    Err(format!("送出輸入失敗：{last}"))
}

async fn key_event(i: &Input, keysym: u32, pressed: bool) -> Result<(), ashpd::Error> {
    let state = if pressed { KeyState::Pressed } else { KeyState::Released };
    i.remote.notify_keyboard_keysym(&i.session, keysym as i32, state, Default::default()).await
}

async fn tap_keysym(i: &Input, keysym: u32) -> Result<(), ashpd::Error> {
    key_event(i, keysym, true).await?;
    key_event(i, keysym, false).await
}

async fn move_to(i: &Input, x: f64, y: f64) -> Result<(), ashpd::Error> {
    let (sx, sy) = wm::stream_point(x, y, i.pos, i.size);
    i.remote.notify_pointer_motion_absolute(&i.session, i.stream, sx, sy, Default::default()).await
}

async fn button_tap(i: &Input, button: i32) -> Result<(), ashpd::Error> {
    i.remote.notify_pointer_button(&i.session, button, KeyState::Pressed, Default::default()).await?;
    i.remote.notify_pointer_button(&i.session, button, KeyState::Released, Default::default()).await
}

// ── 截圖 ────────────────────────────────────────────────────────────────────────────

/// 一張全螢幕截圖（物理像素 RGBA）與縮放比例（物理 ÷ 邏輯）、邏輯座標原點。
struct Grab {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
    scale: f64,
    origin: (f64, f64),
}

async fn grab() -> Result<Grab, String> {
    grab_inner().await.map_err(|e| {
        log::warn!("Wayland 截圖失敗：{e}");
        e
    })
}

async fn grab_inner() -> Result<Grab, String> {
    let conn = registered_connection().await.map_err(|e| perr("無法連線到桌面的 D-Bus", e))?;
    let shot = Screenshot::request()
        .interactive(false)
        .connection(Some(conn))
        .send()
        .await
        .map_err(|e| perr("Screenshot portal 不可用", e))?
        .response()
        .map_err(|e| format!("桌面沒有允許截圖（授權框被取消或拒絕）：{e}"))?;
    let path = wm::file_uri_to_path(shot.uri().as_str()).ok_or("portal 回傳的截圖位置不是本機檔案")?;
    let bytes = std::fs::read(&path).map_err(|e| perr("讀取截圖失敗", e))?;
    let _ = std::fs::remove_file(&path); // portal 把截圖存在圖片資料夾；用完就刪，不留垃圾
    let (w, h, rgba) = wm::decode_png_rgba(&bytes)?;
    // 邏輯大小來自輸入 session 的螢幕串流；它建不起來（例如使用者拒絕遠端控制）就當縮放為 1，
    // 截圖與文字辨識仍可用，只是座標在高 DPI 螢幕上不準。
    let (scale, origin) = match input().await {
        Ok(i) => (wm::scale_factor(w, i.size.0), (i.pos.0 as f64, i.pos.1 as f64)),
        Err(e) => {
            log::warn!("Wayland：取不到螢幕邏輯大小，縮放以 1 計：{e}");
            (1.0, (0.0, 0.0))
        }
    };
    Ok(Grab { w, h, rgba, scale, origin })
}

// ── Backend ──────────────────────────────────────────────────────────────────────────

pub struct WaylandBackend;

#[async_trait]
impl Backend for WaylandBackend {
    fn permissions(&self) -> Permissions {
        // 授權由桌面在第一次使用時詢問，這裡不預先擋。
        Permissions { accessibility: true, screen_recording: true }
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, String> {
        Ok(vec![AppInfo {
            name: "Desktop".to_string(),
            localized: Some("桌面".to_string()),
            bundle_id: Some("wayland-desktop".to_string()),
            pid: WHOLE_DESKTOP_PID,
            frontmost: true,
        }])
    }

    async fn snapshot(&self, _pid: i32, _max_depth: usize) -> Result<ElementNode, String> {
        let g = grab().await?;
        let (w, h) = (g.w as f64 / g.scale, g.h as f64 / g.scale);
        Ok(ElementNode {
            id: "0".into(),
            role: "AXApplication".into(),
            title: "Desktop".into(),
            children: vec![ElementNode {
                id: "0.0".into(),
                role: "AXWindow".into(),
                title: "Wayland 桌面（整個畫面）".into(),
                x: g.origin.0,
                y: g.origin.1,
                w,
                h,
                children: vec![],
            }],
            ..Default::default()
        })
    }

    async fn element_info(&self, _pid: i32, _id: &str) -> Result<ElementNode, String> {
        Err("Wayland 沒有無障礙元件樹；請用 computer_read_screen／computer_click_text／computer_click_icon".to_string())
    }

    async fn element_at(&self, _x: f64, _y: f64) -> Result<Option<ElementNode>, String> {
        Ok(None)
    }

    async fn screenshot(&self, _pid: Option<i32>) -> Result<Shot, String> {
        let g = grab().await?;
        let (lw, lh) = ((g.w as f64 / g.scale).round() as u32, (g.h as f64 / g.scale).round() as u32);
        let rgba = wm::downscale_rgba(&g.rgba, g.w, g.h, lw, lh);
        let (w, h) = if rgba.len() == (lw as usize) * (lh as usize) * 4 { (lw, lh) } else { (g.w, g.h) };
        let rgba = if (w, h) == (g.w, g.h) { g.rgba.clone() } else { rgba };
        let png = winmap::encode_png_rgba(w, h, &rgba)?;
        Ok(Shot { png, width: w, height: h, origin_x: g.origin.0, origin_y: g.origin.1 })
    }

    async fn activate(&self, _pid: i32) -> Result<(), String> {
        Ok(()) // Wayland 沒有切換前景的途徑；操作的永遠是整個畫面
    }

    async fn press_element(&self, _pid: i32, _id: &str) -> Result<(), String> {
        Err("Wayland 沒有無障礙元件可以直接啟動".to_string())
    }

    async fn click_at(&self, x: f64, y: f64, button: MouseButton, double: bool) -> Result<(), String> {
        let code = match button {
            MouseButton::Left => wm::BTN_LEFT,
            MouseButton::Right => wm::BTN_RIGHT,
        };
        with_input(|i| async move {
            move_to(&i, x, y).await?;
            tokio::time::sleep(Duration::from_millis(40)).await;
            for n in 0..if double { 2 } else { 1 } {
                if n > 0 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                button_tap(&i, code).await?;
            }
            Ok(())
        })
        .await
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        let text = text.to_string();
        with_input(|i| {
            let text = text.clone();
            async move {
                for c in text.chars() {
                    let keysym = match c {
                        '\n' | '\r' => 0xff0d,
                        '\t' => 0xff09,
                        _ => linuxmap::char_keysym(c),
                    };
                    tap_keysym(&i, keysym).await?;
                    tokio::time::sleep(Duration::from_millis(8)).await;
                }
                Ok(())
            }
        })
        .await
    }

    async fn focus_element(&self, _pid: i32, _id: &str) -> Result<(), String> {
        Err("Wayland 沒有無障礙元件可以聚焦；請先用 computer_click_text 點到輸入欄".to_string())
    }

    async fn key(&self, combo: &KeyCombo) -> Result<(), String> {
        let keysym = linuxmap::keysym_for_key(&combo.key).ok_or_else(|| format!("不支援的按鍵「{}」", combo.key))?;
        let mods = linuxmap::modifier_keysyms(combo);
        with_input(|i| {
            let mods = mods.clone();
            async move {
                for m in &mods {
                    key_event(&i, *m, true).await?;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
                tap_keysym(&i, keysym).await?;
                tokio::time::sleep(Duration::from_millis(10)).await;
                for m in mods.iter().rev() {
                    key_event(&i, *m, false).await?;
                }
                Ok(())
            }
        })
        .await
    }

    async fn scroll(&self, dx: i32, dy: i32, at: Option<(f64, f64)>) -> Result<(), String> {
        let (sx, sy) = wm::scroll_steps(dx, dy);
        with_input(|i| async move {
            if let Some((x, y)) = at {
                move_to(&i, x, y).await?;
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            use ashpd::desktop::remote_desktop::Axis;
            if sy != 0 {
                i.remote.notify_pointer_axis_discrete(&i.session, Axis::Vertical, sy, Default::default()).await?;
            }
            if sx != 0 {
                i.remote.notify_pointer_axis_discrete(&i.session, Axis::Horizontal, sx, Default::default()).await?;
            }
            Ok(())
        })
        .await
    }

    async fn focused_text(&self, _pid: i32) -> Result<Option<FocusedText>, String> {
        Ok(None)
    }

    async fn read_screen(&self, _pid: Option<i32>) -> Result<Vec<ScreenText>, String> {
        let g = grab().await?;
        let png = winmap::encode_png_rgba(g.w, g.h, &g.rgba)?;
        let lines = tokio::task::spawn_blocking(move || tesseract_ocr(&png))
            .await
            .map_err(|e| format!("內部錯誤：{e}"))??;
        Ok(lines
            .into_iter()
            .map(|l| ScreenText {
                text: l.text,
                x: g.origin.0 + l.x / g.scale,
                y: g.origin.1 + l.y / g.scale,
                w: l.w / g.scale,
                h: l.h / g.scale,
            })
            .collect())
    }

    async fn find_icons(&self, _pid: i32, region: Region) -> Result<Vec<IconRect>, String> {
        let g = grab().await?;
        let (w, h, scale, origin) = (g.w as usize, g.h as usize, g.scale, g.origin);
        let bgra = wm::rgba_to_bgra(&g.rgba);
        tokio::task::spawn_blocking(move || {
            let lum = winmap::bgra_to_luminance(&bgra);
            detect_icons(&lum, w, h, scale, region)
                .into_iter()
                .map(|r| IconRect {
                    x: origin.0 + r.x / scale,
                    y: origin.1 + r.y / scale,
                    w: r.w / scale,
                    h: r.h / scale,
                })
                .collect()
        })
        .await
        .map_err(|e| format!("內部錯誤：{e}"))
    }

    async fn open_app(&self, name: &str) -> Result<(), String> {
        let name = name.to_string();
        tokio::task::spawn_blocking(move || {
            let launched = spawn_detached("gtk-launch", &[&name]).or_else(|_| spawn_detached(&name, &[]));
            launched.map_err(|e| format!("無法開啟「{name}」：{e}。名稱要和桌面入口或 PATH 裡的程式一致"))
        })
        .await
        .map_err(|e| format!("內部錯誤：{e}"))?
    }

    async fn open_url(&self, url: &str, app: Option<&str>) -> Result<(), String> {
        let (url, app) = (url.to_string(), app.map(str::to_string));
        tokio::task::spawn_blocking(move || {
            let r = match &app {
                Some(a) => spawn_detached(a, &[&url]),
                None => spawn_detached("xdg-open", &[&url]),
            };
            r.map_err(|e| format!("無法開啟網址：{e}"))
        })
        .await
        .map_err(|e| format!("內部錯誤：{e}"))?
    }
}
