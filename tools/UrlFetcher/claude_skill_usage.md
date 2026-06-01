# WebFetcher Skill 使用說明

## 概述

`WebFetcher` 是一個 C# .NET 命令行工具，提供給 AI（Claude）透過 Skill 調用，用於抓取網頁內容。

## Skill 配置文件

```yaml
name: webfetch
description: Fetch website content from command line
model: claude-opus-4-6
runs: ["dotnet", "run", "--project", "UrlFetcher/Program.cs"]
icon: https://raw.githubusercontent.com/octocat/octocat/main/octocat.png
open: []
image: https://raw.githubusercontent.com/octocat/octocat/main/octocat.png

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

## 使用方式

### 1. 基本抓取

```bash
dotnet run --project UrlFetcher -u https://www.example.com
```

### 2. 靜默模式（適合自動化處理）

```bash
dotnet run --project UrlFetcher -u https://www.example.com -s
```

### 3. 保存結果到文件

```bash
dotnet run --project UrlFetcher -u https://www.example.com -o output.txt
```

### 4. 完整參數

```bash
dotnet run --project UrlFetcher -u https://www.example.com -s -o output.txt
```

## Skill 參數說明

| 參數 | 類型 | 必填 | 說明 |
|------|------|------|------|
| `url` | string | 是 | 要抓取的網址 |
| `silent` | bool | 否 | 靜默模式，不顯示狀態信息 |
| `output` | string | 否 | 將結果保存到的文件路徑 |

## 返回碼

| 返回碼 | 說明 |
|--------|------|
| 0 | 成功 |
| 1 | 失敗或錯誤 |

## 錯誤處理

### 常見的錯誤情況

1. **無效的網址**
   ```
   錯誤：無法解析的 URL 格式
   ```

2. **網絡超時**
   ```
   錯誤：請求超時（超過 30 秒）
   ```

3. **未知選項**
   ```
   錯誤：Unknown option: --unknown
   ```

4. **缺少必填參數**
   ```
   错误：必须指定网址 (-u 或 --url)
   ```

## AI Skill 集成示例

### 用法 1：抓取網頁內容進行分析

```yaml
- name: webfetch
  spec:
    url: "https://www.example.com"
    silent: true
  output: result.txt
```

### 用法 2：獲取多個網站內容

```yaml
- name: webfetch
  spec:
    url: "https://www.example.com/page1"
  output: page1.txt

- name: webfetch
  spec:
    url: "https://www.example.com/page2"
  output: page2.txt
```

### 用法 3：靜默模式批量處理

```bash
dotnet run --project UrlFetcher -u https://api.example.com/data -s -o data.json
```

## 注意事項

1. **User-Agent**: 工具使用 Chrome 瀏覽器的 User-Agent，模擬正常用戶訪問
2. **自動 https**: 如果 URL 沒有協議前綴，工具會自動添加 `https://`
3. **重定向**: 自動跟隨 HTTP 重定向
4. **解壓縮**: 支援 GZip 和 Deflate 解壓縮
5. **超時時間**: 30 秒超時設定

## 擴展功能建議

- [ ] 添加更多 HTTP 方法（POST, PUT, DELETE）
- [ ] 添加自定義請求頭
- [ ] 添加身份驗證支持
- [ ] 添加結果格式化輸出（JSON, XML 等）
- [ ] 添加並行抓取功能

## 開發者信息

### 快速編譯與運行

```bash
# 進入項目目錄
cd UrlFetcher

# 恢復依賴
dotnet restore

# 編譯
dotnet build

# 運行
dotnet run -- -u https://www.example.com
```

### 代碼結構

- `Program.cs`: 主程序文件，包含命令行解析器和 HTTP 請求邏輯
- `claude_skill.yaml`: Skill 配置文件
- `claude_skill_usage.md`: Skill 使用說明文檔
