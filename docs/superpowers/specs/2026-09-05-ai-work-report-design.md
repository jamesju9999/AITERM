# AI 工作報告 — 設計規格

**日期：** 2026-09-05
**狀態：** 已核准，待寫實作計畫
**前置里程碑：** 工作看板專案化（`2026-09-04-task-board-projects-design.md`，已完成並合併）

---

## 1. 目標

使用者選一個專案，讓 AI 把該專案看板中的所有工作項目整理成一份工作報告。

這是使用者最初提出的三個需求裡的最後一項（另外兩項——新增專案、專案下掛多個工作——已於 2026-09-05 完成）。

報告是一份 **HTML 文件**，透過這個 App 既有的 artifact 機制產生，存進該專案的資料夾累積成歷史。專案資料夾自成一體，所以報告會跟著專案走——複製資料夾給另一台 AITerm 時，歷史報告一起過去。

## 2. 核心決策

| # | 決策 | 理由 |
|---|---|---|
| D1 | **兩階段**：先把每張已完成卡片的對話記錄各自摘要，再把所有摘要合成報告 | 對話記錄可能每份好幾千字，一個專案十張卡就會塞爆 context。分兩階段可以講出「怎麼做的、遇到什麼問題」，而不只是「做了哪些事」 |
| D2 | 摘要**快取**在 `tasks.db` 的 `ai_summary` 欄位 | 已完成的卡片是不可變的，對話記錄不會再變，摘要只需算一次。第二次產報告的成本從 N+1 次降到 1 次 |
| D3 | 報告是 **HTML**，用既有的 artifact 機制產生 | 使用者明確指定「用目前產生 HTML 的那套方法」。重用 `artifact-html` 協定與 `ArtifactHtmlFrame`，不另造輪子 |
| D4 | 存進 `<專案>/reports/<時間戳>.html`，**累積歷史** | 與「複製資料夾＝匯出」的既有設計一致；可回頭比對進度 |
| D5 | **兩種報告風格**，產生時選 | 自我回顧與對外報告的重點完全不同（前者要講卡住在哪、後者要講成果） |
| D6 | 涵蓋**全部四欄**的卡片 | 一份報告就是專案的完整現況，也自然帶出「下一步要做什麼」 |
| D7 | 入口放**兩處**：專案總覽的卡片、專案看板的工具列 | 想快速出報告不用先點進去；人已經在看板裡也不用退出來 |
| D8 | 彈出報告視窗，側邊可切換歷史報告 | 用既有的 `ArtifactHtmlFrame` 在沙箱 iframe 呈現 |
| D9 | 一張卡的摘要失敗**不中斷**整份報告 | 十張卡因為一張失敗而全部白跑，代價太高 |
| D10 | 卡片上限 **100 張**，超過取最近的並在報告中註明 | 防止極端情況下第二階段輸入爆掉 |
| D11 | **不做**自訂提示詞 | 兩種風格夠用（YAGNI） |

## 3. 資料流程

```
使用者按「產生工作報告」→ 選風格
        ↓
  listTasks(projectId) 取得全部卡片
        ↓
  ── 第一階段：逐張補摘要（只針對 status=done 且 ai_summary 是空的）──
  readTranscript(projectId, taskId) → AI → 300 字內的履行摘要
        ↓ tasks_set_summary 寫回快取
        ↓（每張更新進度；可取消）
  ── 第二階段：合成 ──
  全部卡片的欄位 + 摘要 + 風格 → invokeAiChat(supportsArtifacts=true)
        ↓
  splitArtifactFence 從回覆中抽出 artifact-html
        ↓
  reports_save(projectId, html) → <專案>/reports/2026-09-05-1430.html
        ↓
  ReportDialog 用 ArtifactHtmlFrame 呈現
```

### 第一階段的輸入

每張已完成卡片，餵給 AI 的是：標題、內容（body）、工作目錄、結果（success/failed/cancelled）、錯誤訊息（若有）、以及對話記錄全文。

要求輸出：**300 字以內**的繁體中文摘要，講「實際做了什麼、遇到什麼問題、最後結果」。

### 第二階段的輸入

- 專案名稱與描述
- 每張卡片的：標題、狀態、結果、建立/派工/完成時間、工作目錄
- 已完成卡片的摘要（第一階段產生或從快取讀出）
- 選定的報告風格

## 4. 報告風格

兩種，產生時選：

| 風格 | 重點 |
|---|---|
| **回顧進度**（給自己） | 做到哪了、哪些卡住了、下一步該做什麼。可以講技術細節、直接點出失敗原因 |
| **工作報告**（給上司／客戶） | 這段期間完成了哪些工作、成果是什麼。語氣正式、少講技術細節、重點在產出而非過程 |

兩組提示詞放在同一個模組裡並列，方便對照維護。

## 5. 後端

### 5.1 新增指令：`src-tauri/src/commands/reports.rs`

| 指令 | 參數 | 行為 |
|---|---|---|
| `reports_save` | `project_id, html` | 寫入 `<專案>/reports/<YYYY-MM-DD-HHMM>.html`（必要時建目錄），回傳檔名。同一分鐘內重複產生時，在檔名後加 `-2`、`-3` 以免覆蓋 |
| `reports_list` | `project_id` | 回傳該專案的歷史報告，每筆含檔名、修改時間、從 HTML 的 `<title>` 取出的標題。依時間新到舊排序 |
| `reports_read` | `project_id, filename` | 讀回該份 HTML |

**`filename` 必須做路徑防護**：只接受不含路徑分隔字元與 `..` 的檔名，一律解析成 `<專案>/reports/<filename>`。前端只會傳 `reports_list` 給過的檔名，但這個指令不能假設呼叫端守規矩。

