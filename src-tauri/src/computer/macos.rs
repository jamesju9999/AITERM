//! macOS 後端：AX（輔助使用）讀元件樹與按鈕、CGEvent 送滑鼠鍵盤、`screencapture` 截圖。
//!
//! 所有 AX／CGEvent 呼叫都放進 `spawn_blocking`：AX 呼叫可能因對方 App 無回應而卡住
//! （已設 2 秒訊息逾時），而 `CGEventSource` 不是 `Send`，必須在使用它的執行緒內建立。

use std::ffi::c_void;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::string::CFString;
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef};
use core_foundation_sys::string::CFStringRef;
use core_graphics::access::ScreenCaptureAccess;
use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::{CGPoint, CGSize};

use super::backend::*;
use super::icons::{detect_icons, Region};

type AXUIElementRef = *const c_void;
type AXError = i32;
const AX_OK: AXError = 0;
const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;
const MAX_NODES: usize = 2000;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementSetMessagingTimeout(el: AXUIElementRef, seconds: f32) -> AXError;
    fn AXUIElementCopyAttributeValue(el: AXUIElementRef, attr: CFStringRef, out: *mut CFTypeRef) -> AXError;
    fn AXUIElementSetAttributeValue(el: AXUIElementRef, attr: CFStringRef, v: CFTypeRef) -> AXError;
    fn AXUIElementPerformAction(el: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXUIElementCopyElementAtPosition(app: AXUIElementRef, x: f32, y: f32, out: *mut AXUIElementRef) -> AXError;
    fn AXValueGetValue(v: CFTypeRef, kind: u32, out: *mut c_void) -> u8;
}

/// 擁有一個 CF 物件（AXUIElement 或屬性值），離開時釋放。
#[derive(Debug)]
struct Owned(CFTypeRef);
unsafe impl Send for Owned {}
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

impl Owned {
    fn app(pid: i32) -> Result<Owned, String> {
        let r = unsafe { AXUIElementCreateApplication(pid) };
        if r.is_null() {
            return Err(format!("無法建立 pid {pid} 的無障礙元件"));
        }
        unsafe { AXUIElementSetMessagingTimeout(r, 2.0) };
        Ok(Owned(r))
    }

    fn attr(&self, name: &str) -> Option<Owned> {
        let key = CFString::new(name);
        let mut out: CFTypeRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(self.0, key.as_concrete_TypeRef(), &mut out) };
        if err == AX_OK && !out.is_null() { Some(Owned(out)) } else { None }
    }

    fn string(&self, name: &str) -> Option<String> {
        let v = self.attr(name)?;
        if unsafe { CFGetTypeID(v.0) } != CFString::type_id() {
            return None;
        }
        // wrap_under_get_rule 會 retain；Owned 之後的 drop 釋放原本那一份。
        let s = unsafe { CFString::wrap_under_get_rule(v.0 as CFStringRef) };
        Some(s.to_string())
    }

    fn point(&self, name: &str) -> Option<CGPoint> {
        let v = self.attr(name)?;
        let mut p = CGPoint { x: 0.0, y: 0.0 };
        (unsafe { AXValueGetValue(v.0, AX_VALUE_CGPOINT, &mut p as *mut _ as *mut c_void) } != 0).then_some(p)
    }

    fn size(&self, name: &str) -> Option<CGSize> {
        let v = self.attr(name)?;
        let mut s = CGSize { width: 0.0, height: 0.0 };
        (unsafe { AXValueGetValue(v.0, AX_VALUE_CGSIZE, &mut s as *mut _ as *mut c_void) } != 0).then_some(s)
    }

    fn children(&self) -> Vec<Owned> {
        let Some(arr) = self.attr("AXChildren") else { return vec![] };
        let arr_ref = arr.0 as CFArrayRef;
        let n = unsafe { CFArrayGetCount(arr_ref) };
        (0..n)
            .filter_map(|i| {
                let p = unsafe { CFArrayGetValueAtIndex(arr_ref, i) };
                if p.is_null() { return None; }
                unsafe { CFRetain(p) };
                Some(Owned(p))
            })
            .collect()
    }

    fn retained(&self) -> Owned {
        unsafe { CFRetain(self.0) };
        Owned(self.0)
    }
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n { format!("{t}…") } else { t }
}

fn describe(el: &Owned, id: &str) -> ElementNode {
    let role = el.string("AXRole").unwrap_or_default();
    let title = ["AXTitle", "AXDescription", "AXValue", "AXPlaceholderValue"]
        .iter()
        .find_map(|k| el.string(k).filter(|s| !s.trim().is_empty()))
        .map(|s| truncate(s.trim(), 80))
        .unwrap_or_default();
    let p = el.point("AXPosition").unwrap_or(CGPoint { x: 0.0, y: 0.0 });
    let s = el.size("AXSize").unwrap_or(CGSize { width: 0.0, height: 0.0 });
    ElementNode { id: id.to_string(), role, title, x: p.x, y: p.y, w: s.width, h: s.height, children: vec![] }
}

