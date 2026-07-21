# 知識庫（Knowledge Base）— 設計規格

**日期：** 2026-07-21
**功能：** 文件知識庫（NotebookLM 風格）Tab
**狀態：** 待實作

---

## 1. 功能目標

讓使用者指定一個本機資料夾（內含 PDF、Word、Excel、PowerPoint、圖片、EPUB 等多種格式的文件），透過自然語言與 AI 對話，AI 的回答只能根據該資料夾內的文件內容，並附上出處引用（來源檔案 / 章節）。

支援管理多個獨立的「筆記本」（notebook），每個筆記本綁定一個資料夾，各自維護獨立的搜尋索引與對話歷史。

**與 Code Assistant 的關係：** 架構上是姊妹功能——同樣是「指定目錄 + 工具迭代式問答 + 防幻覺」，差異在於：Code Assistant 讀程式碼用 grep-style 文字搜尋；知識庫讀文件（含非文字格式）用語意向量搜尋（embedding）。實作會大量重用 Code Assistant 已驗證的 agent loop 模式（checkpoint compression、防幻覺 system prompt、streaming、tool call 卡片 UI）。

---

## 2. 使用者流程

1. 使用者從 Tab Bar 開啟「知識庫」Tab
2. 左側顯示筆記本清單（可能為空）；點擊「新增筆記本」→ 選擇資料夾 + 命名 + 選擇 embedding provider
3. 新筆記本建立後顯示「尚未同步」，點擊「同步」開始索引（顯示進度：已處理 N/M）
4. 索引完成後可開始提問；AI 透過工具搜尋文件內容 → 回答並附出處引用
5. 資料夾內容變動後，「同步」按鈕重新可用（只處理新增/變更/刪除的檔案）
6. 側邊欄可切換筆記本、重新命名、刪除

---

## 3. 資料模型

新增一個 app 專用的本地 SQLite 檔案（`{app_data_dir}/knowledge_base.sqlite`，透過既有的 `sqlx` sqlite feature 存取，不需新增套件），與使用者的外部資料庫連線功能完全分離。

```sql
notebooks (
  id            TEXT PRIMARY KEY,   -- uuid
  name          TEXT NOT NULL,
  folder_path   TEXT NOT NULL,
  embed_provider_id TEXT,           -- 對應 config 裡的 provider id
  embed_model   TEXT,
  embed_dim     INTEGER,            -- 該筆記本目前索引使用的向量維度
  last_synced_at INTEGER            -- unix timestamp, NULL = 從未同步
)

documents (
  id            TEXT PRIMARY KEY,
  notebook_id   TEXT NOT NULL,
  rel_path      TEXT NOT NULL,      -- 相對 folder_path
  mtime         INTEGER NOT NULL,
  content_hash  TEXT NOT NULL,      -- 判斷是否需要重新轉換/切片
  markdown_cache TEXT,              -- MarkItDown 轉換結果快取
  status        TEXT NOT NULL,      -- 'ok' | 'error'
  error_message TEXT
)

chunks (
  id            TEXT PRIMARY KEY,
  document_id   TEXT NOT NULL,
  chunk_index   INTEGER NOT NULL,
  text          TEXT NOT NULL,
  location_hint TEXT,               -- 最近的標題/章節，或頁碼（若來源格式有提供）
  embedding     BLOB NOT NULL       -- little-endian f32 陣列
)
```

**向量搜尋策略：** 資料量估計落在數千到數萬個 chunk（200+ 文件 × 每份約 20-50 chunk）。直接在 Rust 端做 brute-force cosine similarity（載入該筆記本所有向量、線性掃描），數萬筆向量仍是毫秒等級，不需引入 `sqlite-vec` 之類的向量索引擴充套件，維持依賴精簡。若之後實際使用量遠超預期造成效能問題，這層可以之後再抽換，不影響上層介面。

**維度一致性：** 同一筆記本內的所有 chunk 必須用同一個 embedding model 產生。若使用者更換筆記本的 embedding provider/model，須提示「將清空現有索引並要求重新完整同步」（`embed_dim` 不符即視為需要重建）。

