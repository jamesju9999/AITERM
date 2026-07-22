# 終端機區塊（Block）渲染架構重構 — 設計規格

**日期：** 2026-07-22
**狀態：** 已核准，待實作

---

## 目標

參考 Warp 終端機的區塊樣式（每個指令是一張完整卡片，含路徑／git branch／耗時／diff 統計的常駐標頭列），對 AITerm 目前的終端機區塊功能做架構性重構，同時解決兩個問題：

1. **視覺資訊不足**：目前區塊只有一條左側色條 + hover 才顯示的操作按鈕，沒有任何常駐 metadata。
2. **定位不穩定**：目前用「往上下各搜尋 100 行文字比對指令內容」的方式去猜區塊在 xterm 畫布上的像素位置（`TerminalView.tsx` 約 920–1027 行），長輸出、捲動、指令文字重複時容易跑位。

這兩者不是各自獨立的問題——標頭列若疊在不穩定的座標系統上會讓錯位更明顯，因此本次採取根本性做法：改變區塊的渲染架構本身。

---

## 範疇

**包含：**
- 區塊渲染架構從「疊加在 xterm 畫布上的 pixel-position overlay」改為「headless 解析 + 獨立 DOM 區塊清單」
- `TerminalBlock` 資料結構擴充（時間戳、cwd、原始輸出、渲染後的結構化內容）
- 新的 `TerminalBlockCard` 元件，含 Warp 風格標頭列（路徑／git branch／diff 統計／耗時／exit code）
- 新增 Rust 後端指令 `get_git_block_info`（輕量 git branch + diff 統計查詢）
- 搜尋功能（`doSearch`/`closeSearch`）改造，橫跨即時 xterm 內容與已完成區塊
- 長輸出的摺疊／展開、截斷渲染

**不包含：**
- Alternate buffer（全螢幕互動程式如 vim/htop/ssh curses UI）的渲染方式——維持現況，完全不受影響
- 完整的虛擬捲動（list virtualization）——先用截斷＋摺疊處理效能，未來如有需要再評估加入
- 針對單一指令造成的 diff 差異做歸因——git 統計是「區塊完成當下工作目錄的整體未提交變更狀態」，不是逐指令追蹤

---

## 整體架構

### 現況

- 所有 PTY 輸出（包含歷史內容）都寫進可見的 xterm.js 緩衝區
- React 用「行號 × 字元高度」的像素運算，疊加一層 DOM 色條在 xterm 畫布上方，位置靠文字比對猜測

### 重構後

- **可見的即時 xterm**：職責縮小為「目前的 prompt／正在執行中指令的即時輸出」，串流、色彩、游標行為不變
- **區塊完成時（OSC 133 `D` 觸發）**：
  1. 把該區塊累積的原始 PTY bytes 餵給一個不掛載到畫面上的 **headless xterm.js 實例**（欄數與可見終端一致）解析一次，取出結構化的逐行文字＋樣式資料
  2. 解析完成後才把區塊標記為可顯示（避免空卡片閃爍）
  3. 呼叫 `term.clear()` 清空可見終端的畫布與捲動歷史，讓即時 viewport 回到乾淨狀態
- **區塊清單**：即時 xterm 上方一個獨立可捲動的 React 清單，由 `TerminalBlockCard` 依序渲染每個已完成區塊，是往後歷史記錄的唯一真相來源

此架構下不再需要任何像素位置猜測，`TerminalView.tsx` 中原本的 `mappedBlocks`/`parsedPromptY` 猜測邏輯（約 920–1027 行）整段移除。

---

## 資料層

### `TerminalBlock` 型別變更（`src/hooks/useTerminalBlocks.ts`）

```ts
export interface TerminalBlock {
  id: string;
  command: string;
  status: "running" | "completed" | "failed";
  exitCode?: number;
  startTime: number;              // 新增：OSC 133 C 觸發時的 Date.now()
  endTime?: number;                // 新增：OSC 133 D 觸發時的 Date.now()
  cwd?: string;                    // 新增：區塊開始當下的路徑快照
  rawOutput: string;                // 取代原本從未寫入的 output 欄位；累積的原始文字
  renderedLines?: RenderedLine[];   // 新增：headless 解析後的結構化內容
  gitInfo?: GitBlockInfo | null;    // 新增：非同步取得的 git branch/diff 資訊
}

export interface RenderedLine {
  spans: RenderedSpan[];
}
export interface RenderedSpan {
  text: string;
  fg?: string;
  bg?: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
}

export interface GitBlockInfo {
  branch: string;
  insertions: number;
  deletions: number;
}
```

