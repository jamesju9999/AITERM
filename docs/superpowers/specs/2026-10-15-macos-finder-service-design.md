# macOS Finder 服務：「在 AITerm 開啟」

日期：2026-10-15（分支 `feat/finder-service`）

## 問題

v1.30.0 讓 AITerm 能被「打開方式」叫起來，但 Finder 對一般資料夾的右鍵沒有「打開方式」（macOS 本來如此）。Windows 已經有右鍵「在 AITerm 開啟」，macOS 缺這個入口。

## 行為（使用者決定）

- Finder 選取一個或多個項目 → 右鍵 →「服務」→「在 AITerm 開啟」（系統語言非中文時是「Open in AITerm」）。
- 資料夾：在該資料夾開新分頁。檔案：在檔案所在的資料夾開新分頁，**不執行檔案**（`.sh`／`.command` 也一樣）。
- 多個項目各開一個分頁；同一個資料夾只開一次；已不存在的路徑略過。
- AITerm 沒開著時由 macOS 啟動它；開著時在現有視窗開分頁並拉到前景。

## 做法：App 內建 NSServices

另外考慮過 Automator 快速動作（要另外安裝到 `~/Library/Services`、移除 App 會留下殘檔）與 Finder Sync 擴充（要另建 Xcode target 與簽章），都比較重。

### Info.plist（`src-tauri/Info.plist`，Tauri 會合併）

```xml
<key>NSServices</key>
<array><dict>
  <key>NSMenuItem</key><dict><key>default</key><string>Open in AITerm</string></dict>
  <key>NSMessage</key><string>openInAITerm</string>
  <key>NSPortName</key><string>AITerm</string>
  <key>NSRequiredContext</key><dict/>
  <key>NSSendFileTypes</key><array><string>public.item</string></array>
</dict></array>
```

`NSRequiredContext` 讓服務預設就啟用。中文名稱放 `zh-Hant.lproj/ServicesMenu.strings`（`"Open in AITerm" = "在 AITerm 開啟";`），透過 `tauri.macos.conf.json` 的 `bundle.resources` 放進 `Contents/Resources/zh-Hant.lproj/`。

### Rust（`src-tauri/src/launch/macos_service.rs`，只在 macOS 編譯）

- `service_requests(paths: &[PathBuf]) -> Vec<LaunchRequest>`（純函式，跨平台可測）：資料夾→自己；檔案→上一層；不存在→略過；依出現順序去重；`script` 與 `command` 一律 `None`。
- objc2 `define_class!` 的 `AITermServiceProvider`，方法 `openInAITerm:userData:error:`：從 `NSPasteboard.pasteboardItems()` 逐項讀 `NSPasteboardTypeFileURL` 字串 → `url::Url` → 路徑 → `service_requests` → `enqueue_and_notify` ＋ `raise_main_window`。
- `register(app)`：在 `setup` 裡（主執行緒）`NSApplication.setServicesProvider(provider)`，provider 用 static 保留；接著 `NSUpdateDynamicServices()`。AppHandle 存在 `OnceLock`。
- `objc2-app-kit` 加 features：`NSApplication`、`NSResponder`、`NSPasteboard`、`NSPasteboardItem`。

## 測試

- `service_requests`：資料夾、檔案→上一層、腳本檔不變成 script、多個項目去重、不存在略過。每條先紅。
- 守衛測試（`tests/os_registration.rs`）：`Info.plist` 有 `NSServices`，`NSMessage` 與 Rust 的 selector 常數一致；`ServicesMenu.strings` 的鍵等於 `NSMenuItem.default`；`tauri.macos.conf.json` 有把它放進 resources。
- 實機：服務只對打包好的 App 生效。用隔離的打包版（不同 identifier、不碰使用者的 AITerm）確認 `pbs -dump` 裡有服務、`Contents/Resources/zh-Hant.lproj/ServicesMenu.strings` 有打包進去；最後請使用者在 Finder 右鍵點選。

## 已知限制

- Finder 預設把服務放在「服務」子選單；要放到最上層需使用者到「系統設定 → 鍵盤 → 鍵盤快速鍵 → 服務」調整。
- `tauri dev` 不會登記服務（不是 .app）。
- 剛安裝或更新後，系統可能要重新登入或執行 `/System/Library/CoreServices/pbs -update` 才會出現（啟動時已呼叫 `NSUpdateDynamicServices()` 降低這個情況）。
