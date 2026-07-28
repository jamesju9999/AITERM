# App 內一鍵自動更新（Tauri Updater）

**日期**：2026-07-28
**狀態**：待審閱

## 背景與目標

AITerm 目前已經會偵測新版本，但只到「告知」為止：`src/App.tsx:34-52` 在啟動時打 GitHub tags API，發現版本不同就在 TabBar 顯示紅點；`src/components/Settings/AboutPage.tsx:20-39` 有一顆「檢查更新」按鈕，偵測到新版時提供一個連結，點下去只是用瀏覽器開啟 GitHub release 頁面。使用者仍必須自己下載安裝檔、自己走一次安裝流程。

**目標**：使用者在 App 內按下按鈕，App 自行下載新版、驗證簽章、就地替換，重啟後即為新版本，全程不需接觸安裝程式。

技術方案採用 Tauri 官方的 `tauri-plugin-updater`（v2）。

## 範圍界定（重要決策，已與使用者確認）

| 決策點 | 選擇 |
|---|---|
| 更新機制 | **Tauri 官方 `tauri-plugin-updater`**，真正的就地替換 + 重啟，非「下載安裝檔再開啟」 |
| 觸發方式 | **啟動自動檢查 + 主動彈窗提醒**，同時保留 About 頁的手動檢查／更新按鈕 |
| Linux `.deb` | **保留 .deb 發佈**，但 deb 安裝的使用者在 App 內降級為手動提示（＝現行行為），不提供一鍵按鈕 |
| Release 流程 | build job 改為 **`releaseDraft: true`**，由新增的 `finalize` job 組出 `latest.json` 後才 publish |
| 下載完成後 | **不自動重啟**，停在「已就緒」狀態等使用者明確按下重新啟動 |
| 版本檢查邏輯 | 目前 `App.tsx` 與 `AboutPage.tsx` 各有一份重複實作，**收斂為單一 `useUpdater` hook** |

### 明確排除（Non-goals）

- 不做「完全靜默背景更新」（使用者無感自動裝好）——對終端機 App 風險太高。
- 不做更新頻道（stable / beta）切換。
- 不做增量／差分更新（delta update）。
- 不做 macOS Intel（x86_64）支援——現行 release 本來就只建 `aarch64-apple-darwin`。
- 不改變 `.deb` 的建置與發佈方式，也不動 Linux DB2 sidecar 隨 .deb 發行的現況。

## 信任鏈（前置條件，必須先於程式碼完成）

Tauri updater 的安全性完全建立在簽章上：App 只接受用專屬私鑰簽署過的更新包。這是防止第三方投遞惡意更新的唯一防線。

1. 在本機產生金鑰對：
   ```bash
   npm run tauri signer generate -- -w ~/.tauri/aiterm_updater.key
   ```
   （會要求設定密碼。）
