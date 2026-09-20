//! 驗證「讓作業系統認得 AITerm 是終端機」的靜態註冊檔：macOS Info.plist、
//! Linux .desktop 樣板與 deb 維護腳本。這些檔案只在打包時才被讀到，
//! 一旦寫壞，要到裝了套件才會發現，所以在這裡用測試釘住。

use std::fs;
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::{Mutex, MutexGuard};

/// 序列化所有「寫出可執行檔再執行它」或會 spawn 行程的測試。
/// Linux 上，一個執行緒剛寫完並關閉的檔案，可能因為另一條執行緒同時 fork、
/// 子行程還握著那個寫入用的 fd，而讓 exec 得到 ETXTBSY（"Text file busy"，
/// rust-lang/rust#114554）。這個 flake 在 macOS 上重現不出來，鎖是依已知
/// 成因做的預防，並未在實機上觀察到它消失。
/// 持有者若 panic（測試本來就會 assert 失敗），鎖會中毒，所以取鎖時無視中毒。
#[cfg(unix)]
static PROCESS_LOCK: Mutex<()> = Mutex::new(());

#[cfg(unix)]
fn process_lock() -> MutexGuard<'static, ()> {
    PROCESS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

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
    let _lock = process_lock();
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
    /// 假的 `dpkg` 也會檢查收到的參數必須剛好是 `-L aiterm`（測試把
    /// DPKG_MAINTSCRIPT_PACKAGE 設成 aiterm），否則腳本讀錯環境變數時，
    /// 實機上會變成「安裝成功但從不註冊」，測試卻什麼都看不出來。
    ///
    /// 回傳的第一個元素是 [`process_lock`] 的鎖：由這裡發出，測試就不會忘記拿；
    /// 它在 tuple 拆開後最後才被釋放，所以涵蓋整個測試（含清掉暫存目錄）。
    fn stub_env(exec: &str) -> (MutexGuard<'static, ()>, tempfile::TempDir, PathBuf, PathBuf) {
        let lock = process_lock();
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
                "[ \"$*\" = \"-L aiterm\" ] || exit 1\necho /usr/bin/uv\necho /usr/bin/other-sidecar\necho '{}'\necho /usr/share/icons/hicolor/128x128/apps/AITerm.png",
                desktop.display()
            ),
        );
        write_stub("update-alternatives", format!("echo \"$@\" >> '{}'", log.display()));
        (lock, dir, bin, log)
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
        let _lock = process_lock();
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

    /// dpkg 直接照 shebang 執行維護腳本；`#!/bin/sh\r` 會變成找不到直譯器。
    /// repo 之前發生過 CRLF 汙染，`.gitattributes` 的 `eol=lf` 是跨平台的釘子；
    /// 這個測試只在 unix 跑，因為 Windows 的 CI checkout 可能被 autocrlf 改寫。
    #[test]
    fn linux_packaging_files_have_no_carriage_returns() {
        for file in ["linux/postinst.sh", "linux/prerm.sh", "linux/aiterm.desktop"] {
            let bytes = fs::read(root().join(file)).unwrap();
            assert!(!bytes.contains(&b'\r'), "{file} 含有 CR（\\r）位元組，dpkg 會找不到直譯器");
        }
    }

    #[test]
    fn postinst_registers_the_binary_named_in_the_installed_desktop_file() {
        let (_lock, _guard, bin, log) = stub_env("fixture-bin-7");
        run_script("linux/postinst.sh", "configure", &bin);
        assert_eq!(
            logged(&log).trim(),
            "--install /usr/bin/x-terminal-emulator x-terminal-emulator /usr/bin/fixture-bin-7 10"
        );
    }

    #[test]
    fn postinst_uses_an_absolute_exec_path_as_is() {
        let (_lock, _guard, bin, log) = stub_env("/opt/fixture/bin-8");
        run_script("linux/postinst.sh", "configure", &bin);
        assert_eq!(
            logged(&log).trim(),
            "--install /usr/bin/x-terminal-emulator x-terminal-emulator /opt/fixture/bin-8 10"
        );
    }

    #[test]
    fn postinst_does_nothing_for_other_actions() {
        let (_lock, _guard, bin, log) = stub_env("fixture-bin-7");
        run_script("linux/postinst.sh", "abort-upgrade", &bin);
        assert_eq!(logged(&log), "");
    }

    #[test]
    fn prerm_unregisters_on_remove() {
        let (_lock, _guard, bin, log) = stub_env("fixture-bin-7");
        run_script("linux/prerm.sh", "remove", &bin);
        assert_eq!(logged(&log).trim(), "--remove x-terminal-emulator /usr/bin/fixture-bin-7");
    }

    #[test]
    fn prerm_uses_an_absolute_exec_path_as_is() {
        let (_lock, _guard, bin, log) = stub_env("/opt/fixture/bin-8");
        run_script("linux/prerm.sh", "remove", &bin);
        assert_eq!(logged(&log).trim(), "--remove x-terminal-emulator /opt/fixture/bin-8");
    }

    #[test]
    fn prerm_keeps_the_registration_during_an_upgrade() {
        let (_lock, _guard, bin, log) = stub_env("fixture-bin-7");
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

    let _lock = process_lock();
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

// ── Windows：檔案總管右鍵選單（NSIS hook）──

fn hooks_bytes() -> Vec<u8> {
    fs::read(root().join("installer/hooks.nsh")).unwrap_or_else(|e| panic!("讀不到 installer/hooks.nsh: {e}"))
}

/// hook 內容，去掉 BOM 與註解行（註解裡出現的字不算數）。
fn hooks_code() -> String {
    let text = String::from_utf8(hooks_bytes()).expect("hooks.nsh 必須是 UTF-8");
    text.trim_start_matches('\u{feff}')
        .lines()
        .filter(|l| !l.trim_start().starts_with(';'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn nsis_hooks_is_utf8_with_bom_so_the_chinese_label_survives() {
    assert!(
        hooks_bytes().starts_with(&[0xEF, 0xBB, 0xBF]),
        "hooks.nsh 必須是 UTF-8 with BOM：那是 NSIS 唯一明確的編碼宣告，沒有 BOM 的檔案會依 ANSI 字碼頁轉換（除非指定 /charset），\
         而檔案裡有中文標籤。POSIX 版 makensis 把沒有 BOM 的檔案當 UTF-8 讀，所以編譯測試看不出差別，只有這條把關"
    );
}

#[test]
fn windows_conf_points_at_the_installer_hooks_and_keeps_the_existing_nsis_settings() {
    let conf: serde_json::Value = serde_json::from_str(&read("tauri.windows.conf.json")).expect("conf 不是合法 JSON");
    let nsis = &conf["bundle"]["windows"]["nsis"];
    assert_eq!(nsis["installerHooks"], "installer/hooks.nsh");
    for k in ["headerImage", "sidebarImage", "installerIcon"] {
        assert!(nsis[k].is_string(), "nsis.{k} 不見了");
    }
    assert!(root().join("installer/hooks.nsh").exists());
}

/// 取出 `!macro NAME … !macroend` 之間的內容（NAME 只要是開頭就行，例如
/// `AITERM_ADD_VERB` 後面還有參數）。
fn hook_macro_body(code: &str, name: &str) -> String {
    let marker = format!("!macro {name}");
    code.split(&marker)
        .nth(1)
        .and_then(|rest| rest.split("!macroend").next())
        .unwrap_or_else(|| panic!("找不到 {marker}，或它沒有對應的 !macroend"))
        .to_string()
}

#[test]
fn hooks_register_folder_background_and_drive_verbs_and_remove_them_on_uninstall() {
    let h = hooks_code();
    assert!(h.contains("!macro NSIS_HOOK_POSTINSTALL"), "缺 POSTINSTALL hook");
    // 移除放在 POSTUNINSTALL：Tauri 的 PREUNINSTALL 在「應用程式執行中」的檢查之前就跑，
    // 使用者在那一步取消的話，程式還裝著、選單卻已經被刪掉了。
    assert!(h.contains("!macro NSIS_HOOK_POSTUNINSTALL"), "缺 POSTUNINSTALL hook");
    if h.contains("!macro NSIS_HOOK_PREUNINSTALL") {
        assert!(
            !hook_macro_body(&h, "NSIS_HOOK_PREUNINSTALL").contains("DeleteRegKey"),
            "PREUNINSTALL 不可移除選單（使用者可能在後面的「程式執行中」對話框取消）"
        );
    }
    // 註冊與移除各自要落在對的 macro 裡：只用整份 contains 的話，把 DeleteRegKey
    // 搬進 POSTINSTALL（解除安裝時什麼都不刪）也會綠。
    let (post, un) = (hook_macro_body(&h, "NSIS_HOOK_POSTINSTALL"), hook_macro_body(&h, "NSIS_HOOK_POSTUNINSTALL"));
    for (key, arg) in [("Directory", "%1"), ("Directory\\Background", "%V"), ("Drive", "%1")] {
        assert!(
            post.contains(&format!("!insertmacro AITERM_ADD_VERB \"{key}\" \"{arg}\"")),
            "安裝時沒有為 {key} 註冊（參數 {arg}）"
        );
        assert!(
            un.contains(&format!("DeleteRegKey SHCTX \"Software\\Classes\\{key}\\shell\\AITerm\"")),
            "解除安裝時沒有移除 {key}"
        );
    }
}

/// `NoWorkingDirectory`：沒有它，檔案總管會把被點的資料夾當成新行程的 cwd，而第一個
/// 實例會一直握著那個 cwd 到結束——AITerm 開著的時候那個資料夾就刪不掉、改不了名
/// （Microsoft 自己的 cmd verb 也設這個值）。我們傳的參數都是絕對路徑，不依賴 cwd。
/// `MultiSelectModel=Single`：預設模型會對每個被選取的資料夾各啟動一個行程（一次
/// 最多十幾個），在 AITerm 還沒執行時會跟單一實例外掛搶；Single 讓多選時不顯示這一項。
#[test]
fn hook_verbs_do_not_pin_the_cwd_and_are_hidden_on_multi_select() {
    let h = hooks_code();
    let verb = hook_macro_body(&h, "AITERM_ADD_VERB");
    for line in [
        r#"WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "NoWorkingDirectory" """#,
        r#"WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "MultiSelectModel" "Single""#,
    ] {
        assert!(verb.contains(line), "AITERM_ADD_VERB 缺少這一行:\n{line}");
    }
}

#[test]
fn hooks_follow_the_install_mode_and_never_hardcode_the_binary_name() {
    let h = hooks_code();
    assert!(h.contains("SHCTX"), "要用 SHCTX 才會跟著 currentUser／perMachine 安裝模式");
    assert!(!h.contains("HKCU") && !h.contains("HKLM"), "不可硬寫登錄區");
    assert!(h.contains("${MAINBINARYNAME}.exe"), "執行檔名稱要用 Tauri 的 MAINBINARYNAME");
    let lower = h.to_lowercase();
    assert!(!lower.contains("app.exe") && !lower.contains("aiterm.exe"), "不可硬寫執行檔名稱");
    // 執行檔與參數都要加引號（安裝路徑、資料夾路徑都可能含空格）
    assert!(
        h.contains(r##"$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"${ARG}$\""##),
        "command 必須把執行檔與參數都加引號"
    );
}

/// 用真的 makensis 編譯 hook（包在最小的 wrapper 裡）。沒有 makensis 就略過。
/// 負向對照：故意寫錯一條指令，makensis 會以非 0 結束，所以這個測試真的抓得到語法錯誤。
///
/// 只看退出碼不夠：`${MAINBINARYNAME}` 沒定義時 makensis 只會發 warning 6000 而照常
/// 產出安裝程式，所以 6000 也算失敗（不能用 `-WX`：wrapper 引入的 MUI2 會發無害的
/// 6001）。再用 `SetCompress off` 讓字串表原樣留在 exe 裡，掃 UTF-16LE 確認三個登錄
/// 位置、兩個標籤、兩個值與執行檔名真的都編進去了——這樣它檢查的是語意，不只是語法。
#[cfg(unix)]
#[test]
fn nsis_hooks_compile_with_makensis_when_it_is_installed() {
    let _lock = process_lock();
    if std::process::Command::new("makensis").arg("-VERSION").output().is_err() {
        eprintln!("makensis 不在 PATH，略過 hook 編譯檢查");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let wrapper = format!(
        r#"Unicode true
SetCompress off
!include MUI2.nsh
!include FileFunc.nsh
!include x64.nsh
!include WordFunc.nsh
!include "{hooks}"
!define MAINBINARYNAME "app"
Name "hooks-check"
OutFile "hooks-check.exe"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\AITerm"
Section
  !insertmacro NSIS_HOOK_POSTINSTALL
  WriteUninstaller "$INSTDIR\u.exe"
SectionEnd
Section Uninstall
  !insertmacro NSIS_HOOK_POSTUNINSTALL
SectionEnd
"#,
        hooks = root().join("installer/hooks.nsh").display()
    );
    fs::write(dir.path().join("wrapper.nsi"), wrapper).unwrap();
    let out = std::process::Command::new("makensis")
        .args(["-V2", "-NOCD", "wrapper.nsi"])
        .current_dir(dir.path())
        .output()
        .expect("makensis 無法執行");
    let log = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "hook 無法用 makensis 編譯:\n{log}");
    assert!(!log.contains("6000:"), "makensis 發出 warning 6000（多半是有未定義的變數或常數）:\n{log}");

    let exe = fs::read(dir.path().join("hooks-check.exe")).expect("沒有產出安裝程式");
    let utf16 = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect() };
    for needle in [
        r"Software\Classes\Directory\shell\AITerm",
        r"Software\Classes\Directory\Background\shell\AITerm",
        r"Software\Classes\Drive\shell\AITerm",
        "Open in AITerm",
        "在 AITerm 開啟",
        "NoWorkingDirectory",
        "MultiSelectModel",
        "app.exe",
    ] {
        let n = utf16(needle);
        assert!(exe.windows(n.len()).any(|w| w == n.as_slice()), "編出來的安裝程式裡找不到 {needle:?}");
    }
}

/// hook 是被插進 Tauri 自己的 installer.nsi 裡執行的，不能動共用暫存器
/// `$0`–`$9`、`$R0`–`$R9`（Tauri 之後怎麼用它們我們管不著，今天沒事只是碰巧）。
/// 要暫存就宣告自己的 Var，System::Call 的回傳值走堆疊（`.s`）再 Pop 進 Var。
/// 這裡逐字元掃：`$` 後面直接接數字或 `R`＋數字才算；`$\"`、`$INSTDIR`、
/// `${MAINBINARYNAME}`、`$AITerm…` 都不會被誤抓。
#[test]
fn hooks_never_touch_the_shared_nsis_registers() {
    let code = hooks_code();
    let mut hits: Vec<String> = Vec::new();
    for line in code.lines() {
        let b = line.as_bytes();
        for (i, &c) in b.iter().enumerate() {
            let next = b.get(i + 1).copied().unwrap_or(b' ');
            let after = b.get(i + 2).copied().unwrap_or(b' ');
            let dollar_reg = c == b'$' && (next.is_ascii_digit() || (next == b'R' && after.is_ascii_digit()));
            // System::Call 的輸出／輸入規格：`.r0`、`.R0`、`r0`（輸入）
            let call_reg = line.contains("System::Call")
                && (c == b'.' || c == b' ')
                && (next == b'r' || next == b'R')
                && after.is_ascii_digit();
            if dollar_reg || call_reg {
                hits.push(line.trim().to_string());
                break;
            }
        }
    }
    assert!(hits.is_empty(), "hook 動用了共用暫存器（改用自己的 Var 與堆疊）:\n{}", hits.join("\n"));
}
