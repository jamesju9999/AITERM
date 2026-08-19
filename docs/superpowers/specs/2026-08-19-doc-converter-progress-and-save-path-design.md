# 文件轉換器：轉換中動畫 + 下載存路徑 — 設計

日期：2026-08-19
狀態：已核可，直接實作（範圍小，不走完整 plan/subagent 流程）

## 問題

文件轉換器（`DocConverterView.tsx`）目前有兩個小缺口：

1. 轉換中（`extracting` 為 true）只顯示靜態文字「正在提取...」，沒有任何動態指示，使用者不確定是否卡住。
2. 下載轉換後的 Markdown（`downloadMd`/`downloadRawMd`）用 `Blob` + `<a download>` 技巧，直接存到瀏覽器/WebView 預設下載位置，使用者無法選擇存檔路徑或檔名。

## 範圍

**含：**
- 轉換中顯示不確定進度動畫（轉圈 + 文字），不做假的百分比
- 下載 md 時改用原生「另存新檔」對話框，每次都能指定路徑與檔名

**不含：**
- 真實的轉換進度百分比（anydoc 與 MarkItDown 的 `converter.py` 都沒有進度回報機制，確認過原始碼）
- 預設下載資料夾設定（使用者選擇每次都跳對話框，非一次性設定）

## 決策紀錄

| 決策 | 選定 |
|---|---|
| 進度顯示方式 | 不確定進度動畫（轉圈圖示 + 文字），非百分比 |
| 存檔路徑方式 | 每次下載都跳原生「另存新檔」對話框 |

## 設計

### 1. 轉換中動畫

`.doc-converter__dropzone` 在 `extracting` 為 true 時，現有的靜態 `{t.dc_extracting}` 文字旁加一個轉圈圖示（沿用專案裡 AI 正規化階段已有的 `⟳` 字元，用 CSS `@keyframes` 讓它旋轉；不加真實或假的進度數字）。

### 2. 下載存路徑

沿用 `ChatHistorySidebar.tsx`「匯出對話記錄」功能已經在用的既有模式：

```ts
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "../../ipc/fs";

const path = await save({
  defaultPath: fileName, // 例如 "report.md"
  filters: [{ name: "Markdown", extensions: ["md"] }],
});
if (!path) return; // 使用者取消
await writeTextFile(path, content);
```

取代 `downloadMd`/`downloadRawMd` 裡原本的 `Blob` + `URL.createObjectURL` + `<a download>` 那段。`downloadSuccess` 提示文字沿用既有邏輯，不需要改文案（`t.dc_download_success(fileName)` 本來就沒有特別強調「瀏覽器下載」字樣，直接沿用沒有語意問題）。

## 影響檔案

- `src/components/DocConverter/DocConverterView.tsx`（`downloadMd`/`downloadRawMd` 改用 `save`+`writeTextFile`；`extracting` 分支加轉圈圖示）
- `src/components/DocConverter/DocConverterView.css`（新增 spinner 的 `@keyframes` 動畫）

## 測試

- `DocConverterView.test.tsx` 既有測試需要 mock `@tauri-apps/plugin-dialog` 的 `save` 與 `../../ipc/fs` 的 `writeTextFile`（目前測試檔案裡 `downloadMd`/`downloadRawMd` 沒有被直接測試到，需要新增測試涵蓋：使用者取消存檔對話框時不寫檔、選定路徑後正確呼叫 `writeTextFile`）
