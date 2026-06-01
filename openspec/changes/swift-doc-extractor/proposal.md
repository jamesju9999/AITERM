# Proposal: SWIFT Doc Extractor Tab

## 概述

在 AITerm 新增一個專屬 Tab，讓使用者能從 SWIFT 開發者文件網站（docs.developer.swift.com）萃取 API 參考文件，並匯出為 AI 可消費的 Markdown 檔案。

## 目標

- 使用者輸入 SWIFT 文件 URL（如 `https://docs.developer.swift.com/docs`）
- 顯示樹狀結構的文件章節，支援全選 / 部分選 / 關鍵字過濾
- 萃取選定章節轉為 Markdown
- 可設定輸出路徑，以及合併成單一檔案或分開存放
- 輸出格式適合放入 Claude Project Knowledge / GPT Custom Instructions 等

## 使用者故事

1. 使用者在 Tab 輸入文件根網址
2. App 爬取左側 sidebar，顯示完整樹狀文件結構
3. 使用者透過 checkbox 樹選取需要的章節（可關鍵字過濾）
4. 設定輸出目錄與合併模式
5. 點擊「萃取」，App 逐頁抓取並轉為 Markdown
6. 完成後顯示產生的檔案清單

## 技術偵察摘要（explore 階段結論）

### 網站特性
- `docs.developer.swift.com` 是 JavaScript SPA
- WAF 偵測非瀏覽器流量並靜默 drop（TLS 握手成功但 HTTP 無回應）
- 靜態 HttpClient（UrlFetcher 現行實作）完全無法取得內容
- curl + 完整 Chrome headers 同樣被擋 → 判斷為 TLS fingerprint 或 TCP 行為偵測

### UrlFetcher 狀態
- UrlFetcher 本身運作正常（httpbin.org / example.com 均可正確抓取）
- 問題在 SWIFT 伺服器端，非工具問題

### 解決方向：Playwright headless Chromium
- 啟動真實 Chromium，可繞過 WAF 的 TLS fingerprint 偵測
- 等待 `networkidle` 確保 SPA 渲染完成後再抓 DOM
- 環境已就緒：Node.js v25.2.1、npx playwright 1.60.0
- Chromium 尚未安裝（需 `npx playwright install chromium`，~130MB）

## 架構選項（待驗證後決定）

### 選項 A：Node.js 腳本（推薦先驗證）
```
tools/SwiftDocFetcher/
  fetcher.js          ← Playwright 爬蟲腳本
  package.json

Tauri → spawn Node.js process → 回傳 JSON / Markdown
```
- 優點：Playwright Node.js API 最成熟、輕量
- 缺點：依賴使用者機器有 Node.js

### 選項 B：擴充 UrlFetcher（.NET + Microsoft.Playwright）
```
tools/UrlFetcher/
  UrlFetcher.csproj   ← 加入 Microsoft.Playwright 套件
  Program.cs          ← 新增 --playwright flag
```
- 優點：統一入口，與現有 UrlFetcher skill 整合
- 缺點：.csproj 變重，Chromium 打包複雜度高

## 開放問題

1. **Playwright 驗證**：能否真正繞過 SWIFT WAF？→ 待 Node.js 腳本測試確認
2. **樹狀結構來源**：sidebar 是 API 回傳 JSON 還是渲染在 DOM？需要觀察 Network tab
3. **登入牆**：SWIFT 文件是否部分內容需要登入？
4. **輸出格式**：
   - 每個 API endpoint 一個 md？還是每個章節一個 md？
   - 是否保留 code sample、request/response schema？
5. **整合方式**：合併 md 的大小上限（Claude Project 單檔 < 500KB）

## 下一步

1. 離開 explore 模式
2. 寫 Node.js Playwright 驗證腳本
3. 跑 `npx playwright install chromium`
4. 測試能否取得 SWIFT 頁面內容
5. 觀察 DOM 結構 → 決定 sidebar 爬取策略
6. 確認後選擇架構方案，建立 implementation plan
