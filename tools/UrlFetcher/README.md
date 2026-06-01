# WebFetcher - .NET 命令行網頁內容抓取工具

一個簡單、高效的命令行工具，用於抓取網頁內容。設計給 AI（Claude）透過 Skill 調用，用於動態獲取網頁內容。

## 功能特點

- **動態指定網址**: 可以通過命令行參數靈活指定要抓取的網址
- **自動補充協議**: 如果網址缺少 `http://` 或 `https://`，工具會自動添加
- **靜默模式**: 適合自動化處理和腳本調用
- **結果存檔**: 可以將抓取結果保存到文件
- **AI 集成**: 提供完備的 Skill 配置文件，方便 AI 系統調用

## 環境要求

- .NET 9.0 SDK 或更高版本
- 網路連接（用於抓取網頁）

## 安裝與運行

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

## 命令行選項

| 選項 | 說明 | 必填 |
|------|------|------|
| `-u, --url <URL>` | 指定要抓取的網址 | 是 |
| `-s, --silent` | 靜默模式，不顯示狀態信息 | 否 |
| `-o, --output <FILE>` | 將結果保存到的文件路徑 | 否 |
| `-h, --help` | 顯示幫助信息 | 否 |

## 使用示例

### 示例 1: 抓取網頁內容

```bash
dotnet run --project UrlFetcher -u https://www.google.com
```

### 示例 2: 使用完整選項名稱

```bash
dotnet run --project UrlFetcher --url "https://www.example.com"
```

### 示例 3: 靜默模式 + 保存到文件

```bash
dotnet run --project UrlFetcher -u https://www.example.com -s -o result.txt
```

## Skill 配置文件

### claude_skill_config.yaml 文件結構

```yaml
name: urlfetch
description: Fetch website content from command line. Allows AI to dynamically grab website content.
model: claude-opus-4-6

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

## 參數說明

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

## 技術細節

### User-Agent

工具使用 Chrome 瀏覽器的 User-Agent：

```
Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36
```

### 網絡配置

- 自動跟隨 HTTP 重定向
- 支援 GZip 和 Deflate 解壓縮
- 30 秒超時設定

### 自動 URL 修正

如果輸入的網址沒有協議前綴，工具會自動添加 `https://` 前綴。

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

## 目錄結構

```
UrlFetcher/
├── Program.cs                    # 主程序文件
├── UrlFetcher.csproj             # 專案配置文件
├── README.md                     # 使用說明文檔
├── SKILL_USAGE.md               # Skill 使用說明書
├── SAMPLE_USAGE.md              # Skill 使用範例
├── claude_skill_config.yaml     # Skill 配置文件
└── USAGE_ZH.md                  # 中文使用說明
```

## 開發者信息

### 代碼結構

- **Program.cs**: 主程序文件，包含命令行解析器和 HTTP 請求邏輯
  - `Main`: 程序入口點
  - `DisplayHelp`: 顯示幫助信息
  - `BuildClient`: 創建 HttpClient 配置
  - `FetchUrlContent`: 抓取網頁內容

## 擴展功能建議

- [ ] 添加更多 HTTP 方法（POST, PUT, DELETE）
- [ ] 添加自定義請求頭
- [ ] 添加身份驗證支持
- [ ] 添加結果格式化輸出（JSON, XML 等）
- [ ] 添加並行抓取功能

## 文檔列表

| 文件 | 說明 |
|------|------|
| `README.md` | 项目完整说明 |
| `SKILL_USAGE.md` | Skill 使用说明书 |
| `SAMPLE_USAGE.md` | Skill 使用范例 |
| `USAGE_ZH.md` | 中文使用说明 |
| `claude_skill_config.yaml` | Skill 配置文件 |

## 許可證

MIT License
