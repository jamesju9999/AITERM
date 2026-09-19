//! 讓 AITerm 被當成終端機使用：把「開這個資料夾／跑這個指令」的請求，
//! 從冷啟動 argv、第二次啟動、macOS Opened 三個入口收斂成同一個佇列。

pub mod parse;
pub mod queue;

pub use parse::{args_from_file_urls, parse_args, LaunchRequest};
pub use queue::LaunchQueue;

use tauri::{AppHandle, Emitter, Manager, State};

/// 有新請求入列時發給前端的事件（無酬載——前端收到後自己 `take_launch_requests`）。
pub const PENDING_EVENT: &str = "launch-request-pending";

/// 前端取走所有待處理請求。
#[tauri::command]
pub fn take_launch_requests(queue: State<'_, LaunchQueue>) -> Vec<LaunchRequest> {
    queue.take()
}

/// 入列並通知前端。沒有請求就什麼都不做。
pub fn enqueue_and_notify(app: &AppHandle, requests: Vec<LaunchRequest>) {
    if requests.is_empty() {
        return;
    }
    app.state::<LaunchQueue>().push(requests);
    if let Err(e) = app.emit(PENDING_EVENT, ()) {
        log::warn!("emit {PENDING_EVENT} failed: {e}");
    }
}

/// single-instance 外掛傳來的 cwd 字串 → 可用的工作目錄。
/// 外掛在取不到 cwd（目錄已被刪、無權限）或路徑不是合法 UTF-8 時會傳空字串
/// （`current_dir().unwrap_or_default().to_str().unwrap_or_default()`），
/// 這代表「不知道」。必須回 `None`：`parse_args` 對只有 `-e` 的請求會退回這個
/// cwd，若給 `Some("")`，新分頁的起始目錄就會變成空字串而不是「用預設」。
fn invoking_cwd(cwd: &str) -> Option<&std::path::Path> {
    if cwd.is_empty() {
        None
    } else {
        Some(std::path::Path::new(cwd))
    }
}

/// 把主視窗拉到前景（還原最小化、顯示、取得焦點）。
fn raise_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 第二次啟動（single-instance 外掛的 callback）：解析、入列、把視窗拉到前景。
pub fn on_second_instance(app: &AppHandle, argv: Vec<String>, cwd: String) {
    let requests = parse_args(&argv, invoking_cwd(&cwd));
    enqueue_and_notify(app, requests);
    raise_main_window(app);
}

/// macOS「用 AITerm 開啟」／拖到 Dock 圖示。其它平台沒有這個事件。
pub fn on_run_event(app: &AppHandle, event: &tauri::RunEvent) {
    #[cfg(target_os = "macos")]
    {
        if let tauri::RunEvent::Opened { urls } = event {
            let argv = args_from_file_urls(urls);
            let requests = parse_args(&argv, None);
            // 沒解析出任何請求就不搶焦點。
            if !requests.is_empty() {
                enqueue_and_notify(app, requests);
                raise_main_window(app);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, event);
    }
}

#[cfg(test)]
mod tests {
    use super::{invoking_cwd, parse_args};

    fn ls_argv() -> Vec<String> {
        ["aiterm", "-e", "ls"].map(String::from).to_vec()
    }

    #[test]
    fn empty_cwd_leaves_a_command_only_request_without_a_start_dir() {
        let got = parse_args(&ls_argv(), invoking_cwd(""));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cwd, None);
        assert_eq!(got[0].command, Some(vec!["ls".to_string()]));
    }

    #[test]
    fn non_empty_cwd_becomes_the_start_dir_of_a_command_only_request() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_string_lossy().into_owned();
        let got = parse_args(&ls_argv(), invoking_cwd(&cwd));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cwd, Some(cwd));
        assert_eq!(got[0].command, Some(vec!["ls".to_string()]));
    }
}
