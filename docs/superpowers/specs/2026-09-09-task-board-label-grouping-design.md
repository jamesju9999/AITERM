# 工作看板卡片 Label 分組 — 設計

日期：2026-09-09
狀態：待使用者複審

## 問題

工作看板的四個狀態欄（planning/queued/running/done）裡，卡片是一條扁平的清單，卡片一多就很難一眼看出「哪些卡片彼此相關」。使用者想要能在同一個狀態欄內把卡片依某種分類概念分群組顯示。

## 決策過程（brainstorming 已確認）

- **分類依據**：新增一個卡片欄位 `label`（單一值、自由輸入文字），不做多值 tags——多值會讓「這張卡屬於哪個群組」變成一對多，拖曳、`sort_order` 語意都會變得曖昧。也不做「跨欄泳道（同一 Label 在四欄對齊成同一列）」——結構改動太大，且使用者確認過 Label 只要跟著卡片自然移動即可，不需要跨欄視覺對齊。
- **套用範圍**：全部四欄（planning/queued/running/done）都套用同一套分組規則。
- **未分類卡片**：不成組，維持原本排序，顯示在該欄最上面（分組是加法，不改變舊有行為）。
- **群組排序**：依「該 Label 在這一欄第一次出現的 `created_at`」由舊到新排——順序穩定，不會因為卡片增減而跳動（跟依數量排序相比）。
- **摺疊狀態**：不持久化，純 React state，每次掛載都全部展開。
- **搜尋**：現有的看板即時搜尋（`title`/`body`/`project_dir`）與封存清單搜尋都要一併比對 `label`。
- **視覺樣式**：Label 用字串雜湊出一個色相值，同一個 Label 永遠同色；不用固定色票（自由文字沒有固定語意），淺色/深色主題都要能正常顯示。

## 資料模型

`tasks` table 新增一個 nullable 欄位：

```sql
ALTER TABLE tasks ADD COLUMN label TEXT
```

沿用 `src-tauri/src/tasks/mod.rs` 既有的 best-effort 遷移寫法（跟 `ai_summary`/`archived_at`/`session_id` 等欄位同一套模式，`let _ = sqlx::query(...)`，欄位已存在時的錯誤刻意丟掉）。`init_schema` 的 `CREATE TABLE IF NOT EXISTS tasks (...)` 也要把 `label TEXT` 加進完整欄位清單，讓全新資料庫一次到位。

`TaskRow`（`src-tauri/src/tasks/store.rs:15`）加一個欄位：

```rust
/// 使用者自由輸入的分類文字，用來在同一狀態欄內把卡片分組顯示。
/// `None` 代表未分類。
pub label: Option<String>,
```

## 後端改動

**`store::create_task`**（`store.rs:73`）簽章加 `label: Option<&str>`，`INSERT` 語句加上 `label` 欄位與對應 bind；`clone_task_fields`（`store.rs:109`）也要把來源卡片的 `label` 一併複製過去，跟現有複製 `project_dir`/`parallel_ok`/`interactive` 同一個邏輯。

**`store::update_task_fields`**（`store.rs:380`）簽章加 `label: Option<&str>`，`UPDATE` 語句加上 `label = ?`——跟 `title`/`body`/`project_dir` 綁在同一個函式、同一個 `edit_allowed` 閘門（`commands/tasks.rs:17`，只有 `status == planning` 時可編輯），Label 是同等級的「可編輯詮釋欄位」。

**新函式 `store::distinct_labels`**，完全鏡射現有的 `distinct_project_dirs`（`store.rs:579`）：

```rust
/// 這個專案的卡片用過的 Label，去重複＋排序。跟 `distinct_project_dirs`
/// 同一個用途——新增/編輯卡片時给一鍵選取，不必每次重新手打。
pub async fn distinct_labels(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT label FROM tasks WHERE label IS NOT NULL AND label <> '' ORDER BY label",
    )
    .fetch_all(pool)
    .await
}
```

**`commands/tasks.rs`**：`CreateArgs`/`UpdateArgs`（分別在 `:64`、`:100`）加 `pub label: Option<String>`；`tasks_create`/`tasks_update` 呼叫 `create_task`/`update_task_fields` 時多傳這個參數。新增指令 `tasks_used_labels`，鏡射 `tasks_used_dirs`（`commands/tasks.rs:460`）：

```rust
#[tauri::command]
pub async fn tasks_used_labels(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<String>, String> {
    let p = project(&reg, &project_id)?;
    store::distinct_labels(&p.pool).await.map_err(|e| e.to_string())
}
```

在 `lib.rs` 的 command 註冊清單（`:115`、`:597` 附近，`tasks_used_dirs` 旁邊）加上這個新指令。

**封存搜尋**（`store.rs:326` 的 `ARCHIVED_WHERE`）加一個比對條件：

```rust
const ARCHIVED_WHERE: &str = "archived_at IS NOT NULL AND (
        ?1 = ''
        OR title       LIKE ?2 ESCAPE '\\'
        OR body        LIKE ?2 ESCAPE '\\'
        OR project_dir LIKE ?2 ESCAPE '\\'
        OR label       LIKE ?2 ESCAPE '\\'
    )";
```

