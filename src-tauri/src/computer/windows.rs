//! Windows 後端：UI Automation 讀元件樹與按按鈕、`SendInput` 送滑鼠鍵盤、GDI 截圖、
//! `Windows.Media.Ocr` 辨識畫面文字。設計見
//! docs/superpowers/specs/2026-10-13-computer-use-windows-design.md。
//!
//! **這份程式碼在開發當下只通過型別檢查（針對 x86_64-pc-windows-msvc）與純邏輯的單元測試
//! （`winmap.rs`），尚未在真實的 Windows 上執行。** 在實機驗收前不可宣稱已支援。
//!
//! 所有 COM／WinRT 呼叫都放進 `spawn_blocking`：UI Automation 可能因對方程式無回應而卡住，
//! 而且 COM 物件有執行緒親和性（每條執行緒各自初始化 COM、各自建立 `IUIAutomation`）。
//! 座標一律是實體像素（Tauri 程式是 per-monitor DPI aware），截圖、UIA、`SendInput` 同一個座標系。

use std::ffi::c_void;
use std::time::Duration;

use ::windows::core::{Interface, HSTRING, PCWSTR, PWSTR};
use ::windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use ::windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
};
use ::windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use ::windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use ::windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, IUIAutomationTreeWalker, IUIAutomationValuePattern,
    UIA_ExpandCollapsePatternId, UIA_InvokePatternId, UIA_SelectionItemPatternId, UIA_TogglePatternId, UIA_ValuePatternId,
};
use ::windows::Win32::UI::HiDpi::GetDpiForWindow;
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT,
    MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
};
use ::windows::Win32::UI::Shell::ShellExecuteW;
use ::windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetSystemMetrics, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow,
    ShowWindow, GWL_EXSTYLE, GW_OWNER, SM_CXSCREEN, SM_CYSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SW_RESTORE, SW_SHOWNORMAL, WS_EX_TOOLWINDOW,
};
use async_trait::async_trait;

use super::backend::*;
use super::icons::{detect_icons, Region};
use super::winmap;

// ── 小工具 ───────────────────────────────────────────────────────────────────────────

const MAX_NODES: usize = 2000;
const MAX_WINDOWS_PER_APP: usize = 5;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(move || {
        init_com();
        f()
    })
    .await
    .map_err(|e| format!("內部錯誤：{e}"))?
}

/// 每條執行緒初始化一次 COM（多執行緒模型）。已經以別的模型初始化過就算了——UIA 兩種都能用。
fn init_com() {
    thread_local! { static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    DONE.with(|d| {
        if !d.get() {
            // SAFETY: 初始化目前執行緒的 COM；結果刻意忽略（RPC_E_CHANGED_MODE 表示已初始化）。
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            d.set(true);
        }
    });
}

fn werr(what: &str, e: ::windows::core::Error) -> String {
    format!("{what}：{e}")
}

fn rect_xywh(r: &RECT) -> (i32, i32, i32, i32) {
    (r.left, r.top, r.right - r.left, r.bottom - r.top)
}

// ── 視窗與行程 ───────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Win {
    hwnd: HWND,
    pid: u32,
    title: String,
}

// HWND 只是一個整數控制代碼，跨執行緒傳遞沒有問題。
unsafe impl Send for Win {}

/// 使用者看得到的頂層視窗，由上到下（Z 順序）。
fn top_windows() -> Vec<Win> {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> ::windows::core::BOOL {
        // SAFETY: lparam 是 `top_windows` 傳進來的 `&mut Vec<Win>`，列舉期間一直有效。
        let out = unsafe { &mut *(lparam.0 as *mut Vec<Win>) };
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        if visible {
            let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
            let tool = ex & WS_EX_TOOLWINDOW.0 != 0;
            let has_owner = unsafe { GetWindow(hwnd, GW_OWNER) }.map(|h| !h.0.is_null()).unwrap_or(false);
            let mut cloaked: u32 = 0;
            let _ = unsafe {
                DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut c_void, std::mem::size_of::<u32>() as u32)
            };
            let len = unsafe { GetWindowTextLengthW(hwnd) };
            let mut title = String::new();
            if len > 0 {
                let mut buf = vec![0u16; len as usize + 1];
                let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
                title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            }
            if winmap::is_app_window(visible, cloaked != 0, tool, has_owner, &title) {
                let mut pid = 0u32;
                unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
                if pid != 0 {
                    out.push(Win { hwnd, pid, title });
                }
            }
        }
        true.into()
    }
    let mut v: Vec<Win> = Vec::new();
    // SAFETY: 回呼只在本呼叫期間使用 `v`。
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut v as *mut Vec<Win> as isize)) };
    v
}

