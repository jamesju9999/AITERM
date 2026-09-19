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
    assert!(
        d.contains("StartupWMClass={{exec}}"),
        "缺 StartupWMClass：Tauri 預設樣板有這行，自訂樣板會整份取代預設，少了它 dock 無法把執行中的視窗歸到啟動器"
    );
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

    /// 建一個假的 PATH：`dpkg -L` 回報套件的檔案清單，`update-alternatives`
    /// 只把收到的參數記到 log。
    ///
    /// 固定值都刻意選成「實作不可能碰巧寫死」的樣子：`exec` 由呼叫端給（預設
    /// 用 fixture-bin-7，真實的執行檔名叫別的東西——crate 名是 app），清單裡
    /// 也夾了 sidecar 與圖示，逼腳本真的去挑 .desktop 並讀它的 Exec=。
    fn stub_env(exec: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let desktop = dir.path().join("AITerm.desktop");
        fs::write(&desktop, format!("[Desktop Entry]\nExec={exec} %F\nName=AITerm\n")).unwrap();
        let log = dir.path().join("alternatives.log");

        let write_stub = |name: &str, body: String| {
            let path = bin.join(name);
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        };
        write_stub(
            "dpkg",
            format!(
                "echo /usr/bin/uv\necho /usr/bin/other-sidecar\necho '{}'\necho /usr/share/icons/hicolor/128x128/apps/AITerm.png",
                desktop.display()
            ),
        );
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
        let (_guard, bin, log) = stub_env("fixture-bin-7");
        run_script("linux/postinst.sh", "configure", &bin);
        assert_eq!(
            logged(&log).trim(),
            "--install /usr/bin/x-terminal-emulator x-terminal-emulator /usr/bin/fixture-bin-7 40"
        );
    }

    #[test]
    fn postinst_uses_an_absolute_exec_path_as_is() {
        let (_guard, bin, log) = stub_env("/opt/fixture/bin-8");
        run_script("linux/postinst.sh", "configure", &bin);
        assert_eq!(
            logged(&log).trim(),
            "--install /usr/bin/x-terminal-emulator x-terminal-emulator /opt/fixture/bin-8 40"
        );
    }

    #[test]
    fn postinst_does_nothing_for_other_actions() {
        let (_guard, bin, log) = stub_env("fixture-bin-7");
        run_script("linux/postinst.sh", "abort-upgrade", &bin);
        assert_eq!(logged(&log), "");
    }

    #[test]
    fn prerm_unregisters_on_remove() {
        let (_guard, bin, log) = stub_env("fixture-bin-7");
        run_script("linux/prerm.sh", "remove", &bin);
        assert_eq!(logged(&log).trim(), "--remove x-terminal-emulator /usr/bin/fixture-bin-7");
    }

    #[test]
    fn prerm_uses_an_absolute_exec_path_as_is() {
        let (_guard, bin, log) = stub_env("/opt/fixture/bin-8");
        run_script("linux/prerm.sh", "remove", &bin);
        assert_eq!(logged(&log).trim(), "--remove x-terminal-emulator /opt/fixture/bin-8");
    }

    #[test]
    fn prerm_keeps_the_registration_during_an_upgrade() {
        let (_guard, bin, log) = stub_env("fixture-bin-7");
        run_script("linux/prerm.sh", "upgrade", &bin);
        assert_eq!(logged(&log), "", "升級不可移除註冊，否則使用者選的終端機會被重設");
    }
}

/// release.yml 的兩條 Linux .deb 腿會用內嵌 python 把 tauri.linux.conf.json
/// **整份改寫**（為了塞進 db2 sidecar 路徑），所以 conf 裡新增的任何東西，
/// 只要沒同步進那份 dict，就會在正式發佈的 .deb 裡靜默消失（MarkItDown 與
/// 終端機註冊都會如此）。這裡把 workflow 裡那段 python 抽出來實際執行，
/// 逐項比對 repo 裡提交的 conf——唯一允許的差異是 db2 sidecar 路徑。
#[cfg(unix)]
#[test]
fn release_workflow_regenerates_the_committed_linux_conf_except_the_db2_path() {
    use serde_json::Value;
    use std::process::Command;

    const PLACEHOLDER: &str = "PLACEHOLDER_DIR";
    let yml = read("../.github/workflows/release.yml");
    let lines: Vec<&str> = yml.lines().collect();

    let step = lines
        .iter()
        .position(|l| l.contains("name: Patch tauri.linux.conf.json with DB2 resources"))
        .expect("release.yml 找不到改寫 tauri.linux.conf.json 的步驟");
    let open = step
        + lines[step..]
            .iter()
            .position(|l| l.contains("python3 -c \""))
            .expect("該步驟裡找不到 python3 -c \"");
    let close = open
        + 1
        + lines[open + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with("\" > src-tauri/tauri.linux.conf.json"))
            .expect("找不到結尾的 \" > src-tauri/tauri.linux.conf.json");
    let body = &lines[open + 1..close];

    // YAML 的 block scalar 會先去掉共同縮排才交給 shell，這裡照做。
    let indent = body
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .expect("python 內容是空的");
    let source = body
        .iter()
        .map(|l| l.get(indent..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("${{ matrix.db2_sidecar_dir }}", PLACEHOLDER);

    // 真正執行時這段字在 shell 的雙引號裡；出現這些字元 shell 會先改寫它，
    // 此時直接餵給 python 的測試就不等價了。
    for bad in ['"', '$', '`', '\\'] {
        assert!(!source.contains(bad), "內嵌 python 含有會被 shell 雙引號改寫的字元 {bad:?}");
    }

    let out = match Command::new("python3").arg("-c").arg(&source).output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("略過：找不到 python3，無法執行 release.yml 內嵌的 python");
            return;
        }
        Err(e) => panic!("python3 執行失敗: {e}"),
    };
    assert!(out.status.success(), "內嵌 python 執行失敗: {}", String::from_utf8_lossy(&out.stderr));
    let mut generated: Value = serde_json::from_slice(&out.stdout).expect("內嵌 python 的輸出不是 JSON");
    let committed: Value = serde_json::from_str(&read("tauri.linux.conf.json")).expect("conf 不是合法 JSON");

    // 唯一允許的差異：db2 sidecar 那一筆。先確認它真的在，代換才有意義。
    let dropped = generated["bundle"]["resources"]
        .as_object_mut()
        .expect("workflow 產出的 bundle.resources 必須是 object")
        .remove(PLACEHOLDER);
    assert_eq!(dropped, Some(Value::from("db2-sidecar")), "workflow 的 resources 少了 db2 sidecar 那一筆");

    assert_eq!(generated["bundle"]["linux"], committed["bundle"]["linux"], "bundle.linux 沒同步進 release.yml");
    assert_eq!(generated["bundle"]["externalBin"], committed["bundle"]["externalBin"], "bundle.externalBin 沒同步");
    assert_eq!(generated["bundle"]["resources"], committed["bundle"]["resources"], "bundle.resources 沒同步");
    assert_eq!(generated, committed, "release.yml 重新產生的 conf 與提交的不同（除 db2 路徑外不應有差異）");
}
