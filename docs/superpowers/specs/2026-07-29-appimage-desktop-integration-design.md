# AppImage 桌面整合（選單項目與圖示）

**日期**：2026-07-29
**狀態**：待審閱

## 背景與目標

以 AppImage 執行時，Ubuntu 的 dock 顯示通用執行檔圖示（齒輪）而非 AITerm 的圖示，系統應用程式選單中也找不到 AITerm。`.deb` 安裝則一切正常。

### 已排除的兩個假設

調查過程排除了兩個看似合理的原因，記錄於此以免日後重複：

**不是「app 沒有設定視窗圖示」。** 實機 `xprop` 確認 `_NET_WM_ICON(CARDINAL) = Icon (32 x 32)` 存在且帶有正確的像素資料。在 Rust 端呼叫 `set_icon` 不會有任何幫助——圖示早就在那裡，是 GNOME 不採用。

**不是「`StartupWMClass` 對不上」。** AppImage 內的 `.desktop` 是：

```
Exec=app
StartupWMClass=app
Icon=app
Name=AITerm
```

而 `xprop` 量到的 `WM_CLASS(STRING) = "app", "App"`——**完全相符**。Tauri 的範本本來就有 `StartupWMClass={{exec}}`，沒有缺欄位。

### 真正的原因

**AppImage 不會把 `.desktop` 註冊到系統。** 該檔案只存在於 AppImage 內部，而 GNOME 只從 `/usr/share/applications` 與 `~/.local/share/applications` 讀取。找不到可比對的項目時，GNOME/Ubuntu Dock 對 X11 視窗顯示通用執行檔圖示，**不會**回退去用 `_NET_WM_ICON`（實機確認）。

`.deb` 之所以正常，是因為它會把 `.desktop` 與圖示安裝到系統路徑。

這是 AppImage 的固有設計（單檔可攜、不動系統），不是程式缺陷。本設計是在該設計之上，提供使用者選擇加入的整合。

**目標**：以 AppImage 執行時，讓使用者能選擇建立系統選單項目，使 dock 圖示與應用程式選單正常。

## 範圍界定（已與使用者確認）

| 決策點 | 選擇 |
|---|---|
| 觸發方式 | **首次執行時詢問**，不靜默寫入 |
| 拒絕後 | 記入設定，不再詢問；仍可由設定頁手動建立 |
| 路徑失效 | **每次啟動自動修正** `Exec=` |
| 移除 | **提供**，位於設定頁 |
| 與更新提示並存 | 有更新提示時**不顯示**本提示 |

### 明確排除（Non-goals）

- 不做 `.deb` / macOS / Windows 的任何整合——那些安裝方式本來就正常。
- 不整合 MIME 類型關聯、不註冊為預設終端機。
- 不寫入 `/usr/share/applications`（需要 root，且與 AppImage 的無侵入性相違）。
- 不呼叫 `update-desktop-database`：`~/.local/share/applications` 的變更 GNOME 會自行偵測，多一個外部指令只是多一個失敗面。
- 不處理使用者自行編輯過 `.desktop` 的情形——自動修正只改寫 `Exec=` 一行。

## 偵測

沿用 `commands/updater.rs::supported_for` 已驗證的訊號：`APPIMAGE` 環境變數存在即代表以 AppImage 執行。該變數由 AppImage type-2 runtime 匯出，是格式級契約而非 Tauri 的實作細節；`updater_supported` 的兩個分支已於實機觀察到正確運作（AppImage → 一鍵更新、解壓後執行 → 手動下載提示）。

非 Linux 平台不編譯相關程式碼。

## 後端設計

新增 `src-tauri/src/commands/appimage.rs`：

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationState {
    /// 非 AppImage 執行（含所有非 Linux 平台）
    NotAppimage,
    /// 是 AppImage，但尚未建立選單項目
    Available,
    /// 已建立，附目前 .desktop 指向的路徑
    Integrated { exec_path: String },
}

