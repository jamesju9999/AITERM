//! Linux（X11）後端：EWMH 列視窗與切換前景、XTEST 送滑鼠鍵盤、`GetImage` 截圖、
//! 以 `tesseract` 命令列辨識畫面文字。設計見 docs/superpowers/specs/2026-10-13-computer-use-linux-design.md。
//!
//! 沒有無障礙樹：Linux 的 AT-SPI 要走 D-Bus 且各工具套件的支援參差，第一版刻意不做。`snapshot` 只回傳
//! 視窗層級，共用邏輯會把它判定為「幾乎沒有無障礙資訊」並引導模型改用 OCR（`computer_read_screen`、
//! `computer_click_text`）與圖示偵測；這兩條路徑本來就是為 Chromium／Qt 這類 App 設計的。
//!
//! Wayland 不支援（見 `linuxmap::session_support`）。
//!
//! **這份程式碼在開發當下只通過型別檢查（針對 x86_64-unknown-linux-gnu）與純邏輯的單元測試
//! （`linuxmap.rs`），尚未在真實的 Linux 桌面上執行。** 在實機驗收前不可宣稱已支援。

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, ImageFormat, Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

use super::backend::*;
use super::icons::{detect_icons, Region};
use super::{linuxmap, winmap};

const MAX_WINDOWS_PER_APP: usize = 5;

// X11 事件型別碼（XTEST fake_input 的 `type` 參數）。
const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;
const BUTTON_PRESS: u8 = 4;
const BUTTON_RELEASE: u8 = 5;
const MOTION_NOTIFY: u8 = 6;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("內部錯誤：{e}"))?
}

fn xerr(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what}：{e}")
}

/// 一條 X 連線與常用的 atom。每次呼叫各自連線（連線很便宜），不跨執行緒共用。
struct X {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
}

struct Atoms {
    client_list: u32,
    active_window: u32,
    wm_pid: u32,
    wm_name: u32,
    wm_class: u32,
    utf8: u32,
    wm_state: u32,
    hidden: u32,
    window_type: u32,
    type_normal: u32,
}

fn atom(conn: &RustConnection, name: &str) -> Result<u32, String> {
    Ok(conn
        .intern_atom(false, name.as_bytes())
        .map_err(|e| xerr("intern_atom", e))?
        .reply()
        .map_err(|e| xerr("intern_atom", e))?
        .atom)
}

fn connect() -> Result<X, String> {
    linuxmap::session_support(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("DISPLAY").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    )?;
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("無法連線到 X11 顯示：{e}"))?;
    let root = conn.setup().roots[screen_num].root;
    let atoms = Atoms {
        client_list: atom(&conn, "_NET_CLIENT_LIST")?,
        active_window: atom(&conn, "_NET_ACTIVE_WINDOW")?,
        wm_pid: atom(&conn, "_NET_WM_PID")?,
        wm_name: atom(&conn, "_NET_WM_NAME")?,
        wm_class: u32::from(AtomEnum::WM_CLASS),
        utf8: atom(&conn, "UTF8_STRING")?,
        wm_state: atom(&conn, "_NET_WM_STATE")?,
        hidden: atom(&conn, "_NET_WM_STATE_HIDDEN")?,
        window_type: atom(&conn, "_NET_WM_WINDOW_TYPE")?,
        type_normal: atom(&conn, "_NET_WM_WINDOW_TYPE_NORMAL")?,
    };
    Ok(X { conn, root, atoms })
}

fn prop32(x: &X, win: Window, prop: u32, ty: impl Into<u32>) -> Option<Vec<u32>> {
    let r = x.conn.get_property(false, win, prop, ty, 0, 4096).ok()?.reply().ok()?;
    r.value32().map(|v| v.collect())
}

fn prop_bytes(x: &X, win: Window, prop: u32, ty: impl Into<u32>) -> Option<Vec<u8>> {
    let r = x.conn.get_property(false, win, prop, ty, 0, 4096).ok()?.reply().ok()?;
    (!r.value.is_empty()).then_some(r.value)
}

#[derive(Clone)]
struct Win {
    id: Window,
    pid: u32,
    title: String,
    class: Option<String>,
}

