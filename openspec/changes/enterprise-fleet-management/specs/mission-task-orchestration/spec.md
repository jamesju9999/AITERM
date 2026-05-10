## ADDED Requirements

### Requirement: Mission 建立（AI 自動規劃）

Management Server SHALL 提供 Planning Agent，接受管理者的自然語言需求描述，自動產生 OpenSpec 規格（proposal.md + design.md + tasks.md），並將規格 commit 進目標 Git repo 的 `docs/openspec/missions/{mission-id}/` 路徑。Planning Agent 生成的 Task DAG SHALL 在管理者確認後才執行，管理者 SHALL 可修改 Phase、Task 指派、依賴關係後再確認。

#### Scenario: 管理者輸入需求，AI 生成 Task DAG

- **WHEN** 管理者在 Mission Builder 輸入「開發 ABC 登入與權限管理功能」並選擇目標 repo 與可用 AITERM
- **THEN** Planning Agent 生成 Task DAG（含 Phase 分組與依賴關係），以 Preview 形式呈現供管理者審查，尚未執行

#### Scenario: 管理者確認並執行 Mission

- **WHEN** 管理者在 Preview 中確認（或調整後確認）Task DAG
- **THEN** OpenSpec 規格 commit 進 Git repo，Task Scheduler 開始按 DAG 分派任務給各 AITERM

---

### Requirement: Mission 建立（手動上傳 OpenSpec）

管理者 SHALL 可直接上傳現有的 OpenSpec 規格檔（tasks.md 至少必須存在），Management Server SHALL 解析 tasks.md 中的任務清單，提供管理者指派各任務給 AITERM 的介面，確認後執行。

#### Scenario: 上傳 OpenSpec tasks.md

- **WHEN** 管理者上傳 tasks.md（含 Phase/Task 結構）
- **THEN** Dashboard 解析並展示任務清單，管理者可為每個 Task 指派目標 AITERM

---

### Requirement: Task DAG 調度與依賴管理

Task Scheduler SHALL 以 DAG 形式管理任務依賴關係。同一 Phase 內無依賴的 Task SHALL 並行分派；後續 Phase 的 Task SHALL 等待所有前置 Phase Task 完成（狀態為 `done`）後才分派。任何 Task 失敗（狀態 `failed`）SHALL 暫停整個 Mission 並通知管理者。

#### Scenario: 並行任務同時分派

- **WHEN** Phase 1 的兩個 Task 均無依賴且各自有可用的 AITERM
- **THEN** 兩個 Task 同時以 Task Packet 形式分派，互不等待

#### Scenario: 任務失敗暫停 Mission

- **WHEN** Phase 2 的某個 Task 回報 `failed`
- **THEN** Mission 狀態變為 `paused`，後續 Phase 的 Task 不分派，管理者收到通知

---

### Requirement: Checkpoint（人工審查節點）

Mission DAG 中 SHALL 可插入 Checkpoint 節點。Checkpoint 前所有 Task 完成後，Mission SHALL 暫停並通知管理者。管理者在 Dashboard 審查（例如查看 PR）並明確確認後，後續 Phase 才繼續執行。

#### Scenario: Checkpoint 觸發

- **WHEN** Checkpoint 前的所有 Task 均完成
- **THEN** Mission 狀態變為 `awaiting_approval`，管理者收到通知，後續 Task 不分派

#### Scenario: 管理者核准 Checkpoint

- **WHEN** 管理者在 Dashboard 點擊「核准繼續」
- **THEN** Mission 恢復執行，後續 Phase 的 Task 開始分派

---

### Requirement: Task Packet 結構

每個分派給 AITERM 的任務 SHALL 以結構化 Task Packet 傳遞，包含：`task_id`、`mission_id`、`title`、`description`、OpenSpec 規格在 Git repo 中的路徑（`spec_path`）、VCS 設定（`repo`、`base_branch`、`work_branch`）、指定 AI Provider、`execution_mode`、`max_steps`、完成後動作（`on_complete`，例如開 PR）。

#### Scenario: AITERM 收到 Task Packet 並執行

- **WHEN** AITERM 的 heartbeat 回應包含新的 Task Packet
- **THEN** AITERM Task Runner clone repo、讀取 spec_path 的規格、啟動 /opsx:apply Agent Loop 開始實作

---

### Requirement: Mission 狀態監控

管理者 SHALL 可在 Dashboard 即時查看 Mission 的整體進度、各 Task 的狀態（`queued` / `running` / `done` / `failed`）、執行中的 AITERM 與已完成的 git commit/PR 連結。

#### Scenario: Task 進度更新

- **WHEN** AITERM 透過 heartbeat 回報任務進度（完成步驟數 / 總步驟數）
- **THEN** Dashboard 的 Task 卡片即時更新進度百分比