fn foreground_pid() -> Option<u32> {
    // SAFETY: 純查詢。
    let h = unsafe { GetForegroundWindow() };
    if h.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

fn process_path(pid: u32) -> Option<String> {
    // SAFETY: 取得唯讀查詢權限的行程控制代碼，用完立刻關閉。
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// 這個 pid 的主視窗：前景視窗屬於它就用前景的，否則取 Z 順序最上面的一個。
fn main_window(pid: u32) -> Option<Win> {
    let wins = top_windows();
    let fg = unsafe { GetForegroundWindow() };
    wins.iter().find(|w| w.pid == pid && w.hwnd == fg).cloned().or_else(|| wins.into_iter().find(|w| w.pid == pid))
}

/// 這個 App 的視窗，依 HWND 數值排序——排序要穩定，元件 id 的第一段是它在這份清單裡的索引，
/// 不能因為 Z 順序改變（例如剛切到前景）就跟著變。
fn app_windows(pid: u32) -> Vec<Win> {
    let mut v: Vec<Win> = top_windows().into_iter().filter(|w| w.pid == pid).collect();
    v.sort_by_key(|w| w.hwnd.0 as usize);
    v.truncate(MAX_WINDOWS_PER_APP);
    v
}

fn window_frame(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    let mut r = RECT::default();
    // 優先用 DWM 的實際外框（不含看不見的調整大小邊框與陰影），失敗再退回 GetWindowRect。
    let ok = unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut c_void, std::mem::size_of::<RECT>() as u32)
    }
    .is_ok();
    if !ok && unsafe { GetWindowRect(hwnd, &mut r) }.is_err() {
        return None;
    }
    let (x, y, w, h) = rect_xywh(&r);
    (w > 0 && h > 0).then_some((x, y, w, h))
}

fn virtual_desktop() -> (i32, i32, i32, i32) {
    // SAFETY: 純查詢。
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

fn force_foreground(hwnd: HWND) {
    // SAFETY: 標準的「把視窗帶到前景」流程；Windows 的前景鎖會擋掉一般的 SetForegroundWindow，
    // 暫時把輸入佇列接到目前前景視窗的執行緒就能通過。
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let fg = GetForegroundWindow();
        let cur = GetCurrentThreadId();
        let fg_thread = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let attached = fg_thread != 0 && fg_thread != cur && AttachThreadInput(cur, fg_thread, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(cur, fg_thread, false);
        }
    }
}

// ── 截圖（GDI）──────────────────────────────────────────────────────────────────────

struct Bgra {
    data: Vec<u8>,
    width: i32,
    height: i32,
    /// 這張圖在螢幕上的左上角。
    x: i32,
    y: i32,
}

