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

## 架構：用 OSC 133 B/C 標記做「絕對位置」擷取

**這一節是核准後修正過的版本。** 原版設計是收到 `A` 時把當下這一行內容存成快照、收到
`C` 時跟快照比對前綴，但深入核對 `preexec`/`precmd`/`prompt` 的實際執行順序後發現：
`A` 是從 hook 裡印出來的，而 hook **在提示字元文字被印出來之前**就執行完了——換句話說
收到 `A` 事件的當下，提示字元文字根本還沒出現在畫面上，讀到的永遠是空行或前一行的
殘留。這個地基是錯的，整段改用下面的方式。

### 現有 OSC 133 標記時序（`src-tauri/src/pty/shell.rs`）

- **zsh/bash**：`preexec` hook 在使用者按下 Enter、shell 即將執行指令**之前**觸發，
  送出 `C`；此時 shell 的行編輯器（zle/readline）已經完成了 Enter 的換行回顯——也就
  是說遊標已經換到新的一行，**上一行仍完整保留著「提示字元 + 剛打完的指令文字」**。
  `precmd` hook 在下一個提示字元畫出來之前觸發，先送 `D`（若有指令剛結束），再送
  `A`；`A` 送出之後 shell 才真正把提示字元文字印出來（`$PS1`/`PROMPT_COMMAND` 都是
  「hook 先跑、提示字元文字後印」的順序）。
- **PowerShell**：目前只有 `prompt` 函式覆寫，在下一個提示字元畫出來時送 `D`+`A`，
  **完全沒有 `C`**（見下方「Windows 擴充」一節，這次要新增）；提示字元文字是這個函式
  的回傳值，一樣是「先送 `D`+`A`、函式回傳後才輪到提示字元文字被印出來」的順序。
- **cmd.exe**：只有靠 `PROMPT` 環境變數送裸的 `D`+`A`，沒有 hook 機制，本次不處理。

### 新增機制：`B` 標記

OSC 133 規格本來就定義了第三種標記 `B`（prompt 結束、即將開始輸入），這個專案目前完全
沒用到。跟 `A`/`C`/`D` 不同，`B` 不是從 hook 印出來的，而是**直接接在提示字元文字的
尾巴**（zsh 接在 `PS1` 尾端、bash 接在 `PS1` 尾端、PowerShell 接在 `prompt` 函式回傳
字串的尾端）——這樣它一定是在提示字元文字真正畫出來**之後**才出現，順序保證正確。

1. **`B` 時記錄絕對位置**：`useTerminalBlocks.ts` 的 OSC 133 handler 新增 `data === "B"`
   分支，把當下的游標位置（絕對行號 `term.buffer.active.cursorY + term.buffer.active.baseY`
   與欄位 `term.buffer.active.cursorX`）存進一個 ref（例如 `promptEndRef`）。因為 `B`
   保證在提示字元文字之後才出現，這個位置就是「輸入從這裡開始」的精確座標——不需要
   再讀取或比對任何文字內容，成本比原本的行文字快照更低。
2. **`C` 時嘗試還原文字**：跟原設計一樣，先檢查 `blocksRef.current` 有沒有一個
   `status === "running"` 的區塊：
   - **有**：本機自己送出的指令，維持現有行為（no-op）。
   - **沒有**：呼叫新函式 `recoverUntrackedCommand()`（見下一節），嘗試從畫面內容還
     原指令文字。
     - **還原成功**：呼叫既有的 `beginTrackedBlock(還原出的文字)`。這一支函式本來就
       只做「區塊記帳」、不會往 PTY 寫任何東西，語意上完全符合「遠端指令的位元組早就
       經由 shell 回顯處理過了，這裡只是要幫它建檔」。之後 `D` 事件會自然把它
       finalize——不需要新增任何 finalize 相關程式碼。
     - **還原失敗**（見下方邊界情況）：退回現有的 `onUntrackedCommandBoundary("start")`
       安全網，維持目前的高度撐高行為，不建立卡片。