fn build_tree(el: &Owned, id: &str, depth: usize, max_depth: usize, count: &mut usize) -> ElementNode {
    *count += 1;
    let mut node = describe(el, id);
    // 選單彈出層（File／Edit…的整串選項）常有上百項、而且多半看不見，
    // 展開只會把行數預算吃光；選單列上的標題（AXMenuBarItem）仍保留。
    if depth < max_depth && node.role != "AXMenu" {
        for (i, child) in el.children().iter().enumerate() {
            if *count >= MAX_NODES { break; }
            node.children.push(build_tree(child, &format!("{id}.{i}"), depth + 1, max_depth, count));
        }
    }
    node
}

/// `0.3.1` → 從 App 根節點依序取第 3 個、再第 1 個子節點。
fn resolve(pid: i32, id: &str) -> Result<Owned, String> {
    let mut parts = id.split('.');
    if parts.next() != Some("0") {
        return Err(format!("無效的元件 id「{id}」"));
    }
    let mut cur = Owned::app(pid)?;
    for p in parts {
        let idx: usize = p.parse().map_err(|_| format!("無效的元件 id「{id}」"))?;
        let mut kids = cur.children();
        if idx >= kids.len() {
            return Err("畫面已變，這個元件 id 已失效。請重新呼叫 computer_snapshot。".to_string());
        }
        cur = kids.swap_remove(idx);
    }
    Ok(cur)
}

/// 讓目前執行緒內的 CGEvent 呼叫共用一個 source。
fn source() -> Result<CGEventSource, String> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|_| "無法建立事件來源".to_string())
}

fn flags_for(c: &KeyCombo) -> CGEventFlags {
    let mut f = CGEventFlags::empty();
    if c.cmd { f |= CGEventFlags::CGEventFlagCommand; }
    if c.ctrl { f |= CGEventFlags::CGEventFlagControl; }
    if c.alt { f |= CGEventFlags::CGEventFlagAlternate; }
    if c.shift { f |= CGEventFlags::CGEventFlagShift; }
    f
}

/// ANSI 鍵盤的虛擬鍵碼（Carbon `kVK_*`）。
fn keycode(key: &str) -> Option<u16> {
    Some(match key {
        "a" => 0, "s" => 1, "d" => 2, "f" => 3, "h" => 4, "g" => 5, "z" => 6, "x" => 7, "c" => 8, "v" => 9,
        "b" => 11, "q" => 12, "w" => 13, "e" => 14, "r" => 15, "y" => 16, "t" => 17,
        "1" => 18, "2" => 19, "3" => 20, "4" => 21, "6" => 22, "5" => 23, "=" => 24, "9" => 25, "7" => 26,
        "-" => 27, "8" => 28, "0" => 29, "]" => 30, "o" => 31, "u" => 32, "[" => 33, "i" => 34, "p" => 35,
        "l" => 37, "j" => 38, "'" => 39, "k" => 40, ";" => 41, "\\" => 42, "," => 43, "/" => 44, "n" => 45,
        "m" => 46, "." => 47, "`" => 50,
        "return" => 36, "tab" => 48, "space" => 49, "delete" => 51, "escape" => 53,
        "forwarddelete" => 117, "home" => 115, "end" => 119, "pageup" => 116, "pagedown" => 121,
        "left" => 123, "right" => 124, "down" => 125, "up" => 126,
        "f1" => 122, "f2" => 120, "f3" => 99, "f4" => 118, "f5" => 96, "f6" => 97, "f7" => 98, "f8" => 100,
        "f9" => 101, "f10" => 109, "f11" => 103, "f12" => 111,
        _ => return None,
    })
}

async fn run_cmd(program: &str, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("無法執行 {program}：{e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("{program} 失敗：{}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" {
        return None;
    }
    Some((u32::from_be_bytes(bytes[16..20].try_into().ok()?), u32::from_be_bytes(bytes[20..24].try_into().ok()?)))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("內部錯誤：{e}"))?
}

use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};

/// 目前執行中、有視窗介面的一般 App（與原本 JXA 的 `backgroundOnly: false` 同義）。
fn running_apps() -> Vec<AppInfo> {
    let ws = NSWorkspace::sharedWorkspace();
    ws.runningApplications()
        .iter()
        .filter(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular)
        .filter_map(|a| {
            let localized = a.localizedName().map(|n| n.to_string());
            // 行程名稱 = 可執行檔名，和舊版 System Events 回的 `name` 一致（TextEdit、Notes、MSTeams…）。
            let process_name = a
                .executableURL()
                .and_then(|u| u.lastPathComponent())
                .map(|n| n.to_string())
                .or_else(|| localized.clone())?;
            let localized = localized.filter(|l| *l != process_name);
            Some(AppInfo {
                name: process_name,
                localized,
                bundle_id: a.bundleIdentifier().map(|b| b.to_string()),
                pid: a.processIdentifier(),
                frontmost: a.isActive(),
            })
        })
        .collect()
}

/// 從 `lsappinfo info` 的輸出取出 `pid = 123`。
fn parse_lsappinfo_pid(out: &str) -> Option<i32> {
    let rest = out.split("pid = ").nth(1)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok().filter(|p| *p > 0)
}

