# 工作看板專案化 — 設計規格

**日期：** 2026-09-04
**狀態：** 已核准，待寫實作計畫
**里程碑範圍：** 只做專案基礎建設。AI 工作報告與跨機器匯入的進階處理各自獨立成後續里程碑。

---

## 1. 目標

工作看板目前是一份扁平的全域工作清單：`tasks` 表每一列自帶一個 `project_dir`，看板直接把所有卡片依 status 分成四欄。

本里程碑導入「專案」這一層：

- 使用者先建立專案，才能進入工作看板。看板永遠是**某一個專案的**看板。
- 一個專案底下可以有很多工作，而這些工作可以散布在不同的工作目錄。
- 專案是磁碟上一個自成一體的資料夾。複製資料夾就等於匯出，開啟 `.aitprj` 就等於匯入。

## 2. 核心決策

| # | 決策 | 理由 |
|---|---|---|
| D1 | 專案**不綁定**工作目錄 | 一個專案的工作可能橫跨多個 repo（前端 repo + 後端 repo） |
| D2 | 專案是磁碟資料夾，含 `.aitprj` 清單檔與**自己的 `tasks.db`** | 讓「匯出／匯入」與「專案資料夾」變成同一個功能，不必實作兩次 |
| D3 | 資料夾位置由使用者在建立時指定 | 使用者可放進 iCloud／Dropbox／git repo，直接分享給另一台 AITerm |
| D4 | 可同時開啟多個專案 | 排程器跨所有專案派工 |
| D5 | 並行上限維持**全域**，預設值 2 → 5，設定中仍可調整（clamp 1..16 不變） | 使用者確認：要可調 |
| D6 | `parallel_ok = false`（獨佔工作）維持**全域**生效 | 專案是純標籤，不同專案可能指向同一個 repo；跨專案同時改同一份程式碼比互相等待更糟 |
| D7 | 不做「暫停專案」開關 | 欄位已是控制點：卡片留在「規劃」欄就不派工。第二個控制點只會製造「拖到待執行卻沒動靜」的困惑 |
| D8 | 移除專案時**詢問**是否連同磁碟資料夾刪除 | 使用者確認：要問 |
| D9 | 既有卡片自動搬遷到「預設專案」，舊檔案**複製而非搬移** | 搬遷若有 bug，原始資料仍在原地 |

## 3. 磁碟結構

```
~/Projects/makemoney/            ← 專案資料夾（位置由使用者指定）
   makemoney.aitprj              ← 專案清單檔（JSON）
   tasks.db                      ← 這個專案的卡片
   tasks/<task_id>/
      attachments/
      transcript.txt
```

`.aitprj` 內容：

```json
{
  "schema": 1,
  "id": "9f2c8d14-0b3a-4e77-9a51-6c2f0d8e4b12",
  "name": "makemoney",
  "description": "",
  "created_at": "2026-09-04T10:00:00Z"
}
```

- `id` 是 UUID 而非路徑。資料夾改名或搬家後，專案仍是同一個。
- `name` 是顯示名稱，與資料夾名稱獨立。重新命名專案只改這個欄位，不動磁碟路徑。
- `schema` 現在就寫入，供後續里程碑判斷跨機器匯入的格式相容性。本里程碑只認 `1`。
- `tasks.db` 的 schema 與現行 `tasks/mod.rs::init_schema` **完全相同**，一個字都不改。`store.rs` 的所有函式原樣沿用，只是換一個連線池。

## 4. 後端架構

### 4.1 新增模組

| 檔案 | 職責 |
|---|---|
| `src-tauri/src/projects/mod.rs` | `ProjectRegistry`：`id → { path, name, pool }`。建立、開啟、移除、讀寫 `.aitprj` |
| `src-tauri/src/projects/migrate.rs` | 舊 `tasks.db` 的一次性搬遷（見 §6） |
| `src-tauri/src/commands/projects.rs` | `projects_*` 指令 |

### 4.2 `ProjectRegistry` 取代 `TasksDb`

現行 `TasksDb` 是 Tauri 管理的單例，持有唯一一個 `SqlitePool`（`tasks/mod.rs`）。它被 `lib.rs`、`tasks/scheduler.rs`、`commands/tasks.rs`、`tests/task_board.rs` 使用。

改為：

```rust
pub struct OpenProject {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub pool: SqlitePool,
}

pub struct ProjectRegistry {
    open: RwLock<HashMap<String, OpenProject>>,
}
```

啟動時：讀取設定中的專案路徑清單 → 逐一載入 `.aitprj` → 開啟連線池 → 執行 `init_schema`。載入失敗的專案不會擋住其他專案（見 §7）。

### 4.3 專案清單的持久化

專案路徑清單存在既有的設定檔（`config/types.rs` 的 `TaskBoardConfig`）：

```rust
pub struct TaskBoardConfig {
    pub max_concurrent: u32,     // 預設 2 → 5
    pub claude_command: String,
    #[serde(default)]
    pub project_paths: Vec<String>,   // 專案資料夾的絕對路徑
}
```

