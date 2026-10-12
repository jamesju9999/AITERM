# 電腦操作：Windows 後端

日期：2026-10-13　前置：`computer-use-design`、`computer-use-performance-design`
範圍：先 Windows；Linux（X11）是下一份 spec。Wayland 不支援，明確標示。

## 驗證限制（先講清楚）

開發機是 Mac，無法執行 Windows 程式。能做的驗證：CI 的 Windows 編譯與單元測試、在 Mac 上對 `x86_64-pc-windows-msvc` 做型別檢查（用只含後端模組的小 crate，避開需要 MSVC 的 C 依賴），以及純邏輯（按鍵對照、座標換算、控制項角色對照…）放在與平台無關的模組裡測試。**真實操作行為需要在 Windows 機器（Parallels）上驗收**，在那之前文件與 CHANGELOG 不得宣稱「已支援」。

## 對應表（`Backend` trait）

| 能力 | Windows 做法 |
|---|---|
| 列 App | `EnumWindows` 收集可見、非工具視窗、有標題的頂層視窗，依 pid 分組；名稱＝行程可執行檔名（`notepad`、`chrome`），`bundle_id`＝小寫完整檔名（`notepad.exe`），視窗標題當別名；排除 AITerm 自己 |
| 元件樹／元件資訊／座標找元件 | UI Automation（`IUIAutomation`，Control view walker）；id 仍是子節點索引路徑；ControlType 對照成與 macOS 相同的 `AX…` 角色名，提示詞與工具說明不必分平台 |
| 點擊元件 | UIA `InvokePattern`，退而求其次 `TogglePattern`／`SelectionItemPattern`，再退回座標點擊 |
| 滑鼠／鍵盤／捲動 | `SendInput`（滑鼠絕對座標對應整個虛擬桌面；文字用 `KEYEVENTF_UNICODE`，不受輸入法與鍵盤配置影響；滾輪 120/格） |
| 按鍵組合 | 虛擬鍵碼對照表；**`cmd` 在 Windows 對應 Ctrl**（模型習慣寫 `cmd+s`），`ctrl`、`alt`、`shift` 照舊 |
| 切到前景 | 還原最小化、`SetForegroundWindow`（用 `AttachThreadInput` 繞過前景鎖），輪詢 `GetForegroundWindow` 確認；確認不到明確報錯 |
| 截圖 | GDI `BitBlt`（視窗範圍用 `DwmGetWindowAttribute(EXTENDED_FRAME_BOUNDS)`），以 `png` 編碼 |
| 文字辨識（OCR） | `Windows.Media.Ocr`（系統內建，語言用使用者設定檔的語言）；沒有對應語言包時回傳可操作的錯誤 |
| 圖示偵測 | 共用 `icons.rs`；縮放比用 `GetDpiForWindow / 96` |
| 開啟 App／網頁 | `ShellExecuteW`（不經 `cmd`，沒有命令列注入）；網址帶 `app` 時以該 App 為執行檔、網址為參數 |
| 焦點欄位內容 | UIA `GetFocusedElement` + `ValuePattern`；`IsPassword` 一律不回傳內容 |
| 權限 | 無需授權（`accessibility`／`screen_recording` 皆視為已授權）；對系統管理員權限視窗操作會被 UIPI 擋下，錯誤訊息要說明 |

座標一律是實體像素（Tauri 程式為 per-monitor DPI aware），截圖、UIA、`SendInput` 同一個座標系，不需要換算。

## 安全（與 macOS 相同的關卡，不重寫）

總開關、每個 App 首次授權、破壞性確認、AITerm 自保、稽核都在 `ComputerService`，後端只是換掉。Windows 額外：
- `open_app`／`open_url` 的 App 名稱拒絕路徑分隔字元與引號（所有平台一致）：授權對話框顯示的是名稱，不能讓名稱其實是任意路徑。
- `shift+delete`（永久刪除）列入危險按鍵，與 `cmd+delete` 同級逐次確認（所有平台）。

## 非支援平台

`builtin_tool_infos` 在不支援的平台回傳空清單（模型看不到用不了的工具）；設定頁的「電腦操作」區塊標明目前支援的平台。Linux 在 X11 後端完成前維持「尚未支援」。

## 結構

- `winmap.rs`（無平台條件編譯）：UIA ControlType→角色、按鍵→虛擬鍵碼、修飾鍵、絕對座標正規化、滾輪換算、UTF-16 拆分、可執行檔名→App 名稱。單元測試在三個平台的 CI 都會跑。
- `windows.rs`（`#[cfg(windows)]`）：呼叫 Win32／UIA／WinRT，薄薄一層，邏輯盡量交給 `winmap.rs`。
