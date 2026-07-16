# 統一按鈕視覺系統 — 交接筆記（給接手的 Gemini）

**寫給：** 接手繼續執行的 AI 助理
**日期：** 2026-07-16
**目前狀態：** Task 1-9 已完成並通過雙階段 review（spec compliance + code quality），僅剩 Task 10（整體視覺回歸驗證）待執行。

---

## 這是什麼

這是一個橫跨 23 個檔案、統一 app 內所有按鈕視覺風格的大型重構。背景、設計決策、完整規格都在：

- **設計規格：** `docs/superpowers/specs/2026-07-16-unified-button-system-design.md`
- **實作計畫：** `docs/superpowers/plans/2026-07-16-unified-button-system.md`（10 個 Task 的完整規格，含每個檔案的確切 before/after 程式碼）

**請先讀完這兩份文件再動手**，尤其是 plan 文件的 Task 10 章節（整體視覺回歸驗證），那是你唯一需要執行的部分。

---

## 工作環境

- **Git worktree：** `/Users/jamesju/Documents/GitHub/AITERM/.claude/worktrees/unified-button-system`
- **分支：** `worktree-unified-button-system`
- **目前 HEAD：** `cdb2a99`
- 這個 worktree 與主要工作目錄（`/Users/jamesju/Documents/GitHub/AITERM`，在 `master` 分支）是分開的，互不影響。**請在這個 worktree 目錄下工作**，不要動到 master。

---

## 已完成的工作（Task 1-9，全部已 commit）

```
5f560be feat(styles): add shared aiterm-btn button style system              [Task 1]
ecdcb9c refactor(loop-studio): migrate buttons to shared aiterm-btn classes  [Task 2]
ef3369d fix(loop-studio): bump specificity on ghost-button hover overrides   [Task 2 修正]
940e75b refactor(database-view): migrate buttons to shared aiterm-btn classes[Task 3]
f6ae862 fix(database-view): restore hover color on session-delete button    [Task 3 修正]
5687b44 fix(database-view): fix hover color on 2 more ghost buttons         [Task 3 修正]
85469f2 refactor(settings): migrate About/Providers/MCP buttons             [Task 4]
9100508 refactor(enterprise): migrate task accept/reject buttons            [Task 5]
d72bd92 refactor(settings): dedupe DatabaseConnectionsPage/VcsConnections   [Task 6]
bd6349d refactor(doc-converter,file-explorer): migrate buttons             [Task 7]
e95f781 fix(file-explorer): fix fe-btn padding losing cascade tie          [Task 7 修正]
2385469 refactor(terminal-view): migrate block/search buttons             [Task 8]
ac8a779 fix(terminal-view): restore hover background on AI/toggle buttons [Task 8 修正]
f6fff4c refactor(chat-panels): collapse 4 circular send-button defs       [Task 9]
cdb2a99 fix(chat-panels): fix specificity ties on hover and padding       [Task 9 修正]
```

每個 Task 都經過：implementer 實作 → spec compliance reviewer 驗證 → code quality reviewer 驗證 → 發現問題就修正再驗證，直到兩個 review 都通過才算完成。這 15 個 commit 都已經過這個流程，**不需要重新檢查**。

### 建立了什麼

`src/styles/buttons.css`（全域引入於 `src/App.css` 開頭）定義了共用的按鈕 class 系統：

```css
.aiterm-btn                    /* base：cursor/transition/flex 版面 */
.aiterm-btn--primary           /* 主要動作：漸層背景、8px 圓角、hover 上浮+發光 */
.aiterm-btn--secondary         /* 次要／外框：透明底、6px 圓角、hover 加深 */
.aiterm-btn--danger            /* 危險：靜止淡紅底，hover 轉實心紅 */
.aiterm-btn--danger-solid      /* 危險確認鈕：靜止就是實心紅（已在二次確認情境） */
.aiterm-btn--ghost             /* 純圖示/文字，無底無框 */
.aiterm-btn--icon              /* 圓形/方形圖示按鈕，固定 28x28px */
.aiterm-btn--sm                /* 尺寸修飾：密集列表用 */
```