這個設計最大的好處：一旦 `beginTrackedBlock` 被呼叫，遠端指令就完全匯入本機既有的
資料流——卡片顯示、`visibleBlockCount` 觸發的即時窗格撐高/縮回、Bookmark/Copy，全部
不需要新寫任何專門給「遠端」的 UI 分支。額外的好處：用絕對位置取代文字前綴比對後，
連「提示字元本身跨好幾行」（例如某些主題的兩行式提示字元）都自然能正確處理，不需要
額外設計。

### zsh/bash：接在 `PS1` 尾端

跟 PowerShell 的 `prompt` 函式不同，zsh/bash 的 `PS1` 可能是靜態字串（設一次就不變），
也可能被 oh-my-zsh/starship 這類框架每次 `precmd` 都重新整段生成。若每次 `precmd`
都無條件把 `B` 標記接到 `PS1` 尾端，靜態 `PS1` 的情況下標記會每次都疊加、無限增長，
所以要先檢查是否已經存在同一份標記才接：

**zsh**（`__aiterm_precmd`，接在既有的 `printf '\x1b]133;A\x07'` 之後）：

```zsh
printf '\x1b]133;A\x07'
local b_marker=$'%{\e]133;B\a%}'
if [[ "$PS1" != *"$b_marker"* ]]; then
  PS1="${PS1}${b_marker}"
fi
```

`%{...%}` 是 zsh 提示字元語法裡的「零寬度」標記，告訴 zsh 這段內容不佔實際欄位（跟
色碼跳脫序列用同一種語法），避免游標定位/自動換行的欄位計算被這段不可見的位元組
干擾。

**bash：獨立函式，接在 `PROMPT_COMMAND` 鏈的最後面（不是 `__aiterm_precmd` 裡）**

跟 zsh 不同，bash 沒有「先跑完使用者/框架的 hook、我們的 hook 保證排最後」這種保證。
現有的 `PROMPT_COMMAND="__aiterm_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"` 是把
`__aiterm_precmd` **插在最前面**——這是刻意的：`local ec=$?` 必須在任何其他指令執行
之前讀走 exit code，晚一步 `$?` 就被覆蓋了，這個順序不能動。但這也代表如果沿用跟
zsh 一樣「在 precmd 裡直接接 `B` 標記」的寫法，我們的 `B` 標記會在 `PROMPT_COMMAND`
鏈**最前面**就被接上——如果使用者裝了 starship 這類每次都整段重新生成 `PS1` 的框架
（它自己的 hook 排在我們的 `__aiterm_precmd` **之後**執行，因為它是在 `.bashrc` 被
`source` 進來時登記的，我們的 `PROMPT_COMMAND` 賦值發生在那之後、把自己插在最前面），
框架事後整段覆蓋 `PS1` 的動作會把我們剛接上去的 `B` 標記直接蓋掉。

解法是把「接 `B` 標記」拆成一個獨立函式，**附加在 `PROMPT_COMMAND` 鏈的最後面**，
保證不管框架怎麼重寫 `PS1`，都是在那之後才補上 `B`。完整的三支函式（含
`__aiterm_in_precmd` guard，見下方說明）：

```bash
__aiterm_cmd_running=0
__aiterm_in_precmd=0

__aiterm_preexec() {
  if [[ $__aiterm_in_precmd -eq 1 ]]; then
    return
  fi
  if [[ $__aiterm_cmd_running -eq 0 ]]; then
    __aiterm_cmd_running=1
    printf '\x1b]133;C\x07'
  fi
}
trap '__aiterm_preexec' DEBUG

__aiterm_precmd() {
  local ec=$?
  __aiterm_in_precmd=1
  if [[ $__aiterm_cmd_running -eq 1 ]]; then
    __aiterm_cmd_running=0
    printf '\x1b]133;D;%s\x07' "$ec"
  fi
  printf '\x1b]133;A\x07'
}

__aiterm_append_b_marker() {
  local b_marker=$'\[\e]133;B\a\]'
  if [[ "$PS1" != *"$b_marker"* ]]; then
    PS1="${PS1}${b_marker}"
  fi
  __aiterm_in_precmd=0
}

PROMPT_COMMAND="__aiterm_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND};__aiterm_append_b_marker"
```

