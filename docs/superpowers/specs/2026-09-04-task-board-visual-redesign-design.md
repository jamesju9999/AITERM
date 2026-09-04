# 工作看板視覺重新設計 — 設計

日期：2026-09-04
狀態：待使用者複審

## 問題

工作看板（Task Board）現有畫面功能正常，但視覺完全沒有設計感：卡片是純平面深色方塊、按鈕是完全無樣式的原生 `<button>`（沒有套用 app 其他對話框已經在用的 `.aiterm-btn` 系統）、四欄版面標題/計數/badge 都擠在一起，狀態（執行中/成功/失敗）沒有一眼可辨的視覺區分。

透過瀏覽器輔助工具跟使用者來回看過幾輪配色跟卡片方向，最終定案如下。

## 現況調查

| 事實 | 位置 |
|---|---|
| `TaskCard.tsx` 的所有 `<button>` 完全沒有 className，是無樣式原生按鈕 | `src/components/TaskBoard/TaskCard.tsx` |
| `TaskEditorDialog.tsx`/`TranscriptDialog.tsx` 已經在用 `.aiterm-btn`/`.aiterm-btn--primary`/`.aiterm-btn--secondary`/`.aiterm-btn--ghost` | 同上兩個檔案 |
| `.aiterm-btn--primary` 用 `var(--accent-gradient, linear-gradient(135deg, var(--accent, #a855f7), #6366f1))`——`--accent` 不是寫死的紫色，是使用者可在設定頁切換的**主題色**（`src/lib/themes.ts` 定義了好幾組主題，實測使用者目前作用中的主題 `--accent` 是 `#34d399`青綠色，不是原本以為的紫色） | `src/styles/buttons.css`、`src/lib/themes.ts` |
| 因為 `--accent` 隨使用者選的主題變動，工作看板若要「不管使用者選哪個主題、看起來都是同一種藍」，不能直接沿用 `var(--accent)`，需要工作看板自己一組獨立、寫死的色彩變數 | 同上 |
| `TaskColumn.tsx` 的 `highlighted` 用 `.task-column--drop-target`，目前是紫色系（`var(--accent, #a855f7)`），同樣要換 | `src/components/TaskBoard/TaskColumn.tsx`、`index.css` |
| `TaskEditorDialog`/`TranscriptDialog` 在 JSX 裡是 `.task-board` 這個根 `<div>` 的子元素（跟 `.task-board-columns`同層），不是獨立掛在別處——把新色彩變數定義在 `.task-board` 上，兩個對話框可以直接透過 CSS 變數繼承拿到，不用另外複製一份數值 | `src/components/TaskBoard/index.tsx` |
| 拖曳中的 ghost 卡片是 `createPortal` 掛到 `document.body`（`.task-card-ghost`），今天稍早才修過它漏繼承 `user-select:none` 的問題——新增傾斜效果一樣要直接寫在這個 class 上 | `src/components/TaskBoard/index.tsx`、`index.css` |

## 範圍

**含（工作看板整個範圍統一套用新配色，不是只有卡片）：**
- 卡片本體（`TaskCard.tsx` + `index.css` 卡片相關 class）
- 四欄版面（`TaskColumn.tsx`，欄位標題/計數/拖放高亮）
- 新增/編輯工作對話框（`TaskEditorDialog.tsx`）
- 對話記錄對話框（`TranscriptDialog.tsx`）
- 拖曳中的 ghost 卡片傾斜效果（`index.tsx`/`index.css`）

**不含：**
- app 其他地方（分頁列、終端機、設定頁等）的配色——維持使用者自選的主題色不動，這次是工作看板專屬的一套獨立配色，不影響/不依賴全域 `--accent`
- 拖曳邏輯本身（判斷可不可以放、哪個欄位合法）——今天稍早已經修過，這次純視覺，不動任何行為邏輯
- 新增任何目前沒有的功能性 UI 元素（例如排序、篩選、搜尋）——只做既有元素的視覺重製

## 設計

### 色彩

工作看板專屬、寫死的一組 CSS 變數，定義在 `.task-board` 這個根容器上（讓底下所有子元素，包含兩個對話框，都能直接繼承用）：

```css
.task-board {
  --tb-bg: #0a0e14;
  --tb-card-bg-top: #141a24;
  --tb-card-bg-bottom: #10151d;
  --tb-border: #1f2733;
  --tb-text: #eef2f7;
  --tb-text-muted: #6b7a8f;
  --tb-accent: #2f6fed;
  --tb-accent-dark: #1e4fc7;
  --tb-accent-dim: rgba(47, 111, 237, 0.14);
  --tb-success: #22c55e;
  --tb-success-dim: rgba(34, 197, 94, 0.14);
  --tb-failed: #ef4444;
  --tb-failed-dim: rgba(239, 68, 68, 0.14);
  --tb-neutral: #3a4556;
}
```

（實際數值來自跟使用者在瀏覽器輔助工具裡反覆確認過的版本，不是隨意挑的。）

