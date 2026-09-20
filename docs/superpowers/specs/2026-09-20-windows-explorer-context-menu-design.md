# Windows 檔案總管右鍵「在 AITerm 開啟」

日期：2026-09-20
承接：`2026-09-19-open-as-terminal-mac-linux-design.md`（第一份 spec 已把解析層、佇列、單一實例做好，
並留了「給第二份 spec（Windows）的接口」）。本份只做 Windows 兩件事中的 **A**；**B（設成 Windows 的預設終端機，
`ITerminalHandoff` COM 委派）不在本份範圍**，另開一輪並先做技術評估。

## 目標

- 在檔案總管對**資料夾**、**資料夾空白處**、**磁碟機**按右鍵，出現「在 AITerm 開啟」，點下去在 AITerm 開一個 cwd 為該處的新分頁。
- AITerm 已在執行時，在既有視窗開新分頁並把視窗帶到前景（靠 tao 的 `set_focus()`，見 §3；**實機才能確認**）。
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
| 怎麼註冊 | Tauri NSIS 的 `installerHooks`（安裝 `NSIS_HOOK_POSTINSTALL`、移除 `NSIS_HOOK_POSTUNINSTALL`） | 官方支援的擴充點；不需要自訂整份安裝樣板；解除安裝與更新流程都會經過 hook |
| 移除放哪個 hook | `NSIS_HOOK_POSTUNINSTALL`，**不是** `PREUNINSTALL` | Tauri 樣板的 `PREUNINSTALL` 在「應用程式執行中」檢查（`CheckIfAppIsRunning`）之前就跑；使用者在那個對話框取消的話，程式還裝著、選單卻已被刪掉。`POSTUNINSTALL` 在 `Section Uninstall` 的最後、無條件執行，移除因此與「真的解除安裝成功」同進退 |
| 寫哪個登錄區 | `SHCTX` | 隨安裝模式：`currentUser`（目前預設）→ 目前使用者，`perMachine` → 本機，不必硬寫 |
| 註冊哪些位置 | `Directory\shell`（`%1`）、`Directory\Background\shell`（`%V`）、`Drive\shell`（`%1`） | 資料夾、資料夾空白處、磁碟機 |
| 選單文字 | 系統 UI 語言是繁體中文（0x0404／0x0C04／0x1404）顯示「在 AITerm 開啟」，否則「Open in AITerm」 | 安裝程式本身只有英文，用 `GetUserDefaultUILanguage` 判斷（回傳值走 NSIS 堆疊再 Pop 進自己的變數，hook 不動共用暫存器 `$0`–`$9`／`$R0`–`$R9`）；hook 檔存成 **UTF-8 with BOM**：BOM 是 NSIS 唯一明確的編碼宣告，沒有 BOM 的檔案會依 ANSI 字碼頁轉換（除非指定 `/charset`），而檔案裡有中文。POSIX 版 makensis 會把沒有 BOM 的檔案當 UTF-8 讀，所以在 macOS／Linux 上編譯看不出差別，只有靜態測試與 Windows 上的實際建置能把關 |
| 執行檔名稱 | `${MAINBINARYNAME}.exe` | 由 Tauri 決定（此專案實際是 `app.exe`），不硬寫 |
| 指令 | `"$INSTDIR\${MAINBINARYNAME}.exe" "%1"`（背景用 `%V`） | 路徑含空格 |
| 不釘住工作目錄 | 每個 verb 寫入 `NoWorkingDirectory`（空字串） | 沒有它，檔案總管會把被點的資料夾當成新行程的 cwd，而第一個實例會握著那個 cwd 到結束——AITerm 開著時那個資料夾就刪不掉、改不了名（Microsoft 自己的 `cmd` verb 也設這個值）。我們傳的參數都是絕對路徑，不依賴 cwd |
| 多選 | 每個 verb 寫入 `MultiSelectModel` = `Single` | 預設模型會對每個被選取的資料夾各啟動一個行程（一次最多十幾個），在 AITerm 還沒執行時會跟單一實例外掛搶；`Single` 讓多選時不顯示這一項 |

## 設計

### 1. 安裝程式 hook：`src-tauri/installer/hooks.nsh`