2. **公鑰**寫入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`——可公開，進 git。
3. **私鑰與密碼**存為 GitHub Repository Secrets：
   - `TAURI_SIGNING_PRIVATE_KEY`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

### 已完成（2026-07-28）

金鑰對已產生於 `~/.tauri/aiterm_updater.key{,.pub}`，GitHub Secrets 已設定完成。

minisign key ID：`B5F4C8732C15A4A`

寫入 `tauri.conf.json` 的 `plugins.updater.pubkey` 值（base64 全文，可公開）：

```
dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEI1RjRDODczMkMxNUE0QQpSV1JLV3NFeWgweGZDMktVNHhiOUpxUnE5WXdRaHRqV3JnRnFMOFg0V1ByMWRVQ2Yxc1JVcGR6SQo=
```

### 兩個不可逆的前提

- **私鑰遺失 = 自動更新永久失效。** 所有已安裝的 App 都只信任內嵌的那把公鑰；更換金鑰等同要求全體使用者手動重裝一次。金鑰檔必須另行備份保存。
- **現有 v1.1.0 使用者無法自動升級上來。** 他們安裝的版本不含 updater plugin，因此第一個支援更新的版本（例如 v1.2.0）必須手動安裝一次，之後才進入一鍵更新的循環。

### 附帶效益

Updater 下載的檔案不經瀏覽器，不會被 macOS 加上 quarantine 屬性。因此**透過自動更新升級的 macOS 使用者不再需要執行 `xattr -cr /Applications/AITerm.app`**——這個目前寫在每份 release note 裡的痛點會自然消失（僅限自動更新路徑；首次手動安裝仍需要）。

## 發佈端設計（`.github/workflows/release.yml`）

### `latest.json` 清單

App 透過一份 `latest.json` 判斷是否有新版：

```json
{
  "version": "1.2.0",
  "notes": "……更新說明……",
  "pub_date": "2026-07-28T00:00:00Z",
  "platforms": {
    "darwin-aarch64": { "signature": "...", "url": "https://github.com/jamesju9999/AITERM/releases/download/v1.2.0/AITerm.app.tar.gz" },
    "windows-x86_64": { "signature": "...", "url": "https://github.com/jamesju9999/AITERM/releases/download/v1.2.0/AITerm_1.2.0_x64-setup.exe" },
    "linux-x86_64":   { "signature": "...", "url": "https://github.com/jamesju9999/AITERM/releases/download/v1.2.0/AITerm_1.2.0_amd64.AppImage" },
    "linux-aarch64":  { "signature": "...", "url": "https://github.com/jamesju9999/AITERM/releases/download/v1.2.0/AITerm_1.2.0_aarch64.AppImage" }
  }
}
```

上列檔名為示意。`finalize` job 不寫死檔名，而是從 draft release 的實際 asset 列表比對副檔名與 `.sig` 配對推導出網址，避免 Tauri 產物命名規則變動導致清單失效。

`tauri.conf.json` 的 updater endpoint 指向：
```
https://github.com/jamesju9999/AITERM/releases/latest/download/latest.json
```

### 為什麼需要獨立的 `finalize` job

現行 workflow 有 **6 個平行的 build job**。若讓每個 job 各自產生並上傳 `latest.json`，就是六個 job 對同一個 release asset 做 read-modify-write：先完成的 job 寫入的平台項目會被後完成的 job 覆蓋掉，最終清單只剩其中一兩個平台。這不是理論風險，平行 matrix 必然觸發。

因此新增一個 `finalize` job：

- `needs: build`——等全部 6 個 build job 結束才啟動，是唯一的寫入者，不存在 race。
- 從 draft release 撈取各平台的 updater artifact 與對應 `.sig` 檔。
- 組出完整的 `latest.json` 並上傳為 release asset。
- 最後才把 release 從 draft 改為 published。

搭配 build job 改為 `releaseDraft: true`，可消除「release 已公開、但 `latest.json` 尚未齊全」的空窗期——否則在該空窗期檢查更新的使用者會拿到殘缺的清單。

### 其他 workflow 改動

- `src-tauri/tauri.conf.json` 加入 `bundle.createUpdaterArtifacts: true`。
- 各 build job 加入 `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 環境變數。
- 兩個 `.deb` job（`ubuntu-24.04`、`ubuntu-24.04-arm`）不產生 updater artifact；`.deb` 維持現行建置與發佈方式不變。
- 產出的 updater artifact：macOS 為 `.app.tar.gz`、Windows 為 NSIS `-setup.exe`、Linux 為 `.AppImage`，各自附帶 `.sig`。

## App 端設計

### Rust（`src-tauri/`）

- `Cargo.toml` 加入 `tauri-plugin-updater = "2"` 與 `tauri-plugin-process = "2"`（後者提供 relaunch 能力）。
- `src/lib.rs:213` 附近註冊兩個 plugin。
- `capabilities/default.json` 加入 `updater:default`、`process:allow-restart`。
- 新增 command `updater_supported() -> bool`：
  - 非 Linux：一律 `true`。
  - Linux：以 `std::env::var("APPIMAGE").is_ok()` 判定。

#### 為什麼需要 `updater_supported`

`latest.json` 的 `linux-x86_64` 項目指向的是 **AppImage**。若讓 `.deb` 安裝的使用者直接呼叫 `check()`，它會回報「有新版」並給出 AppImage 的下載網址——一鍵更新會失敗，或在最壞情況下裝出與其安裝方式不符的東西。

因此必須在**提供按鈕之前**就用明確的環境判定擋掉，而不是等呼叫失敗後去比對錯誤訊息字串（字串比對會隨 plugin 版本改動而失效）。

