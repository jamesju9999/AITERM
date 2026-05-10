## Why

企業技術團隊（開發、DBA、維運）需要一套統一的 AI 輔助工作平台，讓管理者能像 IT 部門主管一樣，透過單一管理介面分派任務、管控 AI Provider、監看執行過程並稽核所有操作紀錄，而不需要擴編大量人力。AITERM Enterprise Fleet Management 將現有的個人版 AITERM 擴展為可集中管理的機隊，讓一名管理者即可統籌指揮多台 AITERM 同步執行軟體開發、測試、部署等任務。

## What Changes

- **新增 Management Server**：獨立服務，負責機隊登錄、任務調度、稽核儲存、Skill 派送、VCS 憑證管理與政策下推
- **新增 Admin Web Dashboard**：管理者網頁介面，支援 Mission 建立（AI 自動規劃或手動上傳 OpenSpec）、機隊監控、Skill Registry、VCS 管理、AI Provider 指派
- **AITERM Client 新增企業模組**：向 Management Server 心跳回報、接收任務（Task Packet）、同步 Skill、使用短效 VCS Token、回報指令歷史與 AI 對話
- **新增 Headless Worker 模式**：AITERM 可以無 GUI 方式運行於專用執行機器，自動接受並執行任務，行為類似 CI Runner
- **新增 Mission 與 Task 調度模型**：Manager 輸入需求 → Planning Agent（AI）拆解成 Task DAG → 自動分派給適合的 AITERM → 依賴關係串接執行 → Checkpoint 人工審查
- **OpenSpec 作為任務規格標準格式**：管理者可上傳 OpenSpec 規格（proposal/design/tasks），或由 Planning Agent 自動生成；規格存入對應 Git repo 的 `docs/` 目錄，AITERM 執行時讀取
- **Skill Registry 與集中派送**：管理者統一管理公司 Skill，可按角色或專案推送到各 AITERM；Headless Worker 自動安裝，Interactive 模式通知員工後生效
- **VCS 統一憑證管理**：Git/SVN token 集中存放於 Management Server（加密），任務派送時生成短效 Scoped Token 給 AITERM，任務結束後自動失效
- **政策引擎**：管理者可按角色設定 AI Provider、ExecutionMode、VCS 分支權限，設定推送到 AITERM 後覆蓋本地設定

## Capabilities

### New Capabilities

- `management-server`: Management Server 核心服務，包含機隊登錄、任務調度（DAG）、稽核儲存、認證授權、政策引擎、VCS Vault、Skill Registry
- `admin-web-dashboard`: 管理者網頁介面，包含 Mission Builder、機隊視圖、Skill Manager、VCS Manager、Provider Manager、稽核瀏覽器、Mission Monitor
- `enterprise-client-module`: AITERM Client 端企業模組，包含 Enterprise Agent（心跳/任務接收）、Skill Sync、Task Runner（/opsx:apply 觸發）、Activity Reporter、VCS Credential Manager
- `headless-worker-mode`: AITERM 無 GUI 執行模式，可安裝於專用機器作為自動任務執行器
- `mission-task-orchestration`: Mission（任務群）與 Task（單一任務）的建模、AI 自動規劃（Planning Agent）、DAG 依賴調度、Checkpoint 機制
- `skill-registry-distribution`: 公司 Skill 版本管理、角色/專案指派、推送到 AITERM 機隊的完整流程
- `vcs-vault`: Git/SVN 憑證集中管理、短效 Scoped Token 生成與失效機制、操作稽核

### Modified Capabilities

（無：現有個人版功能保持不變，企業模組為選擇性啟用）

## Impact

- **src-tauri/src/**：新增 `enterprise/` 模組（enterprise agent、skill sync、activity reporter、vcs credential manager）
- **src/**：新增企業模式 UI（任務通知、進度顯示、企業設定頁）
- **新服務 management-server/**：獨立 Rust（axum + SQLx）或 TypeScript（Next.js + Prisma）專案
- **新服務 admin-dashboard/**：React SPA 或 Next.js 專案
- **AppConfig**：新增 `enterprise_server_url`、`enterprise_device_id`、`enterprise_policy` 欄位
- **部署**：SaaS（多租戶雲端）與 On-Premise（Docker Compose / k8s Helm Chart）兩種部署方案
- **資料敏感性**：指令歷史與 AI 對話含潛在敏感資訊，需敏感資料遮罩機制（密碼、API key pattern 偵測）
