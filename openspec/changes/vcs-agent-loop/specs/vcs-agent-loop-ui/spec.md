## ADDED Requirements

### Requirement: Step 卡片雙層顯示
每個執行步驟 SHALL 在對話中顯示為卡片，同時呈現兩層資訊：技術指令層（⚙️）與 AI 語意摘要層（💬）。

#### Scenario: 步驟卡片渲染
- **WHEN** agent loop 執行一個步驟並取得結果
- **THEN** 顯示卡片，包含：步驟編號（Step N/M）、⚙️ commandDisplay（操作的技術說明）、💬 aiSummary（AI 一句話）、結果（複用現有 VcsResult 卡片渲染）

#### Scenario: 進行中狀態
- **WHEN** agent loop 正在執行某一步（等待 backend 回應中）
- **THEN** 顯示 loading 狀態卡片：「Step N 執行中…」

### Requirement: 中途打字重導方向
用戶 SHALL 可在 loop 執行的任意時間點輸入新訊息，系統將其注入 history 並在當前步驟完成後繼續。

#### Scenario: loop 執行中輸入
- **WHEN** loop isRunning=true，用戶在輸入框送出訊息
- **THEN** 訊息以 user bubble 顯示於對話中，在當前步驟完成後作為 history entry 注入，AI 下一步規劃時看到此訊息

#### Scenario: 方向調整生效
- **WHEN** 用戶中途輸入後 AI 呼叫 `vcs_agent_step`
- **THEN** AI 的下一步 intent 反映用戶的新方向（例如從 commit 搜尋改為 PR 搜尋）

### Requirement: Loop 結束顯示最終答案
當 `VcsAgentDecision.done = true` 時，系統 SHALL 顯示最終答案 bubble。

#### Scenario: 目標達成
- **WHEN** AI 回傳 `done: true`
- **THEN** 顯示綠色「✅ 目標達成」卡片，內容為 `final_answer` 文字

#### Scenario: 達到步驟上限
- **WHEN** loop 因步驟上限中止
- **THEN** 顯示「⚠️ 已達步驟上限（N 步）」卡片，附上最後一步的結果摘要

### Requirement: 停止 loop
用戶 SHALL 可在 loop 執行中手動停止。

#### Scenario: 手動停止
- **WHEN** loop isRunning=true，用戶點擊「停止」按鈕
- **THEN** loop 在當前步驟完成後停止，不再規劃下一步，顯示「已停止」提示

### Requirement: 新目標重置對話
當 loop 未執行中，用戶送出訊息 SHALL 開始新的 loop。

#### Scenario: 開始新 loop
- **WHEN** isRunning=false，用戶輸入新目標並送出
- **THEN** 清空對話歷史，以新目標開始 loop（step 1 開始）

#### Scenario: 繼續追問
- **WHEN** isRunning=false，用戶輸入追問（如「那個 commit 的作者是誰？」）
- **THEN** 以追問為新目標開始 loop，先前對話歷史清空（loop 是獨立的）