/// 目前最前面的 App 的 pid。用 `lsappinfo`（約 10 毫秒、即時）：NSWorkspace 的 `frontmostApplication`
/// 要靠主執行緒 run loop 更新，在背景執行緒讀會是舊值；AX 的 AXFocusedApplication 則不一定有值。
/// 系統對話框（例如密碼提示）在最前面時，這裡會如實回傳它的 pid，而不是前一個一般 App。
async fn frontmost_pid() -> Option<i32> {
    let asn = run_cmd("lsappinfo", &["front"]).await.ok()?;
    let info = run_cmd("lsappinfo", &["info", "-only", "pid", asn.trim()]).await.ok()?;
    parse_lsappinfo_pid(&info)
}

/// 輪詢到 `pid` 成為最前面的 App，或逾時。
async fn wait_frontmost(pid: i32, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if frontmost_pid().await == Some(pid) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// 用系統的視窗清單（CoreGraphics）取得某個 App 在螢幕上最上層的可見一般視窗。
/// 不依賴無障礙 API，所以 Chromium／Qt 這類 AX 視窗資訊不可信的 App（例如 LINE）也能用；
/// 視窗位置與大小本來就不需要螢幕錄製權限。
fn window_bounds_for_pid(pid: i32) -> Option<(f64, f64, f64, f64)> {
    use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex};
    use core_foundation_sys::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
    use core_foundation_sys::number::{kCFNumberDoubleType, CFNumberGetValue, CFNumberRef};
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID,
    };

    fn num(d: CFDictionaryRef, key: CFStringRef) -> Option<f64> {
        let v = unsafe { CFDictionaryGetValue(d, key as *const c_void) };
        if v.is_null() {
            return None;
        }
        let mut out = 0f64;
        let ok = unsafe { CFNumberGetValue(v as CFNumberRef, kCFNumberDoubleType, &mut out as *mut f64 as *mut c_void) };
        ok.then_some(out)
    }

    let list = copy_window_info(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID)?;
    let arr = list.as_concrete_TypeRef();
    let (kx, ky, kw, kh) = (CFString::new("X"), CFString::new("Y"), CFString::new("Width"), CFString::new("Height"));
    let mut best: Option<(f64, f64, f64, f64)> = None;
    for i in 0..unsafe { CFArrayGetCount(arr) } {
        let d = unsafe { CFArrayGetValueAtIndex(arr, i) } as CFDictionaryRef;
        if d.is_null() || num(d, unsafe { kCGWindowOwnerPID }) != Some(pid as f64) {
            continue;
        }
        if num(d, unsafe { kCGWindowLayer }) != Some(0.0) {
            continue; // 只要一般視窗，不要選單、浮動面板
        }
        let b = unsafe { CFDictionaryGetValue(d, kCGWindowBounds as *const c_void) } as CFDictionaryRef;
        if b.is_null() {
            continue;
        }
        let (Some(x), Some(y), Some(w), Some(h)) = (
            num(b, kx.as_concrete_TypeRef()),
            num(b, ky.as_concrete_TypeRef()),
            num(b, kw.as_concrete_TypeRef()),
            num(b, kh.as_concrete_TypeRef()),
        ) else {
            continue;
        };
        if w < 50.0 || h < 50.0 {
            continue;
        }
        // 系統清單是由前到後排序：第一個就是這個 App 最上層的視窗（剛 activate 過，也就是使用者正在看的那個）。
        // 不挑最大的——LINE 這類 App 同時有好友清單與聊天兩個視窗，最大的不一定是要操作的。
        best = Some((x, y, w, h));
        break;
    }
    best
}

fn full_screen_region() -> (f64, f64, f64, f64, Option<String>) {
    let b = CGDisplay::main().bounds();
    (0.0, 0.0, b.size.width, b.size.height, None)
}

fn region_arg(x: f64, y: f64, w: f64, h: f64) -> String {
    format!("{},{},{},{}", x as i64, y as i64, w as i64, h as i64)
}

/// 要截的範圍：整個主螢幕，或該 App 的前景視窗。
/// 視窗位置先問系統視窗清單，再退回 AX；都找不到就回錯誤（不偷偷改截整個螢幕）。
/// 回傳 (x, y, w, h, screencapture 的 -R 參數)。
async fn capture_region(pid: Option<i32>) -> Result<(f64, f64, f64, f64, Option<String>), String> {
    let Some(pid) = pid else { return Ok(full_screen_region()) };
    blocking(move || {
        if let Some((x, y, w, h)) = window_bounds_for_pid(pid) {
            return Ok((x, y, w, h, Some(region_arg(x, y, w, h))));
        }
        if let Ok(app) = Owned::app(pid) {
            if let Some(win) = app.attr("AXFocusedWindow").or_else(|| app.attr("AXMainWindow")) {
                if let (Some(p), Some(s)) = (win.point("AXPosition"), win.size("AXSize")) {
                    if s.width >= 50.0 && s.height >= 50.0 {
                        return Ok((p.x, p.y, s.width, s.height, Some(region_arg(p.x, p.y, s.width, s.height))));
                    }
                }
            }
        }
        // 找不到這個 App 的可見視窗：不要偷偷改截整個螢幕——那會把別的 App 的文字當成它的。
        Err("這個 App 目前沒有可見的視窗（可能被最小化、在其他桌面，或還沒打開視窗）".to_string())
    })
    .await
}

