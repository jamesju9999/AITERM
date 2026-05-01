## 1. Config & Types（Rust）

- [x] 1.1 在 `src-tauri/src/config/types.rs` 新增 `VcsType`（git/svn）、`VcsWriteMode`（read_only/guarded/full_auto）enum
- [x] 1.2 新增 `VcsConnection` struct（id, name, vcs_type, url, username, write_mode），加入 `AppConfig.vcs_connections: Vec<VcsConnection>`
- [x] 1.3 為新型別補充 TOML roundtrip 測試（比照現有 `db_type_roundtrips_toml` 測試模式）

## 2. VCS Backend 模組（Rust）

- [x] 2.1 建立 `src-tauri/src/vcs/` 目錄，新增 `mod.rs`、`types.rs`、`git.rs`、`svn.rs`
- [x] 2.2 在 `types.rs` 定義 `VcsIntent` enum（LogQuery, DiffView, Blame, BranchList, PrList, IssueList, ActionsList, RevertCommit, CherryPick, CreatePr, MergePr, CreateIssue, TriggerWorkflow, SvnCommit, SvnRevert, SvnUpdate）及 `VcsResult` 回應型別
- [x] 2.3 在 `git.rs` 實作本地 git 操作：`log`、`show`（diff）、`blame`、`branch -a`（Level 1）
- [x] 2.4 在`git.rs` 實作 GitHub API 查詢：PR list、PR comments、Issues list、Actions workflow runs（Level 2，使用 `reqwest`）
- [x] 2.5 在 `git.rs` 實作 GitHub API 寫入：create PR、merge PR、create issue、trigger workflow dispatch（Level 3）
- [x] 2.6 在 `git.rs` 實作本地 git 寫入：`revert`、`cherry-pick`、branch create/delete/checkout
- [x] 2.7 在 `svn.rs` 實作 SVN 查詢：`svn log --xml`、`svn diff`、`svn blame`、`svn info`（CLI dispatch）
- [x] 2.8 在 `svn.rs` 實作 SVN 寫入：`svn revert`、`svn commit`、`svn update`
- [x] 2.9 在 `mod.rs` 實作 `VcsManager`：持有 connections、`detect_repo(path)` 偵測 repo 類型與 root、比對 connection

## 3. IPC Commands（Rust）

- [x] 3.1 在 `commands.rs` 新增 `vcs_list_connections`、`vcs_add_connection`、`vcs_update_connection`、`vcs_remove_connection`（比照 db_* 指令）
- [x] 3.2 新增 `vcs_test_connection(connection_input) -> Result<String, String>`
- [x] 3.3 新增 `vcs_detect_repo(path: String) -> Result<VcsRepoInfo, String>`（回傳 repo type、root、matched connection id）
- [x] 3.4 新增 `vcs_query(query: String, repo_info: VcsRepoInfo, session_id: String)` — AI 解讀意圖 → 執行 → 串流結果（使用既有 `ai-stream` event 機制）
- [x] 3.5 在 `lib.rs` 的 `tauri::Builder` 中 invoke handler 加入新 commands

## 4. Settings UI（Frontend）

- [x] 4.1 新增 `src/components/Settings/VcsConnectionsPage.tsx`（比照 `DatabaseConnectionsPage.tsx` 結構）
- [x] 4.2 在 `SettingsView.tsx` 加入「🔀 版本控制」sidebar tab，渲染 `VcsConnectionsPage`
- [x] 4.3 在 `src/ipc/vcs.ts` 新增 IPC wrapper functions（vcsListConnections、vcsAddConnection、vcsUpdateConnection、vcsRemoveConnection、vcsTestConnection）
- [x] 4.4 為 VcsConnectionsPage 補充基本 render 測試

## 5. VCS Panel UI（Frontend）

- [x] 5.1 建立 `src/components/VcsView/` 目錄，新增 `VcsView.tsx`、`VcsView.css`
- [x] 5.2 實作 `useVcsCwd` hook：輪詢 active terminal CWD → 呼叫 `vcs_detect_repo` → 回傳 repo context
- [x] 5.3 實作 `useVcsChat` hook：管理對話歷史與 streaming（比照 `useAiChat.ts`）
- [x] 5.4 實作 VCS Panel header：顯示 repo root、VCS type badge、write_mode selector
- [x] 5.5 實作 `VcsMessageBubble.tsx`：渲染 commit 卡片列表、diff viewer、blame 表格、PR 卡片
- [x] 5.6 在 commit/PR 卡片實作情境按鈕（查看 diff、還原、Merge PR），依 write_mode 控制 disabled 狀態
- [x] 5.7 實作 guarded 模式的寫入確認 UI（操作摘要 + 確認/取消按鈕）
- [x] 5.8 實作「無 token」降級提示與快速跳轉設定頁面連結

## 6. Tab 整合（Frontend）

- [x] 6.1 在 `TerminalApp.tsx` 的 tab type 加入 `vcs`
- [x] 6.2 在 tab bar 加入 VCS tab 的新增入口（`NewTabPicker` 或 tab bar 按鈕）
- [x] 6.3 在 `TerminalApp.tsx` 的 tab render 邏輯加入 `VcsView` 的渲染分支

## 7. i18n

- [x] 7.1 在 `src/lib/i18n.ts` 新增 VCS 相關字串的 en / zh-TW 翻譯（panel title、提示文字、按鈕標籤）
