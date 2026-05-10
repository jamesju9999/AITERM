## ADDED Requirements

### Requirement: Headless Worker 啟動模式

AITERM SHALL 支援以 `--headless` flag 啟動的 CLI 模式，此模式下不載入 Tauri GUI，以純背景程序運行。Headless Worker SHALL 可安裝為系統服務（Linux: systemd、macOS: launchd、Windows: Windows Service），開機自動啟動。啟動時 SHALL 讀取設定檔中的 `enterprise_server_url` 與 `enterprise_device_token`，並開始 heartbeat polling。

#### Scenario: Headless Worker 啟動並登錄

- **WHEN** 執行 `aiterm --headless` 且設定檔含有效的 `enterprise_device_token`
- **THEN** Worker 開始 heartbeat，Management Server 顯示裝置為 `active`（Headless 類型）

#### Scenario: 系統服務重啟後自動恢復

- **WHEN** 系統重啟後 systemd 啟動 aiterm-worker 服務
- **THEN** Worker 自動恢復 heartbeat，未完成的任務重新拉取並繼續執行

---

### Requirement: Headless Worker 自動接受任務

Headless Worker SHALL 在 heartbeat 回應中收到 Task Packet 時，自動開始執行（無需人工確認）。Worker SHALL 支援同時執行多個 Task（預設上限為 2，可由管理者設定），各 Task 在獨立的 PTY session 中隔離執行。

#### Scenario: Worker 自動接受並執行任務

- **WHEN** heartbeat 回應包含新 Task Packet 且 Worker 當前任務數低於上限
- **THEN** Worker 立即開始執行任務，向 Server 回報狀態 `running`

#### Scenario: Worker 達到任務數上限

- **WHEN** heartbeat 回應包含新 Task 但 Worker 已達並發上限
- **THEN** Worker 回報自身狀態為 `busy`，Scheduler 將 Task 分派給其他可用 Worker 或排隊

---

### Requirement: 危險操作遠端確認

Headless Worker 執行中遇到 AI 建議 `risk_level: dangerous` 的指令時，SHALL 不自動執行，而是暫停當前 Task 並透過 heartbeat 向 Management Server 回報 `awaiting_approval`（含危險指令內容）。管理者在 Dashboard 確認後，Worker 才繼續執行；管理者拒絕則 Task 標記為 `failed`。

#### Scenario: 危險指令等待管理者確認

- **WHEN** AI 建議執行 `DROP TABLE users`（risk_level: dangerous）
- **THEN** Worker 暫停，Server 通知管理者，Dashboard 顯示待確認的危險指令

#### Scenario: 管理者拒絕危險指令

- **WHEN** 管理者在 Dashboard 點擊「拒絕」
- **THEN** Worker 收到拒絕回應，Task 標記為 `failed`（reason: `dangerous_command_rejected`）

---

### Requirement: Headless Worker 操作 Log

Headless Worker 的所有操作（任務接收、Agent Loop 步驟、git 操作、錯誤）SHALL 寫入本地 log 檔（rotating，保留最近 7 天），同時透過 Activity Reporter 批次上傳至 Management Server。管理者 SHALL 可在 Dashboard 的裝置詳情頁查看 Worker 的 Log 串流。

#### Scenario: 管理者查看 Worker Log

- **WHEN** 管理者在 Dashboard 點擊某 Headless Worker 的「查看 Log」
- **THEN** Dashboard 顯示最近 100 行 log，並以 SSE 持續串流新 log 項目