---

## 4. 同步流程（手動觸發，增量處理）

1. 掃描 `folder_path`（遞迴），比對每個檔案的 mtime/hash 與 `documents` 表：
   - 新檔案 → 待轉換
   - mtime/hash 變更 → 待重新轉換（刪除舊 chunks）
   - 已消失的檔案 → 刪除對應 document + chunks
   - 不支援格式 → 略過，不計入待處理清單
2. 逐一（限制併發數，例如 3 個並行）：
   - 呼叫既有的 `markitdown_convert`（重用，不用重寫）轉成 markdown
   - 依 markdown 標題切成帶重疊的 chunk（約 800 token / 150 token overlap，優先沿標題邊界切）
   - 呼叫 embedding provider 取得每個 chunk 的向量
   - 寫入 `documents` + `chunks`
   - 透過事件回報進度
3. 轉換/embedding 失敗的檔案記錄 `status = 'error'` 與錯誤訊息，不中斷整體同步；同步結束後於 UI 顯示「N 個檔案失敗」摘要（可展開看細節）
4. 全部完成後更新 `notebooks.last_synced_at`

**支援格式：** 沿用現有 `markitdown_pick_file` 的清單 — `xlsx xls csv docx pdf pptx html htm jpg jpeg png gif webp epub msg txt md rst xml json`。

---

## 5. Embedding Provider 抽象層

新增輕量抽象（獨立於既有聊天用的 `ai/router.rs`，但共用 provider 設定與 API key 存取）：

| Provider 類型 | 呼叫端點 |
|---|---|
| Ollama | `POST /api/embed` |
| OpenAI / OpenAI 相容 | `POST /v1/embeddings` |
| Anthropic | 不支援（Anthropic 無 embedding API）— UI 上不可選 |

每個筆記本各自記錄使用哪個 provider + model（建立筆記本時選擇，之後可更換但會觸發重新索引提示）。

---

## 6. 問答架構：Tool-use 迭代式（重用 Code Assistant agent loop）

不做單次 RAG（embed 問題 → 撈 top-K → 一次性回答），改為讓 AI 擁有工具、自行決定要搜尋幾次、搜什麼：

| Tool | 簽名 | 用途 |
|------|------|------|
| `search_documents` | `(query: string, top_k?: number)` | 對 query 做 embedding，回傳最相關的 chunk 列表（含來源檔名、location_hint、相似度）。`top_k` 預設 8，上限 20 |
| `read_document` | `(path: string)` | 讀取某份文件完整轉換後的 markdown 內容 |

這讓 AI 可以處理需要多跳推理的問題（例如「比較 A 文件和 B 文件的差異」需要各自搜尋），而不是被單次檢索結果侷限。

**重用 Code Assistant 既有機制：**
- Checkpoint Compression（長對話在 30K token 時摘要壓縮）
- 防幻覈 system prompt（「只能引用透過 search_documents/read_document 實際取得的內容」）
- 已讀取／僅搜尋 chip 標示（比照 Code Assistant 的 `read_file` vs `search_in_files` 綠╱灰 chip）
- Streaming + tool call 卡片 UI（`ToolCallCard`）

**不支援 tool-use 的 provider：** 同 Code Assistant，fallback 為簡化版：直接對問題做一次 `search_documents`，把結果組進 context 一次性回答（無多跳能力），並顯示提示 banner。

---

## 7. 出處引用（Citations）

每個回答下方顯示來源 chip：檔名 + `location_hint`（若有）。點擊 chip 開啟該來源檔案（OS 預設程式開啟，透過既有的檔案開啟 IPC）。

**重要限制（需在 UI 上誠實標示）：** MarkItDown 轉換不保證每種格式都保留頁碼／章節資訊（例如 PDF 是否有頁碼標記取決於來源檔案結構）。`location_hint` 為 best-effort：有標題結構的文件會標出最近的標題；沒有結構的純文字/圖片說明只會標出檔名。不承諾所有引用都精確到頁碼。

