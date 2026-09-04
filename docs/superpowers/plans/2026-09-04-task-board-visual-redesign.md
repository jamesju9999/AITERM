# 工作看板視覺重新設計 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把工作看板（卡片、四欄版面、新增/編輯工作對話框、對話記錄對話框）從無樣式的原生元件，換成使用者在瀏覽器輔助工具裡定案的深藍配色設計——不管使用者選了 app 哪個主題都一樣，純視覺、不動任何拖曳/派工邏輯。

**Architecture:** 兩個層次：① 卡片/欄位/ghost 這幾個工作看板自己專屬的元素，直接在 `index.css` 寫死新的深藍色數值；② 兩個對話框（`TaskEditorDialog`/`TranscriptDialog`）完全不改 `.tsx`——它們現有的 `.aiterm-btn`/`.task-dialog`/`.task-field-input` 這些既有規則本來就是透過 `var(--accent, ...)`/`var(--bg-secondary, ...)` 這些全域主題變數上色，所以只要在 `.task-board` 這個外層容器上，把這些變數**局部覆寫**成新的深藍值，兩個對話框就會自動跟著換色，完全不用碰它們的程式碼。

**Tech Stack:** CSS custom properties（變數覆寫）、React（`TaskCard.tsx` 的 JSX 結構調整）。

---

## 背景（每個任務都可能用到）

- 完整設計依據：`docs/superpowers/specs/2026-09-04-task-board-visual-redesign-design.md`
- 這個 app 的全域主題系統（`src/lib/themes.ts`）會設定一整組 CSS 變數在 `:root` 上：`--bg-primary`/`--bg-secondary`/`--bg-tertiary`/`--bg-surface`/`--border-color`/`--border-subtle`/`--text-primary`/`--text-secondary`/`--text-muted`/`--accent`/`--accent-dim`，隨使用者選的主題變動。
- 但 `src/styles/buttons.css`（`.aiterm-btn--secondary`）讀的是 `var(--border, #333)`——不是 `--border-color`，這是既有、跟這次改動無關的小落差（`--border` 這個變數名稱主題系統從來沒設過，一直吃 fallback 值）。這次要在 `.task-board` 同時覆寫 `--border` 跟 `--border-color` 兩個名字，繞過這個既有落差，**不去修它**（不在這次範圍內）。
- `src/components/TaskBoard/index.css`：`.task-column`/`.task-card`/`.task-card-ghost` 目前讀的是 `var(--panel, ...)`/`var(--card, ...)`/`var(--accent, ...)`，同樣要覆寫這些名字。
- `TaskEditorDialog.tsx`/`TranscriptDialog.tsx` 在 JSX 裡是 `.task-board` 這個根 `<div>` 底下的子元素（不是 portal），CSS 變數會自然繼承下去。

---

### Task 1: `index.css` — 變數覆寫 + 卡片/欄位/ghost 重新設計

**Files:**
- Modify: `src/components/TaskBoard/index.css`

這個任務純 CSS，沒有新的自動化測試可寫（沒有邏輯變化）——用「跑一次既有測試群組確認沒有回歸」取代傳統的紅燈/綠燈循環，這在純視覺改動裡就是有意義的驗證。

- [ ] **Step 1: 執行既有測試，記錄基準線**

Run: `npm run test -- src/components/TaskBoard 2>&1 | tail -10`
Expected: 全部通過（記下測試數量，這個任務結束後要一樣的數字，證明沒有改到任何行為）。

- [ ] **Step 2: 讀取目前完整的 `index.css`**

先讀一次目前的 `src/components/TaskBoard/index.css` 全文，確認接下來要替換的每一段程式碼跟這裡列出的完全一致（如果檔案內容跟這裡假設的不同，以實際檔案為準，調整替換範圍，但整體改動意圖不變）。

- [ ] **Step 3: 加變數覆寫區塊**

在 `.task-board { display: flex; flex-direction: column; height: 100%; overflow: hidden; }` 這行**之後**加一段新規則：

