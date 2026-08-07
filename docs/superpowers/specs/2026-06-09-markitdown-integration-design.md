# MarkItDown 整合設計

**日期：** 2026-06-09  
**狀態：** 已核准  
**目標：** 將 Microsoft MarkItDown Python 函式庫整合進 DocConverterView，取代現有前端 JS 解析器，並擴充支援格式。

---

## 背景

現有 `DocConverterView.tsx` 使用三個前端 JS 函式庫解析文件：
- `xlsx`（Excel）
- `mammoth`（Word .docx）
- `pdfjs-dist`（PDF）

此架構限制了支援格式（無 .pptx、.html、圖片等），且維護三套解析器增加複雜度。

[Microsoft MarkItDown](https://github.com/microsoft/markitdown) 是一個 Python 函式庫，可將多種文件格式統一轉換為 Markdown，涵蓋範圍遠超現有解析器。

---

## 決策

- **整合方式：** Python Sidecar（沿用 `api_docs/runner.rs` 模式）
- **取代範圍：** 完全取代現有 JS 解析器（xlsx、mammoth、pdfjs-dist）
- **AI 正規化：** 保留，MarkItDown 負責文字提取，AI 仍負責結構化整理

---

## 架構與資料流

```
使用者拖放 / 選擇檔案
        ↓
DocConverterView.tsx
  → IPC: markitdown_convert(path)
        ↓
src-tauri/src/commands/markitdown.rs
  → 呼叫 tools/MarkItDown/converter.py <file_path>
        ↓
converter.py
  → pip auto-install markitdown[all]（首次使用，後續 no-op）
  → markitdown.convert(file_path).text_content
  → stdout JSON line: { "type": "done", "markdown": "..." }
        ↓
Rust 接收 → 回傳 Ok(markdown_string)
        ↓
前端顯示 rawText（已是 Markdown）
        ↓
（可選）AI 正規化步驟（不變）
```

---

## 支援格式

| 格式 | 副檔名 |
|------|--------|
| Excel | `.xlsx`, `.xls` |
| CSV | `.csv` |
| Word | `.docx` |
| PDF | `.pdf` |
| PowerPoint | `.pptx` |
| HTML | `.html`, `.htm` |
| 圖片 | `.jpg`, `.jpeg`, `.png`, `.gif`, `.webp` |
| EPUB | `.epub` |
| Outlook 郵件 | `.msg` |
| 純文字 / Markup | `.txt`, `.md`, `.rst`, `.xml`, `.json` |

---

## 檔案清單

### 新增

| 檔案 | 用途 |
|------|------|
| `tools/MarkItDown/converter.py` | Python 腳本，接受 file path 參數，輸出 JSON line |
| `tools/MarkItDown/requirements.txt` | `markitdown[all]` |
| `src-tauri/src/commands/markitdown.rs` | Tauri command，呼叫 Python sidecar |
| `src/ipc/markitdown.ts` | 前端 IPC wrapper |

### 修改

| 檔案 | 變更內容 |
|------|---------|
| `src/components/DocConverter/DocConverterView.tsx` | 移除 JS 解析器，改呼叫 `markitdown_convert` IPC；更新 dropzone accept 屬性與格式提示文字 |
| `src-tauri/src/commands/mod.rs` | 新增 markitdown module 與 command 註冊 |
| `src-tauri/src/lib.rs` | 加入 `markitdown_convert` 至 invoke handler |
| `src/lib/i18n.ts` | 更新 `dc_dropzone_formats` 文字 |

### 移除依賴

- `xlsx`（npm）
- `mammoth`（npm）
- `pdfjs-dist`（npm）

---

## Python Sidecar 模式

沿用 `src-tauri/src/api_docs/runner.rs` 現有實作：

1. **auto-install：** Rust 在執行 converter.py 前，先執行 `pip install -r requirements.txt --quiet`
2. **Linux 相容：** 加上 `--user --break-system-packages`（Ubuntu 22.04+）
3. **Windows：** `CREATE_NO_WINDOW` flag 隱藏 console
4. **stdout protocol：** JSON lines，目前只需 `done` 與 `error` 兩種訊息

```python
# converter.py stdout protocol
{ "type": "done", "markdown": "<converted text>" }
{ "type": "error", "message": "<error description>" }
```

---

## 錯誤處理

| 情境 | 處理方式 |
|------|---------|
| Python 未安裝 | Rust 回傳錯誤字串，前端顯示提示 |
| 不支援格式 | MarkItDown 拋出例外 → JSON error → 前端顯示錯誤 |
| 檔案不存在 | Python 拋出例外 → JSON error |
| pip install 失敗 | emit warn log，嘗試繼續執行（markitdown 可能已安裝） |

---

## UI 變更

- Dropzone `accept` 屬性擴充含所有新格式
- `dc_dropzone_formats` i18n 字串更新
- 移除 `detectFormat()` 函式（不再需要前端判斷格式）
- 轉換中改為單一 loading 狀態（等待 IPC 回傳），不再有前端非同步解析步驟

---

## 測試策略

**前端（Vitest）：**
- 更新 DocConverterView 相關測試，mock `markitdown_convert` IPC
- 移除 `detectFormat`、`extractExcel`、`extractWord`、`extractPdf` 單元測試

**Rust：**
- `markitdown.rs` command handler 單元測試，mock Python subprocess 輸出

**手動驗收：**
- 拖放 `.xlsx`、`.docx`、`.pdf`、`.pptx`、`.html` 各一次確認輸出
- Python 未安裝時錯誤提示清楚
- AI 正規化步驟仍正常運作

---

## 跨平台考量

- macOS / Windows / Linux 均需可用（Python sidecar 已有三平台測試範例）
- Python 3 必須存在於系統 PATH（與現有 ApiDocFetcher 相同前提）
- 無需新增 Tauri platform config override