`local ec=$?` 必須維持在 `__aiterm_precmd` 的第一行不變、`__aiterm_in_precmd=1`
一定要放在它**之後**才能設——這裡不是保守起見，是硬性要求：**bash 的純變數賦值
本身就會把 `$?` 重設成 0**（用 `bash -c 'false; echo "$?"; x=1; echo "$?"'` 可以
直接驗證，輸出是 `1` 再 `0`），不是只有呼叫外部指令才會覆蓋它。若順序反過來，
`ec` 會不分青紅皂白永遠是 `0`，每個指令的 `D` 標記都會誤報成功，這個錯誤此文件
先前的版本寫反過，記錄下來避免以後又重蹈覆轍。

`\[...\]` 是 bash/readline 提示字元語法裡的零寬度標記等效寫法（既有的顏色控制碼在
使用者自己的 `PS1` 裡也是這樣包的）。guard 邏輯（先檢查、不存在才接）跟 zsh 版一樣。

**這裡曾經漏掉一個只有真的跑一次互動式 bash 才會現形的 bug，記錄下來**：bash 的
`DEBUG` trap（`__aiterm_preexec` 掛在上面，用來送 `C`）會在 `PROMPT_COMMAND`
裡**每一個**用分號隔開的項目執行前都各自觸發一次，不是只有使用者真正打的指令才會
觸發。把「接 `B` 標記」拆成 `PROMPT_COMMAND` 裡第二個項目後，呼叫
`__aiterm_append_b_marker` 這個動作本身也會讓 `DEBUG` trap 再觸發一次——而這時候
`__aiterm_cmd_running` 才剛被 `__aiterm_precmd` 重設成 `0`（同一輪 `PROMPT_COMMAND`
求值裡，`__aiterm_precmd` 就在它前面剛執行完），於是 `__aiterm_preexec` 原本那個
`if [[ $__aiterm_cmd_running -eq 0 ]]` 的判斷會誤判成「有新指令要執行」，在 `B`
標記都還沒送出、使用者也還沒打任何字之前，就先送出一個假的 `C`。前端
`recoverUntrackedCommand` 會因此在使用者真正打字之前就已經被觸發、用當時（可能是
還沒更新到這一輪、甚至完全是 `null`）的 `promptEndRef` 建出一個內容錯誤的區塊，
而且因為這個假區塊處於 `running` 狀態，等使用者真正送出指令、真正的 `C` 觸發時，
`useTerminalBlocks.ts` 的 OSC handler 會判斷「已經有本機追蹤區塊」而完全跳過還原
——真正的指令反而永遠不會被還原，被那個假區塊卡住。

這個 bug 在原本（Task 2 之前）的單一項目版本裡不會發生：`PROMPT_COMMAND` 只有
`__aiterm_precmd` 一個項目時，沒有第二個「呼叫」的邊界可以讓 `DEBUG` trap 在
`__aiterm_cmd_running` 已經被重設成 `0` 之後又再次觸發。是 Task 2 把它拆成兩個
項目之後才讓這個既有機制的隱性缺陷變成每個 bash 使用者都會百分之百踩到的問題
（原本只有裝了會在 `PROMPT_COMMAND` 裡放多個項目的框架的使用者才可能踩到）。

修法是額外加一個 `__aiterm_in_precmd` 旗標：在 `__aiterm_precmd`（保證是
`PROMPT_COMMAND` 的第一個項目）擷取完 `$?` 之後立刻設成 `1`，在
`__aiterm_append_b_marker`（保證是最後一個項目）結尾設回 `0`；`__aiterm_preexec`
只要看到這個旗標是 `1`，不管 `__aiterm_cmd_running` 是什麼值都直接跳過，等於把整段
`PROMPT_COMMAND` 求值期間（包含中間任何框架自己的項目）都豁免於 `DEBUG` trap 之外
——這同時也把上一段提到的「裝了多項目框架的使用者本來就可能踩到」的既有隱性缺陷
一併修掉，不是額外多做的事，是同一個機制的自然延伸。

**這個 bug 完全無法透過現有的字串比對單元測試發現**——測試檢查的是「產生出來的
腳本內容長什麼樣子」，不是「這份腳本在真正的 shell 裡執行起來的時序」。發現方式是
實際用 Python 的 `pty` 模組開一個真正的互動式 `bash -i --rcfile <產生出來的檔案>`、
送一個指令進去、把原始位元組流連同所有 OSC 133 標記的出現順序印出來比對——跟
`docs/superpowers/plans/2026-08-27-remote-command-text-recovery.md` Task 2 的
程式碼品質審查當時建議「之後應該加一個真的執行產生出來的 rcfile 的整合測試」是
同一個方向，只是這次是先用一次性腳本手動驗證，確認問題存在並且修法有效之後才記錄
下來。

