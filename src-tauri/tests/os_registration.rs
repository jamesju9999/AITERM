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

#[test]
fn desktop_template_is_a_terminal_emulator_that_opens_directories() {
    let d = read("linux/aiterm.desktop");
    assert!(d.contains("TerminalEmulator"), "缺 TerminalEmulator 類別");
    assert!(d.contains("inode/directory"), "缺 MimeType inode/directory");
    assert!(d.contains("Exec={{exec}} %F"), "Exec 要接 %F 才會收到檔案管理員給的路徑");
    for var in ["{{name}}", "{{icon}}"] {
        assert!(d.contains(var), "樣板缺少變數 {var}");
    }
}

#[test]
fn linux_conf_points_at_the_template_and_both_scripts() {
    let conf: serde_json::Value =
        serde_json::from_str(&read("tauri.linux.conf.json")).expect("tauri.linux.conf.json 不是合法 JSON");
    let linux = &conf["bundle"]["linux"];
    // deb 與 rpm 各自要有樣板，否則其中一種套件就不會有終端機類別。
    for pkg in ["deb", "rpm"] {
        assert_eq!(
            linux[pkg]["desktopTemplate"], "linux/aiterm.desktop",
            "bundle.linux.{pkg}.desktopTemplate 不對"
        );
    }
    assert_eq!(linux["deb"]["postInstallScript"], "linux/postinst.sh");
    assert_eq!(linux["deb"]["preRemoveScript"], "linux/prerm.sh");
}

#[cfg(unix)]
mod maintainer_scripts {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    /// 建一個假的 PATH：`dpkg -L` 回報一個裝好的 .desktop，
    /// `update-alternatives` 只把收到的參數記到 log。
    fn stub_env() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let desktop = dir.path().join("AITerm.desktop");
        fs::write(&desktop, "[Desktop Entry]\nExec=AITerm %F\nName=AITerm\n").unwrap();
        let log = dir.path().join("alternatives.log");

        let write_stub = |name: &str, body: String| {
            let path = bin.join(name);
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        };
        write_stub("dpkg", format!("echo '{}'", desktop.display()));
        write_stub("update-alternatives", format!("echo \"$@\" >> '{}'", log.display()));
        (dir, bin, log)
    }

    fn run_script(script: &str, action: &str, bin: &PathBuf) {
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let status = Command::new("sh")
            .arg(root().join(script))
            .arg(action)
            .env("PATH", path)
            .env("DPKG_MAINTSCRIPT_PACKAGE", "aiterm")
            .status()
            .expect("sh 無法執行");
        assert!(status.success(), "{script} {action} 應該成功結束");
    }

    fn logged(log: &PathBuf) -> String {
        fs::read_to_string(log).unwrap_or_default()
    }

    #[test]
    fn scripts_are_valid_shell() {
        for script in ["linux/postinst.sh", "linux/prerm.sh"] {
            let status = Command::new("sh").arg("-n").arg(root().join(script)).status().unwrap();
            assert!(status.success(), "{script} 語法錯誤");
        }
    }

    /// 其它測試都用 `sh <path>` 執行，不看執行位元，所以少了 +x 也會綠。
    /// dpkg 要求維護腳本可執行；tauri-bundler 2.8 目前會用 0755 建立 .deb 裡的
    /// 目的檔（create_script_file_from_path），所以不靠原檔的位元也裝得起來，
    /// 但這是實作細節、可能隨版本改變，因此仍把 git mode 守在 100755。
    #[test]
    fn scripts_are_executable_because_dpkg_requires_it() {
        for script in ["linux/postinst.sh", "linux/prerm.sh"] {
            let mode = fs::metadata(root().join(script)).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "{script} 缺少執行位元（git mode 要是 100755），dpkg 會拒絕執行");
        }
    }

    #[test]
    fn postinst_registers_the_binary_named_in_the_installed_desktop_file() {
        let (_guard, bin, log) = stub_env();
        run_script("linux/postinst.sh", "configure", &bin);
        assert_eq!(
            logged(&log).trim(),
            "--install /usr/bin/x-terminal-emulator x-terminal-emulator /usr/bin/AITerm 40"
        );
    }

    #[test]
    fn postinst_does_nothing_for_other_actions() {
        let (_guard, bin, log) = stub_env();
        run_script("linux/postinst.sh", "abort-upgrade", &bin);
        assert_eq!(logged(&log), "");
    }

    #[test]
    fn prerm_unregisters_on_remove() {
        let (_guard, bin, log) = stub_env();
        run_script("linux/prerm.sh", "remove", &bin);
        assert_eq!(logged(&log).trim(), "--remove x-terminal-emulator /usr/bin/AITerm");
    }

    #[test]
    fn prerm_keeps_the_registration_during_an_upgrade() {
        let (_guard, bin, log) = stub_env();
        run_script("linux/prerm.sh", "upgrade", &bin);
        assert_eq!(logged(&log), "", "升級不可移除註冊，否則使用者選的終端機會被重設");
    }
}
