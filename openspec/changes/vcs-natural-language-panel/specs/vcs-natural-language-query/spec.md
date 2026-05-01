## ADDED Requirements

### Requirement: 自然語言 VCS 查詢
系統 SHALL 接受自然語言輸入，AI 解讀意圖後執行對應的 VCS 操作，並以結構化卡片呈現結果。支援的查詢意圖包含：log 查詢、diff 查看、blame、branch 列表、PR/Issues/Actions 查詢（GitHub）。

#### Scenario: 查詢修改歷史
- **WHEN** 用戶輸入「上週哪些人修改了 src/auth/ 這個資料夾」
- **THEN** 系統執行對應的 `git log` 或 `svn log`，以 commit 卡片列表回傳，每張卡片顯示 hash/revision、作者、日期、message、修改檔案數

#### Scenario: 查看特定 commit 的 diff
- **WHEN** 用戶輸入「顯示那個 commit 的詳細變更」或點擊卡片上的「查看 diff」
- **THEN** 系統顯示 unified diff，語法高亮

#### Scenario: 查看 blame
- **WHEN** 用戶輸入「這個檔案哪一行是誰改的」並指定檔案路徑
- **THEN** 系統回傳 blame 表格，每行顯示作者、日期、revision

#### Scenario: 查詢 GitHub PR 列表
- **WHEN** 用戶輸入「有哪些 open PR」且連線有 GitHub token
- **THEN** 系統呼叫 GitHub API，以 PR 卡片列表回傳，顯示 title、號碼、作者、狀態、最後更新時間

#### Scenario: 查詢 GitHub Actions 狀態
- **WHEN** 用戶輸入「最近的 CI 跑過了嗎」
- **THEN** 系統呼叫 GitHub API 取得最近 workflow runs，以狀態卡片回傳

#### Scenario: 無 token 時降級
- **WHEN** 用戶查詢 PR/Issues/Actions 但目前連線無 GitHub token
- **THEN** 系統提示「此功能需要 GitHub token，請至設定頁面新增」，並提供快速連結

### Requirement: 查詢結果的互動按鈕
Read-only 查詢結果卡片 SHALL 提供情境式操作按鈕，點擊後以自然語言觸發下一步操作或執行 write 操作（受 write_mode 控制）。

#### Scenario: Commit 卡片互動
- **WHEN** log 查詢回傳 commit 卡片列表
- **THEN** 每張卡片顯示「查看 diff」、「還原此 commit」（受 write_mode 控制）按鈕

#### Scenario: PR 卡片互動
- **WHEN** PR 列表回傳
- **THEN** 每張 PR 卡片顯示「查看 PR comments」、「Merge」（受 write_mode 控制）按鈕

### Requirement: 大型結果集分頁
系統 SHALL 在查詢結果超過 100 筆時，預設只顯示前 100 筆，並提示用戶可要求更多。

#### Scenario: 結果截斷提示
- **WHEN** log 查詢結果超過 100 筆
- **THEN** 回傳前 100 筆並顯示「顯示前 100 筆，輸入『更多』查看下一頁」

#### Scenario: 用戶要求更多
- **WHEN** 用戶輸入「更多」或「繼續」
- **THEN** 系統回傳下一頁 100 筆結果
