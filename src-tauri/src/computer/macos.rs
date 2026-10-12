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
        const JXA: &str = r#"
var se = Application("System Events");
var ps = se.processes.whose({backgroundOnly: false})();
JSON.stringify(ps.map(function (p) {
  return {name: p.name(), bundle_id: p.bundleIdentifier(), pid: p.unixId(), frontmost: p.frontmost()};
}));"#;
        let out = run_cmd("osascript", &["-l", "JavaScript", "-e", JXA]).await?;
        #[derive(serde::Deserialize)]
        struct Row { name: String, bundle_id: Option<String>, pid: i32, frontmost: bool }
        let rows: Vec<Row> = serde_json::from_str(&out).map_err(|e| format!("無法解析 App 清單：{e}"))?;
        Ok(rows.into_iter().map(|r| AppInfo { name: r.name, bundle_id: r.bundle_id, pid: r.pid, frontmost: r.frontmost }).collect())
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
        // 範圍：整個主螢幕，或該 App 的主視窗（用 AX 取得它在螢幕座標的位置與大小）。
        let (x, y, w, h, region) = match pid {
            None => {
                let b = CGDisplay::main().bounds();
                (0.0, 0.0, b.size.width, b.size.height, None)
            }
            Some(pid) => blocking(move || {
                let app = Owned::app(pid)?;
                let win = app
                    .attr("AXFocusedWindow")
                    .or_else(|| app.attr("AXMainWindow"))
                    .ok_or_else(|| "這個 App 沒有可截圖的視窗（可能被最小化或在其他桌面）".to_string())?;
                let p = win.point("AXPosition").ok_or("讀不到視窗位置")?;
                let s = win.size("AXSize").ok_or("讀不到視窗大小")?;
                Ok((p.x, p.y, s.width, s.height, Some(format!("{},{},{},{}", p.x as i64, p.y as i64, s.width as i64, s.height as i64))))
            })
            .await?,
        };
        let dir = std::env::temp_dir();
        let id = uuid::Uuid::new_v4();
        let raw = dir.join(format!("aiterm-shot-{id}.png"));
        let small = dir.join(format!("aiterm-shot-{id}-s.png"));
        let raw_s = raw.to_string_lossy().to_string();
        let small_s = small.to_string_lossy().to_string();

        let result = async {
            match &region {
                Some(r) => run_cmd("screencapture", &["-x", "-t", "png", "-R", r, &raw_s]).await?,
                None => run_cmd("screencapture", &["-x", "-t", "png", &raw_s]).await?,
            };
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
        let _ = h;
        result
    }

    async fn activate(&self, pid: i32) -> Result<(), String> {
        let script = format!(
            "tell application \"System Events\" to set frontmost of (first process whose unix id is {pid}) to true"
        );
        run_cmd("osascript", &["-e", &script]).await?;
        tokio::time::sleep(Duration::from_millis(150)).await; // 等視窗真的到前景
        Ok(())
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
}
