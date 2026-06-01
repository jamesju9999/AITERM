# WebFetcher Skill 使用範例

## 快速開始

### 1. 基本用法

```bash
/skill urlfetch -u https://www.example.com
```

**執行結果：**

```
自動添加 https://前綴：https://www.example.com

狀態碼：OK

===== HTML 內容 =====
<!DOCTYPE html>
...
</html>
```

### 2. 靜默模式

```bash
/skill urlfetch -u https://www.google.com --silent
```

**執行結果：**（只輸出 HTML 內容，不顯示狀態信息）

```
<!DOCTYPE html>
...
</html>
```

### 3. 保存結果到文件

```bash
/skill urlfetch -u https://www.github.com -o /tmp/github_page.html
```

### 4. 獲取 API 響應內容

```bash
/skill urlfetch -u https://api.github.com/users/octocat --silent
```

## 命令列選項詳解

### 必填參數

| 選項 | 說明 | 範例 |
|------|------|------|
| `-u, --url <URL>` | 指定要抓取的網址 | `-u https://www.example.com` |

### 可選參數

| 選項 | 說明 | 範例 |
|------|------|------|
| `-s, --silent` | 靜默模式，不顯示狀態信息 | `--silent` |
| `-o, --output <FILE>` | 將結果保存到的文件 | `-o output.txt` |
| `-h, --help` | 顯示幫助信息 | `--help` |

## 使用場景

### 場景 1: 抓取網頁內容進行分析

```bash
/skill urlfetch -u "https://www.example.com" | grep -o '<title>.*</title>'
```

### 場景 2: 批量抓取多個網站

```bash
for url in site1.com site2.com site3.com; do
  /skill urlfetch -u "https://$url" --silent -o "${url}_page.html"
done
```

### 場景 3: API 數據獲取

```bash
/skill urlfetch -u "https://api.github.com/repos/mono/mono" \
  -s -o repository_data.json
```

### 場景 4: 網頁自動測試

```bash
/skill urlfetch -u "https://www.example.com" --silent > test_output.html
# 然後對 test_output.html 進行測試驗證
```

### 場景 5: 網頁內容提取

```bash
# 提取標題
/skill urlfetch -u "https://www.example.com" --silent | grep -o '<h1>.*</h1>'

# 提取連結
/skill urlfetch -u "https://www.example.com" --silent | grep -o 'href="[^"]*"'
```

## 完整的 Claude Skill 配置文件

```yaml
name: urlfetch
description: Fetch website content from command line. Allows AI to dynamically grab website content.
model: claude-opus-4-6

runs:
  - dotnet
  - run
  - --project
  - UrlFetcher/Program.cs

spec:
  - name: url
    description: The URL to fetch content from
    prompt: the URL
    required: true

  - name: silent
    description: Silent mode, do not display status information
    prompt: silent mode flag
    required: false

  - name: output
    description: Save the result to a file
    prompt: output file path
    required: false
```

## 返回碼說明

| 返回碼 | 說明 | 處理方式 |
|--------|------|---------|
| 0 | 成功 | 繼續處理網頁內容 |
| 1 | 失敗或錯誤 | 檢查錯誤信息，重新嘗試 |

## 常見問題解答

### Q1: 為什麼我的 URL 顯示自動添加 https://？
A: 如果您提供的 URL 沒有協議前綴，工具會自動添加 `https://` 前綴。例如：
- 輸入：`www.example.com`
- 處理後：`https://www.example.com`

### Q2: 如何避免顯示狀態信息？
A: 使用 `--silent` 或 `-s` 參數：
```bash
/skill urlfetch -u https://www.example.com --silent
```

### Q3: 如何保存網頁內容到文件？
A: 使用 `-o` 或 `--output` 參數：
```bash
/skill urlfetch -u https://www.example.com -o page.html
```

### Q4: 什麼是 HTTP 狀態碼？
A: 狀態碼表示 HTTP 請求的結果，例如：
- `200 OK` - 成功
- `404 Not Found` - 網頁不存在
- `500 Internal Server Error` - 服務器錯誤

### Q5: 如何處理需要登錄的網站？
A: 目前版本不支持身份驗證。對於需要登錄的網站，需要使用其他方法處理。

## 技術詳解

### User-Agent 配置

工具使用 Chrome 瀏覽器的 User-Agent 模擬正常用戶訪問：

```
Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36
```

### 網絡配置

- 自動跟隨 HTTP 重定向
- 支援 GZip 和 Deflate 解壓縮
- 30 秒超時設定

### 返回值類型

- **成功時返回碼**: 0
- **失敗時返回碼**: 1

## 最佳實踐

1. **始終提供完整的 URL**（包含 https://）
2. **使用靜默模式**處理大量網頁抓取
3. **保存結果到文件**進行後續分析
4. **檢查返回碼**確保操作成功
5. **處理網絡超時**時的錯誤情況

## 相關文檔

- [README.md](README.md) - 完整项目说明
- [USAGE_ZH.md](USAGE_ZH.md) - 中文使用指南
- [claude_skill_config.yaml](claude_skill_config.yaml) - Skill 配置文件
- [SKILL_USAGE.md](SKILL_USAGE.md) - Skill 使用說明書

## 更新記錄

### v1.0.0 (2026-03-18)
- 初始版本發布
- 支持基本網頁抓取
- 支持靜默模式
- 支持結果保存到文件
- 提供 Claude Skill 配置文件
