## 1. Management Server — 基礎建設

- [x] 1.1 建立 `management-server/` 專案（Rust + axum + SQLx + PostgreSQL）
- [x] 1.2 設計並建立資料庫 schema（devices、orgs、users、tasks、missions、audit_logs、skills、vcs_credentials）
- [x] 1.3 實作裝置登錄端點 `POST /api/devices/register`（回傳 device_token）
- [x] 1.4 實作 heartbeat 端點 `POST /api/devices/{id}/heartbeat`（更新 last_seen、回傳 tasks+policy）
- [x] 1.5 實作裝置離線偵測（90 秒未 heartbeat 自動標記 `offline`）
- [x] 1.6 實作管理者帳號認證（email + 密碼 + TOTP 2FA）
- [x] 1.7 實作多租戶資料隔離 middleware（SaaS 模式 org_id 篩選）
- [x] 1.8 實作 SSE 端點（供 Admin Dashboard 接收即時更新）

## 2. Management Server — Policy Engine

- [x] 2.1 實作 Policy 資料模型（per device/role：AI Provider、ExecutionMode、VCS 權限）
- [x] 2.2 實作 Policy CRUD API（管理者設定、版本號遞增）
- [x] 2.3 在 heartbeat 回應中攜帶 policy 版本號與更新內容
- [x] 2.4 實作角色層級 Policy 繼承（裝置繼承角色預設，可個別覆蓋）

## 3. Management Server — Mission 與 Task 調度

- [x] 3.1 實作 Mission 資料模型（mission、phase、task、dependency 關係、DAG 狀態機）
- [x] 3.2 實作 Planning Agent（呼叫 AI API，輸入需求描述，輸出 OpenSpec 格式 Task DAG）
- [x] 3.3 實作 OpenSpec 規格 commit 至 Git repo（`docs/openspec/missions/{id}/`）
- [x] 3.4 實作 Task Scheduler（DAG 並行分派、依賴等待、Phase 推進邏輯）
- [x] 3.5 實作 Checkpoint 節點（Mission 暫停、通知管理者、等待確認）
- [x] 3.6 實作 Task 狀態更新 API（`POST /api/tasks/{id}/complete`、`/fail`、`/progress`）
- [x] 3.7 實作任務失敗處理（Mission 暫停、重新分配 Headless Worker 任務）
- [x] 3.8 實作危險指令遠端確認 API（Headless Worker 暫停後管理者核准/拒絕）

## 4. Management Server — VCS Vault

- [x] 4.1 實作 VCS 憑證加密儲存（AES-256-GCM，金鑰存 Server 環境變數）
- [x] 4.2 實作 VCS 連線測試端點
- [x] 4.3 實作 GitHub App installation token 的短效 Scoped Token 生成
- [x] 4.4 實作 GitLab PAT Scoped Token 生成（限定 repo + branch 權限）
- [x] 4.5 實作 SVN 憑證傳遞（SVN 無 token 機制，改用加密傳遞帳號密碼）
- [x] 4.6 實作 Token 刷新 API（`POST /api/tasks/{id}/refresh-token`）
- [x] 4.7 實作 Token 撤銷流程（任務完成/失敗後自動撤銷）
- [x] 4.8 實作專案 VCS 對應設定（repo URL、branch 策略、角色 push 權限）

## 5. Management Server — Skill Registry

- [x] 5.1 實作 Skill 資料模型（skill_id、版本、內容、狀態、指派對象）
- [x] 5.2 實作 Skill 上傳與版本管理 API
- [x] 5.3 實作雙人審查流程（待審 → 批准，上傳者不可自審）
- [x] 5.4 實作 Skill 推送邏輯（heartbeat 回應攜帶 skill 更新清單）
- [x] 5.5 實作 Skill 移除與版本回滾 API
- [x] 5.6 實作 Skill 安裝狀態追蹤（各 AITERM 回報安裝結果）

## 6. Management Server — 稽核 Log

- [x] 6.1 實作指令歷史接收端點 `POST /api/activity/commands`
- [x] 6.2 實作 AI 對話接收端點 `POST /api/activity/ai-conversations`
- [x] 6.3 實作 Skill 安裝事件記錄
- [x] 6.4 實作稽核 Log 查詢 API（依裝置、時間範圍、關鍵字搜尋）
- [x] 6.5 實作 Headless Worker Log 串流 API（SSE）

## 7. AITERM Client — 企業模組（AppConfig 擴展）

