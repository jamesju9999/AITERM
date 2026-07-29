# 拆開更新的下載與安裝

**日期**：2026-07-29
**狀態**：待審閱

## 背景

`tauri-plugin-updater` 2.10.1 的 Windows 安裝路徑在啟動 NSIS 安裝程式後，最後一行是 `std::process::exit(0)`（`updater.rs`，其 doc comment 亦寫明「run before we run the installer and exit the app through `std::process::exit(0)` on Windows」）。

目前 `useUpdater.ts` 的 `install()` 呼叫合併版 API `downloadAndInstall()`，於是三平台行為分歧：

| 平台 | 安裝方式 | App | `ready` 狀態 |
|---|---|---|---|
| macOS | 解壓 `.app.tar.gz` 就地替換 | 存活 | 顯示 |
| Linux AppImage | 就地替換 AppImage | 存活 | 顯示 |
| **Windows** | 啟動安裝程式後 `exit(0)` | **立即結束** | **無法顯示** |

### 真正的問題不是那個畫面

本專案刻意要求使用者明確操作才重啟，理由是 AITerm 是終端機——重啟會結束所有 PTY 分頁與執行中的指令。該警告（`update_restart_warning`：「重新啟動將結束所有終端機分頁與執行中的指令。」）顯示於 `ready` 狀態。

**Windows 上這個警告永遠不會出現。** 使用者可能正在執行長時間指令，按下【立即更新】的下一秒 App 就消失，事前毫無告知。同一顆按鈕在 macOS 上意為「下載」，在 Windows 上意為「立刻關閉所有東西」。

### 一個未被注意的次要問題

即使在 macOS 與 Linux 上，`downloadAndInstall()` 也是**下載完就立刻替換檔案**。使用者若接著按【稍後】繼續工作數小時，他執行的是一個 `.app` bundle 或 AppImage 檔案已被抽換的行程。

## 目標

讓三個平台走**完全相同**的流程，且破壞性動作一律發生在使用者看過警告並明確按下按鈕之後。

## 設計

`@tauri-apps/plugin-updater` 2.10.1 已將兩者拆開（`index.d.ts`）：

```typescript
download(onEvent?: (progress: DownloadEvent) => void, options?: DownloadOptions): Promise<void>;
install(): Promise<void>;
downloadAndInstall(onEvent?, options?): Promise<void>;
```

改用前兩者：

| 使用者動作 | 現在 | 改後 |
|---|---|---|
| 按【立即更新】 | `downloadAndInstall()` | `download()`，**只下載** |
| 進入 `ready` | 已安裝，等重啟 | 已下載，等安裝 |
| 按【重新啟動以完成更新】 | `relaunch()` | `install()` → `relaunch()` |

**不需要任何平台偵測。** 不新增 Rust 指令、不新增 i18n 分支、UI 不區分平台。Windows 的 `install()` 依然 `exit(0)`，但那發生在使用者已同意結束所有分頁之後；NSIS 安裝程式會帶著原本的參數重新啟動 App（plugin 原始碼中的 `current_exe_args` 即為此用途），因此 `relaunch()` 在 Windows 上不會執行也無妨。

`ready` 狀態的既有文案「更新已下載完成」在改後**更貼切**——它字面上就是「已下載」。

## 狀態機的必要調整

| 項目 | 現況 | 改後 |
|---|---|---|
| `pendingRef` | 下載完成後清為 `null`（`useUpdater.ts:177`） | **必須保留**，`install()` 要用 |
| `stagedRef` | 標記已下載，阻止 `check()` 覆蓋 | 不變，同一個守衛繼續有效 |
| `relaunch()` | `await processRelaunch()` | 先 `install()` 再 `processRelaunch()`；失敗時進入 `error` 狀態 |

`relaunch()` 從「不會失敗」變成「可能失敗」，這是本次改動中最容易被忽略的一點：簽章驗證、磁碟權限、安裝程式無法啟動都會在此浮現，必須落到 `error` 狀態（`phase: "install"`）而非靜默。

## 取捨

**記憶體**：下載後的封包會持續被持有到安裝或關閉 App（約 45–90 MB），目前僅短暫持有。判斷為可接受——使用者是主動按下下載的，且 plugin 提供的 `close()` 會使封包失效、必須重新下載，代價更高。

**已接受的行為改變**：使用者按【立即更新】後若一直不重啟，更新就一直不會被套用。這正是本設計的意圖——把「何時失去終端機分頁」的決定權交還給使用者。

## 測試策略

`useUpdater.test.ts` 現有 28 個測試，全部 mock `downloadAndInstall`，需改為 mock `download` 與 `install`。

| 層級 | 內容 |
|---|---|
| 前端 | 按【立即更新】後 `download` 被呼叫，**且 `install` 未被呼叫** |
| 前端 | `ready` 狀態下按重啟，`install` 先於 `relaunch` 被呼叫 |
| 前端 | `install()` 失敗時進入 `error` 狀態且 `phase` 為 `"install"`，不呼叫 `relaunch` |
| 前端 | 下載完成後 `pendingRef` 仍持有 update（以「重啟仍可成功安裝」間接驗證） |
| 前端 | 下載完成後再次 `check()` 不會丟棄已下載的更新（既有 `stagedRef` 行為不得回歸） |

**mutation testing 為驗收條件**，重點在第一項：把 `download()` 改回 `downloadAndInstall()` **必須有測試失敗**。若沒有，這次改動就沒有任何保護——外觀完全正常，而 Windows 使用者依舊在毫無警告下被關掉 App。

## 驗證限制

**Windows 的實際行為無法在 macOS 或 CI 驗證。** 必須實機確認：

1. 按【立即更新】後 App **不會**結束，出現「更新已下載完成」與警告
2. 按【重新啟動以完成更新】後安裝程式啟動、App 結束
3. 安裝完成後 App 自動重新啟動（NSIS 的 `current_exe_args` 行為）
4. macOS 與 Linux 的更新流程未回歸

第 3 項若不成立（安裝後不自動啟動），屬 NSIS 行為而非本設計缺陷，但須據實記錄，因為使用者按下的按鈕寫著「重新啟動以完成更新」。

## 驗證狀態（v1.2.6，2026-07-29）

**本次發版無法驗證任何一項。**

更新流程由**舊版**的程式碼執行：使用者從 v1.2.5 更新到 v1.2.6 時，跑的是 v1.2.5 的 `downloadAndInstall()`，行為與改動前完全相同。新流程要到 **v1.2.6 更新至 v1.2.7** 時才會首次執行。

這與本專案先前遇到的【查看完整說明】按鈕是同一個結構性限制的兩種樣貌：

- **UI 元素**：由新版渲染，但只出現在更新提示中，而收到提示的機器跑的是舊版
- **updater 行為**：由舊版執行，新版的改動要到下一次更新才生效

一般化的說法是：**任何位於更新路徑上的改動，都無法在引入它的那一版被觀察到。**

安裝畫面的外觀（`headerImage` / `sidebarImage` / branding 文字）不受此限，因為它屬於安裝檔本身——但**App 內建更新看不到**：`tauri-plugin-updater` 的 `WindowsUpdateInstallMode` 預設為 `Passive`，其原始碼註解為「unattended mode, which means the installation only shows a progress bar」，歡迎頁與頁首橫幅皆不顯示。須手動執行 `setup.exe` 才看得到。使用者已於 v1.2.6 確認安裝畫面更新無誤。