Task 2-9 已將全 app ~35 個按鈕定義遷移到這套系統，涵蓋：LoopStudio、DatabaseView、Settings（About/Providers/Mcp/Enterprise/DatabaseConnections/VcsConnections）、DocConverter、FileExplorer、TerminalView、VcsView、CrossDbView、AiPanel、DesignView、CommandBookmarks。

### ⚠️ 重要教訓：CSS specificity tie 陷阱（會一直冒出來，請主動避開）

這是這次重構過程中**最常見、最容易漏掉的 bug**，在 Task 2/3/7/8/9 都各自踩過一次才修正。原理：

> 當一個舊的 per-file class（例如 `.old-btn`）保留了自己的樣式規則（尤其是 `:hover` 底下設定 `color`/`background`/`padding`/`transform`/`box-shadow` 等），同時又在 JSX 裡跟共用的 variant class（如 `aiterm-btn--primary`）疊加使用時，兩個 class 的 specificity 可能打平（都是單一 class selector）。這種情況下**誰贏取決於 CSS 檔案在最終 bundle 裡的載入順序**，而這個順序**不一定跟你想的一樣**（`buttons.css` 雖然在 `App.css` 開頭 `@import`，但因為 Vite 的 import graph，實際常常會排在元件自己的 CSS 之後）。結果就是共用 class 悄悄贏過你想保留的舊樣式，肉眼看程式碼完全看不出問題，**必須實際 `npm run build` 後查看 `dist/assets/index-*.css` 才能確認真正的載入順序**。

**修法：** 把舊 class 的規則改成複合選擇器（compound selector），例如：
```css
/* 錯誤 — 會 tie */
.old-btn:hover:not(:disabled) { color: #f87171; }

/* 正確 — 複合選擇器，specificity 保證更高，不受載入順序影響 */
.old-btn.aiterm-btn:hover:not(:disabled) { color: #f87171; }
/* 或用實際疊加的 variant class 也可以： */
.old-btn.aiterm-btn--ghost:hover:not(:disabled) { color: #f87171; }
```

如果衝突的屬性不是靠 selector 就能贏（例如元素本身有 inline `style={{ color: ... }}`），複合選擇器沒用，因為 **inline style 永遠贏過任何非 `!important` 的樣式表規則**，這時只能在樣式表規則加 `!important`。

**在 Task 10 視覺驗收時，請特別留意這個陷阱** — 如果你發現某個按鈕的顏色/padding/hover 效果「看起來不太對」，先懷疑是不是這個 tie 問題，用 `npm run build` 查看實際輸出的 CSS 來確認，不要只看原始碼。

---

## 你要做的事：Task 10（整體視覺回歸驗證）

完整規格在 `docs/superpowers/plans/2026-07-16-unified-button-system.md` 的「Task 10」章節，這裡摘要：