---

## 8. 事件協議（Streaming）

比照 Code Assistant 的 `code-assistant-event`，新增：

```typescript
// 同步進度
type KbSyncEvent =
  | { kind: "progress"; processed: number; total: number; current_file: string }
  | { kind: "file_error"; path: string; message: string }
  | { kind: "done"; indexed: number; failed: number; deleted: number }

// 問答（結構同 CodeAssistantEvent，工具換成 search_documents / read_document）
type KbChatEvent =
  | { kind: "tool_call"; tool: string; args: Record<string, string>; call_id: string }
  | { kind: "tool_result"; call_id: string; content: string; truncated: boolean }
  | { kind: "text_delta"; delta: string }
  | { kind: "token_count"; count: number; limit: number }
  | { kind: "done" }
  | { kind: "error"; error: AiError }
```

---

## 9. 架構（檔案配置）

### 後端（Rust）
```
src-tauri/src/
  knowledge_base/
    mod.rs        ← chat agent loop（重用 code_assistant/mod.rs 架構）
    tools.rs      ← search_documents / read_document 實作
    embedding.rs  ← Ollama / OpenAI 相容 embedding provider 抽象
    ingest.rs     ← 掃描/轉換/切片/embedding pipeline
    db.rs         ← SQLite schema、CRUD、cosine similarity 搜尋
    chunk.rs      ← markdown 切片邏輯

  commands/
    knowledge_base.rs  ← Tauri commands（見下）
```

**主要 Tauri commands：**
```rust
kb_create_notebook(name, folder_path, embed_provider_id, embed_model) -> NotebookInfo
kb_list_notebooks() -> Vec<NotebookInfo>
kb_delete_notebook(notebook_id)
kb_sync_notebook(notebook_id, app: AppHandle)  // emits KbSyncEvent
kb_chat(notebook_id, messages, provider_id, locale, app: AppHandle)  // emits KbChatEvent
```

### 前端（React / TypeScript）
```
src/
  components/KnowledgeBaseView/
    index.tsx          ← 主容器（筆記本清單 + 對話介面）
    NotebookSidebar.tsx
    NotebookCreateDialog.tsx
    SyncProgress.tsx
    styles.css

  ipc/
    knowledgeBase.ts
  hooks/
    useKnowledgeBase.ts   ← 比照 useCodeAssistant.ts
```

**重用既有元件：** `ToolCallCard`（來自 CodeAssistantView，可考慮抽到共用位置）、`MarkdownText`、`ModelPickerButton`。

---

## 10. 安全限制

| 限制 | 值 |
|------|----|
| 單次同步併發轉換數 | 3 |
| `search_documents` 單次回傳上限 | 20 個 chunk |
| `read_document` 單檔上限 | 沿用 MarkItDown 轉換結果，無額外截斷（信任 markitdown 輸出大小合理；若過大比照 Code Assistant 的 100KB 截斷邏輯） |
| Chat agent loop 最大 tool call 輪數 | 20 輪 / 使用者訊息（同 Code Assistant） |
| 累積 token 估算上限 / Checkpoint 閾值 | 沿用 Code Assistant 現有常數 |

---

## 11. 不在此次範圍內

- 自動監控資料夾變化並即時重新索引（目前為手動同步）
- 向量索引升級為 ANN（sqlite-vec 等），除非未來證實效能不足
- 筆記本間交叉搜尋（一次只問一個筆記本）
- 文件編輯能力（唯讀）
- PDF 精確頁碼引用的保證（best-effort，取決於 MarkItDown 輸出）
- 匯出對話紀錄（可比照 Code Assistant 未來加入）

---

## 12. 待確認的實作細節（留給 writing-plans 階段）

- Chunk 切分的確切演算法（標題邊界 + 固定大小 fallback 的細節）
- embedding 呼叫失敗的重試策略（單一 chunk 失敗是否讓整份文件失敗）
- 新增筆記本時若資料夾內已有大量檔案，是否需要限制單次同步檔案數上限
