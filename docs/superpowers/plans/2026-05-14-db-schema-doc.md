# DB Schema Documentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a standalone Doc Converter tab (Word/PDF/Excel → structured Markdown) and schema doc injection into DatabaseAiChat so the AI understands table/column semantics.

**Architecture:** Feature 1 (Doc Converter tab) extracts raw content via JS libraries (SheetJS/mammoth/pdf.js) then batches it through the configured AI provider to produce a `## TABLE_NAME + column table` structured Markdown file. Feature 2 (Schema Injection) reads that `.md` file from localStorage per connection and injects keyword-relevant sections into `buildSystemPrompt()`. A shared `schemaDoc.ts` utility handles all parsing and relevance scoring.

**Tech Stack:** React 19 + TypeScript, xlsx (SheetJS), mammoth, pdfjs-dist, Tauri IPC `aiChat`, Vitest

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `src/lib/schemaDoc.ts` | Create | Parse MD → Map; relevance scoring; build inject string |
| `src/lib/schemaDoc.test.ts` | Create | Unit tests for above |
| `src/components/DocConverter/DocConverterView.tsx` | Create | Full Doc Converter tab UI + extraction + AI normalization |
| `src/components/DocConverter/DocConverterView.css` | Create | Minimal styles |
| `src/components/TabBar/index.tsx` | Modify | Add `"doc-converter"` to `Tab.type` union + `onPickerSelect` signature |
| `src/components/NewTabPicker/index.tsx` | Modify | Add Doc Converter entry |
| `src/components/NewTabPicker/NewTabPicker.test.tsx` | Modify | Cover new tab type |
| `src/lib/i18n.ts` | Modify | Add `doc_converter_tab` + `new_doc_converter_desc` strings |
| `src/components/TerminalApp.tsx` | Modify | Wire `"doc-converter"` in picker + render |
| `src/components/DatabaseView/DatabaseAiChat.tsx` | Modify | Upload button + localStorage + prompt injection |

---

### Task 1: schemaDoc.ts — parse + relevance + inject

**Files:**
- Create: `src/lib/schemaDoc.ts`
- Create: `src/lib/schemaDoc.test.ts`

- [ ] **Step 1: Write failing tests**

```typescript
// src/lib/schemaDoc.test.ts
import { describe, it, expect } from 'vitest';
import { parseSchemaDoc, buildSchemaSection } from './schemaDoc';

const SAMPLE_MD = `## ORDERS
訂單主表

| 欄位名 | 型別 | 說明 |
|--------|------|------|
| ORDER_ID | INT | 訂單唯一識別碼 |
| STATUS | INT | 0=待付款, 1=已完成 |

## CUSTOMERS
客戶主表

| 欄位名 | 型別 | 說明 |
|--------|------|------|
| CUST_ID | INT | 客戶識別碼 |`;

describe('parseSchemaDoc', () => {
  it('parses two sections keyed by lowercase table name', () => {
    const map = parseSchemaDoc(SAMPLE_MD);
    expect(map.size).toBe(2);
    expect(map.has('orders')).toBe(true);
    expect(map.has('customers')).toBe(true);
  });

  it('section text includes the ## header and column table', () => {
    const map = parseSchemaDoc(SAMPLE_MD);
    const section = map.get('orders')!;
    expect(section).toContain('## ORDERS');
    expect(section).toContain('ORDER_ID');
  });

  it('returns empty map for empty string', () => {
    expect(parseSchemaDoc('').size).toBe(0);
  });

  it('handles doc with single section', () => {
    const map = parseSchemaDoc('## PRODUCTS\n\n| 欄位 | 型別 | 說明 |\n|--|--|--|\n| ID | INT | PK |');
    expect(map.size).toBe(1);
    expect(map.has('products')).toBe(true);
  });
});

describe('buildSchemaSection', () => {
  it('returns empty string for empty map', () => {
    expect(buildSchemaSection(new Map(), [], 'query')).toBe('');
  });

  it('includes ## 資料表欄位說明 header when doc is present', () => {
    const map = parseSchemaDoc(SAMPLE_MD);
    const result = buildSchemaSection(map, ['ORDERS', 'CUSTOMERS'], '查詢訂單', 6000);
    expect(result).toContain('## 資料表欄位說明');
  });

  it('ranks ORDERS above CUSTOMERS for order-related question', () => {
    const map = parseSchemaDoc(SAMPLE_MD);
    const result = buildSchemaSection(map, ['ORDERS', 'CUSTOMERS'], '查詢訂單金額', 6000);
    const orderIdx = result.indexOf('## ORDERS');
    const custIdx = result.indexOf('## CUSTOMERS');
    expect(orderIdx).toBeGreaterThanOrEqual(0);
    // ORDERS should appear before CUSTOMERS
    expect(orderIdx).toBeLessThan(custIdx === -1 ? Infinity : custIdx);
  });

  it('moves low-priority tables to TOC when budget is tight', () => {
    const map = parseSchemaDoc(SAMPLE_MD);
    // Very tight budget — only room for one section
    const result = buildSchemaSection(map, [], 'orders', 80);
    expect(result).toContain('其他可用資料表');
  });
});
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
npm run test -- --reporter=verbose schemaDoc
```

