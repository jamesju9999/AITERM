# Skill: urlfetch - 網頁內容抓取工具

## 描述
這個 Skill 允許 AI 系統動態抓取指定網址的網頁內容。使用 C# .NET 實現的命令列工具，提供穩定可靠的網頁抓取功能。

## 工具位置
```
/Users/jamesju/Documents/CodeSample/JAVA/SwiftRefAPI-LLM/UrlFetcher
```

## 使用方式

### 基本 Skill 調用格式

**語法：**
```
/skill urlfetch -u <URL> [--silent] [-o <output-file>]
```

### 常用範例

#### 1. 抓取網頁 HTML 內容
```
/skill urlfetch -u https://www.example.com
```

#### 2. 靜默模式（僅輸出 HTML 內容，不顯示狀態資訊）
```
/skill urlfetch -u https://www.google.com --silent
```

#### 3. 保存結果到文件
```
/skill urlfetch -u https://www.github.com -o /tmp/github_page.html
```

#### 4. 獲取 API 響應內容
```
/skill urlfetch -u https://api.github.com/users/octocat --silent
```

#### 5. 抓取需要自我調整 URL 的網站
```
/skill urlfetch -u www.example.com  # 自動添加 https:// 前綴
```

## 命令列選項

| 選項 | 長格式 | 說明 | 必要性 |
|------|--------|------|--------|
| `-u` | `--url` | 指定要抓取的網址 | **必要** |
| `-s` | `--silent` | 靜默模式，不顯示狀態信息 | 可選 |
| `-o` | `--output` | 將結果保存指定的文件 | 可選 |
| `-h` | `--help` | 顯示幫助信息 | 可選 |

## 輸出格式

### 一般模式（預設）
```
狀態碼：OK

===== HTML 內容 =====
<!DOCTYPE html>
<html>
...
</html>
```

### 靜默模式
```
<!DOCTYPE html>
<html>
...
</html>
```

## 返回碼

| 返回碼 | 說明 |
|--------|------|
| `0` | 成功 |
| `1` | 失敗或錯誤 |

## 進階使用技巧

### 在腳本中使用
```bash
#!/bin/bash
URL="https://www.example.com"
RESULT=$(dotnet run --project UrlFetcher -u "$URL" --silent)
echo "$RESULT" > output.html
```

### 批量抓取
```bash
for url in url1.com url2.com url3.com; do
  dotnet run --project UrlFetcher -u "https://$url" --silent -o "${url}.html"
done
```

### 與其他工具結合
```bash
# 抓取後提取特定內容
dotnet run --project UrlFetcher -u https://www.example.com --silent | grep -o '<title>.*</title>'
```

## 注意事項

1. **URL 格式**: 如果未提供 `http://` 或 `https://` 前綴，工具會自動添加 `https://`
2. **超時設定**: 預設超時時間為 30 秒
3. **User-Agent**: 預設使用 Chrome 瀏覽器的 User-Agent 以避免被阻擋
4. **壓縮處理**: 自動解壓 GZip/Deflate 壓縮的響應內容
5. **重定向**: 自動跟隨 HTTP 重定向

## 錯誤處理

### 常見錯誤及解決方案

| 錯誤類型 | 解決方案 |
|----------|----------|
| 網址無效 | 檢查 URL 格式是否正確 |
| 網絡超時 | 增加超時時間或檢查網絡連接 |
| HTTP 錯誤（404/500） | 檢查網址是否正確，或網站暫時不可用 |
| 被網站阻擋 | 嘗試使用不同的 URL 或時間 |

## 技術規格

- **目標框架**: .NET 9.0
- **執行方式**: `dotnet run --project UrlFetcher [選項]`
- **編譯後二進位**: `bin/Debug/net9.0/UrlFetcher`

## 相關檔案

- `Program.cs` - 主程序碼
- `UrlFetcher.csproj` - 專案配置文件
- `README.md` - 完整文檔
- `skill-urlfetch.md` - 此 Skill 說明文件

## 給 AI 開發者的建議

當您需要使用此 Skill 時：

1. 明確指定目標 URL
2. 根據需求選擇是否使用 `--silent` 模式
3. 如果需要保存結果，使用 `-o` 指定文件路徑
4. 檢查返回碼確認操作是否成功

此 Skill 適合用於：
- 網頁內容分析
- API 數據獲取
- 網際網路資訊抓取
- 自動化測試資料收集
