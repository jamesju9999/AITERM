# Agent 協作桌布 第二期：審查與合併 — 設計

日期：2026-10-15
狀態：決定已由使用者拍板（見第一期 spec §4），使用者授權直接實作到完成
前置：`docs/superpowers/specs/2026-10-15-agent-canvas-design.md`（第一期，已完成於 `feat/agent-canvas`）

## 目標

Coder 每完成一個模組，Code Reviewer 自動審查；不通過就退回原 Coder 修正（同一模組最多退回 2 輪）；
所有模組都通過後，Orchestrator 自動把各 Coder 的分支合併到整合分支。桌布上用連線與光棒呈現交接。

## 拍板的決定（第一期 §4）

- 連線自動建立：每個看板專案最多 1 個 Reviewer、1 個 Orchestrator；Coder→Reviewer→Orchestrator
  （沒有 Reviewer 時 Coder→Orchestrator）。
- 每個模組做完就審（不是整個佇列做完才審）。
- 審查判定寫進檔案 `review.json`，不從終端機文字猜。只有正確性／需求不符才 fail，風格寫進 reasons。
- 不通過 → 在原 Coder 佇列**最前面**插一張修正卡，修完再審；同模組 fail 第 3 次 → 停下、標「需處理」、通知使用者。
- 所有模組通過後才合併。

## 名詞

- **模組**：使用者指派給 Coder 的原始卡片（`card_kind = 'work'`）。
- **模組鏈**：模組本身＋它衍生的審查卡與修正卡（`module_id = 模組 id`）。

## 資料模型

`tasks` 新欄位（`ALTER TABLE ... ADD COLUMN`，吞錯）：

| 欄位 | 說明 |
|---|---|
| `card_kind TEXT NOT NULL DEFAULT 'work'` | `work` \| `review` \| `fix` \| `merge` |
| `module_id TEXT` | review/fix 卡指向模組；work/merge 為 NULL |
| `review_of TEXT` | review 卡：被審的那張 work/fix 卡 |
| `verdict TEXT` | review 卡：`pass` \| `fail`，結束時由 review.json 寫入 |
| `review_reasons TEXT` | review 卡：reasons 的 JSON 陣列字串 |
| `module_override INTEGER NOT NULL DEFAULT 0` | 模組卡：使用者手動放行 |

`agents` 新欄位：`last_merge_signature TEXT`——上一次為哪一組模組版本建立過合併卡。

## 模組狀態（Rust `workflow::module_state` 與 TS `deriveModuleState` 同一套規則）

依模組鏈判斷，第一個符合的勝出：

1. 鏈中有 work/fix 卡 running → `coding`；有 queued → `queued`（fix 卡 → `fix_queued`）
2. 模組卡未完成（planning/queued）→ 依其狀態
3. `module_override` → `passed`
4. 最新一張 work/fix 卡失敗（coder 失敗，Agent 會暫停）→ `failed`
5. 最新的 review 卡：running → `reviewing`、queued → `awaiting_review`、
   done 且 verdict pass → `passed`、done 且 verdict fail → fail 次數 > 2 ? `attention` : `fix_queued`、
   done 但沒有 verdict（審查卡失敗）→ `review_failed`
6. 沒有任何 review 卡 → `passed`（這個模組完成時沒有 Reviewer）

`fail 次數` = 該模組 verdict = fail 的 review 卡數。

## 工作流程（`src-tauri/src/tasks/workflow.rs`）

`after_card_finished(pool, project_path, task_id) -> Vec<WorkflowEvent>`，在 watch closure 裡 `persist_outcome` 之後呼叫，
結束後 `wake.notify_one()`。只處理有 `agent_id` 的卡片。

- **work/fix 卡成功**，專案有 Reviewer → 建 review 卡：`agent_id = reviewer`、`module_id`、`review_of`、
  `start_sha/end_sha` 複製自被審卡、`project_dir` = 模組卡的 `project_dir`、狀態 queued 排在 Reviewer 佇列最後、
  `parallel_ok = true`、`interactive = false`。標題「審查：<模組標題>」，內文見下。
- **review 卡成功**：已在 finalize 階段寫好 verdict（見下）。
  - pass → 不建卡。
  - fail 且 fail 次數 ≤ 2 → 建 fix 卡：`agent_id` = 模組的 Coder、`module_id`、標題「修正：<模組標題>（第 n 輪）」、
    內文附審查 reasons 與原始需求、插到 Coder 佇列最前面。
  - fail 且 fail 次數 > 2 → 不建卡，回傳 `WorkflowEvent::Attention { module_title }`，由呼叫端發 `task-finished`
    （outcome failed、error「審查不通過超過上限，需要你處理」）沿用既有桌面／Telegram 通知。
