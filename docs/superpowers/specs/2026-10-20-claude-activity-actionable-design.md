# Claude 協作圖：通知、token 排行、主 Claude 活動

使用者選了三項讓協作圖「有事才打擾、看得出哪裡浪費、看得到主 Claude 在做什麼」：

## 1. 通知（前端）
- 事件：**失敗**（節點 running→failed）、**全部完成**（本輪曾有 running，現在 0 個 running，且至少一個節點）、**卡住**（running 節點超過 3 分鐘沒有任何事件，每個節點只提醒一次）。
- 「卡住」靠新欄位 `Node.last_event_ms`（該 agent 的任何 hook 事件都更新），前端每 30 秒（有 running 時）以當下時間比對。
- 只在「使用者沒在看」時發：分頁非作用中，或視窗沒有焦點。權限走 `ensureNotificationPermission`。
- 設定：「Claude 協作圖通知」開關（localStorage，預設開），在設定→一般的同一區塊。
- 切到另一輪或收合面板不影響通知（掛在 TerminalView，分頁一直掛載）。

## 2. token 排行（前端，純函式）
- 面板標頭顯示本輪 token 合計。
- 本輪有 ≥2 個節點有 token 時，token 最多的節點卡片出現「最耗」徽章；詳情顯示佔本輪百分比。

## 3. 主 Claude 活動（後端＋前端）
- Snapshot 新增 `main_tool_count`（本輪主 agent 工具呼叫次數）與 `main_recent`（最近 10 筆工具提示，新的在後）。Prompt 與 SessionEnd 重置。Agent 工具本身也算（它是主 Claude 的動作）。
- 點根卡片：詳情顯示「本輪 N 次工具呼叫」與最近工具清單；根卡片顯示次數。

## 不做
歷史保存、點節點看完整 transcript（之後再議）。