Expected: FAIL — `Cannot find module './schemaDoc'`

- [ ] **Step 3: Implement schemaDoc.ts**

```typescript
// src/lib/schemaDoc.ts

/**
 * Parse a structured Markdown schema doc into a map of
 * lowercase table name → full section text (including ## header).
 */
export function parseSchemaDoc(md: string): Map<string, string> {
  const map = new Map<string, string>();
  const lines = md.split('\n');
  let currentTable: string | null = null;
  let currentLines: string[] = [];

  for (const line of lines) {
    if (line.startsWith('## ')) {
      if (currentTable !== null) {
        map.set(currentTable.toLowerCase(), currentLines.join('\n').trimEnd());
      }
      currentTable = line.slice(3).trim();
      currentLines = [line];
    } else if (currentTable !== null) {
      currentLines.push(line);
    }
  }
  if (currentTable !== null && currentLines.length > 0) {
    map.set(currentTable.toLowerCase(), currentLines.join('\n').trimEnd());
  }
  return map;
}

/** Rough token estimate: 1 token ≈ 3 chars (covers mixed Chinese/English). */
function estimateTokens(text: string): number {
  return Math.ceil(text.length / 3);
}

/** Chinese→English keyword hints for relevance scoring. */
const SYNONYMS: [RegExp, string][] = [
  [/訂單/,   'order'],
  [/客戶/,   'customer'],
  [/金額|付款/, 'amount'],
  [/產品|商品/, 'product'],
  [/出貨/,   'ship'],
  [/發票/,   'invoice'],
  [/庫存/,   'inventory'],
  [/員工/,   'employee'],
  [/帳號|帳戶/, 'account'],
];

/**
 * Build the schema section string to inject into a system prompt.
 *
 * High-relevance tables get their full section text.
 * Remaining tables appear as one-line TOC entries.
 *
 * @param docMap        - from parseSchemaDoc()
 * @param dbTableNames  - live table names from dbListTables
 * @param userQuestion  - the user's current question text
 * @param tokenBudget   - max tokens for the whole injected block (default 6000)
 */
export function buildSchemaSection(
  docMap: Map<string, string>,
  dbTableNames: string[],
  userQuestion: string,
  tokenBudget = 6000,
): string {
  if (docMap.size === 0) return '';

  const questionLower = userQuestion.toLowerCase();
  const dbNamesLower = dbTableNames.map(n => n.toLowerCase());

  // Score each table
  const scores = new Map<string, number>();
  for (const tableLower of docMap.keys()) {
    let score = 0;
    if (dbNamesLower.includes(tableLower)) score += 3;
    if (questionLower.includes(tableLower)) score += 2;
    for (const [pattern, keyword] of SYNONYMS) {
      if (pattern.test(questionLower) && tableLower.includes(keyword)) score += 1;
    }
    scores.set(tableLower, score);
  }

  const sorted = [...docMap.keys()].sort(
    (a, b) => (scores.get(b) ?? 0) - (scores.get(a) ?? 0),
  );

  const fullSections: string[] = [];
  const tocEntries: string[] = [];
  let usedTokens = 0;
  const budget80 = tokenBudget * 0.8;

  for (const tableLower of sorted) {
    const section = docMap.get(tableLower)!;
    const tokens = estimateTokens(section);
    if (usedTokens + tokens <= budget80) {
      fullSections.push(section);
      usedTokens += tokens;
    } else {
      const lines = section.split('\n');
      // First non-header, non-table line = short description
      const desc = lines.find((l, i) =>
        i > 0 && l.trim() && !l.startsWith('|') && !l.startsWith('#'),
      ) ?? '';
      const tableName = lines[0].slice(3).trim();
      tocEntries.push(`- ${tableName}${desc ? ': ' + desc.trim() : ''}`);
    }
  }

  const parts: string[] = ['## 資料表欄位說明\n'];
  if (fullSections.length > 0) parts.push(fullSections.join('\n\n'));
  if (tocEntries.length > 0) {
    parts.push('\n其他可用資料表：\n' + tocEntries.join('\n'));
  }
  return parts.join('\n');
}
```

- [ ] **Step 4: Run tests — verify they pass**

```bash
npm run test -- --reporter=verbose schemaDoc
```

Expected: all 8 tests PASS

- [ ] **Step 5: Type check**

```bash
npx tsc --noEmit
```

Expected: no errors

- [ ] **Step 6: Commit**

```bash
git add src/lib/schemaDoc.ts src/lib/schemaDoc.test.ts
git commit -m "feat(db): add schemaDoc utility — parse MD schema docs and build relevance-scored inject strings"
```

---

### Task 2: Tab type + i18n + NewTabPicker + TerminalApp wiring