### 前端：單一 hook `src/hooks/useUpdater.ts`

取代目前分散在 `App.tsx:34-52` 與 `AboutPage.tsx:20-39` 的兩份重複 tags 檢查實作。兩份實作各自打 GitHub tags API、各自用字串相等比對版本，是雙重維護點與潛在的行為不一致來源。

**狀態機**：

```
idle → checking → none | available | unsupported | error
                     │
                  available → downloading(進度) → ready → (使用者按下重新啟動)
                                                → error
```

**狀態型別**：

```ts
type UpdaterState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "none" }
  | { status: "available"; version: string; notes: string }
  | { status: "downloading"; version: string; downloaded: number; total: number | null }
  | { status: "ready"; version: string }
  | { status: "unsupported"; version: string }   // .deb 或其他不支援的安裝方式
  | { status: "error"; message: string };
```

**對外介面**：`{ state, check(), install(), relaunch(), dismiss() }`

`unsupported` 狀態仍帶有新版本號，讓 `.deb` 使用者看得到「有 v1.2.0 可用，請至 GitHub 下載」加上連結——即現行行為原封不動保留。

下載進度來自 `downloadAndInstall()` 的事件回呼（`Started` 提供 `contentLength`、`Progress` 提供 `chunkLength`、`Finished`）。`total` 允許為 `null`，因為伺服器不保證回傳 `Content-Length`；此時 UI 顯示不定量進度指示而非百分比。

### UI

由於 `tauri.conf.json` 設定 `decorations: false`（自繪標題列），更新提示採用 **in-app React modal**，而非 dialog plugin，樣式沿用既有的 `aiterm-btn` / `aiterm-btn--primary`。

流程：

1. 啟動時自動檢查 → 偵測到新版跳出**非阻斷式**彈窗：版本號、更新說明、【立即更新】【稍後】。
2. 按下【立即更新】→ 同一彈窗切換為下載進度條。
3. 下載完成 → 切換為【重新啟動以完成更新】。
4. TabBar 紅點沿用現有 `hasUpdate` prop（`App.tsx:88`），改由 `useUpdater` 供給。
5. About 頁保留手動【檢查更新】按鈕，並在有新版時顯示【立即更新】。

**狀態共享方式**：`AboutPage` 位於 `SettingsView` 路由之下，離 `App.tsx` 有數層之遠，逐層傳 prop 不可行。因此比照既有的 `LocaleContext`，新增 `src/contexts/UpdaterContext.tsx`：hook 實例在 `AppRoutes` 建立一次，透過 Provider 下放。彈窗與 About 頁因此共用同一份狀態，不會出現兩邊各自檢查、進度不同步的情形。

### 設計約束：重啟必須由使用者明確觸發

AITerm 是終端機應用程式——重啟會終止所有 PTY session 與正在執行的指令。因此：

- 下載完成後**絕不自動重啟**，停在「已就緒」狀態。
- 重新啟動按鈕旁明確標示：「重新啟動將結束所有終端機分頁與執行中的指令」。
- 使用者可直接關閉彈窗——安裝已經完成，下次自行開啟 App 時新版本即生效。

這使得體驗實際上是「兩鍵」而非「一鍵」，是為了資料安全刻意付出的代價。

## 已知風險與權衡

### 權限收窄

`capabilities/default.json` 使用 `updater:allow-check` + `updater:allow-download-and-install`，而非 `updater:default`。後者額外含 `allow-download` 與 `allow-install` 兩項本專案用不到的授權——`install(update_rid, bytes_rid)` 接受呼叫端提供的 bytes resource，等於通往同一個安裝原語的第二條無人使用路徑。此檔既有風格本就是窄授權（`dialog:allow-open`/`allow-save` 而非 `dialog:default`，六條顯式 `core:window:allow-*`）。

`process` 只授予 `allow-restart`，不用 `process:default`——後者含 `allow-exit`，等於給前端一個「無確認直接終止所有 PTY」的原語，與本設計刻意要求使用者確認重啟的原則直接衝突。

### `csp: null` 與 updater 的交集

