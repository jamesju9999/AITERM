# Agent 協作支援 Codex — 設計

日期：2026-10-17
狀態：已與使用者確認方向，待實作

## 目標

Agent 協作裡的每一位 Agent 可以各自選用 **Claude Code** 或 **Codex** 來做事。同一個團隊可以混用（例如 Codex 工程師＋Claude 審核者）。既有團隊行為完全不變（預設 Claude Code）。

範圍只限 Agent 協作的卡片。工作看板的一般卡片、互動模式卡片照舊只走 Claude Code。

## 探勘結論（codex-cli 0.148.0，實測）

| 需要的能力 | Codex 的做法 | 備註 |
|---|---|---|
| 非互動執行 | `codex exec ... "<prompt>"` | 結束時行程退出；stdin 不是 TTY 時會等輸入，派工在 PTY 裡跑沒問題 |
| 寫檔 | `--sandbox workspace-write` | 預設 read-only；worktree 裡 `git commit` 可正常寫 |
| 網路 | `-c sandbox_workspace_write.network_access=true` | 預設 DNS 被擋；打開後 curl 200、可 bind 127.0.0.1 埠 |
| 模型 | `-m <model>` | 供應商沿用使用者 `~/.codex/config.toml` |
| hooks | `-c 'hooks.SessionStart=[{hooks=[{type="command",command="..."}]}]'` ＋ `--dangerously-bypass-hook-trust` | 只支援 command（沒有 http）；每個事件帶 `session_id`、`transcript_path` |
| MCP（HTTP） | `-c mcp_servers.aiterm.url="..."` ＋ `-c mcp_servers.aiterm.default_tools_approval_mode="approve"` | 不設 approve 會被「approval policy is never」拒絕 |
| MCP（stdio） | `-c mcp_servers.playwright.command="npx"` ＋ `args=[...]` | 同上要 approve |
| 接著做 | `codex exec resume <session_id> "<prompt>"` | 寫回同一個 rollout 檔，記得前文 |
| token | rollout JSONL 的 `event_msg`／`token_count`，`info.total_token_usage`（累計） | `input_tokens` 已含 `cached_input_tokens` |
| 額度用完 | exit 1；rollout 最後的 `task_complete.error.codex_error_info == "usage_limit_exceeded"` | **不觸發 Stop hook**；`rate_limits.rate_limit_reached_type` 是 null，不能用 |
| 信任畫面 | `exec` 不出現 | `exec`＋`workspace-write` 會在使用者 config 寫一筆**主程式庫**的 trust（每個專案一筆，不累積） |

## 使用者看到的改變

1. **Agent 設定對話框**多一個「AI 工具：Claude Code／Codex」。選 Codex 時「模型」必填（使用者 config 裡的預設模型不一定存在於供應商上），旁邊提示「填供應商上實際存在的模型名稱」。
2. **Agent 卡片**副標題的模型小標籤前面標出工具（Codex 的卡片顯示 `Codex · <模型>`）。
3. **設定 → 工作看板**多一個「Codex 環境變數」區：多行 `KEY=VALUE`，存在系統鑰匙圈。派工時只帶給 Codex 卡片的終端機（例如本機 oMLX 需要的 `OMLX_API_KEY`）。
4. **團隊範本**記住每位 Agent 的 AI 工具。
5. Codex 卡片撞到額度上限：卡片顯示「等待額度」，每 10 分鐘重試一次（`codex exec resume` 接著做）；成功之前這位 Agent 不派別的卡。

## 架構

### 資料

- `agents` 表加 `cli TEXT NOT NULL DEFAULT 'claude'`（`'claude'`｜`'codex'`）。`AgentRow`、`NewAgent`、`UpdateAgent`、前端 `AgentRow` 都加 `cli`。
- 前端團隊範本 `TeamTemplate.agents[]` 加 `cli?`（舊範本沒有就是 claude）。
- 「Codex 環境變數」：一個 secret（keychain）`task_board.codex_env`，內容是多行 `KEY=VALUE`。設定頁只顯示有沒有設定、不回顯內容（同其他金鑰的做法）。

