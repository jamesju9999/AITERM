## ADDED Requirements

### Requirement: 機隊總覽視圖

Admin Dashboard SHALL 提供機隊總覽頁面，顯示所有已登錄 AITERM 的即時狀態（`active` / `busy` / `offline`）、類型（Interactive / Headless Worker）、角色、最後活躍時間、當前執行任務。管理者 SHALL 可從總覽直接進入各 AITERM 的詳情頁（設定、Log、任務歷史）。

#### Scenario: 機隊總覽顯示即時狀態

- **WHEN** 管理者開啟機隊總覽頁面
- **THEN** 頁面以 SSE 持續更新各 AITERM 狀態，離線裝置以灰色標示

---

### Requirement: Mission Builder

Dashboard SHALL 提供 Mission Builder，支援兩種建立方式：（A）輸入自然語言需求，交由 Planning Agent AI 生成 Task DAG；（B）上傳 OpenSpec 規格（tasks.md），解析後手動指派 AITERM。兩種方式均 SHALL 提供 DAG 視覺化預覽，管理者可拖曳調整 Phase 順序、修改 Task 指派對象、新增/移除 Checkpoint，確認後執行。

#### Scenario: 使用 AI 自動規劃 Mission

- **WHEN** 管理者輸入需求文字、選擇目標 Git repo 與可用 AITERM，點擊「AI 規劃」
- **THEN** Planning Agent 生成 Task DAG 預覽，管理者可編輯後確認執行

#### Scenario: 上傳 OpenSpec 建立 Mission

- **WHEN** 管理者上傳 tasks.md 並指派各 Task 給對應 AITERM
- **THEN** Dashboard 展示任務清單與依賴關係，確認後執行

---

### Requirement: Mission 監控視圖

管理者 SHALL 可查看進行中 Mission 的 DAG 進度視圖，各 Task 節點顯示狀態（queued / running / done / failed）與執行 AITERM。`awaiting_approval`（Checkpoint 或危險指令）狀態的節點 SHALL 以醒目標示，並提供「核准」/「拒絕」按鈕。完成的 Task SHALL 顯示 Git commit hash 或 PR 連結。

#### Scenario: 危險指令待確認顯示

- **WHEN** Headless Worker 遇到危險指令並暫停
- **THEN** DAG 視圖中對應 Task 節點顯示警告圖示，展開後顯示危險指令內容與「核准執行」/「拒絕」按鈕

---

### Requirement: 稽核 Log 瀏覽器

Dashboard SHALL 提供稽核 Log 瀏覽器，支援依裝置、使用者、時間範圍、關鍵字搜尋指令歷史與 AI 對話。每筆指令歷史 SHALL 顯示：執行時間、指令文字（已過濾敏感資料）、exitCode、所屬 Mission/Task（若有）。AI 對話 SHALL 可展開查看完整 prompt 與回應。

#### Scenario: 搜尋特定使用者的指令歷史

- **WHEN** 管理者在稽核 Log 選擇裝置「DEV1」並設定時間範圍為「今日」
- **THEN** 顯示 DEV1 今日所有指令與 AI 對話，按時間倒序排列

---

### Requirement: 全域設定管理

Dashboard SHALL 提供統一設定頁面，管理者可設定：各角色的 AI Provider 指派、ExecutionMode 限制、VCS 憑證（詳見 vcs-vault spec）、Skill Registry（詳見 skill-registry-distribution spec）、Headless Worker 並發任務上限。所有設定變更 SHALL 記錄於稽核 Log。

#### Scenario: 管理者更新角色 AI Provider

- **WHEN** 管理者將 DBA 角色的 AI Provider 從 GPT-4o 改為 Claude Sonnet
- **THEN** 設定即時儲存，下一次 DBA 角色 AITERM heartbeat 收到更新的 policy

---

### Requirement: 管理者帳號管理

Dashboard SHALL 支援管理者帳號的建立、停用、角色指派（`admin` / `reviewer`）。`reviewer` 角色 SHALL 可審查 Skill 但不可修改 Policy。所有帳號操作 SHALL 以 email + 密碼認證，支援 2FA（TOTP）。

#### Scenario: 建立新管理者帳號

- **WHEN** 現有管理者邀請新成員並設定角色為 `reviewer`
- **THEN** 新成員收到邀請 email，完成密碼設定後可登入 Dashboard，僅能進行 Skill 審查操作
