## ADDED Requirements

### Requirement: 寫入操作的 guarded 模式預覽
在 `guarded` write_mode 下，系統 SHALL 在執行任何寫入操作前顯示操作摘要與預覽，等待用戶確認。

#### Scenario: Revert commit 的確認流程
- **WHEN** 用戶要求「還原這個 commit」且 write_mode=guarded
- **THEN** 系統顯示操作摘要（將執行的指令說明、影響範圍）與 [確認執行] [取消] 按鈕，等待用戶選擇

#### Scenario: 用戶確認寫入操作
- **WHEN** 用戶點擊「確認執行」
- **THEN** 系統執行操作，回傳結果卡片（成功或錯誤）

#### Scenario: 用戶取消寫入操作
- **WHEN** 用戶點擊「取消」
- **THEN** 操作中止，不執行任何 VCS 操作

### Requirement: read_only 模式下禁用寫入
在 `read_only` write_mode 下，系統 SHALL 禁用所有寫入操作的入口。

#### Scenario: 寫入按鈕 disabled
- **WHEN** 目前連線的 write_mode=read_only
- **THEN** 結果卡片上的寫入按鈕（還原、Merge PR 等）顯示為 disabled 狀態，hover 顯示「此連線為唯讀模式」tooltip

#### Scenario: 自然語言要求寫入操作
- **WHEN** 用戶輸入寫入意圖（如「幫我 merge 這個 PR」）且 write_mode=read_only
- **THEN** 系統回覆「目前為唯讀模式，無法執行此操作。如需啟用，請至設定頁面修改寫入模式。」

### Requirement: Git 本地寫入操作
系統 SHALL 支援以下 Git 本地寫入操作（受 write_mode 控制）：revert commit、cherry-pick commit、create branch、delete branch、checkout branch。

#### Scenario: Revert commit
- **WHEN** 用戶要求還原某個 commit 且通過 write_mode 確認
- **THEN** 系統執行 `git revert <sha>`，回傳新 commit 的資訊卡片

#### Scenario: Cherry-pick commit
- **WHEN** 用戶要求「把那個修 bug 的 commit 帶到這個 branch」
- **THEN** 系統執行 `git cherry-pick <sha>`，回傳結果

### Requirement: GitHub 寫入操作（Level 3）
系統 SHALL 支援以下 GitHub API 寫入操作（需要 write scope token，受 write_mode 控制）：create PR、merge PR、create issue、trigger workflow dispatch。

#### Scenario: 建立 PR
- **WHEN** 用戶輸入「幫我開一個 PR，標題是『Fix auth bug』，從 fix/auth 合到 main」且 write_mode 允許
- **THEN** 系統顯示 PR 資訊預覽（title、base、head branch），確認後呼叫 GitHub API 建立，回傳 PR URL

#### Scenario: Merge PR
- **WHEN** 用戶要求 merge 某個 PR 且 write_mode 允許
- **THEN** 系統顯示 merge 確認（PR title、號碼、merge method），確認後執行，回傳結果

#### Scenario: Token 缺少 write scope
- **WHEN** 用戶執行 Level 3 操作但 token 只有 read scope
- **THEN** 系統顯示「此操作需要 GitHub token 的寫入權限（repo scope），請更新設定中的 token」

### Requirement: SVN 寫入操作
系統 SHALL 支援以下 SVN 寫入操作（受 write_mode 控制）：revert working copy changes、commit、update。

#### Scenario: SVN revert
- **WHEN** 用戶要求「取消這個檔案的修改」且 write_mode 允許
- **THEN** 系統執行 `svn revert <path>`，回傳結果

#### Scenario: SVN commit
- **WHEN** 用戶輸入「提交目前的修改，message 是『Fix login bug』」且 write_mode 允許
- **THEN** 系統顯示即將 commit 的檔案清單作為預覽（guarded 模式），確認後執行 `svn commit -m "Fix login bug"`
