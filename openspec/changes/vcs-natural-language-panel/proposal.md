## Why

開發者（包含不熟悉 CLI 的用戶與有經驗的工程師）在查詢版控歷史、追蹤變更、操作 GitHub/SVN 時，需要記憶大量指令語法。AITerm 已具備 AI 驅動的終端與資料庫操作，自然應延伸到版控操作，讓用戶以自然語言完成從查詢到寫入的全部 VCS 工作流。

## What Changes

- 新增 **VCS Panel** tab（與 DatabaseView、FileExplorer 並列）
- 新增 `vcs_connections` 設定欄位至 `AppConfig`（TOML），密鑰存入 OS keychain
- Settings 新增「版本控制」頁面，提供 VCS 連線的 CRUD
- Rust backend 新增 `vcs/` 模組，支援 Git（本地）、GitHub API（Level 1-3）、SVN
- VCS Panel 自動感知 active terminal 的 CWD，偵測 repo 類型並比對已設定的連線
- 寫入操作支援三段式權限控制：`read_only / guarded / full_auto`

## Capabilities

### New Capabilities

- `vcs-connection-config`: 管理 VCS 連線設定（Git/GitHub/SVN），包含認證資訊的 CRUD 與 keychain 整合
- `vcs-natural-language-query`: 接受自然語言輸入，AI 解讀意圖後執行對應的 VCS 操作並串流結果
- `vcs-cwd-detection`: 自動偵測 active terminal CWD 的 repo 類型（git/svn）及 root，並比對已設定的連線
- `vcs-write-operations`: 支援寫入操作（revert、cherry-pick、create PR、merge PR、create issue、trigger workflow），依 write_mode 控制風險

### Modified Capabilities

## Impact

- **Config**: `src-tauri/src/config/types.rs` — 新增 `VcsConnection`、`VcsType`、`VcsWriteMode` 型別，`AppConfig` 加入 `vcs_connections: Vec<VcsConnection>`
- **Backend**: 新增 `src-tauri/src/vcs/` 模組（mod.rs、git.rs、svn.rs、types.rs）
- **Commands**: `src-tauri/src/commands.rs` 新增 `vcs_query`、`vcs_list_connections`、`vcs_add_connection`、`vcs_update_connection`、`vcs_remove_connection` 等 IPC commands
- **Frontend**: 新增 `src/components/VcsView/`、`src/components/Settings/VcsConnectionsPage.tsx`、`src/ipc/vcs.ts`
- **Tab system**: `TerminalApp.tsx` 加入 VCS tab 類型
- **Dependencies**: `reqwest`（已有）用於 GitHub API；SVN 操作透過系統 CLI（`svn` 指令）
