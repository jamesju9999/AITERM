//! Wayland 的全域快捷鍵：走 xdg-desktop-portal 的 GlobalShortcuts。
//!
//! `tauri-plugin-global-shortcut`（global-hotkey）在 Linux 只會用 X11 的 XGrabKey，
//! Wayland 桌面不允許程式自己搶鍵，註冊永遠不會觸發。Wayland 的做法是向 portal 申請，
//! 由桌面（GNOME 48+、KDE 5.27+ 等）彈出授權並讓使用者確認實際綁的按鍵。
//! portal 只接受「偏好的」按鍵，最後綁成什麼由桌面決定。

/// Tauri accelerator（如 `Ctrl+\``、`CmdOrCtrl+Shift+Space`）轉成 XDG shortcuts 規格的
/// trigger（如 `CTRL+grave`、`CTRL+SHIFT+space`）。看不懂就回 `None`（交給桌面讓使用者自己綁）。
pub fn to_xdg_trigger(accel: &str) -> Option<String> {
    let mut mods: Vec<&'static str> = Vec::new();
    let mut key: Option<String> = None;
    let parts: Vec<&str> = accel.split('+').map(str::trim).collect();
    // 「Ctrl++」這種以 + 當按鍵的寫法：split 之後尾端是空字串。
    let plus_key = parts.len() >= 2 && parts.last() == Some(&"") && parts[parts.len() - 2] == "";
    let n = if plus_key { parts.len() - 2 } else { parts.len() };
    for (i, p) in parts.iter().take(n).enumerate() {
        let lower = p.to_ascii_lowercase();
        let m = match lower.as_str() {
            "ctrl" | "control" | "cmdorctrl" | "commandorcontrol" => Some("CTRL"),
            "alt" | "option" => Some("ALT"),
            "shift" => Some("SHIFT"),
            "super" | "cmd" | "command" | "meta" => Some("LOGO"),
            _ => None,
        };
        match m {
            Some(m) => {
                if !mods.contains(&m) {
                    mods.push(m);
                }
            }
            None if i == n - 1 => key = Some(keysym(p)?),
            None => return None,
        }
    }
    if plus_key {
        key = Some("plus".to_string());
    }
    let key = key?;
    let mut out: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
    out.push(key);
    Some(out.join("+"))
}

/// Tauri 的按鍵名轉成 X keysym 名稱。
fn keysym(k: &str) -> Option<String> {
    if k.chars().count() == 1 {
        let c = k.chars().next()?;
        let name = match c {
            '`' => "grave",
            '-' => "minus",
            '=' => "equal",
            ',' => "comma",
            '.' => "period",
            '/' => "slash",
            ';' => "semicolon",
            '\'' => "apostrophe",
            '[' => "bracketleft",
            ']' => "bracketright",
            '\\' => "backslash",
            c if c.is_ascii_alphabetic() => return Some(c.to_ascii_lowercase().to_string()),
            c if c.is_ascii_digit() => return Some(c.to_string()),
            _ => return None,
        };
        return Some(name.to_string());
    }
    let lower = k.to_ascii_lowercase();
    let name = match lower.as_str() {
        "space" => "space",
        "enter" | "return" => "Return",
        "tab" => "Tab",
        "esc" | "escape" => "Escape",
        "backspace" => "BackSpace",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "plus" => "plus",
        f if f.starts_with('f') && f[1..].parse::<u8>().map_or(false, |n| (1..=24).contains(&n)) => {
            return Some(format!("F{}", &f[1..]));
        }
        _ => return None,
    };
    Some(name.to_string())
}

/// portal 用 App ID 找對應的 `<id>.desktop` 才認得出呼叫者。AppImage 不會裝 .desktop，
/// deb 的檔名又不是這個 ID，所以自己在使用者的 applications 目錄寫一份（隱藏，不進選單）。
pub fn desktop_entry_content(exec: &str) -> String {
    let exec = exec.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "[Desktop Entry]\nType=Application\nName=AITerm\nExec=\"{exec}\"\nTerminal=false\nNoDisplay=true\n"
    )
}

/// 目前是不是 Wayland 工作階段。
#[cfg(target_os = "linux")]
pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").map_or(false, |v| v.eq_ignore_ascii_case("wayland"))
        || (std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none())
}

#[cfg(target_os = "linux")]
mod session {
    use std::sync::Mutex;

    use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
    use ashpd::desktop::CreateSessionOptions;
    use futures_util::StreamExt;
    use tauri::AppHandle;

    const SHORTCUT_ID: &str = "toggle-window";
    /// 與 tauri.conf.json 的 identifier 一致。
    const APP_ID: &str = "com.aiterm.app";

