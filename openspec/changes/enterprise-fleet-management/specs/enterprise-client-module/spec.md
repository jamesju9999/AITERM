## ADDED Requirements

### Requirement: 企業模式啟用

AITERM Client SHALL 在 `AppConfig.enterprise_server_url` 非空時自動進入企業模式。企業模式啟用後，Server 下推的 policy（AI Provider、ExecutionMode、VCS 權限）SHALL 覆蓋所有本地設定，Settings 頁面的受控欄位 SHALL 顯示為唯讀並標示「由管理者設定」。

#### Scenario: 首次設定企業伺服器 URL

- **WHEN** 使用者在 Settings 輸入 `enterprise_server_url` 並完成登錄流程
- **THEN** AITERM 取得 `device_token`，開始每 30 秒 heartbeat，Settings 受控欄位變為唯讀

#### Scenario: 移除企業伺服器 URL

- **WHEN** 使用者清除 `enterprise_server_url`（需管理者授權或 token 已撤銷）
- **THEN** AITERM 回到個人模式，本地設定恢復可編輯

---

### Requirement: Enterprise Agent（心跳與任務接收）

AITERM SHALL 維護一個背景 Enterprise Agent，每 30 秒向 Management Server 送出 heartbeat（含裝置狀態、當前任務狀態、系統資源概況）。heartbeat 回應 SHALL 被解析為：待執行 Task Packet、Policy 更新、Skill 更新清單。

#### Scenario: heartbeat 回應帶有新任務

- **WHEN** heartbeat 回應包含新的 Task Packet
- **THEN** Interactive 模式顯示任務通知讓使用者確認；Headless Worker 自動開始執行

#### Scenario: heartbeat 失敗（網路中斷）

- **WHEN** heartbeat 連續 3 次失敗
- **THEN** AITERM 進入離線模式，顯示「與管理伺服器失去連線」警示；已接收的 Task 仍可繼續執行

---

### Requirement: Task Runner

AITERM SHALL 在接受任務後，依照 Task Packet 執行以下流程：使用 VCS Credential Manager 取得 Scoped Token → clone/fetch 目標 repo → 切換至 `work_branch` → 讀取 `spec_path` 的 OpenSpec 規格 → 觸發 `/opsx:apply` Agent Loop → 完成後執行 `on_complete` 動作（如開 PR）→ 向 Server 回報完成狀態。

#### Scenario: Task 成功完成

- **WHEN** Agent Loop 完成所有實作步驟並成功 push
- **THEN** Task Runner 執行 `on_complete`（例如開 PR），向 Server POST `/api/tasks/{id}/complete`，釋放 Scoped Token

#### Scenario: Task 超過 max_steps

- **WHEN** Agent Loop 步驟數達到 Task Packet 的 `max_steps` 上限
- **THEN** Task Runner 向 Server 回報 `failed`（reason: `max_steps_exceeded`），AITERM 顯示通知

---

### Requirement: Activity Reporter（敏感資料過濾）

AITERM SHALL 在回報指令歷史與 AI 對話前，於本地執行敏感資料過濾：以 regex pattern 偵測並替換密碼（`password=`、`--password`）、API key（`Bearer `、`api_key=`）、環境變數指派（`export FOO=`、`set FOO=`）中的值為 `[REDACTED]`。過濾後的資料 SHALL 批次（每分鐘或每 10 筆）POST 至 Management Server。

#### Scenario: 含密碼的指令被過濾

- **WHEN** 使用者執行 `mysql -u root -p secret123`
- **THEN** Activity Reporter 過濾後回報 `mysql -u root -p [REDACTED]`，原始值不離開 AITERM 機器

#### Scenario: AI 對話含 API Key

- **WHEN** AI 對話包含 `Authorization: Bearer sk-abc123`
- **THEN** 回報內容為 `Authorization: Bearer [REDACTED]`

---

### Requirement: Interactive 模式任務通知

Interactive AITERM 收到新任務時 SHALL 顯示非阻斷式通知面板，包含任務標題、指派者、預計步驟數。使用者 SHALL 可選擇「立即執行」（背景執行，UI 顯示進度）、「延後」（排入待辦）或「拒絕」（回報 Server）。任務執行時 SHALL 在 UI 中顯示獨立的任務進度面板，使用者可監看 AI 操作步驟，但不需要介入。

#### Scenario: 使用者接受並背景執行任務

- **WHEN** 使用者點擊「立即執行」
- **THEN** 任務在背景執行，UI 顯示進度面板，使用者可繼續操作其他 tab

#### Scenario: 使用者延後任務

- **WHEN** 使用者點擊「延後」
- **THEN** 任務存入待辦清單，heartbeat 繼續回報 `queued` 狀態，管理者可見