/// 讓背景執行、但沒有視窗的 App 重新打開視窗（等同點 Dock 圖示，`open -b` 會觸發 reopen 事件），
/// 並等視窗真的出現。只在需要看畫面（OCR）時使用，不在一般的 activate 裡做——
/// 有些 App（例如 Finder）的 reopen 會多開一個視窗。
async fn reopen_window(pid: i32) -> Result<(), String> {
    let bundle = running_apps()
        .into_iter()
        .find(|a| a.pid == pid)
        .and_then(|a| a.bundle_id)
        .ok_or("找不到這個 App 的 bundle id")?;
    run_cmd("open", &["-b", &bundle]).await?;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(2500) {
        if blocking(move || Ok(window_bounds_for_pid(pid).is_some())).await.unwrap_or(false) {
            tokio::time::sleep(Duration::from_millis(250)).await; // 讓視窗內容畫完
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("重新開啟後仍然沒有視窗".to_string())
}

/// App 在背景執行、卻沒有任何可見視窗（把視窗關掉了——LINE 這類常見）時，畫面上什麼都操作不了，
/// 它的無障礙樹也只剩一個 1x1 的幽靈視窗。這裡做一次「點 Dock 圖示」等效的重新開啟。
/// 盡力而為：失敗不影響後續動作。Finder 例外——它沒有視窗是常態，reopen 會多開一個視窗。
async fn ensure_window(pid: i32) {
    let has_window = blocking(move || Ok(window_bounds_for_pid(pid).is_some())).await.unwrap_or(true);
    if has_window {
        return;
    }
    let is_finder = running_apps().iter().any(|a| a.pid == pid && a.bundle_id.as_deref() == Some("com.apple.finder"));
    if !is_finder {
        let _ = reopen_window(pid).await;
    }
}

/// 取得視窗範圍；App 在背景執行但視窗被關掉（LINE 這類常見）時，做一次「點 Dock 圖示」等效的重新開啟再試。
async fn region_with_reopen(pid: Option<i32>) -> Result<(f64, f64, f64, f64, Option<String>), String> {
    match capture_region(pid).await {
        Ok(r) => Ok(r),
        Err(e) => {
            let Some(pid) = pid else { return Err(e) };
            reopen_window(pid).await.map_err(|_| e.clone())?;
            capture_region(Some(pid)).await
        }
    }
}

/// 讀 PNG 並轉成 0~255 的亮度（列優先），回傳 (亮度, 寬, 高)。
fn load_luminance(path: &str) -> Result<(Vec<u8>, usize, usize), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("讀取截圖失敗：{e}"))?;
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("截圖不是有效的 PNG：{e}"))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("解碼截圖失敗：{e}"))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = info.color_type.samples();
    let lum = png_to_luminance(&buf[..info.buffer_size()], w, h, channels);
    Ok((lum, w, h))
}

fn png_to_luminance(data: &[u8], w: usize, h: usize, channels: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h);
    for px in data.chunks_exact(channels).take(w * h) {
        let v = match channels {
            1 | 2 => px[0] as f64,
            _ => 0.299 * px[0] as f64 + 0.587 * px[1] as f64 + 0.114 * px[2] as f64,
        };
        out.push(v.round() as u8);
    }
    out
}

async fn capture_to(path: &str, region: Option<&str>) -> Result<(), String> {
    match region {
        Some(r) => run_cmd("screencapture", &["-x", "-t", "png", "-R", r, path]).await?,
        None => run_cmd("screencapture", &["-x", "-t", "png", path]).await?,
    };
    Ok(())
}

/// Vision 辨識出的一段文字；座標是 0~1 的相對座標，原點在左下角。
struct OcrLine {
    text: String,
    nx: f64,
    ny: f64,
    nw: f64,
    nh: f64,
}

impl OcrLine {
    /// 轉成螢幕座標（原點在左上角）。`(x, y, w, h)` 是被辨識的畫面範圍在螢幕上的位置與大小。
    fn to_screen(&self, x: f64, y: f64, w: f64, h: f64) -> ScreenText {
        ScreenText {
            text: self.text.clone(),
            x: x + self.nx * w,
            y: y + (1.0 - self.ny - self.nh) * h,
            w: self.nw * w,
            h: self.nh * h,
        }
    }
}

