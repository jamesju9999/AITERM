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
