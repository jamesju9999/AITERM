# 遠端指令文字還原與卡片化 Design Spec

## 背景與問題

`2026-08-27-remote-terminal-full-parity-design.md`（計畫③A）完成後，實機測試發現兩個
`docs/superpowers/plans/2026-08-27-remote-terminal-full-parity.md` 沒有涵蓋的問題：

1. **直接在遠端終端機分頁打字送出的指令，不會變成 Block Card**——本機端從沒呼叫過
   `submitCommand`/`beginTrackedBlock`，因為那兩支函式只在「這台機器自己送出指令」的
   路徑上（WarpInput 或本機直接打字）。遠端觀看者拿到控制權後送進來的按鍵，是後端
   Rust 直接寫進 PTY，完全不會經過 `TerminalView.tsx` 的 `onData` handler。
2. 因為問題 1，`useTerminalBlocks.ts` 原本「即時窗格自動撐高」邏輯完全依賴「有沒有
   一個 running 中的區塊」這個信號，遠端指令執行時窗格永遠撐不高（或撐高後不知道何
   時該收回）。這個子問題已經用 `onUntrackedCommandBoundary` 機制修過兩輪（撐高時機、
   收回時機各修一次，詳見 `TerminalView.tsx` 裡對應的中文註解與 commit 歷史），但使
   用者在真機測試後指出：**這只是治標**——不管怎麼調高度上限，遠端指令的輸出永遠只
   能停留在一個天生有高度上限、無法變成卡片、無法保留歷史的小預覽窗格裡，跟本機終
   端機「完成的指令會變成可捲動、佔滿版面的卡片」這個核心體驗不一致。

本次要解決的就是問題 1 的根本：**讓遠端指令也能被追蹤成完整的 Block Card，文字要跟
使用者實際輸入的完全一致**，而不是只在卡片標題塞一個通用標籤。問題 1 一旦解決，問題
2 會隨著本機既有的「`visibleBlockCount` 變動觸發撐高/縮回」邏輯自然一併解決，不需要
額外的特殊處理。

## 核心限制

遠端觀看者的按鍵是直接由後端 Rust 寫進 PTY 的（`share/server.rs` 收到觀看端送來的
輸入後直接呼叫 PTY 寫入），完全不經過本機 `TerminalView.tsx` 的 xterm 實例或它的
`onData` handler。本機既有的指令文字還原機制（`readLineExcludingInlinePrediction`，
在 `onData` 裡對每個按下 Enter 的瞬間做「畫面快照 diff」）因此完全用不上——沒有
`onData` 事件可以掛。

但不管輸入來源是本機鍵盤還是遠端，shell 對輸入的**回顯（echo）**與 shell 整合腳本送出
的 OSC 133 標記，最終都會走同一條 PTY 輸出串流、被同一個本機 xterm 實例畫出來。也就
是說，我們不需要攔截輸入本身——只要在對的時間點讀取「畫面上本來就看得到的內容」，就
能反推出指令文字。

## 架構：用 OSC 133 A/C 標記做畫面快照 diff

### 現有 OSC 133 標記時序（`src-tauri/src/pty/shell.rs`）

- **zsh/bash**：`preexec` hook 在使用者按下 Enter、shell 即將執行指令**之前**觸發，
  送出 `C`；此時 shell 的行編輯器（zle/readline）已經完成了 Enter 的換行回顯——也就
  是說遊標已經換到新的一行，**上一行仍完整保留著「提示字元 + 剛打完的指令文字」**。
  `precmd` hook 在下一個提示字元畫出來之前觸發，先送 `D`（若有指令剛結束），再送
  `A`（新提示字元，畫面上還沒有任何輸入）。
- **PowerShell**：目前只有 `prompt` 函式覆寫，在下一個提示字元畫出來時送 `D`+`A`，
  **完全沒有 `C`**（見下方「Windows 擴充」一節，這次要新增）。
- **cmd.exe**：只有靠 `PROMPT` 環境變數送裸的 `D`+`A`，沒有 hook 機制，本次不處理。

### 新增機制

1. **`A` 時做提示字元快照**：`useTerminalBlocks.ts` 的 OSC 133 handler 收到 `A` 時，
   讀取當下游標所在行的內容（`term.buffer.active.getLine(cursorY + baseY)`），存進
   一個 ref（例如 `lastPromptSnapshotRef`）。這一步不分本機/遠端，每次 `A` 都會更新，
   成本很低（讀一行文字，不做任何額外運算）。