### 卡片

- 卡片背景改上下漸層（`--tb-card-bg-top` → `--tb-card-bg-bottom`），圓角加大到 12px，陰影加深。
- 左側 3px 細條依狀態變色：執行中＝`--tb-accent`（另外加一圈淡淡的同色 `box-shadow` 當發光效果）、成功＝`--tb-success`、失敗＝`--tb-failed`、計畫中/待執行＝`--tb-neutral`。
- 互動模式的徽章改成一個 24×24 圓角圖示 chip（漸層藍底 + 👤），放在標題列右側，取代原本純文字 badge。
- 「執行中」狀態徽章加一個會跳動（CSS `@keyframes` 透明度呼吸）的小圓點，一眼看出「還在跑」跟「已經結束」的差異。
- 徽章一律改成藥丸形（`border-radius: 20px`）、淡色底+同色系文字（不是實心色塊）。
- 資訊區（標題/路徑/徽章）跟動作按鈕區之間加一條淡淡的分隔線。
- 路徑前面加一個 📁 圖示，字體改等寬（`var(--mono)`），跟標題的視覺重量拉開差距。

### 按鈕

工作看板專屬的按鈕樣式（不是重新定義全域 `.aiterm-btn`，避免影響 app 其他地方；命名比照既有 `.aiterm-btn--*` 修飾詞的模式，但獨立成 `.tb-btn--*`）：
- 次要動作（停止/開啟分頁/對話記錄/重新派工）：`.tb-btn--ghost`，透明底 + 細邊框 + 淺灰字。
- 主要動作（標記完成/儲存）：`.tb-btn--primary`，`--tb-accent` → `--tb-accent-dark` 漸層底、白字、淡藍色 `box-shadow`。
- 危險動作（刪除）：`.tb-btn--danger-ghost`，透明底 + 淡紅邊框 + 淡紅字（不是實色紅底，跟其他次要按鈕視覺量感一致，只是用顏色標示危險性）。
- 按鈕形狀改小圓角矩形（7px），跟卡片本身的圓角語言一致。

### 欄位（四欄版面）

- 欄位標題文字縮小、全大寫、加字距，弱化成「分類標籤」的視覺角色，不跟卡片標題搶注意力。
- 拖放時的高亮邊框/背景色改用 `--tb-accent`（原本是紫色系 `var(--accent)`）。

### 對話框（新增/編輯工作、對話記錄）

- 邊框、輸入框 focus 狀態、主要按鈕（儲存）全部改用上面這組 `--tb-*` 變數，不再吃 app 全域 `--accent`。
- 「互動模式」/「並行執行」checkbox 的 `accent-color` 也改成 `--tb-accent`。
- 其餘版面結構（欄位順序、間距）不變，這次是換色 + 按鈕樣式，不是重新排版整個對話框。

### 拖曳中的傾斜效果

`.task-card-ghost`（`createPortal` 到 `document.body` 的那個跟隨游標的卡片）加：

```css
.task-card-ghost {
  transform: translate(-50%, -16px) rotate(-8deg) scale(1.04);
  transition: transform 0.15s ease-out;
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(47, 111, 237, 0.2);
}
```

放開滑鼠的瞬間這個元素就從 DOM 移除（現有邏輯），不需要額外處理「彈回」動畫。

## 已知限制

- 這次只做工作看板這一個功能區的配色，跟 app 其他地方（使用者自選的主題）會不一樣——這是使用者明確要的（統一工作看板內部，不影響全域主題），不是遺漏。
- 顏色數值是深色模式（app 目前唯一有的模式）下決定的；如果 app 之後支援淺色模式，工作看板這組寫死的深色變數需要另外處理，這次不含。

## 測試

視覺改動為主，沒有邏輯變更，既有的行為測試（拖曳、按鈕點擊呼叫哪個 ipc 函式等）應該完全不受影響、不需要修改斷言。驗證方式：
- 跑一次既有的 `TaskBoard` 測試群組，確認全部通過（class name 改變不影響既有測試，因為測試斷言的是文字內容/ipc 呼叫，不是 CSS class）。
- `npx tsc -b`/`npm run lint` 確認乾淨（CSS 檔案本身不會被這兩個工具檢查，但改動會牽動 `.tsx` 檔案的 className 屬性，仍要跑）。
- 手動視覺驗證：實際跑 app，四個欄位各放一張卡片（含至少一張互動模式、一張成功、一張失敗），對照瀏覽器輔助工具裡定案的畫面，確認顏色/圓角/間距/徽章/分隔線都對得上；實際測一次拖曳，確認 -8° 傾斜有出現、放開後正常消失；打開新增/編輯工作對話框跟對話記錄對話框，確認邊框/按鈕/checkbox 都換成新色。

## 相關

`docs/superpowers/specs/2026-07-16-unified-button-system-design.md`（app 全域 `.aiterm-btn` 系統的出處，這次工作看板刻意不共用它的 `--accent`，另外建一組獨立變數）