**Files:**
- Modify: `src/components/TabBar/index.tsx:10`
- Modify: `src/components/TabBar/index.tsx:39`
- Modify: `src/lib/i18n.ts`
- Modify: `src/components/NewTabPicker/index.tsx`
- Modify: `src/components/NewTabPicker/NewTabPicker.test.tsx`
- Modify: `src/components/TerminalApp.tsx:135`
- Modify: `src/components/TerminalApp.tsx` (render section)

- [ ] **Step 1: Write failing test for new tab type**

Add to `src/components/NewTabPicker/NewTabPicker.test.tsx` inside the existing `describe("NewTabPicker")` block:

```typescript
it("renders Doc Converter option", () => {
  render(<NewTabPicker onSelect={() => {}} onClose={() => {}} />);
  expect(screen.getByText("文件轉換器")).toBeInTheDocument();
});

it("calls onSelect with doc-converter when clicked", () => {
  const onSelect = vi.fn();
  render(<NewTabPicker onSelect={onSelect} onClose={() => {}} />);
  fireEvent.click(screen.getByText("文件轉換器"));
  expect(onSelect).toHaveBeenCalledWith("doc-converter");
});
```

- [ ] **Step 2: Run tests — verify new tests fail**

```bash
npm run test -- --reporter=verbose NewTabPicker
```

Expected: 2 new tests FAIL — "文件轉換器" not found

- [ ] **Step 3: Add i18n strings**

In `src/lib/i18n.ts`, add after the `new_vcs_desc` entry in the **zh-TW** section (around line 146):

```typescript
    doc_converter_tab: "文件轉換器",
    new_doc_converter_desc: "將 Word/PDF/Excel 轉換成結構化 Markdown",
```

And after the `new_vcs_desc` entry in the **en** section (around line 315):

```typescript
    doc_converter_tab: "Doc Converter",
    new_doc_converter_desc: "Convert Word/PDF/Excel to structured Markdown",
```

Also add `doc_converter_tab` and `new_doc_converter_desc` to the TypeScript interface definition (search for `vcs_tab:` in the interface and add below it):

```typescript
    doc_converter_tab: string;
    new_doc_converter_desc: string;
```

- [ ] **Step 4: Extend Tab type union**

In `src/components/TabBar/index.tsx` line 10, change:

```typescript
  type: "terminal" | "database" | "design" | "cross-db" | "vcs";
```

to:

```typescript
  type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter";
```

Line 39 — update `onPickerSelect` signature:

```typescript
  onPickerSelect?: (type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter") => void;
```

- [ ] **Step 5: Add Doc Converter entry in NewTabPicker**

In `src/components/NewTabPicker/index.tsx`, update the `Props` interface:

```typescript
interface Props {
  onSelect: (type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter") => void;
  onClose: () => void;
}
```

Add a new button after the VCS button (before the closing `</div>`):

```tsx
      <button
        className="new-tab-picker__item"
        onClick={() => { onSelect("doc-converter"); onClose(); }}
      >
        <span className="new-tab-picker__icon">📄</span>
        <div>
          <div className="new-tab-picker__label">{t.doc_converter_tab}</div>
          <div className="new-tab-picker__desc">{t.new_doc_converter_desc}</div>
        </div>
      </button>
```

- [ ] **Step 6: Wire in TerminalApp**

In `src/components/TerminalApp.tsx`, update `handlePickerSelect` signature and add the title mapping:

```typescript
  const handlePickerSelect = useCallback((type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter") => {
    const newId = crypto.randomUUID();
    let title = "Terminal";
    if (type === "database") title = t.database_tab;
    if (type === "design") title = "Design";
    if (type === "cross-db") title = t.cross_db_tab;
    if (type === "vcs") title = t.vcs_tab;
    if (type === "doc-converter") title = t.doc_converter_tab;
    setTabs((prev) => [...prev, { id: newId, title, type }]);
    setActiveId(newId);
    setPickerOpen(false);
  }, [t.database_tab, t.cross_db_tab, t.vcs_tab, t.doc_converter_tab]);
```

In the render section (around line 310 where `tab.type === "vcs"`), add a new condition:

```tsx
              ) : tab.type === "doc-converter" ? (
                <DocConverterView isActive={isActive} />
```

Add the import at the top of TerminalApp.tsx:

```typescript
import { DocConverterView } from "./DocConverter/DocConverterView";
```

- [ ] **Step 7: Create DocConverterView stub** (so TerminalApp compiles)

Create `src/components/DocConverter/DocConverterView.tsx` with just enough to compile:

```typescript
// src/components/DocConverter/DocConverterView.tsx
export function DocConverterView({ isActive }: { isActive: boolean }) {
  if (!isActive) return null;
  return (
    <div style={{ padding: 24, color: "#e6e6e6" }}>
      Doc Converter — 建置中
    </div>
  );
}
```

- [ ] **Step 8: Run tests — verify all pass**

```bash
npm run test -- --reporter=verbose NewTabPicker
```

Expected: all 6 tests PASS