fn capture_bgra(x: i32, y: i32, w: i32, h: i32) -> Result<Bgra, String> {
    if w <= 0 || h <= 0 {
        return Err("截圖範圍是空的".to_string());
    }
    // SAFETY: 標準的 GDI 螢幕擷取；所有取得的資源都在離開前釋放。
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("無法取得螢幕 DC".to_string());
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let blt = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
        let mut data = vec![0u8; (w as usize) * (h as usize) * 4];
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 負值＝由上到下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let lines = GetDIBits(mem, bmp, 0, h as u32, Some(data.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        blt.map_err(|e| werr("螢幕擷取失敗", e))?;
        if lines == 0 {
            return Err("讀取畫面像素失敗".to_string());
        }
        Ok(Bgra { data, width: w, height: h, x, y })
    }
}

/// 要擷取的範圍：指定 App 的主視窗，或整個主螢幕；夾在虛擬桌面內。回傳 (x, y, w, h, dpi 縮放比)。
fn capture_target(pid: Option<u32>) -> Result<(i32, i32, i32, i32, f64), String> {
    let (rect, scale) = match pid {
        Some(pid) => {
            let win = main_window(pid).ok_or("這個 App 目前沒有可見的視窗（可能被最小化、在其他虛擬桌面，或還沒打開視窗）")?;
            let frame = window_frame(win.hwnd).ok_or("讀不到視窗位置")?;
            // SAFETY: 純查詢。
            let dpi = unsafe { GetDpiForWindow(win.hwnd) };
            (frame, winmap::scale_from_dpi(dpi))
        }
        None => {
            // SAFETY: 純查詢。
            let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
            ((0, 0, w, h), 1.0)
        }
    };
    let (x, y, w, h) = winmap::intersect(rect, virtual_desktop()).ok_or("視窗在所有螢幕之外")?;
    Ok((x, y, w, h, scale))
}

// ── UI Automation ───────────────────────────────────────────────────────────────────

fn uia() -> Result<IUIAutomation, String> {
    // SAFETY: 建立行程內的 UI Automation 物件；COM 已在本執行緒初始化。
    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.map_err(|e| werr("無法啟動 UI Automation", e))
}

fn element_value(el: &IUIAutomationElement) -> Option<String> {
    // SAFETY: 讀取 ValuePattern；元素沒有這個 pattern 時回錯誤。
    unsafe {
        let p: IUIAutomationValuePattern = el.GetCurrentPattern(UIA_ValuePatternId).ok()?.cast().ok()?;
        p.CurrentValue().ok().map(|b| b.to_string())
    }
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n { format!("{t}…") } else { t }
}

fn describe(el: &IUIAutomationElement, id: &str) -> ElementNode {
    // SAFETY: 純屬性讀取；任何失敗都退回空值，不中斷整棵樹。
    unsafe {
        let role = winmap::uia_role(el.CurrentControlType().map(|t| t.0).unwrap_or(0)).to_string();
        let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        let title = if name.trim().is_empty() {
            element_value(el).map(|v| truncate(v.trim(), 80)).unwrap_or_default()
        } else {
            truncate(name.trim(), 80)
        };
        let r = el.CurrentBoundingRectangle().unwrap_or_default();
        let (x, y, w, h) = rect_xywh(&r);
        ElementNode { id: id.to_string(), role, title, x: x as f64, y: y as f64, w: w as f64, h: h as f64, children: vec![] }
    }
}

fn children_of(walker: &IUIAutomationTreeWalker, el: &IUIAutomationElement) -> Vec<IUIAutomationElement> {
    let mut out = Vec::new();
    // SAFETY: 走訪 UIA 樹；沒有孩子或失敗都回空。
    unsafe {
        let mut cur = walker.GetFirstChildElement(el).ok();
        while let Some(c) = cur {
            cur = walker.GetNextSiblingElement(&c).ok();
            out.push(c);
            if out.len() >= 400 {
                break; // 單一節點的孩子數上限，避免巨大的清單吃光時間
            }
        }
    }
    out
}

fn build_tree(
    walker: &IUIAutomationTreeWalker,
    el: &IUIAutomationElement,
    id: &str,
    depth: usize,
    max_depth: usize,
    count: &mut usize,
) -> ElementNode {
    *count += 1;
    let mut node = describe(el, id);
    if depth < max_depth {
        for (i, child) in children_of(walker, el).iter().enumerate() {
            if *count >= MAX_NODES {
                break;
            }
            node.children.push(build_tree(walker, child, &format!("{id}.{i}"), depth + 1, max_depth, count));
        }
    }
    node
}

/// `0.2.1` → 第 2 個視窗（`app_windows` 的順序）底下第 1 個孩子。`0` 本身是 App 根節點（不是元素）。
fn resolve(pid: u32, id: &str, auto: &IUIAutomation) -> Result<IUIAutomationElement, String> {
    let stale = || "畫面已變，這個元件 id 已失效。請重新呼叫 computer_snapshot。".to_string();
    let mut parts = id.split('.');
    if parts.next() != Some("0") {
        return Err(format!("無效的元件 id「{id}」"));
    }
    let Some(win_idx) = parts.next() else {
        return Err("元件 id 要指到視窗或更深的元件".to_string());
    };
    let win_idx: usize = win_idx.parse().map_err(|_| format!("無效的元件 id「{id}」"))?;
    let win = app_windows(pid).into_iter().nth(win_idx).ok_or_else(stale)?;
    // SAFETY: 由 HWND 取得 UIA 元素並沿索引路徑往下走。
    unsafe {
        let walker = auto.ControlViewWalker().map_err(|e| werr("無法建立走訪器", e))?;
        let mut cur = auto.ElementFromHandle(win.hwnd).map_err(|e| werr("無法取得視窗元件", e))?;
        for p in parts {
            let idx: usize = p.parse().map_err(|_| format!("無效的元件 id「{id}」"))?;
            cur = children_of(&walker, &cur).into_iter().nth(idx).ok_or_else(stale)?;
        }
        Ok(cur)
    }
}

// ── SendInput ───────────────────────────────────────────────────────────────────────

fn send(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    // SAFETY: 送出合成輸入事件。
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err("輸入事件被系統擋下（目標視窗可能以系統管理員權限執行；Windows 不允許一般權限的程式對它送滑鼠鍵盤）".to_string())
    }
}