/// 用 macOS Vision 辨識圖檔中的文字。支援繁中、簡中與英文。
fn ocr_file(path: &str) -> Result<Vec<OcrLine>, String> {
    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
    use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};

    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let handler = unsafe {
        VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), &url, &NSDictionary::new())
    };
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);
    let langs = NSArray::from_retained_slice(&[
        NSString::from_str("zh-Hant"),
        NSString::from_str("zh-Hans"),
        NSString::from_str("en-US"),
    ]);
    request.setRecognitionLanguages(&langs);
    let as_request: Retained<VNRequest> = Retained::into_super(Retained::into_super(request.clone()));
    let requests = NSArray::from_retained_slice(&[as_request]);
    handler.performRequests_error(&requests).map_err(|e| format!("文字辨識失敗：{}", e.localizedDescription()))?;

    let mut out = Vec::new();
    if let Some(results) = request.results() {
        for obs in results.iter() {
            let cands = unsafe { obs.topCandidates(1) };
            let Some(best) = cands.firstObject() else { continue };
            let text = best.string().to_string();
            if text.trim().is_empty() {
                continue;
            }
            let r = unsafe { obs.boundingBox() };
            out.push(OcrLine { text, nx: r.origin.x, ny: r.origin.y, nw: r.size.width, nh: r.size.height });
        }
    }
    Ok(out)
}

pub struct MacBackend;

#[async_trait]
impl Backend for MacBackend {
    fn permissions(&self) -> Permissions {
        Permissions {
            accessibility: unsafe { AXIsProcessTrusted() } != 0,
            screen_recording: ScreenCaptureAccess.preflight(),
        }
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, String> {
        // 原生呼叫（NSWorkspace）：幾毫秒；原本用 osascript／JXA 要約 1 秒，而且每個工具呼叫都要列一次。
        blocking(|| Ok(running_apps())).await
    }

    async fn snapshot(&self, pid: i32, max_depth: usize) -> Result<ElementNode, String> {
        blocking(move || {
            let root = Owned::app(pid)?;
            let mut count = 0;
            Ok(build_tree(&root, "0", 0, max_depth, &mut count))
        })
        .await
    }

    async fn element_info(&self, pid: i32, id: &str) -> Result<ElementNode, String> {
        let id = id.to_string();
        blocking(move || Ok(describe(&resolve(pid, &id)?, &id))).await
    }

    async fn element_at(&self, x: f64, y: f64) -> Result<Option<ElementNode>, String> {
        blocking(move || {
            let sys = Owned(unsafe { AXUIElementCreateSystemWide() });
            let mut out: AXUIElementRef = std::ptr::null();
            let err = unsafe { AXUIElementCopyElementAtPosition(sys.0, x as f32, y as f32, &mut out) };
            if err != AX_OK || out.is_null() {
                return Ok(None);
            }
            Ok(Some(describe(&Owned(out), "?")))
        })
        .await
    }

    async fn screenshot(&self, pid: Option<i32>) -> Result<Shot, String> {
        let (x, y, w, _h, region) = capture_region(pid).await?;
        let dir = std::env::temp_dir();
        let id = uuid::Uuid::new_v4();
        let raw = dir.join(format!("aiterm-shot-{id}.png"));
        let small = dir.join(format!("aiterm-shot-{id}-s.png"));
        let raw_s = raw.to_string_lossy().to_string();
        let small_s = small.to_string_lossy().to_string();

        let result = async {
            capture_to(&raw_s, region.as_deref()).await?;
            // Retina 的截圖是實體像素；縮回邏輯像素，座標與點擊才一致、檔案也小。
            let logical_w = (w as i64).max(1).to_string();
            run_cmd("sips", &["--resampleWidth", &logical_w, &raw_s, "--out", &small_s]).await?;
            let png = tokio::fs::read(&small).await.map_err(|e| format!("讀取截圖失敗：{e}"))?;
            let (pw, ph) = png_size(&png).ok_or("截圖不是有效的 PNG（螢幕錄製權限可能尚未生效，需重開 App）")?;
            Ok::<_, String>(Shot { png, width: pw, height: ph, origin_x: x, origin_y: y })
        }
        .await;
        let _ = tokio::fs::remove_file(&raw).await;
        let _ = tokio::fs::remove_file(&small).await;
        result
    }

    async fn read_screen(&self, pid: Option<i32>) -> Result<Vec<ScreenText>, String> {
        let (x, y, w, h, region) = region_with_reopen(pid).await?;
        let raw = std::env::temp_dir().join(format!("aiterm-ocr-{}.png", uuid::Uuid::new_v4()));
        let raw_s = raw.to_string_lossy().to_string();
        let result = async {
            // 用原始（Retina）解析度辨識，字比較清楚；Vision 回傳的是 0~1 的相對座標，與解析度無關。
            capture_to(&raw_s, region.as_deref()).await?;
            let path = raw_s.clone();
            let lines = blocking(move || ocr_file(&path)).await?;
            Ok::<_, String>(lines.into_iter().map(|l| l.to_screen(x, y, w, h)).collect::<Vec<_>>())
        }
        .await;
        let _ = tokio::fs::remove_file(&raw).await;
        result
    }

    async fn find_icons(&self, pid: i32, region: Region) -> Result<Vec<IconRect>, String> {
        let (x, y, w, _h, rect) = region_with_reopen(Some(pid)).await?;
        let raw = std::env::temp_dir().join(format!("aiterm-icons-{}.png", uuid::Uuid::new_v4()));
        let raw_s = raw.to_string_lossy().to_string();
        let result = async {
            capture_to(&raw_s, rect.as_deref()).await?;
            let path = raw_s.clone();
            blocking(move || {
                let (lum, iw, ih) = load_luminance(&path)?;
                let scale = iw as f64 / w.max(1.0);
                Ok(detect_icons(&lum, iw, ih, scale, region)
                    .into_iter()
                    .map(|r| IconRect { x: x + r.x / scale, y: y + r.y / scale, w: r.w / scale, h: r.h / scale })
                    .collect::<Vec<_>>())
            })
            .await
        }
        .await;
        let _ = tokio::fs::remove_file(&raw).await;
        result
    }

    async fn activate(&self, pid: i32) -> Result<(), String> {
        // 連續幾個動作都在同一個 App 上很常見：已經在最前面就什麼都不用做（原本每次都要 ~0.3 秒）。
        if frontmost_pid().await == Some(pid) {
            ensure_window(pid).await;
            return Ok(());
        }
        let script = format!(
            "tell application \"System Events\" to set frontmost of (first process whose unix id is {pid}) to true"
        );
        run_cmd("osascript", &["-e", &script]).await?;
        // 輪詢確認而不是固定睡一下：快的時候更快；被系統對話框擋住時能明確報錯，
        // 而不是把按鍵與文字送進別的視窗。
        if wait_frontmost(pid, Duration::from_millis(1000)).await {
            ensure_window(pid).await;
            Ok(())
        } else {
            Err("無法把這個 App 切到最前面（可能有系統對話框擋住，或這個 App 沒有可顯示的視窗）".to_string())
        }
    }

    async fn press_element(&self, pid: i32, id: &str) -> Result<(), String> {
        let id = id.to_string();
        blocking(move || {
            let el = resolve(pid, &id)?;
            let action = CFString::new("AXPress");
            let err = unsafe { AXUIElementPerformAction(el.0, action.as_concrete_TypeRef()) };
            if err == AX_OK { Ok(()) } else { Err(format!("AXPress 失敗（錯誤碼 {err}）")) }
        })
        .await
    }

    async fn click_at(&self, x: f64, y: f64, button: MouseButton, double: bool) -> Result<(), String> {
        blocking(move || {
            let pt = CGPoint { x, y };
            let (down, up, btn) = match button {
                MouseButton::Left => (CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGMouseButton::Left),
                MouseButton::Right => (CGEventType::RightMouseDown, CGEventType::RightMouseUp, CGMouseButton::Right),
            };
            let mv = CGEvent::new_mouse_event(source()?, CGEventType::MouseMoved, pt, CGMouseButton::Left)
                .map_err(|_| "無法建立滑鼠事件")?;
            mv.post(CGEventTapLocation::HID);
            std::thread::sleep(Duration::from_millis(40));
            for click in 1..=if double { 2 } else { 1 } {
                for t in [down, up] {
                    let e = CGEvent::new_mouse_event(source()?, t, pt, btn).map_err(|_| "無法建立滑鼠事件")?;
                    e.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click);
                    e.post(CGEventTapLocation::HID);
                    std::thread::sleep(Duration::from_millis(30));
                }
            }
            Ok(())
        })
        .await
    }