更新的網路請求走 Rust 端的 `reqwest`，不受 webview CSP 管轄，且 minisign 驗簽無條件執行——被注入的 JS 無法讓 updater 安裝任意程式碼。這部分是正交的。

但有一個非零交集：plugin 的 `check` command 接受前端傳入的 `allow_downgrades`，為 true 時版本比較器從 `update.version > current` 換成 `update.version != current`。也就是說 webview 中的任意 JS 可以繞過單調遞增保證，強制回滾到任何**經過正確簽章的舊版**，重新引入已修掉的漏洞。同一個 command 也接受呼叫端指定的 `proxy`。

前提是要先有 XSS。AITerm 會渲染 AI 輸出與 web-fetch 內容，這個前提並非全然不可及。`csp: null` 是既有設定、不屬於本次變更，但「替一個 `csp: null` 的 app 接上網路更新器」正是風險計算改變的時點，故記錄於此。收窄權限無法消除這點（`allow-check` 就足以觸發）。

### 已知殘留風險：同 tag 多個 draft release

`releaseDraft: true` 讓 `tauri-action` 無法用 `getReleaseByTag` 取得既有 release（draft 沒有真正的 tag ref），改成分頁掃描全部 release 尋找同名 tag 的 draft，找不到就建立。GitHub **允許**同一個 `tag_name` 存在多個 draft，所以兩個 build job 同時撲空時會產生兩個 draft，資產各分一半。

觸發窗口只有幾秒（release 建立發生在 build 完成後，六個 leg 的時長差距通常是數十分鐘），且後果 fail-safe——`finalize` 下載到的 draft 缺 `.sig`，`expected` 檢查會 hard fail，不會發出殘缺的 manifest。但需要人工刪除多餘 draft。

根治方式是新增一個先跑的 `create-release` job 產出 `releaseId`，六個 build 步驟改吃該 input（會整段跳過 `getOrCreateRelease`）。這超出本次「surgical」範圍，列為後續工作。

**首次實跑時應確認**：draft 只有一個，且其中只有一份 `latest.json`。

### 開發者本機建置

`createUpdaterArtifacts: true` 一旦啟用，`tauri build` 在偵測到 pubkey 卻找不到 `TAURI_SIGNING_PRIVATE_KEY` 時會直接失敗。沒有私鑰的開發者必須改用 `npm run tauri build -- --no-sign`。此限制記錄於 `CLAUDE.md` 的 Commands 區塊。

## 錯誤處理

| 情境 | 行為 |
|---|---|
| 啟動時自動檢查失敗（網路等） | 靜默忽略，維持現行 best-effort 行為 |
| 使用者手動按檢查而失敗 | 顯示錯誤訊息 |
| 簽章驗證失敗 | 明確錯誤訊息，**不**靜默降級為手動下載連結 |
| 下載中斷 | 進入 `error` 狀態，允許重試 |
| `.deb` 等不支援的安裝方式 | `unsupported` 狀態，顯示 GitHub 下載連結 |

## i18n

新增字串至 `src/lib/i18n.ts`，en 與 zh-TW 兩組並行。

既有 key 的處置：`about_update_available`、`about_checking`、`about_up_to_date`、`about_check_updates` 全部沿用，文案不動。`about_update_link`（「前往下載」）僅保留給 `unsupported` 狀態使用。

`about_update_error`（「檢查失敗，請稍後再試」）**淘汰**，由新的 `update_failed`（「更新失敗」）取代——hook 用同一個 `error` 狀態涵蓋「檢查失敗」與「下載失敗」兩種情況，原文案對後者是錯的。

新增的 key 涵蓋彈窗標題、【立即更新】、【稍後】、下載進度、【重新啟動以完成更新】、重啟警告文字、更新失敗訊息、不支援自動更新的說明。

## 測試策略

**自動化測試**：

- `useUpdater` 的 Vitest 單元測試：mock `@tauri-apps/plugin-updater`，覆蓋 available / none / unsupported / error / 下載進度（含 `total` 為 `null`）各路徑。
- 更新彈窗元件的 render 測試：各狀態顯示正確的按鈕與文案。
- `updater_supported` 的 Rust 單元測試：涵蓋 Linux 有／無 `APPIMAGE` 環境變數的分支。

**端對端驗證（無法以自動化測試取代）**：