`label` 是 nullable：SQLite 對 `NULL LIKE anything` 求值是 `NULL`（在 `WHERE` 裡等同「不成立」），未分類的封存卡片不會被誤判成任何關鍵字都比對得到，不需要額外的 `COALESCE`。

## 前端改動

### IPC 型別（`src/ipc/tasks.ts`）

`TaskRow`/`TaskWithAttachments` 加 `label: string | null`；`createTask`/`updateTask` 的參數物件加 `label: string | null`。

### 新增 `usedLabels`（`src/ipc/projects.ts`）

跟 `usedDirs`（`:50`）放同一個檔案——維持現有慣例（`usedDirs` 雖然語意上屬於 tasks，但現在就放在 `projects.ts`）：

```ts
export const usedLabels = (projectId: string): Promise<string[]> =>
  invoke("tasks_used_labels", { projectId });
```

### 卡片編輯（`TaskEditorDialog.tsx`）

比照現有的 `dir`/`dirChoices` 那一套（`:44`、`:86-95`、`:311-343`）：新增 `label` state（`card?.label ?? ""`）、`labelChoices` state 由 `usedLabels(projectId)` 填入、一個文字輸入框 + 下面一排「用過的 Label」chip 按鈕（點了就把值填進輸入框）。跟 `dir` 不同的地方：**不**用 `localStorage` 記上次輸入值——Label 是分類用途，沒有「預設延續上一張卡」的理由，每張新卡預設空白。UI 位置放在 `dir` 欄位那個 `task-dialog-group`（`:311-343`）之後、`parallel_ok`/`interactive` checkbox 之前，跟 `dir` 一樣獨立一個 `task-field` 區塊。

### 色相雜湊（新檔案 `src/lib/labelColor.ts`）

```ts
/** 把任意字串雜湊成一個穩定的 0–359 色相值，同一個字串永遠同色。 */
export function hashLabelHue(label: string): number {
  let h = 0;
  for (let i = 0; i < label.length; i++) {
    h = (h * 31 + label.charCodeAt(i)) | 0;
  }
  return Math.abs(h) % 360;
}
```

配色透過 CSS 自訂屬性 `--label-hue` 傳進樣式，樣式本身用 `hsl(var(--label-hue) ...)`。**不需要淺色/深色雙軌寫法**：`src/components/TaskBoard/index.css:1-18` 開頭的註解已經講清楚——整個 `.task-board`（含 portal 出去的 `.task-card-ghost`）刻意寫死一組深藍配色變數，不管使用者在設定頁選了 app 哪個主題都固定顯示同一套，跟全域的 light/dark 主題系統無關。所以 Label 徽章只要比照同檔案既有的 `.task-badge--success` 等規則（`:591-593`，直接用固定變數、沒有 media query），用同一種「單一深色配色」寫法即可：

```css
.task-label-chip {
  background: hsl(var(--label-hue) 45% 22%);
  color: hsl(var(--label-hue) 70% 78%);
}
```

固定飽和度/亮度、只變化色相，確保任何雜湊出來的色相在這組深色底下都維持足夠對比度。

### 卡片本身（`TaskCard.tsx`）

有 `label` 時，在既有的 `task-card-badges`（`:86-99`）那排多顯示一個徽章，套上面的 `.task-label-chip` 樣式跟 `--label-hue`。

### 看板分組（`ProjectBoard.tsx`）

新增一個純函式（放在元件內或獨立 util 皆可，因為只依賴傳入的卡片陣列）：

```ts
interface LabelGroup { label: string; cards: TaskWithAttachments[] }

function groupByLabel(cards: TaskWithAttachments[]): {
  ungrouped: TaskWithAttachments[];
  groups: LabelGroup[];
} {
  const ungrouped: TaskWithAttachments[] = [];
  const byLabel = new Map<string, TaskWithAttachments[]>();
  for (const c of cards) {
    const label = c.label?.trim();
    if (!label) { ungrouped.push(c); continue; }
    const arr = byLabel.get(label) ?? [];
    arr.push(c);
    byLabel.set(label, arr);
  }
  const groups = [...byLabel.entries()]
    // created_at 是 SQLite `datetime('now')` 產生的固定寬度字串
    // （'YYYY-MM-DD HH:MM:SS'），字串比較跟時間先後完全一致——刻意不用
    // `Date.parse`：Tauri 在三個平台各自嵌入不同的 WebView 引擎
    // （WebKit/WebView2/WebKitGTK），對「非 ISO 8601」日期字串的寬鬆解析
    // 行為並不保證一致，字串比較沒有這個跨平台風險。
    .map(([label, cards]) => ({
      label,
      cards,
      firstSeen: cards.reduce((min, c) => (c.created_at < min ? c.created_at : min), cards[0].created_at),
    }))
    .sort((a, b) => (a.firstSeen < b.firstSeen ? -1 : a.firstSeen > b.firstSeen ? 1 : 0))
    .map(({ label, cards }) => ({ label, cards }));
  return { ungrouped, groups };
}
```

