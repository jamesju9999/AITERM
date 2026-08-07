# AITerm 設計規格

- **日期**:2026-04-10
- **狀態**:Draft (待 user review)
- **作者**:James Chu (with Claude)

## 1. 專案目標

打造一款類似 Warp 的跨平台 AI 終端機桌面應用,主要特色:

1. 使用者可用自然語言指示 AI 產生並執行 CLI 命令,無需記憶繁瑣指令
2. 支援雲端與本地端 AI 後端,可同時設定多個並自由切換
3. UI 驅動的設定體驗,降低一般使用者的設定門檻
4. 內建命令安全分級機制,避免 AI 產出的危險命令被誤執行

## 2. 技術選型

| 面向 | 選擇 | 理由 |
|---|---|---|
| 桌面框架 | **Tauri** | 啟動快 (~20MB 記憶體)、原生效能、Rust 生態與終端機/PTY/本地 AI 整合佳 |
| 後端語言 | **Rust** | 效能、記憶體安全、成熟 PTY crate (`portable-pty`)、Warp 本身即 Rust |
| 前端 | **React + TypeScript** | 生態成熟、`xterm.js` 即業界標準 |
| 終端機渲染 | **xterm.js** | VS Code / Warp 皆採用,穩定可靠 |
| 設定檔格式 | **TOML** | 人類可讀、Rust 原生支援、適合 UI 與檔案雙向同步 |
| 密鑰儲存 | **OS keychain** | Windows Credential Manager / macOS Keychain / libsecret |
| 本地 AI | **Ollama + OpenAI-Compatible** | 涵蓋 Ollama、LM Studio、llama.cpp server、vLLM |
| 打包目標 | Windows MSI (v1 主平台),macOS / Linux 為次要 |

## 3. 系統架構

```
┌───────────────────────────────────────────────────────┐
│  AITerm (Tauri App)                                   │
│                                                       │
│  ┌─────────── Frontend (React + xterm.js) ────────┐   │
│  │  • TerminalView (xterm.js, 多分頁)             │   │
│  │  • AIPanel (側邊多輪對話, Ctrl+I 開啟)         │   │
│  │  • CommandPreview (AI 產出命令的確認框)        │   │
│  │  • SettingsView                                │   │
│  └──────────────┬─────────────────────────────────┘   │
│                 │ Tauri IPC (invoke / event)          │
│  ┌──────────────▼─────────────────────────────────┐   │
│  │  Backend (Rust)                                │   │
│  │                                                │   │
│  │  pty_manager     ai_router      context_builder│   │
│  │  command_guard   config_store   secret_store   │   │
│  └────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────┘
```

### 3.1 模組職責

每個 Rust 模組都必須能獨立單元測試。

| 模組 | 職責 |
|---|---|
| `pty_manager` | 建立 shell 子行程、管理 PTY I/O、多分頁 session |
| `ai_router` | 抽象 AI 後端 (`trait AiProvider`),支援串流,統一錯誤分類 |
| `context_builder` | 收集 OS/shell/cwd/環境變數/最近 N 條命令作為 AI 上下文 |
| `command_guard` | 命令風險分級引擎 (Safe / NeedsConfirm / Dangerous / Blocked) |
| `config_store` | TOML 設定讀寫,UI 層的儲存後端,原子寫入 |
| `secret_store` | OS keychain 封裝,管理 API Key CRUD |

### 3.2 前後端邊界

- 前端 **不直接呼叫** 任何 AI API。所有 AI 請求都透過 Tauri invoke 進後端,避免 API Key 洩漏到 WebView。
- 後端透過 Tauri event 向前端推送串流 chunk、PTY 輸出、AI 命令就緒通知等,避免長時間阻塞 IPC。

## 4. 核心資料流

### 4.0 執行模式定義

使用者可隨時透過 `Ctrl+Shift+M` 切換以下三種執行模式:

| 模式 | 行為 |
|---|---|
| **A. 一律確認 (預設)** | 任何 AI 產出的命令都顯示預覽框,需使用者明確確認才執行 |
| **B. 分級自動** | 依 `command_guard` 風險分級決定:`Safe` 自動執行;`NeedsConfirm`/`Dangerous` 顯示預覽框;`Blocked` 不提供執行 |
| **C. 全自動 Agent** | `Safe` 與 `NeedsConfirm` 自動執行;`Dangerous` 仍強制確認;`Blocked` 拒絕 |

