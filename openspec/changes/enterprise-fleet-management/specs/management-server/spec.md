## ADDED Requirements

### Requirement: AITERM 實例登錄

Management Server SHALL 提供裝置登錄端點，允許 AITERM 實例以 device token 形式加入機隊。每個實例 SHALL 擁有唯一的 `device_id`（UUID v4），並記錄以下元資料：名稱、類型（Interactive / Headless Worker）、角色（Dev / DBA / Ops / QA）、安裝平台（macOS / Windows / Linux）。

#### Scenario: 新 AITERM 首次登錄

- **WHEN** AITERM 以 org token 和裝置資訊 POST `/api/devices/register`
- **THEN** Server 回傳唯一 `device_token`，裝置狀態設為 `pending_approval`

#### Scenario: 管理者核准裝置

- **WHEN** 管理者在 Dashboard 核准 `pending_approval` 的裝置
- **THEN** 裝置狀態變為 `active`，後續 heartbeat 正常處理

#### Scenario: 未授權裝置嘗試 heartbeat

- **WHEN** `device_token` 不存在或已撤銷的裝置送出 heartbeat
- **THEN** Server 回傳 `401 Unauthorized`，AITERM 停止 polling 並通知使用者

---

### Requirement: 心跳與狀態追蹤

Management Server SHALL 每 30 秒接受 AITERM 的 heartbeat，更新裝置的最後活躍時間與當前狀態（`idle` / `busy` / `offline`）。超過 90 秒未收到 heartbeat 的裝置 SHALL 自動標記為 `offline`。

#### Scenario: 正常 heartbeat

- **WHEN** AITERM 送出 `POST /api/devices/{id}/heartbeat`（含當前 CPU、任務狀態）
- **THEN** Server 更新 `last_seen` 並回傳待執行 tasks 與 policy 更新

#### Scenario: 裝置超時離線

- **WHEN** 裝置最後 heartbeat 超過 90 秒前
- **THEN** Server 將裝置標記為 `offline`，Dashboard 顯示離線警示

---

### Requirement: Policy Engine

Management Server SHALL 維護每個裝置的政策設定（AI Provider、ExecutionMode、VCS 分支權限），並在每次 heartbeat 回應中攜帶最新 policy 版本號。AITERM 發現版本號更新時 SHALL 套用新 policy，且 Server 下推的 policy SHALL 覆蓋本地設定。

#### Scenario: 管理者更新角色 ExecutionMode

- **WHEN** 管理者將 Dev 角色的 ExecutionMode 改為 `AlwaysConfirm`
- **THEN** 下一次 Dev 角色裝置的 heartbeat 回應帶有新 policy，AITERM 套用後本地的 `FullAuto` 設定被覆蓋

---

### Requirement: 稽核 Log 儲存

Management Server SHALL 接受 AITERM 回報的活動紀錄（指令歷史、AI 對話、Skill 執行），並以 `org_id + device_id + timestamp` 索引儲存。敏感資料 SHALL 於 AITERM 端過濾後才送出，Server 不做二次過濾。管理者 SHALL 可依時間範圍、裝置、使用者搜尋稽核紀錄。

#### Scenario: 指令歷史回報

- **WHEN** AITERM 執行一條 terminal 指令後 POST `/api/activity/commands`
- **THEN** Server 儲存指令文字（已過濾敏感資料）、執行時間、exitCode、所屬 task_id（若有）

#### Scenario: AI 對話回報

- **WHEN** AITERM AI 對話結束後 POST `/api/activity/ai-conversations`
- **THEN** Server 儲存使用者 prompt、AI 回應、使用的 provider、token 用量

---

### Requirement: 多租戶資料隔離（SaaS 模式）

SaaS 部署模式下，Management Server SHALL 確保每個 Org 的所有資料（裝置、任務、稽核 Log、Skill、VCS Token）嚴格隔離，任何 API 端點均 SHALL 驗證請求的 `org_id` 與認證 token 的歸屬一致。

#### Scenario: 跨 Org 資料存取嘗試

- **WHEN** Org A 的 token 嘗試存取 Org B 的裝置列表
- **THEN** Server 回傳 `403 Forbidden`，不洩漏 Org B 的任何資訊
