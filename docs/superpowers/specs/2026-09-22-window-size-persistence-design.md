# 記住視窗大小/位置/最大化狀態

2026-09-22

## 驗證狀態

在 macOS（這台開發機，`npm run tauri:dev`）用 osascript 實際操作真的視窗
驗證過，Windows／Linux 未測試。

**確認可用：**
- 調整大小＋位置後正常關閉（Cmd+Q，走自訂 Quit 選單項目那條路徑）→ 重開，
  視窗準確還原到關閉前的大小與位置（`1300×850 @ (60,60)`，逐項比對過
  `.window-state.json` 與畫面截圖）。
- 全新安裝（沒有 `.window-state.json`）時，正確退回 `tauri.conf.json` 的
  800×600 預設值。
- 有指令執行中時關閉視窗，跳出「還有工作正在進行」確認對話框；按「取消
  （繼續執行）」後 App 正常繼續跑，視窗大小沒有被提早存檔覆蓋；指令結束後
  真的關閉，存的是取消當下已經調整過的大小，沒有被中間狀態污染。
- 順便在同一輪操作裡驗證到今天稍早做的 Ctrl+C 轉發修正在真機上也正常運作
  （`sleep 300` 跑到一半用 Ctrl+C 中斷成功，`^C` / exit 130）。

**發現一個跟這個功能本身無關的既有限制：**
這台機器上的 dev build，視窗用的是自繪標題列＋macOS Overlay 樣式
（`decorations: false`），原生的 macOS「Zoom」（雙擊標題列／選單列
Window → Zoom）與「Fill」都對這個視窗完全沒有作用——視窗大小不會變。
用選單結構查證過這個視窗只有 `AXCloseButton`／`AXFullScreenButton`／
`AXMinimizeButton` 三顆按鈕，沒有標準的 `AXZoomButton`。這代表「最大化」
這個狀態在目前這個視窗設定下，一般使用者可能根本無法用標準手勢觸發，
不是這次接的 `tauri-plugin-window-state` 造成的——換不同做法一樣換不出
「最大化」狀態。plugin 本身在這個情境下正確記錄了 `"maximized": false`，
沒有記錯，只是這條路徑沒有機會被觸發到，所以「最大化後關閉重開會維持
最大化」這一項**沒有被驗證到**。是否要另外處理這個視窗手勢限制，留給
使用者決定，不在這次的範圍內。

## 問題

AITerm 每次啟動視窗大小都固定 800×600（`src-tauri/tauri.conf.json` 的
`app.windows[0]`），不管使用者上次關閉前把視窗調成什麼大小、拖到哪個位置、
或是最大化過，下次開啟都要重新調整一次。

## 目標

關閉 App 前的視窗大小、位置、是否最大化，下次開啟時自動還原。三平台
（macOS／Windows／Linux）都要能用。

## 做法

採用官方維護的 `tauri-plugin-window-state`（v2），不自己手刻
resize/move/close 事件監聽與存檔邏輯。

### 運作方式

- 視窗 `Resized`／`Moved` 事件發生時，plugin 在記憶體裡即時更新暫存狀態
  （不會每次都寫磁碟）。
- App 結束、`RunEvent::Exit` 觸發時，plugin 把暫存狀態寫進磁碟上的設定檔。
- 下次啟動時，plugin 自動讀回存檔並套用到視窗，不需要額外的 Rust 或前端
  程式碼去手動呼叫還原。
- 沒有存檔（例如全新安裝、第一次啟動）時，退回 `tauri.conf.json` 目前的
  800×600 預設值，行為與現在一致。
- 記住的位置如果因為外接螢幕被拔掉等原因，超出目前所有螢幕的可見範圍，
  plugin 內建的邊界檢查會自動把視窗拉回可見範圍，不需要額外處理。

### 為什麼 `RunEvent::Exit` 這個時機點是安全的

這個專案已經有依賴 `RunEvent::Exit` 做收尾工作的先例——`src-tauri/src/
lib.rs:687` 的 IMAP 登出就是掛在這個事件上，理由是 `ExitRequested` 可能被
取消，掛在那裡做「真的要結束才做」的事會有把不會發生的離開當成真的離開來
處理的風險。

查證過 macOS 上的自訂 Quit 選單項目（`src-tauri/src/quit.rs`，v1.32.0 加的
關閉確認機制的一部分）雖然繞過了 `WindowEvent::CloseRequested` /
`RunEvent::ExitRequested`，但最終還是會走到 `RunEvent::Exit`——這正是
`tauri-plugin-window-state` 落盤存檔所掛的事件，兩者天生相容：

- 使用者在關閉確認對話框按「取消（繼續執行）」：App 不結束，`RunEvent::
  Exit` 不會觸發，狀態本來就不該存，維持原樣。
- 使用者按「關閉並中止」，或沒有進行中工作時直接關閉：走到真正的結束，
  `RunEvent::Exit` 觸發，plugin 存檔。

不需要為了這個功能去改動既有的關閉確認流程。

## 改動範圍

- `src-tauri/Cargo.toml`：新增 `tauri-plugin-window-state` 依賴（比照現有
  的 `tauri-plugin-dialog`、`tauri-plugin-notification` 等寫法）。
- `src-tauri/src/lib.rs`：在既有的 `.plugin(...)` 註冊鏈裡加一行
  `.plugin(tauri_plugin_window_state::Builder::default().build())`。
- `src-tauri/tauri.conf.json`：不需要改，現有的 `width`/`height` 繼續當
  「還沒有存檔時」的預設值。
- 前端：不需要改。這個功能完全在 Rust 端透過視窗事件運作，不經過
  `invoke`，`src-tauri/capabilities/default.json` 也不需要加權限。

## 不做的事

- 不記憶多視窗（這個 App 目前只有一個主視窗，沒有 label，plugin 預設行為
  就是套用到它）。
- 不另外做「重設回預設大小」的 UI 入口——這次沒有被要求，YAGNI。

## 測試

Rust 端沒有值得寫的單元測試——這是 plugin 內部行為，不是這個專案自己的
邏輯。驗證方式是三平台真機測試：

1. 調整視窗大小／位置／最大化後正常關閉（紅色按鈕、Cmd+Q／Alt+F4），
   重新開啟，確認還原成上次的狀態。
2. 全新安裝（沒有存檔）時，確認還是 800×600 的預設值。
3. 有工作進行中時關閉視窗，在確認對話框按「取消」，確認視窗狀態沒有被
   提早存檔覆蓋掉（用調整過但還沒關閉的大小去驗證：取消後再手動關閉，
   應該存的是取消當下的最終大小，不是被取消的那次嘗試的中間值——這其實
   是同一個狀態，只是用來確認「取消」不會導致存到一半或存壞）。
4. （若方便測試）拔掉外接螢幕，確認記住的位置若跑到螢幕外會被拉回可見
   範圍，不會出現視窗開在畫面外看不到的情況。