/// 使用者看得到的一般視窗（EWMH `_NET_CLIENT_LIST`），由下到上（堆疊順序，與規格相反，所以後面會反轉）。
fn client_windows(x: &X) -> Vec<Win> {
    let ids = prop32(x, x.root, x.atoms.client_list, AtomEnum::WINDOW).unwrap_or_default();
    let mut out = Vec::new();
    for id in ids {
        // 被縮到最小的視窗不算「可見」。
        if prop32(x, id, x.atoms.wm_state, AtomEnum::ATOM).map(|s| s.contains(&x.atoms.hidden)).unwrap_or(false) {
            continue;
        }
        // 只要一般視窗（沒設定類型時視為一般）；選單、工具列、通知不要。
        if let Some(types) = prop32(x, id, x.atoms.window_type, AtomEnum::ATOM) {
            if !types.is_empty() && !types.contains(&x.atoms.type_normal) {
                continue;
            }
        }
        let pid = prop32(x, id, x.atoms.wm_pid, AtomEnum::CARDINAL).and_then(|v| v.first().copied()).unwrap_or(0);
        if pid == 0 {
            continue;
        }
        let title = prop_bytes(x, id, x.atoms.wm_name, x.atoms.utf8)
            .or_else(|| prop_bytes(x, id, u32::from(AtomEnum::WM_NAME), AtomEnum::STRING))
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        let class = prop_bytes(x, id, x.atoms.wm_class, AtomEnum::STRING).and_then(|b| linuxmap::app_name_from_wm_class(&b));
        out.push(Win { id, pid, title, class });
    }
    out.reverse(); // 最上層在前
    out
}

fn process_name(pid: u32, fallback: Option<&str>) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .and_then(|c| linuxmap::app_name_from_comm(&c))
        .or_else(|| fallback.map(str::to_string))
        .unwrap_or_else(|| format!("pid{pid}"))
}

fn active_window(x: &X) -> Option<Window> {
    prop32(x, x.root, x.atoms.active_window, AtomEnum::WINDOW).and_then(|v| v.first().copied()).filter(|w| *w != 0)
}

fn active_pid(x: &X) -> Option<u32> {
    let w = active_window(x)?;
    prop32(x, w, x.atoms.wm_pid, AtomEnum::CARDINAL).and_then(|v| v.first().copied())
}

fn main_window(x: &X, pid: u32) -> Option<Win> {
    let wins = client_windows(x);
    let active = active_window(x);
    wins.iter().find(|w| w.pid == pid && Some(w.id) == active).cloned().or_else(|| wins.into_iter().find(|w| w.pid == pid))
}

/// 視窗在螢幕上的位置（含邊框內側，轉成根視窗座標）。
fn window_geometry(x: &X, win: Window) -> Option<(i32, i32, i32, i32)> {
    let g = x.conn.get_geometry(win).ok()?.reply().ok()?;
    let t = x.conn.translate_coordinates(win, x.root, 0, 0).ok()?.reply().ok()?;
    let (w, h) = (g.width as i32, g.height as i32);
    (w > 0 && h > 0).then_some((t.dst_x as i32, t.dst_y as i32, w, h))
}

fn screen_size(x: &X) -> (i32, i32) {
    let s = &x.conn.setup().roots[0];
    (s.width_in_pixels as i32, s.height_in_pixels as i32)
}

// ── 截圖 ─────────────────────────────────────────────────────────────────────────────

struct Bgra {
    data: Vec<u8>,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
}

fn capture(x: &X, rx: i32, ry: i32, w: i32, h: i32) -> Result<Bgra, String> {
    let (sw, sh) = screen_size(x);
    let (rx, ry, w, h) = winmap::intersect((rx, ry, w, h), (0, 0, sw, sh)).ok_or("範圍在螢幕之外")?;
    let img = x
        .conn
        .get_image(ImageFormat::Z_PIXMAP, x.root, rx as i16, ry as i16, w as u16, h as u16, !0)
        .map_err(|e| xerr("GetImage", e))?
        .reply()
        .map_err(|e| xerr("GetImage", e))?;
    // 常見的 ZPixmap 是每像素 4 位元組（BGRX）；深度 24／32 都一樣。其他格式就老實回報。
    if img.data.len() < (w as usize) * (h as usize) * 4 {
        return Err("螢幕影像格式不支援（預期每像素 32 位元）".to_string());
    }
    Ok(Bgra { data: img.data, width: w, height: h, x: rx, y: ry })
}