- 每次呼叫最後跑 `maybe_enqueue_merge`：
  - 有 Orchestrator、未暫停、沒有 queued/running 卡；
  - 專案內所有 Coder 與 Reviewer 都沒有 queued/running 卡；
  - 至少一個模組，且所有模組（未封存、`agent_id` 屬於 Coder 的 work 卡）狀態都是 `passed`；
  - signature（各 Coder 的 id 與其分支 HEAD = 該 Coder 最新成功 work/fix 卡的 `end_sha`，排序後串起來）≠ `last_merge_signature`
  → 建 merge 卡（`agent_id = orchestrator`），寫回 signature。
- `agents_override_module`（手動放行）也呼叫 `maybe_enqueue_merge`。

### Review 卡的工作目錄

Reviewer 不能在 Coder 的 worktree 審（Coder 可能正在做下一個模組）。派工時建一個**唯讀用途的 detached worktree**：
`git -C <模組 project_dir> worktree add --detach <專案>/agents/<reviewer>/reviews/<review_id> <end_sha>`。
結束後 `git worktree remove --force` 該目錄（失敗只記 log）。

### Review 卡結束（finalize，依 Agent kind 分流）

- coder／orchestrator：第一期行為（自動 commit、記 end_sha、失敗暫停）。
- reviewer：不 commit。成功時讀 `<專案>/tasks/<review_id>/review.json`：
  `{"verdict":"pass"|"fail","reasons":[string]}`；解析失敗或檔案不存在 → outcome 改 failed（「審查沒有產出有效的 review.json」）、
  Reviewer 暫停。成功解析 → 寫 `verdict`、`review_reasons`。失敗／取消 → Reviewer 暫停。最後移除 review worktree。

### 提示詞內文（建立卡片時寫入 body，派工時再加 Agent 的角色前綴）

- review：模組標題、受審版本、`git diff <start>..<end>`、原始需求、（修正輪）上一輪 reasons、判定規則、不要修改任何檔案、
  review.json 的絕對路徑與格式。
- fix：「審查沒有通過，請修正模組…（第 n 輪，最多 2 輪）」、reasons 條列、原始需求。
- merge：在目前目錄（整合分支）依序 `git merge` 各 Coder 分支（列出分支名與負責模組），衝突理解雙方意圖後解決並 commit，跑測試，不要 push。

### Orchestrator 的工作目錄

沿用 `prepare_agent_worktree`（分支 `aiterm-agent/<orchestrator id>`，從 `orchestrator.project_dir` HEAD 分出），
畫面上稱「整合分支」。成果用既有「合併分支」按鈕合回程式庫。

## 指令

- `agents_create`：允許 `reviewer`／`orchestrator`；同專案已有同種類 → Err「每個專案最多一個 …」。
- 新增 `agents_override_module(project_id, task_id)`：模組狀態必須是 `attention` 或 `review_failed`；設 `module_override = 1`，
  跑 `maybe_enqueue_merge`，poke 排程器。
- `tasks_assign_agent`：只能指派給 `coder`。

## 前端

- Agent 池加 Reviewer、Orchestrator（已存在就停用並顯示提示）；對話框依 kind 給不同角色預設。
- `deriveModuleState`（TS）＋ Coder 卡的 chip 改成**每個模組一顆**，顯示模組狀態圖示；`attention`／`review_failed` 的 chip 可點「放行」。
- Reviewer 卡：審查中顯示「審查 <模組>（<Coder>）」與 `start..end` 短 sha；佇列數。
- Orchestrator 卡：「等待 x/y 通過」、合併中、完成；「合併分支」按鈕沿用。
- 狀態色：reviewer 執行中黃色、orchestrator 執行中綠色（第一期 coder 藍色）。
- `EdgeLayer`（SVG）：自動連線；流動光棒（下游正在處理上游的東西：reviewer 跑的 review 卡屬於該 coder → coder→reviewer 黃；
  orchestrator 跑 merge → reviewer→orchestrator 綠）；一次性光點：前後兩次資料比對，新出現的 review 卡（coder→reviewer 藍）、
  新的 pass verdict（reviewer→orchestrator 綠）、新出現的 fix 卡（reviewer→coder 橘）。
- 模組庫只列 `card_kind = work` 的計畫中卡片（本來就只有 work 會在 planning）。

## 測試

- Rust：`module_state` 各分支；`after_card_finished`（work 成功→review 卡；review pass／fail→fix 卡在 Coder 佇列最前；
  第 3 次 fail→Attention 且不建卡；沒有 Reviewer→不建卡）；`maybe_enqueue_merge`（全通過才建、有東西在跑不建、signature 相同不重建）；
  review.json 解析；detached review worktree 建立與清除；agents_create 種類上限（store 層函式）。
- 前端：`deriveModuleState`；EdgeLayer 依 Agent 組合產生正確的邊；池中 Reviewer 已存在時停用；放行按鈕呼叫 IPC。
- 實機：假 claude 擴充審查（依內文決定 pass/fail、寫 review.json）與合併（依清單 `git merge`），跑完整流程：
  兩 Coder × 模組 → 審查 → 一個模組退回一輪後通過 → 一個模組連 3 次不通過 → 放行 → 自動合併。