只存路徑。名稱與 id 每次啟動時從 `.aitprj` 讀取——這樣使用者在 Finder 裡改了專案檔，App 下次啟動就會看到。

### 4.4 指令層

`commands/tasks.rs` 的 12 個 `tasks_*` 指令**各加一個 `project_id` 參數**，改為從 registry 取得連線池。

`tasks/mod.rs::task_dir(task_id)` 改為 `task_dir(project_path, task_id)`。呼叫點共 4 處：`scheduler.rs:121`、`commands/tasks.rs:220`、`:245`、`:304`。

新增 `commands/projects.rs`：

| 指令 | 行為 |
|---|---|
| `projects_list` | 回傳所有已知專案：`{ id, name, description, path, status, counts }`。`status` 為 `ok \| missing \| invalid \| incompatible`；`counts` 為 `{ planning, queued, running, done }` 四欄各自的卡片數（非 `ok` 狀態的專案其 `counts` 全為 0） |
| `projects_create` | 參數 `{ parent_dir, name, description }`。建資料夾 → 寫 `.aitprj` → 建 `tasks.db` 跑 `init_schema` → 加入清單並開啟 |
| `projects_open` | 參數 `{ aitprj_path }`。驗證後加入清單並開啟。**這就是匯入** |
| `projects_remove` | 參數 `{ id, delete_folder: bool }`。從清單移除；`delete_folder` 為真時一併刪除磁碟資料夾 |
| `projects_rename` | 參數 `{ id, name, description }`。只改 `.aitprj`，不動路徑 |
| `projects_used_dirs` | 參數 `{ id }`。回傳該專案卡片用過的工作目錄（`SELECT DISTINCT project_dir`），供新增工作時快捷選取 |

### 4.5 排程器

`tasks/scheduler.rs::drain_once` 現行簽名接收單一 `&TasksDb`。改為接收 `&ProjectRegistry`：

1. 跨所有開啟的專案收集 `running` 卡片（每筆帶 `project_id`）
2. 跨所有開啟的專案收集 `queued` 卡片，依 `sort_order` 排序後串接
3. 呼叫既有的 `pick_next()`，全域上限與獨佔規則的語意完全不變（D5、D6）
4. 派工時帶著該卡片的 `project_id`，讓 dispatcher 拿到正確的專案路徑

`pick_next()` 這個純函式的**邏輯與簽名皆不變**，只是輸入從單一專案的卡片變成跨專案的聯集。既有的單元測試因此仍然有效。

### 4.6 監控器

`tasks/monitor.rs` 目前透過 `TasksDb` 寫回完成結果。改為攜帶 `project_id`，寫回時向 registry 查詢連線池。若專案在監控期間被移除，寫回失敗僅記錄錯誤，不使流程崩潰。

## 5. 前端架構

### 5.1 兩層導覽

`src/components/TaskBoard/index.tsx` 從「看板」變成「路由器」：

```
沒選專案 → <ProjectList />
選了專案 → <ProjectBoard projectId={...} />
```

| 檔案 | 職責 |
|---|---|
| `TaskBoard/index.tsx` | 只保留兩層之間的切換狀態 |
| `TaskBoard/ProjectList.tsx` | 專案總覽：卡片列出名稱、描述、工作數、執行中數；「+ 新專案」「開啟現有專案」；空狀態 |
| `TaskBoard/ProjectCreateDialog.tsx` | 名稱 + 描述 + 父目錄選擇 |
| `TaskBoard/ProjectBoard.tsx` | 現行 `index.tsx` 的四欄看板本體原封不動搬入，加上返回鍵與專案名標題 |

`TerminalApp.tsx` **不需要任何修改**——它只掛一個 `<TaskBoardView />`（`TerminalApp.tsx:668`），兩層導覽全在其內部。

### 5.2 建立專案流程

輸入名稱 + 挑父目錄 → 產生 `<父目錄>/<名稱>/<名稱>.aitprj`。

- 父目錄預設帶上次使用的位置，沿用 `TaskEditorDialog` 的 `localStorage` 做法（`aiterm_last_task_dir` 的同款模式，另用一個 key）。
- 名稱含有檔案系統非法字元時，資料夾名做安全轉換，`.aitprj` 內的 `name` 保留原字串。
- 目標資料夾已存在且非空時拒絕建立並說明。

### 5.3 移除專案（D8）

沿用 `TaskCard.tsx` 既有的兩段式原生對話框模式：

1. `confirm("確定要移除專案「X」嗎？")`
2. 若確認 → `confirm("要連同磁碟上的資料夾一起刪除嗎？此動作無法復原。")`
3. 依第二題的答案呼叫 `projects_remove({ id, delete_folder })`

**必須使用 `@tauri-apps/plugin-dialog` 的非同步 `confirm()`，不可使用 `window.confirm`** — Tauri 的 webview 沒有真正實作 `window.confirm`（見 `TaskCard.tsx:31` 的註解）。同時，這些對話框位於點擊處理器中，不可放進掛載 effect（StrictMode 會雙重呼叫）。

### 5.4 新增工作時的工作目錄快捷選項