`tauri.windows.conf.json` 的 `bundle.windows.nsis.installerHooks` 指向它。POSTINSTALL 依系統語言決定標籤後，
對三個位置各寫入預設值（標籤）、`Icon`、`NoWorkingDirectory`、`MultiSelectModel`、`command`；POSTUNINSTALL 刪除三個位置的 `AITerm` 鍵。
更新流程有兩條路，最終狀態都一致：靜默更新（`/UPDATE`）時，Tauri 樣板會**略過**舊版的解除安裝（`PageLeaveReinstall` 直接短路），
只有 POSTINSTALL 會跑，並覆寫同樣的鍵；互動式的「先解除安裝舊版再安裝」路徑才會先跑舊版的解除安裝 hook（POSTUNINSTALL），再跑新版的 POSTINSTALL。

### 2. 命令列參數：磁碟機根目錄的結尾引號

Windows 的命令列規則下，`"D:\"` 裡的 `\"` 是跳脫的引號，所以檔案總管對磁碟機根目錄（`%1` = `D:\`）傳來的參數
會變成 `D:"`。`parse_args` 的位置參數在 **Windows 上**把結尾的 `"` 還原成 `\`（純函式 `restore_trailing_backslash`，
所有平台都能單元測試；只在 Windows 才套用，因為 Unix 檔名本來就可以以 `"` 結尾）。
單一實例轉發（外掛以 `|` 串接參數）傳的是已經被誤解析的字串，所以在接收端還原一樣有效；`|` 不是合法的 Windows 檔名字元，
串接不會誤拆路徑。

### 3. 前景權限（**不另外處理**，依賴 tao；實機發現只閃爍時才加備案）

單一實例外掛在 Windows 上（已讀原始碼確認）只用 `FindWindowW` ＋ `WM_COPYDATA` 把參數送給第一個實例，
本身沒有處理前景權限。一開始的設計因此打算在 `run()` 最前面呼叫 `AllowSetForegroundWindow(ASFW_ANY)`，
但**讀了 tao 0.34.8 的原始碼後撤銷了**：`Window::set_focus()` 在視窗可見、未最小化、還不是前景時，
本來就會呼叫 `force_window_active()`——先試 `SetForegroundWindow`，失敗再用「模擬 Alt 鍵」的已知手法繞過前景鎖定
（`platform_impl/windows/window.rs`）。第一個實例收到轉發後走的 `raise_main_window`（`unminimize`→`show`→`set_focus`）
因此多半已經能浮到前景。沒有證據顯示需要，卻要付出「每次啟動 AITerm 都讓系統的前景保護放鬆一小段時間」的代價，所以不加。

**備案（只有在 Windows 實機上發現視窗仍只在工作列閃爍時才啟用）：** 第二個行程是使用者從檔案總管點出來的，
擁有前景權限；在 `run()` 最前面（在外掛把它結束之前）呼叫 `AllowSetForegroundWindow(ASFW_ANY)`。已用暫存 crate 對
`x86_64-pc-windows-msvc` 交叉編譯驗證過簽章（`windows-sys` 0.60，feature `Win32_UI_WindowsAndMessaging`，
`use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};`，`unsafe { AllowSetForegroundWindow(ASFW_ANY); }`，
無指標參數）。代價：任何行程在下一次使用者輸入之前都可以搶前景。

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
  只看退出碼不夠：`${MAINBINARYNAME}` 沒定義時 makensis 只發 warning 6000、照常產出安裝程式，所以輸出裡有 `6000:` 也算失敗
  （不能用 `-WX`，wrapper 引入的 MUI2 會發無害的 6001）。wrapper 另加 `SetCompress off`，測試再掃編出來的 exe 裡的 UTF-16LE 字串，
  確認三個登錄位置、兩個標籤、`NoWorkingDirectory`、`MultiSelectModel`、`app.exe` 真的都編進去了——檢查的是語意，不只是語法。
  沒有 `makensis` 時該測試略過，不影響 CI。

**做不到、需要 Windows 實機或 CI 的：**
- 安裝後右鍵選單真的出現、點下去真的開分頁、視窗真的浮到前景、解除安裝後選單消失、標籤語言。
- 已在執行的視窗是否真的浮到前景，而不是只在工作列閃爍（依賴 tao 的 `set_focus()`；依 Windows 版本與焦點狀態而定）。
這些要等推上 GitHub 讓 CI 出 Windows 安裝檔、在 Windows 上安裝測試；本份完成時**不會宣稱 Windows 已實測**。