```css
/* 工作看板專屬的深藍配色——不管使用者在設定頁選了 app 哪個主題，這個區塊
   內看到的都是同一種深藍。做法是局部覆寫這個容器範圍內的全域主題變數
   （--accent/--bg-*/--border* 等），讓 TaskEditorDialog/TranscriptDialog
   既有的 .aiterm-btn/.task-dialog/.task-field-input 規則透過 CSS 變數
   繼承自動跟著換色，完全不用改那兩個檔案的程式碼。
   --border 跟 --border-color 兩個都覆寫：buttons.css 的 .aiterm-btn--secondary
   讀的是 --border，主題系統實際設的是 --border-color，這是既有的落差，
   這裡只是覆寫兩邊繞過去，不是在修那個落差（不在這次範圍內）。 */
.task-board {
  --bg-primary: #0a0e14;
  --bg-secondary: #10151d;
  --bg-tertiary: #141a24;
  --bg-surface: #141a24;
  --bg-hover: #1a2230;
  --border: #1f2733;
  --border-color: #1f2733;
  --border-subtle: #1f2733;
  --text-primary: #eef2f7;
  --text-secondary: #a8b6c8;
  --text-muted: #6b7a8f;
  --accent: #2f6fed;
  --accent-dim: rgba(47, 111, 237, 0.14);
  --accent-glow: rgba(47, 111, 237, 0.35);
  --panel: #141a24;
  --card: #141a24;
  /* 這個功能區自己額外需要、主題系統裡沒有對應概念的顏色（成功/失敗/中性/
     深藍主色的加深版，給漸層跟 hover 用）。 */
  --tb-accent-dark: #1e4fc7;
  --tb-success: #22c55e;
  --tb-success-dim: rgba(34, 197, 94, 0.14);
  --tb-failed: #ef4444;
  --tb-failed-dim: rgba(239, 68, 68, 0.14);
  --tb-cancelled: #eab308;
  --tb-cancelled-dim: rgba(234, 179, 8, 0.14);
  --tb-neutral: #3a4556;
}
```

- [ ] **Step 4: 重寫欄位相關規則**

把這兩行：

```css
.task-column { display: flex; flex-direction: column; background: var(--panel, #1b1b1b); border-radius: 8px; overflow: hidden; min-height: 0; border: 1px solid transparent; }
.task-column--drop-target { border-color: var(--accent, #a855f7); background: var(--accent-dim, rgba(168, 85, 247, 0.08)); }
```

換成：

```css
.task-column { display: flex; flex-direction: column; background: var(--panel); border-radius: 10px; overflow: hidden; min-height: 0; border: 1px solid transparent; }
.task-column--drop-target { border-color: var(--accent); background: var(--accent-dim); }
```

把這行：

```css
.task-column-head { display: flex; justify-content: space-between; align-items: center; padding: 10px 12px; font-weight: 600; border-bottom: 1px solid var(--border, #2a2a2a); flex-shrink: 0; }
```

換成（標題縮小、全大寫、加字距，弱化成分類標籤）：

```css
.task-column-head { display: flex; justify-content: space-between; align-items: center; padding: 10px 12px; border-bottom: 1px solid var(--border-color); flex-shrink: 0; }
.task-column-title { font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-secondary); }
```

（`.task-column-title` 這個 class 在 `TaskColumn.tsx` 的 `<span className="task-column-title">{title}</span>` 已經存在，這裡只是新增對應的 CSS 規則，不用改 `.tsx`。）

- [ ] **Step 5: 重寫卡片拖曳外框 + ghost（含傾斜效果）**

把這幾段：

```css
.task-card-drag-wrap--draggable { cursor: grab; }
.task-card-drag-wrap--draggable:active { cursor: grabbing; }
...
.task-card-drag-wrap--dragging {
  opacity: 0.35;
  pointer-events: none;
}
...
.task-card-ghost {
  position: fixed;
  transform: translate(-50%, -16px);
  pointer-events: none;
  z-index: 1000;
  width: 220px;
  background: var(--card, #242424);
  border: 1px solid var(--accent, #a855f7);
  border-radius: 6px;
  padding: 10px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  user-select: none;
  -webkit-user-select: none;
}
```

換成（拖曳來源卡片行為不變；ghost 加 -8° 傾斜 + 放大 1.04 倍 + 更深的陰影，放開時因為元素直接從 DOM 移除，不需要額外的「彈回」動畫）：