### 4.1 流程 A — 一般命令 (非 AI)

```
使用者輸入 "ls -la" → TerminalView (xterm.js)
  → Tauri invoke "pty_write" → pty_manager 寫入 shell stdin
  → shell 執行 → stdout/stderr → pty_manager 讀取
  → Tauri event "pty_data" → TerminalView 渲染
```

純透傳,AI 不介入。

### 4.2 流程 B — 前綴觸發 AI (`/ai`)

```
使用者輸入 "/ai 找出大於 100MB 的檔案"
  ↓
前端偵測 "/ai " 前綴 → 不送 PTY,改 invoke "ai_query"
  ↓
ai_router:
  1. context_builder.snapshot() 組出 {os, shell, cwd, 最近 5 條命令}
  2. 依使用者設定選擇目前 provider (雲/本地)
  3. 組 system prompt,要求 AI 輸出結構化 JSON {explanation, command, risk_level}
  4. 串流呼叫 provider.generate()
  ↓
AI 回傳結構化結果
  ↓
command_guard.classify(cmd) → Safe / NeedsConfirm / Dangerous / Blocked
  ↓
Tauri event "ai_command_ready" → 前端 CommandPreview
  ↓
依執行模式:
  • 模式 A (一律確認):顯示預覽框
  • 模式 B (分級):Safe 直接 pty_write;其他顯示預覽框
  • 模式 C (全自動):Safe/NeedsConfirm 自動執行,Dangerous 強制確認,Blocked 拒絕
  ↓
使用者確認後 → pty_write → 走流程 A 後半段
```

### 4.3 流程 C — AI 面板多輪對話 (`Ctrl+I`)

```
Ctrl+I → 開啟右側 AIPanel
  ↓
使用者在面板輸入自然語言 (可多輪)
  ↓
前端維護 conversation history (陣列)
  ↓
invoke "ai_chat" 帶整段 history + 最新 snapshot
  ↓
ai_router 串流回傳文字 + 可能夾帶 <cmd>...</cmd> 標記的建議命令
  ↓
前端渲染對話泡泡;若偵測到 <cmd> → 顯示「執行此命令」按鈕
  ↓
使用者點按鈕 → 走流程 B 的確認/執行路徑
```

### 4.4 AI 回應格式

兩種流程使用不同格式,各有理由:

| 流程 | 回應格式 | 理由 |
|---|---|---|
| 4.2 行內 `/ai` | **純結構化 JSON** `{explanation, command, risk_level}` | 目標是「產一條可執行命令」,格式固定才能穩定解析並送 `command_guard` |
| 4.3 多輪面板 | **自由文字 + `<cmd>...</cmd>` 標籤包住的建議命令** | 目標是對話與多步推理,需要自然文字;`<cmd>` 標籤讓前端能精準抽出可執行片段 |

### 4.5 關鍵設計決策

1. **AI 串流全程透過 Tauri event**,不阻塞 invoke
2. **Context snapshot 快取 + diff**:每次 query 只送變化部分,節省 token
3. **最近命令上下文上限可設定**,預設 5 條,避免敏感輸出外洩
4. **結構化解析優先於正則**:§4.4 的兩種格式都能讓前端用 schema 驗證或 XML 抽取,而非靠脆弱的正則猜測

## 5. AI Provider 抽象

### 5.1 Trait 定義

```rust
pub struct GenerateRequest {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub context: EnvSnapshot,
    pub mode: QueryMode,               // SingleCommand | Chat
    pub max_tokens: Option<u32>,
}

pub struct GenerateChunk {
    pub delta: String,
    pub done: bool,
    pub usage: Option<TokenUsage>,
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn capabilities(&self) -> Capabilities;

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError>;

    async fn health_check(&self) -> Result<(), AiError>;
}
```

### 5.2 v1 內建 Providers

| Provider | 類別 | 端點 |
|---|---|---|
| `OpenAiClient` | 雲端 | `https://api.openai.com/v1/chat/completions` (SSE) |
| `AnthropicClient` | 雲端 | `https://api.anthropic.com/v1/messages` (SSE) |
| `GeminiClient` | 雲端 | Google Gemini REST |
| `OllamaClient` | 本地 | `http://localhost:11434/api/chat` |
| `OpenAiCompatibleClient` | 通用 | 使用者自填 base_url,涵蓋 LM Studio、vLLM、OpenRouter、DeepSeek、自架服務 |

