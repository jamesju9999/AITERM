# 架構導覽第二輪優化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. 設計見 `docs/superpowers/specs/2026-10-22-architecture-explorer-design.md` 的「第二輪優化」。每個純函式先寫會紅的測試（含讓正確與錯誤行為結果可區分的資料），再實作。

**Goal:** 實作 spec 第二輪 A–D。**Tech Stack:** Rust（scanner 常數）、React 19、Vitest。既有檔案在 `src/components/ArchitectureView/`、`src-tauri/src/commands/architecture.rs`。

## Task 1：掃描上限（A）
- `src-tauri/src/commands/architecture.rs`：`MAX_FILES = 20000`。
- 前端截斷提示改用 `arch_truncated_n`（zh-TW/en 都加）：「專案檔案很多，只掃描了前 {n} 個檔案」，n 取 `scan.files.length`。更新相關測試。
- 驗證：`cd src-tauri && CARGO_TARGET_DIR=/private/tmp/claude-501/aiterm-arch-target cargo test --workspace --no-fail-fast 2>&1 | grep "test result"`。

## Task 2：純邏輯（新檔 `drill.ts`、`flow.ts`、`overview.ts`，各有 `.test.ts`；`layout.ts` 增補）
`model.ts` 的 `Architecture` 加 `drills?: Record<string, Architecture>`（可選，舊存檔相容；測試要涵蓋讀到沒有 `drills` 的舊檔）。

- `drill.ts`
  - `buildDrillGraph(scan: ScanResult, files: string[], maxNodes = 60): { arch: Architecture; hidden: number }`。規則見 spec B。節點 id＝群組路徑（檔案節點＝檔案路徑），name＝路徑最後一段，`layer` 為深度欄 id（`d0`、`d1`…，名稱「第 1 層」…）。連線 `kind:"import", source:"scan", confidence:1`，只含兩端都在 `files` 內且分屬不同節點者。環：DFS 遇到回邊就忽略該邊的深度貢獻（連線仍保留）。
  - `drillKey(path: string[]): string`（以 `\u0000` 連接模組 id）。
  - `filesOfModule(arch, moduleId)`。
  - 測試必含：共同前綴計算（`a/b/x.ts`、`a/b/c/y.ts` → 前綴 `a/b`，群 `a/b/c` 與檔案節點 `a/b/x.ts`）；環不會無窮迴圈；超過 maxNodes 時保留度數最高者且 `hidden` 正確；外部檔案的連線被排除。
- `flow.ts`
  - `flowPaths(arch, start, opts?: {maxDepth=6, maxPaths=8}): { down: Edge[][]; up: Edge[][] }`：簡單路徑（不重複節點），由 start 出發往下游、以及到 start 的上游；以最長者優先排序後截斷。
  - `groupEdges(edges, selfId): GroupedEdge[]`：同一對模組、同 kind 的兩個方向合併為 `{ other, kind, mutual: boolean, edges }`。
  - 測試含：A→B→C、A→C 的多路徑、環 A→B→A 不重複走、深度與條數上限、互相引用合併且不同 kind 不合併。
- `overview.ts`
  - `buildOverview(scan, arch)`：`{ languages: {lang,count}[]（降冪）, tech: {name,count}[]（跨模組彙整、降冪）, deps: string[]（前 30）, moduleCount, edgeCount }`。測試含彙整與降冪排序。
- `layout.ts` 增補（純函式，測試先行）：`spreadLabels(points: {id,x,y}[], minDist: number): {id,x,y}[]`（重疊的標籤沿 y 錯開、順序穩定）；`clampZoom(z)`（0.3–2.5）；`zoomAt(view, factor, cx, cy)`（以游標為中心縮放，回傳新的 `{scale, tx, ty}`）；`centerOffset(contentH, viewportH)`（內容矮於視窗時回傳垂直置中的位移，否則 0）。

## Task 3：UI A（鑽取、AI 細分、概覽、詳情合併）
- `index.tsx`：狀態 `drillPath: {id:string; files:string[]}[]`。工具列下方顯示麵包屑（「全部」可點返回根；各層可點跳回）。卡片雙擊或詳情面板「展開」鈕進入；單一檔案節點不顯示展開。目前顯示圖＝根圖，或 `arch.drills?.[drillKey(ids)] ?? buildDrillGraph(...).arch`。選取狀態在切換層級時清空。
- 「AI 細分此模組」鈕（只在鑽取中且該層無 AI 結果時顯示；有結果時顯示「重新細分」）：以 `scan` 過濾成只含該模組檔案與內部連線的子 `ScanResult` 呼叫既有的 `buildAnalyzeMessage`／`parseAnalysis`／`mergeScanEdges`，沿用供應商選擇、已等時間與取消；成功後寫入 `drills[key]` 並 `archSave` 整份。抽出共用的 `runAnalysis(subScan)` 避免複製貼上；既有根層分析行為與測試不變。
- `DetailPanel.tsx`：加「展開」鈕、`groupEdges` 合併顯示（互相引用用「↔」）、外部引用數量（需傳入 `scan` 以計算：被外部引用／引用外部的 import 條數）。
- 未選取模組時顯示概覽（`buildOverview`）：語言分布條、技術標籤、依賴 chips、模組／連線數。
- i18n 全部加到 zh-TW 與 en。測試：雙擊進入並顯示麵包屑、返回、AI 細分成功後存檔且下次進入優先顯示、舊存檔無 `drills` 不報錯、概覽在未選取時出現。

## Task 4：UI B（資料流、縮放平移、標籤與名稱）
- 詳情面板「資料流」區（`flowPaths`）：每條路徑一個可點的卡片，列出步驟（模組名 —kind→ 模組名、備註、`file:line`）；點選後 `activePath` 設定，畫布只高亮該路徑的連線，再點取消。
- 高亮連線加光點（SVG `<animateMotion>`，每條 2 個、相位錯開，時長依連線長度 1–4 秒）；`prefers-reduced-motion: reduce` 不渲染光點。
- 標籤：用 `spreadLabels` 錯開、渲染在最上層並有 `var(--bg-primary)` 實底；只對高亮或 hover 的連線顯示（維持現狀）。
- 名稱：卡片名稱與層標題 `-webkit-line-clamp: 2`，完整名稱 `title`。
- 縮放平移：Ctrl/⌘＋滾輪以游標為中心縮放（`zoomAt`），背景拖曳平移（滑鼠按在卡片上不觸發），「適合視窗」鈕重設為 `fitScale` 並置中；取代目前純 CSS 自動縮放，初次載入仍是適合視窗。內容矮於視窗時用 `centerOffset` 垂直置中。
- 測試：點路徑卡只高亮該路徑、reduced-motion 下沒有光點、適合視窗鈕重設。

## Task 5：驗收與收尾
`npx tsc -b`、`npx eslint`（動過的檔案）、`npm run test`、Rust 全量測試；在 dev App（已從本 worktree 以 HMR 執行）實機走一遍：雙擊鑽取、AI 細分（可用較快的供應商）、資料流路徑、縮放平移、概覽。**不要自己打 tag。**