/// 要擷取的範圍：指定 App 的主視窗，或整個螢幕。回傳 (x, y, w, h)。
fn capture_rect(x: &X, pid: Option<u32>) -> Result<(i32, i32, i32, i32), String> {
    match pid {
        Some(pid) => {
            let win = main_window(x, pid).ok_or("這個 App 目前沒有可見的視窗（可能被最小化、在其他工作區，或還沒打開視窗）")?;
            window_geometry(x, win.id).ok_or_else(|| "讀不到視窗位置".to_string())
        }
        None => {
            let (w, h) = screen_size(x);
            Ok((0, 0, w, h))
        }
    }
}

// ── XTEST 輸入 ───────────────────────────────────────────────────────────────────────

fn fake(x: &X, ty: u8, detail: u8, rx: i16, ry: i16) -> Result<(), String> {
    x.conn
        .xtest_fake_input(ty, detail, 0, x.root, rx, ry, 0)
        .map_err(|e| xerr("xtest", e))?;
    Ok(())
}

fn flush(x: &X) -> Result<(), String> {
    x.conn.flush().map_err(|e| xerr("flush", e))
}

fn move_pointer(x: &X, px: f64, py: f64) -> Result<(), String> {
    let (w, h) = screen_size(x);
    let cx = (px.round() as i32).clamp(0, w - 1) as i16;
    let cy = (py.round() as i32).clamp(0, h - 1) as i16;
    fake(x, MOTION_NOTIFY, 0, cx, cy)?;
    flush(x)
}

fn click_button(x: &X, button: u8) -> Result<(), String> {
    fake(x, BUTTON_PRESS, button, 0, 0)?;
    flush(x)?;
    std::thread::sleep(Duration::from_millis(30));
    fake(x, BUTTON_RELEASE, button, 0, 0)?;
    flush(x)?;
    std::thread::sleep(Duration::from_millis(30));
    Ok(())
}

/// keysym → keycode（目前的鍵盤配置裡有這個 keysym 的鍵），並回傳是否需要 Shift。
fn keycode_for(x: &X, keysym: u32) -> Result<Option<(u8, bool)>, String> {
    let setup = x.conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = x
        .conn
        .get_keyboard_mapping(min, max - min + 1)
        .map_err(|e| xerr("get_keyboard_mapping", e))?
        .reply()
        .map_err(|e| xerr("get_keyboard_mapping", e))?;
    let per = map.keysyms_per_keycode as usize;
    for (i, chunk) in map.keysyms.chunks(per).enumerate() {
        if let Some(col) = chunk.iter().position(|k| *k == keysym) {
            // 第 0、2 欄是未按 Shift，第 1、3 欄是按 Shift。
            return Ok(Some((min + i as u8, col % 2 == 1)));
        }
    }
    Ok(None)
}

fn press_keysym(x: &X, keysym: u32, extra_shift: bool) -> Result<(), String> {
    let (kc, shift) = keycode_for(x, keysym)?.ok_or_else(|| format!("目前的鍵盤配置沒有 keysym {keysym:#x}"))?;
    let shift_kc = keycode_for(x, linuxmap::XK_SHIFT_L)?.map(|(k, _)| k);
    let need_shift = (shift || extra_shift) && shift_kc.is_some();
    if need_shift {
        fake(x, KEY_PRESS, shift_kc.unwrap_or(0), 0, 0)?;
    }
    fake(x, KEY_PRESS, kc, 0, 0)?;
    fake(x, KEY_RELEASE, kc, 0, 0)?;
    if need_shift {
        fake(x, KEY_RELEASE, shift_kc.unwrap_or(0), 0, 0)?;
    }
    flush(x)
}

/// 目前鍵盤配置裡沒有的字元（中文、emoji…）：暫時把一個沒用到的 keycode 對應到該字元的 Unicode keysym，
/// 按一下，再還原（xdotool 的做法）。
fn type_unmapped(x: &X, keysym: u32) -> Result<(), String> {
    let setup = x.conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = x
        .conn
        .get_keyboard_mapping(min, max - min + 1)
        .map_err(|e| xerr("get_keyboard_mapping", e))?
        .reply()
        .map_err(|e| xerr("get_keyboard_mapping", e))?;
    let per = map.keysyms_per_keycode as usize;
    let spare = map
        .keysyms
        .chunks(per)
        .enumerate()
        .rev()
        .find(|(_, c)| c.iter().all(|k| *k == 0))
        .map(|(i, _)| min + i as u8)
        .ok_or("找不到空的 keycode 可以暫時對應這個字元")?;
    let mut syms = vec![0u32; per];
    syms[0] = keysym;
    x.conn.change_keyboard_mapping(1, spare, per as u8, &syms).map_err(|e| xerr("change_keyboard_mapping", e))?;
    flush(x)?;
    std::thread::sleep(Duration::from_millis(30)); // 讓 X server 與目標程式更新鍵盤對應
    fake(x, KEY_PRESS, spare, 0, 0)?;
    fake(x, KEY_RELEASE, spare, 0, 0)?;
    flush(x)?;
    std::thread::sleep(Duration::from_millis(30));
    let zero = vec![0u32; per];
    x.conn.change_keyboard_mapping(1, spare, per as u8, &zero).map_err(|e| xerr("change_keyboard_mapping", e))?;
    flush(x)
}

