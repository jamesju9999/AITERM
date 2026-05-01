## Context

AITerm 已有以下可複用的基礎設施：
- `AppConfig` TOML + OS keychain 模式（`db_connections`）— VCS 連線設定直接比照
- `execution_mode`（AlwaysConfirm / Graded / FullAuto）— 寫入操作的風險控制沿用此概念
- FileExplorer 的 `ptyCwdRef` polling — CWD 感知機制可直接複用
- `reqwest` + `rustls-tls` — GitHub API HTTP client 已存在
- AI streaming 模式（`ai-stream` events）— VCS 查詢結果串流沿用同樣的 Tauri event 機制
- DatabaseView 的 chat-style UI 模式 — VCS Panel UI 參照此結構

SVN 在企業環境中仍有實際使用需求，需要與 Git/GitHub 共存於同一套 UX 流程。

## Goals / Non-Goals

**Goals:**
- 自然語言驅動的 VCS 查詢與操作（read + write）
- 支援 Git（本地）、GitHub API（PR/Issues/Actions）、SVN（CLI）
- 設定整合進現有 `AppConfig` + keychain 體系
- CWD 自動感知，無需用戶手動切換 repo context
- 三段式寫入控制（read_only / guarded / full_auto）

**Non-Goals:**
- 不實作 git GUI（branch graph、merge tool 等視覺化工具）
- 不支援 GitLab / Bitbucket API（留作後續擴充）
- 不實作 SVN conflict resolution UI
- 不取代 terminal tab 的 git 操作（兩者並存）

## Decisions

### 1. VCS 抽象層放在 Rust backend，不在 AI prompt

**選擇**：AI 只負責將自然語言轉換成語意意圖（`VcsIntent` enum），由 Rust 的 `git.rs`/`svn.rs` 執行具體操作。

**理由**：避免 AI 直接拼接 shell 指令造成 injection 風險；Rust 層可做型別安全的參數驗證；不同 VCS 的指令差異由 backend 屏蔽，AI prompt 保持簡單。

**替代方案**：讓 AI 直接生成 git/svn 指令（像 `/ai` 功能）→ 捨棄，因為 VCS Panel 目標是「無需懂語法」，而非指令生成器。

---

### 2. SVN 透過系統 CLI，不用 Rust library

**選擇**：`svn` 操作透過 `Command::new("svn")` 執行系統安裝的 SVN CLI。

**理由**：Rust 的 SVN binding 不成熟；企業 SVN 環境通常已有正確設定的 CLI（含 SSL cert、proxy 設定）；CLI 輸出格式穩定，XML output (`--xml`) 容易解析。

**替代方案**：使用 `svn-rs` crate → 捨棄，維護狀況不佳且功能不完整。

---

### 3. GitHub API 分三個 scope，token 為單一欄位

**選擇**：每個 GitHub connection 只儲存一個 token（存於 keychain）。系統在執行操作前動態判斷所需 scope，若 token 權限不足則提示用戶。

**Level 1**（無 token）：本地 git CLI 操作  
**Level 2**（`repo` read scope）：PR/Issues/Actions 查詢  
**Level 3**（`repo` write scope）：Create PR、merge、create issue、trigger workflow

**理由**：用戶不需要理解 OAuth scope；一個 token 即可涵蓋所有需求；GitHub PAT 本身就是單一字串。

---

### 4. CWD 感知：VcsView 自行 poll，不從 TerminalApp 傳入

**選擇**：`VcsView` 透過 `vcs_detect_repo` IPC command 輪詢 active terminal 的 CWD（複用 `ptyCwdRef` 機制），結果存於 component state。

**理由**：與 FileExplorer 使用相同模式，避免在 TerminalApp 增加跨 tab 的狀態傳遞複雜度。

---

### 5. 結果呈現：結構化卡片，不是 raw terminal output

**選擇**：VCS 操作結果以結構化 JSON 從 backend 回傳，frontend 渲染為卡片（log 列表、diff viewer、blame 表格、PR card）。

**理由**：讓初學者能看懂結果；write 操作的確認 UI 需要結構化資料；Markdown diff block 搭配現有 `MermaidBlock` 模式可擴充。

## Risks / Trade-offs

- **SVN 未安裝** → 偵測 `svn --version` 失敗時，UI 顯示「需要安裝 SVN CLI」提示，SVN 相關功能 disabled
- **GitHub rate limit** → Level 2/3 操作加上 rate limit header 檢查，超限時顯示友善提示並建議稍後重試
- **大型 repo 的 git log 效能** → 查詢時加上預設 `--max-count=100` 限制，用戶可在對話中要求「更多」
- **Write 操作風險** → `guarded` 模式下所有寫入操作必須顯示 diff preview + 操作摘要，用戶確認後才執行；`full_auto` 模式需在 Settings 中手動開啟，不可在 VCS Panel 內直接升級

## Migration Plan

- `AppConfig` 加入 `vcs_connections` 欄位有 `#[serde(default)]`，舊設定檔直接相容，無需 migration
- 新 tab 類型不影響現有 tab 的狀態，可安全部署

## Open Questions

- GitHub OAuth App flow（device flow）是否需要？或只支援 PAT？→ 初版只做 PAT，與現有 provider config 模式一致
- SVN connection 是否需要 `svn:// ` 與 `https://` 以外的 protocol（如 `svn+ssh://`）？→ 初版支援 http/https/svn，ssh 留作後續
