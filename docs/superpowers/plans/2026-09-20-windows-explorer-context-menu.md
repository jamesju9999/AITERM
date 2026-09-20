# Windows 檔案總管右鍵「在 AITerm 開啟」Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在檔案總管對資料夾／資料夾空白處／磁碟機按右鍵可「在 AITerm 開啟」，AITerm 已在執行時把視窗浮到前景。

**Architecture:** Tauri NSIS `installerHooks` 在安裝時寫入三個位置的 shell verb（`SHCTX`），解除安裝時刪除；`parse_args` 在 Windows 上還原磁碟機根目錄被命令列規則吃掉的結尾反斜線；`run()` 最前面開放前景權限，讓已在執行的第一個實例能把視窗拉到前景。

**Tech Stack:** NSIS（`makensis` 驗證語法）、Rust（`windows-sys`）、Tauri 2 bundle 設定。

**Spec:** `docs/superpowers/specs/2026-09-20-windows-explorer-context-menu-design.md`

**全域注意事項：**
- 這台是 macOS，Windows 專屬程式碼只能靠「純函式測試＋交叉編譯＋makensis 編譯」驗證；**不可宣稱 Windows 已實測**。
- Rust 指令在 `src-tauri/` 下跑；`cd src-tauri && cargo test --lib launch` 迭代，最後跑 `cargo test --workspace --no-fail-fast` 並逐行看 `test result:`（本機基線：51 行、1687 通過、0 失敗）。
- `makensis` 不在 PATH：已解壓到暫存目錄。使用前 `export PATH=<scratchpad>/nsis/makensis/3.12/bin:$PATH NSISDIR=<scratchpad>/nsis/makensis/3.12/share/nsis`（scratchpad = `/private/tmp/claude-501/-Users-jamesju-Documents-GitHub-AITERM/e2b32877-0c7a-4e30-8390-bd5d39806d15/scratchpad`）。
- `git add` 只加明確路徑；`docs/superpowers/` 被 gitignore，要 `git add -f`；commit 結尾加 `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>`。
- 每個新測試都要先看到它紅（或用突變證明會紅），fixture 必須能區分正確與錯誤行為。
- worktree 內 `node_modules`、`src-tauri/target`、`src-tauri/binaries` 是 symlink，不要動。

## File Structure

| 檔案 | 動作 | 職責 |
|------|------|------|
| `src-tauri/src/launch/parse.rs` | 修改 | `restore_trailing_backslash`＋在 Windows 套用於位置參數 |
| `src-tauri/installer/hooks.nsh` | 建立（UTF-8 with BOM） | NSIS hook：寫入／刪除三個右鍵選單項目 |
| `src-tauri/tauri.windows.conf.json` | 修改 | `installerHooks` 指向 hook |
| `src-tauri/tests/os_registration.rs` | 修改 | hook 靜態內容測試、conf 測試、makensis 編譯測試 |
| `src-tauri/Cargo.toml` | 修改 | `windows-sys` 加 `Win32_UI_WindowsAndMessaging` |
| `src-tauri/src/launch/mod.rs` | 修改 | `allow_foreground_takeover()` |
| `src-tauri/src/lib.rs` | 修改 | `run()` 第一行呼叫它 |

---

### Task 1: 位置參數的結尾引號還原（Windows）

**Files:** Modify `src-tauri/src/launch/parse.rs`

背景：Windows 命令列規則下，`"D:\"` 的 `\"` 是跳脫的引號，檔案總管對磁碟機根目錄傳的 `%1` = `D:\` 會變成參數 `D:"`。

- [ ] **Step 1: 寫失敗的測試**（加在 `parse.rs` 的 `mod tests`；先讀該檔確認 `p()`、`argv()` 輔助函式與 `use` 的寫法，沿用它們）

```rust
    #[test]
    fn a_trailing_quote_is_restored_to_the_backslash_it_ate() {
        // 檔案總管傳 "D:\" → 命令列解析後變成 D:"
        assert_eq!(restore_trailing_backslash("D:\""), "D:\\");
        assert_eq!(restore_trailing_backslash("C:\\Users\\me\\x\""), "C:\\Users\\me\\x\\");
    }

    #[test]
    fn arguments_without_a_trailing_quote_are_left_alone() {
        assert_eq!(restore_trailing_backslash("C:\\Users\\me"), "C:\\Users\\me");
        assert_eq!(restore_trailing_backslash("a\"b"), "a\"b"); // 只處理結尾
        assert_eq!(restore_trailing_backslash(""), "");
    }

    /// 只有 Windows 才套用：Unix 的檔名本來就可以以 `"` 結尾，動了就找不到目錄。
    #[cfg(unix)]
    #[test]
    fn on_unix_a_directory_whose_name_ends_with_a_quote_is_not_touched() {
        let base = tempfile::tempdir().unwrap();
        let weird = base.path().join("weird\"");
        fs::create_dir(&weird).unwrap();
        let got = parse_args(&argv(&[&p(&weird)]), None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cwd, Some(p(&weird)));
    }

    /// Windows CI 會跑：把「被誤解析成結尾引號」的資料夾參數還原後找得到目錄。
    #[cfg(windows)]
    #[test]
    fn on_windows_a_mangled_drive_style_argument_still_finds_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mangled = format!("{}\"", p(dir.path())); // 模擬 "<dir>\" 被吃掉反斜線＋引號
        let got = parse_args(&argv(&[&mangled]), None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cwd, Some(p(dir.path())));
    }
