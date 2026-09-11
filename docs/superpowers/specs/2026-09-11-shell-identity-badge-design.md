# 分頁狀態列顯示 shell 身分，偵測到 Windows PowerShell 5.1 時建議升級

日期：2026-09-11

## 起因

使用者機器上裝了 PowerShell 7.6.6，但 AITerm 行程啟動時的 `PATH` 裡找不到
`pwsh.exe`，於是 `windows_default_shell()`（`crates/aiterm-core/src/pty/shell.rs:26-30`）
退回用 `powershell.exe`，也就是 Windows PowerShell 5.1。

5.1 計算全形字寬度有誤：它把「下午」算成 2 格，終端機實際畫成 4 格。
`dir` 的表格會把每一列補到「畫面寬度 − 1」個字元，含中文的列因此實際佔
136 格、超出 135 格的畫面，多出來的補位空白被推到下一行，看起來就是每列
之間多一行空白。實測數據：`Get-ChildItem | Out-String -Stream` 每列都是
134 字元，畫面寬 135，沒有中文的 `Mode` / `----` 兩列不受影響。

`chcp` 改字碼頁修不了（實測 437 與 950 都一樣）。換成 PowerShell 7 就正常。

問題在於使用者**無從得知跑的不是 7**：AITerm 沒有任何地方顯示實際啟動的
是哪一個 shell。這次花了數小時才從一個診斷指令的輸出發現版本是 5.1。

## 目標

讓使用者在遇到問題時能立刻知道跑的是哪個 shell，並且知道怎麼修。

非目標：平常顯示 shell 版本。使用者明確選擇「只在有問題時出現」，狀態列
已經很擁擠。

## 偵測機制

### 版本從 shell 自己回報

PowerShell 整合腳本（`shell.rs:69-139`）已經在真正的 shell 裡執行，載入時
多送一次自訂 OSC 序列：

```
ESC]7000;shell=PowerShell;edition=Desktop;version=5.1.26100.33158 BEL
```

- `edition` 取自 `$PSVersionTable.PSEdition`，`Desktop` 是 5.1、`Core` 是 7.x。
- `version` 取自 `$PSVersionTable.PSVersion`。

這條路徑回報的必定是**實際在跑的那個 shell**，不是從執行檔名推測的。

cmd.exe 的整合只靠 `PROMPT` 環境變數（`shell.rs:160-180`），送不出這個
序列，那種情況就不顯示徽章——不誤報優先於不漏報。

**已驗證的前提**：xterm.js 的 `OscParser.end()` 在沒有註冊對應處理器時走
`_handlerFb`（`node_modules/@xterm/xterm/src/common/parser/OscParser.ts:145`），
那是個不輸出任何東西的 fallback。序列會被解析器吃掉，不會變成畫面文字。
所以遠端觀看端與任何沒有這個處理器的終端機都不會看到亂碼。

### 前端接收

新增 `src/hooks/useShellIdentity.ts`，註冊 OSC 7000 的處理器並解析出
`{ shell, edition, version }`。跟 `useTerminalBlocks` 註冊 OSC 133 的做法
一致，但獨立成一個 hook——兩者用途不同，不該混在同一個檔案。

### 探測 pwsh.exe 裝在哪

新增 Windows 專用的 Tauri 指令，依序檢查這些路徑是否存在 `pwsh.exe`：

- `%ProgramFiles%\PowerShell\7\pwsh.exe`
- `%ProgramFiles(x86)%\PowerShell\7\pwsh.exe`
- `%LOCALAPPDATA%\Microsoft\WindowsApps\pwsh.exe`（Microsoft Store 版）

回傳找到的路徑，或 `None`。

**只在確定 edition 是 `Desktop` 時才呼叫**——正常情況（7.x、或非 Windows）
完全不會執行這段。

## UI

狀態列在 provider 按鈕左邊出現橘色徽章「PowerShell 5.1」。點擊展開說明
面板，沿用現有 `SharePanel` 的樣式與開關模式。

面板內容依探測結果分兩種：

**找到了 pwsh.exe**

> 這個分頁跑的是 Windows PowerShell 5.1。
> 您的電腦上已經裝了 PowerShell 7（`C:\Program Files\PowerShell\7\pwsh.exe`），
> 但 AITerm 啟動時的 PATH 找不到它。請完全關閉 AITerm 再重新開啟。

**沒找到**

> 這個分頁跑的是 Windows PowerShell 5.1，建議改用 PowerShell 7。
>
> `winget install --id Microsoft.PowerShell`　[複製]

兩種都附一句原因：5.1 計算中文字寬度有誤，`dir` 這類表格輸出的欄位會對不
齊、每列之間多一行空白。

文字放進 `src/lib/i18n.ts`，中英各一份。複製按鈕只把指令放進剪貼簿，不代
使用者執行。

**只做本機分頁**（`TerminalView`）。遠端觀看端看的是別人的機器，「重開
AITerm」這個建議對觀看者沒有意義，`RemoteTerminalView` 不接這個 hook。

## 測試

Rust：

- 整合腳本內容確實包含回報版本的那一行。
- 路徑探測：用 `tempfile` 造出假的 `pwsh.exe`，驗找得到；不造，驗回傳 `None`。

前端：

- OSC 7000 進來後，`useShellIdentity` 解析出正確的三個欄位。
- `edition=Core` 不顯示徽章，`edition=Desktop` 才顯示。
- 沒收到任何 OSC 7000（例如 cmd.exe）不顯示徽章。
- 面板文案依探測結果（有路徑／無路徑）切換。

## 風險與取捨

- **OSC 7000 這個號碼可能跟別人撞號**。iTerm2 用 1337、VS Code 用 633，
  7000 目前沒有已知的使用者。撞號的後果是別的程式送的序列被誤判成 shell
  身分，影響僅止於徽章顯示錯誤，不會影響終端機運作。
- **只認得 PowerShell**。zsh/bash/fish 不回報也不顯示徽章。目前沒有其他
  已知的「shell 版本造成顯示錯誤」案例，先不做。
