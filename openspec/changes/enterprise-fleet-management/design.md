## Context

AITERM 目前是一套個人版 Tauri 2 桌面應用，每台機器獨立運作，設定存於本地（TOML + OS Keychain）。現有架構已有兩個「遠端控制」的前例：Telegram 遠端控制（AITERM 主動 poll Telegram API）以及 VCS 整合（Git/SVN 操作），顯示 client 端已具備向外部服務主動連線的能力。

企業版的核心挑戰是在不破壞個人版體驗的前提下，讓多台 AITERM 可被集中管理、任務調度、政策控制。

**現有相關程式碼：**
- `src-tauri/src/config/types.rs`：`AppConfig` 結構，需新增企業欄位
- `src-tauri/src/telegram/`：polling 模式遠端控制，企業 agent 可參考此模式
- `src-tauri/src/vcs/`：VCS 操作，Scoped Token 注入於此
- `src/hooks/useAgentMission.ts`：現有 AI Agent Loop，Task Runner 複用此 hook

## Goals / Non-Goals

**Goals:**
- Management Server 可集中登錄、監控所有 AITERM 實例（Interactive + Headless Worker）
- 管理者可建立 Mission（需求 → AI 自動拆解 Task DAG 或手動上傳 OpenSpec）並分派給 AITERM 機隊
- 管理者可統一設定每台 AITERM 的 AI Provider、ExecutionMode、VCS 權限（Policy Engine）
- AITERM 可在 Full Auto 模式下自主執行任務（/opsx:apply 流程），結果透過 Git 協作
- Skill 可從 Management Server 集中派送到 AITERM 機隊
- VCS 憑證集中管理，AITERM 使用短效 Scoped Token
- 支援 SaaS（多租戶）與 On-Premise（Docker Compose）兩種部署
- 稽核：所有指令歷史、AI 對話、Skill 執行紀錄可在 Admin Dashboard 查閱

**Non-Goals:**
- 第一版不實作管理者即時介入（遠端 kill session、即時修改 AI 對話）
- 不支援 AITERM 之間直接 P2P 通訊（協作透過 Git）
- 不提供管理者帳號的 SSO/SAML 整合（第一版系統自管帳號）
- 不支援 Interactive AITERM 的螢幕串流或遠端桌面
- Headless Worker 不提供雲端代管（客戶自備機器）

## Decisions

### D1：Management Server 技術棧 — Rust (axum) + SQLx

**決策**：Management Server 使用 Rust（axum web framework）+ SQLx（PostgreSQL），與現有 Tauri backend 技術棧一致。

**理由**：
- 程式碼風格與現有 `src-tauri/src/` 一致，降低維護成本
- Rust 的記憶體安全特性對持有 VCS Token 的服務特別重要
- axum 的 WebSocket 支援良好，適合未來即時通訊擴展

**替代方案**：Node.js/TypeScript（開發速度快但引入新語言）；Go（良好中間選擇但增加技術棧多樣性）。

---

### D2：AITERM ↔ Management Server 通訊 — Polling + SSE

**決策**：AITERM 每 30 秒 poll Management Server（任務查詢、Policy 同步、Skill 更新）；Admin Dashboard 透過 SSE（Server-Sent Events）接收即時更新。

```
AITERM                Management Server               Admin Dashboard
  │                          │                               │
  │──── POST /heartbeat ────▶│                               │
  │◀─── { tasks, policy } ───│                               │
  │                          │──── SSE push ────────────────▶│
  │──── POST /activity ─────▶│ (heartbeat received,          │
  │     (command history)    │  task status changed)         │
```

**理由**：
- AITERM 側實作簡單，斷線重連自然（無 WebSocket 狀態管理）
- 任務派送 30 秒延遲在開發工作流程中完全可接受
- SSE 比 WebSocket 更輕量，Admin Dashboard 不需要雙向即時通訊

**替代方案**：全 WebSocket（即時但連線管理複雜，Headless Worker 斷線重連困難）。

---

### D3：OpenSpec 規格存放位置 — Git Repo 的 docs/ 目錄

**決策**：Management Server 在建立 Mission 時，將 OpenSpec 規格（proposal/design/tasks.md）commit 進目標 Git repo 的 `docs/openspec/missions/{mission-id}/` 路徑。AITERM 執行任務時 clone repo 即可讀取規格，執行 `/opsx:apply`。

```
github.com/company/abc/
├── src/
├── tests/
└── docs/
    └── openspec/
        └── missions/
            └── m-007/
                ├── proposal.md
                ├── design.md
                └── tasks.md   ← AITERM 讀此執行
```

**理由**：
- 規格與程式碼同步版控，天然 audit trail
- AITERM 不需要額外 API 取得規格，clone 即得
- 完全符合現有 OpenSpec 工作流程，`/opsx:apply` 無需修改

---

### D4：VCS 憑證管理 — 短效 Scoped Token

