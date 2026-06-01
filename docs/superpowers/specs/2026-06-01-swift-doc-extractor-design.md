# Design Spec: SwiftDocFetcher Tab

**日期**: 2026-06-01
**狀態**: 已核准

---

## 概述

在 AITerm 新增 `swift-doc` Tab，讓使用者從 SWIFT 開發者文件網站（`docs.developer.swift.com`）萃取 API 參考文件，輸出為 AI coding tool 可直接使用的 Markdown 檔案。支援原始萃取與 AI 增強兩種模式。

---

## 技術背景

- SWIFT 文件站是 **Next.js App Router（CSR）**，WAF 封鎖一般 HTTP client
- **解決方案**：Python `curl_cffi`（Chrome TLS 指紋模擬），已驗證可取得 HTTP 200
- 頁面 HTML 的 `__next_f` streaming chunks 內嵌**完整 OpenAPI 3.0 YAML**（每頁 70–111KB）
- **不需要 Playwright / headless browser**
- UrlFetcher 已加入 curl_cffi fallback，macOS / Windows 均可運作

---

## 使用者流程

1. 開新 Tab → 選擇「SWIFT Doc Fetcher」
2. 輸入文件根網址，點「載入文件樹」
3. 左側樹狀結構顯示所有文件頁面，支援：
   - 全選 / 全不選
   - 關鍵字即時篩選
   - 展開 / 收合群組
4. 右側設定面板選擇：
   - 輸出目錄
   - 合併單檔 / 依頁分開
   - 保留內容（Endpoint 描述 / Parameters / Request schema / Response schema / Code samples）
   - AI Provider（選配）
5. 點擊按鈕：
   - **「萃取原始 Markdown」**：快速模式，直接輸出結構化 MD
   - **「萃取 + AI 增強」**：每頁完成後呼叫 AI 改寫成人類可讀格式
6. 進度條 + log 列表即時顯示每頁狀態
7. 完成後顯示輸出檔案清單

---

## 架構

### 元件分層

```
React Frontend (SwiftDocView.tsx)
    ↕ Tauri IPC invoke / events
Rust Backend (commands.rs)
    ↕ spawn subprocess + stdout JSON
Python Script (tools/SwiftDocFetcher/fetcher.py)
    ↕ HTTP
docs.developer.swift.com
```

### 新增 Tauri Commands

| Command | 輸入 | 輸出 |
|---------|------|------|
| `swift_fetch_tree(url)` | 根網址 | `Vec<DocNode>`（樹狀 JSON） |
| `swift_extract_docs(pages, options)` | 頁面清單 + 設定 | 串流 events |

### 新增 Tauri Events

| Event | Payload |
|-------|---------|
| `swift-doc-progress` | `{ current, total, page }` |
| `swift-doc-log` | `{ level, message }` |
| `swift-doc-done` | `{ files: string[] }` |

### Python stdout 協定（line-delimited JSON）

```json
{"type":"tree","data":[{"title":"Getting Started","href":"/docs/getting-started","items":[...]}]}
{"type":"progress","current":1,"total":5,"page":"payment-prevalidation-bav-api-reference"}
{"type":"log","level":"info","message":"✓ BAV API Reference (111KB)"}
{"type":"log","level":"error","message":"✗ Failed: gpi-apis (timeout)"}
{"type":"done","files":["~/swift-docs/swift-api.md"]}
```

---

## 檔案結構

```
src/
  components/
    SwiftDocView/
      SwiftDocView.tsx       ← Tab 主元件
      SwiftDocView.css
      DocTree.tsx            ← 樹狀選擇器（含篩選）
      ExtractionSettings.tsx ← 右側設定面板
      ExtractionLog.tsx      ← 進度條 + log 列表

src-tauri/src/
  swift_doc/
    mod.rs                   ← Tauri commands
    types.rs                 ← DocNode, ExtractionOptions 型別

tools/
  SwiftDocFetcher/
    fetcher.py               ← 主腳本（fetch + parse + convert）
    requirements.txt         ← curl_cffi, pyyaml
    package.json             ← 已存在（Playwright 驗證用，可保留）
```

---

## Python fetcher.py 職責

### fetch_tree(url)
1. `curl_cffi GET` 根頁面
2. regex 提取 `__next_f` chunks
3. 找含 `"title"` + `"href"` + `"items"` 的 chunk
4. 解析 JSON，輸出 `{"type":"tree","data":[...]}` 到 stdout

### extract_pages(pages, options)
對每個選定頁面：
1. `curl_cffi GET` 頁面
2. regex 提取所有含 `openapi:` 的 chunks
3. 解析 YAML → 依 `options.keep` 篩選欄位
4. 轉換為 Markdown（見下節格式）
5. stdout `{"type":"progress",...}` 和 `{"type":"log",...}`
6. 全部完成後寫入 `.md` 檔，stdout `{"type":"done",...}`

---

## Markdown 輸出格式

### 原始模式

```markdown
# {info.title} — {info.version}

> {info.description}

**Base URL**: {servers[0].url}

---

## POST /accounts/verification

**Summary**: Verify beneficiary bank account

### Parameters
| Name | In | Type | Required | Description |
|------|----|------|----------|-------------|
| ... | ... | ... | ... | ... |

### Request Body
\`\`\`json
{ ... }
\`\`\`

### Responses
| Code | Description |
|------|-------------|
| 200 | Success |
| 400 | Bad Request |
```

### AI 增強模式（在原始格式之上）

每個 endpoint 加入：

```markdown
### 白話說明
這個 API 讓你驗證受款方的銀行帳號是否有效...

### 何時使用
當你要發起跨境付款前，建議先呼叫此 API 確認帳號...

### 常見使用情境
1. 零售跨境付款前的帳號驗證
2. 批次付款前的預檢查
```

AI prompt 設計：
- system: 你是 SWIFT API 文件助手，將 OpenAPI spec 轉為銀行開發者容易理解的說明
- user: 以下是 {endpoint} 的 OpenAPI 規格，請以繁體中文或英文（依原始語言）改寫成開發者文件

---

## 整合現有 AITerm 架構

- **AI Provider**：沿用 `src/ipc/provider.ts` 的 `listProviders()`，讓使用者在設定面板選擇已設定的 Provider
- **AI 呼叫**：沿用 `src/ipc/ai.ts` 的 `aiChat()`，streaming 模式
- **Tab 類型**：在 `TerminalApp.tsx` 加入 `"swift-doc"` 類型，沿用現有 Tab 生命週期
- **i18n**：在 `src/lib/i18n.ts` 補充對應字串

---

## 錯誤處理

| 錯誤場景 | 處理方式 |
|---------|---------|
| curl_cffi 未安裝 | 萃取前檢查，顯示安裝指引 `pip install curl_cffi` |
| 頁面 timeout | log 顯示 `✗ Failed`，繼續其餘頁面，最後回報失敗清單 |
| OpenAPI 解析失敗 | log 警告，跳過該頁，不中斷整體流程 |
| AI Provider 未設定 | 「萃取 + AI 增強」按鈕 disable，tooltip 提示需先設定 Provider |
| 輸出目錄不存在 | 自動建立目錄（`mkdir -p`） |

---

## 不在範圍內

- 登入牆後的文件（SWIFT 部分付費內容）
- 非 SWIFT 文件網站（本功能只針對 `docs.developer.swift.com`）
- Markdown 以外的輸出格式（PDF、HTML）
- 自動排程 / 定期更新
