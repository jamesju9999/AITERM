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

**不改 `store::create_task` 的簽章。** 這個函式在 `src-tauri/src/tasks/`、`commands/tasks.rs`、`projects/`底下有六十幾個呼叫點（`dispatch.rs`/`scheduler.rs`/`migrate.rs`/`projects/mod.rs` 的測試都直接呼叫它），加一個必填參數等於逼所有跟 Label 完全無關的檔案跟著改，不是這次改動該碰的範圍。改用現有的「先 create 再另外 set」模式——`use_bridge`/`bridge_tiers` 就是這樣做的（`commands/tasks.rs:83-95`：`create_task` 之後另外呼叫 `store::set_bridge_config`）：

新函式 `store::set_label`（放在 `set_parallel_ok`/`set_interactive` 旁邊）：

```rust
/// 設定卡片的 Label（分類用自由文字，`None` 代表清空）。跟
/// `set_parallel_ok`/`set_interactive`/`set_bridge_config` 同一種「建立後
/// 另外設定的欄位」模式，不擠進 `create_task` 的必要參數清單。
pub async fn set_label(pool: &SqlitePool, id: &str, label: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET label = ? WHERE id = ?")
        .bind(label)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

`clone_task_fields`（`store.rs:109`）在既有的 `create_task(...)` 呼叫之後，多一步 `set_label(pool, &new_id, src.label.as_deref())`——重新排隊時保留原本的分類；跟 `use_bridge`/`bridge_tiers` 目前**不會**被複製剛好相反，因為 Label 純粹是分類標記、沒有帳號/費用面的顧慮，複製過去沒有風險。

**`store::update_task_fields`**（`store.rs:380`）簽章加 `label: Option<&str>`，`UPDATE` 語句加上 `label = ?`。這個函式目前只有 `commands/tasks.rs:134` 一個呼叫點、沒有任何既有測試直接呼叫它，改簽章不會波及其他檔案。Label 跟 `title`/`body`/`project_dir` 綁在同一個函式、同一個 `edit_allowed` 閘門（`commands/tasks.rs:17`，只有 `status == planning` 時可編輯），是同等級的「可編輯詮釋欄位」。

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

**`commands/tasks.rs`**：`CreateArgs`/`UpdateArgs`（分別在 `:64`、`:100`）加 `pub label: Option<String>`。`tasks_create`（`:76-98`）在既有的 `store::set_bridge_config(...)` 呼叫旁邊多一行 `store::set_label(&p.pool, &id, args.label.as_deref())`。`tasks_update`（`:112-146`）呼叫 `update_task_fields` 時多傳 `args.label.as_deref()`。新增指令 `tasks_used_labels`，鏡射 `tasks_used_dirs`（`commands/tasks.rs:460`）：

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

### 色相雜湊（新檔案 `src/components/TaskBoard/labelColor.ts`）

跟 `refinePrompts.ts`/`reportPrompts.ts` 同一種「獨立純函式檔案＋同目錄同名測試」慣例，放在 `TaskBoard/` 目錄下而非全域 `src/lib/`——目前只有這個目錄底下的元件會用到。

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

新增一個純函式，放進獨立檔案 `src/components/TaskBoard/groupByLabel.ts`（跟 `refinePrompts.ts` 同一種慣例：純邏輯不掛在元件裡，方便直接單元測試，不用透過 React Testing Library 掛整個 `ProjectBoard`）：

```ts
import type { TaskWithAttachments } from "../../ipc/tasks";

export interface LabelGroup { label: string; cards: TaskWithAttachments[] }

