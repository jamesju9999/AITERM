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
| Windows 上的 `-e`／腳本 | **一律不自動執行**，只在指定目錄開分頁（PowerShell 與 cmd 的引號規則不同，都不安全；留給 Windows spec） |
| 除錯建置（`tauri:dev`）是否啟用 single-instance | **不啟用**——dev 與正式版共用 identifier `com.aiterm.app`，已有安裝版在跑時 dev 會把參數轉給它然後靜默退出 |

## 架構

### 1. 啟動請求解析（純函式，跨平台）

新增 `src-tauri/src/launch/` 模組。

```rust
pub struct LaunchRequest {
    pub cwd: Option<String>,
    pub script: Option<String>,      // .command / .sh，需使用者確認才執行
    pub command: Option<Vec<String>>, // -e 之後的 argv，直接送進 shell
}

pub fn parse_args(argv: &[String], invoking_cwd: Option<&Path>) -> Vec<LaunchRequest>
```

規則：

- 位置參數是**資料夾** → `cwd`。
- 位置參數是 `.command`／`.sh` 檔 → `cwd` 取其父目錄，`script` 填檔案路徑。
- 位置參數也接受 `file://` URL（經 `url` crate 解碼成路徑）。AppImage 的自我整合會把
  `Exec=` 改寫成 `%U`，檔案管理員因此傳來 `file:///…/my%20proj`；Dolphin 等也送 URI。
  只對位置參數生效，`--working-directory=file://…` 不解析。
- `--working-directory=X` 或 `--working-directory X` → `cwd`。
- `-e`／`--command`／`-x` 之後的**所有** argv → `command`（`x-terminal-emulator -e`
  的慣例：之後全是指令與其參數）。
- 相對路徑以 `invoking_cwd` 展開；不存在的路徑略過，不產生請求。
- 無法辨識的旗標略過。
- `--headless` 由 `main.rs` 在進入 Tauri 之前攔截，不經過本函式。

### 2. 三個入口收斂到同一個佇列

| 入口 | 觸發 |
|------|------|
| 冷啟動 | `setup` 內以 `std::env::args_os()`（逐項 lossy 轉成 `String`；`args()` 遇到非 UTF-8 參數會 panic）與目前 cwd 呼叫 `parse_args` |
| 第二次啟動 | `tauri-plugin-single-instance` 的 callback 帶入 argv 與 cwd（**cwd 可能是空字串**，三個平台的外掛實作都用 `unwrap_or_default()`，空字串視為未知）；同時把主視窗拉到前景。**僅正式建置註冊**（見決策表） |
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
- 排空**串行化**（一條 promise chain），避免兩次重疊的 `take` 亂序處理批次。
- 取走請求後的處理**不可**檢查「effect 已被 cleanup」：StrictMode 開發模式下第一個 effect
  實例被 cleanup 後它的 `await` 才回來，請求已從佇列取走，丟掉就永久遺失。改用穩定的 ref 呼叫
  最新的 handler。此不變量有 StrictMode 測試釘住。

### 4. 前端行為

每個請求：

1. 開新終端機分頁，`pty_create(cwd)`（`src/ipc/pty.ts` 已支援 `cwd`），並切為使用中。
2. `command`：shell 就緒後送出。就緒訊號是 shell 自己發的 OSC 133 A 加上輸出安靜 250 ms；
   若一直看不到 133;A（自訂 shell、序列剛好被 chunk 切開、或 shell 在前端訂閱 PTY 事件之前
   就畫好提示字元）則 10 秒後保底送出。指令字串以 POSIX 單引號逐參數引號化；只要任何參數
   含控制字元（0x00–0x1f、0x7f）或 Unicode 格式字元（C1、零寬、bidi 等），就**不自動送出**，
   只開分頁——因為字串是敲進 pty 的，行編輯器會在 shell 解析引號之前處理 Tab／DEL／^C。
3. `script`：先顯示**應用內**確認對話框（含完整路徑），確認後開分頁並執行；選「只開啟資料夾」
   則開分頁但不執行。不使用 `window.confirm`（Tauri 內有已知問題）。因為 `initialCommand`
   只在 `TerminalView` 掛載時讀取，所以是「先確認、再開分頁」，使用者看到的結果與先開後確認
   相同。對話框**預設聚焦在安全的「只開啟資料夾」**、Esc 等於跳過（啟動請求可能在使用者
   正於別的分頁打字時到來，預設聚焦在 Run 會讓下一個 Space／Enter 直接執行腳本）；多個
   排隊的腳本每個用遞增序號當 `key`，確保每個對話框都重新套用安全預設。
   路徑含控制字元或 Unicode 格式字元時不出確認框、不執行，只開資料夾（否則對話框顯示的
   路徑可能與實際不同，例如 U+202E）。代價：檔名含 ZWJ／ZWNJ／LRM／RLM 的腳本（部分波斯文、
   印度文、emoji 序列檔名）永遠不會自動執行，使用者可在開好的分頁手動執行。
