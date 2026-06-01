/**
 * SWIFT Doc Fetcher — Playwright 驗證腳本
 * 目的：確認能否渲染 SWIFT SPA 並萃取文件內容
 * 用法：node probe.js [url]
 */
const { chromium } = require('playwright');

const TARGET = process.argv[2] ||
  'https://docs.developer.swift.com/docs/api-guides/payment-prevalidation-api/payment-prevalidation-bav-api-reference';

(async () => {
  console.log('啟動 Chromium...');
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
    locale: 'en-US',
  });
  const page = await context.newPage();

  // 攔截 XHR/fetch，找有沒有內容 API
  const apiCalls = [];
  page.on('request', req => {
    if (['fetch', 'xhr'].includes(req.resourceType()))
      apiCalls.push(`[${req.resourceType()}] ${req.url()}`);
  });

  console.log(`前往：${TARGET}`);
  await page.goto(TARGET, { waitUntil: 'domcontentloaded', timeout: 60000 });

  // 等待 main/article 或任何非空內容容器出現
  console.log('等待內容渲染...');
  try {
    await page.waitForFunction(() => {
      const el = document.querySelector('main, article, [class*="content"], [class*="prose"], h1');
      return el && el.innerText && el.innerText.trim().length > 100;
    }, { timeout: 30000 });
  } catch {
    console.warn('等待內容逾時，嘗試擷取現有 DOM...');
  }

  // 1. 頁面標題
  const title = await page.title();
  console.log(`\n=== 標題 ===\n${title}`);

  // 2. 主要文字內容
  const text = await page.evaluate(() => {
    const main = document.querySelector('main, article, [class*="content"]');
    const el = main || document.body;
    return el.innerText.replace(/\s+/g, ' ').trim().slice(0, 3000);
  });
  console.log(`\n=== 主要文字（前 3000 字）===\n${text}`);

  // 3. 頁面所有 h1~h3 標題
  const headings = await page.evaluate(() =>
    [...document.querySelectorAll('h1, h2, h3')]
      .map(h => `${h.tagName}: ${h.innerText.trim()}`)
      .filter(h => h.length > 5)
  );
  console.log(`\n=== 標題結構 ===`);
  headings.forEach(h => console.log(' ', h));

  // 4. API 呼叫
  console.log(`\n=== 背景 API 呼叫（${apiCalls.length} 筆）===`);
  apiCalls.slice(0, 20).forEach(c => console.log(' ', c));

  await browser.close();
  console.log('\n完成。');
})().catch(e => { console.error('錯誤：', e.message); process.exit(1); });
