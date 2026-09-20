# Windows 檔案總管右鍵「在 AITerm 開啟」

日期：2026-09-20
承接：`2026-09-19-open-as-terminal-mac-linux-design.md`（第一份 spec 已把解析層、佇列、單一實例做好，
並留了「給第二份 spec（Windows）的接口」）。本份只做 Windows 兩件事中的 **A**；**B（設成 Windows 的預設終端機，
`ITerminalHandoff` COM 委派）不在本份範圍**，另開一輪並先做技術評估。

## 目標

- 在檔案總管對**資料夾**、**資料夾空白處**、**磁碟機**按右鍵，出現「在 AITerm 開啟」，點下去在 AITerm 開一個 cwd 為該處的新分頁。
- AITerm 已在執行時，在既有視窗開新分頁**並把視窗浮到前景**（不是只在工作列閃一下）。
- 解除安裝時移除；更新（`/UPDATE`）時重新寫入，不留下失效的選單項目。

## 非目標

- Windows 11 的**新式**右鍵選單（頂層那份）：那需要封裝過的 COM `IExplorerCommand`，成本與風險都遠大於本功能。
  傳統的 shell verb 在 Windows 11 要按「顯示其他選項」（Shift+F10）才看得到——記為已知限制。
- 預設終端機（B）。
- 讓 Windows 上的 `-e` 指令／腳本自動執行（第一份 spec 已決定 Windows 一律不自動執行，本份不改）。
- 安裝時的「要不要加入右鍵選單」選項、設定頁開關：不做（YAGNI；想移除就解除安裝）。

## 已定案的決策（依判斷，未逐項詢問使用者）

| 問題 | 決定 | 理由 |
|------|------|------|
| 怎麼註冊 | Tauri NSIS 的 `installerHooks`（`NSIS_HOOK_POSTINSTALL`／`NSIS_HOOK_PREUNINSTALL`） | 官方支援的擴充點；不需要自訂整份安裝樣板；解除安裝與更新流程都會經過 hook |
| 寫哪個登錄區 | `SHCTX` | 隨安裝模式：`currentUser`（目前預設）→ 目前使用者，`perMachine` → 本機，不必硬寫 |
| 註冊哪些位置 | `Directory\shell`（`%1`）、`Directory\Background\shell`（`%V`）、`Drive\shell`（`%1`） | 資料夾、資料夾空白處、磁碟機 |
| 選單文字 | 系統 UI 語言是繁體中文（0x0404／0x0C04／0x1404）顯示「在 AITerm 開啟」，否則「Open in AITerm」 | 安裝程式本身只有英文，用 `GetUserDefaultUILanguage` 判斷；hook 檔存成 **UTF-8 with BOM**（NSIS Unicode 才讀得對中文） |
| 執行檔名稱 | `${MAINBINARYNAME}.exe` | 由 Tauri 決定（此專案實際是 `app.exe`），不硬寫 |
| 指令 | `"$INSTDIR\${MAINBINARYNAME}.exe" "%1"`（背景用 `%V`） | 路徑含空格 |

## 設計

### 1. 安裝程式 hook：`src-tauri/installer/hooks.nsh`

`tauri.windows.conf.json` 的 `bundle.windows.nsis.installerHooks` 指向它。POSTINSTALL 依系統語言決定標籤後，
對三個位置各寫入預設值（標籤）、`Icon`、`command`；PREUNINSTALL 刪除三個位置的 `AITerm` 鍵。
更新流程（舊版解除安裝 `/UPDATE` → 新版安裝）會先刪再寫，最終狀態一致。

### 2. 命令列參數：磁碟機根目錄的結尾引號

Windows 的命令列規則下，`"D:\"` 裡的 `\"` 是跳脫的引號，所以檔案總管對磁碟機根目錄（`%1` = `D:\`）傳來的參數
會變成 `D:"`。`parse_args` 的位置參數在 **Windows 上**把結尾的 `"` 還原成 `\`（純函式 `restore_trailing_backslash`，
所有平台都能單元測試；只在 Windows 才套用，因為 Unix 檔名本來就可以以 `"` 結尾）。
單一實例轉發（外掛以 `|` 串接參數）傳的是已經被誤解析的字串，所以在接收端還原一樣有效；`|` 不是合法的 Windows 檔名字元，
串接不會誤拆路徑。

### 3. 前景權限

單一實例外掛在 Windows 上（已讀原始碼確認）只用 `FindWindowW` ＋ `WM_COPYDATA` 把參數送給第一個實例，
**完全沒有處理前景權限**。前景鎖定規則下，第一個實例的 `set_focus()` 只會讓工作列閃爍。
第二個行程是使用者從檔案總管點出來的，擁有前景權限，所以在 `run()` 最前面（在外掛把它結束之前）呼叫
`AllowSetForegroundWindow(ASFW_ANY)`，讓第一個實例可以把視窗拉到前景。這個呼叫也讓 v1.30.0 已有的
「再開一次 AITerm 就把視窗帶到前面」在 Windows 上真的生效。代價：任何行程在下一次使用者輸入之前都可以搶前景，
視窗極短，且只在 AITerm 自己被使用者啟動的瞬間。

## 已知限制

- Windows 11 要按「顯示其他選項」才看得到（見非目標）。
- 安裝程式的語言選擇器沒有開，標籤語言看的是**作業系統 UI 語言**，不是 AITerm 設定的語言。
- 選單項目的圖示用 `app.exe` 的圖示。
- 以系統管理員身分執行的第一個實例，會擋掉一般權限第二個行程的轉發（UIPI），見第一份 spec。

## 測試與驗收（誠實）

**能在 macOS 上自動驗證：**
- `parse_args` 的結尾引號還原：純函式測試（全平台）＋ `cfg(windows)` 的整合測試（Windows CI 會跑）＋ Unix 上「檔名結尾的 `"` 不被動」的對照測試。
- `hooks.nsh` 的靜態內容測試（三個位置、`SHCTX`、`${MAINBINARYNAME}`、引號、UTF-8 BOM、`tauri.windows.conf.json` 指向）。
- **用真正的 `makensis` 編譯** hook（包在最小的 wrapper 裡）——原型已證明它能抓到寫錯的指令（負向對照：故意寫錯 → 退出碼 1）。
  沒有 `makensis` 時該測試略過，不影響 CI。
- `AllowSetForegroundWindow` 的 FFI 以 `--target x86_64-pc-windows-msvc` 交叉編譯檢查簽章。

**做不到、需要 Windows 實機或 CI 的：**
- 安裝後右鍵選單真的出現、點下去真的開分頁、視窗真的浮到前景、解除安裝後選單消失、標籤語言。
- `AllowSetForegroundWindow` 在實機上是否足以讓第一個實例拿到前景（依 Windows 版本與焦點狀態而定）。
這些要等推上 GitHub 讓 CI 出 Windows 安裝檔、在 Windows 上安裝測試；本份完成時**不會宣稱 Windows 已實測**。
