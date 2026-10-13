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
- 截圖走 Screenshot portal，每次 GNOME 都會閃一下。已改成優先從 ScreenCast 串流經 PipeWire 取最新一格
  （`pipewire_capture.rs`），但 pipewire-rs 需要比 Ubuntu 22.04 新的標頭，所以是選用 feature `wayland-pipewire`：
  .deb（24.04 建置）與 CI 開、AppImage（22.04 建置）不開而退回 portal。

## 實機驗證時修掉的問題（Ubuntu 26，GNOME）
- portal 回 `An app id is required`：非沙盒程式要有對應的 `.desktop`，並在同一條連線 `register_host_app`。
- 授權框不跳（`Only the focused app is allowed to show a system access dialog`）：要傳父視窗。Tauri 在
  GNOME Wayland 上跑在 XWayland，handle 是 X11，要用 `WindowIdentifier::from_xid`。
- AI 看不到目標 App：AITerm 自己的視窗擋住畫面 → 動作與截圖前把主視窗縮小。
- tesseract `OPENSSL_3.2.0 not found`、開 App 靜默失敗：AppImage 的 `LD_LIBRARY_PATH` → `strip_appimage_env`。
- Enter 以 keysym 送出沒反應 → 功能鍵與修飾鍵改送 evdev keycode。