    /// 目前這一份 portal session 的背景工作；換快捷鍵時中止舊的（session 隨之關閉）。
    static CURRENT: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);

    /// 停掉舊的 session；`hotkey` 非空就開新的。失敗只記 log（portal 不存在、使用者拒絕等）。
    pub fn restart(app: &AppHandle, hotkey: &str) {
        let mut cur = CURRENT.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(h) = cur.take() {
            h.abort();
        }
        let hotkey = hotkey.trim().to_string();
        if hotkey.is_empty() {
            return;
        }
        let app = app.clone();
        *cur = Some(tauri::async_runtime::spawn(async move {
            if let Err(e) = run(&app, &hotkey).await {
                log::warn!("Wayland 全域快捷鍵未啟用（需要支援 GlobalShortcuts 的桌面）：{e}");
            }
        }));
    }

    /// 確保 `~/.local/share/applications/<APP_ID>.desktop` 存在且指向目前這個執行檔。
    fn ensure_desktop_entry() -> std::io::Result<()> {
        let exe = std::env::var("APPIMAGE")
            .ok()
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
            .map(Ok)
            .unwrap_or_else(std::env::current_exe)?;
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no HOME"))?;
        let dir = data.join("applications");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{APP_ID}.desktop"));
        let content = super::desktop_entry_content(&exe.to_string_lossy());
        if std::fs::read_to_string(&path).map_or(true, |old| old != content) {
            std::fs::write(&path, content)?;
        }
        Ok(())
    }

    async fn run(app: &AppHandle, hotkey: &str) -> Result<(), ashpd::Error> {
        // 非沙盒（deb／AppImage）的程式 portal 認不出身分，GlobalShortcuts 會被拒絕或靜默無效。
        // 先在「同一條連線」上登記 App ID（登記是跟連線綁的）；失敗不致命，照樣往下試。
        if let Err(e) = ensure_desktop_entry() {
            log::warn!("Wayland 全域快捷鍵：無法寫入 {APP_ID}.desktop：{e}");
        }
        let connection = ashpd::zbus::Connection::session().await?;
        match ashpd::AppID::try_from(APP_ID) {
            Ok(id) => match ashpd::register_host_app_with_connection(connection.clone(), id).await {
                Ok(()) => log::info!("Wayland 全域快捷鍵：已向 portal 登記 App ID {APP_ID}"),
                Err(e) => log::warn!("Wayland 全域快捷鍵：登記 App ID 失敗（照常嘗試）：{e}"),
            },
            Err(e) => log::warn!("Wayland 全域快捷鍵：App ID 無效：{e}"),
        }
        let portal = GlobalShortcuts::with_connection(connection).await?;
        let session = portal.create_session(CreateSessionOptions::default()).await?;
        let mut shortcut = NewShortcut::new(SHORTCUT_ID, "Show / hide AITerm");
        let trigger = super::to_xdg_trigger(hotkey);
        if let Some(t) = trigger.as_deref() {
            shortcut = shortcut.preferred_trigger(t);
        }
        let bound = portal
            .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
            .await?
            .response()?;
        for s in bound.shortcuts() {
            log::info!("Wayland 全域快捷鍵已綁定：{}（{}）", s.id(), s.trigger_description());
        }
        let mut activated = portal.receive_activated().await?;
        while let Some(ev) = activated.next().await {
            if ev.shortcut_id() == SHORTCUT_ID {
                super::super::toggle_main_window(app);
            }
        }
        // session 要活到這裡；stream 結束代表 portal 那端關掉了。
        drop(session);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub use session::restart;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_default_and_common_accelerators() {
        assert_eq!(to_xdg_trigger("Ctrl+`").as_deref(), Some("CTRL+grave"));
        assert_eq!(to_xdg_trigger("CmdOrCtrl+Shift+Space").as_deref(), Some("CTRL+SHIFT+space"));
        assert_eq!(to_xdg_trigger("Alt+F4").as_deref(), Some("ALT+F4"));
        assert_eq!(to_xdg_trigger("Super+A").as_deref(), Some("LOGO+a"));
        assert_eq!(to_xdg_trigger("Ctrl+Alt+1").as_deref(), Some("CTRL+ALT+1"));
        assert_eq!(to_xdg_trigger("Ctrl+PageUp").as_deref(), Some("CTRL+Page_Up"));
    }

    #[test]
    fn desktop_entry_quotes_exec_path() {
        let c = desktop_entry_content("/home/a b/AITerm \"x\".AppImage");
        assert!(c.contains("Exec=\"/home/a b/AITerm \\\"x\\\".AppImage\"\n"), "{c}");
        assert!(c.contains("NoDisplay=true"));
    }

    #[test]
    fn dedups_modifiers_and_handles_plus_key() {
        assert_eq!(to_xdg_trigger("Ctrl+Control+K").as_deref(), Some("CTRL+k"));
        assert_eq!(to_xdg_trigger("Ctrl++").as_deref(), Some("CTRL+plus"));
    }

    #[test]
    fn unknown_keys_yield_none() {
        assert_eq!(to_xdg_trigger("Ctrl+"), None);
        assert_eq!(to_xdg_trigger("Ctrl+Banana"), None);
        assert_eq!(to_xdg_trigger(""), None);
        assert_eq!(to_xdg_trigger("A+B"), None);
    }
}