export function groupByLabel(cards: TaskWithAttachments[]): {
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
- `set_label`：寫入後 `get_task` 讀得到；傳 `None` 能清空既有值
- `clone_task_fields`：來源卡片有 `label` 時，複製出來的新卡片也有同樣的 `label`
- `update_task_fields` 帶 `label` 時正確更新
- 舊資料庫遷移（沒有 `label` 欄位的既有 DB 跑 `init_schema` 之後）：欄位補上、預設 `NULL`、`set_label` 事後可正常寫入——比照既有的 `init_schema_migrates_a_database_that_predates_the_session_columns`
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

## 追加範圍（第一輪實作完成、真機測試後使用者提出）

第一輪做完、在真機（`tauri:dev`）上驗證分組畫面時，使用者發現一個沒被涵蓋到的情境：**同一狀態欄裡的卡片要怎麼換到別的 Label 群組？** 原設計裡這件事只能透過「編輯工作」改 Label 文字達成，而編輯工作只對 `planning` 卡片開放（跟 `title`/`body`/`project_dir` 共用 `edit_allowed` 閘門）——所以 queued/running/done 的卡片完全無法再改 Label，也就無法換組。使用者確認要補兩件事：

1. **同一欄內拖曳卡片到另一個群組，直接完成換組**（比手動改文字直覺）。
2. **queued/running/done 的卡片也能事後補改/更改 Label**（目前只有 planning 卡片能編輯任何欄位）。

### 設計：不動既有的 `update_task_fields`/`edit_allowed` 閘門

`title`/`body`/`project_dir` 維持原樣——只有 `planning` 能編輯，這條線沒有理由跟著鬆綁。Label 從語意上就跟這三個不一樣（純分類標記，沒有「派工後改了會有風險」的顧慮），所以拆成獨立路徑：

- 後端新增一個不受 `edit_allowed`限制的指令 `tasks_set_label`，直接包 `store::set_label`（已存在，Task 1 就做了，本來就沒有狀態限制）：

  ```rust
  /// 直接設定卡片的 Label，不受 `edit_allowed` 限制——跟
  /// `set_parallel_ok`/`set_interactive`/`set_bridge_config` 同一個「隨時可改」
  /// 類別。給拖曳換組跟「已派工/已完成卡片事後補改 Label」這兩個情境用，
  /// 這兩者都不該連帶開放 title/body/project_dir 的編輯。
  #[tauri::command]
  pub async fn tasks_set_label(
      project_id: String,
      task_id: String,
      label: Option<String>,
      reg: State<'_, ProjectRegistry>,
      app: AppHandle,
  ) -> Result<(), String> {
      let p = project(&reg, &project_id)?;
      store::set_label(&p.pool, &task_id, label.as_deref()).await.map_err(|e| e.to_string())?;
      emit_updated(&app);
      Ok(())
  }
  ```

  跟 `tasks_create`/`tasks_update`/`tasks_move` 等使用者觸發的指令一致，呼叫 `emit_updated`（`tasks_set_summary` 沒有 emit，但那個是完成後系統內部呼叫、不是使用者直接動作，這裡不比照那個例外）。

- 前端 `setTaskLabel`（`src/ipc/tasks.ts`），完全比照 `setSummary` 的呼叫慣例（`invoke("tasks_set_summary", { projectId, taskId: id, summary })`）：

  ```ts
  export const setTaskLabel = (projectId: string, id: string, label: string | null): Promise<void> =>
    invoke("tasks_set_label", { projectId, taskId: id, label });
  ```

`update_task_fields`/`tasks_update`/現有的 `TaskEditorDialog` 完全不改——`planning` 卡片一樣透過完整編輯視窗改 Label，跟今天沒有兩樣。新路徑只服務「非 planning 狀態」跟「拖曳」這兩個新情境，兩條路徑並存、互不干擾（都是寫同一個 `label` 欄位，語意一致，只是觸發方式跟允許的狀態不同）。

### 功能 B：queued/running/done 卡片的「編輯 Label」小視窗

新元件 `TaskLabelDialog.tsx`（跟 `ArchiveDialog.tsx`/`TranscriptDialog.tsx` 同一種「獨立小對話框」慣例）：只有一個 Label 輸入框 + 用過的 Label 快捷 chip（重用跟 `TaskEditorDialog` 一樣的 UI pattern，但不牽涉 `usedDirs`/title/body 等其他欄位），儲存呼叫 `setTaskLabel`，不呼叫 `updateTask`——不會、也不需要動到 title/body/project_dir。

`TaskCard.tsx` 新增一個 prop `onEditLabel: () => void`，在 `queued`（目前這個狀態完全沒有任何操作按鈕）、`running`、`done` 三種狀態的動作列各加一顆小按鈕（新 i18n 鍵 `board_action_edit_label`）觸發它。`planning` 不加——那個狀態已經有「編輯工作」可以改 Label，再加一顆功能重複的按鈕沒有意義。

`ProjectBoard.tsx` 新增 `labelEditingFor: string | null` state，決定要不要渲染 `TaskLabelDialog`，存檔完呼叫 `refresh()`（跟其他小動作一致）。

### 功能 A：同一狀態欄內拖曳卡片到另一個 Label 群組

延伸既有的滑鼠拖曳機制（`ProjectBoard.tsx` 的 `onMove`/`onUp`，見 `statusUnderPoint`/`isLegalDropTarget`）。核心觀察：現有邏輯只認「欄位邊界」——`isLegalDropTarget` 在 `cardRow.status === to`（同欄）時一律回傳 `false`，所以同欄內拖放今天完全没有任何效果。新增一條平行路徑，只在「同欄」這個今天被當作『非法』的情況下觸發：

- `TaskLabelGroup` 的外層 `<div>` 加 `data-task-label-group={label}`，讓拖放時能用 `elementFromPoint(...).closest("[data-task-label-group]")` 認出「這是哪個群組」。
- 新函式 `labelUnderPoint(x, y): string | undefined`——`undefined` 代表「這個位置解析不出目標群組，不要做任何事」（例如根本沒拖到同一欄裡）；空字串 `""` 代表「未分類」；非空字串代表某個 Label：
  1. 先找 `[data-task-label-group]`：命中就回傳它的 label。
  2. 再找 `[data-task-drag-id]`（任何卡片，含被拖曳的那張以外的其他卡）：命中就回傳那張卡目前的 `label ?? ""`。
  3. 再找 `[data-testid^='column-']`：命中（代表滑鼠還在某個欄位範圍內，但沒有壓在任何卡片或群組上）就回傳 `""`（拖到空白區域＝拖去「未分類」）。
  4. 都沒中：回傳 `undefined`。
- `onMove`：當滑鼠所在欄位（`hovered`）等於被拖卡片自己的 `status`（也就是同欄內移動）時，額外算出 `labelUnderPoint` 結果存進新 state `dragOverGroupLabel`，驅動 `TaskLabelGroup` 的高亮（新 prop `highlighted`，比照欄位高亮那套 class 命名）。不是同欄時這個 state 清成 `null`，跟原本的 `dragOverStatus` 邏輯彼此獨立、互不影響（`dragOverStatus` 本來就已經在同欄時維持 `null`，因為 `isLegalDropTarget` 同欄回傳 `false`）。
- `onUp`：
  ```ts
  const to = statusUnderPoint(e.clientX, e.clientY);
  const draggedCard = tasks.find((x) => x.id === st.id);
  if (to && draggedCard && to === draggedCard.status) {
    const targetLabel = labelUnderPoint(e.clientX, e.clientY);
    if (targetLabel !== undefined && targetLabel !== (draggedCard.label ?? "")) {
      void handleRelabel(st.id, targetLabel || null);
    }
  } else if (to) {
    void handleDrop(st.id, to);
  }
  ```
  `handleRelabel` 呼叫 `setTaskLabel` 後直接用 `setTasks` 樂觀更新本地那一筆的 `label`（跟 `handleDrop` 對 `status`/`sort_order` 的樂觀更新同一個寫法），不必等下一輪 `tasks-updated` 事件才刷新畫面。

### 明確不做（這輪追加也排除）

- 不做「拖去空白處＝清空 Label」以外更細緻的視覺回饋（例如空白區域也高亮）——功能上仍然正確（放開就會變未分類），只是沒有額外提示，先接受這個簡化。
- `planning` 狀態不加「編輯 Label」小按鈕——已經有完整編輯視窗涵蓋，不重複。
- 不做「跨欄同時拖曳換狀態又換組」的複合手勢——一次拖曳只認一種結果：同欄比對群組、跨欄比對欄位，兩者互斥（`to === draggedCard.status` 已經是互斥判斷式本身）。
