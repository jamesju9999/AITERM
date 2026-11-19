//! 各作業系統的「使用者閒置多久、螢幕有沒有鎖」。取不到就回 `None`，上層會降級。

use std::sync::Arc;
use std::time::Duration;

use super::presence::InputMonitor;

pub fn os_monitor() -> Arc<dyn InputMonitor> {
    #[cfg(target_os = "macos")]
    return Arc::new(MacMonitor);
    #[cfg(windows)]
    return Arc::new(WinMonitor);
    #[cfg(target_os = "linux")]
    return Arc::new(LinuxMonitor);
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    return Arc::new(super::presence::NullMonitor);
}

#[cfg(target_os = "macos")]
struct MacMonitor;

#[cfg(target_os = "macos")]
mod mac_ffi {
    use core_foundation_sys::dictionary::CFDictionaryRef;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        /// state 0＝kCGEventSourceStateCombinedSessionState；event_type `!0`＝kCGAnyInputEventType。
        pub fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
        pub fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    }
}

#[cfg(target_os = "macos")]
impl InputMonitor for MacMonitor {
    fn idle(&self) -> Option<Duration> {
        let s = unsafe { mac_ffi::CGEventSourceSecondsSinceLastEventType(0, u32::MAX) };
        (s.is_finite() && s >= 0.0).then(|| Duration::from_secs_f64(s))
    }

    fn screen_locked(&self) -> Option<bool> {
        use core_foundation::base::{CFType, TCFType};
        use core_foundation::boolean::CFBoolean;
        use core_foundation::dictionary::CFDictionary;
        use core_foundation::string::CFString;
        let raw = unsafe { mac_ffi::CGSessionCopyCurrentDictionary() };
        if raw.is_null() {
            return None;
        }
        let dict: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_create_rule(raw) };
        let key = CFString::new("CGSSessionScreenIsLocked");
        // 沒有這個鍵＝沒鎖定。
        Some(dict.find(&key).and_then(|v| v.downcast::<CFBoolean>()).map(bool::from).unwrap_or(false))
    }
}

#[cfg(windows)]
struct WinMonitor;

#[cfg(windows)]
impl InputMonitor for WinMonitor {
    fn idle(&self) -> Option<Duration> {
        use windows::Win32::System::SystemInformation::GetTickCount64;
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
            return None;
        }
        // dwTime 是 32 位元的開機毫秒數，會繞回；用 GetTickCount64 的低 32 位元相減才正確。
        let now = unsafe { GetTickCount64() } as u32;
        Some(Duration::from_millis(now.wrapping_sub(info.dwTime) as u64))
    }

    fn screen_locked(&self) -> Option<bool> {
        use windows::Win32::System::StationsAndDesktops::{CloseDesktop, OpenInputDesktop, DESKTOP_SWITCHDESKTOP, DESKTOP_CONTROL_FLAGS};
        // 螢幕鎖定時輸入桌面是 Winlogon 的安全桌面，一般程式開不起來。
        match unsafe { OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_SWITCHDESKTOP) } {
            Ok(h) => {
                let _ = unsafe { CloseDesktop(h) };
                Some(false)
            }
            Err(_) => Some(true),
        }
    }
}

#[cfg(target_os = "linux")]
struct LinuxMonitor;

#[cfg(target_os = "linux")]
impl InputMonitor for LinuxMonitor {
    fn idle(&self) -> Option<Duration> {
        use x11rb::connection::Connection;
        use x11rb::protocol::screensaver::ConnectionExt;
        // Wayland 沒有這個資訊（沒有 DISPLAY 時連不上就是 None）。
        let (conn, num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(num)?.root;
        let info = conn.screensaver_query_info(root).ok()?.reply().ok()?;
        Some(Duration::from_millis(info.ms_since_user_input as u64))
    }

    fn screen_locked(&self) -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_monitor_never_panics_and_reports_sane_values() {
        let m = os_monitor();
        if let Some(i) = m.idle() {
            assert!(i < Duration::from_secs(60 * 60 * 24 * 365), "{i:?}");
        }
        let _ = m.screen_locked();
    }
}
