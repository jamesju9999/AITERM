## ADDED Requirements

### Requirement: Skill Registry 集中管理

Management Server SHALL 提供 Skill Registry，允許管理者上傳 Skill 檔案（markdown 格式）並設定版本號、指派對象（全部 / 特定角色 / 特定專案）。每個 Skill SHALL 有唯一的 `skill_id`（kebab-case）與版本號（semver），同一 `skill_id` 保留歷史版本以支援回滾。

#### Scenario: 管理者上傳新 Skill

- **WHEN** 管理者上傳 `company-coding-standards.md`，設定版本 `v1.0`，指派對象「全部」
- **THEN** Skill 進入 `pending_review` 狀態，通知所有具審查權限的管理者

#### Scenario: 查看 Skill 版本歷史

- **WHEN** 管理者點擊某 Skill 的「版本歷史」
- **THEN** Dashboard 列出所有歷史版本，可比較差異並選擇回滾至特定版本

---

### Requirement: 雙人審查機制

所有新上傳或更新的 Skill SHALL 需要第二位具審查權限的管理者批准，才能進入 `approved` 狀態並推送至 AITERM。上傳者本人 SHALL NOT 可審查自己上傳的 Skill。Skill 內容（markdown 全文）SHALL 在審查介面完整顯示。

#### Scenario: 第二位管理者批准 Skill

- **WHEN** 具審查權限的管理者（非上傳者）在 Dashboard 審查並點擊「批准」
- **THEN** Skill 狀態變為 `approved`，可被選擇推送至 AITERM

#### Scenario: 上傳者嘗試審查自己的 Skill

- **WHEN** Skill 上傳者嘗試批准自己上傳的 Skill
- **THEN** 系統回傳錯誤「不可自行審查」，批准操作被拒絕

---

### Requirement: Skill 推送至 AITERM

`approved` 狀態的 Skill SHALL 可由管理者手動觸發推送，或設定為「自動推送」（approved 後即推送）。Server SHALL 在 heartbeat 回應中攜帶 Skill 更新清單（`skill_id`、`version`、`content`、`action: install/remove`）。AITERM 收到後：Headless Worker 自動安裝；Interactive AITERM 顯示通知，於下次 AI session 開始前生效。

#### Scenario: 管理者手動推送 Skill 至 Ops 角色

- **WHEN** 管理者選擇 `deploy-staging v2.0` 並點擊「推送至 Ops 角色」
- **THEN** 所有 Ops 角色 AITERM 的下一次 heartbeat 回應包含此 Skill，自動/通知後安裝

#### Scenario: Interactive AITERM 收到新 Skill 通知

- **WHEN** heartbeat 回應包含新 Skill
- **THEN** AITERM 顯示通知「管理者推送了新 Skill: company-coding-standards v1.2」，使用者可展開查看完整內容

---

### Requirement: 公司 Skill 優先於本地 Skill

當 AITERM 已有同名本地 Skill，Management Server 推送的公司 Skill SHALL 優先生效。Interactive AITERM SHALL 在通知中標示「此 Skill 覆蓋您的本地版本」。管理者移除某 Skill 時，AITERM SHALL 恢復使用本地版本（若存在）。

#### Scenario: 公司 Skill 覆蓋本地同名 Skill

- **WHEN** AITERM 本地有 `opsx:apply v3.0`，Server 推送 `opsx:apply v3.1`
- **THEN** v3.1 生效，v3.0 被備份。管理者移除後 v3.0 自動恢復

---

### Requirement: Skill 安裝稽核 Log

所有 Skill 安裝、更新、移除事件 SHALL 記錄於稽核 Log，含 `skill_id`、`version`、`device_id`、`timestamp`、`action`。管理者 SHALL 可查詢哪些 AITERM 安裝了哪個版本的 Skill。

#### Scenario: 查詢 Skill 安裝狀態

- **WHEN** 管理者查看 `company-coding-standards v1.2` 的部署狀態
- **THEN** Dashboard 顯示各 AITERM 的安裝狀態（installed / pending / failed）