fn mouse_input(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn vk_event(vk: u16, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    if winmap::is_extended_key(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    key_input(vk, 0, flags)
}

fn move_to(x: f64, y: f64) -> INPUT {
    let (l, t, w, h) = virtual_desktop();
    let (ax, ay) = winmap::to_absolute(x, y, l, t, w, h);
    mouse_input(ax, ay, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK)
}

// ── 文字辨識（WinRT）────────────────────────────────────────────────────────────────

fn ocr_bgra(bgra: &[u8], w: i32, h: i32) -> Result<Vec<(String, f64, f64, f64, f64)>, String> {
    use ::windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use ::windows::Media::Ocr::OcrEngine;
    use ::windows::Storage::Streams::DataWriter;

    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
        "系統沒有可用的文字辨識語言。請到 設定 → 時間與語言 → 語言與區域，安裝要辨識的語言（含「文字辨識」功能）".to_string()
    })?;
    let writer = DataWriter::new().map_err(|e| werr("無法建立緩衝區", e))?;
    writer.WriteBytes(bgra).map_err(|e| werr("無法寫入影像", e))?;
    let buffer = writer.DetachBuffer().map_err(|e| werr("無法建立影像緩衝區", e))?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w, h)
        .map_err(|e| werr("無法建立點陣圖", e))?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .and_then(|op| op.get())
        .map_err(|e| werr("文字辨識失敗", e))?;
    let mut out = Vec::new();
    for line in result.Lines().map_err(|e| werr("讀取辨識結果失敗", e))? {
        let text = line.Text().map(|t| t.to_string()).unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        // 一行的外框 = 它所有字的外框聯集。
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        if let Ok(words) = line.Words() {
            for word in words {
                if let Ok(r) = word.BoundingRect() {
                    x0 = x0.min(r.X as f64);
                    y0 = y0.min(r.Y as f64);
                    x1 = x1.max((r.X + r.Width) as f64);
                    y1 = y1.max((r.Y + r.Height) as f64);
                }
            }
        }
        if x1 > x0 && y1 > y0 {
            out.push((text, x0, y0, x1 - x0, y1 - y0));
        }
    }
    Ok(out)
}

// ── Backend ─────────────────────────────────────────────────────────────────────────

pub struct WindowsBackend;

fn pid_of(pid: i32) -> u32 {
    pid.max(0) as u32
}