// ── OCR（tesseract）──────────────────────────────────────────────────────────────────

pub(super) fn tesseract_ocr(png: &[u8]) -> Result<Vec<ScreenText>, String> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("aiterm-ocr-{}-{}.png", std::process::id(), png.len()));
    std::fs::write(&path, png).map_err(|e| xerr("寫入暫存影像失敗", e))?;
    // 語言：中英文都要。沒裝繁中語言包時退回只用英文。
    let run = |lang: &str| {
        std::process::Command::new("tesseract")
            .arg(&path)
            .arg("stdout")
            .args(["-l", lang, "tsv"])
            .stdin(Stdio::null())
            .output()
    };
    let out = run("eng+chi_tra+chi_sim").and_then(|o| if o.status.success() { Ok(o) } else { run("eng") });
    let _ = std::fs::remove_file(&path);
    match out {
        Ok(o) if o.status.success() => Ok(linuxmap::parse_tesseract_tsv(&String::from_utf8_lossy(&o.stdout))),
        Ok(o) => Err(format!("tesseract 執行失敗：{}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(
            "找不到 tesseract。文字辨識需要安裝它，例如：sudo apt install tesseract-ocr tesseract-ocr-chi-tra（Fedora：sudo dnf install tesseract tesseract-langpack-chi_tra）"
                .to_string(),
        ),
        Err(e) => Err(xerr("無法執行 tesseract", e)),
    }
}

// ── Backend ──────────────────────────────────────────────────────────────────────────

pub struct LinuxBackend;

fn pid_of(pid: i32) -> u32 {
    pid.max(0) as u32
}

#[async_trait]
impl Backend for LinuxBackend {
    fn permissions(&self) -> Permissions {
        // X11 沒有授權機制：連得上顯示就能截圖與送輸入。Wayland 由 `unsupported_reason` 擋下。
        Permissions { accessibility: true, screen_recording: true }
    }