### 新模組 `tasks/codex.rs`（純函式＋小量 I/O）

負責 Codex 專屬的一切，scheduler 只呼叫它：

- `launch_command(opts: &CodexLaunch) -> String`：組出整行指令。`CodexLaunch` 帶 model、prompt 檔路徑、resume 的 session id、hook 指令、ask_human URL、playwright 參數、要關掉的使用者 MCP 名單。
  - 提示詞寫進 `tasks/<id>/prompt.md`，指令的 prompt 參數只是一句「請讀 <path> 並照裡面的指示完成工作。」——避免多行內容在 zsh／bash／PowerShell 的引號問題。路徑用現有的 `quota::path_flag` 同一套引號規則（有特殊字元就放棄並記 log）。
  - `-c` 的 TOML 值一律由程式組（字串跳脫 `\` 與 `"`），不接受使用者輸入的原始字串；模型名稱沿用 `model_flag` 的白名單字元。
- `hook_command(base_url: &str) -> String`：SessionStart 時把 stdin 的 JSON POST 給 AITerm：`curl -s -m 5 -X POST -H 'Content-Type: application/json' --data-binary @- <base>/codex-session`。Windows 用 `curl.exe`（Windows 10 1803 起內建）。
- `parse_usage(rollout_text) -> TokenUsage`：取最後一筆 `token_count.info.total_token_usage`；`input = input_tokens - cached_input_tokens`、`cache_read = cached_input_tokens`、`cache_write = 0`、`output = output_tokens`（含 reasoning）；`calls` = `token_count` 且 `info` 不是 null 的筆數。
- `last_error(rollout_text) -> Option<CodexError>`：最後一筆 `task_complete` 的 `error`；`codex_error_info == "usage_limit_exceeded"` → `CodexError::UsageLimit(message)`，其他 → `CodexError::Other(message)`。
- `user_mcp_servers(config_toml) -> Vec<String>`：使用者 `~/.codex/config.toml` 的 `[mcp_servers.*]` 名稱，精簡啟動時逐一 `-c mcp_servers.<n>.enabled=false`。

### hook 接收（`tasks/quota.rs`）

現有的 hook 伺服器加一條路徑 `codex-session`：收到 `{session_id, transcript_path}` 就寫進卡片的 `session_id`、`session_path`。登記時機沿用 `on_spawned`（Codex 啟動前就登記）。

### 派工（`scheduler.rs` 的 `RealDispatcher::dispatch`）

在決定 agent 之後分流：`agent.cli == "codex"` 且非互動 → Codex 路徑；其他照舊。

Codex 路徑：
- 不產生 session id（Codex 自己產生，SessionStart 回報）；接著做時用卡片上已存的 `session_id`。
- worktree 準備、角色前綴、驗收埠、ask_human 登記全部沿用。
- 指令交給 `spawn_and_run` 的新變體 `spawn_command`：開分頁、寫入指令、**不等 TUI、不打提示詞、不要完成標記**。
- 環境變數：`create_with_app` 的 env 參數擴充成「額外環境變數」清單，Codex 卡片帶上 Codex 環境變數。
- 監看：`WatchMode::Auto`，完成訊號就是行程結束（OSC 133 結束碼）。等 ask_human 時沿用現有暫停卡住偵測。

### ask_human：Codex 版要「等到答案才回傳」

Claude 版的 ask_human 立刻回「已送出，請結束這一輪」，答案之後**打字送進 claude 的對話**。`codex exec` 沒有對話可以打字，結束這一輪行程就退出了，所以 Codex 卡片改成：

