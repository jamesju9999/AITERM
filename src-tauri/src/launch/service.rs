//! macOS Finder 服務「在 AITerm 開啟」（Info.plist 的 `NSServices`）。
//! 設計見 docs/superpowers/specs/2026-10-15-macos-finder-service-design.md。
//!
//! 選取項目怎麼變成分頁的規則（`service_requests`）是純函式、跨平台可測；
//! 接收 Finder 呼叫的 Objective-C 類別只在 macOS 編譯。

use std::path::PathBuf;

use super::LaunchRequest;

/// Info.plist `NSServices[0].NSMessage`；macOS 會呼叫 `<這個名稱>:userData:error:`。
pub const SERVICE_MESSAGE: &str = "openInAITerm";

/// Finder 選取的項目 → 要開的分頁。資料夾開自己、檔案開所在的資料夾；
/// 不執行任何檔案（腳本也一樣，只切目錄）。不存在的略過，同一個資料夾只開一次。
pub fn service_requests(paths: &[PathBuf]) -> Vec<LaunchRequest> {
    let mut seen = std::collections::HashSet::new();
    paths
        .iter()
        .filter_map(|p| {
            if p.is_dir() {
                Some(p.clone())
            } else if p.exists() {
                p.parent().map(|d| d.to_path_buf())
            } else {
                None
            }
        })
        .filter(|dir| seen.insert(dir.clone()))
        .map(|dir| LaunchRequest { cwd: Some(dir.to_string_lossy().into_owned()), script: None, command: None })
        .collect()
}

#[cfg(target_os = "macos")]
pub use macos::register;

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::OnceLock;

    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{define_class, msg_send, ClassType, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSPasteboard, NSPasteboardTypeFileURL, NSUpdateDynamicServices};
    use objc2_foundation::{NSString, NSURL};
    use tauri::AppHandle;

    use super::service_requests;

    static APP: OnceLock<AppHandle> = OnceLock::new();

    define_class!(
        // Finder 呼叫服務時，macOS 會對 NSApp 的 servicesProvider 送
        // `openInAITerm:userData:error:`（名稱來自 Info.plist 的 NSMessage）。
        #[unsafe(super(NSObject))]
        #[name = "AITermServiceProvider"]
        struct ServiceProvider;

        impl ServiceProvider {
            #[unsafe(method(openInAITerm:userData:error:))]
            fn open_in_aiterm(&self, pboard: &NSPasteboard, _user_data: *mut NSString, _error: *mut *mut NSString) {
                let Some(app) = APP.get() else { return };
                let mut paths = Vec::new();
                if let Some(items) = pboard.pasteboardItems() {
                    for item in items.iter() {
                        // SAFETY: AppKit 匯出的常數字串，整個行程期間都有效。
                        let Some(raw) = item.stringForType(unsafe { NSPasteboardTypeFileURL }) else { continue };
                        if let Some(path) = resolve_file_url(&raw.to_string()) {
                            paths.push(path);
                        }
                    }
                }
                let requests = service_requests(&paths);
                log::info!("Finder service: {} item(s) -> {} tab(s)", paths.len(), requests.len());
                if !requests.is_empty() {
                    crate::launch::enqueue_and_notify(app, requests);
                    crate::launch::raise_main_window(app);
                }
            }
        }
    );

    /// 剪貼簿上的 `public.file-url` 字串 → 路徑。Finder 給的是檔案參照網址
    /// （`file:///.file/id=…`），不能直接當路徑用，要交給 NSURL 解析成真正的檔案路徑；
    /// 一般的 `file://` 網址走同一條路也沒問題。
    pub(super) fn resolve_file_url(raw: &str) -> Option<std::path::PathBuf> {
        let url = NSURL::URLWithString(&NSString::from_str(raw))?;
        let path = url.filePathURL()?.path()?;
        Some(std::path::PathBuf::from(path.to_string()))
    }

    #[cfg(test)]
    mod tests {
        use super::resolve_file_url;
        use objc2_foundation::{NSString, NSURL};

        /// Finder 放到剪貼簿的是「檔案參照網址」（file:///.file/id=…），不是一般路徑。
        /// 2026-10-15 v1.41.1 實測：服務收到 1 個項目、開出 0 個分頁。
        #[test]
        fn a_finder_file_reference_url_resolves_to_the_real_folder() {
            let dir = tempfile::tempdir().unwrap();
            let real = dir.path().canonicalize().unwrap();
            let url = NSURL::fileURLWithPath(&NSString::from_str(&real.to_string_lossy()));
            let reference = url.fileReferenceURL().expect("拿不到檔案參照網址");
            let raw = reference.absoluteString().unwrap().to_string();
            assert!(raw.starts_with("file:///.file/id="), "前提不成立，參照網址長這樣：{raw}");
            let got = resolve_file_url(&raw).expect("解析不出路徑");
            assert_eq!(got.canonicalize().unwrap(), real);
        }

        #[test]
        fn a_plain_file_url_with_spaces_still_works() {
            let dir = tempfile::tempdir().unwrap();
            let sub = dir.path().join("BenQ Installer");
            std::fs::create_dir(&sub).unwrap();
            let raw = NSURL::fileURLWithPath(&NSString::from_str(&sub.to_string_lossy())).absoluteString().unwrap().to_string();
            assert_eq!(resolve_file_url(&raw).unwrap().canonicalize().unwrap(), sub.canonicalize().unwrap());
        }
    }

    /// 在主執行緒（Tauri `setup`）呼叫：把 provider 掛到 NSApp 並請系統更新服務清單。
    pub fn register(app: &AppHandle) {
        let _ = APP.set(app.clone());
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("Finder service: not on the main thread, service not registered");
            return;
        };
        // SAFETY: NSObject 子類別、沒有 ivar，`new` 就是 alloc+init。
        let provider: Retained<ServiceProvider> = unsafe { msg_send![ServiceProvider::class(), new] };
        let ns_app = NSApplication::sharedApplication(mtm);
        // SAFETY: provider 實作了 Info.plist 宣告的 selector。servicesProvider 不持有它，
        // 所以刻意洩漏、讓它活到行程結束。
        unsafe { ns_app.setServicesProvider(Some(&provider)) };
        std::mem::forget(provider);
        NSUpdateDynamicServices();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cwds(reqs: &[LaunchRequest]) -> Vec<String> {
        reqs.iter().map(|r| r.cwd.clone().unwrap_or_default()).collect()
    }

    fn s(p: &std::path::Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn a_folder_opens_a_tab_in_itself() {
        let dir = tempfile::tempdir().unwrap();
        let got = service_requests(&[dir.path().to_path_buf()]);
        assert_eq!(cwds(&got), vec![s(dir.path())]);
    }

    #[test]
    fn a_file_opens_a_tab_in_its_folder_and_is_never_run() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("deploy.sh");
        fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
        let got = service_requests(&[script]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cwd, Some(s(dir.path())));
        assert_eq!(got[0].script, None);
        assert_eq!(got[0].command, None);
    }

    #[test]
    fn several_items_open_one_tab_per_distinct_folder_in_order() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let f1 = a.path().join("x.txt");
        let f2 = a.path().join("y.txt");
        fs::write(&f1, "").unwrap();
        fs::write(&f2, "").unwrap();
        let got = service_requests(&[f1, b.path().to_path_buf(), f2, a.path().to_path_buf()]);
        assert_eq!(cwds(&got), vec![s(a.path()), s(b.path())]);
    }

    #[test]
    fn missing_paths_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let got = service_requests(&[gone, dir.path().to_path_buf()]);
        assert_eq!(cwds(&got), vec![s(dir.path())]);
    }
}