    fn unsupported_reason(&self) -> Option<String> {
        linuxmap::session_support(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("DISPLAY").ok().as_deref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        )
        .err()
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, String> {
        blocking(|| {
            let x = connect()?;
            let active = active_pid(&x);
            let mut seen: Vec<u32> = Vec::new();
            let mut apps = Vec::new();
            for w in client_windows(&x) {
                if seen.contains(&w.pid) {
                    continue;
                }
                seen.push(w.pid);
                let name = process_name(w.pid, w.class.as_deref());
                apps.push(AppInfo {
                    // WM_CLASS（如 `firefox`）當別名之一：行程名稱常是 `firefox-bin`、`gnome-terminal-`。
                    localized: w.class.clone().filter(|c| *c != name.to_lowercase()),
                    bundle_id: w.class.clone(),
                    name,
                    pid: w.pid as i32,
                    frontmost: Some(w.pid) == active,
                });
            }
            Ok(apps)
        })
        .await
    }

    async fn snapshot(&self, pid: i32, _max_depth: usize) -> Result<ElementNode, String> {
        let pid = pid_of(pid);
        blocking(move || {
            let x = connect()?;
            let mut wins: Vec<Win> = client_windows(&x).into_iter().filter(|w| w.pid == pid).collect();
            wins.truncate(MAX_WINDOWS_PER_APP);
            if wins.is_empty() {
                return Err("這個 App 沒有可讀取的視窗".to_string());
            }
            let mut root = ElementNode {
                id: "0".into(),
                role: "AXApplication".into(),
                title: process_name(pid, wins[0].class.as_deref()),
                ..Default::default()
            };
            for (i, w) in wins.iter().enumerate() {
                let (gx, gy, gw, gh) = window_geometry(&x, w.id).unwrap_or((0, 0, 0, 0));
                root.children.push(ElementNode {
                    id: format!("0.{i}"),
                    role: "AXWindow".into(),
                    title: w.title.clone(),
                    x: gx as f64,
                    y: gy as f64,
                    w: gw as f64,
                    h: gh as f64,
                    children: vec![],
                });
            }
            Ok(root)
        })
        .await
    }

    async fn element_info(&self, _pid: i32, _id: &str) -> Result<ElementNode, String> {
        Err("Linux 沒有無障礙元件樹；請用 computer_read_screen／computer_click_text／computer_click_icon".to_string())
    }

    async fn element_at(&self, _x: f64, _y: f64) -> Result<Option<ElementNode>, String> {
        Ok(None)
    }

    async fn screenshot(&self, pid: Option<i32>) -> Result<Shot, String> {
        let pid = pid.map(pid_of);
        blocking(move || {
            let x = connect()?;
            let (rx, ry, w, h) = capture_rect(&x, pid)?;
            let img = capture(&x, rx, ry, w, h)?;
            let png = winmap::encode_png_rgba(img.width as u32, img.height as u32, &winmap::bgra_to_rgba(&img.data))?;
            Ok(Shot { png, width: img.width as u32, height: img.height as u32, origin_x: img.x as f64, origin_y: img.y as f64 })
        })
        .await
    }

    async fn activate(&self, pid: i32) -> Result<(), String> {
        let pid = pid_of(pid);
        let already = blocking(move || {
            let x = connect()?;
            if active_pid(&x) == Some(pid) {
                return Ok(true);
            }
            let win = main_window(&x, pid).ok_or("這個 App 沒有可顯示的視窗")?;
            // EWMH：向根視窗送 `_NET_ACTIVE_WINDOW` 要求視窗管理員切換（source=2 表示來自「應用程式的 pager」）。
            let ev = ClientMessageEvent::new(32, win.id, x.atoms.active_window, [2, CURRENT_TIME, 0, 0, 0]);
            x.conn
                .send_event(false, x.root, EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT, ev)
                .map_err(|e| xerr("send_event", e))?;
            x.conn.flush().map_err(|e| xerr("flush", e))?;
            Ok(false)
        })
        .await?;
        if already {
            return Ok(());
        }
        // 輪詢確認；確認不到明確報錯，不把按鍵送進別的視窗。
        for _ in 0..40 {
            let ok = blocking(move || Ok(active_pid(&connect()?) == Some(pid))).await.unwrap_or(false);
            if ok {
                tokio::time::sleep(Duration::from_millis(40)).await;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Err("無法把這個 App 切到最前面（視窗管理員可能不支援 EWMH，或有別的視窗擋住）".to_string())
    }

    async fn press_element(&self, _pid: i32, _id: &str) -> Result<(), String> {
        Err("Linux 沒有無障礙元件可以直接啟動".to_string())
    }

    async fn click_at(&self, px: f64, py: f64, button: MouseButton, double: bool) -> Result<(), String> {
        blocking(move || {
            let x = connect()?;
            move_pointer(&x, px, py)?;
            std::thread::sleep(Duration::from_millis(40));
            let b = match button {
                MouseButton::Left => 1,
                MouseButton::Right => 3,
            };
            for _ in 0..if double { 2 } else { 1 } {
                click_button(&x, b)?;
            }
            Ok(())
        })
        .await
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        let text = text.to_string();
        blocking(move || {
            let x = connect()?;
            for c in text.chars() {
                // 換行與 Tab 要送對應的按鍵，不是 Unicode 字元。
                let keysym = match c {
                    '\n' | '\r' => 0xff0d,
                    '\t' => 0xff09,
                    _ => linuxmap::char_keysym(c),
                };
                match keycode_for(&x, keysym)? {
                    Some(_) => press_keysym(&x, keysym, false)?,
                    None => type_unmapped(&x, keysym)?,
                }
                std::thread::sleep(Duration::from_millis(8));
            }
            Ok(())
        })
        .await
    }

    async fn focus_element(&self, _pid: i32, _id: &str) -> Result<(), String> {
        Err("Linux 沒有無障礙元件可以聚焦；請先用 computer_click_text 點到輸入欄".to_string())
    }

    async fn key(&self, combo: &KeyCombo) -> Result<(), String> {
        let combo = combo.clone();
        blocking(move || {
            let x = connect()?;
            let keysym = linuxmap::keysym_for_key(&combo.key).ok_or_else(|| format!("不支援的按鍵「{}」", combo.key))?;
            let (kc, needs_shift) = keycode_for(&x, keysym)?.ok_or_else(|| format!("目前的鍵盤配置沒有「{}」這個鍵", combo.key))?;
            let mut mods: Vec<u8> = Vec::new();
            for m in linuxmap::modifier_keysyms(&combo) {
                if let Some((mk, _)) = keycode_for(&x, m)? {
                    mods.push(mk);
                }
            }
            if needs_shift && !combo.shift {
                if let Some((sk, _)) = keycode_for(&x, linuxmap::XK_SHIFT_L)? {
                    mods.push(sk);
                }
            }
            for m in &mods {
                fake(&x, KEY_PRESS, *m, 0, 0)?;
            }
            fake(&x, KEY_PRESS, kc, 0, 0)?;
            fake(&x, KEY_RELEASE, kc, 0, 0)?;
            for m in mods.iter().rev() {
                fake(&x, KEY_RELEASE, *m, 0, 0)?;
            }
            flush(&x)
        })
        .await
    }

    async fn scroll(&self, dx: i32, dy: i32, at: Option<(f64, f64)>) -> Result<(), String> {
        blocking(move || {
            let x = connect()?;
            if let Some((px, py)) = at {
                move_pointer(&x, px, py)?;
                std::thread::sleep(Duration::from_millis(40));
            }
            for (button, count) in linuxmap::scroll_buttons(dx, dy) {
                for _ in 0..count {
                    fake(&x, BUTTON_PRESS, button, 0, 0)?;
                    fake(&x, BUTTON_RELEASE, button, 0, 0)?;
                }
            }
            flush(&x)
        })
        .await
    }

    async fn focused_text(&self, _pid: i32) -> Result<Option<FocusedText>, String> {
        Ok(None) // 沒有無障礙樹，讀不到焦點欄位內容
    }

    async fn read_screen(&self, pid: Option<i32>) -> Result<Vec<ScreenText>, String> {
        let pid = pid.map(pid_of);
        blocking(move || {
            let x = connect()?;
            let (rx, ry, w, h) = capture_rect(&x, pid)?;
            let img = capture(&x, rx, ry, w, h)?;
            let png = winmap::encode_png_rgba(img.width as u32, img.height as u32, &winmap::bgra_to_rgba(&img.data))?;
            let lines = tesseract_ocr(&png)?;
            Ok(lines
                .into_iter()
                .map(|l| ScreenText { text: l.text, x: img.x as f64 + l.x, y: img.y as f64 + l.y, w: l.w, h: l.h })
                .collect())
        })
        .await
    }

    async fn find_icons(&self, pid: i32, region: Region) -> Result<Vec<IconRect>, String> {
        let pid = pid_of(pid);
        blocking(move || {
            let x = connect()?;
            let (rx, ry, w, h) = capture_rect(&x, Some(pid))?;
            let img = capture(&x, rx, ry, w, h)?;
            let lum = winmap::bgra_to_luminance(&img.data);
            Ok(detect_icons(&lum, img.width as usize, img.height as usize, 1.0, region)
                .into_iter()
                .map(|r| IconRect { x: img.x as f64 + r.x, y: img.y as f64 + r.y, w: r.w, h: r.h })
                .collect())
        })
        .await
    }

    async fn open_app(&self, name: &str) -> Result<(), String> {
        let name = name.to_string();
        blocking(move || {
            // 先試桌面入口（`gtk-launch firefox`：認 .desktop 檔名），不行再直接執行同名程式。
            // 名稱是獨立參數、沒有經過 shell；呼叫端已拒絕 `-` 開頭、路徑分隔字元與引號。
            let launched = spawn_detached("gtk-launch", &[&name]).or_else(|_| spawn_detached(&name, &[]));
            launched.map_err(|e| format!("無法開啟「{name}」：{e}。名稱要和桌面入口或 PATH 裡的程式一致"))
        })
        .await
    }

    async fn open_url(&self, url: &str, app: Option<&str>) -> Result<(), String> {
        let (url, app) = (url.to_string(), app.map(str::to_string));
        blocking(move || {
            let r = match &app {
                Some(a) => spawn_detached(a, &[&url]),
                None => spawn_detached("xdg-open", &[&url]),
            };
            r.map_err(|e| format!("無法開啟網址：{e}"))
        })
        .await
    }
}

/// 啟動一個行程並不等它結束（它是要被使用者看到的 App，不是子程序）。
pub(super) fn spawn_detached(program: &str, args: &[&str]) -> std::io::Result<()> {
    std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[allow(dead_code)]
fn _keep_xproto_imports(_: xproto::Window) {}