檔案 I/O 留在 command 層、不下放 `store.rs`——與 `tasks_save_transcript` 同一慣例。

### 5.2 `tasks` 表新增欄位

```sql
ALTER TABLE tasks ADD COLUMN ai_summary TEXT
```

沿用 `interactive` 欄位那次的遷移寫法（`init_schema` 裡 `let _ = sqlx::query("ALTER TABLE ...")`，已存在時忽略錯誤）。

`store.rs` 新增：

- `set_summary(pool, task_id, summary)` — 寫入
- `TaskRow` 增加 `ai_summary: Option<String>` 欄位

新增指令 `tasks_set_summary(project_id, task_id, summary)`。

**快取失效**：卡片被 `tasks_clone` 複製時不帶 `ai_summary`（新卡片還沒跑過）。卡片重新派工（從 done 拖回 planning 再跑）時，`mark_dispatched` 要清掉 `ai_summary`——舊摘要描述的是上一次的執行。

## 6. 前端

### 6.1 新檔案

| 檔案 | 職責 |
|---|---|
| `TaskBoard/reportPrompts.ts` | 兩階段、兩風格的提示詞。純字串組裝，無 I/O，好測 |
| `TaskBoard/useWorkReport.ts` | 產生流程的協調：抓卡片 → 補摘要 → 合成 → 存檔。對外暴露 `generate(style)`、`cancel()`、進度與錯誤狀態 |
| `TaskBoard/ReportDialog.tsx` | 報告視窗：`ArtifactHtmlFrame` 呈現 + 側邊歷史清單 + 「另存為…」 |
| `TaskBoard/ReportStyleMenu.tsx` | 產生前選風格 |
| `src/ipc/reports.ts` | `reports_*` 的 IPC 包裝與型別 |

**不搬 `ArtifactPanelProvider`**——`ArtifactHtmlFrame` 是獨立元件（接 `html` 與 `title` 兩個 prop），直接用即可。

### 6.2 入口

- `ProjectList` 的每張專案卡片：一個「報告」按鈕
- `ProjectBoard` 的工具列：一個「產生工作報告」按鈕

兩處都開同一個 `ReportStyleMenu` → `ReportDialog`。

### 6.3 進度與取消

第一階段是 N 次 AI 呼叫，十張卡可能要一兩分鐘。所以：

- 顯示具體進度（「正在整理第 3/10 張…」），不能只轉圈圈
- 可以中途取消；已經寫回的摘要保留（下次不用重跑）
- 取消後不產生報告檔

## 7. 錯誤處理

| 情況 | 行為 |
|---|---|
| AI 未設定 | **開始前就擋下**並提示去設定，不要跑到一半才失敗 |
| 某張卡的摘要失敗（網路、額度、模型錯誤） | 略過該張、繼續其他的。報告中該張註明「這張無法整理」。不中斷 |
| 對話記錄檔不存在 | 該張只用欄位資料產生摘要，不算失敗 |
| 第二階段失敗 | 明確報錯。已寫回的摘要保留 |
| AI 回覆裡沒有 `artifact-html` 區塊 | 明確報錯**並保留原始回覆**讓使用者檢視，不要默默存一份空檔 |
| `reports_save` 失敗 | 報告仍顯示在視窗中，但明說沒有存檔成功 |
| 專案沒有任何卡片 | 擋下並說明，不呼叫 AI |

## 8. 測試策略

**Rust**

- `reports_save`：建立目錄、檔名時間戳、同分鐘重複產生不覆蓋
- `reports_list`：排序、從 `<title>` 取標題、無 `reports/` 目錄時回空陣列
- `reports_read`：正常讀取；**路徑穿越（`../`、絕對路徑、含分隔字元）必須被拒絕**
- `store::set_summary` 與 `ai_summary` 的遷移：舊資料庫（無此欄位）開起來要正常
- `mark_dispatched` 清掉 `ai_summary`

**前端**

- `reportPrompts`：兩種風格產生不同提示詞；卡片超過 100 張時只取最近的且提示詞中註明
- `useWorkReport`：
  - 已有 `ai_summary` 的卡片**不重複呼叫 AI**（快取命中）
  - 某張摘要失敗時**繼續跑完其他張**並產出報告
  - AI 未設定時在呼叫任何摘要之前就中止
  - 取消時停止後續呼叫，且已寫回的摘要保留
  - 回覆中沒有 artifact 區塊時回報錯誤且不呼叫 `reports_save`
- `ReportDialog`：歷史清單切換、空歷史的呈現

**驗證指令**（依 CLAUDE.md）

```bash
npx tsc -b                     # 不可用 tsc --noEmit（根 tsconfig 是 solution file，永遠 exit 0）
npm run test
cd src-tauri && cargo test     # 完整，不可用 --lib
npm run lint                   # 不得超過現況 125
```

## 9. 跨平台考量

- 路徑一律用 `PathBuf`，不手寫 `/`。
- 報告檔名只含 `YYYY-MM-DD-HHMM` 與可能的 `-N` 後綴，全部是檔案系統安全字元。
- **Windows：測試中若要刪除專案資料夾，必須先關閉 `SqlitePool`**——Windows 不允許刪除還有檔案被開著的目錄。這是上一個里程碑實際踩過的坑。

## 10. 本里程碑不做

- 自訂報告提示詞（D11）
- 跨專案的合併報告
- 報告的排程／自動產生
- 報告匯出成 PDF／Word（`ArtifactHtmlFrame` 已可另存 HTML，其餘交給瀏覽器列印）
- 報告內容的編輯
