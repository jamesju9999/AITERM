# Design Spec: API Docs Tab

**日期**: 2026-06-01
**狀態**: 已核准

---

## 概述

在 AITerm 新增 **API Docs** Tab，讓使用者從任意 API 文件網站萃取文件，輸出為 AI coding tool 可直接使用的 Markdown 檔案。支援原始萃取與 AI 增強兩種模式。

已驗證可運作的目標網站類型：
- 有公開 OpenAPI / Swagger 規格的網站（Stripe、OpenAI、GitHub、Twilio 等）
- Mintlify / Next.js App Router 架設的文件站（SWIFT、Anthropic、Vercel 等）
- Swagger UI / Redoc 架設的網站
- Docusaurus 架設的網站
- 其他：AI fallback 從 HTML 萃取（準確率較低，但覆蓋範圍最廣）

---

## 技術背景

### SWIFT 文件站（已驗證）
- Next.js App Router（CSR），WAF 封鎖一般 HTTP client
- **解決方案**：Python `curl_cffi`（Chrome TLS 指紋模擬），已驗證可取得 HTTP 200
- 頁面 HTML 的 `__next_f` streaming chunks 內嵌完整 OpenAPI 3.0 YAML（每頁 70–111KB）
- 不需要 Playwright / headless browser

### UrlFetcher 現狀
- 已加入 curl_cffi fallback，macOS / Windows 均可運作
- 一般網站走 .NET HttpClient（快），SWIFT 等 WAF 封鎖站走 curl_cffi（自動 fallback）

---

## 使用者流程

1. 開新 Tab → 選擇「API Docs」
2. 輸入文件根網址，點「載入文件樹」
   - 系統自動偵測網站平台類型並顯示（如：`Mintlify`、`Swagger UI`、`OpenAPI Direct`）
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
React Frontend (ApiDocsView.tsx)
    ↕ Tauri IPC invoke / events
Rust Backend (commands.rs)
    ↕ spawn subprocess + stdout JSON
Python Script (tools/ApiDocFetcher/fetcher.py)
    ├─ detector.py      ← 偵測平台類型
    ├─ strategies/
    │   ├─ openapi_direct.py   ← 直接下載 OpenAPI spec
    │   ├─ mintlify_next.py    ← __next_f chunks 解析
    │   ├─ swagger_ui.py       ← 偵測 spec URL → 下載
    │   ├─ redoc.py            ← 偵測 spec URL → 下載
    │   ├─ docusaurus.py       ← sidebar.json + MD 頁面
    │   └─ ai_generic.py       ← AI 從 HTML 萃取（fallback）
    └─ converter.py     ← OpenAPI / HTML → Markdown
```

### 偵測順序（detector.py）

```
1. 嘗試常見 OpenAPI 路徑：/openapi.json, /swagger.json, /api-docs, /openapi.yaml
   → 成功：openapi-direct 策略
2. curl_cffi 抓首頁 HTML，檢查：
   a. __next_f chunks 含 openapi: → mintlify-next 策略
   b. <div id="swagger-ui"> 或 SwaggerUIBundle → swagger-ui 策略
   c. <redoc> 或 ReDoc.init → redoc 策略
   d. docusaurus-classic 或 /docs/sidebar.json → docusaurus 策略
3. 都沒有 → ai-generic 策略（AI 從 HTML 萃取）
```

### 新增 Tauri Commands

| Command | 輸入 | 輸出 |
|---------|------|------|
| `api_docs_detect(url)` | 網站 URL | `{ platform, confidence }` |
| `api_docs_fetch_tree(url)` | 網站 URL | `Vec<DocNode>`（樹狀 JSON） |
| `api_docs_extract(pages, options)` | 頁面清單 + 設定 | 串流 events |

### 新增 Tauri Events

| Event | Payload |
|-------|---------|
| `api-docs-progress` | `{ current, total, page }` |
| `api-docs-log` | `{ level, message }` |
| `api-docs-done` | `{ files: string[] }` |

### Python stdout 協定（line-delimited JSON）

```json
{"type":"detected","platform":"mintlify-next","confidence":"high"}
{"type":"tree","data":[{"title":"Getting Started","href":"/docs/getting-started","items":[...]}]}
{"type":"progress","current":1,"total":5,"page":"payment-prevalidation-bav-api-reference"}
{"type":"log","level":"info","message":"✓ BAV API Reference (111KB)"}
{"type":"log","level":"warn","message":"⚠ ai-generic fallback used for: some-page"}
{"type":"log","level":"error","message":"✗ Failed: gpi-apis (timeout)"}
{"type":"done","files":["~/api-docs/swift-api.md"]}
```

---

## 檔案結構

```
src/
  components/
    ApiDocsView/
      ApiDocsView.tsx        ← Tab 主元件
      ApiDocsView.css
      DocTree.tsx            ← 樹狀選擇器（含篩選）
      ExtractionSettings.tsx ← 右側設定面板
      ExtractionLog.tsx      ← 進度條 + log 列表