`OpenAiCompatibleClient` 是降低維護成本的關鍵:一個實作涵蓋眾多相容服務。

### 5.3 設定檔範例

```toml
default_provider = "claude-sonnet"

[[providers]]
id = "claude-sonnet"
type = "anthropic"
model = "claude-sonnet-4-5"
# api_key 存在 keychain,key 為 "aiterm:claude-sonnet"

[[providers]]
id = "local-llama"
type = "ollama"
base_url = "http://localhost:11434"
model = "llama3.1:8b"

[[providers]]
id = "lm-studio"
type = "openai-compatible"
base_url = "http://localhost:1234/v1"
model = "qwen2.5-coder-7b"
```

設定檔位置:`%APPDATA%/AITerm/config.toml` (Windows)、`~/.config/aiterm/config.toml` (Unix)。

### 5.4 錯誤分類

| 類別 | 例子 | UI 呈現 |
|---|---|---|
| `NotConfigured` | 沒設 api_key | 引導到設定頁 |
| `Network` | 連不上 API | 可重試 + 建議切 provider |
| `RateLimit` | 429 | 顯示等待時間 + 建議切本地 |
| `AuthFailed` | 401 | 引導到設定頁改 key |
| `ModelError` | 模型回覆無效 | 顯示原文 + 可重試 |
| `LocalEngineDown` | Ollama 沒開 | 提示「Ollama 未啟動」+ 一鍵切雲端 |

## 6. 設定 UI

### 6.1 原則

設定 UI 是第一等公民,TOML 僅作為底層儲存。一般使用者不需手動編輯設定檔。

### 6.2 進入方式

- 右上角齒輪圖示
- 快捷鍵 `Ctrl+,`
- 首次啟動自動開啟 Onboarding Wizard

### 6.3 設定頁結構

左側 sidebar 分頁:
- AI Providers (重點頁)
- General (UI 語言、主題)
- Appearance (字型、字級、配色)
- Shell (預設 shell、啟動目錄、環境變數)
- Keybindings (可視化編輯,含衝突偵測)
- Privacy & Context (最近命令條數、是否送 cwd/env、敏感字脫敏規則)
- About

### 6.4 AI Providers 頁

- 列出已設定的 providers,顯示 health 狀態 (✓ Healthy / ✗ Down)
- 每列有 Test / Edit / Remove 按鈕
- 頂部有 Default Provider 下拉選單
- `+ Add Provider` 按鈕 → 彈出引導卡片 (選 provider type → 填表單)

**動態表單原則** (依 provider type 變化):
- **OpenAI / Anthropic / Gemini**:Display Name + API Key (masked) + Model (自動從 API 撈清單) + Test Connection
- **Ollama**:自動偵測本地服務,自動撈已下載模型填下拉,若未啟動顯示「Ollama 未啟動,點此查看安裝說明」
- **OpenAI-Compatible**:Display Name + Base URL + API Key + Model,Base URL 提供常見預設 quick-pick (LM Studio / vLLM / OpenRouter / DeepSeek)

### 6.5 Onboarding Wizard (首次啟動)

3 步驟:
1. 歡迎頁,簡介 AITerm 功能
2. 新增第一個 Provider (可 Skip for now)
3. 選執行模式 (預設:一律確認)

### 6.6 設定儲存實作

- **原子寫入**:write + rename,避免半寫損毀
- **Live reload**:存檔後立即套用,無需重啟
- **Import/Export**:支援匯出設定 (**不含** API Key) 供備份/換機使用

## 7. 命令安全分級 (command_guard)

### 7.1 分級定義

| 等級 | 行為 | 範例 |
|---|---|---|
| **Safe** | 模式 B 下可直接執行 | `ls`、`pwd`、`cat`、`git status`、`git log`、`echo`、`which`、`ps`、`df`、`type`、PowerShell `Get-*` |
| **NeedsConfirm** | 一律顯示預覽框 | `git commit`、`git push`、`npm install`、`pip install`、`mv`、`cp`、`mkdir`、`touch`、任何寫檔/改環境的命令 |
| **Dangerous** | 紅色警告 + 二次確認 | `rm -rf`、`del /s /q`、`format`、`dd`、`curl \| sh`、`sudo`、`chmod 777`、`reg delete`、`Remove-Item -Recurse`、`shutdown`、`kill -9` |
| **Blocked** | 不提供執行按鈕,僅顯示說明 | `rm -rf /`、fork bomb、`mkfs`、針對系統磁碟的 `format` |

