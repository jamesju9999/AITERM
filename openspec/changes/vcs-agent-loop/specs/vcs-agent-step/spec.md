## ADDED Requirements

### Requirement: AI 規劃下一步 VCS 操作
系統 SHALL 提供 `vcs_agent_step` IPC command，接受目標、歷史紀錄、repo info、provider id，回傳 AI 決定的下一步操作或最終答案。

#### Scenario: AI 決定繼續執行
- **WHEN** `vcs_agent_step` 被呼叫且目標尚未達成
- **THEN** 回傳 `VcsAgentDecision { done: false, intent: <VcsIntent>, summary: <一句話> }`

#### Scenario: AI 決定目標達成
- **WHEN** `vcs_agent_step` 被呼叫且 AI 判斷目標已達成
- **THEN** 回傳 `VcsAgentDecision { done: true, final_answer: <摘要>, summary: <一句話> }`

#### Scenario: AI 解析失敗
- **WHEN** AI 回傳無法解析為 `VcsAgentDecision` 的內容
- **THEN** 回傳錯誤，loop 終止並顯示「AI 無法規劃下一步」錯誤訊息

### Requirement: History 包含用戶中途輸入
系統 SHALL 支援在 history 中混合 user message 和 step result，讓 AI 感知用戶中途的方向調整。

#### Scenario: 用戶中途訊息改變方向
- **WHEN** history 中包含 user role 的訊息（在某個 step 之後）
- **THEN** AI 看到該訊息後 SHALL 調整計劃方向，不繼續原有路徑

#### Scenario: 空 history（第一步）
- **WHEN** history 為空（第一次呼叫）
- **THEN** AI 根據 goal 決定第一個 VcsIntent

### Requirement: 選擇 AI provider
系統 SHALL 支援 optional `provider_id` 參數，指定哪個 AI provider 用於規劃決策。

#### Scenario: 指定 provider
- **WHEN** `provider_id` 非空且對應已設定的 provider
- **THEN** `vcs_agent_step` 使用該 provider 進行 AI 推理

#### Scenario: 未指定 provider
- **WHEN** `provider_id` 為 null 或未傳入
- **THEN** 使用系統預設 provider（`default_provider`）

### Requirement: 步驟上限強制中止
系統 SHALL 在 history 中的 step 數量達到 `max_agent_steps` 時，在 frontend 停止呼叫 `vcs_agent_step` 並顯示上限提示。

#### Scenario: 達到步驟上限
- **WHEN** 已執行步驟數 >= `max_agent_steps`（且 max_agent_steps > 0）
- **THEN** loop 終止，顯示「已達步驟上限（N 步），以下是目前結果：」並附上最後一步的結果