```css
.task-card-drag-wrap--draggable { cursor: grab; }
.task-card-drag-wrap--draggable:active { cursor: grabbing; }
.task-card-drag-wrap--dragging {
  opacity: 0.35;
  pointer-events: none;
}
.task-card-ghost {
  position: fixed;
  transform: translate(-50%, -16px) rotate(-8deg) scale(1.04);
  pointer-events: none;
  z-index: 1000;
  width: 220px;
  background: linear-gradient(180deg, #141a24 0%, #10151d 100%);
  border: 1px solid var(--accent);
  border-radius: 12px;
  padding: 14px;
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(47, 111, 237, 0.2);
  user-select: none;
  -webkit-user-select: none;
}
```

- [ ] **Step 6: 重寫卡片本體、標題、meta、badge、動作區**

把這幾段：

```css
.task-card { background: var(--card, #242424); border: 1px solid var(--border, #2f2f2f); border-radius: 6px; padding: 10px; cursor: grab; user-select: none; -webkit-user-select: none; -webkit-user-drag: none; }
.task-card-title { font-weight: 600; margin-bottom: 4px; }
.task-card-meta { font-size: 12px; opacity: 0.65; margin-top: 2px; }
.task-card-actions { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
.task-badge { font-size: 11px; padding: 1px 6px; border-radius: 4px; display: inline-block; margin-top: 4px; }
.task-badge--success { background: #1f4023; color: #b6f0c2; }
.task-badge--failed { background: #4a1f1f; color: #f0b6b6; }
.task-badge--cancelled { background: #3a3a1f; color: #efe7b0; }
.task-badge--interactive { background: #1f2a3a; color: #b6d4f0; }
```

換成：

```css
.task-card {
  position: relative;
  background: linear-gradient(180deg, #141a24 0%, #10151d 100%);
  border: 1px solid var(--border-color);
  border-radius: 12px;
  padding: 14px 14px 12px 16px;
  cursor: grab;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
  overflow: hidden;
  user-select: none;
  -webkit-user-select: none;
  -webkit-user-drag: none;
}
/* 左側狀態色條——用卡片外層的 data-task-status 屬性驅動，見 TaskCard.tsx。 */
.task-card::before {
  content: "";
  position: absolute;
  left: 0; top: 0; bottom: 0;
  width: 3px;
  background: var(--tb-neutral);
}
.task-card[data-task-status="running"]::before { background: var(--accent); box-shadow: 0 0 8px var(--accent); }
.task-card[data-task-status="success"]::before { background: var(--tb-success); }
.task-card[data-task-status="failed"]::before { background: var(--tb-failed); }
.task-card[data-task-status="cancelled"]::before { background: var(--tb-cancelled); }

.task-card-title { font-weight: 700; font-size: 14.5px; color: var(--text-primary); line-height: 1.35; }
.task-card-meta {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--text-muted);
  margin-top: 5px;
  font-family: var(--mono, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.task-card-avatar {
  width: 24px; height: 24px; border-radius: 7px;
  background: linear-gradient(135deg, var(--accent), var(--tb-accent-dark));
  display: flex; align-items: center; justify-content: center;
  font-size: 12px; flex-shrink: 0;
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.06);
}
.task-card-top-row { display: flex; align-items: flex-start; justify-content: space-between; gap: 10px; }

.task-card-badges { display: flex; gap: 6px; margin-top: 11px; flex-wrap: wrap; }
.task-badge { font-size: 10px; padding: 3px 8px 3px 6px; border-radius: 20px; font-weight: 600; display: inline-flex; align-items: center; gap: 4px; letter-spacing: 0.01em; }
.task-badge--success { background: var(--tb-success-dim); color: var(--tb-success); }
.task-badge--failed { background: var(--tb-failed-dim); color: var(--tb-failed); }
.task-badge--cancelled { background: var(--tb-cancelled-dim); color: var(--tb-cancelled); }
.task-badge--interactive { background: var(--accent-dim); color: var(--accent); border: 1px solid rgba(47, 111, 237, 0.3); }
.task-badge--running { background: var(--accent-dim); color: var(--accent); }
.task-badge-dot { width: 5px; height: 5px; border-radius: 50%; background: currentColor; animation: task-badge-pulse 1.6s ease-in-out infinite; }
@keyframes task-badge-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.3; } }

.task-card-divider { height: 1px; background: linear-gradient(90deg, var(--border-color), transparent); margin: 11px 0 9px; }

.task-card-actions { display: flex; flex-wrap: wrap; gap: 6px; }
```

