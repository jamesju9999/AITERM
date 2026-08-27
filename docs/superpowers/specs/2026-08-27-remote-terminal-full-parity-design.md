# 遠端終端機共享 計畫③A：觀看端 UI/UX 完整對等 — 設計

日期：2026-08-27
狀態：待使用者核可

## 問題

計畫①②完成後，遠端分頁能連線、能看、能打字，但實機測試（Mac 主控端 ← Windows 觀看端）發現體驗跟本機終端機差很多：畫面高度對不上、沒有分段卡片、沒有書籤、輸入方式是直接對著 xterm 逐鍵轉送而不是本機那條有歷史記錄的輸入框。使用者要的是「完整的終端機體驗，唯一差別是資料流由遠端過來」。

## 範圍

**含：**

- `useTerminalBlocks` 改成可插拔寫入目標與主控端平台，讓遠端分頁能重用同一套 OSC 133 解析、分段卡片邏輯
- `RemoteTerminalView` 整合 `WarpInput`（結構化輸入框，含歷史）與即時逐鍵轉送（互動程式如 vim、Ctrl+C）
- xterm 外觀（字型/主題/游標）跟本機分頁同步
- 尺寸：主控端決定 cols/rows，觀看端自動縮字級塞滿視窗，塞不下給捲軸
- 書籤：直接重用 `CommandBookmarks`，不需改動
- 協定新增一個欄位：主控端平台（給 Windows ConPTY 清畫面同步用）

**不含（明確排除，理由見下）：**

- AI 面板（`/ai` `/agent` 指令執行、「Ask AI」按鈕）——需要新的信任邊界設計，留給計畫③B
- 指令卡片的 git 分支/增減行數徽章——需要主控端本機 git 資料，跟 AI 面板同性質，留給③B 或未來項目一併考慮
- 輸入框的路徑自動完成——同樣需要主控端本機檔案系統，直接不啟用（`WarpInput` 沒有 `sessionId` 時本來就會停用這個功能，不用改它）

## 現況調查（設計的事實基礎）

| 事實 | 位置 |
|---|---|
| `useTerminalBlocks` 的分段卡片邏輯完全由 `appendOutput(chunk)` 解析輸出位元組流（OSC 133 標記）驅動，不直接讀本機 PTY——理論上可以接上任何位元組來源 | `src/hooks/useTerminalBlocks.ts` |
| 但 hook 內部三處硬寫 `writePty(sessionId, ...)`：整行送出、`clear`/`cls`、Windows ConPTY 的 Ctrl+L 同步 | `src/hooks/useTerminalBlocks.ts:211,265,302` |
| Windows 判斷用 `navigator.platform`——這只在本機分頁成立（看的人＝跑 shell 的人）。搬到遠端分頁會判斷成觀看端自己的平台，不是主控端的，會讓 ConPTY 清畫面同步在跨平台情境下失效或誤發 | `src/hooks/useTerminalBlocks.ts:250` |
| `WarpInput` 已經是寫入目標無關的：透過 `onSubmit` callback 送出整行文字，`sessionId` 只用來做路徑自動完成 | `src/components/WarpInput.tsx` |
| 本機終端機的鍵盤輸入其實是兩條通道並行：`term.onData` 直接逐鍵轉送給 PTY（互動程式用），`WarpInput` 是額外疊加的結構化整行輸入框，兩者之間有 diff 兩邊畫面文字來還原「使用者實際打了什麼」的同步機制，外加 IME 組字、focus-tracking 跳脫序列過濾等細節 | `src/components/TerminalView.tsx:1281` 附近 |
| `RemoteTerminalView` 目前沒有套用 app 的字型/主題設定，`cursorBlink: false`（本機是 `true`），沒有 `ResizeObserver`／字級自動縮放 | `src/components/RemoteTerminalView/index.tsx` |
| AI context 的 `dir_listing`、指令卡片的 `getGitBlockInfo(cwd)` 都是直接呼叫本機檔案系統/git，不是解析輸出位元組流得到的——這是它們無法直接搬到遠端分頁的根本原因 | `src-tauri/src/ai/context.rs:29`、`src/components/TerminalView.tsx:645` |
| `ServerMessage::Granted` 目前只帶 `mode`/`cols`/`rows` | `src-tauri/src/share/protocol.rs:115` |

## 協定變更：主控端平台

`Granted` 訊息新增 `host_os: "windows" | "macos" | "linux"`，直接用後端已經在算的 `std::env::consts::OS`（AI context 的 `os` 欄位就是這樣拿的）。不含使用者名稱、路徑等敏感資訊，純粹是「這台機器的 shell 是什麼系統跑的」，觀看端拿它取代 `navigator.platform` 判斷。

`PROTOCOL_VERSION` 這次不用跳號——`Granted` 加欄位是向後相容的擴充（觀看端只讀它認得的欄位），沒有改變既有欄位的語意。

## `useTerminalBlocks` 的介面變更

新增兩個帶預設值的參數，本機分頁呼叫端**字面上不用改**：