**決策**：Git/SVN Token 加密存於 Management Server（AES-256-GCM）。任務派送時，Server 對 GitHub/GitLab 產生短效 Fine-grained Token（僅限特定 repo 的特定 branch，有效期等於任務預估時間 + 緩衝），隨 Task Packet 傳送給 AITERM。任務結束（成功或失敗）後 AITERM 回報，Server 嘗試撤銷 Token。

**理由**：
- AITERM 機器上永遠不存放永久性 VCS 憑證
- 員工機器被入侵不會洩漏 repo 的長期存取權
- 可從 Server 端集中稽核所有 git 操作

**替代方案**：每台 AITERM 持有 Deploy Key（無法做到 task 級別的權限限制）。

---

### D5：Headless Worker 架構 — AITERM CLI 模式

**決策**：Headless Worker 是 AITERM 的 CLI-only 模式，透過 feature flag（`--headless`）啟動，無 Tauri GUI，以系統服務（systemd / launchd）形式運行。所有 GUI 互動改為透過 Management Server API 回報。

**危險操作處理**：Headless Worker 遇到 `risk_level: dangerous` 的 AI 建議時，不自動執行，改為暫停任務並透過 heartbeat 通知 Management Server，由 Admin Dashboard 顯示警告等待管理者遠端確認。

**替代方案**：獨立的 CLI binary（重複大量 Tauri core 邏輯，維護成本高）。

---

### D6：Skill 派送安全模型 — 雙人審查

**決策**：所有上傳到 Skill Registry 的 Skill 必須經過「第二位管理者帳號」審查批准後才能推送到 AITERM。AITERM 收到新 Skill 時：Interactive 模式顯示通知（員工可看到 Skill 內容）；Headless Worker 自動安裝並記錄至稽核 Log。

**理由**：Skill 本質上是注入到 AI 的指令，若管理者帳號被盜用，惡意 Skill 可能在所有機器上觸發危險操作。雙人控制是最低安全基線。

---

### D7：多租戶與 On-Premise 共用同一 Server 程式碼

**決策**：Management Server 以 `DEPLOYMENT_MODE` 環境變數區分 `saas`（多租戶，每個 Org 資料隔離）與 `onpremise`（單租戶，無 Org 隔離層）。程式碼主體相同，差異在 middleware 層。

**SaaS**：每個 API 請求帶 `X-Org-ID` header，DB query 全部加 `WHERE org_id = ?` 篩選。
**On-Premise**：打包為 Docker Compose（server + postgres + dashboard），提供 Helm Chart for k8s。

## Risks / Trade-offs

**[Risk] AITERM 機器離線時任務積壓**
→ 緩解：Task 狀態機包含 `queued`（等待 AITERM 上線）；Interactive AITERM 上線後詢問員工是否執行排隊任務；Headless Worker 若離線超過閾值，Scheduler 自動重新分配給其他 Worker。

**[Risk] 指令歷史包含密碼/API Key**
→ 緩解：Activity Reporter 在回報前進行 pattern 過濾（`export FOO=...`、`password=`、`Bearer ...` 等），替換為 `[REDACTED]`；此過濾在 AITERM 本地執行，敏感資料不離機器。

**[Risk] Planning Agent 拆解任務品質不穩定**
→ 緩解：AI 生成的 Task DAG 必須經管理者在 Dashboard 確認後才執行（不自動 fire）；管理者可以手動調整 Phase/Task 再確認。

**[Risk] Short-lived Token 在長任務中過期**
→ 緩解：AITERM 在 Token 剩餘時效低於 20% 時，主動向 Server 請求 Token 刷新；Server 記錄 Token 刷新次數於稽核 Log。

**[Risk] Skill 惡意推送**
→ 緩解：D6 雙人審查機制；Interactive AITERM 顯示 Skill 完整內容通知；稽核 Log 記錄所有 Skill 安裝/執行事件。

## Migration Plan

1. **Phase 1**：Management Server + Admin Dashboard（基礎版）— 機隊登錄、心跳、Policy 推送、指令歷史稽核
2. **Phase 2**：Mission 與 Task 調度 — Planning Agent、Task DAG、OpenSpec 整合、Git Commit 規格
3. **Phase 3**：Headless Worker 模式 — CLI 模式、系統服務、危險操作遠端確認
4. **Phase 4**：Skill Registry 與派送 — 雙人審查流程、版本管理、角色指派
5. **Phase 5**：SaaS 多租戶 + On-Premise 打包

個人版功能不受影響；企業模式透過設定 `enterprise_server_url` 啟用，未設定時維持現有行為。

## Open Questions

- **Planning Agent 的輸入 context**：Management Server 在 AI 拆解任務時是否自動 clone repo 讀取現有 codebase 結構？對大型 repo 的效能影響需評估。
- **GitHub Fine-grained Token API**：目前 GitHub Fine-grained PAT 不支援程式化生成（需 OAuth App 或 GitHub App），需確認用 GitHub App installation token 方案是否足夠。GitLab/SVN 需個別評估。
- **Headless Worker 的任務並發數**：同一台 Headless Worker 是否允許同時執行多個 Task？需評估 PTY session 隔離與資源競爭。