- [ ] **Step 7: 新增工作看板專屬的按鈕樣式**

在檔案裡（例如緊接在 `.task-card-actions` 規則之後）加一段全新的按鈕系統，專屬於工作看板卡片內的動作按鈕（不影響/不重新定義全域 `.aiterm-btn`，那個給對話框用，透過 Step 3 的變數覆寫自動換色即可）：

```css
.tb-btn {
  cursor: pointer;
  font-family: inherit;
  border-radius: 7px;
  padding: 5px 10px;
  font-size: 11.5px;
  font-weight: 600;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  transition: all 0.15s ease;
}
.tb-btn:disabled { opacity: 0.4; cursor: default; }
.tb-btn--ghost { background: transparent; border: 1px solid var(--border-color); color: var(--text-secondary); }
.tb-btn--ghost:hover:not(:disabled) { background: var(--bg-hover); }
.tb-btn--primary { background: linear-gradient(135deg, var(--accent), var(--tb-accent-dark)); border: none; color: #fff; box-shadow: 0 2px 8px var(--accent-glow); }
.tb-btn--primary:hover:not(:disabled) { transform: translateY(-1px); }
.tb-btn--danger-ghost { background: transparent; border: 1px solid rgba(239, 68, 68, 0.35); color: #e8929a; }
.tb-btn--danger-ghost:hover:not(:disabled) { background: rgba(239, 68, 68, 0.1); }
```

- [ ] **Step 8: 執行測試，確認沒有回歸**

Run: `npm run test -- src/components/TaskBoard 2>&1 | tail -10`
Expected: 通過數量跟 Step 1 記下的基準線一樣（CSS 改動不影響任何測試斷言）。

Run: `npx tsc -b`
Expected: 乾淨（這個任務沒有動 `.tsx`，理論上不會有變化，但仍要確認）。

