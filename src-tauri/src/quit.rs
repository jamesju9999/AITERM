//! 退出確認：macOS 的 Cmd+Q 不經前端的 `onCloseRequested`，而走
//! `RunEvent::ExitRequested`。這裡先攔下來，交給前端決定是否真的退出。

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

/// 攔下退出請求後通知前端的事件（無酬載）。
pub const QUIT_REQUESTED_EVENT: &str = "app://quit-requested";

/// 前端已確認「可以退出」的旗標。確認流程的最後一步（`destroy()` 視窗）
/// 會讓 runtime 再送一次 `ExitRequested`（最後一個視窗關閉），
/// 那一次必須放行，否則 App 會殘留成沒有視窗的殭屍程序。
#[derive(Default)]
pub struct QuitState {
    confirmed: AtomicBool,
}

/// 這次 `ExitRequested` 要不要攔下。
///
/// - `code` 是 `Some`：程式自己呼叫了 `app.exit(code)`（例如更新後重啟），不是
///   使用者要退出，一律放行。
/// - `confirmed`：前端已確認過，放行。
pub fn should_intercept_exit(code: Option<i32>, confirmed: bool) -> bool {
    code.is_none() && !confirmed
}

/// 前端在「確認可以退出」（或發現沒有任何忙碌分頁）之後、`destroy()` 視窗之前呼叫。
#[tauri::command]
pub fn set_quit_confirmed(state: State<'_, QuitState>) {
    state.confirmed.store(true, Ordering::SeqCst);
}

/// 接在 `App::run` 回呼裡。
pub fn on_run_event(app: &AppHandle, event: &tauri::RunEvent) {
    if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
        let confirmed = app.state::<QuitState>().confirmed.load(Ordering::SeqCst);
        if should_intercept_exit(*code, confirmed) {
            api.prevent_exit();
            if let Err(e) = app.emit(QUIT_REQUESTED_EVENT, ()) {
                log::warn!("emit {QUIT_REQUESTED_EVENT} failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_intercept_exit;

    #[test]
    fn user_initiated_quit_is_intercepted_until_confirmed() {
        assert!(should_intercept_exit(None, false));
    }

    #[test]
    fn confirmed_quit_passes_through() {
        assert!(!should_intercept_exit(None, true));
    }

    #[test]
    fn programmatic_exit_is_never_intercepted() {
        assert!(!should_intercept_exit(Some(0), false));
        assert!(!should_intercept_exit(Some(1), false));
    }
}