移除欄位：`startMarker`、`endMarker`、`startLine`、`endLine`、`decorationCreated`（舊座標猜測機制專用，不再需要）。

### 擷取時機

所有 PTY 輸出目前只有單一匯集點：`TerminalView.tsx:543` 的 `onPtyData` callback（呼叫 `term.write(text)`）。在同一處新增一步：只要最新區塊狀態為 `running`，同一份 `text` 也累加進該區塊的 `rawOutput`，不另外監聽、不會有兩份資料不同步的風險。

### 區塊完成流程（OSC 133 `D`）

1. 凍結 `rawOutput`
2. 丟進 headless xterm 解析，產生 `renderedLines`
3. 標記區塊為可顯示
4. `term.clear()` 清空可見終端

---

## Git 標頭 metadata

### 新增邏輯與 Tauri command

- `GitBlockInfo` struct 新增於 `src-tauri/src/vcs/types.rs`（現有 `BlameEntry`/`BranchEntry` 等型別所在處），需 `#[derive(Serialize, Deserialize)]` 以便跨 IPC 傳遞
- `GitClient`（`src-tauri/src/vcs/git.rs`）新增方法 `quick_block_info(&self) -> Option<GitBlockInfo>`，比照現有 shell-out 風格（不引入 git2 crate）：跑 `git rev-parse --abbrev-ref HEAD` 取得 branch、`git diff --shortstat` 取得增刪行數，非 git 目錄或指令失敗回傳 `None`
- `#[tauri::command]` 入口比照現有命名慣例放在 `src-tauri/src/commands/vcs.rs`：

```rust
#[tauri::command]
pub async fn vcs_get_block_info(cwd: String) -> Option<GitBlockInfo> {
    GitClient::new(cwd, None).quick_block_info().await
}
```

### 前端呼叫時機

- 區塊完成時，卡片先用已有資料（路徑／指令／輸出／耗時／exit code）立刻渲染
- 非同步呼叫 `vcs_get_block_info(cwd)`，回傳後才補上 git 資訊那一小段文字，不阻塞區塊完成的當下
- **防抖動**：同一 session 500ms 內只呼叫一次，期間完成的區塊共用同一份結果（避免 Agent Loop 連續執行多個指令時 git subprocess 被連續觸發）

---

## `TerminalBlockCard` 元件

**標頭列**：路徑、git branch、diff 統計（`+N -M`，無 git 資訊時不顯示這段）、耗時（`endTime - startTime`）、exit code（非 0 時標紅）

**內容區**：依 `renderedLines` 渲染成 `<pre>`/`<span>`，可點擊標頭摺疊/展開；超過 500 行預設只渲染部分內容 + 「顯示完整輸出（還有 N 行）」展開按鈕，`rawOutput`/`renderedLines` 完整內容仍保留供搜尋與複製使用

**hover 操作按鈕**：沿用現有的 Ask AI／Bookmark／Copy（失敗時額外顯示 Ask AI）

---

## 搜尋功能改造

現有搜尋：`SearchAddon` 對 xterm 緩衝區做 `findNext`/`findPrevious`，結果只顯示 found/not found。

改造後把「所有已完成區塊（依時間排序）＋ 目前即時內容」視為一條連續的搜尋範圍：

- **即時 xterm 部分**：不動，繼續用 `SearchAddon`
- **已完成區塊部分**：對每個區塊的純文字內容做線性字串比對，找到就 `scrollIntoView` 捲動過去，並在 `TerminalBlockCard` 內把命中文字用 `<mark>` 包起來高亮
- **Next/Prev 串接**：先在目前所在區段找，找不到才移動到下一段；到達最新內容（即時 xterm）後找不到則循環回最舊的區塊，維持現有「按下一個會循環」的手感
- 搜尋邏輯抽成獨立純函式，不依賴 DOM，方便單元測試