- [ ] **Step 9: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src/components/TaskBoard/index.css
git commit -m "feat(tasks): task board deep-blue visual redesign — variables, columns, cards, ghost"
```

---

### Task 2: `TaskCard.tsx` — 套用新版面結構跟按鈕

**Files:**
- Modify: `src/components/TaskBoard/TaskCard.tsx`
- Test: `src/components/TaskBoard/index.test.tsx`

Task 1 的 CSS 規則已經就位，這個任務讓 `TaskCard.tsx` 的 JSX 實際產生那些規則要吃的結構（`data-task-status` 屬性、`.task-card-top-row`/`.task-card-avatar`/`.task-card-badges`/`.task-card-divider`、新按鈕 class）。

- [ ] **Step 1: 寫兩個新的結構性測試，鎖住這次改版的關鍵行為**

在 `src/components/TaskBoard/index.test.tsx`，靠近既有的 `"interactive running card shows the interactive badge and a Mark Done button that calls markTaskDone"` 測試旁邊，加：

```ts
  it("running card carries a data-task-status attribute matching its status, for the CSS left-accent-bar", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Running one", status: "running", tab_id: "tab-1" }),
    ]);
    view();
    const cardEl = await screen.findByText("Running one");
    const cardRoot = cardEl.closest(".task-card") as HTMLElement;
    expect(cardRoot.dataset.taskStatus).toBe("running");
  });

  it("done+success card's data-task-status reflects the outcome, not just the status", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "Done one", status: "done", outcome: "success" }),
    ]);
    view();
    const cardEl = await screen.findByText("Done one");
    const cardRoot = cardEl.closest(".task-card") as HTMLElement;
    expect(cardRoot.dataset.taskStatus).toBe("success");
  });

  it("interactive running card's Mark Done button uses the primary button style, Stop uses ghost", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Chatting", status: "running", tab_id: "tab-1", interactive: true }),
    ]);
    view();
    await screen.findByText("Chatting");
    expect(screen.getByRole("button", { name: /標記完成|Mark Done/ }).className).toContain("tb-btn--primary");
    expect(screen.getByRole("button", { name: /停止|Stop/ }).className).toContain("tb-btn--ghost");
  });
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx 2>&1 | tail -40`
Expected: 新增的 3 個測試 FAIL——`data-task-status` 屬性目前不存在，按鈕目前也沒有任何 className。

- [ ] **Step 3: 讀取目前完整的 `TaskCard.tsx`**

先讀一次目前的 `src/components/TaskBoard/TaskCard.tsx` 全文，確認接下來要替換的內容跟這裡假設的一致。

- [ ] **Step 4: 實作**

把整個 `return (...)` 區塊換成：

```tsx
  const cardStatus =
    card.status === "done" ? (card.outcome ?? "done") : card.status;

  return (
    <div className="task-card" data-task-status={cardStatus}>
      <div className="task-card-top-row">
        <div>
          <div className="task-card-title">{card.title}</div>
          <div className="task-card-meta">📁 {card.project_dir}</div>
          {!card.parallel_ok && <div className="task-card-meta">⚑ {t.board_card_solo_hint}</div>}
          {card.status === "running" && <div className="task-card-meta">{t.board_running_hint}</div>}
          {card.status === "done" && card.error_message && (
            <div className="task-card-meta">{card.error_message}</div>
          )}
        </div>
        {card.interactive && <div className="task-card-avatar">👤</div>}
      </div>

      <div className="task-card-badges">
        {card.status === "running" && (
          <span className="task-badge task-badge--running">
            <span className="task-badge-dot" />
            {t.board_col_running}
          </span>
        )}
        {card.interactive && (
          <span className="task-badge task-badge--interactive">{t.board_badge_interactive}</span>
        )}
        {card.status === "done" && card.outcome && (
          <span className={`task-badge task-badge--${card.outcome}`}>{outcomeLabel}</span>
        )}
      </div>

      <div className="task-card-divider" />

      <div className="task-card-actions">
        {card.status === "planning" && (
          <>
            <button className="tb-btn tb-btn--ghost" disabled={busy} onClick={onEdit}>{t.board_edit_card}</button>
            <button className="tb-btn tb-btn--danger-ghost" disabled={busy} onClick={() => void remove()}>{t.board_delete}</button>
          </>
        )}
        {card.status === "running" && (
          <>
            <button className="tb-btn tb-btn--ghost" disabled={busy} onClick={() => void run(() => stopTask(card.id))}>
              {t.board_action_stop}
            </button>
            {card.interactive && (
              <button className="tb-btn tb-btn--primary" disabled={busy} onClick={() => void run(() => markTaskDone(card.id))}>
                {t.board_action_mark_done}
              </button>
            )}
            {card.tab_id && <button className="tb-btn tb-btn--ghost" onClick={openTab}>{t.board_action_open_tab}</button>}
          </>
        )}
        {card.status === "done" && (
          <>
            {card.transcript_path && (
              <button className="tb-btn tb-btn--ghost" onClick={onViewTranscript}>{t.board_action_transcript}</button>
            )}
            <button className="tb-btn tb-btn--ghost" disabled={busy} onClick={() => void run(() => cloneTask(card.id))}>
              {t.board_action_requeue}
            </button>
            <button className="tb-btn tb-btn--danger-ghost" disabled={busy} onClick={() => void remove()}>{t.board_delete}</button>
          </>
        )}
      </div>
    </div>
  );
```

（`cardStatus` 這個計算值放在 `return` 前面、`outcomeLabel` 定義之後即可——`outcomeLabel` 這段既有程式碼不用動。`t.board_col_running`（「執行中」/`Running`）沿用工作看板既有的欄位標題字串，不需要新增 i18n key。）

- [ ] **Step 5: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx 2>&1 | tail -60`
Expected: 全部通過（既有 + 這次新增 3 個）。

- [ ] **Step 6: 廣泛回歸測試 + tsc + eslint**

Run: `npm run test -- src/components/TaskBoard 2>&1 | tail -10`
Expected: 全部通過，數量比 Task 1 記下的基準線多 3（這次新增的測試）。

Run: `npx tsc -b`
Expected: 乾淨。

Run: `npx eslint src/components/TaskBoard/TaskCard.tsx src/components/TaskBoard/index.test.tsx`
Expected: 乾淨。