zsh 不需要這種拆分/guard：`add-zsh-hook precmd __aiterm_precmd` 是在使用者的
`.zshrc`（框架的 `add-zsh-hook precmd` 呼叫多半在裡面）被 `source` 進來**之後**
才登記，而 `add-zsh-hook` 依登記順序執行，所以我們的 hook 保證排在框架的 hook
之後執行，`B` 標記接上去時 `PS1` 已經是框架處理過的最終版本，不需要額外拆分；用
同一支 Python 探測腳本實測 zsh 版，`B`/`C` 的順序與位置完全正確，沒有這個問題
（zsh 的 `precmd_functions` 陣列式 hook 呼叫機制跟 bash 的 `PROMPT_COMMAND`
字串式分號串接是不同的機制，不會觸發同一種 `DEBUG` trap 重入問題）。

### `recoverUntrackedCommand()` 演算法

```
輸入：term（Terminal 實例）、promptEndRef.current（{ row, col } 或 null）
輸出：還原出的指令文字，或 null（代表還原失敗）

1. 若 promptEndRef.current 為 null（這個 session 從沒收過一次 B），回傳 null。
2. { row: startRow, col: startCol } = promptEndRef.current。
3. endRow = term.buffer.active.cursorY + term.buffer.active.baseY - 1
   （OSC C 觸發時，遊標已經因為 Enter 換行到新的一行，所以往上一行才是輸入內容
   實際結束的地方）。
4. 若 endRow < startRow，回傳 null（游標比對照的起點還早，代表期間發生過 clear 之類
   讓緩衝區位置對不上的事，安全起見不硬湊）。
5. 依序取 startRow 到 endRow（含頭尾）每一行：
     - 若 term.buffer.active.getLine(該行) 回傳 undefined（該行已被捲出緩衝區），
       回傳 null。
     - 除了第一行（startRow）以外，若該行的 isWrapped 為 false（代表這一行不是
       欄寬自動換行接續上一行、而是中間出現了一次「真正的換行」），回傳 null——
       見下方邊界情況「B/C 之間出現真正的換行」的說明。
     - 第一行（startRow）只取 startCol 之後的部分：
       getLine(startRow).translateToString(true, startCol)
     - 其餘行取整行：getLine(該行).translateToString(true)
   把這些片段依序接起來成 fullLine。
6. 回傳 fullLine.trim()。若結果是空字串，回傳 null（沒有實際指令內容，例如使用者
   直接按了空白 Enter）。
```

`IBufferLine.translateToString(trimRight, startColumn, endColumn)` 是 xterm.js 內建
API，直接支援「只取某欄位之後」的擷取，不需要自己手動切字串。`isWrapped` 同樣是
`IBufferLine` 內建欄位，xterm.js 只在「內容超出欄寬、被迫自動換到下一行」時才會把
它設成 `true`；由實際輸入的 `\r`/`\n` 造成的換行，這個欄位是 `false`。

### 邊界情況

- **這個 session 第一個指令剛好是遠端送的、還沒收過任何 `B`**：`recoverUntrackedCommand`
  回傳 null，退回 `onUntrackedCommandBoundary` 安全網。屬於少見的一次性情況（正常使用
  下，連線建立後本機一定至少已經歷過一次 `B`——只要顯示過一次提示字元就會有）。
- **`endRow < startRow`（例如 `B` 之後、`C` 之前發生了 `clear` 讓緩衝區位置錯位）**：
  視為還原失敗，走安全網。
- **`startRow` 已被捲出緩衝區（scrollback 有限、期間輸出量極大）**：視為還原失敗，
  走安全網。
- **`clear`/`cls`**：`beginTrackedBlock` 內部已經會處理指令字串等於 `clear`/`cls` 的
  情況（清空整個卡片歷史、不建立區塊），還原出來的文字若剛好是這兩個字串，直接沿用
  現有邏輯，不需要額外處理。
