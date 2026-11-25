# 問題收集 Server：自動處理題目（③）

日期：2026-11-25　狀態：已核可（唯讀＋可改檔案需人工核准）

## 目標

Server 自己用 Claude Code 處理題目（診斷並回覆；必要時改程式），不必等成員領取。結果與成員處理一樣變成**草稿**，由人看過才送出。

## 設計

- **執行器**：server 為每一題啟動一個 `claude -p` 子行程（`--output-format json`、`--no-session-persistence`），工作目錄是該專案的 `work_dir`（server 專案已有此欄位）。設定存在 `intake_settings` 的 `server_runner`：`claude_path`（預設 `claude`）、`max_concurrent`（預設 1）、`timeout_secs`（預設 600，上限 800，必須小於領取租約 900 秒）、`max_turns`（預設 20）。
- **認證**：用 server 主機上 Claude Code 自己的登入（管理員在主機執行一次 `claude login`），或主機環境裡的金鑰；AITerm server 不存也不讀。`status` 與設定頁顯示 claude 是否可用（`claude --version`）。不可用時 server 不會領任何題目。
- **觸發**：專案設定新增 `auto_process`（預設關）。開啟後 server 定時掃描，對該專案「已核准、待派送」的題目以領取者 `server` 領取（沿用現有原子領取與租約），受並行上限限制；與成員手動領取並存，誰先領到誰處理。
- **兩種模式**
  - **唯讀（預設）**：`--allowedTools Read Grep Glob`，並明確禁用 Edit、Write、Bash、網路相關工具。用於：管道設成「只診斷」、題目種類是提問、**所有外部客戶的題目**、以及沒有標種類又無法分類時。參考資料夾（server 上的路徑，此處有效）以 `--add-dir` 開放給唯讀工具。
  - **可改檔案（需人工核准）**：只對管道設成「可改檔案」、等級是內部、種類是「要改程式」的題目。在**隔離的 git worktree**（`<資料夾>/worktrees/<題目>`，專屬分支 `intake/<題目>`）內執行，允許 Edit、Write，另外只開放專案設定的驗證指令這一個 Bash 指令。做完 commit 到該分支、跑驗證，題目結果是草稿，並標記「待核准合併」。**永不自動合併**。
- **核准合併**（只有管理員）：`POST /v1/items/{id}/merge`，`approve: true` 把分支以 `--no-ff` 合併進 `work_dir` 目前的分支（衝突就中止並回報，不留半套），再移除 worktree 與分支；`approve: false` 直接移除。題目新增欄位 `branch`、`merge_state`（`pending`／`merged`／`rejected`），畫面在題目詳情顯示變更摘要與核准／駁回按鈕。
- **結果**：回覆與驗證報告先去敏，以 `finish_claim` 交成草稿（含驗證結果）。逾時、非零離開、輸出不是預期格式：`fail_claim` 記原因。子行程逾時會被終止；server 重啟時留下的處理中題目，租約逾期後自動放回。
- **安全**：傳給子行程的環境變數最小化（`PATH`、`HOME`、Claude 需要的 `ANTHROPIC_*`／`CLAUDE_*`）；提示詞沿用既有的去敏與診斷提示；外部題目永遠唯讀；改檔案只發生在隔離的 worktree，主工作目錄不被直接修改，直到管理員核准。
- **驗證指令移進共用 crate**：`verify` 目前在桌面程式裡，搬到 `aiterm-intake-core`，桌面與 server 共用。

## 不做

自動合併、server 上執行任意指令、多組 Claude 帳號輪替、串流顯示處理過程（第一版只回傳最終結果）。

## 驗證計畫

假的 `claude`（可設定路徑的腳本）驅動自動測試：唯讀與可改檔案各自傳了正確的工具參數、外部題目強制唯讀、worktree 與分支、驗證、核准合併、駁回、衝突、逾時被終止、並行上限、claude 不可用時不領題。最後用真實 Claude Code 在真實 git 專案跑一次端到端。