```

- [ ] **Step 2: 確認紅**：`cd src-tauri && cargo test --lib launch::parse`。預期：編譯失敗（`restore_trailing_backslash` 不存在）。

- [ ] **Step 3: 實作**（在 `parse.rs` 的 `positional()` 前面加純函式，並在 `positional()` **最開頭**、`file://` 處理之前套用於 `raw`；`positional` 的簽章與其餘邏輯不變）

```rust
use std::borrow::Cow;

/// Windows 的命令列規則：`"D:\"` 裡的 `\"` 是跳脫的引號，所以檔案總管對磁碟機根目錄
/// （`%1` = `D:\`）傳來的參數會變成 `D:"`。把結尾的 `"` 還原成它吃掉的 `\`。
/// 純函式，所有平台都能單元測試；實際只在 Windows 套用（見 `positional`）。
fn restore_trailing_backslash(raw: &str) -> Cow<'_, str> {
    match raw.strip_suffix('"') {
        Some(head) => Cow::Owned(format!("{head}\\")),
        None => Cow::Borrowed(raw),
    }
}
```

在 `positional()` 開頭加：

```rust
    // 只有 Windows 的命令列規則會把 `\"` 吃成引號；Unix 的檔名可以合法地以 `"` 結尾。
    let raw = if cfg!(windows) { restore_trailing_backslash(raw) } else { Cow::Borrowed(raw) };
    let raw = raw.as_ref();
```

（若 `Cow` 已在該檔 `use` 過就不要重複；若 `positional` 之後的程式碼用的是別的變數名，維持原邏輯、只把入口的 `raw` 換成處理過的那份。）

- [ ] **Step 4: 確認綠**：`cargo test --lib launch::parse`。預期全綠（含既有的 `file://` 與其他測試）。

- [ ] **Step 5: 突變檢查**：(a) 讓 `restore_trailing_backslash` 永遠回傳原字串 → 前兩個測試紅；(b) 把 `cfg!(windows)` 改成 `true` → Unix 的對照測試 `on_unix_a_directory_whose_name_ends_with_a_quote_is_not_touched` 紅。各自還原（用 `cmp` 對照備份）。

- [ ] **Step 6: 交叉編譯檢查 Windows 那條測試路徑**：`cd src-tauri && cargo check --tests --lib --target x86_64-pc-windows-msvc -p app 2>&1 | tail`。若因 C 相依（ring、native-tls 等）在 macOS 上無法交叉編譯，改用暫存 crate 只檢查 `restore_trailing_backslash` 與 `cfg(windows)` 測試的語法，並如實回報做不到的部分。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/launch/parse.rs
git commit -m "$(cat <<'EOF'
fix(launch): restore the trailing backslash Windows eats from a quoted drive root

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: NSIS hook、設定與測試

**Files:** Create `src-tauri/installer/hooks.nsh`；Modify `src-tauri/tauri.windows.conf.json`、`src-tauri/tests/os_registration.rs`

- [ ] **Step 1: 寫失敗的測試**（追加到 `os_registration.rs`；先讀該檔，沿用它的 `root()`、`read()`，以及 unix 專用的 `process_lock()`／`PROCESS_LOCK` 輔助——凡是會 spawn 行程的測試都要拿鎖）

```rust
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
        "hooks.nsh 必須是 UTF-8 with BOM，否則 NSIS（Unicode）會把「在 AITerm 開啟」讀成亂碼"
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

#[test]
fn hooks_register_folder_background_and_drive_verbs_and_remove_them_on_uninstall() {
    let h = hooks_code();
    assert!(h.contains("!macro NSIS_HOOK_POSTINSTALL"), "缺 POSTINSTALL hook");
    assert!(h.contains("!macro NSIS_HOOK_PREUNINSTALL"), "缺 PREUNINSTALL hook");
    for (key, arg) in [("Directory", "%1"), ("Directory\\Background", "%V"), ("Drive", "%1")] {
        assert!(
            h.contains(&format!("!insertmacro AITERM_ADD_VERB \"{key}\" \"{arg}\"")),
            "安裝時沒有為 {key} 註冊（參數 {arg}）"
        );
        assert!(
            h.contains(&format!("DeleteRegKey SHCTX \"Software\\Classes\\{key}\\shell\\AITerm\"")),
            "解除安裝時沒有移除 {key}"
        );
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
  !insertmacro NSIS_HOOK_PREUNINSTALL
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
    assert!(
        out.status.success(),
        "hook 無法用 makensis 編譯:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.path().join("hooks-check.exe").exists(), "沒有產出安裝程式");
}
```