---

## 效能與邊界情況

**效能**：不引入虛擬捲動函式庫（目前專案未安裝任何一種），改用截斷 + 摺疊處理：
- 單一區塊輸出超過 500 行時預設截斷渲染，提供展開按鈕
- 區塊標頭可點擊摺疊/展開內容，摺疊後固定高度
- 若未來實際遇到「單一 session 累積上千個區塊導致清單卡頓」，再評估加入虛擬捲動

**邊界情況**：
1. **指令中途進入 alternate buffer（vim/htop/less 等）**：捕捉到的 `rawOutput` 含完整畫面重繪序列，headless 解析後會呈現「回到 normal buffer 那一刻的最終畫面」，非互動過程錄影——可接受的預設行為
2. **沒有 shell 整合（OSC 133 不會觸發）**：整套區塊機制不啟動，終端機退化為現況的普通模式，不影響現有使用者
3. **區塊渲染完成後使用者調整視窗大小**：歷史區塊內容是在捕捉當下欄寬解析出的靜態文字，resize 後不會重新換行——預期行為，不處理

---

## 測試

**前端（Vitest + RTL）：**
- `useTerminalBlocks.test.ts`：模擬 OSC 133 `C`/`D` 序列，驗證區塊建立、輸出片段正確累積進 `rawOutput`、依 exit code 標記 `completed`/`failed` 並產生 `renderedLines`；連續快速多個區塊時 `rawOutput` 不互相污染
- `TerminalBlockCard.test.tsx`：`renderedLines` 正確渲染成對應樣式的 span、摺疊/展開切換正常、搜尋高亮 prop 正確包出 `<mark>`
- 搜尋邏輯純函式：跨區塊 + 即時內容的 next/prev 循環搜尋正確性

**後端（`cargo test` + `tempfile`）：**
- `get_git_block_info`：用 `tempfile` 建暫存 git 目錄，驗證 branch 名稱與增刪行數正確
- 非 git 目錄：驗證回傳 `None`，不 panic

---

## 檔案清單

| 檔案 | 變更類型 |
|---|---|
| `src/hooks/useTerminalBlocks.ts` | 修改：`TerminalBlock` 型別擴充、`rawOutput` 累積邏輯、headless 解析、`term.clear()` 呼叫 |
| `src/components/TerminalView.tsx` | 修改：移除 `mappedBlocks`/`parsedPromptY` 座標猜測邏輯與 overlay 渲染；新增區塊清單容器；`onPtyData` 內加入 `rawOutput` 累積；`doSearch`/`closeSearch` 改造 |
| `src/components/TerminalBlockCard.tsx` | 新增：區塊卡片元件（標頭列、內容渲染、摺疊/展開、hover 操作按鈕） |
| `src/hooks/useHeadlessAnsiParser.ts`（或等效檔案） | 新增：headless xterm 解析封裝，將原始 bytes 轉為 `RenderedLine[]` |
| `src-tauri/src/vcs/git.rs` | 修改：`GitClient` 新增 `quick_block_info` 方法 |
| `src-tauri/src/vcs/types.rs` | 修改：新增 `GitBlockInfo` struct |
| `src-tauri/src/commands/vcs.rs` | 修改：新增 `vcs_get_block_info` command 並註冊 |
| `src/ipc/vcs.ts` | 修改：新增 `vcs_get_block_info` 的 TS wrapper |

---

## 成功標準

1. 一般指令執行完成後，區塊清單出現對應卡片，標頭列顯示路徑／git branch（若為 git 目錄）／diff 統計／耗時／exit code
2. 長輸出、捲動、重複指令文字等情境下，區塊內容與標頭資訊皆正確對應，不再出現跑位
3. Alternate buffer 程式（vim/htop 等）行為與現況一致，不受影響
4. 搜尋功能可同時在已完成區塊與即時內容中找到符合字串，並正確循環
5. 摺疊/展開與長輸出截斷功能正常運作
6. 前端與後端新增測試皆通過；`npx tsc --noEmit` 與 `cargo test` 無錯誤