- [ ] **Step 9: Type check**

```bash
npx tsc --noEmit
```

Expected: no errors

- [ ] **Step 10: Commit**

```bash
git add src/lib/i18n.ts src/components/TabBar/index.tsx src/components/NewTabPicker/index.tsx src/components/NewTabPicker/NewTabPicker.test.tsx src/components/TerminalApp.tsx src/components/DocConverter/DocConverterView.tsx
git commit -m "feat(tabs): add doc-converter tab type and NewTabPicker entry"
```

---

### Task 3: Install npm deps + DocConverterView file extraction

**Files:**
- Modify: `package.json` (via npm install)
- Modify: `src/components/DocConverter/DocConverterView.tsx` (replace stub)
- Create: `src/components/DocConverter/DocConverterView.css`

- [ ] **Step 1: Install dependencies**

```bash
npm install xlsx mammoth pdfjs-dist
```

Expected: package.json and package-lock.json updated with xlsx, mammoth, pdfjs-dist

- [ ] **Step 2: Verify imports compile**

Create a quick test by adding these imports to the top of DocConverterView.tsx temporarily and running tsc:

Actually skip — just write the implementation and run tsc at the end.

- [ ] **Step 3: Implement DocConverterView with extraction only**

Replace `src/components/DocConverter/DocConverterView.tsx` entirely:

```typescript
// src/components/DocConverter/DocConverterView.tsx
import { useState, useRef, useCallback } from "react";
import * as XLSX from "xlsx";
import mammoth from "mammoth";
import * as pdfjsLib from "pdfjs-dist";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { useEffect } from "react";
import "./DocConverterView.css";

// PDF.js worker — use CDN-style inline worker for Tauri (no external network needed)
pdfjsLib.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url,
).toString();

type Format = "excel" | "word" | "pdf";

interface ExtractState {
  format: Format;
  fileName: string;
  rawText: string; // raw extracted text, pre-AI
}

export function DocConverterView({ isActive }: { isActive: boolean }) {
  const [extractState, setExtractState] = useState<ExtractState | null>(null);
  const [extracting, setExtracting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const dropRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
  }, []);

  const detectFormat = (name: string): Format | null => {
    const lower = name.toLowerCase();
    if (lower.endsWith(".xlsx") || lower.endsWith(".xls") || lower.endsWith(".csv")) return "excel";
    if (lower.endsWith(".docx")) return "word";
    if (lower.endsWith(".pdf")) return "pdf";
    return null;
  };

  const extractExcel = async (buffer: ArrayBuffer): Promise<string> => {
    const wb = XLSX.read(buffer, { type: "array" });
    const parts: string[] = [];
    for (const sheetName of wb.SheetNames) {
      const csv = XLSX.utils.sheet_to_csv(wb.Sheets[sheetName]);
      parts.push(`[Sheet: ${sheetName}]\n${csv}`);
    }
    return parts.join("\n\n");
  };

  const extractWord = async (buffer: ArrayBuffer): Promise<string> => {
    const result = await mammoth.extractRawText({ arrayBuffer: buffer });
    return result.value;
  };

  const extractPdf = async (buffer: ArrayBuffer): Promise<string> => {
    const pdf = await pdfjsLib.getDocument({ data: buffer }).promise;
    const pages: string[] = [];
    for (let i = 1; i <= pdf.numPages; i++) {
      const page = await pdf.getPage(i);
      const content = await page.getTextContent();
      const pageText = content.items
        .map((item) => ("str" in item ? item.str : ""))
        .join(" ");
      pages.push(pageText);
    }
    return pages.join("\n");
  };

  const processFile = useCallback(async (file: File) => {
    setError(null);
    setExtractState(null);
    const format = detectFormat(file.name);
    if (!format) {
      setError("不支援的格式。請使用 Excel (.xlsx), Word (.docx) 或 PDF (.pdf)");
      return;
    }
    setExtracting(true);
    try {
      const buffer = await file.arrayBuffer();
      let rawText = "";
      if (format === "excel") rawText = await extractExcel(buffer);
      else if (format === "word") rawText = await extractWord(buffer);
      else rawText = await extractPdf(buffer);

      setExtractState({ format, fileName: file.name, rawText });
    } catch (e) {
      setError(`提取失敗：${String(e)}`);
    } finally {
      setExtracting(false);
    }
  }, []);

  const handleFileInput = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) processFile(file);
    e.target.value = "";
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    const file = e.dataTransfer.files[0];
    if (file) processFile(file);
  };

  if (!isActive) return null;

  return (
    <div className="doc-converter">
      <div className="doc-converter__header">
        <h2>📄 文件轉換器</h2>
        <p>將 Word / PDF / Excel 資料字典轉換成結構化 Markdown Schema 文件</p>
      </div>

      {/* Upload zone */}
      <div
        ref={dropRef}
        className="doc-converter__dropzone"
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDrop}
        onClick={() => fileInputRef.current?.click()}
      >
        {extracting ? (
          <span>⟳ 提取中...</span>
        ) : (
          <>
            <span className="doc-converter__dropzone-icon">📂</span>
            <span>拖放或點擊選擇檔案</span>
            <span className="doc-converter__dropzone-hint">支援 .xlsx .xls .csv .docx .pdf</span>
          </>
        )}
        <input
          ref={fileInputRef}
          type="file"
          accept=".xlsx,.xls,.csv,.docx,.pdf"
          style={{ display: "none" }}
          onChange={handleFileInput}
        />
      </div>

      {error && (
        <div className="doc-converter__error">{error}</div>
      )}

      {/* Provider selector */}
      <div className="doc-converter__toolbar">
        <span className="doc-converter__toolbar-label">AI 模型</span>
        <select
          value={selectedProviderId}
          onChange={(e) => setSelectedProviderId(e.target.value)}
          className="doc-converter__select"
        >
          {providers.map((p) => (
            <option key={p.id} value={p.id}>
              {p.display_name} ({p.model}){p.is_default ? " ★" : ""}
            </option>
          ))}
          {providers.length === 0 && <option value="">（未設定）</option>}
        </select>
      </div>

      {/* Raw extraction preview (debug / confirmation) */}
      {extractState && (
        <div className="doc-converter__raw-preview">
          <div className="doc-converter__raw-header">
            偵測到：{extractState.fileName}（{extractState.format.toUpperCase()}）
            · 提取 {extractState.rawText.length.toLocaleString()} 字元
          </div>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Create CSS**

```css
/* src/components/DocConverter/DocConverterView.css */
.doc-converter {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: #0c0c0c;
  color: #e6e6e6;
  padding: 24px;
  gap: 16px;
  overflow-y: auto;
}

