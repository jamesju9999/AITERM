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
  const dbNamesSet = new Set(dbTableNames.map(n => n.toLowerCase()));

  // Score each table
  const scores = new Map<string, number>();
  for (const tableLower of docMap.keys()) {
    let score = 0;
    if (dbNamesSet.has(tableLower)) score += 3;
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
    const section = docMap.get(tableLower) ?? '';
    if (!section) continue;
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
