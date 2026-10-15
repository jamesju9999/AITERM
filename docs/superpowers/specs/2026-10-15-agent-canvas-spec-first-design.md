# Agent 協作：規格先行（規格 → 審查 → 協調者拆解派工）— 設計

日期：2026-10-15
狀態：使用者已拍板兩個決定，授權直接實作
前置：第一期、第二期 spec（`2026-10-15-agent-canvas-design.md`、`2026-10-15-agent-canvas-phase2-design.md`）

## 目標

Coder（規格撰寫師）寫規格 → Reviewer 審規格（不通過照舊退回，最多 2 輪）→ 通過後**協調者（真的 Claude）讀規格、拆成多張實作任務**
→ 自動派給**最閒的 Coder** → 每張實作照第二期流程「實作 → 審查 → 退回／通過」→ 全部通過後協調者合併（規格分支也一起合併）。

## 拍板的決定

1. 協調者讀規格後拆成多張（不是一份規格一張）。
2. 實作卡自動派給最閒的 Coder（不是指定、不是丟回模組庫）。

## 資料模型

- `agents.produces_spec INTEGER NOT NULL DEFAULT 0`——這個 Coder 產出規格。對話框勾選；選「規格撰寫師」預設會勾。
- `tasks.is_spec INTEGER NOT NULL DEFAULT 0`——指派時從 Agent 複製（Agent 之後被改或刪除都不影響已指派的卡）。
- `tasks.from_spec TEXT`——實作卡來自哪個規格模組。
- 新 `card_kind = 'plan'`：協調者拆解規格的卡（`agent_id` = Orchestrator、`module_id` = 規格模組、`start_sha/end_sha` = 通過審查的規格版本範圍）。

## 流程（`workflow::after_card_finished` 擴充）

- review 卡 pass，且模組 `is_spec`，且有 Orchestrator → 建 plan 卡（排在佇列尾端）。沒有 Orchestrator → 規格模組照一般模組視為通過。
- plan 卡派工：跟審查一樣在**受審版本的唯讀 detached worktree**（`<專案>/agents/<orch>/reviews/<plan_id>`）裡跑，
  提示詞含：模組標題與需求、規格變更範圍 `git diff start..end`、可用的實作 Coder 清單（名稱＋角色，排除產出規格的 Coder）、
  `tasks.json` 的絕對路徑與格式 `{"tasks":[{"title": string, "body": string}]}`、每張 body 要自給自足並註明對應的規格章節、不要修改檔案。
- plan 卡結案（finalize）：讀 `<專案>/tasks/<plan_id>/tasks.json`，驗證 1–20 張、title 非空；不合格 → 失敗、暫停 Orchestrator。清掉 worktree。
- plan 卡成功（after_card_finished）：再讀一次 tasks.json，依序建實作卡：
  - `card_kind = work`、`from_spec = 規格模組 id`、`status = queued`；
  - 內文 = 任務 body ＋「## 規格全文」（規格範圍內新增／修改的檔案在 `end_sha` 的內容，`git show`，總長上限 30000 字元，超過截斷並註明）；
  - **派給最閒的 Coder**：候選 = `produces_spec = 0` 的 Coder；依（queued+running 卡數、名下 work 卡總數、建立順序）最小者；
    每建一張就把該 Coder 的計數 +1（多張會平均分散）；`project_dir` = 該 Coder 的。
  - 沒有任何可實作的 Coder → 卡片建在「計畫中」（模組庫）不指派，並發 `Attention`（「沒有可實作的 Coder」）。

## 模組狀態

規格模組（`is_spec`）在審查通過之後多兩種狀態：

- 有 plan 卡 queued／running → `dispatching`（協調者拆解中）
- plan 卡失敗 → `dispatch_failed`（Orchestrator 已暫停，可重試）
- plan 卡成功 → `passed`（後續由各實作卡自己的模組狀態負責）

沒有 Orchestrator 時，`is_spec` 的模組在審查通過後就是 `passed`。合併條件不變（所有模組 `passed`）——規格模組在拆解完成前不會是 passed，
實作卡在同一次 `after_card_finished` 裡建立，所以不會在中間空窗觸發合併。

## 前端

- 對話框（Coder）：勾選「產出規格：審查通過後交給協調者拆解、派給其他 Coder 實作」；選「規格撰寫師」預設勾起。
- Agent 卡：產出規格的 Coder 名稱旁顯示 📐。
- 模組 chip 新狀態：`dispatching` 🧭、`dispatch_failed` ⚠。
- Orchestrator 執行中：plan 卡顯示「拆解規格：<模組>」，merge 卡維持「合併各 Coder 分支中」。
- 一次性光點：新出現的 plan 卡 Reviewer→Orchestrator（綠）；新出現的 `from_spec` 實作卡 Orchestrator→該 Coder（紫）。

## 測試

- Rust：assign 會複製 `is_spec`；spec 模組 pass → plan 卡（含範圍、Coder 清單、tasks.json 路徑）；沒有 Orchestrator 不建；
  `parse_plan_json` 驗證；plan 成功 → 依最閒規則分散建卡、跳過產出規格的 Coder、規格全文附在內文（tempdir git repo）；沒有實作 Coder → 建在 planning＋Attention；
  `module_state` 的 dispatching／dispatch_failed／passed；合併在拆解完成前不觸發。
- 前端：deriveModuleState 新狀態；對話框勾選與預設；光點。
- 實機：假 claude 增加「拆解」：寫 tasks.json（2 張）；跑完整流程。