.doc-converter__header h2 {
  margin: 0 0 4px 0;
  font-size: 16px;
  font-weight: 600;
  color: #e6e6e6;
}

.doc-converter__header p {
  margin: 0;
  font-size: 12px;
  color: #666;
}

.doc-converter__dropzone {
  border: 2px dashed #2a2a2a;
  border-radius: 8px;
  padding: 32px 24px;
  text-align: center;
  cursor: pointer;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #888;
  transition: border-color 0.2s, background 0.2s;
}

.doc-converter__dropzone:hover {
  border-color: #4a4a4a;
  background: #111;
}

.doc-converter__dropzone-icon { font-size: 32px; }
.doc-converter__dropzone-hint { font-size: 11px; color: #555; }

.doc-converter__error {
  background: #2a1a1a;
  border: 1px solid #f87171;
  border-radius: 6px;
  padding: 10px 14px;
  font-size: 12px;
  color: #f87171;
}

.doc-converter__toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
}

.doc-converter__toolbar-label { font-size: 11px; color: #555; }

.doc-converter__select {
  background: #0c0c0c;
  border: 1px solid #2a2a2a;
  color: #aaa;
  border-radius: 4px;
  padding: 2px 6px;
  font-size: 11px;
  cursor: pointer;
  outline: none;
}

.doc-converter__raw-header {
  font-size: 11px;
  color: #555;
  padding: 6px 10px;
  background: #111;
  border: 1px solid #1e1e1e;
  border-radius: 4px;
}

.doc-converter__preview {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-height: 0;
}

.doc-converter__preview-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.doc-converter__preview-label { font-size: 11px; color: #555; }

.doc-converter__preview-box {
  flex: 1;
  background: #0a0a0a;
  border: 1px solid #1e1e1e;
  border-radius: 6px;
  padding: 12px;
  overflow-y: auto;
  font-family: monospace;
  font-size: 12px;
  color: #ccc;
  white-space: pre-wrap;
  min-height: 200px;
  max-height: 400px;
}

.doc-converter__actions {
  display: flex;
  gap: 8px;
}

.doc-converter__btn {
  border-radius: 6px;
  padding: 7px 16px;
  font-size: 12px;
  cursor: pointer;
  border: 1px solid;
}

.doc-converter__btn--primary {
  background: #1e3a2e;
  border-color: #34d399;
  color: #34d399;
}

.doc-converter__btn--secondary {
  background: transparent;
  border-color: #2a2a2a;
  color: #888;
}

.doc-converter__btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.doc-converter__progress {
  font-size: 11px;
  color: #888;
  display: flex;
  align-items: center;
  gap: 6px;
}
```

- [ ] **Step 5: Type check**

```bash
npx tsc --noEmit
```

Expected: no errors. If pdfjs-dist types cause issues, add `// @ts-ignore` on the `.str` access or cast `item` to `{ str: string }`.

- [ ] **Step 6: Manual smoke test**

```bash
npm run tauri:dev
```

Open a new Doc Converter tab. Verify:
- Drop zone renders
- Clicking opens file picker
- Dropping a .xlsx file shows "提取 N 字元"
- Dropping an unsupported file shows error message

- [ ] **Step 7: Commit**

```bash
git add package.json package-lock.json src/components/DocConverter/
git commit -m "feat(doc-converter): file extraction — SheetJS/mammoth/pdfjs read raw content"
```

---

### Task 4: DocConverterView — AI normalization + preview + download

**Files:**
- Modify: `src/components/DocConverter/DocConverterView.tsx`

- [ ] **Step 1: Add AI normalization to DocConverterView**

Add the following state and logic to `DocConverterView.tsx`. The component already has `extractState` with raw text. Now we add a second phase that batches the raw text and calls AI.

Add these state variables after the existing ones:

```typescript
  const [mdOutput, setMdOutput] = useState<string>("");
  const [normalizing, setNormalizing] = useState(false);
  const [normalizeProgress, setNormalizeProgress] = useState<{ step: number; total: number } | null>(null);
  const stoppedRef = useRef(false);
```

Add this constant before the component (after imports):

```typescript
const CHUNK_SIZE = 3500; // chars per AI call — ~1000-1200 tokens input

const NORMALIZATION_SYSTEM_PROMPT = `你是資料字典格式化工具。將輸入的原始文字整理成結構化的 Markdown 格式。

每個資料表輸出：
## TABLE_NAME
一行說明（如果有）

| 欄位名 | 型別 | 說明 |
|--------|------|------|
| 欄位1 | 型別 | 說明文字 |

規則：
1. 每張表必須以 ## 開頭的標題行（## 表名）
2. 欄位資訊放在 3 欄 Markdown 表格（欄位名 | 型別 | 說明）
3. 如果原文件未提供型別，填入 -
4. 只輸出 Markdown，不要加解釋文字或開場白
5. 保留原始的資料表名稱和欄位名稱（不要翻譯）`;
```

Add these imports at the top:

```typescript
import { aiChat, formatAiError } from "../../ipc/ai";
```

Add the normalize function inside the component (after `processFile`):

```typescript
  const normalizeWithAi = useCallback(async () => {
    if (!extractState) return;
    setMdOutput("");
    setNormalizing(true);
    stoppedRef.current = false;

    const text = extractState.rawText;
    const totalChunks = Math.ceil(text.length / CHUNK_SIZE);
    setNormalizeProgress({ step: 0, total: totalChunks });

    const parts: string[] = [];
    for (let i = 0; i < totalChunks; i++) {
      if (stoppedRef.current) break;
      setNormalizeProgress({ step: i + 1, total: totalChunks });
      const chunk = text.slice(i * CHUNK_SIZE, (i + 1) * CHUNK_SIZE);
      try {
        const result = await aiChat(
          chunk,
          NORMALIZATION_SYSTEM_PROMPT,
          [],
          selectedProviderId || undefined,
        );
        parts.push(result.trim());
      } catch (e) {
        setError(`AI 正規化失敗（步驟 ${i + 1}）：${formatAiError({ kind: "network", message: String(e) })}`);
        break;
      }
    }

    setMdOutput(parts.join("\n\n"));
    setNormalizing(false);
    setNormalizeProgress(null);
  }, [extractState, selectedProviderId]);
```

Add the download function inside the component:

```typescript
  const downloadMd = useCallback(() => {
    if (!mdOutput) return;
    const baseName = extractState?.fileName.replace(/\.[^.]+$/, "") ?? "schema";
    const blob = new Blob([mdOutput], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${baseName}.md`;
    a.click();
    URL.revokeObjectURL(url);
  }, [mdOutput, extractState]);