1. **`npx tsc --noEmit`** 全域跑一次，確認完全無錯誤。
2. **`npm run lint`** 全域跑一次，確認沒有新增的錯誤（跟這個 refactor 開始前的 baseline 比較——本次改動過程中沒有新增任何 lint 錯誤，只是touch 到的檔案原本就有一些 pre-existing 的警告，那些不用管）。
3. **Playwright 截圖驗證**（沿用本次重構過程中一直使用的手法）：對至少 3 個代表性面板做視覺確認：
   - **LoopStudio**：確認 `.ls-start-btn`（漸層主要鈕）、`.ls-stop-btn`（淡紅危險鈕）、`.ls-close-cancel-btn`/`.ls-close-discard-btn`（次要+危險確認鈕配對）
   - **Settings → Providers 頁**：確認 `.btn-add`（漸層主要鈕）、`provider-card-actions` 內按鈕（次要+危險）
   - **DatabaseView → AI Chat**：確認送出鈕（圓形漸層，disabled 時正確變暗）、停止鈕（圓形實心紅）
   - 建議也看一下 **VcsView**、**CrossDbView**、**AiPanel**、**DesignView** 的圓形送出鈕（Task 9 剛統一過，特別容易有上面提到的 specificity tie 問題）
   - 建議也看一下 **CommandBookmarks** 的關閉/刪除按鈕 hover 效果

   截圖驗證的具體做法（本次重構全程都是這樣做的）：
   ```bash
   # 建一個暫時的 preview harness（比照範例，mock 必要 props），或直接跑正常的 dev server
   npm run dev
   # 用 playwright CLI 截圖（不需要額外裝 playwright，npx 會自動下載）
   npx playwright screenshot --viewport-size=WIDTH,HEIGHT --wait-for-timeout=1000 http://localhost:1420/ output.png
   ```
   如果需要 mock props 來繞過 Tauri IPC 依賴，可以參考本次重構第一階段（通知圓點修正）用過的手法：暫時把 `src/main.tsx` 換成一個渲染單一元件的臨時檔案，截圖完後**務必用 `git diff -- src/main.tsx` 確認完全還原**（`main.tsx` 有 CRLF 換行，若用 Write 工具重寫要注意換行符會被改成 LF 導致整檔案 diff——最安全的還原方式是 `git checkout -- src/main.tsx`，不要手動重寫）。

4. **確認選中狀態顏色語彙沒有意外被動到：**
   ```bash
   grep -rn "aiterm-agent-toggle--on" src/components/*/[A-Za-z]*.css
   ```
   應該只有 3 處定義：`TerminalView.css`（固定綠）、`DatabaseView/index.css`（固定綠）、`AiPanel/styles.css`（`var(--accent)`）。這 3 處在整個重構過程中都刻意沒有被動過，Task 10 只是最後確認一次。

5. 如果視覺驗收發現任何問題（顏色不對、padding 跑掉、hover 沒反應），**優先懷疑上面說的 specificity tie 問題**，用 `npm run build` 查看實際的 `dist/assets/index-*.css` 來確認，不要只看原始碼猜測。修法一律是複合選擇器或 `!important`（視情況，參考上面的說明），不要改回舊的寫法或引入新的樣式差異。

6. 全部驗證通過後，commit（如果 Task 10 有任何修正的話）並回報給使用者。**Task 10 是純驗證性質，理論上不需要新的功能性修改**，除非驗收時真的發現遺漏的 specificity 問題。

---

## 不要做的事

- 不要重新 review 或修改 Task 1-9 已經 commit 的內容，除非 Task 10 驗收時真的發現視覺問題。
- 不要用 `git stash` 做任何前後比對——這個 repo 的 stash stack 是跨 worktree、跨分支共用的，整個重構過程中已經有好幾次不小心撞到使用者在其他分支（`feature/provider-tool-calling-stage1`/`stage2-anthropic`）留下的 WIP stash（雖然每次都有驚無險地清理乾淨，但風險是真實的）。要比較前後狀態一律用 `git diff`/`git show <sha>:<path>`。
- 不要改變任何按鈕的 onClick/disabled/確認流程邏輯——整個重構全程都是純樣式遷移，這個原則到 Task 10 也一樣適用。
- 不要合併回 `master`——完成後把結果回報給使用者，由使用者決定要不要 merge（這是 subagent-driven-development 流程的慣例，實際合併決策留給人類）。

---

## 給使用者的簡短版本（如果你只想看結論）

10 個 task 完成了 9 個，通過完整的兩階段 review（spec compliance + code quality），過程中修正了 8 次因為 CSS specificity tie 導致的視覺 bug（都已修好並驗證）。剩下 Task 10（純驗證，跑 tsc/lint + Playwright 截圖確認 3-6 個代表性面板的按鈕視覺正確），沒有已知的待辦問題。所有工作都在獨立的 git worktree（`worktree-unified-button-system` 分支）裡，完全沒碰到 `master`。