- **TUI / 全螢幕互動內容**（vim、htop 等）：這類內容走 `isAlternateBuffer` 判斷的另一
  套高度邏輯，shell 本來就不會在這類程式執行期間送出 OSC 133 標記，不受影響。
- **還原出的文字為空**：視為還原失敗，走安全網，不建立品質可疑的卡片。
- **`B`/`C` 之間出現真正的換行**（例如遠端觀看者用貼上功能，一次貼進去的內容本身
  含有換行、在真正按下送出的 Enter 之前就已經出現在畫面上；或是某些 shell 的續行
  提示字元）：這種情況下，`startRow` 到 `endRow` 之間會有一行不是「欄寬自動換行」
  接續上一行，而是由真正的換行字元造成的全新一行。若不分青紅皂白把這些行接在一起，
  會把兩段語意無關的內容黏成一串看似合理、實則錯誤的指令文字——比直接還原失敗、
  走安全網還要糟（安全網不會建立錯誤資料，硬接出來的錯字串卻會被當成「還原成功」
  顯示給使用者）。用 `isWrapped` 欄位分辨：只要 `startRow` 之後的任何一行不是
  自動換行接續（`isWrapped === false`），就視為還原失敗。

## Windows（PowerShell）擴充

PowerShell 需要兩處新增：`C` 標記（目前完全沒有）、`B` 標記（接在提示字元文字尾端）。

### `C`：`Set-PSReadLineKeyHandler` 覆寫 Enter 鍵

```powershell
$global:__aiterm_orig_enter_handler = (Get-PSReadLineKeyHandler -Bound |
    Where-Object { $_.Key -eq "Enter" }).Function

Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
    param($key, $arg)
    try {
        if ($global:__aiterm_orig_enter_handler -and
            $global:__aiterm_orig_enter_handler -ne "AcceptLine") {
            [Microsoft.PowerShell.PSConsoleReadLine]::($global:__aiterm_orig_enter_handler)($key, $arg)
        } else {
            [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine($key, $arg)
        }
    } catch {
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine($key, $arg)
    }
    [Console]::Write("$([char]27)]133;C$([char]7)")
}
```

- **這裡曾經寫錯過一次，記錄下來避免重蹈覆轍**：第一版寫成
  `Get-PSReadLineKeyHandler -Chord Enter -Bound`，但 `-Chord`（別名 `-Key`）只是
  `Set-PSReadLineKeyHandler`/`Remove-PSReadLineKeyHandler` 的參數，`Get-PSReadLineKeyHandler`
  只接受 `-Bound`/`-Unbound`，不接受 `-Chord`——這是「參數綁定失敗」，屬於敘述層級
  的中止錯誤，不會被 `-ErrorAction SilentlyContinue` 吞掉，每次開新分頁都會在使用者
  面前印出一個紅字錯誤，而且 `$global:__aiterm_orig_enter_handler` 會永遠是
  `$null`，靜默丟棄使用者原本的 Enter 綁定（例如 vi 模式的 `ViAcceptLine`）。改成
  `Get-PSReadLineKeyHandler -Bound | Where-Object { $_.Key -eq "Enter" }` 這個
  標準寫法列出所有已綁定按鍵、篩出 Enter 那一筆。
- **`.Function` 不保證永遠是真的靜態方法名稱**：使用者若把 Enter 綁定成自訂
  `-ScriptBlock`（常見於 predictive completion/自訂 fzf 綁定等設定），
  `.Function` 讀回來的值通常是 `"Unknown"` 這類佔位字串，不是
  `PSConsoleReadLine` 上真的存在的方法——動態呼叫這種值會丟
  `MethodNotFoundException`，讓使用者每按一次 Enter 就當機。用 `try`/`catch`
  包住動態呼叫，抓不到就退回安全的 `AcceptLine`，避免這種情況直接把終端機弄壞。
