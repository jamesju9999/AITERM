# WebFetcher 工具使用說明

## 概述

`WebFetcher` 是一個 C# .NET 命令行工具，用於抓取網頁內容。此工具提供給 AI（Claude）透過 Skill 調用，讓 AI 能動態獲取網頁內容。

## 功能特點

- 支援 HTTP/HTTPS 網頁抓取
- 自動添加 https:// 前綴（如果未指定）
- 自動解壓 GZip/Deflate 壓縮內容
- 靜默模式（僅輸出內容）
- 結果保存到文件
- 豐富的錯誤處理

## 編譯與執行

### 編譯專案

```bash
dotnet build
```

### 執行方式

```bash
# 基本用法
dotnet run -- -u https://www.example.com

# 靜默模式（僅輸出內容）
dotnet run -- -u https://www.example.com --silent

# 保存結果到文件
dotnet run -- -u https://www.example.com -o result.txt

# 查看幫助資訊
dotnet run -- --help
```

## 命令列選項

| 選項 | 說明 | 是否必要 |
|------|------|---------|
| `-u, --url <URL>` | 指定要抓取的網址 | 是 |
| `-s, --silent` | 靜默模式，不顯示狀態信息 | 否 |
| `-o, --output <FILE>` | 將結果保存到文件 | 否 |
| `-h, --help` | 顯示幫助信息 | 否 |

## Skill 使用方式

### 格式

AI 可以透過 Skill 機制呼叫此工具，格式如下：

```
/skill urlfetch -u <目标网址>
```

### 範例 Skill 命令

**範例 1: 獲取網頁 HTML 內容**
```
/skill urlfetch -u https://www.example.com
```

**範例 2: 靜默模式獲取內容（僅輸出 HTML）**
```
/skill urlfetch -u https://www.google.com --silent
```

**範例 3: 保存結果到文件**
```
/skill urlfetch -u https://www.github.com --output /tmp/github_page.html
```

**範例 4: 完整輸岀（包含狀態碼）**
```
/skill urlfetch -u https://api.github.com
```

### 預期輸出

**一般模式：**
```
狀態碼：OK

===== HTML 內容 =====
<!DOCTYPE html>
...（網頁內容）...
```

**靜默模式：**
```
<!DOCTYPE html>
...（網頁內容）...
```

### 返回碼

- `0` - 成功
- `1` - 失敗或錯誤

## 進階用法

### 設定 User-Agent

工具預設使用 Chrome 的 User-Agent 來避免被拒絕。

### 處理錯誤

- 無效的 URL 格式
- 網絡超時（預設 30 秒）
- HTTP 錯誤狀態碼（如 404、500 等）

### 批量處理

可以在腳本中循環呼叫此工具來抓取多個網頁：

```bash
for url in url1.com url2.com url3.com; do
  dotnet run -- -u https://$url --silent > ${url}.html
done
```

## 技術細節

- **目標框架**: .NET 9.0
- **主要依賴**: System.Net.Http
- **授權**: MIT License

## 常見問題

**Q: 為什麼抓取某些網站失敗？**
A: 可能是網站的阻擋機制。嘗試使用 `--silent` 模式或使用不同的 User-Agent。

**Q: 返回的內容是壓縮的？**
A: 工具已自動處理 GZip/Deflate 壓縮，會自動解壓後返回。

**Q: 如何處理需要登入的網頁？**
A: 此工具僅提供基本的 HTTP GET 請求，不支持處理 Cookies 或 Session。
