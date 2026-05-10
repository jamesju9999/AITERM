## ADDED Requirements

### Requirement: VCS 憑證集中儲存

Management Server SHALL 提供 VCS Vault，允許管理者儲存 Git（GitHub App installation token、GitLab PAT）與 SVN 帳號密碼。所有憑證 SHALL 以 AES-256-GCM 加密後儲存於資料庫，明文僅在記憶體中短暫存在。管理者 SHALL 可測試連線、查看已儲存的憑證清單（顯示名稱與類型，不顯示明文），以及刪除憑證。

#### Scenario: 管理者新增 GitHub App Token

- **WHEN** 管理者在 VCS Manager 輸入 GitHub App Installation Token 並儲存
- **THEN** Token 加密後存入 DB，Dashboard 顯示「github-app / 最後更新時間」，不顯示 Token 值

#### Scenario: 測試 VCS 連線

- **WHEN** 管理者點擊「測試連線」
- **THEN** Server 以儲存的憑證嘗試 list repo，回傳成功或具體錯誤訊息

---

### Requirement: 專案 VCS 對應設定

管理者 SHALL 可為每個專案設定 VCS 對應：目標 repo URL、使用哪組 VCS 憑證、Branch 命名策略（work branch pattern、base branch）、各角色的 push 權限範圍。

#### Scenario: 設定專案 Branch 策略

- **WHEN** 管理者設定 ABC 專案的 work branch pattern 為 `feature/{task-id}-{description}`
- **THEN** 分派到 ABC 專案的 Task，其 Task Packet 中的 `work_branch` 自動以此 pattern 生成

---

### Requirement: 短效 Scoped Token 生成與失效

任務分派時，Management Server SHALL 依據 Task 的 VCS 設定，向 VCS Provider 請求短效 Scoped Token（僅限特定 repo 的特定 branch 的 push 權限，有效期為任務預估時長 + 1 小時緩衝）。Token SHALL 隨 Task Packet 加密傳送給 AITERM。任務完成或失敗後，AITERM 回報結果時 Server SHALL 嘗試撤銷 Token，AITERM 本地 SHALL 刪除 Token。AITERM 在 Token 剩餘時效低於 20% 時 SHALL 主動向 Server 請求刷新。

#### Scenario: 任務分派時生成 Scoped Token

- **WHEN** Scheduler 分派 Task 給 AITERM
- **THEN** Server 生成 Scoped Token（限 feature/t-042-login branch）並附於 Task Packet 中

#### Scenario: Token 即將過期，AITERM 請求刷新

- **WHEN** 任務執行中，Token 剩餘時效低於 20%
- **THEN** AITERM POST `/api/tasks/{id}/refresh-token`，Server 回傳新 Token，舊 Token 失效

#### Scenario: 任務完成後 Token 清理

- **WHEN** AITERM 回報任務完成
- **THEN** Server 撤銷對應 Scoped Token，AITERM 刪除本地儲存的 Token

---

### Requirement: VCS 操作稽核

Management Server SHALL 記錄所有透過 Scoped Token 執行的 VCS 操作（clone、fetch、push、PR 建立），含 Task ID、AITERM device_id、操作時間、目標 repo 與 branch。管理者 SHALL 可在稽核 Log 中依 repo 與時間範圍篩選 VCS 操作紀錄。

#### Scenario: 查詢特定 repo 的 push 歷史

- **WHEN** 管理者在稽核 Log 篩選 repo `github.com/company/abc` 的 push 操作
- **THEN** Dashboard 列出所有符合的 push 紀錄，含執行者（device_id）、branch、時間
