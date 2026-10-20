# Claude 協作圖：工具呼叫節點

只有一個 claude（沒有 subagent）時，圖只有根卡片。把每次工具呼叫畫成節點，讓單一 claude 也有流程可看。

## 資料（後端 `Tree`）
- `Snapshot.tools: Vec<ToolCall>`；`ToolCall {id(tool_use_id), parent(呼叫者 agent_id，主 Claude 為 null), tool, hint, category, status, started_ms, ended_ms, turn}`。
- 來源：`PreToolUse`（非 Agent/Task、非 SubagentHandback、有 tool_use_id）→ running；`PostToolUse` → done；新 hook `PostToolUseFailure`（kind `post-fail`）→ failed。重複的 Pre 不重複加。
- `category`：`file`（Read/Edit/Write/NotebookEdit/Glob/Grep）、`shell`（Bash）、`web`（WebFetch/WebSearch）、`database`（Bash 指令以 psql/mysql/sqlite3/mongosh/redis-cli 開頭，或 MCP 工具名含 sql/postgres/mysql/sqlite/mongo/redis/database/supabase/dynamodb）、`mcp`（其他 `mcp__*`）、`other`。
- MCP 提示改成 `server: tool`（不顯示 `mcp__` 前綴）。
- 新一輪提問時，上一輪仍 running 的工具標為 interrupted（Esc 中斷不會有 Post）。`SessionEnd` 清空。最多保留 300 筆，超過丟最舊的已結束者。

## 畫面（前端）
- 工具節點是圖上的葉節點，掛在呼叫者底下；沿用現有版面、箭頭、移動小點、路徑高亮、自動捲動。
- 每個呼叫者只顯示最近 6 個工具，更早的收成一張虛線「更早 N 個工具」卡；點它展開全部。
- 卡片：類別圖示與顏色＋提示文字＋狀態＋耗時；詳情顯示類別、完整提示、狀態、耗時。
- 工具列新增「工具」開關（預設開，記在 localStorage）；關掉就回到只有 subagent。
- subagent 被折疊進「已完成 N 個」時，它的工具節點一併隱藏。時間軸仍只列 subagent。
- 面板顯示條件不變（有 subagent 或主 Claude 用過工具）。

## 不做
工具的輸入/輸出全文、資料庫連線本身當節點（只用類別標示）。