`visibleIn(s)`（`:131`）的輸出（已經過搜尋過濾、`byStatus` 排序）餵給 `groupByLabel`；群組內卡片順序照原樣保留（不重新排序），只是被切成「未分類」跟「依 Label 分組」兩塊。

`visibleIn` 本身的過濾條件（`:135`）加入 `label`：

```ts
[c.title, c.body, c.project_dir, c.label ?? ""].some((f) => f.toLowerCase().includes(q)),
```

渲染部分：把目前 `visibleIn(s).map((cardRow) => ...)`（`:294-319`）那段抽成一個 `renderCard(cardRow)` 函式（原樣搬過去，邏輯不變），然後：

```tsx
{(() => {
  const { ungrouped, groups } = groupByLabel(visibleIn(s));
  return (
    <>
      {ungrouped.map(renderCard)}
      {groups.map((g) => (
        <TaskLabelGroup key={g.label} label={g.label} count={g.cards.length}>
          {g.cards.map(renderCard)}
        </TaskLabelGroup>
      ))}
    </>
  );
})()}
```

### 新元件 `TaskLabelGroup.tsx`

```tsx
export function TaskLabelGroup({
  label,
  count,
  children,
}: {
  label: string;
  count: number;
  children: ReactNode;
}) {
  const [collapsed, setCollapsed] = useState(false);
  return (
    <div className="task-label-group">
      <button
        type="button"
        className="task-label-group-header"
        style={{ "--label-hue": hashLabelHue(label) } as CSSProperties}
        onClick={() => setCollapsed((c) => !c)}
      >
        <span className="task-label-group-caret">{collapsed ? "▸" : "▾"}</span>
        <span className="task-label-chip">{label}</span>
        <span className="task-label-group-count">({count})</span>
      </button>
      {!collapsed && <div className="task-label-group-body">{children}</div>}
    </div>
  );
}
```

拖曳（`handleCardMouseDown`、`statusUnderPoint`、drag ghost 等）完全不用改——那些都是以 `data-task-drag-id`/`data-testid="column-*"` 為準，跟卡片被包在哪一層 DOM 結構無關；`isLegalDropTarget`/`handleDrop` 判斷的是卡片的 `status`，一樣不受分組影響。

## i18n

新增（`en` / `zh-TW` 各一份，放進 `src/lib/i18n.ts` 既有的 `board_*` 群組）：

- `board_card_label`：欄位標題（例：「Label」/「Label」——這是使用者自訂詞彙，中英文都直接用 Label 不翻譯，比照卡片其他技術性欄位名稱）
- `board_card_label_placeholder`：輸入框 placeholder（例：「例如：緊急、文件」）

不需要新增「未分類」相關字串——未分類卡片沒有群組標頭。

## 明確不做的部分

- 不做多值 tags（一張卡多個分類）——單值 Label 已經定案，多值是需要真的碰到需求再做的進階功能。
- 不做跨欄泳道（Label 在四欄之間視覺對齊成同一列）——各欄分組彼此獨立。
- 不維護一份獨立管理的 Label 清單（沒有「刪除/重新命名 Label」這種管理介面）——Label 就是卡片上的自由文字欄位，跟現有 `project_dir` 同等級。
- 不持久化摺疊狀態——每次都預設全展開。
- 不處理「同一個 Label 打錯字變成兩個群組」（例如「緊急」跟「緊急 」多一個空白）——`groupByLabel` 只用 `.trim()` 去頭尾空白，不做進一步正規化（大小寫、全形半形等），跟使用者選定的「自由輸入」方向一致：容錯是使用者自己的責任，系統不代為判斷兩個字串是否「意圖相同」。

## 測試

**Rust**（`src-tauri/src/tasks/store.rs` 既有的 `#[cfg(test)]` 區塊）：

- `distinct_labels`：去重複＋排序（比照 `distinct_project_dirs_dedupes_and_sorts`）、跳過 `NULL`/空字串（比照 `distinct_project_dirs_skips_empty_strings`）
- `create_task`/`clone_task_fields` 帶 `label` 時正確寫入/複製
- `update_task_fields` 帶 `label` 時正確更新
- `search_archived`：關鍵字比對得到 `label` 相符的封存卡片；`label` 為 `NULL` 的卡片不會被任何關鍵字誤配到

**前端**：

- `groupByLabel`（可獨立匯出成 util 測試，或透過 `ProjectBoard` 的既有測試檔案）：
  - 未分類卡片維持原序、排最前面
  - 群組依「該 Label 最早出現的 `created_at`」排序，不是依數量或字母
  - 同一群組內卡片順序沿用原本排序，不被重新洗牌
- `TaskLabelGroup`：點擊標頭切換展開/摺疊；預設展開
- `TaskCard`：有 `label` 時顯示徽章，沒有時不顯示
- `hashLabelHue`：同字串永遠同色相；回傳值落在 0–359
- `ProjectBoard` 搜尋測試延伸：關鍵字比對 `label` 能篩出卡片
- `TaskEditorDialog`：`usedLabels` 回傳的清單正確渲染成可點的 chip；點擊後填入輸入框；儲存時 `label` 正確帶進 `createTask`/`updateTask` 的呼叫參數
