# 架構導覽分頁（Architecture Explorer）設計

日期：2026-10-22

## 目標

使用者在 AITerm 選一個專案資料夾，就能看到「前端 → 傳輸（IPC／API）→ 後端 → 資料存取」的分層圖。點選任一模組，可以知道資料如何流動、用了什麼技術。支援任意專案類型。

## 範圍決定

- 形態：AITerm 內建功能，以獨立分頁呈現（類似 Agent 協作桌布）。
- 資料來源：靜態掃描為主，AI 補說明與分層。
- 專案類型：不限。靜態掃描只保證檔案與 import 關係；分層與跨層呼叫由 AI 判斷並標註信心度。

## 畫面

- 由左到右的分層欄，每層內是模組方塊，方塊間以箭頭連線。
- 實線＝掃描確認；虛線＝AI 推論（附信心度與佐證檔案行號）。
- 點模組：右側滑出詳情面板，內容含用途、技術與框架、資料進出方式、上下游模組（可跳轉）、對應檔案。
- 點一條呼叫鏈：沿路徑高亮，顯示完整資料流。
- `git HEAD` 與上次分析不同時，提示「架構可能已過期，重新分析？」。

## 資料模型

存於專案內 `.aiterm/architecture.json`：

- `layers[]`：id、名稱、順序。
- `modules[]`：id、layerId、名稱、檔案清單、技術標籤、AI 摘要、資料流說明。
- `edges[]`：from、to、kind（`import`／`ipc`／`http`／`sql`）、source（`scan`／`ai`）、confidence、evidence（檔案與行號）。
- `meta`：掃描時間、git HEAD。

## 後端（Rust）

- 新增 `architecture/` 模組：
  - `scanner`：遍歷檔案、解析 import 與依賴清單，遵守 `.gitignore`。
  - `store`：讀寫 `.aiterm/architecture.json`、取得 git HEAD、限制在專案根目錄內的安全讀檔。
- 新增 command：`arch_scan`／`arch_load`／`arch_save`／`arch_read_files`。
- AI 分析在前端進行（沿用 `ai_complete`，與其他一次性 AI 補全一致），回應以掃描結果驗證，丟棄不存在的檔案、行號、模組與未知 kind。
- 全程使用跨平台路徑處理，需在 macOS、Windows、Linux 都能運作。

## 錯誤處理

- AI 未設定：只顯示掃描結果（實線），並提示設定 AI。
- AI 輸出格式不符、或引用不存在的檔案或行號：丟棄該筆並在畫面顯示丟棄數。
- 專案過大：設檔案數上限；超過時只掃頂層目錄並提示。

## 測試

- Rust：以 tempfile 建小型 fixture 專案，驗證 import 解析與 JSON 結構。
- 前端驗證（`parseAnalysis`）會丟掉幻覺連線；fixture 必須讓正確與錯誤行為的結果可區分。
- 前端 Vitest：點模組開啟詳情面板、虛實線渲染。
- 實機驗收：以 AITerm 自己的 repo 為對象。

## 實作前須確認

- 分頁類型的註冊方式（參考既有分頁型別，避免做成錯誤的分頁類型）。
- 圖形渲染使用的函式庫（需不依賴 OpenSSL 等平台特定項目，並符合現有前端依賴）。