- **順序很重要，且跟直覺相反**：`C` 是在呼叫 `AcceptLine`（或使用者原本綁定的處理
  方式）**之後**才印，不是之前。`AcceptLine` 會讓 PSReadLine 完成這次輸入、回顯結尾
  的換行，之後游標才會真的換到新的一行——這跟 zsh/bash 的 `preexec`「換行回顯已經
  發生、上一行還留著提示字元+指令」時序一致，讓 `recoverUntrackedCommand()` 的
  `endRow = cursorY - 1` 邏輯不需要為了 PowerShell 另外分支。若反過來先印 `C` 再呼叫
  `AcceptLine`，這時游標還停在原本那一行（換行回顯還沒發生），`cursorY - 1` 會讀到
  錯的一行。這個順序假設**必須在真的 Windows 機器上驗證**（見下方測試策略）。
- 跟現有 `prompt` 函式覆寫「保留使用者原本設定、包一層再呼叫」是同一個模式。
- PSReadLine 統一處理輸入緩衝區的按鍵，不分這個 Enter 是本機鍵盤敲的還是遠端寫進
  PTY 的位元組——遠端指令一樣會觸發這個覆寫，達到跟 zsh/bash `preexec` 等效的效果。
- `cmd.exe` 沒有 PSReadLine、沒有任何 hook 機制，維持現狀不處理——這是它現有整合本
  來就比 PowerShell/zsh/bash 弱的已知落差，不算本次功能倒退。

### `B`：接在 `prompt` 函式回傳字串的尾端

現有 `inject_powershell_integration` 的 `prompt` 函式覆寫（`shell.rs`）目前是呼叫
使用者原本的 `prompt` 函式、讓它的輸出透過「未捕捉的管線輸出自動變成函式回傳值」這個
PowerShell 慣例流出去。要在尾端接上 `B`，得先把這個輸出實際捕捉成變數，才能接字串：

