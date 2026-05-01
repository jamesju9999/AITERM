## ADDED Requirements

### Requirement: VCS Chat provider 選擇器
VCS Panel SHALL 提供 provider 選擇器，讓用戶選擇執行 VCS AI 推理的模型，比照 DatabaseAiChat 的實作。

#### Scenario: 選擇 provider
- **WHEN** 用戶在 VCS Panel 的 provider 下拉選單選擇一個 provider
- **THEN** 後續所有 `vcs_agent_step` 呼叫使用該 provider

#### Scenario: 使用預設 provider
- **WHEN** 用戶未選擇 provider（初始狀態）
- **THEN** 使用系統預設 provider（`default_provider`），選擇器顯示「預設」或 default provider 的 display_name

#### Scenario: Provider 列表為空
- **WHEN** 系統未設定任何 provider
- **THEN** 選擇器顯示「未設定 AI」，並提示用戶前往 Settings → AI 供應商 新增