- 登記時標記這張卡「提問要等答案」（`QuotaState::register` 多一個參數）。
- `ask_human` 被呼叫：照舊建立問題、進入等決策、通知；接著**把這個 HTTP 請求掛著**，直到答案來才回傳工具結果「Q? 的回答：…（請依這個答案繼續。）」。卡片結束（解除登記）時等待被丟棄，回傳「問題已取消」。
- 送出答案（`deliver_answer`）：先交給正在等的請求；有人在等就不往分頁打字。Codex 卡片沒人在等時回錯誤「Agent 目前沒有在等這個答案」，不往 shell 打字。
- 指令加 `-c mcp_servers.aiterm.tool_timeout_sec=86400`（等人回答可能很久）。
- Codex 卡片的自主說明改成「用 ask_human 提問，工具會等到有人回答才回傳答案，照答案繼續」。
- App 重開：Codex 卡片沒答的問題直接取消；接著做的提示詞請它需要的話重新提問。

### 結案

在 `watch` 結束、`finalize_*` 之前：Codex 卡片若結果是 Failed（結束碼非 0），讀 `session_path` 的 rollout：
- `UsageLimit` → 卡片回到佇列（`status='queued'`、`resume_session=1`、`quota_wait_at=now`），不走失敗流程、不建後續卡。
- `Other(msg)` → Failed，錯誤訊息換成 Codex 的訊息。
- 讀不到 → 照舊 Failed。

token：`usage::task_usage` 在 Claude 的路徑都找不到時，若卡片的 `session_path` 是 Codex rollout（路徑含 `/.codex/sessions/` 或檔名 `rollout-*.jsonl`），改用 `codex::parse_usage`。結案時一樣存 `usage.json`。

對話記錄：Codex 卡片不複製 Claude 格式的 session.jsonl，退回終端機擷取（現有 fallback）。

### 排程

`drain_once` 篩選佇列時：Agent 的佇列最前面那張卡 `quota_wait_at` 不到 `RETRY_AFTER`（10 分鐘）→ 這位 Agent 這一輪不派工。滿 10 分鐘就照常派出（`resume_session` 讓它 `codex exec resume`）。

### App 重開

沿用「重開後接著做」：Codex 卡片有 `session_id` 就標 `resume_session`；`recover_after_restart` 判斷「找得到對話記錄」改成看卡片的 `session_path` 檔案存在（Codex）或 Claude 的找法（Claude）。

## 錯誤處理

- `curl` 不存在／hook 沒回報：卡片照跑，只是沒有 session id（不能接著做、token 顯示不出來），不影響結果。
- 使用者 config 讀不到：精簡啟動就不關使用者 MCP。
- Codex 沒安裝：分頁裡會顯示 command not found，卡片以非 0 結束碼失敗，錯誤訊息照舊。
- 模型不存在：Codex 自己 exit 1，rollout 的錯誤訊息帶出「Model 'x' not found」。

## 測試

單元測試：
- `codex::launch_command`：一般、接著做、有/沒有 ask_human、驗收卡的 playwright、精簡啟動關使用者 MCP、TOML 字串跳脫、模型名不合法就不加。
- `codex::parse_usage`、`last_error`：用探勘時的真實 rollout 片段當 fixture（額度用完那份、成功那份）。
- `user_mcp_servers`：使用者 config 片段。
- quota hook `codex-session` 寫入 session_id/session_path。
- 結案分類：UsageLimit 回佇列且不建後續卡；Other 失敗並換訊息。
- 排程：佇列頭在等額度的 Agent 不派工，滿 10 分鐘才派。
- agents 表 `cli` 欄位的讀寫、預設 claude。
- 前端：設定對話框選 Codex 時模型必填；卡片標示 Codex；團隊範本存取 cli；設定頁 Codex 環境變數。

實機（macOS）：
- Ask Test 放一位 Codex 工程師（oMLX 的 Qwen3.6-35B-A3B-4bit）＋ Claude 審核者／協調者，跑完一個小模組：派工、commit、審查、合併。
- 額度用完：用使用者額度已用完的 ChatGPT 帳號（gpt-5.6-luna）派一張卡，確認變成「等待額度」而不是失敗。
- App 重開後 Codex 卡片接著做。

Windows／Linux 只有單元測試，未實機。
