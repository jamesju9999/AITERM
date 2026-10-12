# 對話（MCP）模式的「執行終端機指令」工具

日期：2026-10-13（整合 MCP 與 Agent 的第 1 步）

## 問題

Ask AI 有兩套迴圈：Agent 模式（⚡）自動執行 `<cmd>` 指令但不會呼叫 MCP 工具；對話模式會呼叫 MCP 工具，但模型要跑 shell 指令時只能寫出 `<cmd>` 區塊，等使用者點 ▶。模型沒有「執行指令」的工具，就在對話裡寫指令（例如 `open -a Safari …`）。

## 做法

在對話的工具迴圈裡加一個內建工具 `aiterm_terminal__run_command(command)`，內部走和 Agent 模式同一條執行路徑（`onExecuteCommand` → 終端機分頁 → 完成回呼），不重寫。

## 安全：沿用「執行模式」與指令風險分級

Agent 模式目前不經確認就執行；這個新工具不能比它鬆，所以多一道閘門（`terminal_command_gate`，在 Rust）：

| 風險（`CommandGuard::classify`） | always-confirm | graded | full-auto |
|---|---|---|---|
| Safe | 問 | 直接跑 | 直接跑 |
| NeedsConfirm | 問 | 問 | 直接跑 |
| Dangerous | 問 | 問 | 問 |
| Blocked | 拒絕（不問） | 拒絕 | 拒絕 |

「問」用既有的授權對話框（新增 `command` 種類：顯示指令與風險原因，只有「拒絕」與「允許這一次」，沒有「永遠允許」），60 秒沒回應視為拒絕。被拒絕或被擋下時，工具回傳 `is_error` 的原因文字給模型。

## 執行

前端攔截這個工具名：先過閘門，再用 AiPanel 的 `runCommand`（與 Agent 迴圈相同：`repairUnterminatedHeredocs`、`onExecuteCommand`、`getPtyRecentOutput`、`describeExit`、輸出取末 2000 字）執行，結果以文字回給模型。Windows 提權（exit 740）等既有行為都在 `onExecuteCommand` 之後，原樣沿用。

按停止時，若有指令正在跑，送 Ctrl+C 讓 shell 回到提示字元（與 Agent 的中止一致）。

## 提示詞

有這個工具時附加：要「做」事就呼叫它，不要寫 `<cmd>` 區塊讓使用者點；`<cmd>` 只用來示範不該自動執行的指令。一次一個指令。

## 不做（後續步驟）

統一 ⚡ 開關與步數上限、合併 Agent 迴圈（第 2、3 步）。
