# 讓 AITerm 被作業系統當成終端機使用（地基＋macOS＋Linux）

日期：2026-09-19
範圍：第一份 spec。Windows 預設終端機（`ITerminalHandoff` COM 委派）另開第二份 spec，
本份只保證解析層與佇列可被它重用。

## 目標

- 從 Finder／檔案管理員「用 AITerm 開啟」資料夾，或雙擊 `.command`／`.sh`，會在
  **執行中的** AITerm 視窗開新分頁，cwd 為該資料夾（或該檔所在目錄）。
- Linux 上 AITerm 可作為 `x-terminal-emulator` 的替代品：支援
  `-e <指令>` 與 `--working-directory=<路徑>`。
- 這條路徑目前完全不存在：`tauri.conf.json` 只有 `category: DeveloperTool`，沒有
  single-instance、沒有啟動參數解析、沒有 `RunEvent::Opened` 處理。

## 非目標

- Windows 預設終端機、Windows 檔案總管整合（第二份 spec）。
- macOS Finder Services（「在此開啟 AITerm」右鍵服務）。macOS 沒有預設終端機設定，
  只做「打開方式」與拖到 Dock 圖示。
- AppImage／rpm 的 `x-terminal-emulator` 註冊（AppImage 沒有安裝階段；rpm 只給
  `.desktop` 類別）。

## 已定案的決策

| 問題 | 決定 |
|------|------|
| AITerm 已在執行時被要求開資料夾 | 加 single-instance，在既有視窗開新分頁並切過去 |
| 開 `.command`／`.sh` | 新分頁，經應用內確認對話框後自動執行 |
| Linux 參數 | 支援 `-e` 與 `--working-directory` |
| 拆分 | 地基＋macOS＋Linux 為第一份 spec |

## 架構

### 1. 啟動請求解析（純函式，跨平台）

新增 `src-tauri/src/launch/` 模組。

```rust
pub struct LaunchRequest {
    pub cwd: Option<PathBuf>,
    pub script: Option<PathBuf>,     // .command / .sh，需使用者確認才執行
    pub command: Option<Vec<String>>, // -e 之後的 argv，直接送進 shell
}

pub fn parse_args(argv: &[String], invoking_cwd: Option<&Path>) -> Vec<LaunchRequest>
```

規則：

- 位置參數是**資料夾** → `cwd`。
- 位置參數是 `.command`／`.sh` 檔 → `cwd` 取其父目錄，`script` 填檔案路徑。
- `--working-directory=X` 或 `--working-directory X` → `cwd`。
- `-e`／`--command`／`-x` 之後的**所有** argv → `command`（`x-terminal-emulator -e`
  的慣例：之後全是指令與其參數）。
- 相對路徑以 `invoking_cwd` 展開；不存在的路徑略過，不產生請求。
- 無法辨識的旗標略過。
- `--headless` 由 `main.rs` 在進入 Tauri 之前攔截，不經過本函式。

### 2. 三個入口收斂到同一個佇列

| 入口 | 觸發 |
|------|------|
| 冷啟動 | `setup` 內以 `std::env::args()` 與目前 cwd 呼叫 `parse_args` |
| 第二次啟動 | `tauri-plugin-single-instance` 的 callback 帶入 argv 與 cwd；同時把主視窗拉到前景 |
| macOS `RunEvent::Opened { urls }` | 取 `file://` URL 轉成路徑，餵給同一個解析／入列流程 |

`RunEvent::Opened` 掛在既有的 `.run(|app_handle, event| …)`（`lib.rs`）。

### 3. 送達前端：先入列、後通知、前端主動排空

原則：事件在前端訂閱之前送出會永久遺失（本 repo 已踩過兩次），所以請求不靠事件承載。

- `LaunchQueue`：`Mutex<Vec<LaunchRequest>>`，Tauri managed state。
- 入列後發 `launch-request-pending` 事件（無酬載）。
- 新 command `take_launch_requests() -> Vec<LaunchRequest>`：排空並回傳。
- 前端在 `TerminalApp` 掛載時**先 `listen`、再呼叫 `take_launch_requests()`**；
  之後每次收到事件再排空一次。訂閱前入列的請求會在第一次排空時取到。
