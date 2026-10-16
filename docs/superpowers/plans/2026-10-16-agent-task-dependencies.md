# 規格拆出的實作卡：相依關係 — 實作計畫

> 規格：docs/superpowers/specs/2026-10-16-agent-task-dependencies-design.md
> 每一步：先寫會失敗的測試 → 實作 → 測試通過 → commit。

**Goal:** 拆卡標出相依；有相依的卡派給前置的擁有者；跨工程師前置要等審過並在開工前合進來；重新分配只搬獨立的卡；派工過程看得到。

## Task 1：資料欄位
- `src-tauri/src/tasks/mod.rs`：`ALTER TABLE tasks ADD COLUMN depends_on TEXT / assigned_from TEXT / assigned_at INTEGER / premerged TEXT`
- `store.rs` `TaskRow` 加四個 `Option` 欄位；測試裡的 TaskRow 字面值補欄位。
- 測試：舊資料庫遷移後可讀寫這四欄。

## Task 2：`tasks.json` 的 `depends_on`
- `workflow.rs` `PlanTask.depends_on: Vec<usize>`（`#[serde(default)]`）；`parse_plan_json` 驗證：索引 < 自己位置，否則 Err（寫出第幾個任務、哪個索引）。
- `plan_body` 提示加相依說明與新格式範例。
- 測試：舊格式、合法相依、自己／往後／超出範圍報錯；提示內容含 `depends_on`。

## Task 3：派工選人＋存相依
- `dispatch_plan`：依序建卡，索引→id；有相依時挑「擁有最多前置」的工程師，平手比忙碌程度；`depends_on` 存成 id 的 JSON 陣列。
- 測試：同一位擁有者集中；兩位擁有者平手挑較閒的；`depends_on` 存的是 id。

## Task 4：開工條件
- `workflow::blocked_cards(pool) -> HashSet<String>`：queued 卡中，有別位工程師的前置還沒 `Passed` 的。
- `scheduler::candidate_heads` 多一個 `blocked` 參數：Agent 佇列跳過被擋的卡，改交出同一位的下一張。`drain_once` 傳入。
- 測試：純函式跳過被擋卡；`blocked_cards` 前置通過前後；同一位擁有者的前置不擋。

## Task 5：開工前合併
- `GitClient::merge_rev(rev, message)`：`git merge --no-edit -m <msg> <rev>`。
- `workflow::premerge_plan(pool, task) -> Vec<(dep_id, title, sha)>`：別位工程師前置的審過版本（通過審查卡的 `end_sha`；沒有審查／手動放行時用鏈上最後成功 work/fix 的 `end_sha`）。
- `scheduler` Agent 卡派工：worktree 準備好之後、`set_start_sha` 之前合併；衝突 → `merge --abort`、回 Err（列出檔案）；成功寫 `premerged`。
- 測試（真 git repo）：審過的版本被合進來、`start_sha` 在合併之後；衝突時還原且回 Err。

## Task 6：重新分配只搬獨立的卡
- `rebalance_idle_coders`：跳過有 `depends_on` 或被別張依賴的卡；搬動時寫 `assigned_from`、`assigned_at`。
- 測試：有相依／被依賴的不搬；搬動記錄來源。

## Task 7：前端
- `ipc/tasks.ts` 加四個欄位。
- `AgentNode`：等前置的模組 chip 顯示 ⛓，title「等前置模組：A、B」。
- 活動記錄：派工、重新分配、開工前合併三種事件。
- 派工虛線：可滑過的透明粗線＋`<title>` 說明。
- i18n 中英文。
- 測試：chip ⛓；三種活動事件；虛線說明。

## Task 8：驗證
- `cargo test --workspace --no-fail-fast`、`npx vitest run`、`npx tsc -b`、`npx eslint`。
- 實機：在測試專案用假 claude 跑一份含整合任務的拆卡，確認整合卡等前置通過才開工、工作資料夾有前置的檔案。