`TaskEditorDialog` 的目錄選擇器，除了「瀏覽…」外，額外列出 `projects_used_dirs` 回傳的目錄清單。這解決 D1（工作橫跨多個目錄）帶來的重複挑選摩擦。

### 5.5 IPC 層

- 新增 `src/ipc/projects.ts`，型別鏡射 Rust 端。
- `src/ipc/tasks.ts` 的每個函式增加 `projectId` 參數。

## 6. 既有資料搬遷

啟動時，若同時滿足：`<資料區>/AITERM/tasks.db` 存在、其中有卡片、且尚無搬遷標記——

1. 建立 `<資料區>/AITERM/projects/預設專案/`（搬遷時使用者不在場，故此處不讓使用者挑位置）
2. **複製**（非搬移）`tasks.db` 與 `tasks/` 目錄進去
3. 寫出 `.aitprj`（新 UUID、`name` 為「預設專案」）
4. 將路徑加入設定的 `project_paths`，寫下搬遷標記
5. 舊檔案原地保留不刪

搬遷標記為 `<資料區>/AITERM/.projects_migrated` 這個檔案。以磁碟上的標記檔而非設定欄位表示，是為了讓「設定檔被重置」不會導致資料被重複搬遷一次。

搬遷失敗時記錄錯誤並不寫標記，App 仍可正常啟動（使用者會看到空的專案清單，舊資料未受損）。

## 7. 錯誤處理

| 情況 | 行為 |
|---|---|
| 資料夾被刪除或搬走 | 該專案在清單中標示為「遺失」，可移除或重新指定位置。其他專案不受影響 |
| `.aitprj` 損毀或非合法 JSON | 標示為「無法讀取」，其他專案照常運作 |
| `schema` 版本大於本程式支援 | 標示為「版本不相容」並拒絕開啟，不嘗試猜測格式 |
| 開啟的專案 id 已在清單中 | 拒絕並提示已存在。（同一專案的複本如何併存，留待後續匯入里程碑處理） |
| 有工作執行中時移除專案 | 擋下並說明「請先停止執行中的工作」 |
| `tasks.db` 開啟失敗 | 該專案標示為錯誤。**不可回退成 in-memory 資料庫**——現行 `TasksDb::new()` 有這個回退，會靜默吃掉資料，新程式碼不沿用 |

## 8. 測試策略

**Rust**

- `projects/mod.rs`：建立、開啟、移除、重新命名；`.aitprj` 損毀／版本不相容／資料夾遺失各自的處理（`tempfile`）
- `projects/migrate.rs`：有舊資料時搬遷成功且原檔保留；已有標記時不重複搬遷；無舊資料時無動作
- `scheduler.rs`：跨專案聯集後 `pick_next` 仍遵守全域上限與獨佔規則
- 既有 `src-tauri/tests/task_board.rs` 改寫為建立臨時專案，而非使用全域資料庫

**前端**

- `ProjectList`：空狀態、專案清單渲染、進入看板的導覽、遺失專案的呈現
- `ProjectCreateDialog`：名稱驗證、父目錄選擇
- 移除專案的兩段式對話框：三種路徑（取消／只移除／連同資料夾刪除）各驗證一次
- 既有 `TaskBoard/index.test.tsx`（751 行）需加上專案層；`TerminalApp.taskBoard.test.tsx` 的 mock 需更新

**驗證指令**（依 CLAUDE.md）

```bash
npx tsc -b                     # 型別檢查（不可用 tsc --noEmit）
npm run test                   # 前端 Vitest
cd src-tauri && cargo test     # 完整 Rust 測試，不可只跑 --lib
```

## 9. 跨平台考量

- 路徑處理一律使用 `PathBuf`，不得手寫 `/` 分隔字串。
- 專案名稱轉資料夾名時，需處理 Windows 的非法字元（`< > : " / \ | ? *`）與保留名稱（`CON`、`PRN`、`AUX`、`NUL`、`COM1`–`COM9`、`LPT1`–`LPT9`）。
- 刪除資料夾使用 `std::fs::remove_dir_all`，跨平台可用。

## 10. 風險

這是 breaking-change 等級的後端重構。功能面看起來只是「多一層專案」，實際更換的是資料存取的地基：

- 12 個 `tasks_*` 指令的簽名全部改變
- 排程器與監控器的資料來源從單例改為 registry 查詢
- 既有 Rust 整合測試 `tests/task_board.rs` 需改寫
- 前端 `TaskBoard/index.test.tsx`（751 行）需大幅調整

實作時應先完成後端 registry 與遷移並讓 Rust 測試全綠，再動前端，避免兩端同時破裂難以定位。

## 11. 本里程碑不做

- **AI 工作報告**（獨立里程碑）
- **跨機器匯入的進階處理**：`project_dir` 絕對路徑在另一台機器不存在時的重新對應、同一專案複本的 id 衝突合併。本里程碑只提供「開啟 `.aitprj`」這個基本匯入路徑
- 專案層級的並行上限（維持全域，D5）
- 暫停／關閉專案（D7）
- 專案排序、標籤、搜尋