2. **`C` 時嘗試還原文字**：收到 `C` 時，先檢查 `blocksRef.current` 有沒有一個
   `status === "running"` 的區塊（跟現有邏輯一樣）：
   - **有**：代表這是本機自己送出的指令（`submitCommand`/`beginTrackedBlock` 已經建
     立了區塊），維持現有行為（no-op）。
   - **沒有**：呼叫新函式 `recoverUntrackedCommand()`（見下一節），嘗試從畫面內容還
     原指令文字。
     - **還原成功**：呼叫既有的 `beginTrackedBlock(還原出的文字)`。這一支函式本來就
       只做「區塊記帳」、不會往 PTY 寫任何東西（見它現有的文件註解），語意上完全符合
       「遠端指令的位元組早就經由 shell 回顯處理過了，這裡只是要幫它建檔」。之後
       `D` 事件會自然把它 finalize——不需要新增任何 finalize 相關程式碼。
     - **還原失敗**（見下方邊界情況）：退回現有的 `onUntrackedCommandBoundary("start")`
       安全網，維持目前的高度撐高行為，不建立卡片。

這個設計最大的好處：一旦 `beginTrackedBlock` 被呼叫，遠端指令就完全匯入本機既有的
資料流——卡片顯示、`visibleBlockCount` 觸發的即時窗格撐高/縮回、Bookmark/Copy，全部
不需要新寫任何專門給「遠端」的 UI 分支。

### `recoverUntrackedCommand()` 演算法

```
輸入：term（Terminal 實例）、lastPromptSnapshotRef.current（字串或 null）
輸出：還原出的指令文字，或 null（代表還原失敗）

1. 若 lastPromptSnapshotRef.current 為 null（這個 session 從沒收過一次 A），回傳 null。
2. row = term.buffer.active.cursorY - 1（游標上一行，OSC C 觸發時遊標已換到新的一行）。
3. lines = [term.buffer.active.getLine(row + baseY) 的純文字（translateToString(true)）]
4. 只要 row 對應的那一行（往上一行）的 isWrapped 為 true，代表它是接續行：
     row -= 1
     把該行純文字接在 lines 前面
   重複直到 isWrapped 為 false 或 row < 0。
5. fullLine = lines 依序串接的結果。
6. 若 fullLine 沒有以 lastPromptSnapshotRef.current 開頭，回傳 null（比對不出乾淨前綴，
   可能是使用者在還沒收到新 A 之前就又送了一次輸入之類的異常情況）。
7. 回傳 fullLine 去掉前綴、trim 過的字串。若結果是空字串，回傳 null（沒有實際指令內容，
   例如使用者直接按了空白 Enter）。
```

`isWrapped` 是 xterm.js `IBufferLine` 內建欄位，標記這一行是不是上一行的自動換行延續，
不需要額外實作換行偵測邏輯。

### 邊界情況

- **這個 session 第一個指令剛好是遠端送的、還沒收過任何 `A`**：`recoverUntrackedCommand`
  回傳 null，退回 `onUntrackedCommandBoundary` 安全網。屬於少見的一次性情況（正常使用
  下，連線建立後本機一定至少已經歷過一次 `A`）。
- **`clear`/`cls`**：`beginTrackedBlock` 內部已經會處理指令字串等於 `clear`/`cls` 的
  情況（清空整個卡片歷史、不建立區塊），還原出來的文字若剛好是這兩個字串，直接沿用
  現有邏輯，不需要額外處理。
- **TUI / 全螢幕互動內容**（vim、htop 等）：這類內容走 `isAlternateBuffer` 判斷的另一
  套高度邏輯，`recoverUntrackedCommand` 只在一般（非 alternate）緩衝區運作，不受影響。
- **還原出的文字比對失敗或為空**：一律視為還原失敗，走安全網，不建立品質可疑的卡片。

## Windows（PowerShell）擴充

PowerShell 目前沒有等效的「即將執行」時間點，需要在 `inject_powershell_integration`
新增一段 `Set-PSReadLineKeyHandler`：