    async fn type_text(&self, text: &str) -> Result<(), String> {
        let text = text.to_string();
        blocking(move || {
            // CGEvent 的字串一次最多約 20 個 UTF-16 單位，分段送。
            let units: Vec<u16> = text.encode_utf16().collect();
            for chunk in units.chunks(16) {
                for down in [true, false] {
                    let e = CGEvent::new_keyboard_event(source()?, 0, down).map_err(|_| "無法建立鍵盤事件")?;
                    // 明確清掉修飾鍵：緊接在 cmd+a 之後輸入時，不能讓文字被當成快速鍵。
                    e.set_flags(CGEventFlags::empty());
                    e.set_string_from_utf16_unchecked(chunk);
                    e.post(CGEventTapLocation::HID);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Ok(())
        })
        .await
    }

    async fn focus_element(&self, pid: i32, id: &str) -> Result<(), String> {
        let id = id.to_string();
        blocking(move || {
            let el = resolve(pid, &id)?;
            let attr = CFString::new("AXFocused");
            let yes = CFBoolean::true_value();
            let err = unsafe { AXUIElementSetAttributeValue(el.0, attr.as_concrete_TypeRef(), yes.as_CFTypeRef()) };
            if err == AX_OK { Ok(()) } else { Err(format!("無法聚焦該元件（錯誤碼 {err}）")) }
        })
        .await
    }

    async fn key(&self, combo: &KeyCombo) -> Result<(), String> {
        let combo = combo.clone();
        blocking(move || {
            let code = keycode(&combo.key).ok_or_else(|| format!("不支援的按鍵「{}」", combo.key))?;
            let flags = flags_for(&combo);
            for down in [true, false] {
                let e = CGEvent::new_keyboard_event(source()?, code, down).map_err(|_| "無法建立鍵盤事件")?;
                e.set_flags(flags);
                e.post(CGEventTapLocation::HID);
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        })
        .await
    }

    async fn focused_text(&self, pid: i32) -> Result<Option<FocusedText>, String> {
        // 等應用程式處理完剛送出的事件，再讀它的內容。
        tokio::time::sleep(Duration::from_millis(150)).await;
        blocking(move || {
            let app = Owned::app(pid)?;
            let Some(el) = app.attr("AXFocusedUIElement") else { return Ok(None) };
            let role = el.string("AXRole").unwrap_or_default();
            let secure = el.string("AXSubrole").as_deref() == Some("AXSecureTextField") || role == "AXSecureTextField";
            let text = if secure { String::new() } else { el.string("AXValue").unwrap_or_default() };
            Ok(Some(FocusedText { role, text, secure }))
        })
        .await
    }

    async fn scroll(&self, dx: i32, dy: i32, at: Option<(f64, f64)>) -> Result<(), String> {
        blocking(move || {
            if let Some((x, y)) = at {
                let mv = CGEvent::new_mouse_event(source()?, CGEventType::MouseMoved, CGPoint { x, y }, CGMouseButton::Left)
                    .map_err(|_| "無法建立滑鼠事件")?;
                mv.post(CGEventTapLocation::HID);
                std::thread::sleep(Duration::from_millis(40));
            }
            let e = CGEvent::new_scroll_event(source()?, ScrollEventUnit::LINE, 2, dy, dx, 0)
                .map_err(|_| "無法建立捲動事件")?;
            e.post(CGEventTapLocation::HID);
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycode_table_covers_every_name_the_parser_accepts() {
        for k in ["a", "z", "0", "9", "return", "tab", "space", "escape", "delete", "forwarddelete",
                  "left", "right", "up", "down", "home", "end", "pageup", "pagedown", "f1", "f12",
                  "-", "=", "[", "]", "\\", ";", "'", ",", ".", "/", "`"] {
            assert!(keycode(k).is_some(), "{k}");
            assert!(super::super::policy::parse_key(k).is_ok(), "{k}");
        }
        assert_eq!(keycode("s"), Some(1));
        assert_eq!(keycode("return"), Some(36));
    }

    #[test]
    fn png_size_reads_the_ihdr_and_rejects_non_png() {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        b.extend_from_slice(&800u32.to_be_bytes());
        b.extend_from_slice(&600u32.to_be_bytes());
        assert_eq!(png_size(&b), Some((800, 600)));
        assert_eq!(png_size(b"not a png at all, definitely not"), None);
        assert_eq!(png_size(&[]), None);
    }

    #[test]
    fn element_ids_must_start_at_the_app_root() {
        assert!(resolve(1, "1.2").unwrap_err().contains("無效"));
        assert!(resolve(1, "").unwrap_err().contains("無效"));
    }

    #[test]
    fn flags_combine_modifiers() {
        let c = super::super::policy::parse_key("cmd+shift+s").unwrap();
        let f = flags_for(&c);
        assert!(f.contains(CGEventFlags::CGEventFlagCommand) && f.contains(CGEventFlags::CGEventFlagShift));
        assert!(!f.contains(CGEventFlags::CGEventFlagControl));
    }

    #[test]
    fn truncate_marks_cut_text() {
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("abc", 3), "abc");
    }

    // ── 真機冒煙測試：需要 macOS 輔助使用／螢幕錄製權限，所以預設不跑。
    //    cargo test -p app --lib computer::macos -- --ignored --nocapture

    #[tokio::test]
    #[ignore]
    async fn live_list_apps_and_snapshot_finder() {
        let b = MacBackend;
        println!("permissions: {:?}", b.permissions());
        let apps = b.list_apps().await.unwrap();
        println!("{apps:#?}");
        let finder = apps.iter().find(|a| a.bundle_id.as_deref() == Some("com.apple.finder")).expect("Finder");
        let tree = b.snapshot(finder.pid, 3).await.unwrap();
        println!("root {:?} children={}", tree.role, tree.children.len());
        assert_eq!(tree.role, "AXApplication");
        assert!(!tree.children.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn live_full_screen_screenshot() {
        let shot = MacBackend.screenshot(None).await.unwrap();
        println!("{}x{} origin=({}, {}) bytes={}", shot.width, shot.height, shot.origin_x, shot.origin_y, shot.png.len());
        std::fs::write(std::env::temp_dir().join("aiterm-live-shot.png"), &shot.png).unwrap();
        assert!(shot.width > 300 && shot.height > 300);
    }

    #[test]
    fn lsappinfo_pid_is_parsed_from_its_noisy_output() {
        let out = "[ NULL ]  [ NULL ] \n    bundleID=[ NULL ] \n    pid = 32105 !cgsConnection !signalled type=[ NULL ]";
        assert_eq!(parse_lsappinfo_pid(out), Some(32105));
        assert_eq!(parse_lsappinfo_pid("nothing here"), None);
        assert_eq!(parse_lsappinfo_pid("pid = 0 x"), None);
        assert_eq!(parse_lsappinfo_pid("pid = abc"), None);
    }

    #[tokio::test]
    #[ignore]
    async fn live_native_app_list_matches_the_old_osascript_one_and_is_fast() {
        use std::time::Instant;
        let b = MacBackend;
        let t = Instant::now();
        let apps = b.list_apps().await.unwrap();
        let native = t.elapsed();
        println!("TIMING list_apps(native): {native:?} ({} apps)", apps.len());
        assert!(native < Duration::from_millis(300), "{native:?}");

        const JXA: &str = r#"
var se = Application("System Events");
var ps = se.processes.whose({backgroundOnly: false})();
JSON.stringify(ps.map(function (p) { return {name: p.name(), pid: p.unixId()}; }));"#;
        let out = run_cmd("osascript", &["-l", "JavaScript", "-e", JXA]).await.unwrap();
        #[derive(serde::Deserialize)]
        struct Row { name: String, pid: i32 }
        let mut old: Vec<(i32, String)> = serde_json::from_str::<Vec<Row>>(&out).unwrap().into_iter().map(|r| (r.pid, r.name)).collect();
        let mut new: Vec<(i32, String)> = apps.iter().map(|a| (a.pid, a.name.clone())).collect();
        old.sort();
        new.sort();
        assert_eq!(new, old, "原生清單的 App 與名稱應與舊的 osascript 清單完全一致");
    }

    #[tokio::test]
    #[ignore]
    async fn live_activate_is_instant_when_already_frontmost_and_switches_otherwise() {
        use std::time::Instant;
        let b = MacBackend;
        let apps = b.list_apps().await.unwrap();
        let finder = apps.iter().find(|a| a.bundle_id.as_deref() == Some("com.apple.finder")).unwrap();
        let notes = apps.iter().find(|a| a.name == "LINE").or_else(|| apps.iter().find(|a| a.pid != finder.pid)).unwrap();

        let t = Instant::now();
        b.activate(finder.pid).await.unwrap();
        println!("TIMING activate(switch to Finder): {:?}", t.elapsed());
        assert_eq!(frontmost_pid().await, Some(finder.pid));

        let t = Instant::now();
        b.activate(finder.pid).await.unwrap();
        let again = t.elapsed();
        println!("TIMING activate(already frontmost): {again:?}");
        assert!(again < Duration::from_millis(100), "{again:?}");

        b.activate(notes.pid).await.unwrap();
        assert_eq!(frontmost_pid().await, Some(notes.pid));
    }

    #[test]
    fn ocr_coordinates_flip_from_bottom_left_normalized_to_top_left_screen_points() {
        // 畫面範圍：螢幕上 (100,50) 起、1000x500。文字佔左下角 10% 寬、20% 高。
        let l = OcrLine { text: "x".into(), nx: 0.0, ny: 0.0, nw: 0.1, nh: 0.2 };
        let t = l.to_screen(100.0, 50.0, 1000.0, 500.0);
        assert!((t.x - 100.0).abs() < 1e-6 && (t.y - 450.0).abs() < 1e-6 && (t.w - 100.0).abs() < 1e-6 && (t.h - 100.0).abs() < 1e-6, "{t:?}");
        // 左上角的文字（ny + nh = 1）要落在範圍頂端。
        let top = OcrLine { text: "y".into(), nx: 0.5, ny: 0.9, nw: 0.2, nh: 0.1 };
        let t = top.to_screen(0.0, 0.0, 1000.0, 500.0);
        assert!((t.x - 500.0).abs() < 1e-6 && t.y.abs() < 1e-6, "{} {}", t.x, t.y);
    }

    #[tokio::test]
    #[ignore]
    async fn live_ocr_reads_text_from_a_real_window_with_screen_coordinates() {
        use std::time::Instant;
        // 把 TextEdit 開成有已知文字的文件，確認 OCR 讀得到，且座標落在該視窗內。
        let apps = MacBackend.list_apps().await.unwrap();
        let te = apps.iter().find(|a| a.name == "TextEdit").expect("先開 TextEdit");
        MacBackend.activate(te.pid).await.unwrap();
        let mut items = vec![];
        for round in 1..=3 {
            let t = Instant::now();
            items = MacBackend.read_screen(Some(te.pid)).await.unwrap();
            println!("TIMING read_screen(TextEdit) round {round}: {:?} ({} items)", t.elapsed(), items.len());
        }
        for i in items.iter().take(10) {
            println!("OCR {:?} ({:.0},{:.0},{:.0},{:.0})", i.text, i.x, i.y, i.w, i.h);
        }
        assert!(items.iter().any(|i| i.text.contains("你好世界")), "沒讀到預期的文字：{:?}", items.iter().map(|i| &i.text).collect::<Vec<_>>());
    }

    #[test]
    fn png_pixels_convert_to_luminance_for_every_channel_layout() {
        // RGBA：白、黑、純綠
        let rgba = [255, 255, 255, 255, 0, 0, 0, 255, 0, 255, 0, 255];
        assert_eq!(png_to_luminance(&rgba, 3, 1, 4), vec![255, 0, 150]);
        // 灰階與灰階+alpha
        assert_eq!(png_to_luminance(&[10, 200], 2, 1, 1), vec![10, 200]);
        assert_eq!(png_to_luminance(&[10, 255, 200, 255], 2, 1, 2), vec![10, 200]);
        // 資料比宣告的短：只轉有的，不越界
        assert_eq!(png_to_luminance(&[1, 2, 3], 5, 5, 3).len(), 1);
    }
}