```

Update the JSX return to include the new sections (add after the `.doc-converter__raw-preview` div):

```tsx
      {/* AI normalization actions */}
      {extractState && !normalizing && (
        <div className="doc-converter__actions">
          <button
            className="doc-converter__btn doc-converter__btn--primary"
            onClick={normalizeWithAi}
            disabled={!selectedProviderId}
          >
            ✨ AI 正規化
          </button>
          {mdOutput && (
            <button
              className="doc-converter__btn doc-converter__btn--secondary"
              onClick={downloadMd}
            >
              ⬇ 下載 .md
            </button>
          )}
        </div>
      )}

      {normalizing && (
        <div className="doc-converter__actions">
          <div className="doc-converter__progress">
            <span>⟳</span>
            <span>
              AI 正規化中...
              {normalizeProgress && ` (步驟 ${normalizeProgress.step}/${normalizeProgress.total})`}
            </span>
          </div>
          <button
            className="doc-converter__btn doc-converter__btn--secondary"
            onClick={() => { stoppedRef.current = true; }}
          >
            ■ 停止
          </button>
        </div>
      )}

      {/* MD preview */}
      {mdOutput && (
        <div className="doc-converter__preview">
          <div className="doc-converter__preview-header">
            <span className="doc-converter__preview-label">
              預覽（{mdOutput.length.toLocaleString()} 字元）
            </span>
            <button
              className="doc-converter__btn doc-converter__btn--secondary"
              style={{ fontSize: 11, padding: "2px 8px" }}
              onClick={downloadMd}
            >
              ⬇ 下載 .md
            </button>
          </div>
          <pre className="doc-converter__preview-box">{mdOutput}</pre>
        </div>
      )}
