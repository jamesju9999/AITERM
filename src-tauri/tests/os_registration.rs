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

/// 字串比對抓不到結構錯誤（例如漏寫某個 dict 的 LSHandlerRank——缺省值等同
/// Owner，會搶走預設開啟程式）。macOS 上用系統自帶的 plutil 轉成 JSON 再逐個
/// dict 驗證；其它平台沒有 plutil，仍有上面的字串測試把關。
#[cfg(target_os = "macos")]
#[test]
fn info_plist_document_types_have_the_expected_structure() {
    use serde_json::{json, Value};
    let out = std::process::Command::new("plutil")
        .args(["-convert", "json", "-o", "-", "Info.plist"])
        .current_dir(root())
        .output()
        .expect("執行不了 plutil");
    assert!(out.status.success(), "plutil 拒絕 Info.plist: {}", String::from_utf8_lossy(&out.stderr));
    let plist: Value = serde_json::from_slice(&out.stdout).expect("plutil 輸出不是 JSON");
    let types = plist["CFBundleDocumentTypes"].as_array().expect("CFBundleDocumentTypes 必須是 array");
    let expected = [
        ("Viewer", json!(["public.folder"])),
        ("Shell", json!(["com.apple.terminal.shell-script", "public.shell-script"])),
    ];
    assert_eq!(types.len(), expected.len(), "document type dict 數量不對");
    for (role, utis) in expected {
        let t = types
            .iter()
            .find(|t| t["CFBundleTypeRole"] == role)
            .unwrap_or_else(|| panic!("缺少 role={role} 的 dict"));
        assert_eq!(t["LSItemContentTypes"], utis, "role={role} 的 UTI 不對");
        assert_eq!(t["LSHandlerRank"], "Alternate", "role={role} 必須明寫 Alternate（缺省等同 Owner）");
    }
}
