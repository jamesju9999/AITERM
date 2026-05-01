## ADDED Requirements

### Requirement: VCS connection CRUD
系統 SHALL 允許用戶新增、編輯、刪除 VCS 連線設定。每筆連線包含：`id`、`name`、`vcs_type`（git/svn）、`url`（選填，GitHub repo 或 SVN repo URL）、`username`（SVN 用）、`write_mode`（read_only/guarded/full_auto）。密鑰（GitHub token、SVN password）SHALL 儲存於 OS keychain，key 為 `aiterm:vcs:{id}`，不得出現於 TOML 設定檔。

#### Scenario: 新增 Git/GitHub 連線
- **WHEN** 用戶在 Settings → 版本控制頁面填入 name、vcs_type=git、url（GitHub repo URL）、GitHub token
- **THEN** 系統將 metadata 寫入 `AppConfig.vcs_connections`，token 寫入 keychain，並在清單中顯示新連線

#### Scenario: 新增 SVN 連線
- **WHEN** 用戶填入 name、vcs_type=svn、url、username、password
- **THEN** 系統將 metadata 寫入 config，password 寫入 keychain

#### Scenario: 編輯連線
- **WHEN** 用戶點擊已有連線並修改欄位後儲存
- **THEN** config 與 keychain 同步更新，id 不變

#### Scenario: 刪除連線
- **WHEN** 用戶確認刪除一筆連線
- **THEN** 系統從 config 移除該項目，並從 keychain 刪除對應密鑰

### Requirement: VCS connection test
系統 SHALL 提供「測試連線」功能，在儲存前驗證設定是否可用。

#### Scenario: Git 本地連線測試
- **WHEN** 用戶點擊測試，vcs_type=git 且未填 url
- **THEN** 系統顯示「本地 Git 模式，無需測試」

#### Scenario: GitHub API 連線測試
- **WHEN** 用戶點擊測試，填有 GitHub token 與 repo URL
- **THEN** 系統呼叫 GitHub API 驗證 token 有效性，顯示成功或錯誤訊息

#### Scenario: SVN 連線測試
- **WHEN** 用戶點擊測試，vcs_type=svn
- **THEN** 系統執行 `svn info <url>` 驗證認證，顯示成功或錯誤訊息

### Requirement: write_mode 設定
每筆 VCS 連線 SHALL 有獨立的 `write_mode` 設定，預設為 `guarded`。

#### Scenario: 預設為 guarded
- **WHEN** 用戶新增連線時未選擇 write_mode
- **THEN** write_mode 預設為 guarded

#### Scenario: full_auto 需要明確開啟
- **WHEN** 用戶在 Settings 中將 write_mode 設為 full_auto 並儲存
- **THEN** 系統接受設定，VCS Panel 中寫入操作不再需要確認
