# Agent 協作：App 重啟後接著做

日期：2026-10-17
狀態：使用者同意（「依你的建議做直到功能完成為止」），直接實作

## 問題

App 重新啟動（dev App 重編、關掉 App、當機）時，所有「執行中」的卡片被標成
`cancelled`（「app 重啟，工作已中斷」），對應的 Agent 被暫停。做到一半的工作作廢，
使用者要手動重試、重新花額度。這幾天實測發生了好幾次。

## 作法

### 啟動時（取代原本的 recover）

對每張還停在 `running` 的卡片：

| 卡片 | 處理 |
|---|---|
| Agent 卡、非互動、有 `session_id`、`~/.claude/projects` 找得到 `<session_id>.jsonl` | 回到 `queued`、`resume_session = 1`、排在該 Agent 佇列最前面；Agent **不暫停** |
| Agent 卡、非互動，但找不到對話記錄（claude 根本沒啟動起來） | 回到 `queued`、從頭開始（什麼都還沒做，重來不會浪費）；Agent 不暫停 |
| 其他（互動卡、看板一般卡片） | 照舊：cancelled＋暫停 Agent |

同時清掉 `reply_wait_at`、`quota_wait_at`（重啟後重新偵測）。

### 派工時

`resume_session = 1` 的卡片：
- 指令用 `claude --resume <原 session_id>` 取代 `--session-id <新 id>`；其他旗標（額度 hook、MCP、模型）照舊。
  實測（claude 2.1.300）：`--resume` 記得之前的對話內容，而且繼續寫在**同一個** `<session_id>.jsonl`，
  所以結案時複製對話記錄、算 token 都不用改。
- 在**同一個工作目錄**跑（Agent 的 worktree、審查／驗收目錄的路徑都是固定的）；
  claude 依工作目錄找 session，路徑一樣才接得回去。
- 工程師卡**跳過**開工前的「同步整合結果」「合併前置」與重記 `start_sha`——半成品還沒 commit，
  這些步驟會動到它，`start_sha` 也已經記過了。
- 唯讀卡（審查、拆解、驗收）的目錄照常重建（沒有要保留的東西）；驗收卡重新挑埠並附在提示裡。
- 提示改成「AITerm 剛才重新啟動，你的工作被中斷了。請先檢查目前的進度（看檔案、git status），
  接著把這張卡做完；如果你在等 ask_human 的答案，繼續等。」＋原本的完成標記指示。
- 派出去之後清掉 `resume_session`（再被中斷時會重新判斷）。
- 卡片還有未回答的 ask_human 問題時，登記後立刻恢復「等決策」狀態。

## 不做

- 互動卡、看板一般卡片的續跑（互動卡由使用者操作，突然被送一段提示不妥）。
- 看板一般卡片的 worktree 情境。

## 測試

- `recover_after_restart`：四種卡片各自的結果、排序在佇列最前、Agent 不暫停／暫停、清掉等待欄位。
- `launch_command`：resume 用 `--resume`。
- `resume_prompt` 內容。
- 實機：派一張卡，執行中讓 dev App 重新編譯重啟，確認卡片接著做完、對話記錄是同一份。
