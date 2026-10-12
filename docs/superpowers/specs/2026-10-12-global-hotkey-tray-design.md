# 全域快捷鍵＋托盤常駐＋開機自啟 設計

日期：2026-10-12

## 目標

讓 AITerm 成為「電腦操作的入口」：任何時候按一組全域快捷鍵就能喚出主視窗並直接打字；關閉視窗不終止背景工作；可選擇開機自動啟動。

## 範圍

做：全域快捷鍵、托盤／選單列圖示、關閉視窗縮到托盤（可設定）、開機自啟（可設定）、設定頁。

不做：獨立浮動輸入條、Quake 下拉終端機、macOS 純選單列（隱藏 Dock）模式、GUI 控制與截圖（另一份 spec）。

## 做法

官方外掛：`tauri-plugin-global-shortcut`、`tauri-plugin-autostart`，加上 tauri 內建 `tray-icon` feature。不自行呼叫各平台 hotkey API。

## 行為

1. **快捷鍵**（預設 `Ctrl+\``，可改）切換主視窗：
   - 視窗隱藏、最小化或未取得焦點 → 顯示、還原、置前、聚焦，並通知前端聚焦輸入框。
   - 視窗已聚焦 → 隱藏。
2. **托盤圖示**（macOS 為選單列）：左鍵切換視窗（同快捷鍵邏輯）；選單項目「顯示 AITerm」「新分頁」「設定」「結束」。
3. **關閉視窗（✕／Alt+F4）**：`keep_in_tray` 為真時只隱藏視窗，PTY 與 Agent 繼續跑，不做忙碌確認；為假時維持原本的 `useWindowCloseGuard` 流程。
4. **結束**（托盤選單、Cmd+Q）：先顯示視窗，再走既有的 `app://quit-requested` → 忙碌確認 → `destroy()`。不新增第二條退出路徑。
5. **開機自啟**：`autostart` 為真時註冊系統自啟項；自啟啟動帶 `--minimized` 旗標，此時不顯示視窗。

## 設定欄位（AppConfig）

| 欄位 | 型別 | 預設 |
|------|------|------|
| `global_hotkey` | `String`（空字串＝停用） | `"Ctrl+`"` |
| `keep_in_tray` | `bool` | `true` |
| `autostart` | `bool` | `false` |

舊設定檔缺欄位時用預設值（`#[serde(default)]`）。

## 元件

- `src-tauri/src/resident/mod.rs`：`toggle_main_window`、`show_main_window`、`apply_hotkey`、`apply_autostart`、托盤建立。純邏輯（快捷鍵字串驗證、視窗切換決策）抽成可單元測試的函式。
- `commands/config.rs`：`set_global_hotkey`（註冊失敗回傳錯誤並保留舊值）、`set_keep_in_tray`、`set_autostart`。
- 前端 `ipc/config.ts` 對應包裝；`GeneralPage.tsx` 新增「常駐與快捷鍵」區塊；`i18n.ts` 加 en／zh-TW 字串。
- `useWindowCloseGuard`：原生關閉時依 `keep_in_tray` 分流（hide 或 attempt）。
- 事件 `app://focus-input`：Rust 在喚出視窗後送出，`TerminalApp` 收到後聚焦目前分頁輸入框。

## 邊界與風險

- 快捷鍵被占用或字串無效：`set_global_hotkey` 回傳錯誤，設定頁顯示，設定檔不變；啟動時註冊失敗只記 log，不擋啟動。
- Linux Wayland 不支援全域快捷鍵：註冊失敗走同一條錯誤路徑，設定頁顯示訊息；托盤與自啟照常。
- 隱藏視窗用 `window.hide()`（webview 與 xterm 繼續存活，尺寸不變），不卸載內容。
- single-instance（僅正式版）第二次啟動已會 `raise_main_window`，對隱藏視窗同樣有效。
- 自啟旗標 `--minimized` 需被 `launch::parse_args` 忽略，不被當成路徑。
- `keep_in_tray` 為真但托盤建立失敗時，不得隱藏視窗（否則使用者無法找回）：關閉退回原流程。

## 測試

- Rust 單元：設定預設值與舊檔相容、TOML 往返、快捷鍵字串驗證、視窗切換決策表、`--minimized` 解析。
- 前端：`useWindowCloseGuard` 兩種分流、設定頁欄位與錯誤顯示。
- 真機（macOS）：快捷鍵喚出／隱藏、托盤選單、關閉縮到托盤後 PTY 仍在跑、結束時忙碌確認、自啟旗標。Windows／Linux 僅編譯與單元測試。
