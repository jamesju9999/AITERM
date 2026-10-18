# Agent 協作教學輔助 設計

日期：2026-10-18　狀態：使用者核可（A＋B＋C＋D 全做、導覽第一次自動、清單進度全域）

## 目標

第一次用 Agent 協作的人知道每個區塊是什麼、下一步該做什麼；會用的人不被打擾。

## 四個部分

| | 形式 | 何時出現 | 怎麼關 |
|---|---|---|---|
| A | 步驟導覽（聚光燈） | 第一次打開 Agent 協作自動跑一次；之後從「?」重看 | 「略過」、Esc、最後一步「完成」 |
| B | 上手清單（邊做邊勾） | 八步沒全做完、使用者沒關掉之前一直在 | 「×」隱藏；全做完後按「完成」 |
| C | 情境提示 | 依目前專案狀態，卡在某一步時在相關位置冒一個 | 每個提示自己的「×」；「?」裡可整個關掉 |
| D | 說明面板 | 頂列「?」 | 關閉鈕、Esc |

## 上手步驟（B 與 C 共用）

`guideSteps.ts` 定義八步，完成條件全部從桌布狀態推導（`agents`、`tasks`），不靠「使用者按過某個按鈕」：

1. `coder` 放第一位工程師（或套用團隊範本）：有 kind=coder 的 Agent
2. `module` 新增一個模組：有 card_kind=work 的卡
3. `assign` 把模組拖到工程師身上：有 work 卡的 agent_id 不是 null
4. `reviewer` 放審核者
5. `orchestrator` 放協調者
6. `verifier` 放驗收者
7. `merged` 第一次合併完成：有 card_kind=merge、status=done、outcome=success 的卡
8. `verified` 第一次驗收報告：有 card_kind=verify、status=done 的卡（通過與否都算）

**進度全域、只增不減**：在任何專案觀察到某一步完成就記下，換專案或刪掉 Agent 也不會變回未完成。存在 localStorage
`aiterm.agentCanvas.guide`（跟團隊範本、我的角色一樣存在這台電腦），形狀：

```ts
{ tourSeen: boolean; checklistHidden: boolean; hintsOff: boolean; done: StepId[]; dismissedHints: HintId[] }
```

讀取失敗（私密視窗、壞 JSON）一律當作全新狀態；寫入失敗忽略。

## A 步驟導覽

- 依序圈出：角色池、團隊範本鈕、模組庫、桌布、待決策、活動記錄、「?」。每步一段話＋「上一步／下一步／略過」與「n / 7」。
- 用 `data-guide="<id>"` 標記目標元素；找不到的步驟（例如還沒選專案時沒有範本鈕）直接跳過。
- 聚光燈：目標元素外框＋`box-shadow: 0 0 0 9999px` 壓暗其他地方；說明框放在目標下方，空間不夠就放上方，左右夾在視窗內。視窗大小改變時重算。
- 自動開始條件：分頁是作用中、專案清單已載入、`tourSeen` 為 false。結束（任何方式）就寫 `tourSeen: true`。
- 不用 `role="dialog"`（`role="region"`＋`aria-label`），不搶其他對話框的角色。

## B 上手清單

- 浮在桌布右上角的小面板；可收成一顆「開始使用 n/8」的膠囊（收合狀態只在這次畫面有效）。
- 已完成 ✓、目前這一步 →（下方多一行怎麼做）、其他 ○。
- 八步全完成時面板顯示「全部完成」與「完成」鈕，按下寫 `checklistHidden: true`。
- 「×」也寫 `checklistHidden: true`；從「?」可以再打開。

## C 情境提示

`contextHint(agents, tasks)` 依「目前這個專案」的狀態回傳最多一個提示（與全域進度無關，所以新專案仍有引導）：

| 條件（依序取第一個成立的） | 指向 | 內容 |
|---|---|---|
| 有工程師、沒有任何 work 卡 | 模組庫 | 在模組庫新增模組，寫下要做的功能 |
| 模組庫有未指派模組，且有工程師名下沒有 work 卡 | 那位工程師的卡片 | 把模組拖到這位工程師身上就會開工 |
| 有已指派的模組、沒有審核者 | 角色池的審核者 | 放一位審核者，模組完成後會自動送審 |
| 有審核者、沒有協調者 | 角色池的協調者 | 放一位協調者，全部通過後會自動合併 |
| 有協調者、沒有驗收者 | 角色池的驗收者 | 想在合併後自動驗收，可以放驗收者 |

- 導覽進行中不顯示。`hintsOff` 時不顯示。被「×」關掉的提示 id 記在 `dismissedHints`，之後不再出現。
- 位置依目標元素的 `getBoundingClientRect` 相對於畫面計算；指向工程師卡片時用 `data-guide-agent="<id>"`。目標不在畫面上就不顯示。

## D 說明面板

頂列最右邊的「?」（`data-guide="help"`）打開。內容：

- **流程**：工程師各自分支寫模組 → 審核者逐模組審（最多退回 2 輪）→ 協調者合併 → 驗收者照驗收標準實跑。
- **角色**：工程師、審核者、協調者、驗收者各一句；規格撰寫師（工程師的預設角色，勾「產出規格」後由協調者拆成模組）。
- **狀態燈號**：待命、等待名額、編碼中、審查中、合併中、驗收中、完成、需處理（按重試或跳過）——沿用卡片上同一組文字。
- **常見問題**：程式庫要是有 commit 的 git repo；審核退回 2 輪仍不過會停下等你；待決策是 Agent 在問你；「同時上限」是全域同時執行的卡片數；範本存在這台電腦；Codex 要先在終端機登入。
- **動作**：「重看導覽」、「打開上手清單」、「顯示情境提示」開關。

## 檔案

- `src/components/AgentCanvas/guide/guideSteps.ts`：步驟、進度存取、`observedSteps`、`contextHint`（純函式）
- `src/components/AgentCanvas/guide/GuideTour.tsx`、`GuideChecklist.tsx`、`GuideHint.tsx`、`GuideHelp.tsx`
- `src/components/AgentCanvas/guide/guide.css`
- `index.tsx`：持有 guide 狀態、加 `data-guide` 標記、「?」鈕、掛上四個元件；`AgentNode.tsx` 加 `data-guide-agent`
- `src/lib/i18n.ts`：zh-TW 與 en 字串

## 測試

- `guideSteps.test.ts`：每一步的完成條件（含反例：review 卡、未完成的 merge 卡不算）、只增不減的合併、壞 JSON 讀取、`contextHint` 每條規則與優先順序。
- `index.test.tsx`：第一次打開自動出現導覽、略過後重新掛載不再出現、「?」重看；清單隨狀態打勾、「×」隱藏後從「?」打開；提示出現與「×」關掉。既有測試在 beforeEach 預設 guide 為「全部看過、全部關閉」，不受影響。

## 不做

- 不做互動式強制步驟（不鎖住其他操作）。
- 不做每專案進度、不同步到雲端。
- 不在工作看板等其他頁面加導覽。
