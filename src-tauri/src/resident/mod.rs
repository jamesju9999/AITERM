//! 常駐：全域快捷鍵、托盤圖示、關閉縮到托盤、開機自啟。
//! 設計見 docs/superpowers/specs/2026-10-12-global-hotkey-tray-design.md。
//!
//! 視窗只用 `hide()`／`show()`，不卸載 webview：xterm 在沒有尺寸的元素上 resize 會崩潰，
//! 而且 PTY 與 Agent 都要在視窗隱藏時繼續跑。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::config::ConfigStore;
use crate::quit::QUIT_REQUESTED_EVENT;

/// 喚出視窗後通知前端聚焦輸入框（無酬載）。
pub const FOCUS_INPUT_EVENT: &str = "app://focus-input";
/// 托盤選單「新分頁」：前端開新分頁選單（無酬載）。
pub const NEW_TAB_EVENT: &str = "app://tray-new-tab";
/// 自啟時帶的旗標：啟動後不顯示視窗。
pub const MINIMIZED_FLAG: &str = "--minimized";

const TRAY_ID: &str = "aiterm-tray";
const MENU_SHOW: &str = "tray-show";
const MENU_NEW_TAB: &str = "tray-new-tab";
const MENU_QUIT: &str = "tray-quit";

/// 托盤是否成功建立。沒有托盤時絕不能把視窗藏起來，否則使用者找不回來。
#[derive(Default)]
pub struct TrayState(AtomicBool);

impl TrayState {
    fn set(&self, ok: bool) {
        self.0.store(ok, Ordering::SeqCst);
    }
    pub fn is_ok(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WindowAction {
    Show,
    Hide,
}

/// 快捷鍵／托盤左鍵的切換決策：只有「看得見且正拿著焦點」才隱藏，其餘一律叫出來。
pub fn decide_toggle(visible: bool, minimized: bool, focused: bool) -> WindowAction {
    if visible && !minimized && focused {
        WindowAction::Hide
    } else {
        WindowAction::Show
    }
}

/// 空字串＝停用（`Ok(None)`）；否則必須是能解析的 accelerator。
pub fn parse_hotkey(s: &str) -> Result<Option<Shortcut>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<Shortcut>()
        .map(Some)
        .map_err(|e| format!("無效的快捷鍵「{s}」：{e}"))
}

/// 該不該以隱藏視窗的方式啟動（自啟旗標且托盤可用）。
pub fn should_start_hidden(args: &[String], tray_ok: bool) -> bool {
    tray_ok && args.iter().any(|a| a == MINIMIZED_FLAG)
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        let _ = app.emit(FOCUS_INPUT_EVENT, ());
    }
}

pub fn toggle_main_window(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    let action = decide_toggle(
        w.is_visible().unwrap_or(false),
        w.is_minimized().unwrap_or(false),
        w.is_focused().unwrap_or(false),
    );
    match action {
        WindowAction::Hide => {
            let _ = w.hide();
        }
        WindowAction::Show => show_main_window(app),
    }
}

/// 關閉視窗時呼叫：`keep_in_tray` 開啟且托盤存在才隱藏，回傳是否已隱藏。
/// 回傳 false 時前端走原本的忙碌確認＋結束流程。
pub fn hide_for_close(app: &AppHandle) -> bool {
    let keep = app.state::<std::sync::Arc<ConfigStore>>().get().keep_in_tray;
    let tray_ok = app.state::<TrayState>().is_ok();
    if !(keep && tray_ok) {
        return false;
    }
    match app.get_webview_window("main") {
        Some(w) => w.hide().is_ok(),
        None => false,
    }
}

/// 把已註冊的快捷鍵從 `old` 換成 `new`。新值註冊失敗時舊值原樣保留。
pub fn apply_hotkey(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    let new_sc = parse_hotkey(new)?;
    let old_sc = parse_hotkey(old).ok().flatten();
    let gs = app.global_shortcut();
    if let Some(sc) = &old_sc {
        if gs.is_registered(*sc) {
            let _ = gs.unregister(*sc);
        }
    }
    if let Some(sc) = new_sc {
        if let Err(e) = gs.register(sc) {
            // 還原舊的，別讓使用者一失敗就什麼都沒有。
            if let Some(old_sc) = old_sc {
                let _ = gs.register(old_sc);
            }
            return Err(format!("無法註冊快捷鍵「{new}」（可能被其他程式占用）：{e}"));
        }
    }
    Ok(())
}

pub fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable().map_err(|e| e.to_string())
    } else {
        mgr.disable().map_err(|e| e.to_string())
    }
}

/// 全域快捷鍵外掛的 handler：只處理按下，切換視窗。
pub fn shortcut_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle_main_window(app);
            }
        })
        .build()
}

pub fn autostart_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec![MINIMIZED_FLAG]),
    )
}

fn install_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, MENU_SHOW, "Show AITerm", true, None::<&str>)?;
    let new_tab = MenuItem::with_id(app, MENU_NEW_TAB, "New Tab", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &new_tab, &sep, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("AITerm")
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_SHOW => show_main_window(app),
            MENU_NEW_TAB => {
                show_main_window(app);
                let _ = app.emit(NEW_TAB_EVENT, ());
            }
            MENU_QUIT => {
                // 先顯示視窗：忙碌確認框畫在視窗裡，視窗藏著使用者看不到。
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = app.emit(QUIT_REQUESTED_EVENT, ());
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// 啟動時呼叫：建托盤、註冊快捷鍵、依旗標決定是否藏起視窗。每一步失敗只記 log。
pub fn setup(app: &AppHandle) {
    let tray_ok = match install_tray(app) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("托盤建立失敗，關閉視窗將維持結束 App：{e}");
            false
        }
    };
    app.state::<TrayState>().set(tray_ok);

    let hotkey = app.state::<std::sync::Arc<ConfigStore>>().get().global_hotkey;
    if let Err(e) = apply_hotkey(app, "", &hotkey) {
        log::warn!("全域快捷鍵未啟用：{e}");
    }

    let args: Vec<String> = std::env::args().collect();
    if should_start_hidden(&args, tray_ok) {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.hide();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_hides_only_when_visible_unminimized_and_focused() {
        assert_eq!(decide_toggle(true, false, true), WindowAction::Hide);
        assert_eq!(decide_toggle(false, false, false), WindowAction::Show);
        assert_eq!(decide_toggle(true, true, true), WindowAction::Show);
        assert_eq!(decide_toggle(true, false, false), WindowAction::Show);
    }

    #[test]
    fn parse_hotkey_accepts_default_and_blank_disables() {
        assert!(parse_hotkey("Ctrl+`").unwrap().is_some());
        assert!(parse_hotkey("CmdOrCtrl+Shift+Space").unwrap().is_some());
        assert!(parse_hotkey("").unwrap().is_none());
        assert!(parse_hotkey("   ").unwrap().is_none());
    }

    #[test]
    fn parse_hotkey_rejects_garbage() {
        assert!(parse_hotkey("Ctrl+").is_err());
        assert!(parse_hotkey("nonsense key").is_err());
    }

    #[test]
    fn hidden_start_requires_flag_and_working_tray() {
        let with = vec!["aiterm".to_string(), MINIMIZED_FLAG.to_string()];
        let without = vec!["aiterm".to_string()];
        assert!(should_start_hidden(&with, true));
        assert!(!should_start_hidden(&with, false));
        assert!(!should_start_hidden(&without, true));
    }
}