```typescript
export function useTerminalBlocks(
  sessionId: string,
  term: Terminal | null,
  cwdRef?: React.RefObject<string>,
  onLiveClear?: () => void,
  onCommandSettled?: (exitCode: number) => void,
  onCommandStarted?: (cmd: string) => void,
  /** 指令怎麼寫出去。預設包一層 `writePty(sessionId, data)`，跟改動前的行為
   *  完全一樣。遠端分頁傳 `(data) => shareViewerSend(connId, data)`。 */
  write: (data: string) => void = (data) => writePty(sessionId, data),
  /** 主控端平台，只影響 Windows ConPTY 的 Ctrl+L 清畫面同步邏輯（見上面
   *  「現況調查」那條）。預設讀 `navigator.platform`，跟改動前行為一樣；
   *  遠端分頁傳 `Granted` 訊息裡的 `host_os`。 */
  hostPlatform: "windows" | "other" = navigator.platform.toLowerCase().startsWith("win") ? "windows" : "other",
): UseTerminalBlocksResult
```

原本三處 `writePty(sessionId, ...)` 改呼叫 `write(...)`；`const isWindows = navigator.platform...` 改成 `const isWindows = hostPlatform === "windows"`。

## `RemoteTerminalView` 的組裝

```
RemoteTerminalView
├─ useTerminalBlocks(fakeSessionId, term, cwdRef, onLiveClear, ..., write, hostPlatform)
│    write：mode !== "control" 時傳一個 no-op（唯讀根本不建立會寫入的 write，
│           不是每次呼叫都判斷一次）；control 時傳 (data) => shareViewerSend(connId, data)
│    hostPlatform：從 Granted 事件拿到的 host_os 轉成 "windows" | "other"
├─ WarpInput
│    onSubmit：同上面的 write；用既有的 `parseAiPrefix.ts` 判斷是不是
│              `/ai`/`/agent` 開頭（跟本機分頁同一套規則，不重新猜字首），
│              是的話不呼叫 write，顯示「AI 指令目前不支援於遠端分頁」提示
│    disabled：mode !== "control"
│    sessionId：不傳（讓路徑自動完成自然停用）
├─ 分段卡片列表（複用 TerminalBlockCard，隱藏 Ask AI 按鈕、不顯示 git 徽章）
├─ 一小段即時逐鍵轉送（複製 term.onData 裡忽略 focus-tracking 跳脫序列、
│   AI 面板開著時不轉送這兩條；不含 IME 組字/貼上快照比對——分段卡片的
│   指令文字是從主控端的 OSC 133 標記解出來的，不需要在觀看端重建）
└─ xterm 外觀：套用跟 TerminalView 同一份 settings（字型/字級/主題即時
    同步），cursorBlink: true；ResizeObserver 依容器像素尺寸反推字級塞滿
    cols×rows，最小可讀字級（8px）後不再縮，改由內層容器出捲軸
```

`onShareViewerResync` 的處理除了原本清空 xterm 畫面，現在多呼叫一次 `clearAllBlocks()`——漏掉的位元組可能連帶讓分段卡片內容跟畫面對不上，這跟本機分頁執行 `clear`/`cls` 時「畫面跟卡片一起清空」是同一個邏輯。

## 錯誤處理

- 字級縮到最小仍塞不下：內層容器（`.aiterm-remote-terminal__screen` 的子層，不是整層）開捲軸，外層維持 `overflow: clip`——整層改 `overflow: auto`會重現這個 repo 踩過的「貼上內容被瀏覽器捲出視野變空白」那個坑。
- `/ai`/`/agent` 開頭的輸入：顯示提示文字，不送出、不呼叫 `ai_query`。
- 唯讀模式下 `WarpInput`：整個顯示為停用狀態＋提示，不是送出後才被拒絕（跟原本 spec 「按鍵根本不送出」的精神一致，這次延伸到結構化輸入框上）。

## 測試

- `useTerminalBlocks` 新測試：`hostPlatform: "other"` 時即使跑在 Windows 瀏覽器上也不觸發 Ctrl+L 同步邏輯（證明參數真的生效，不是加了參數但邏輯還是抓 `navigator.platform`）；傳自訂 `write` 後執行指令，斷言 `writePty`（mock）完全沒被呼叫（防止「忘記接新的 write，其實還是寫去本機 PTY」）。
- `RemoteTerminalView` 新測試（比照 `SharePanel`/`tabTypeCoverage` 的寫法，mock IPC）：唯讀時 `WarpInput` 顯示停用；輸入 `/ai ...` 顯示提示且不呼叫 `shareViewerSend`；收到 `Resync` 事件後 blocks 被清空。
- 手動驗證（自動化測不到）：改完 `useTerminalBlocks` 後，在本機分頁手動走一次 WarpInput 歷史方向、Windows Ctrl+L 同步、貼上不被捲出視野這三個曾經記過教訓的行為，確認沒有退化。

## 已知限制

| 限制 | 說明 |
|---|---|
| 即時逐鍵轉送不含 IME 組字/貼上快照比對 | 這兩個機制的目的是「還原使用者實際打了什麼字」給分段卡片用；遠端分頁的卡片內容來自主控端自己的 OSC 133 標記，不需要在觀看端重建，所以刻意不搬 |
| git 徽章、AI 面板、路徑自動完成不可用 | 都需要主控端本機資料或新的信任邊界，明確排除，見「範圍」 |