```powershell
$global:__aiterm_orig_enter_handler = (Get-PSReadLineKeyHandler -Chord Enter -Bound `
    -ErrorAction SilentlyContinue).Function

Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
    param($key, $arg)
    if ($global:__aiterm_orig_enter_handler -and
        $global:__aiterm_orig_enter_handler -ne "AcceptLine") {
        [Microsoft.PowerShell.PSConsoleReadLine]::($global:__aiterm_orig_enter_handler)($key, $arg)
    } else {
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine($key, $arg)
    }
    [Console]::Write("$([char]27)]133;C$([char]7)")
}
```

- **順序很重要，且跟直覺相反**：`C` 是在呼叫 `AcceptLine`（或使用者原本綁定的處理
  方式）**之後**才印，不是之前。`AcceptLine` 會讓 PSReadLine 完成這次輸入、回顯結尾
  的換行，之後游標才會真的換到新的一行——這跟 zsh/bash 的 `preexec`「換行回顯已經
  發生、上一行還留著提示字元+指令」時序一致，讓 `recoverUntrackedCommand()` 的
  `cursorY - 1` 邏輯不需要為了 PowerShell 另外分支。若反過來先印 `C` 再呼叫
  `AcceptLine`，這時游標還停在原本那一行（換行回顯還沒發生），`cursorY - 1` 會讀到
  錯的一行。這個順序假設**必須在真的 Windows 機器上驗證**（見下方測試策略）。
- 跟現有 `prompt` 函式覆寫「保留使用者原本設定、包一層再呼叫」是同一個模式。
- PSReadLine 統一處理輸入緩衝區的按鍵，不分這個 Enter 是本機鍵盤敲的還是遠端寫進
  PTY 的位元組——遠端指令一樣會觸發這個覆寫，達到跟 zsh/bash `preexec` 等效的效果。
- `cmd.exe` 沒有 PSReadLine、沒有任何 hook 機制，維持現狀不處理——這是它現有整合本
  來就比 PowerShell/zsh/bash 弱的已知落差，不算本次功能倒退。

## 對既有程式碼的影響

- `useTerminalBlocks.ts`：新增 `lastPromptSnapshotRef`、`recoverUntrackedCommand()`；
  `A` 分支新增快照邏輯；`C` 分支的「沒有本機追蹤區塊」情況改成先嘗試還原、失敗才呼叫
  `onUntrackedCommandBoundary("start")`。
- `TerminalView.tsx`：`onUntrackedCommandBoundary` 相關程式碼（`untrackedCommandBoundaryRef`
  與它在 `liveRows` 撐高/收回的邏輯）**保留、不刪除**，但註解要更新，說明它現在的定位
  是「文字還原失敗時的保底」而非主要機制。
- `src-tauri/src/pty/shell.rs`：`inject_powershell_integration` 新增 Enter 鍵覆寫。

## 測試策略

### 單元測試（`useTerminalBlocks.test.ts`）

沿用現有測試檔已經有的「組出真的 OSC 133 位元組序列餵給真的 xterm.js 實例」模式，新增：

1. 沒有本機追蹤區塊時，手動把 `A` 對應的提示字元文字、`C` 前的指令文字依序 `write`
   進 term，觸發 `C`，驗證 `beginTrackedBlock` 被呼叫、且傳入的文字跟實際打的一致。
2. 換行情境：`write` 一個會自動換行成兩行以上的長指令，驗證能正確併回完整指令文字。
3. 還原失敗情境（例如故意不送 `A`），驗證退回呼叫 `onUntrackedCommandBoundary("start")`
   而不是硬湊一個錯誤的 `beginTrackedBlock` 呼叫。
4. 本機自己有追蹤區塊時（`submitCommand` 已呼叫過），驗證 `recoverUntrackedCommand`
   完全不會被觸發，不影響現有行為。

### 無法只靠單元測試涵蓋的部分（需要真機驗證）

- macOS zsh、bash 的實際 `preexec` 回顯時序是否真的如預期（上一行是否完整保留指令
  文字）。
- Windows PowerShell 的 `Set-PSReadLineKeyHandler` 覆寫是否會跟常見的 PowerShell
  設定檔（例如已經自訂 Enter 鍵、或裝了 PSReadLine 相關外掛）互相干擾。
- 兩台機器的完整流程：遠端觀看者取得控制權 → 送出指令 → 本機端出現文字正確、高度
  正確的卡片。

## 範圍界定

- **支援**：zsh、bash、PowerShell 三種 shell 的遠端指令文字還原。
- **不支援**（維持現狀，非本次倒退）：`cmd.exe`（沒有 hook 機制）。
- **不受影響**：TUI/全螢幕互動內容（走既有 `isAlternateBuffer` 邏輯）、本機自己送出
  的指令（不受這條新路徑影響）。
- **不處理**：多行貼上/heredoc 等本機既有機制本身就沒有完美處理的情境，遠端指令的
  還原邏輯不要求超越本機既有能力範圍。
