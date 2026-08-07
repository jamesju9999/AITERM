# DB Schema Documentation Design

**Date:** 2026-05-14
**Status:** Approved

## Overview

Two closely related features that together enable AI-powered database queries to be significantly more accurate when the database uses cryptic column names, non-obvious value domains, or complex relationships.

**Feature 1 — Doc Converter Tab:** A standalone tool that converts Word/PDF/Excel data dictionary documents into a structured Markdown file.

**Feature 2 — Schema Doc Injection:** The DatabaseAiChat component accepts the resulting `.md` file and injects relevant sections into the AI system prompt so the AI understands what each table and column means.

---

## Feature 1: Doc Converter Tab

### Entry Point

A new tab type `doc-converter` is added to the existing tab system in `TerminalApp.tsx`. It appears in the **New Tab** picker alongside Terminal, Database, Design, VCS, and Cross-DB.

### Conversion Pipeline

```
┌─ Step 1: Extract raw content ──────────────────────────┐
│                                                         │
│  Excel (.xlsx/.xls)  →  SheetJS (xlsx npm)             │
│  Word  (.docx)       →  mammoth.js (npm)  → HTML       │
│  PDF   (.pdf)        →  pdfjs-dist (npm)  → raw text   │
│                                                         │
│  Optional enhancement: if user has configured a        │
│  Docling MCP server in AITerm settings, Step 1 calls   │
│  the MCP tool instead of JS libraries for all formats. │
└─────────────────────────────────────────────────────────┘
           │
           ▼
┌─ Step 2: AI normalization ──────────────────────────────┐
│                                                         │
│  Batch the raw content (~10 tables per AI call)         │
│  System prompt: "整理成 ## 表名 + 欄位表格格式"          │
│  Uses the user's currently selected AI provider         │
│  Sequential calls, concatenated output                  │
└─────────────────────────────────────────────────────────┘
           │
           ▼
┌─ Step 3: Preview + Download ────────────────────────────┐
│  Live preview of structured MD                          │
│  User may manually edit before downloading              │
│  Download button → saves .md file locally               │
└─────────────────────────────────────────────────────────┘
```

### Output Format (fixed contract)

The AI normalization prompt must always produce this exact structure:

```markdown
## TABLE_NAME
一行說明（如果原文件有說明）

| 欄位名 | 型別 | 說明 |
|--------|------|------|
| COL1 | INT | 說明文字 |
| COL2 | VARCHAR(50) | 說明文字 |

## NEXT_TABLE
...
```

Rules:
- Each table starts with `## TABLE_NAME` (H2, uppercase or as-written in source)
- Optional one-line table description follows
- Column info in a 3-column Markdown table: 欄位名 | 型別 | 說明
- No extra nesting, no sub-sections

This format is the contract between Feature 1 (producer) and Feature 2 (consumer). Feature 2 splits on `## ` anchors, so the format must be consistent.

### UI Layout

```
┌─ Doc Converter ─────────────────────────────────────────┐
│                                                          │
│  ┌─ 上傳區 ──────────────────────────────────────────┐  │
│  │   📂  拖放或選擇 Excel / Word / PDF               │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  ── 偵測到: orders_data_dictionary.xlsx ──────────────── │
│                                                          │
│  ⟳ 提取中... / ⟳ AI 正規化中... (步驟 3/12)              │
│                                                          │
│  ┌─ 預覽 ─────────────────────────────────────────────┐  │
│  │  ## ORDERS                                         │  │
│  │  訂單主表，記錄每一筆客戶購買記錄。                 │  │
│  │                                                    │  │
│  │  | 欄位名 | 型別 | 說明 |                          │  │
│  │  | ORDER_ID | INT | 訂單唯一識別碼（PK） |          │  │
│  │  | STATUS | INT | 0=待付款, 1=處理中, 2=已完成 |    │  │
│  │  ...                                               │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  [ ⬇ 下載 schema.md ]                                    │
└──────────────────────────────────────────────────────────┘
```

### Excel Format Auto-detection

Excel data dictionaries come in two common shapes:

**Multi-sheet (each sheet = one DB table):**
- Detected when: sheet count > 1 and sheet names look like identifiers (uppercase, underscores, no spaces)
- Sheet name → table name; rows → column entries
- Header row auto-detected (AI identifies which column = field name, type, description)

**Single-sheet (all tables, forward-fill):**
- All rows in one sheet; column A = table name (blank for rows belonging to same table)
- AI receives first 5 rows as sample, identifies column mapping
- Forward-fill logic: empty table name cell inherits the last non-empty value

### AI Normalization Prompt (per batch)

```
你是資料字典轉換工具。以下是從 {format} 提取的原始內容：

{raw_content}

請將上述內容整理成以下格式，每張資料表用 ## 表名 開頭：

## TABLE_NAME
一行表說明

| 欄位名 | 型別 | 說明 |
|--------|------|------|
| 欄位1 | 型別 | 說明 |

規則：
1. 每張表必須有 ## 標題
2. 欄位列在 3 欄 Markdown 表格中（欄位名 | 型別 | 說明）
3. 型別若原文件未提供，填 -
4. 只輸出 Markdown，不要說明文字
```

### Docling MCP Integration (Optional)

If the user has configured a Docling MCP server in AITerm Settings → MCP Servers:

- Doc Converter detects the Docling MCP at startup
- Step 1 replaces JS library extraction with a Docling MCP tool call
- Step 2 (AI normalization) still runs on Docling's output
- UI shows "使用 Docling 強化解析" badge when active
- Graceful fallback to JS libraries if MCP call fails

No changes to the Docling MCP integration are required beyond what the MCP settings page already supports.

### npm Dependencies

| Package | Size | Purpose |
|---------|------|---------|
| `xlsx` | ~1.5 MB | Excel parsing (.xlsx, .xls, .csv, .ods) |
| `mammoth` | ~200 KB | Word .docx → HTML |
| `pdfjs-dist` | ~2 MB | PDF text extraction |
| `turndown` | ~30 KB | HTML → Markdown (for Word output) |

All are pure-JS, run entirely in the Webview. No Rust changes needed for conversion.

---

## Feature 2: Schema Doc Injection in DatabaseAiChat

### Storage

```
localStorage["aiterm-db-schema-doc-{connectionId}"] = md_content (string)
```

Capacity: typical 100-table data dictionary ≈ 50–200 KB, well within the 5–10 MB localStorage limit.

### Upload UI

The `DatabaseAiChat` toolbar gains a new button:

```
[ 模型 selector ]  [ 🕐 歷史 ]  [ ＋ 新對話 ]  [ 📄 Schema 文件 ]
```

Clicking **📄 Schema 文件** opens a small popover:
- If no doc attached: "上傳 .md 文件（從 Doc Converter 產出）" + file picker
- If doc attached: shows filename + table count + "移除" button

Accepted file type: `.md` only.

### System Prompt Injection

`buildSystemPrompt()` is extended to include schema documentation. The injection uses a **keyword relevance + TOC fallback** strategy to stay within the token budget.

#### Parsing

On load (when MD is read from localStorage), the MD is parsed into:

```typescript
Map<string, string>  // tableName (lowercase) → full section text
```

Split on `\n## ` anchors. Table name normalized to lowercase for matching.

#### Relevance Scoring

Given user question text and the tables returned by `dbListTables`:

1. **DB table list match**: any table in `dbListTables` whose name appears in the MD map → score +3
2. **Question keyword match**: tokenize user question; for each token check if any table name contains it (case-insensitive, partial match) → score +2 per match
3. **Chinese synonym expansion**: common Chinese terms mapped to likely English table names ("訂單" → order, "客戶" → cust/customer, "金額" → amount/amt) → score +1 per match

Sort tables by score descending.

#### Injection Strategy (token budget: ~6,000 tokens)

```
┌─ Full section (top-N relevant tables) ─────────────────┐
│  ## ORDERS                                              │
│  | ORDER_ID | INT | 訂單唯一識別碼 |                    │  ~500 tokens each
│  ...                                                    │
└─────────────────────────────────────────────────────────┘
┌─ TOC (remaining tables, first line only) ───────────────┐
│  其他可用資料表：                                        │
│  - PRODUCTS: 商品主表                                   │  ~10 tokens each
│  - SHIPPING: 出貨記錄                                   │
└─────────────────────────────────────────────────────────┘
```

Token estimation: 1 token ≈ 4 characters (Chinese text is ~1.5 chars/token).

Full sections are added in relevance order until budget is 80% consumed; remaining budget goes to TOC entries.

#### Updated System Prompt Structure

```
你是一個資料庫 Agent，可執行多次 SQL 查詢來回答使用者問題。
Schema：「{schema}」，可用資料表：{tableList}。

{schemaDocSection}   ← NEW: injected only when MD is present

【輸出格式規則】：
...
```

Where `schemaDocSection` is:

```
## 資料表欄位說明

{full sections for relevant tables}

其他可用資料表：
- TABLE_X: 說明
- TABLE_Y: 說明
```

---

## Linkage Between the Two Features

Option 3 (chosen): **Loose coupling via .md file download.**

```
Doc Converter Tab             DatabaseAiChat
      │                              │
  [ Download .md ]                   │
      │                              │
      │   user saves file locally    │
      │ ─────────────────────────── ▶│
                               [ 📄 Schema 文件 ]
                               uploads .md
                               → stored in localStorage
                               → injected into prompts
```

The two features have no direct code dependency. The `.md` file is the interface.

---

## Files Affected

### New Files
- `src/components/DocConverter/DocConverterView.tsx` — main tab component
- `src/components/DocConverter/DocConverterView.css`
- `src/lib/schemaDoc.ts` — MD parsing + relevance scoring + injection helpers

### Modified Files
- `src/components/TerminalApp.tsx` — add `doc-converter` tab type
- `src/components/TabBar.tsx` (or equivalent) — add Doc Converter to new-tab picker
- `src/components/DatabaseView/DatabaseAiChat.tsx` — upload button + `buildSystemPrompt()` update

### No Rust Changes Required
All conversion logic is frontend JS. Schema doc storage uses localStorage. No new Tauri commands needed for the core feature.

---

## Out of Scope

- Auto-generating schema docs from live DB schema (SELECT column_name, data_type, ...)
- RAG / vector embedding for large schema docs
- Syncing schema docs across devices
- Versioning or diff of schema docs
- Real-time schema doc editing inside AITerm