- [x] 7.1 在 `AppConfig` 新增 `enterprise_server_url`、`enterprise_device_id`、`enterprise_policy` 欄位
- [x] 7.2 實作企業模式啟用邏輯（`enterprise_server_url` 非空時啟動 Enterprise Agent）
- [x] 7.3 實作 Policy 套用（Server policy 覆蓋本地 AI Provider、ExecutionMode 設定）
- [x] 7.4 Settings 頁面受控欄位標示為唯讀（「由管理者設定」）

## 8. AITERM Client — Enterprise Agent

- [x] 8.1 實作 Enterprise Agent 背景任務（每 30 秒 heartbeat，斷線重試）
- [x] 8.2 heartbeat 請求攜帶裝置狀態（idle/busy）、系統資源概況、當前任務進度
- [x] 8.3 heartbeat 回應解析（Task Packet、Policy 更新、Skill 更新清單）
- [x] 8.4 實作 Activity Reporter（敏感資料過濾 regex、批次回報）
- [x] 8.5 實作 Skill Sync（收到 Skill 更新後安裝/移除，Interactive 顯示通知）

## 9. AITERM Client — Task Runner

- [x] 9.1 實作 Task Packet 接收與解析
- [x] 9.2 實作 VCS Credential Manager（儲存短效 Token、到期前主動刷新）
- [x] 9.3 實作 Task 執行流程：clone repo → 切換 branch → 讀取 spec → 觸發 Agent Loop
- [x] 9.4 整合現有 `useAgentMission.ts`（Task Runner 作為觸發入口）
- [x] 9.5 實作 `on_complete` 動作（開 PR 等）
- [x] 9.6 實作任務進度回報（步驟數透過 heartbeat 回傳）

## 10. AITERM Client — Interactive 模式 UI

- [x] 10.1 實作任務通知面板（收到新 Task 時彈出，含立即執行/延後/拒絕）
- [x] 10.2 實作任務進度面板（背景執行時顯示步驟進度）
- [x] 10.3 企業設定頁面（server URL 輸入、登錄流程、裝置資訊顯示）
- [x] 10.4 Skill 安裝通知（含完整 Skill 內容展開）

## 11. Headless Worker 模式

- [x] 11.1 實作 `--headless` CLI flag，跳過 Tauri GUI 初始化
- [x] 11.2 實作系統服務安裝工具（生成 systemd unit / launchd plist / Windows Service 設定）
- [x] 11.3 Headless Worker 自動接受任務（heartbeat 回應中有 Task 即執行）
- [x] 11.4 實作並發任務管理（獨立 PTY session，上限設定）
- [x] 11.5 危險指令暫停與遠端確認流程（向 Server 回報 `awaiting_approval`，等待核准）
- [x] 11.6 實作本地 log 寫入（rotating，保留 7 天）與 SSE Log 串流上報

## 12. Admin Web Dashboard

- [x] 12.1 建立 Dashboard 專案（React SPA 或 Next.js）
- [x] 12.2 實作管理者登入頁面（email + 密碼 + TOTP）
- [x] 12.3 實作機隊總覽頁面（SSE 即時更新裝置狀態）
- [x] 12.4 實作 Mission Builder（AI 規劃模式 + OpenSpec 上傳模式）
- [x] 12.5 實作 Task DAG 視覺化預覽（可拖曳調整、指派 AITERM）
- [x] 12.6 實作 Mission 監控視圖（DAG 進度、Checkpoint 核准、危險指令確認）
- [x] 12.7 實作稽核 Log 瀏覽器（搜尋、篩選、展開 AI 對話）
- [x] 12.8 實作 VCS Manager（憑證管理、連線測試、專案對應設定）
- [x] 12.9 實作 Skill Manager（上傳、審查、推送、版本回滾）
- [x] 12.10 實作 Policy 設定頁面（角色 AI Provider、ExecutionMode、VCS 權限）
- [x] 12.11 實作管理者帳號管理（邀請、角色指派、停用）
- [x] 12.12 實作 Headless Worker Log 串流頁面（即時 log 顯示）

## 13. 部署打包

- [x] 13.1 建立 Management Server Docker image（multi-stage build）
- [x] 13.2 建立 Admin Dashboard Docker image
- [x] 13.3 建立 On-Premise Docker Compose（server + postgres + dashboard + nginx）
- [x] 13.4 撰寫 On-Premise 安裝文件與環境變數說明
- [x] 13.5 建立 SaaS 多租戶部署設定（`DEPLOYMENT_MODE=saas`）
- [x] 13.6 建立 Helm Chart for Kubernetes（On-Premise k8s 部署）