- [ ] **Step 2: 確認紅**：`cd src-tauri && cargo test --test os_registration`（第一次會編譯整個 app 的測試目標，要幾分鐘）。預期：新增的測試因找不到 `installer/hooks.nsh` 而失敗；makensis 那條若沒有 PATH 會略過（顯示綠），這是預期。

- [ ] **Step 3: 建立 `src-tauri/installer/hooks.nsh`**（**檔案開頭必須是 UTF-8 BOM `EF BB BF`**；用下列內容，逐字。建立後用 `xxd src-tauri/installer/hooks.nsh | head -1` 確認以 `efbb bf` 開頭）

```nsis
; AITerm 的 NSIS 安裝程式 hook（Tauri `bundle.windows.nsis.installerHooks`）：
; 在檔案總管的右鍵選單加入「在 AITerm 開啟」（資料夾、資料夾空白處、磁碟機）。
; 註冊在 SHCTX：currentUser 安裝寫進目前使用者，perMachine 安裝寫進本機；解除安裝時移除。
; 更新（/UPDATE）會先跑舊版的解除安裝 hook 再跑新版的安裝 hook，最終狀態一致。
; Windows 11 的新式右鍵選單要按「顯示其他選項」才看得到（傳統 shell verb 的限制）。
; 這個檔案必須存成 UTF-8 with BOM，否則 NSIS（Unicode）讀不對下面的中文。

Var AITermMenuLabel

; KEY = Software\Classes 底下的位置；ARG = 交給 AITerm 的路徑（資料夾用 %1，資料夾空白處用 %V）。
!macro AITERM_ADD_VERB KEY ARG
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "" "$AITermMenuLabel"
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm" "Icon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  WriteRegStr SHCTX "Software\Classes\${KEY}\shell\AITerm\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"${ARG}$\""
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; 安裝程式本身只有英文，所以用作業系統的 UI 語言決定選單文字：繁體中文（台灣／香港／澳門）用中文。
  StrCpy $AITermMenuLabel "Open in AITerm"
  System::Call 'kernel32::GetUserDefaultUILanguage() i .r0'
  ${If} $0 = 0x0404
  ${OrIf} $0 = 0x0C04
  ${OrIf} $0 = 0x1404
    StrCpy $AITermMenuLabel "在 AITerm 開啟"
  ${EndIf}
  !insertmacro AITERM_ADD_VERB "Directory" "%1"
  !insertmacro AITERM_ADD_VERB "Directory\Background" "%V"
  !insertmacro AITERM_ADD_VERB "Drive" "%1"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey SHCTX "Software\Classes\Directory\shell\AITerm"
  DeleteRegKey SHCTX "Software\Classes\Directory\Background\shell\AITerm"
  DeleteRegKey SHCTX "Software\Classes\Drive\shell\AITerm"
!macroend
```

- [ ] **Step 4: 改 `src-tauri/tauri.windows.conf.json`**：在 `bundle.windows.nsis` 內、`installerIcon` 後加一行 `"installerHooks": "installer/hooks.nsh"`（用 Edit 工具只插入這一行，不要整份重寫；保留其餘所有鍵）。

- [ ] **Step 5: 確認綠**：先 `export PATH=…nsis/makensis/3.12/bin:$PATH NSISDIR=…/share/nsis`（見全域注意事項），再 `cd src-tauri && cargo test --test os_registration`。預期：全過，且 makensis 那條**真的有跑**（用 `-- --nocapture` 確認沒有印出「略過」）。

- [ ] **Step 6: 突變檢查（各自還原，`cmp` 對照備份）**：
  (a) 在 hook 的 `DeleteRegKey` 三行中刪掉 `Drive` 那行 → `hooks_register_…` 紅；
  (b) 把 `SHCTX` 全換成 `HKCU` → `hooks_follow_the_install_mode…` 紅；
  (c) 把檔頭 BOM 去掉 → BOM 測試紅；
  (d) 在 hook 裡把 `WriteRegStr` 寫成 `WriteRegStrr` → makensis 編譯測試紅（證明它有咬到）；
  (e) 刪掉 `tauri.windows.conf.json` 的 `installerHooks` → conf 測試紅。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/installer/hooks.nsh src-tauri/tauri.windows.conf.json src-tauri/tests/os_registration.rs
