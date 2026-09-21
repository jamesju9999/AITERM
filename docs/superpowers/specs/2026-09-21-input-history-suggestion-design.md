# 命令輸入框的歷史灰字建議與接受鍵設定 — 設計

日期：2026-09-21

## 問題

使用者希望在命令列有「推薦命令」，並且能自己選擇用 **Tab** 還是 **→** 接受（參考 Warp 的
「Accept autosuggestion」設定）。AITerm 底部輸入框（`WarpInput`）目前沒有任何行內建議：
只有按 ↑ 展開的歷史清單；Tab 沒被攔截，→ 只是移動游標。

## 目標與非目標

**目標**
- 輸入時，在游標後面用灰字顯示「最近一筆以目前輸入開頭的歷史指令」的剩餘部分（同 fish／zsh-autosuggestions）。
- 接受鍵可在設定頁選：`Tab`（預設）、`→`、`關閉建議`。
- 接受只是把整段填進輸入框，**不自動送出**。

**非目標**
- AI 推薦（使用者選擇先做歷史灰字）。
- 逐字接受（Warp 的 ⇧→）、自訂按鍵。
- 跨分頁即時同步歷史（每個輸入框掛載時讀一次，沿用現況）。

## 行為規則

建議只在**同時**滿足時出現：
1. 設定不是 `off`。
2. 輸入框非空、單行（不含換行）、沒有停用。
3. 游標在文字結尾（沒有選取範圍）。
4. 歷史清單、目錄選單都沒開。
5. 沒有在輸入法組字中。
6. 沒有指令正在執行（`isCommandRunning` 為 false）——那時輸入框的按鍵另有用途。

建議 = 歷史（新→舊）中第一筆「以目前輸入開頭、且比它長」的指令（區分大小寫），顯示其剩餘部分。

接受鍵：
- `tab`：有建議時 Tab 補上整段並 `preventDefault`；沒有建議時維持現況（不攔）。
- `right`：有建議且游標在結尾時 → 補上整段並 `preventDefault`；其他情況 → 維持正常移動游標。
  另一把鍵（未被選為接受鍵者）永遠維持原本行為。
- 補上後游標移到結尾，並重新計算輸入框高度；不送出。

## 設計

### 1. 後端設定（`src-tauri/src/config/types.rs`、`commands/config.rs`、`lib.rs`）
- `enum SuggestionAcceptKey { Tab(預設), Right, Off }`，`serde(rename_all = "kebab-case")`。
- `AppConfig.suggestion_accept_key`，`#[serde(default)]`：舊的 config.toml 沒有此欄位時解析成 `Tab`。
- command `set_suggestion_accept_key`，仿 `set_submit_shortcut`；註冊於 `generate_handler!`。

### 2. 前端型別與設定頁
- `src/ipc/config.ts`：`SuggestionAcceptKey = "tab" | "right" | "off"`、`AppConfig.suggestion_accept_key`、`setSuggestionAcceptKey`。
- `Settings/GeneralPage.tsx`：在「輸入組合鍵」旁新增一段三個單選（沿用 `mode-list` 樣式），中英文字串。
  `suggestion_accept_key` 缺值時前端退回 `"tab"`（`cfg.suggestion_accept_key ?? "tab"`）。

### 3. 純函式（`src/lib/commandSuggestion.ts`）
`findSuggestion(history: string[], value: string): string | null` — 回傳要顯示的**剩餘部分**。
規則 2、（歷史新→舊、開頭相同、較長）都在這裡；UI 條件（游標、清單、組字…）留在元件，函式保持可獨立測試。

### 4. `WarpInput`
- 新 prop `suggestionKey?: SuggestionAcceptKey`，缺省視為 `"off"`（沒傳的呼叫端行為完全不變）。
- textarea 外包一層 `position: relative; flex: 1` 的容器，後面疊一個 `aria-hidden` 的 overlay：
  同字型／字級／行高、`white-space: pre-wrap; word-break` 與 textarea 一致、`pointer-events: none`；
  內容是「不可見的 `value`」＋「灰色的建議剩餘部分」。
- 游標是否在結尾：`onSelect` 更新 state（`selectionStart === selectionEnd === value.length`）。
- `handleKeyDown`：在既有 Enter／方向鍵處理之前判斷接受鍵（見「行為規則」）；IME 組字中一律不處理（既有第一行已 return）。
- overlay 不影響既有輸入、歷史導覽、送出與 `onRawKey` 行為。

### 5. 接線
- `TerminalView`：`suggestionKey` state，`refreshConfig()` 內 `setSuggestionAcceptKey(cfg.suggestion_accept_key ?? "tab")`
  （設定頁改完，切回視窗時 `refreshConfig` 會重新讀取，沿用 `submit_shortcut` 的機制）。
- `RemoteTerminalView`：掛載時 `getConfig()` 讀一次，傳給它的 `WarpInput`。

## 測試

- Rust：預設 `Tab`；舊 config（無此欄位）解析為 `Tab`；`Right`／`Off` TOML 往返。
- `commandSuggestion.test.ts`：空輸入、無符合、完全相同（不建議）、多筆取最新、區分大小寫、含換行輸入不建議。
- `WarpInput.test.tsx`：灰字出現／消失的各條件；`tab` 模式 Tab 接受、→ 不接受；`right` 模式反之，且游標不在結尾時 → 只移動游標；
  `off` 完全沒有灰字、Tab／→ 行為不變；接受後不送出（`onSubmit` 未被呼叫）；多行、歷史清單開啟、指令執行中皆無建議。
  每個新測試先證明會紅，「取決於最新狀態」的判斷用變異驗證。
- 設定頁：三個選項切換會呼叫 `setSuggestionAcceptKey`。

## 真機驗收
用隔離的 `tauri dev`（見 memory `project-window-close-confirm`）在真實輸入框：先送出一筆歷史指令，再輸入其開頭 → 看到灰字；
Tab 與 → 各自在對應設定下接受／不接受；`off` 無灰字；在設定頁切換後回到終端機生效。Windows／Linux 無實機，結案時明確標示未驗證。

## 驗證狀態（實作完成時）

**已驗證（自動化）**：Rust 全 workspace 51 個測試二進位皆 ok；前端 207 個檔案／1799 個測試通過；`tsc -b` 乾淨；四個受影響檔案的
lint 錯誤數與功能開始前完全相同（4／1／1／13，無新增）。每個守門條件（設定關閉、游標不在結尾、組字中、歷史／目錄選單開啟、
指令執行中、修飾鍵、接受鍵互斥）與設定頁、兩個畫面的接線，都以變異驗證證明對應測試會紅。

**尚未驗證（真機）**：灰字疊層與輸入框的**實際視覺對齊**（字型、行高、換行位置、長字串換行時是否錯位），以及 Tab／→ 在真實輸入框的操作。
原因：驗收進行到一半時 Mac 螢幕被鎖定（前景程序變成 `loginwindow`），無法再截圖與送出輸入，依規定不嘗試解鎖，已收掉自己的測試程序。
jsdom 不做版面計算，所以對齊問題**只有真機看得出來**；在這之前不宣稱「灰字位置正確」。
驗收要點：先送出一筆歷史指令再輸入其開頭 → 灰字緊接在游標後；輸入夠長讓它換行 → 灰字與輸入的換行位置一致；
Tab、→ 在對應設定下補上；設定頁切換後回到終端機生效。Windows／Linux 無實機，同樣未驗證。

## 已知限制
- 建議只來自本機最近 100 筆歷史（沿用現有上限）。
- 中文輸入法組字中不顯示建議（避免與候選字視窗衝突）。