```

- [ ] **Step 2: Type check**

```bash
npx tsc --noEmit
```

Expected: no errors

- [ ] **Step 3: Manual smoke test**

```bash
npm run tauri:dev
```

- Open Doc Converter tab
- Drop an Excel file
- Click "AI 正規化"
- Verify progress steps count, output appears in preview box
- Click "⬇ 下載 .md" — verify file downloads with `## TABLE` structure

- [ ] **Step 4: Commit**

```bash
git add src/components/DocConverter/DocConverterView.tsx
git commit -m "feat(doc-converter): AI normalization + preview + download — batched aiChat calls produce structured MD"
```

---

### Task 5: DatabaseAiChat — schema doc upload + localStorage storage

**Files:**
- Modify: `src/components/DatabaseView/DatabaseAiChat.tsx`

- [ ] **Step 1: Add schema doc state + localStorage helpers**

At the top of `DatabaseAiChat.tsx`, add these helpers after the existing `chatStorageKey` / `saveSessions` functions (around line 50):

```typescript
function schemaDocKey(connectionId: string) {
  return `aiterm-db-schema-doc-${connectionId}`;
}

function loadSchemaDoc(connectionId: string): string {
  return localStorage.getItem(schemaDocKey(connectionId)) ?? "";
}

function saveSchemaDoc(connectionId: string, content: string) {
  localStorage.setItem(schemaDocKey(connectionId), content);
}
```

- [ ] **Step 2: Add schema doc state to the component**

Inside `DatabaseAiChat`, add state after the existing `maxStepsRef`:

```typescript
  const [schemaDoc, setSchemaDoc] = useState<string>(() => loadSchemaDoc(connectionId));
  const schemaDocRef = useRef(schemaDoc);
  useEffect(() => { schemaDocRef.current = schemaDoc; }, [schemaDoc]);
```

Reload when `connectionId` changes (add a new `useEffect` after the `listProviders` effect):

```typescript
  useEffect(() => {
    const doc = loadSchemaDoc(connectionId);
    setSchemaDoc(doc);
  }, [connectionId]);
```

- [ ] **Step 3: Add hidden file input + upload/remove functions**

Add inside the component (before the `return`):

```typescript
  const schemaFileInputRef = useRef<HTMLInputElement>(null);

  const handleSchemaUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    file.text().then((content) => {
      saveSchemaDoc(connectionId, content);
      setSchemaDoc(content);
    }).catch(console.error);
    e.target.value = "";
  };

  const removeSchemaDoc = () => {
    localStorage.removeItem(schemaDocKey(connectionId));
    setSchemaDoc("");
  };
```

- [ ] **Step 4: Add 📄 Schema 文件 button to toolbar**

In the toolbar `<div>` (the one containing the history and new-chat buttons, around line 493), add before the `<div style={{ flex: 1 }} />`:

```tsx
          <input
            ref={schemaFileInputRef}
            type="file"
            accept=".md"
            style={{ display: "none" }}
            onChange={handleSchemaUpload}
          />
          <button
            onClick={() => schemaFileInputRef.current?.click()}
            title={schemaDoc ? "更換 Schema 文件" : "上傳 Schema 文件 (.md)"}
            style={{
              background: schemaDoc ? "#1a2a1e" : "transparent",
              border: "1px solid " + (schemaDoc ? "#34d399" : "#2a2a2a"),
              color: schemaDoc ? "#34d399" : "#555",
              borderRadius: 4, padding: "2px 8px", fontSize: 11, cursor: "pointer",
            }}
          >
            📄 Schema{schemaDoc ? " ✓" : ""}
          </button>
          {schemaDoc && (
            <button
              onClick={removeSchemaDoc}
              title="移除 Schema 文件"
              style={{
                background: "transparent", border: "none", color: "#555",
                fontSize: 12, cursor: "pointer", padding: "2px 4px",
              }}
            >
              ×
            </button>
          )}
```

- [ ] **Step 5: Type check**

```bash
npx tsc --noEmit
```

Expected: no errors

- [ ] **Step 6: Manual smoke test**

```bash
npm run tauri:dev
```

- Open a Database tab with a connection
- Click the 📄 Schema button — file picker should open for .md files
- Upload a .md file with `## TABLE` sections
- Verify button shows green "📄 Schema ✓"
- Click × to remove — button returns to grey

- [ ] **Step 7: Commit**

```bash
git add src/components/DatabaseView/DatabaseAiChat.tsx
git commit -m "feat(db-ai): schema doc upload/storage — md file stored in localStorage per connection"
```

---

### Task 6: DatabaseAiChat — inject schema doc into buildSystemPrompt

**Files:**
- Modify: `src/components/DatabaseView/DatabaseAiChat.tsx`

- [ ] **Step 1: Import schemaDoc utilities**

Add to the imports at the top of `DatabaseAiChat.tsx`:

```typescript
import { parseSchemaDoc, buildSchemaSection } from "../../lib/schemaDoc";
```

- [ ] **Step 2: Parse schema doc into a map at load time**

Add a `useMemo` (or derive inline) for the parsed doc map. Add after the `schemaDocRef` declarations:

```typescript
  const schemaDocMap = useMemo(
    () => parseSchemaDoc(schemaDoc),
    [schemaDoc],
  );
```

Add `useMemo` to the React import line:

```typescript
import { useState, useEffect, useRef, useMemo } from "react";
```

- [ ] **Step 3: Update buildSystemPrompt to inject schema section**

Replace the existing `buildSystemPrompt` function:

```typescript
  const buildSystemPrompt = (userQuestion = "") => {
    const tableList = tables.map((t) => t.name).join(", ");
    const maxSteps = maxStepsRef.current;
    const tableNames = tables.map((t) => t.name);
    const schemaSection = buildSchemaSection(schemaDocMap, tableNames, userQuestion, 6000);

    return `你是一個資料庫 Agent，可執行多次 SQL 查詢來回答使用者問題。
Schema：「${schema}」，可用資料表：${tableList || "（載入中）"}。
${schemaSection ? "\n" + schemaSection + "\n" : ""}
【輸出格式規則——違反將導致查詢無法執行】：
1. 需要查詢資料時，僅輸出以下格式，不得有任何前綴說明或後綴說明：
\`\`\`sql
你的SQL
\`\`\`
2. 每次只提供一條純 SQL 語法，不得包在 <cmd>、shell 指令、JSON 或任何其他格式中
3. 已收集足夠資料時，直接用繁體中文給出最終答案，回應中不包含任何 SQL 或程式碼區塊
4. 最多執行 ${maxSteps >= 9999 ? "不限次數" : maxSteps} 次查詢`;
  };
```

- [ ] **Step 4: Pass user question when calling buildSystemPrompt**

In the `send` function, find the two `aiChat` calls and update them to pass the user question to `buildSystemPrompt`:

First call (inside the while loop, around line 285):

```typescript
        const reply = await aiChat(
          lastUserContent,
          buildSystemPrompt(userMsg),   // ← pass userMsg
          loopHistory.slice(0, -1),
          selectedProviderId || undefined,
        );
```

Second call (the summary call after max steps, around line 338):

```typescript
          const summary = await aiChat(
            "請根據以上查詢結果，用繁體中文給出最終完整答案，不要再提供 SQL。",
            buildSystemPrompt(userMsg),   // ← pass userMsg
            loopHistory,
            selectedProviderId || undefined,
          );
```

- [ ] **Step 5: Type check**

```bash
npx tsc --noEmit
```

Expected: no errors

- [ ] **Step 6: Manual end-to-end test**

```bash
npm run tauri:dev
```

Full flow test:
1. Open Doc Converter tab → upload an Excel data dictionary → click AI 正規化 → download the .md
2. Open a Database tab → connect to your DB
3. Click 📄 Schema → upload the .md you just downloaded → verify green ✓
4. In AI chat: type a question referencing tables in your doc
5. Verify in browser devtools console that the system prompt contains `## 資料表欄位說明`
6. Verify AI answer references correct column names from the doc

- [ ] **Step 7: Run full test suite**

```bash
npm run test
```

Expected: all tests pass

- [ ] **Step 8: Type check full project**

```bash
npx tsc --noEmit
```

Expected: no errors

- [ ] **Step 9: Commit**

```bash
git add src/components/DatabaseView/DatabaseAiChat.tsx
git commit -m "feat(db-ai): inject schema doc into system prompt — keyword-based relevance scoring + TOC fallback"
```

---

## Self-Review

### Spec coverage check

| Spec requirement | Task |
|-----------------|------|
| Standalone tab `doc-converter` | Task 2 |
| Excel via SheetJS | Task 3 |
| Word via mammoth | Task 3 |
| PDF via pdfjs-dist | Task 3 |
| Optional Docling MCP | Out of scope (future) |
| AI normalization batched | Task 4 |
| Progress indicator | Task 4 |
| Download .md | Task 4 |
| Output format `## TABLE_NAME + column table` | Task 4 (prompt) |
| `schemaDoc.ts` parse + relevance | Task 1 |
| DatabaseAiChat upload button | Task 5 |
| localStorage per connection | Task 5 |
| buildSystemPrompt injection | Task 6 |
| Token budget with TOC fallback | Task 1 (buildSchemaSection) |
| i18n strings | Task 2 |

### Type consistency check

- `buildSystemPrompt()` → `buildSystemPrompt(userQuestion = "")` — updated all 2 call sites in Task 6 ✓
- `parseSchemaDoc` returns `Map<string, string>` — consumed by `buildSchemaSection` in Task 6 ✓
- `schemaDocMap` is `useMemo` of `parseSchemaDoc(schemaDoc)` — stable across renders ✓
- `Tab.type` union updated in both `TabBar/index.tsx` and `TerminalApp.tsx` ✓