git commit -m "$(cat <<'EOF'
feat(launch): add an "Open in AITerm" Explorer context menu on Windows via NSIS hooks

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 前景權限（Windows）

**Files:** Modify `src-tauri/Cargo.toml`、`src-tauri/src/launch/mod.rs`、`src-tauri/src/lib.rs`

背景：單一實例外掛在 Windows 只用 `FindWindowW`＋`WM_COPYDATA` 轉參數，沒處理前景權限，所以第一個實例的 `set_focus()` 只會讓工作列閃爍。第二個行程（使用者從檔案總管點出來的）有前景權限，在外掛把它結束之前開放給別的行程即可。

- [ ] **Step 1: Cargo.toml**：把 `[target.'cfg(windows)'.dependencies]` 裡 `windows-sys` 的 features 改成 `["Win32_Storage_FileSystem", "Win32_System_WindowsProgramming", "Win32_UI_WindowsAndMessaging"]`。

- [ ] **Step 2: `launch/mod.rs`** 加入（放在 `raise_main_window` 附近）：

```rust
/// Windows：讓「別的行程」（已在執行的第一個實例）能把它的視窗拉到前景。
///
/// 前景鎖定規則下，只有「剛收到使用者輸入」的行程能搶焦點。從檔案總管點出來的第二個行程符合這個條件，
/// 但真正該浮到前面的是第一個實例；single-instance 外掛在 Windows 只用 `WM_COPYDATA` 把參數送過去，
/// 完全沒有處理前景權限，第一個實例的 `set_focus()` 因此只會讓工作列閃爍。所以在外掛把第二個行程結束之前
/// （`run()` 最前面）先開放前景權限。代價：任何行程在下一次使用者輸入之前都可以搶前景，視窗極短。
#[cfg(windows)]
pub fn allow_foreground_takeover() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
    // SAFETY: 純 Win32 呼叫、沒有指標參數；沒有前景權限的行程呼叫只會回傳 FALSE。
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

#[cfg(not(windows))]
pub fn allow_foreground_takeover() {}
```

- [ ] **Step 3: `lib.rs`**：在 `pub fn run()` 的**第一行**加 `launch::allow_foreground_takeover();`（先讀 `run()` 確認它是進入 Tauri 的正式入口，且在任何耗時初始化之前）。`run_headless()` 不加。

- [ ] **Step 4: 編譯檢查**：
  (a) macOS：`cd src-tauri && cargo check`（非 Windows 分支是空函式，不可有新警告）；
  (b) Windows 簽章：`windows-sys` 是純 Rust，用暫存 crate（放在 scratchpad，**不進 repo**）依賴 `windows-sys = { version = "0.60", features = ["Win32_UI_WindowsAndMessaging"] }`，把上面 `allow_foreground_takeover` 的 Windows 版本函式貼進去，`cargo check --target x86_64-pc-windows-msvc`，必須通過。另試 `cargo check --lib --target x86_64-pc-windows-msvc -p app`；若因 C 相依在 macOS 上做不到，如實回報。
  這條路徑**沒有行為測試**（需要 Windows 實機），如實回報。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/launch/mod.rs src-tauri/src/lib.rs
git commit -m "$(cat <<'EOF'
fix(launch): let the running instance take the foreground when Windows starts a second one

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 整體驗證（controller 執行）

- [ ] `cargo test --workspace --no-fail-fast`（逐行看 `test result:`）、`npx tsc -b`、`npm run test`（基線 200 檔／1732 測試，另有既有的 ~80 個 TaskBoard unhandled errors）、eslint 不增加問題。
- [ ] 用 makensis 對最終 hook 再編譯一次，並確認 `git ls-files --eol` 沒有把 `hooks.nsh` 的 BOM 或行尾弄壞。
- [ ] 更新第一份 spec 的「非目標」與「已知限制」：Windows 檔案總管整合已由本份處理。
- [ ] **CHANGELOG 不在這裡寫**：等真的要發版（打 tag）時再寫給使用者看的版本段落；本輪不推送、不打 tag。

## Self-Review

- 覆蓋 spec：hook 三個位置＋解除安裝（Task 2）、標籤語言與 BOM（Task 2 測試）、結尾引號（Task 1）、前景權限（Task 3）、誠實的驗證範圍（各 Task 的回報要求＋Task 4）。
- 沒有 placeholder；型別／名稱一致：`AITERM_ADD_VERB`、`AITermMenuLabel`、`restore_trailing_backslash`、`allow_foreground_takeover`。
- 已知風險：`allow_foreground_takeover` 在實機上是否足夠無法在本機驗證；`cargo check --target x86_64-pc-windows-msvc -p app` 可能因 C 相依做不到。