src-tauri/src/
  api_docs/
    mod.rs                   ← Tauri commands
    types.rs                 ← DocNode, ExtractionOptions 型別

tools/
  ApiDocFetcher/
    fetcher.py               ← 主程式（CLI 入口）
    detector.py              ← 平台偵測
    converter.py             ← OpenAPI YAML/JSON → Markdown
    requirements.txt         ← curl_cffi, pyyaml, beautifulsoup4
    strategies/
      __init__.py
      openapi_direct.py
      mintlify_next.py
      swagger_ui.py
      redoc.py
      docusaurus.py
      ai_generic.py
```

---

## 各策略說明

### openapi-direct
1. 嘗試常見路徑下載 OpenAPI spec（JSON 或 YAML）
2. 解析所有 paths、components
3. 輸出樹狀結構（依 tags 分組）

### mintlify-next（SWIFT 已驗證）
1. curl_cffi GET 頁面，regex 提取 `__next_f` chunks
2. 找含 `"title"` + `"href"` + `"items"` 的 chunk → 樹狀結構
3. 找含 `openapi:` 的 chunk → OpenAPI YAML

### swagger-ui / redoc
1. curl_cffi GET 首頁，解析 spec URL（`url:` 或 `spec:` 屬性）
2. 直接下載 spec，同 openapi-direct 流程

### docusaurus
1. 嘗試 `/docs/sidebar.json` 或 `/_docusaurus/sidebar.json` 取得樹狀結構
2. 逐頁抓取 Markdown（Docusaurus 通常有靜態 MD 路徑）

### ai-generic（fallback）
1. curl_cffi GET 頁面，提取純文字
2. 呼叫 AI Provider 分析：「這個頁面的 API endpoints、parameters、responses 是什麼？」
3. AI 回傳結構化 JSON → 轉換為 Markdown
4. log 中標記為 `ai-generic` 讓使用者知道精確度較低

---

## Markdown 輸出格式

### 原始模式

```markdown
# {info.title} — {info.version}

> {info.description}

**Base URL**: {servers[0].url}
**Platform**: mintlify-next

---

## POST /accounts/verification

**Summary**: Verify beneficiary bank account

### Parameters
| Name | In | Type | Required | Description |
|------|----|------|----------|-------------|

### Request Body
```json
{ ... }
```

### Responses
| Code | Description |
|------|-------------|
| 200  | Success     |
```

### AI 增強模式（附加在每個 endpoint 之後）

```markdown
### 白話說明
這個 API 讓你驗證受款方的銀行帳號是否有效...

### 何時使用
當你要發起跨境付款前，建議先呼叫此 API...

### 常見使用情境
1. 零售跨境付款前的帳號驗證
2. 批次付款前的預檢查
```

AI prompt：
- system：你是 API 文件助手，將技術規格改寫成開發者容易理解的說明
- user：以下是 `{method} {path}` 的 OpenAPI 規格，請改寫成開發者文件（語言：繁體中文或英文，依原始內容語言）

---

## 整合現有 AITerm 架構

- **AI Provider**：沿用 `src/ipc/provider.ts` 的 `listProviders()`
- **AI 呼叫**：沿用 `src/ipc/ai.ts` 的 `aiChat()`，streaming 模式
- **Tab 類型**：在 `TerminalApp.tsx` 加入 `"api-docs"` 類型
- **i18n**：在 `src/lib/i18n.ts` 補充 `api_docs_tab` 等字串

---

## 錯誤處理

| 錯誤場景 | 處理方式 |
|---------|---------|
| curl_cffi 未安裝 | 萃取前檢查，顯示 `pip install curl_cffi` 指引 |
| 無法偵測平台 | 顯示警告，自動降為 ai-generic 策略 |
| 頁面 timeout | log `✗ Failed`，繼續其餘頁面 |
| OpenAPI 解析失敗 | log 警告，跳過該頁 |
| ai-generic AI 呼叫失敗 | log 錯誤，跳過該頁 |
| AI Provider 未設定 | 「萃取 + AI 增強」按鈕 disable，tooltip 提示 |
| 輸出目錄不存在 | 自動 `mkdir -p` |

---

## 不在範圍內

- 需要登入才能看的文件
- 非 HTTP/HTTPS 來源
- Markdown 以外的輸出格式（PDF、HTML）
- 自動排程 / 定期更新
- 即時同步（只做一次性萃取）
