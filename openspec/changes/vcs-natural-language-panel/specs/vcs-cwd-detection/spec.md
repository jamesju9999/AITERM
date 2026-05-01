## ADDED Requirements

### Requirement: 自動偵測 active terminal 的 repo
VCS Panel SHALL 自動偵測 active terminal tab 的 CWD 所在 repo，不需要用戶手動切換 repo context。偵測順序：先試 git，再試 svn。

#### Scenario: CWD 在 Git repo 中
- **WHEN** active terminal 的 CWD 位於 git repo 內
- **THEN** VCS Panel header 顯示 repo root 路徑與「Git」badge

#### Scenario: CWD 在 SVN checkout 中
- **WHEN** active terminal 的 CWD 位於 svn working copy 內
- **THEN** VCS Panel header 顯示 working copy root 路徑與「SVN」badge

#### Scenario: CWD 不在任何 VCS repo 中
- **WHEN** active terminal 的 CWD 不在 git 或 svn repo 內
- **THEN** VCS Panel 顯示「目前目錄不在版控 repo 中」提示

#### Scenario: CWD 切換時自動更新
- **WHEN** 用戶在 terminal 執行 cd 切換到不同 repo
- **THEN** VCS Panel 自動更新 repo context，對話歷史保留但新查詢套用新 context

### Requirement: 比對已設定的 VCS 連線
偵測到 repo 後，系統 SHALL 自動比對 `vcs_connections` 中是否有對應的連線設定（以 repo remote URL 或 SVN URL 匹配），以取得認證資訊。

#### Scenario: 找到對應連線
- **WHEN** 偵測到的 repo remote URL 與某個 VCS connection 的 url 相符
- **THEN** 系統使用該連線的 token/credentials，解鎖對應 Level 的 GitHub/SVN 功能

#### Scenario: 未找到對應連線
- **WHEN** 偵測到 git repo 但無對應的 VCS connection
- **THEN** 系統以本地 git 模式運作（Level 1 only），header 顯示「本地模式」提示，並提供「新增 GitHub 連線」快速連結

### Requirement: SVN CLI 可用性檢查
系統 SHALL 在 VCS Panel 首次偵測到 SVN repo 時，檢查系統是否安裝了 `svn` CLI。

#### Scenario: SVN CLI 未安裝
- **WHEN** 偵測到 SVN repo 但 `svn --version` 失敗
- **THEN** VCS Panel 顯示「需要安裝 SVN CLI 才能使用 SVN 功能」提示，SVN 相關操作 disabled