#[async_trait]
impl Backend for WindowsBackend {
    fn permissions(&self) -> Permissions {
        // Windows 沒有「輔助使用／螢幕錄製」這種授權；對系統管理員權限的視窗操作會被 UIPI 擋下，
        // 由 `send` 回報可操作的錯誤。
        Permissions { accessibility: true, screen_recording: true }
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, String> {
        blocking(|| {
            let fg = foreground_pid();
            let mut seen: Vec<u32> = Vec::new();
            let mut apps = Vec::new();
            for w in top_windows() {
                if seen.contains(&w.pid) {
                    continue;
                }
                seen.push(w.pid);
                let path = process_path(w.pid);
                let mut name = path.as_deref().and_then(winmap::process_name_from_path).unwrap_or_else(|| w.title.clone());
                // UWP／商店 App（計算機、設定…）的視窗都由 ApplicationFrameHost 代管，行程名稱對使用者與模型毫無意義；
                // 改用視窗標題。已知限制：同時開著多個 UWP App 時它們共用一個 pid，只會列出最上層的那個。
                if name.eq_ignore_ascii_case("ApplicationFrameHost") {
                    name = w.title.clone();
                }
                // 視窗標題當別名（「無標題 - 記事本」之類不完全等於，但至少讓清單看得出是哪個視窗）。
                apps.push(AppInfo {
                    name,
                    localized: None,
                    bundle_id: path.as_deref().and_then(winmap::app_id_from_path),
                    pid: w.pid as i32,
                    frontmost: Some(w.pid) == fg,
                });
            }
            Ok(apps)
        })
        .await
    }

    async fn snapshot(&self, pid: i32, max_depth: usize) -> Result<ElementNode, String> {
        let pid = pid_of(pid);
        blocking(move || {
            let auto = uia()?;
            // SAFETY: 建立走訪器並走訪 UIA 樹。
            let walker = unsafe { auto.ControlViewWalker() }.map_err(|e| werr("無法建立走訪器", e))?;
            let mut root = ElementNode {
                id: "0".into(),
                role: "AXApplication".into(),
                title: process_path(pid).as_deref().and_then(winmap::process_name_from_path).unwrap_or_default(),
                ..Default::default()
            };
            let mut count = 0usize;
            for (i, w) in app_windows(pid).iter().enumerate() {
                // SAFETY: 由 HWND 取得元素。
                let Ok(el) = (unsafe { auto.ElementFromHandle(w.hwnd) }) else { continue };
                root.children.push(build_tree(&walker, &el, &format!("0.{i}"), 1, max_depth, &mut count));
                if count >= MAX_NODES {
                    break;
                }
            }
            if root.children.is_empty() {
                return Err("這個 App 沒有可讀取的視窗".to_string());
            }
            Ok(root)
        })
        .await
    }

    async fn element_info(&self, pid: i32, id: &str) -> Result<ElementNode, String> {
        let (pid, id) = (pid_of(pid), id.to_string());
        blocking(move || {
            let auto = uia()?;
            let el = resolve(pid, &id, &auto)?;
            Ok(describe(&el, &id))
        })
        .await
    }

    async fn element_at(&self, x: f64, y: f64) -> Result<Option<ElementNode>, String> {
        blocking(move || {
            let auto = uia()?;
            // SAFETY: 依螢幕座標取得元素。
            Ok(unsafe { auto.ElementFromPoint(POINT { x: x as i32, y: y as i32 }) }.ok().map(|el| describe(&el, "?")))
        })
        .await
    }

    async fn screenshot(&self, pid: Option<i32>) -> Result<Shot, String> {
        let pid = pid.map(pid_of);
        blocking(move || {
            let (x, y, w, h, _) = capture_target(pid)?;
            let img = capture_bgra(x, y, w, h)?;
            let png = winmap::encode_png_rgba(img.width as u32, img.height as u32, &winmap::bgra_to_rgba(&img.data))?;
            Ok(Shot { png, width: img.width as u32, height: img.height as u32, origin_x: img.x as f64, origin_y: img.y as f64 })
        })
        .await
    }

    async fn activate(&self, pid: i32) -> Result<(), String> {
        let pid = pid_of(pid);
        blocking(move || {
            if foreground_pid() == Some(pid) {
                return Ok(());
            }
            let win = main_window(pid).ok_or("這個 App 沒有可顯示的視窗")?;
            force_foreground(win.hwnd);
            Ok(())
        })
        .await?;
        // 輪詢確認，而不是固定睡一下：確認不到（例如被系統對話框或更高權限視窗擋住）就明確報錯。
        for _ in 0..40 {
            if foreground_pid() == Some(pid) {
                tokio::time::sleep(Duration::from_millis(40)).await; // 讓視窗完成切換
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Err("無法把這個 App 切到最前面（可能有系統對話框擋住，或它以系統管理員權限執行）".to_string())
    }

    async fn press_element(&self, pid: i32, id: &str) -> Result<(), String> {
        let (pid, id) = (pid_of(pid), id.to_string());
        blocking(move || {
            let auto = uia()?;
            let el = resolve(pid, &id, &auto)?;
            // SAFETY: 依序嘗試常見的「啟動」pattern。
            unsafe {
                if let Ok(p) = el.GetCurrentPattern(UIA_InvokePatternId).and_then(|u| u.cast::<IUIAutomationInvokePattern>()) {
                    return p.Invoke().map_err(|e| werr("Invoke 失敗", e));
                }
                if let Ok(p) = el.GetCurrentPattern(UIA_TogglePatternId).and_then(|u| u.cast::<IUIAutomationTogglePattern>()) {
                    return p.Toggle().map_err(|e| werr("Toggle 失敗", e));
                }
                if let Ok(p) = el.GetCurrentPattern(UIA_SelectionItemPatternId).and_then(|u| u.cast::<IUIAutomationSelectionItemPattern>()) {
                    return p.Select().map_err(|e| werr("Select 失敗", e));
                }
                if let Ok(p) = el.GetCurrentPattern(UIA_ExpandCollapsePatternId).and_then(|u| u.cast::<IUIAutomationExpandCollapsePattern>()) {
                    return p.Expand().map_err(|e| werr("Expand 失敗", e));
                }
            }
            Err("這個元件不支援直接啟動".to_string())
        })
        .await
    }

    async fn click_at(&self, x: f64, y: f64, button: MouseButton, double: bool) -> Result<(), String> {
        blocking(move || {
            let (down, up) = match button {
                MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            };
            send(&[move_to(x, y)])?;
            std::thread::sleep(Duration::from_millis(40));
            for _ in 0..if double { 2 } else { 1 } {
                send(&[mouse_input(0, 0, 0, down)])?;
                std::thread::sleep(Duration::from_millis(30));
                send(&[mouse_input(0, 0, 0, up)])?;
                std::thread::sleep(Duration::from_millis(30));
            }
            Ok(())
        })
        .await
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        let events = winmap::text_events(text);
        blocking(move || {
            // KEYEVENTF_UNICODE：直接送字元，不經鍵盤配置與輸入法，中文與 emoji 都能輸入；
            // 換行與 Tab 改送真正的按鍵。
            for chunk in events.chunks(64) {
                let inputs: Vec<INPUT> = chunk
                    .iter()
                    .map(|e| match *e {
                        winmap::TextEvent::Unicode(unit, up) => {
                            let mut flags = KEYEVENTF_UNICODE;
                            if up {
                                flags |= KEYEVENTF_KEYUP;
                            }
                            key_input(0, unit, flags)
                        }
                        winmap::TextEvent::Vk(vk, up) => vk_event(vk, up),
                    })
                    .collect();
                send(&inputs)?;
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        })
        .await
    }

    async fn focus_element(&self, pid: i32, id: &str) -> Result<(), String> {
        let (pid, id) = (pid_of(pid), id.to_string());
        blocking(move || {
            let auto = uia()?;
            let el = resolve(pid, &id, &auto)?;
            // SAFETY: 把焦點移到該元件。
            unsafe { el.SetFocus() }.map_err(|e| werr("無法聚焦該元件", e))
        })
        .await
    }

    async fn key(&self, combo: &KeyCombo) -> Result<(), String> {
        let combo = combo.clone();
        blocking(move || {
            let vk = winmap::vk_for_key(&combo.key).ok_or_else(|| format!("不支援的按鍵「{}」", combo.key))?;
            let mods = winmap::modifier_keys(&combo);
            let mut inputs: Vec<INPUT> = Vec::new();
            for m in &mods {
                inputs.push(vk_event(*m, false));
            }
            inputs.push(vk_event(vk, false));
            inputs.push(vk_event(vk, true));
            for m in mods.iter().rev() {
                inputs.push(vk_event(*m, true));
            }
            send(&inputs)
        })
        .await
    }

    async fn scroll(&self, dx: i32, dy: i32, at: Option<(f64, f64)>) -> Result<(), String> {
        blocking(move || {
            if let Some((x, y)) = at {
                send(&[move_to(x, y)])?;
                std::thread::sleep(Duration::from_millis(40));
            }
            let (vertical, horizontal) = winmap::wheel_amounts(dx, dy);
            let mut inputs = Vec::new();
            if vertical != 0 {
                inputs.push(mouse_input(0, 0, vertical, MOUSEEVENTF_WHEEL));
            }
            if horizontal != 0 {
                inputs.push(mouse_input(0, 0, horizontal, MOUSEEVENTF_HWHEEL));
            }
            send(&inputs)
        })
        .await
    }

    async fn focused_text(&self, _pid: i32) -> Result<Option<FocusedText>, String> {
        // 等應用程式處理完剛送出的事件，再讀它的內容。
        tokio::time::sleep(Duration::from_millis(150)).await;
        blocking(|| {
            let auto = uia()?;
            // SAFETY: 讀取目前焦點元素。
            unsafe {
                let Ok(el) = auto.GetFocusedElement() else { return Ok(None) };
                let role = winmap::uia_role(el.CurrentControlType().map(|t| t.0).unwrap_or(0)).to_string();
                let secure = el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false);
                let text = if secure { String::new() } else { element_value(&el).unwrap_or_default() };
                Ok(Some(FocusedText { role, text, secure }))
            }
        })
        .await
    }

    async fn read_screen(&self, pid: Option<i32>) -> Result<Vec<ScreenText>, String> {
        let pid = pid.map(pid_of);
        blocking(move || {
            let (x, y, w, h, _) = capture_target(pid)?;
            let img = capture_bgra(x, y, w, h)?;
            let lines = ocr_bgra(&img.data, img.width, img.height)?;
            Ok(lines
                .into_iter()
                .map(|(text, lx, ly, lw, lh)| ScreenText { text, x: img.x as f64 + lx, y: img.y as f64 + ly, w: lw, h: lh })
                .collect())
        })
        .await
    }

    async fn find_icons(&self, pid: i32, region: Region) -> Result<Vec<IconRect>, String> {
        let pid = pid_of(pid);
        blocking(move || {
            let (x, y, w, h, scale) = capture_target(Some(pid))?;
            let img = capture_bgra(x, y, w, h)?;
            let lum = winmap::bgra_to_luminance(&img.data);
            Ok(detect_icons(&lum, img.width as usize, img.height as usize, scale, region)
                .into_iter()
                .map(|r| IconRect { x: img.x as f64 + r.x, y: img.y as f64 + r.y, w: r.w, h: r.h })
                .collect())
        })
        .await
    }

    async fn open_app(&self, name: &str) -> Result<(), String> {
        let name = name.to_string();
        blocking(move || shell_open(&name, None)).await
    }

    async fn open_url(&self, url: &str, app: Option<&str>) -> Result<(), String> {
        let (url, app) = (url.to_string(), app.map(str::to_string));
        blocking(move || match app {
            // 指定 App：它當執行檔，網址當參數（用引號包起來；呼叫端已拒絕網址中的引號與空白）。
            Some(a) => shell_open(&a, Some(&format!("\"{url}\""))),
            None => shell_open(&url, None),
        })
        .await
    }
}

/// `ShellExecuteW`：不經 `cmd`，所以沒有命令列注入；名稱可以是 `notepad`、`chrome`（App Paths／PATH 解析）或網址。
fn shell_open(target: &str, params: Option<&str>) -> Result<(), String> {
    let target_w = HSTRING::from(target);
    let params_w = params.map(HSTRING::from);
    // SAFETY: 兩個字串在呼叫期間都有效。
    let r = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &target_w,
            params_w.as_ref().map(|p| PCWSTR(p.as_ptr())).unwrap_or(PCWSTR::null()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 回傳值大於 32 才是成功。
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("無法開啟「{target}」（錯誤碼 {}）。名稱要和「開始」功能表或 PATH 裡的一致", r.0 as isize))
    }
}