```powershell
function global:prompt {
    $wasSuccess = $?
    $origExit = $global:LASTEXITCODE
    $ec = if ($wasSuccess) { 0 } else { if ($origExit) { $origExit } else { 1 } }

    [Console]::Write("$([char]27)]133;D;$ec$([char]7)")
    [Console]::Write("$([char]27)]133;A$([char]7)")

    $renderedRaw = if ($global:__aiterm_orig_prompt) {
        & $global:__aiterm_orig_prompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }
    # 少見情況：使用者原本的 prompt 函式若不是單一字串、而是多筆管線輸出（例如沒有
    # 用分號/換行抑制的多行輸出），直接字串插值會被 $OFS（預設一個空白）接起來，
    # 跟主控台原本逐行印出的樣子不一樣。用換行接回去，貼近原本會呈現的樣子。
    $rendered = $renderedRaw -join "`n"

    $global:LASTEXITCODE = $origExit
    "$rendered$([char]27)]133;B$([char]7)"
}
```

- 差異只在於原本 `& $global:__aiterm_orig_prompt` 是讓輸出直接流出去，現在改成存進
  `$rendered`，最後一行組合成單一字串，讓它變成這個函式唯一的回傳值——`B` 標記因此
  保證接在提示字元文字的最後一個字元之後，符合「`B` 一定在提示字元畫出來之後才出現」
  的設計前提。
- 若使用者原本的 `prompt` 函式是用 `Write-Host` 直接印字元、不回傳字串（少見但存在
  的寫法），`$rendered` 會是空值，`B` 標記仍然會被印出來，只是接在空字串後面——這種
  情況下 `recoverUntrackedCommand` 讀到的 `startCol` 會落在使用者自訂輸出結束的欄位，
  沿用既有「原本 `prompt` 函式覆寫」機制早已有的同類限制，不是本次新增的落差。

## 對既有程式碼的影響

- `useTerminalBlocks.ts`：新增 `promptEndRef`、`recoverUntrackedCommand()`；OSC 133
  handler 新增 `data === "B"` 分支記錄絕對位置；`C` 分支的「沒有本機追蹤區塊」情況
  改成先嘗試還原、失敗才呼叫 `onUntrackedCommandBoundary("start")`。
- `TerminalView.tsx`：`onUntrackedCommandBoundary` 相關程式碼（`untrackedCommandBoundaryRef`
  與它在 `liveRows` 撐高/收回的邏輯）**保留、不刪除**，但註解要更新，說明它現在的定位
  是「文字還原失敗時的保底」而非主要機制。
- `src-tauri/src/pty/shell.rs`：
  - `inject_shell_integration` 的 zsh 分支在 `precmd` 內容裡新增一段「`PS1` 尾端接
    `B` 標記」邏輯。
  - `inject_shell_integration` 的 bash 分支新增一個獨立的 `__aiterm_append_b_marker`
    函式（**不是**接在 `precmd` 裡），附加在 `PROMPT_COMMAND` 鏈的最後面——理由見
    上面「zsh/bash：接在 PS1 尾端」一節，bash 沒有 zsh 那種「保證排在框架 hook 之後」
    的機制，`B` 標記必須拆成獨立、附加在鏈尾的函式才不會被框架整段覆蓋 `PS1`。
  - `inject_powershell_integration` 新增 `Set-PSReadLineKeyHandler` 覆寫 Enter 鍵
    （送 `C`），並把 `prompt` 函式改成先把提示字元文字存進變數、尾端接上 `B` 標記
    再回傳。

## 測試策略

### 單元測試（`useTerminalBlocks.test.ts`）

沿用現有測試檔已經有的「組出真的 OSC 133 位元組序列餵給真的 xterm.js 實例」模式，新增：

1. **基本還原**：沒有本機追蹤區塊時，依序 `write` 提示字元文字、OSC `B`、指令文字、
   換行（模擬 Enter 的回顯換行）、OSC `C`，驗證 `beginTrackedBlock` 被呼叫、且傳入的
   文字跟實際打的一致。
2. **多行指令**：`write` 一個長度超過 term 欄寬、會自動換行成兩行以上的指令文字，
   驗證能正確用 `startRow`～`endRow` 的範圍併回完整指令文字。
3. **多行提示字元**：`B` 標記出現在提示字元的第二行（模擬兩行式提示字元主題），驗證
   `recoverUntrackedCommand` 依然能用 `promptEndRef` 記住的欄位正確截出指令文字，不
   受提示字元本身佔幾行影響。
4. **空指令**：`B` 之後直接送換行 + `C`（使用者只按了 Enter，沒打任何字），驗證回傳
   null、退回 `onUntrackedCommandBoundary("start")`，不建立空白卡片。
5. **還原失敗情境**：故意不送 `B` 就送 `C`，驗證退回呼叫
   `onUntrackedCommandBoundary("start")` 而不是硬湊一個錯誤的 `beginTrackedBlock`
   呼叫——這正是現有測試「signals onUntrackedCommandBoundary when OSC 133 C/D fire
   with no locally-tracked block」（`useTerminalBlocks.test.ts` 第 521 行）已經涵蓋
   的情境，實作後應該不需要修改這個既有測試就能繼續通過。
6. **本機自己有追蹤區塊時**（`submitCommand` 已呼叫過），驗證 `recoverUntrackedCommand`
   完全不會被觸發，不影響現有行為——對應既有測試「does not signal
   onUntrackedCommandBoundary when a local block already covers the command」
   （第 554 行），同樣應該不需要修改就能繼續通過。

### 無法只靠單元測試涵蓋的部分（需要真機驗證）

- macOS zsh、bash 的實際 `preexec`/`precmd` 回顯時序是否真的如預期（`B` 標記是否真的
  接在提示字元文字最後、`C` 觸發時上一行是否完整保留指令文字）。
- Windows PowerShell 的 `Set-PSReadLineKeyHandler` 覆寫是否會跟常見的 PowerShell
  設定檔（例如已經自訂 Enter 鍵、或裝了 PSReadLine 相關外掛）互相干擾；`prompt` 函式
  尾端接 `B` 是否會跟自訂主題（oh-my-posh 等）的輸出格式衝突。
- 兩台機器的完整流程：遠端觀看者取得控制權 → 送出指令 → 本機端出現文字正確、高度
  正確的卡片。

## 範圍界定

- **支援**：zsh、bash、PowerShell 三種 shell 的遠端指令文字還原。
- **不支援**（維持現狀，非本次倒退）：`cmd.exe`（沒有 hook 機制）。
- **不受影響**：TUI/全螢幕互動內容（走既有 `isAlternateBuffer` 邏輯）、本機自己送出
  的指令（不受這條新路徑影響）。
- **不處理**：多行貼上/heredoc 等本機既有機制本身就沒有完美處理的情境，遠端指令的
  還原邏輯不要求超越本機既有能力範圍。
