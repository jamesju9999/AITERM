# Ask AI 使用電腦操作工具 設計

日期：2026-10-12　前置：`2026-10-12-computer-use-design.md`

## 問題

Ask AI 的內建 MCP client 連不上 AITerm 自己的 MCP server：HTTP 傳輸不帶 `Authorization`、不處理 session，且只保留文字內容。

## 做法

不經 HTTP，把電腦操作工具掛成內建的「虛擬 MCP 伺服器」（id `aiterm-computer`，工具名編碼為 `aiterm_computer__computer_*`）：

- 工具定義與分派抽到 `computer/tools.rs`，MCP server 與 Ask AI 共用同一份參數型別與解析（`ComputerClickArgs::into_parts` 等），不複製。
- 列出：`get_mcp_tools` 與 `ai_chat` 組工具清單時，在真實 MCP 工具後面加上 `builtin_tool_infos(cfg)`；總開關（`mcp_tool_server.computer_enabled`）關閉時一個都不給，模型看不到。
- 執行：`execute_mcp_tool` 先檢查 `builtin_tool_name`，命中就交給 `ComputerService`——授權、危險確認、稽核與外部 MCP 入口完全是同一道關卡，沒有第二條路。
- 結果一律包成 `McpToolResult`：失敗（拒絕、缺權限、參數錯誤）是 `is_error` 加原因文字，讓模型讀得到。
- 前端不需要改：既有的工具迴圈（`useMcpChat`）本來就用 `getMcpTools`／`executeMcpTool`。

## 限制

`computer_screenshot` 的影像在這個對話看不到：回傳尺寸說明並提示改用 `computer_snapshot`。要讓模型看圖需要各 provider 支援 tool-result 圖片，另開一輪。

## 測試

單元：工具清單與編碼名、前綴偵測不誤判、參數驗證、影像換文字、開關關閉時清單為空、`call` 走完整關卡（首次授權寫入設定、拒絕不碰後端、危險點擊仍被問、壞參數不 panic）。