自動更新無法在 `tauri:dev` 下驗證——dev build 不會 self-update。要確認完整鏈路（簽章 → `latest.json` → 下載 → 就地替換 → 重啟）真的可運作，唯一方法是實際發佈兩次：

1. 打 tag 發佈第一個 updater-enabled 版本，手動安裝。
2. 打下一個 tag 發佈更新版本。
3. 在已安裝的 App 中確認彈窗出現、下載成功、重啟後版本號正確。

實作計畫中必須排入此步驟，不得以「單元測試通過」代替端對端驗證。

## 驗證結果（2026-07-28）

### 自動化

前端 358 個測試 / 47 個檔案、Rust 294 個單元測試加整合測試全數通過，`tsc --noEmit` 乾淨。分支變更檔案的 `eslint` 只剩一個既有例外（`UpdaterContext.tsx` 的 `react-refresh/only-export-components`，與 `LocaleContext.tsx` 完全同型）。

關鍵路徑以 mutation testing 驗證，非僅看綠燈：`useUpdater` 共 12 個變異全數被抓、`UpdateModal` 7 個全數被抓、`AboutPage` 4 個全數被抓。

### 發佈端（v1.2.0 與 v1.2.1 各驗一次，結果一致）

| 檢查 | 結果 |
|---|---|
| release 已發佈並標為 latest | `draft=false latest=true` |
| 同 tag release 筆數 | 1（未觸發多 draft 風險） |
| `latest.json` 份數 | 1（證實 `uploadUpdaterJson: false` 生效） |
| 平台 key | 四個齊全，無缺少無多餘 |
| 簽章長度 | 404–420 字元，非空 |
| App 實際 endpoint | HTTP 200，解析到正確版本 |
| manifest 內四個下載 URL | 全部 HTTP 200 |
| `.deb.sig` | 出現在資產清單，未進入 `latest.json`（如預期被跳過） |

一併證實了 Task 10 當時標記為未驗證的假設：`gh release download --pattern '*.sig'` 讀得到 draft release 的資產。

### 端對端（macOS Apple Silicon，使用者實機操作）

v1.1.0 → 手動安裝 v1.2.0 → 自動更新至 v1.2.1，使用者確認全部項目無問題，涵蓋：更新提示框自動出現、【稍後】後 TabBar 紅點續亮、About 頁顯示可用更新、下載進度前進、重啟警告出現、**不會自行重啟**、按下重啟後版本正確、macOS 無需 `xattr -cr`。

已安裝的 v1.2.0 在 `latest.json` 尚為 1.2.0 時正確顯示「已是最新版」，證明版本比較不會誤報。

### 尚未驗證

Windows 與 Linux（AppImage / `.deb` fallback）的端對端更新未實機測試——僅 macOS 走完全程。發佈端產物對所有平台皆已驗證齊全。

## 受影響檔案

| 檔案 | 改動 |
|---|---|
| `src-tauri/tauri.conf.json` | 加 `plugins.updater`（pubkey + endpoints）、`bundle.createUpdaterArtifacts` |
| `src-tauri/Cargo.toml` | 加 `tauri-plugin-updater`、`tauri-plugin-process` |
| `src-tauri/src/lib.rs` | 註冊 plugin、註冊 `updater_supported` command |
| `src-tauri/src/commands.rs` | 新增 `updater_supported` |
| `src-tauri/capabilities/default.json` | 加 `updater:default`、`process:allow-restart` |
| `.github/workflows/release.yml` | build job 改 draft、加簽章 env、新增 `finalize` job |
| `package.json` | 加 `@tauri-apps/plugin-updater`、`@tauri-apps/plugin-process` |
| `src/hooks/useUpdater.ts` | 新增 |
| `src/contexts/UpdaterContext.tsx` | 新增 |
| `src/components/UpdateModal.tsx` + `UpdateModal.css` | 新增 |
| `src/App.tsx` | 移除內嵌 tags 檢查，改用 `useUpdater`，掛載彈窗 |
| `src/components/Settings/AboutPage.tsx` | 移除內嵌 tags 檢查，改用 `useUpdater`，加【立即更新】 |
| `src/lib/i18n.ts` | 新增字串 |