#[tauri::command] pub fn appimage_integration_state() -> IntegrationState;
#[tauri::command] pub fn appimage_integrate() -> Result<(), String>;
#[tauri::command] pub fn appimage_remove_integration() -> Result<(), String>;
```

### 寫入內容

| 路徑 | 內容 |
|---|---|
| `~/.local/share/applications/aiterm.desktop` | 見下 |
| `~/.local/share/icons/hicolor/<尺寸>/apps/aiterm.png` | 自 `$APPDIR/usr/share/icons/hicolor/` 複製**所有**既有尺寸 |

`.desktop` **不重新撰寫，而是從 AppDir 內既有的那份複製後改寫兩行**：

來源為 `$APPDIR/usr/share/applications/*.desktop`（`APPDIR` 與 `APPIMAGE` 同為 AppImage runtime 匯出）。該檔已由 bundler 依 `tauri.conf.json` 產生，含正確的 `Name`、`Comment`、`Categories`、`StartupWMClass`。

只改寫這兩行：

```
Exec="<$APPIMAGE 的絕對路徑>" %U
Icon=aiterm
```

**這樣拆的理由**：`Comment` 與 `Categories` 來自 `bundle.shortDescription` 與 `bundle.category`。若在此重新撰寫一份，日後改動 `tauri.conf.json` 就會與這裡的硬編碼漂移——本專案稍早才因 release notes 重複六份而讓兩處文案與實際檔名不符。複製既有檔案讓 bundler 維持唯一來源。

若 `$APPDIR` 的 `.desktop` 讀不到（理論上不應發生），退回以最小欄位自行產生，並在錯誤訊息中說明。

三個關鍵細節：

- **`StartupWMClass` 必須維持 `app`**，不是 `aiterm`。那是實機 `xprop` 量到的真實 `WM_CLASS`，來源是 Rust 執行檔名稱（`Cargo.toml` 的 `name = "app"`）。因為是複製而非重寫，這一行天然被保留——但改寫邏輯**絕不能連帶動到它**。寫錯會讓整個功能靜默失效：選單項目存在且看起來正常，但視窗仍比對不到它，圖示依舊是齒輪。
- **檔名用 `aiterm` 而非 `app`。** `app` 過於通用，容易與其他程式的 `.desktop` / 圖示撞名。`StartupWMClass` 與檔名解耦是刻意的。
- **`Exec=` 必須加引號。** AppImage 常放在含空白的路徑（如 `~/我的 下載/`）。

### 自動修正

在 Tauri 的 `setup` 階段執行，不依賴前端：若 `aiterm.desktop` 存在且其 `Exec=` 與目前的 `$APPIMAGE` 不符，改寫該行。

**只改寫 `Exec=` 一行**，其餘保留——使用者若手動調整過其他欄位不應被覆蓋。

放在後端而非前端的理由：即使使用者從不開啟提示，路徑也會維持正確。使用者升級版本或搬移檔案後，只要手動啟動過一次，選單項目就會自我修復。

## 前端設計

### 首次提示

新增元件，樣式沿用 `UpdateModal`（右下角非阻斷式）。顯示條件**全部**成立時才出現：

1. `appimage_integration_state()` 回傳 `available`
2. 設定中的 `appimage_integration_declined` 為 false
3. onboarding 已完成
4. **目前沒有顯示更新提示**

第 3 點：全新使用者首次啟動時已有 `OnboardingWizard`，再疊一個提示過於吵雜。

第 4 點是刻意的優先序取捨。更新提示與 Enterprise 任務通知都位於右下角，三者並存會互相遮擋（該座標衝突已記錄於 in-app-updater 的 spec，尚未解決）。更新比選單圖示重要，故本提示讓位。

【不用了】寫入 `appimage_integration_declined = true` 並關閉，不再詢問。

### 設定頁

`GeneralPage.tsx` 新增一個 `settings-section`，依狀態切換：

| 狀態 | 顯示 |
|---|---|
| `not_appimage` | **整個區塊不渲染** |
| `available` | 說明文字 + 【建立選單項目】 |
| `integrated` | 目前指向的路徑 + 【移除選單項目】 |

這使得拒絕過首次提示的使用者仍有地方反悔，也是移除功能的所在。

## 設定

`ConfigStore` 新增 `appimage_integration_declined: bool`，比照既有的 `onboarding_done`。

## 錯誤處理

| 情境 | 行為 |
|---|---|
| 家目錄不可寫 | `appimage_integrate` 回傳錯誤字串，UI 顯示 |
| AppDir 中找不到圖示 | 仍寫入 `.desktop`（選單項目可用），圖示缺失；不視為失敗 |
| 自動修正時 `.desktop` 無法解析 | 靜默略過，不改寫、不報錯——使用者可能自行編輯過 |
| `appimage_remove_integration` 時檔案已不存在 | 視為成功（冪等） |

## 測試策略

| 層級 | 內容 |
|---|---|
| Rust（純函式） | `.desktop` 內容產生：含空白的路徑必須被正確引號包覆 |
| Rust（純函式） | 改寫後 `StartupWMClass` 仍為 `app`，且 `Name`/`Comment`/`Categories` 原樣保留 |
| Rust（純函式） | `Exec=` 改寫：相同不動、不同才改、只動該行且保留其餘欄位 |
| 前端 | 四個顯示條件各自為假時提示不出現 |
| 前端 | 【不用了】寫入設定旗標 |
| 前端 | 設定頁三種狀態的渲染 |

`.desktop` 的產生與改寫皆抽為純函式（輸入路徑與既有內容、輸出字串），因此在 macOS 上可完整測試——這與 `updater.rs::supported_for`、`pty/commands.rs::drives_from_mask` 是同一種模式。

**mutation testing 為驗收條件**，重點在：把 `StartupWMClass` 改成 `aiterm` 必須有測試失敗。那是整個功能能否生效的關鍵，而錯了不會有任何明顯症狀——選單項目看起來完全正常，只是圖示仍是齒輪。

## 驗證限制

`$APPIMAGE` 只在真實 AppImage 執行時存在，因此**完整流程無法在 macOS 或 CI 驗證**。純函式與 UI 狀態可自動測試；「dock 圖示真的變正確」必須在 Linux 桌面環境實機確認，項目為：

1. 提示出現，且與更新提示不同時出現
2. 建立後 dock 圖示正確、應用程式選單中找得到 AITerm
3. 把 AppImage 搬到別的目錄後重新啟動，選單項目仍可用
4. 設定頁的移除按鈕確實移除項目與圖示
5. `.deb` 與 macOS 上該區塊完全不出現