### 7.2 判斷邏輯 (三層)

1. **命令解析層** — 用 `shell-words` crate tokenize,處理 `|`、`&&`、`;` 等分隔,對每個子命令分別評估。整體風險取最高等級。
2. **規則表層** — 內建 YAML 規則檔,格式範例:
   ```yaml
   - name: rm-recursive-force
     match:
       program: rm
       flags_any_of: ["-rf", "-fr", "--recursive --force"]
     level: dangerous
     reason: "遞迴強制刪除,無法復原"

   - name: rm-root
     match:
       program: rm
       args_contains: ["/", "/*", "~", "$HOME"]
       flags_any_of: ["-rf", "-fr"]
     level: blocked
     reason: "刪除系統根目錄"
   ```
3. **啟發式層** — 規則未命中但包含下列特徵者升級為 `NeedsConfirm`:管線進入 `sh`/`bash`/`pwsh`、含 `$()` 或反引號的命令替換、重導向到 `/dev/`、含展開後不可預期的環境變數

### 7.3 平台差異

- **Windows 規則集**:`reg delete`、`net user`、`bcdedit`、`fsutil`、PowerShell `Remove-Item -Force -Recurse`、`Stop-Computer`
- **Unix 規則集**:`sudo`/`su`、`/etc` 寫入、`chown`、`chmod -R`

規則檔位置:`resources/command_rules/{windows,unix}.yaml`,編譯時打包進 binary;使用者可於 Privacy & Context 頁載入自訂規則補丁。

### 7.4 預覽框顯示內容

每個預覽框顯示:
- 完整命令
- 風險等級標記 (🟢 Safe / 🟡 Confirm / 🔴 Dangerous)
- 觸發的規則名稱與原因 (例「遞迴強制刪除,無法復原」)
- AI 的自然語言解釋 (來自 AI 回傳的 `explanation` 欄位)

### 7.5 保守原則

- **模糊即升級**:無法確定時一律當 NeedsConfirm 處理
- **模式 C (全自動) 下 Dangerous 仍強制確認**,不可旁路
- **Blocked 不可覆寫**:使用者若真要執行只能自己手動打

## 8. 快捷鍵

| 快捷鍵 | 動作 |
|---|---|
| `Ctrl+T` | 新分頁 |
| `Ctrl+W` | 關閉分頁 |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | 下/上一個分頁 |
| `Ctrl+1`~`Ctrl+9` | 切換到第 N 個分頁 |
| `Ctrl+I` | 開啟/關閉 AI 側邊面板 |
| `Ctrl+Shift+P` | 快速切換 AI provider |
| `Ctrl+,` | 開啟設定 |
| `Ctrl+Shift+M` | 切換執行模式 (A/B/C) |
| `/ai <文字>` | 行內 AI 前綴觸發 |
| `Enter` (預覽框) | 確認執行 AI 命令 |
| `Tab` (預覽框) | 編輯 AI 命令後再執行 |
| `Esc` (預覽框) | 取消 AI 命令 |
| `Ctrl+C` (終端機) | 送 SIGINT 到子行程 |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | 複製 / 貼上 |

全部快捷鍵於設定頁 Keybindings 頁面可視化修改,支援衝突偵測。

## 9. 隱私與資料處理

- **API Key** 只存 OS keychain,永不寫入 TOML
- **最近命令上下文**:預設 5 條,條數可於 Privacy & Context 頁調整
- **敏感字脫敏**:包含 `password`、`token`、`key`、`secret`、`credential` 字樣的輸出在送往 AI 前以 `[REDACTED]` 取代
- **環境變數過濾**:含上述字樣的環境變數不納入 snapshot
- **匯出設定** 不包含 API Key
- 所有 AI 請求皆由使用者觸發,無背景靜默請求

## 10. 測試策略

### 10.1 單元測試 (Rust `cargo test`)

- `command_guard`:table-driven tests,每條規則至少一個 positive + 一個 negative case,**規則 100% 覆蓋**
- `ai_router`:mock provider 驗證 request 組裝、串流中斷處理、錯誤分類
- `context_builder`:敏感字過濾、最近命令上限、snapshot diff 正確性
- `config_store` / `secret_store`:讀寫 round-trip、原子寫入、壞檔復原

