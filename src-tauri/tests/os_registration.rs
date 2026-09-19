//! 驗證「讓作業系統認得 AITerm 是終端機」的靜態註冊檔：macOS Info.plist、
//! Linux .desktop 樣板與 deb 維護腳本。這些檔案只在打包時才被讀到，
//! 一旦寫壞，要到裝了套件才會發現，所以在這裡用測試釘住。

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("讀不到 {rel}: {e}"))
}

#[test]
fn info_plist_declares_folders_and_shell_scripts_as_openable_without_stealing_defaults() {
    let plist = read("Info.plist");
    for uti in ["public.folder", "com.apple.terminal.shell-script", "public.shell-script"] {
        assert!(plist.contains(uti), "Info.plist 缺少 {uti}");
    }
    assert!(plist.contains("CFBundleDocumentTypes"));
    assert!(plist.contains("<string>Alternate</string>"), "必須是 Alternate，不能搶預設");
    assert!(!plist.contains("<string>Owner</string>"), "不可宣告成 Owner");
}
