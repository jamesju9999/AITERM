# 電腦操作：Linux（X11）後端

日期：2026-10-13　前置：`computer-use-windows-design`（驗證限制與結構相同）

## 範圍

只做 **X11**。Wayland 刻意不支援：它不允許一般程式擷取其他視窗的畫面或注入輸入（要靠各桌面環境專屬的 portal／協定，行為不一致）。偵測到 Wayland 工作階段（`XDG_SESSION_TYPE=wayland`，或沒有該變數但有 `WAYLAND_DISPLAY`）時，**所有工具**第一件事就回傳明確說明：請改用 X11 工作階段登入。沒有 `DISPLAY` 也一樣。

## 驗證限制

開發機是 Mac，沒有 Linux 桌面可以跑。驗證方式：對 `x86_64-unknown-linux-gnu` 做型別檢查、純邏輯放在 `linuxmap.rs`（所有平台的 CI 都跑）、Linux CI 的編譯與單元測試。**實機行為需要在 X11 桌面驗收**；在那之前不得宣稱已支援。

## 對應表

| 能力 | Linux 做法 |
|---|---|
| 連線 | `x11rb`（純 Rust 的 X11 協定，不需要 libxcb 等 C 函式庫） |
| 列 App | EWMH `_NET_CLIENT_LIST`：排除最小化（`_NET_WM_STATE_HIDDEN`）與非一般視窗；名稱＝`/proc/<pid>/comm`，WM_CLASS 當別名與識別 |
| 切到前景 | 向根視窗送 `_NET_ACTIVE_WINDOW`，輪詢 `_NET_ACTIVE_WINDOW` 確認；確認不到明確報錯 |
| 滑鼠／鍵盤／捲動 | XTEST（`fake_input`）：滑鼠移動與按鈕、滾輪是按鈕 4~7；按鍵 `cmd`→Ctrl（與 Windows 一致） |
| 文字輸入 | 鍵盤配置裡有的字元直接送；沒有的（中文、emoji）暫時把一個空的 keycode 對應到 Unicode keysym 再還原（xdotool 做法）；換行→Return、Tab→Tab |
| 截圖 | `GetImage`（ZPixmap，每像素 32 位元）→ PNG |
| 文字辨識 | 呼叫 `tesseract <圖> stdout tsv`（語言 `eng+chi_tra+chi_sim`，沒裝語言包退回 `eng`），解析 TSV 的單字框組成行；沒有安裝時回傳安裝指令 |
| 圖示偵測 | 共用 `icons.rs` |
| 開啟 App／網頁 | App：先 `gtk-launch <名稱>`（認 .desktop 檔名）再直接執行同名程式；網頁：`xdg-open <網址>`，指定 App 時以該程式開啟。名稱是獨立參數、不經 shell |
| 無障礙元件樹／聚焦欄位／直接點元件 | **不做**（見下） |

### 為什麼不做無障礙元件樹（AT-SPI）

AT-SPI 要走 D-Bus、各工具套件（GTK／Qt／Electron）的支援參差、而且常常需要使用者先在桌面設定開啟輔助技術。第一版 `snapshot` 只回傳視窗層級；共用邏輯會因此判定這個 App「幾乎沒有無障礙資訊」，提示並自動改用 OCR（`computer_read_screen`、`computer_click_text`）與圖示偵測——這兩條路徑本來就是為 Chromium／Qt 這類 App 設計的，在 Linux 上它們就是主要路徑。AT-SPI 留待實機驗證 OCR 路徑之後再評估。

## 安全

與其他平台相同：關卡全在 `ComputerService`，後端只是換掉。