4. 新增 i18n 字串（en／zh-TW），en 與 zh-TW 兩邊都要補（en 是合併物件，`tsc` 抓不到漏字串，
   要靠 `localeSources` 測試）。

### 5. 系統註冊

**macOS**（`src-tauri/tauri.macos.conf.json` ＋ Info.plist 合併檔）
- 宣告可開啟 `public.folder` 與 `.command`／`.sh`（`CFBundleDocumentTypes`，Role 為 Viewer／
  Shell，不搶預設）。
- 效果：Finder「打開方式」、拖到 Dock 圖示。不承諾「設為預設」。

**Linux**（`src-tauri/tauri.linux.conf.json`）
- `.desktop`：`Categories=System;TerminalEmulator;Development;`、`MimeType=inode/directory;`、
  `Exec={{exec}} %F`。透過 `bundle.linux.deb.desktopTemplate` 與 `bundle.linux.rpm.desktopTemplate`
  覆寫。**自訂樣板會整份取代 Tauri 的預設樣板**，所以預設樣板裡的 `StartupWMClass={{exec}}`
  必須自己帶著（少了它 GNOME／KDE 的 dock 無法把執行中的視窗歸到啟動器）。AppImage 也經由
  bundler 的 `debian::generate_data` 用到 deb 的樣板。
- deb：`postInstallScript` 執行
  `update-alternatives --install /usr/bin/x-terminal-emulator x-terminal-emulator <bin> 10`；
  `preRemoveScript` 在 `remove`／`deconfigure` 時執行 `update-alternatives --remove x-terminal-emulator <bin>`，
  升級不移除。`<bin>` 從套件已安裝的 `.desktop` 的 `Exec=` 讀出（`dpkg -L`），不寫死。
  **priority 是 10，不是 40**：Debian sid 上 x-terminal-emulator 的實際優先度是 terminator 50；
  gnome-terminal／konsole／xfce4-terminal 40；mate-terminal 35；tilix／lxterm 30；
  xterm／lxterminal／urxvt／kitty／foot 20。設 40 會贏過所有 ≤35 的終端機，且若先於某個 40 的終端機
  安裝就一直保有預設，等於悄悄接管使用者的預設終端機。10 低於所有現有終端機，不會被自動選中，
  仍可用 `update-alternatives --config x-terminal-emulator` 選用；它若是唯一候選則自動生效。
- AppImage／rpm：只有 `.desktop` 類別，沒有 `x-terminal-emulator` 註冊。
- **release workflow 同步**：`release.yml` 的「Patch tauri.linux.conf.json」步驟用內嵌 python dict
  **整份重寫**該檔給 .deb 打包，所以新增的 `bundle.linux` 區塊必須同步進那個 dict，否則正式發布的
  .deb 會悄悄丟掉註冊。由 `os_registration.rs` 的守卫測試釘住（執行工作流實際會產生的 JSON 並與
  repo 內的 conf 比對，唯一允許的差異是 DB2 sidecar 路徑）。
- `.gitattributes` 以 `src-tauri/linux/* text eol=lf` 釘住維護腳本的行尾（`#!/bin/sh\r` 會讓 dpkg 失敗）。

## 錯誤處理

- 路徑不存在／無權限：略過該請求，不彈錯誤。
- `script` 檔不可讀或沒有執行權限：不特別處理——確認框已顯示完整路徑，執行時由 shell 自己報
  `permission denied`（Terminal.app 對不可執行的 `.command` 也是失敗，行為相當）。
- 佇列在主視窗建立前入列：由「先入列後排空」機制承接，不特別處理。
- 參數含控制字元／Unicode 格式字元：fail closed，只開分頁不執行（見 §4）。

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
- macOS：需用**正式 build**（`tauri:dev` 沒有 single-instance，且 Info.plist 文件類型只在打包後的
  `.app` 才有）。實測用**不同的 identifier**（`com.aiterm.launchtest`）、隔離的 `HOME`、獨立的
  `CARGO_TARGET_DIR`，只操作自己啟動的 PID——**不可**照舊寫的 `pkill -x AITerm`：程序名是 `app`，
  且會殺掉使用者正在用的 App；同一個 identifier 下，剛編好的 build 也會把參數轉給已在跑的舊版然後靜默退出。
  這台 Mac 上 `cargo build --release` 需加 `CARGO_PROFILE_RELEASE_STRIP=none`（strip 後的 proc-macro
  dylib 無法 `dlopen`，master 上同樣失敗，與本功能無關）。
