//! macOS 的 Cmd+Q：把預設選單的 Quit 換成只發事件的自訂項目，讓前端決定要不要退出。
//!
//! 為什麼不在 `RunEvent::ExitRequested` 攔：實測（tauri 2.10、macOS）預設選單的 Quit
//! 走原生 `terminate:`，直接進到 `RunEvent::Exit`，`ExitRequested` 根本不會送出——
//! 有指令在跑時 Cmd+Q 會無聲無息地把它殺掉。所以攔截點只能是選單項目本身。
//!
//! 前端收到 [`QUIT_REQUESTED_EVENT`] 後走與視窗 ✕ 相同的流程（`useWindowCloseGuard`）：
//! 沒有忙碌分頁就 `destroy()` 視窗，最後一個視窗關閉後 runtime 自行退出。

use tauri::{AppHandle, Emitter};

/// 使用者按下 Quit（Cmd+Q）時通知前端的事件（無酬載）。
pub const QUIT_REQUESTED_EVENT: &str = "app://quit-requested";

/// 自訂 Quit 選單項目的 id。
pub const QUIT_MENU_ID: &str = "aiterm-quit";

/// 接在 `Builder::on_menu_event`。
pub fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    if event.id().as_ref() == QUIT_MENU_ID {
        if let Err(e) = app.emit(QUIT_REQUESTED_EVENT, ()) {
            log::warn!("emit {QUIT_REQUESTED_EVENT} failed: {e}");
        }
    }
}

/// 沿用 Tauri 的預設選單（App／Edit／View／Window…，複製貼上等快速鍵都靠它），
/// 只把 App 選單最後一項的 Quit 換成自訂項目，快速鍵同樣是 Cmd+Q。
#[cfg(target_os = "macos")]
pub fn install_quit_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, MenuItemKind};

    let menu = Menu::default(app.handle())?;
    let replaced = match menu.items()?.into_iter().next() {
        Some(MenuItemKind::Submenu(app_menu)) => match app_menu.items()?.into_iter().last() {
            Some(MenuItemKind::Predefined(quit)) => {
                app_menu.remove(&quit)?;
                let name = app.package_info().name.clone();
                app_menu.append(&MenuItem::with_id(
                    app,
                    QUIT_MENU_ID,
                    format!("Quit {name}"),
                    true,
                    Some("Cmd+Q"),
                )?)?;
                true
            }
            _ => false,
        },
        _ => false,
    };
    if !replaced {
        // 預設選單的結構變了：不換，退回原本的（無確認）Quit，但要看得見。
        log::warn!("找不到預設選單裡的 Quit 項目，Cmd+Q 不會有工作進行中的確認");
    }
    app.set_menu(menu)?;
    Ok(())
}
