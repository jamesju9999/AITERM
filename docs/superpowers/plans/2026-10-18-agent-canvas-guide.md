# Agent 協作教學輔助 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Agent 協作頁加上步驟導覽、上手清單、情境提示與說明面板（規格：`docs/superpowers/specs/2026-10-18-agent-canvas-guide-design.md`）。

**Architecture:** 純函式模組 `guide/guideSteps.ts` 負責步驟定義、完成判斷、進度存取與情境提示規則；四個呈現元件只吃它的輸出。`index.tsx` 持有 guide 狀態（localStorage 鏡像），把 `data-guide` 標記放到既有元素上。

**Tech Stack:** React 19、TypeScript、Vitest＋React Testing Library。

---

### Task 1: guideSteps.ts（純邏輯）

**Files:** Create `src/components/AgentCanvas/guide/guideSteps.ts`、`guideSteps.test.ts`

- [ ] 先寫測試：
  - `observedSteps`：八步各一個正例；反例——只有 review 卡不算 `module`；status=done 但 outcome=failure 的 merge 卡不算 `merged`；running 的 verify 卡不算 `verified`。
  - `withObserved`：只增不減（舊的 done 保留、新的加入、不重複），沒有新的時回傳同一個物件（避免無謂寫入）。
  - `loadGuide`：沒資料、壞 JSON、欄位型別不對 → 預設值；未知 step id 過濾掉。
  - `contextHint`：表中五條規則各一個正例，以及優先順序（同時符合時取前面的）、沒有工程師時回 null、全部角色都在時回 null。
- [ ] 跑 `npx vitest run src/components/AgentCanvas/guide` 確認紅。
- [ ] 實作：`STEP_IDS`、`StepId`、`GuideState`、`DEFAULT_GUIDE`、`loadGuide()`、`saveGuide(state)`、`observedSteps(agents, tasks)`、`withObserved(state, observed)`、`HintId`、`Hint {id, anchor: {guide} | {agent}}`、`contextHint(agents, tasks)`。
- [ ] 跑測試確認綠；把其中一條條件改壞確認對應測試會紅（突變檢查）。
- [ ] Commit。

### Task 2: i18n 字串

**Files:** Modify `src/lib/i18n.ts`（zh-TW 與 en 同步）

- [ ] 新增：`ac_guide_*`（導覽 7 步標題與內文、上一步／下一步／略過／完成／n of m）、`ac_check_*`（清單標題、八步標題與做法、全部完成、收合）、`ac_hint_*`（五個提示、關閉）、`ac_help_*`（按鈕、各段標題與內容、三個動作）。
- [ ] `npx tsc -b`；跑 i18n 測試（`npx vitest run src/lib`）確認兩語系鍵一致。

### Task 3: 呈現元件

**Files:** Create `guide/GuideTour.tsx`、`GuideChecklist.tsx`、`GuideHint.tsx`、`GuideHelp.tsx`、`guide.css`

- [ ] `GuideTour`：props `{ onDone }`；步驟表 `[{guide, title, body}]`；每次換步驟或 resize 用 `document.querySelector('[data-guide="…"]')` 找目標，找不到就跳到下一個可用步驟；Esc 與「略過」呼叫 `onDone`；`role="region"`。
- [ ] `GuideChecklist`：props `{ done: Set<StepId>, onHide }`；本地 `collapsed` 狀態；全部完成時顯示完成鈕。
- [ ] `GuideHint`：props `{ hint, onDismiss }`；用 `useLayoutEffect`＋resize／scroll 監聽計算位置；目標不存在就不畫。
- [ ] `GuideHelp`：props `{ hintsOn, onToggleHints, onReplayTour, onShowChecklist, onClose }`；沿用 `task-dialog` 樣式。

### Task 4: 接到 index.tsx

**Files:** Modify `src/components/AgentCanvas/index.tsx`、`AgentNode.tsx`、`index.test.tsx`

- [ ] 先寫整合測試（導覽自動出現→略過→重新掛載不再出現→「?」重看；清單打勾與隱藏／再打開；提示出現與關閉），確認紅。
- [ ] 既有測試的 `beforeEach` 寫入「全部看過、全部關閉」的 guide 狀態。
- [ ] `index.tsx`：`guide` state＋`updateGuide`（同步寫 localStorage）；effect 把 `observedSteps(agents, tasks)` 併進去；加 `data-guide`：`pool`、`templates`、`library`（展開與收合兩種 aside）、`canvas`、`inbox`、`log`、`help`；頂列加「?」鈕；掛四個元件。
- [ ] `AgentNode.tsx` 根元素加 `data-guide-agent={agent.id}`。
- [ ] 全部前端測試、`npx tsc -b`、改到的檔案跑 eslint。
- [ ] Commit。

### Task 5: 實機驗收

- [ ] dev App 清掉 `aiterm.agentCanvas.guide` 後打開 Agent 協作：導覽自動出現、每步圈對位置、略過後不再出現。
- [ ] 清單隨放 Agent／新增模組打勾；提示指到正確的位置。
- [ ] 「?」面板三個動作都有效。