- [ ] **Step 7: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src/components/TaskBoard/TaskCard.tsx src/components/TaskBoard/index.test.tsx
git commit -m "feat(tasks): TaskCard uses the new deep-blue layout (avatar chip, status bar, pill badges)"
```

---

### Task 3: 驗證整輪（含手動視覺驗證）

- [ ] **Step 1:** `npm run test 2>&1 | tail -8` → 全部通過
- [ ] **Step 2:** `npx tsc -b` → 乾淨
- [ ] **Step 3:** `npm run lint 2>&1 | grep -iE "taskboard|task-card"` → 空（跟工作看板相關的部分零錯誤；repo 既有的其他無關 lint 錯誤不算）
- [ ] **Step 4（手動視覺驗證，需要真的啟動 app）：** `npm run tauri:dev`：
  1. 工作看板每一欄各放至少一張卡片：計畫中、待執行、一張互動模式的執行中、一張成功、一張失敗——確認左側色條顏色正確（藍/中性/綠/紅）、執行中卡片的圓點有跳動、互動模式卡片右上角有圖示 chip。
  2. 開新增工作對話框，確認邊框/輸入框/checkbox/儲存按鈕都是深藍配色，不是使用者目前設定頁選的那個主題色。
  3. 打開一張已完成卡片的對話記錄對話框，同樣確認配色跟卡片、對話框一致。
  4. 拖曳一張卡片，確認 ghost 有 -8° 傾斜跟放大效果，放開後正常消失、卡片正確落在目標欄位。
  5. 確認拖曳時文字不會被選取（今天稍早修過的行為，這次改動不應該讓它退化）。
- [ ] **Step 5:** 如果 Step 4 發現任何問題，修正後回到 Step 1 重跑整輪驗證，再 commit 修正。

---

## Self-Review

**Spec 覆蓋：**
- 卡片左側狀態色條、跳動圓點、圖示化互動徽章、分隔線、藥丸徽章 → Task 1（CSS）+ Task 2（JSX 結構）。✅
- 按鈕重新設計（ghost/primary/danger-ghost）→ Task 1 定義 `.tb-btn--*`，Task 2 套用到每個按鈕。✅
- 四欄版面（標題弱化、拖放高亮改色）→ Task 1。✅
- 兩個對話框統一配色，且不改對話框本身的程式碼 → Task 1 的變數覆寫區塊，透過 CSS 繼承自動生效，Task 3 手動驗證確認。✅
- 拖曳傾斜效果 → Task 1（`.task-card-ghost` 的 `transform`）。✅
- 不影響 app 其他地方的主題 → 變數覆寫只發生在 `.task-board` 這個選擇器範圍內，app 其他地方讀的是 `:root` 上的原始值，不受影響。✅
- 不動拖曳/派工邏輯 → 整份計畫沒有任何一個任務碰 `handleDrop`/`isLegalDropTarget`/`dispatch`/`scheduler` 這些檔案。✅

**Placeholder 掃描：** 無 TBD/TODO，每個 CSS/程式碼區塊都是完整可貼上的內容。

**型別一致性檢查：**
- `data-task-status` 屬性值（`"planning"`/`"queued"`/`"running"`/`"success"`/`"failed"`/`"cancelled"`）在 Task 1 的 CSS 選擇器（`[data-task-status="running"]` 等）跟 Task 2 的 `cardStatus` 計算值用法一致——`card.status` 是 `"done"` 時改用 `card.outcome`，其餘狀態直接沿用 `card.status`，兩邊沒有拼字或值域落差。
- `.tb-btn--ghost`/`.tb-btn--primary`/`.tb-btn--danger-ghost` 這三個 class 名稱在 Task 1（定義）跟 Task 2（套用到每個 `<button>`）完全一致。
- `.task-card-avatar`/`.task-card-top-row`/`.task-card-badges`/`.task-card-divider`/`.task-badge-dot` 這幾個新 class 名稱，Task 1 的 CSS 選擇器跟 Task 2 的 JSX className 一一對應，沒有遺漏或多餘。

## 相關

`docs/superpowers/specs/2026-09-04-task-board-visual-redesign-design.md`（這份計畫的設計依據）
`docs/superpowers/specs/2026-07-16-unified-button-system-design.md`（app 全域 `.aiterm-btn` 系統，這次靠變數覆寫沿用它，不重新定義）