### 10.2 整合測試 (Rust `tests/`)

- `pty_manager`:實際開子 shell、送命令、讀輸出 (Windows: cmd/pwsh;Unix: bash)
- 各 `AiProvider`:使用 `wiremock` 模擬 API 回應的 contract test,驗證 SSE 解析與錯誤碼對應

### 10.3 E2E 手動驗收清單 (v1 必過)

1. 新裝 → onboarding → 加一個雲端 provider → `/ai list current directory` 成功
2. 加一個 Ollama provider → 快速切換 → 能用本地模型產命令
3. 模式 A 下 `rm -rf /` 被 block;模式 B 下 `ls` 自動執行、`rm test.txt` 需確認
4. AI 面板多輪對話:第二輪能參照第一輪上下文
5. 殺掉 Ollama 後使用 local provider,得到友善錯誤 + 一鍵切雲端
6. 設定頁修改 default provider → 無需重啟即生效
7. 匯出設定 → 換機匯入 → API Key 不會被匯出

## 11. 里程碑

| M | 名稱 | 範圍 | 成果 |
|---|---|---|---|
| **M0** | 骨架 | Tauri + React 初始化、xterm.js + portable-pty、單分頁能開 shell | 功能等同 cmd 的空殼 |
| **M1** | AI 行內觸發 | `ai_router` + `OpenAiClient` + `/ai` 前綴 + 一律確認預覽框 | 能用一家雲端模型產命令 |
| **M2** | 多 Provider + 設定 UI | `AnthropicClient`、`OllamaClient`、`OpenAiCompatibleClient`、設定頁、keychain、onboarding wizard | 使用者可在 UI 自由加/切後端 |
| **M3** | 分級執行 + 安全 | `command_guard` + 模式 B + 規則表 + 風險標記顯示 | 安全分級可用,規則測試齊全 |
| **M4** | AI 面板 + 多輪對話 | `Ctrl+I` 面板、串流顯示、多輪對話、`<cmd>` 解析 | 多輪 AI 對話 + 一鍵執行建議命令 |
| **M5** | 多分頁 + Context Builder | 多分頁、最近命令上下文、敏感字過濾、`GeminiClient` | 完整 B 版 MVP |
| **M6** | 打磨 | 主題、字型、快捷鍵編輯、錯誤訊息、文件、Windows MSI 安裝包 | 可公開 release 的 v1.0 |

### 11.1 非目標 (v1 不做)

- Agent 模式 (連續多命令自動執行)
- 專案感知 (偵測 git/package.json/requirements.txt)
- 外掛系統
- 遠端 SSH session
- 協作 session
- 多主題商店
- 行動版

這些留給 v2+。

## 12. 程式碼結構

```
AITerm/
├── src-tauri/                  # Rust backend
│   ├── src/
│   │   ├── main.rs
│   │   ├── pty/
│   │   ├── ai/
│   │   │   ├── mod.rs          # trait AiProvider
│   │   │   ├── openai.rs
│   │   │   ├── anthropic.rs
│   │   │   ├── gemini.rs
│   │   │   ├── ollama.rs
│   │   │   └── compatible.rs
│   │   ├── context/
│   │   ├── guard/
│   │   │   ├── mod.rs
│   │   │   └── rules.rs
│   │   ├── config/
│   │   └── secret/
│   ├── resources/
│   │   └── command_rules/
│   │       ├── windows.yaml
│   │       └── unix.yaml
│   └── tests/
├── src/                        # React frontend
│   ├── components/
│   │   ├── Terminal/
│   │   ├── AiPanel/
│   │   ├── CommandPreview/
│   │   └── Settings/
│   ├── hooks/
│   ├── ipc/                    # Tauri invoke 封裝
│   └── i18n/                   # 繁中 / 英文
├── docs/
│   └── superpowers/specs/
└── README.md
```

## 13. 預設值一覽

| 項目 | 預設 |
|---|---|
| UI 語言 | 繁體中文 |
| Windows 預設 shell | PowerShell 7 (`pwsh`) |
| 執行模式 | A (一律確認) |
| 最近命令上下文條數 | 5 |
| 前綴 | `/ai` |
| 設定檔位置 (Windows) | `%APPDATA%/AITerm/config.toml` |
| API Key 儲存 | Windows Credential Manager |
| 敏感字脫敏 | 啟用,關鍵字:password, token, key, secret, credential |
