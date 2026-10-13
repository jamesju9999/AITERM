# 電腦操作：Linux Wayland 後端

## 為什麼
Ubuntu 25.10 起預設只有 Wayland 工作階段，X11 後端（`linux.rs`）在那裡一律回「不支援」。Wayland 不讓程式
自己讀別人的視窗或注入輸入，唯一正路是 xdg-desktop-portal。

## 做法（`computer/linux_wayland.rs`，純邏輯在 `waylandmap.rs`）
- 啟動時 `resident::portal::is_wayland()` 為真就用 `WaylandBackend`，否則維持 X11 的 `LinuxBackend`。
- 截圖：Screenshot portal（非互動）。讀完即刪 portal 存的檔。物理像素依螢幕串流的邏輯大小縮成邏輯座標。
- 輸入：RemoteDesktop portal＋ScreenCast（取得螢幕座標系）。鍵盤用 keysym、滑鼠用絕對移動＋evdev 按鈕、
  滾輪用離散步數（方向與本程式相反）。授權用 restore token 存在 `~/.local/share/com.aiterm.app/wayland_input_token`。
- portal 靠 App ID 認人：寫一份隱藏的 `~/.local/share/applications/com.aiterm.app.desktop`，並在同一條
  D-Bus 連線上 `register_host_app`（與全域快捷鍵共用 `registered_connection`）。
- 沒有 App 清單與切換前景：整個桌面是一個虛擬 App（`Desktop`／`桌面`，pid −1）。`snapshot` 只回一個全螢幕視窗，
  共用邏輯會引導模型改用 OCR（tesseract）與圖示偵測。`open_app`／`open_url` 同 X11 版。

## 已知限制
- 桌面可能在截圖時閃光或播放快門聲。
- 第一次使用要在桌面的授權框按允許（截圖、遠端控制各一次）。
- 只支援單一螢幕（`select_sources` 不開多選）。
- 只通過 x86_64-unknown-linux-gnu 型別檢查與純邏輯測試，尚未在實際 Wayland 桌面驗證。