- **2026-09-20 macOS 實測結果**（打包出的 `.app`；螢幕當時鎖定，無法截圖或點對話框，
  以下用「PTY 子行程的 cwd 與行程表」佐證，不依賴畫面）：
  | 情境 | 結果 |
  |------|------|
  | 打包後 `Info.plist` | 合併成功，兩個文件類型、`LSHandlerRank=Alternate` 都在 |
  | 冷啟動 `open -a <app> "<含空格的資料夾>"` | ✅ 多一個 PTY shell，cwd 為該資料夾 |
  | 已在執行時再 `open -a` 另一個資料夾 | ✅ 同一個行程、多一個 shell，cwd 正確，沒有第二個實例 |
  | `open -a` 一個 `.command` | ⚠️ 確認前沒有新 shell、腳本沒有執行——**與對話框等待中一致，但無法與「請求被丟掉」區分** |
  | 二進位直接帶 `--working-directory=… -e sleep 300`（single-instance 轉發） | ✅ 第二個行程 **0.05 s** 交接後退出；新 shell 的 cwd 正確；`sleep 300` 在其底下執行 |
  | 從啟動到指令開始執行 | ✅ **0.73 s**（走 OSC 133 A＋安靜的快路徑，非 10 s 保底；zsh、乾淨的 `HOME`） |
  第二次啟動的交接延遲（原先擔心要先跑完 DB 初始化）在此環境實測可忽略，不需處理。
- **尚未驗證（需要人在螢幕前）**：確認對話框實際出現、預設聚焦、Run／Skip 的行為；新分頁是否成為
  前景分頁（有 Vitest 釘住，但沒有實機）；Finder「打開方式」清單與拖到 Dock 圖示；慢 rc 的使用者
  （oh-my-posh、p10k）下的就緒時序。
- Linux：沒有實機。只驗證產出的 `.desktop` 與 deb 腳本內容；`update-alternatives`
  實際行為要靠 CI 或 VM，未實測前不宣稱完成。
- 跨平台：`parse_args` 與佇列不含平台專屬 API；single-instance 三平台皆可用，
  Windows 上任何指令／腳本都不會被自動執行（見決策表）。

## 已知限制

- **Windows 上的 single-instance 轉發以 `|` 串接參數**（外掛內部實作）：含 `|` 的 `-e` 參數會被拆開，
  無法在我們這邊修。且轉發端用 `std::env::args()`，遇到非 UTF-8 參數會在送出端 panic。Windows 不在本份
  範圍，且 Windows 上的指令一律不自動執行。
- **updater 重啟會重放原始 argv**：以 `aiterm -e <cmd>` 啟動的工作階段，更新後重啟會重新入列並再執行該指令。
- **除錯建置沒有 single-instance**：無法在 `tauri:dev` 驗證「第二次啟動轉發」，要用正式建置。
  同一個 identifier 下，正式建置若遇到已在跑的（舊版）AITerm 也會轉給它然後退出——驗收前要先 `pkill -x AITerm`。
- **第二次啟動會先跑完 `run()` 內 Builder 之前的初始化**（開 5 個 SQLite、`prune_expired`、載入專案）才能
  交接給第一個實例：single-instance 外掛只能在 plugin setup 階段攔截。見驗收段的實測數字。
- **就緒訊號的時序**：OSC 133 A 在提示字元畫出**之前**發出，且前端要先訂閱 PTY 事件才收得到；慢 prompt
  （oh-my-posh、p10k 無 instant prompt）時指令可能先於提示字元出現（輸入不會丟，tty 行規範會緩衝），
  shell 在訂閱前就畫好提示字元時則要等 10 秒保底。
- **檔名含 ZWJ／ZWNJ／LRM／RLM 的腳本不會自動執行**（見 §4），只開資料夾。
- `.command` 沒有執行權限時，確認後會得到 `permission denied`（不做 `sh <file>` 的退路）。
- 未做 Finder Services、未讓 AppImage 註冊 `x-terminal-emulator`；`AppImage` 的 `.desktop` 只多了終端機類別與資料夾。
- 只在 macOS 上驗證；Linux 只驗證產出的檔案與腳本邏輯（以假的 `dpkg`／`update-alternatives`），
  真實 deb 的 `Exec=` 值、`update-alternatives` 實際行為、Nautilus「在終端機開啟」是否出現 AITerm，
  要等 CI 出 deb 後在 Ubuntu 上驗證。

## 給第二份 spec（Windows）的接口

`LaunchRequest`、`LaunchQueue`、`take_launch_requests` 與前端消費端不含平台假設，
Windows 的 `ITerminalHandoff` 伺服器收到交接後，可轉成同一種請求（另需新增「附著既有
ConPTY 管線」的欄位，屬第二份 spec 範圍）。