- 監聽器必須在 `TerminalApp` 層（永遠掛載），不可放在條件掛載的子元件。
- 監聽器需正確 `unlisten`，避免同一請求被處理兩次。

### 4. 前端行為

每個請求：

1. 開新終端機分頁，`pty_create(cwd)`（`src/ipc/pty.ts` 已支援 `cwd`），並切為使用中。
2. `command`：shell 就緒後送出（沿用既有的「就緒訊號」機制，不自己猜延遲）。
3. `script`：先顯示**應用內**確認對話框，內容含完整路徑；使用者確認後才送出執行。
   不使用 `window.confirm`（Tauri 內有已知問題）。取消則保留已開好的分頁，什麼都不執行。
4. 新增 i18n 字串（en／zh-TW），en 與 zh-TW 兩邊都要補（en 是合併物件，`tsc` 抓不到漏字串，
   要靠 `localeSources` 測試）。

### 5. 系統註冊

**macOS**（`src-tauri/tauri.macos.conf.json` ＋ Info.plist 合併檔）
- 宣告可開啟 `public.folder` 與 `.command`／`.sh`（`CFBundleDocumentTypes`，Role 為 Viewer／
  Shell，不搶預設）。
- 效果：Finder「打開方式」、拖到 Dock 圖示。不承諾「設為預設」。

**Linux**（`src-tauri/tauri.linux.conf.json`）
- `.desktop`：`Categories=System;TerminalEmulator;`、`MimeType=inode/directory;`。透過
  `bundle.linux.deb.desktopTemplate` 覆寫（Tauri 2 已提供）。
- deb：`postInstallScript` 執行
  `update-alternatives --install /usr/bin/x-terminal-emulator x-terminal-emulator <bin> 40`；
  `preRemoveScript` 執行 `update-alternatives --remove x-terminal-emulator <bin>`。
  priority 40 低於多數發行版預設終端機，不搶既有設定，使用者可用
  `update-alternatives --config` 切換。
- AppImage／rpm：只有 `.desktop` 類別。

## 錯誤處理

- 路徑不存在／無權限：略過該請求，記錄 log，不彈錯誤。
- `script` 檔不可讀：分頁照開，不執行，顯示一行提示。
- 佇列在主視窗建立前入列：由「先入列後排空」機制承接，不特別處理。

## 測試

**Rust（單元）**
- `parse_args`：資料夾、`.command`／`.sh`、`--working-directory` 兩種寫法、`-e` 吃掉其後全部
  argv、相對路徑展開、不存在路徑略過、未知旗標略過。
- `LaunchQueue`：入列後 `take` 取得且清空、再次 `take` 為空。

**前端（Vitest）**
- 掛載後先訂閱再排空：訂閱**之前**就入列的請求仍會開分頁。
- 收到 pending 事件會再排空。
- `script` 請求：確認前不送出指令、確認後才送出、取消則不送。
- unmount 後不再處理事件。
- 每個新測試都要先證明會紅（fixture 必須能區分正確與錯誤行為）。

**驗收（實機）**
- macOS：本機用 `open -a AITerm <資料夾>` 驗證冷啟動與已執行兩種情況，需用**正式 build**
  並確認跑的是新二進位（`tauri:dev` 的 watcher 不可靠）。
- Linux：沒有實機。只驗證產出的 `.desktop` 與 deb 腳本內容；`update-alternatives`
  實際行為要靠 CI 或 VM，未實測前不宣稱完成。
- 跨平台：`parse_args` 與佇列不含平台專屬 API；single-instance 三平台皆可用，
  Windows 行為不受本份 spec 影響（未註冊任何 Windows 入口）。

## 給第二份 spec（Windows）的接口

`LaunchRequest`、`LaunchQueue`、`take_launch_requests` 與前端消費端不含平台假設，
Windows 的 `ITerminalHandoff` 伺服器收到交接後，可轉成同一種請求（另需新增「附著既有
ConPTY 管線」的欄位，屬第二份 spec 範圍）。
