# AITerm M1 設計規格 — AI 行內觸發 (`/ai`)

- **日期**:2026-04-10
- **狀態**:Draft (待 user review)
- **作者**:James Chu (with Claude)
- **前置依賴**:M0(Tauri + xterm.js + portable-pty 骨架,已完成)
- **上層規格**:`2026-04-10-aiterm-design.md`(以下簡稱「主規格」)

本文件是主規格 §11 里程碑表中 **M1** 的細化設計。主規格定義了 AITerm 的整體架構;本文件定義「第一條 AI 主線能跑通」的最小可驗收範圍。

---

## 1. 目標與非目標

### 1.1 目標

交付一條**最小可驗收**的 AI 行內觸發主線:

> 使用者在 terminal 打 `/ai <自然語言>` → 系統攔截 → 呼叫 OpenAI → 解析 AI 回傳的結構化 JSON → 在 CommandPreview 框裡顯示命令與說明 → 使用者按 Enter 執行或 Esc 取消 → 執行走既有 PTY 路徑。

M1 的單一成功標準:**一個剛 clone 這個 repo 的使用者,在設好 `OPENAI_API_KEY` 後,能用自然語言讓 AITerm 產出一條 pwsh 命令並實際執行。**

### 1.2 非目標(M1 明確不做)

| 功能 | 留給 |
|---|---|
| 多 AI provider(Anthropic、Gemini、Ollama、OpenAI-Compatible) | M2 |
| 設定 UI / `config_store` / `secret_store` / onboarding wizard | M2 |
| 命令安全分級 `command_guard`、風險顏色、執行模式 B/C | M3 |
| AI 側邊面板 `Ctrl+I`、多輪對話、串流顯示 | M4 |
| 多分頁、最近命令歷史、敏感字脫敏、`context_builder` 模組化 | M5 |
| 主題、字型、Keybindings 編輯、MSI 打包 | M6 |
| 預覽框 Tab 編輯、可取消的 invoke、loading 動畫 | M6 之後 |
| 使用 AI 回的 `risk_level` 欄位 | M3(M1 只解析不使用)|
| 命令歷史上下文 | M5 |
| 前端 state 管理函式庫、CSS 框架 | 永遠不加,保持簡樸 |

---

## 2. 關鍵決策摘要

以下每一項都是在 brainstorming 階段被逐一確認的決策。後續實作不得重新辯論。

| # | 決策 | 原因 |
|---|---|---|
| D1 | **第一個 provider:OpenAI** | SSE 格式是業界事實標準,M2 的 `OpenAiCompatibleClient` 可直接重用 |
| D2 | **API Key 來源:環境變數 `OPENAI_API_KEY`** | 最小實作、零 UI、M2 接 keychain 時這條 path 降級為 fallback 即可 |
| D3 | **AI 回應:buffered 收齊才顯示**(M1 不做串流 UX) | `/ai` 是「一次查詢、一條命令」,串流 UX 價值留給 M4 多輪對話 |
| D4 | **Context 範圍:OS + shell + cwd**,最近命令歷史完全不送 | 三個欄位是命令正確性的**必要**條件;歷史的價值在多輪,M1 用不到 |
| D5 | **錯誤呈現:全部寫進 terminal 的紅字**,不另開 UI | 符合 terminal 使用者心智模型,零新元件 |
| D6 | **驗收緊窄:Enter 執行 / Esc 取消,無 Tab 編輯** | Tab 編輯的價值要在「AI 產錯命令」時才顯現,那是 M3 的命題 |
| D7 | **M1 完全不碰 `command_guard`** | 預設模式 A「一律確認」不依賴分級;分級留給 M3 |
| D8 | **後端模組結構:完整按主規格 §12 切模組**(方案 1) | M2 接四個新 provider 時檔案位置已挖好 |
| D9 | **cwd 追蹤:e2α — 後端追蹤使用者寫進 PTY 的 `cd` 命令** | 三種 shell 行為一致、不污染 shell profile、parse 失敗時優雅降級 |
| D10 | **`risk_level` 欄位必須 parse 成功但 M1 不使用** | 讓 AI 從 M1 就習慣輸出,M3 接 `command_guard` 時零前向相容成本 |
| D11 | **OpenAI 使用 `response_format: json_object`** | 把 JSON 合法性從 prompt 軟性規則升級為 API 硬性保證 |
| D12 | **M1 寫死 `gpt-4o-mini` 模型** | 查詢簡單、便宜、快;M2 設定 UI 上線後變成預設值 |
| D13 | **前端新加 `vitest` devDependency** | M0 沒有前端測試框架;M1 需要跑兩個 unit test |

---

## 3. 架構與模組佈局

### 3.1 後端 Rust 新增結構

```
src-tauri/src/
├── ai/                         ← 新增
│   ├── mod.rs                  trait AiProvider, GenerateRequest, GenerateChunk,
│   │                           AiError, EnvSnapshot, QueryMode, ChatMessage,
│   │                           AiSingleCommand, RiskLevel
│   ├── openai.rs               OpenAiClient (reqwest + SSE 解析)
│   ├── router.rs               AiRouter 結構體:持有 Result<Arc<dyn AiProvider>, AiError>,
│   │                           提供 query() 方法
│   └── context.rs              pub fn snapshot(pty_manager, session_id) -> EnvSnapshot
├── commands/                   ← 新增(或放在 lib.rs 現有集中點,實作階段決定)
│   └── ai.rs                   Tauri command `ai_query`
├── pty/                        (既有,session.rs 需改動以加 cwd 追蹤)
├── lib.rs                      注入 AiRouter 到 Tauri state,註冊 ai_query 命令
└── main.rs                     不動
```

**模組依賴方向(強制無循環):**

```
commands/ai.rs  →  ai::router  →  ai::mod (trait + types)  ←  ai::openai
                       ↓                                      ↑
                  ai::context  ──→  pty::PtyManager ←─────────┘
                                    (只有讀 cwd 的方向)
```

`ai/mod.rs` **只**宣告 trait 與型別,不依賴任何 provider 實作 — 這是 trait 檔案必須獨立的關鍵理由,保證 M2 新增 provider 時不會震盪既有檔案。

### 3.2 後端新依賴 (`Cargo.toml`)

| crate | features | 用途 |
|---|---|---|
| `reqwest` | `["json", "stream", "rustls-tls"]` | HTTP + SSE |
| `async-trait` | — | `trait AiProvider` 的 async 方法 |
| `serde_json` | — | JSON 解析(目前是透過 tauri 間接取得,M1 明確宣告) |
| `futures-util` | — | `StreamExt` 讀 SSE chunks |
| `thiserror` | — | `AiError` 的 `Error` derive |

`tokio`、`parking_lot`、`portable-pty`、`uuid` 在 M0 已裝,不變動版本。

### 3.3 前端 TypeScript 新增結構

```
src/
├── components/
│   ├── TerminalView/           既有,加「/ai 前綴攔截」邏輯
│   │   ├── TerminalView.tsx
│   │   └── parseAiPrefix.ts    ← 新增純函式,便於單元測試
│   └── CommandPreview/         ← 新增
│       └── CommandPreview.tsx  純呈現元件,props: { command, explanation, onConfirm, onCancel }
├── ipc/
│   ├── pty.ts                  既有
│   └── ai.ts                   ← 新增:invokeAiQuery + formatAiError + AiError 型別
└── App.tsx                     加 preview 開關的 useState,把 CommandPreview 疊在 TerminalView 上方
```

**前端刻意不做的事:**
- 不新增 state 管理函式庫(Redux / Zustand / Jotai 皆不加)
- 不新增 CSS 框架(維持 M0 的純 CSS)
- 不新增 router(單頁)
- 不做 i18n 基礎建設(錯誤訊息先寫死英文,UI 文字繁中)

### 3.4 AI Provider trait(M1 版本,定義在 `ai/mod.rs`)

```rust
#[derive(Clone, Debug, Serialize)]
pub struct EnvSnapshot {
    pub os: String,        // "windows" | "macos" | "linux"
    pub shell: String,     // "pwsh" | "powershell" | "cmd" | "bash" | "zsh"
    pub cwd: PathBuf,
}

#[derive(Debug, Clone)]
pub enum QueryMode {
    SingleCommand,  // /ai
    Chat,           // 保留給 M4,M1 不實作這條路
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,      // "user" | "assistant" | "system"
    pub content: String,
}

pub struct GenerateRequest {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub context: EnvSnapshot,
    pub mode: QueryMode,
    pub max_tokens: Option<u32>,
}

pub struct GenerateChunk {
    pub delta: String,
    pub done: bool,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError>;
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct AiSingleCommand {
    pub explanation: String,
    pub command: String,
    pub risk_level: RiskLevel,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Safe,
    NeedsConfirm,
    Dangerous,
}
```

**設計約束:**
- `generate()` 即使在 M1 的 buffered 情境下也**保留 mpsc::Sender 的串流介面**。M1 的 `commands/ai.rs` 會建立 channel、spawn provider、在 rx 上累積 chunks 成完整 String,再送去 parse。這樣 M4 上線串流 UX 時 trait 不用動。
- M1 的 `GenerateRequest.mode` 一律是 `SingleCommand`。`Chat` 變體保留但**不做實作**(可以 `todo!()` 或 `unimplemented!()`,不能被 M1 的任何路徑觸發)。
- `AiError::LocalEngineDown`(主規格 §5.4)M1 不定義 — 沒有本地 provider 就沒有這個錯誤。M2 加 Ollama 時補進 enum。

---

## 4. 資料流

### 4.1 Happy path(`/ai` 完整流程)

```
[1] 使用者在 TerminalView 打字:  /ai find files larger than 100MB
[2] 按下 Enter
     ↓
[3] TerminalView 偵測當前行首是否為 "/ai "
     ├── 不是 → 走既有 invoke("pty_write", ...) 路徑
     └── 是  → 攔截,不送 PTY,繼續 [4]
     ↓
[4] 前端:
     - parseAiPrefix(line) 抽出 "/ai " 後的 query 字串
     - 清掉當前 xterm 行:term.write("\r\x1b[2K")
     - 印一行提示: term.write("→ asking AI...\r\n")
     - setPreviewState({ loading: true })
     ↓
[5] invoke<AiCommandReady>("ai_query", { query, sessionId })
     ↓
[6] 後端 commands/ai.rs::ai_query(query, session_id, state):
       let snapshot = ai::context::snapshot(&state.pty_manager, &session_id);
       let req = GenerateRequest {
           system_prompt: build_single_command_prompt(&snapshot),
           messages: vec![ChatMessage { role: "user".into(), content: query }],
           context: snapshot,
           mode: QueryMode::SingleCommand,
           max_tokens: Some(512),
       };
       let provider = state.ai_router.require_provider()?; // NotConfigured 在這裡噴
       let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
       let handle = tokio::spawn(async move {
           provider.generate(req, tx).await
       });
       let mut buf = String::new();
       while let Some(chunk) = rx.recv().await {
           buf.push_str(&chunk.delta);
           if chunk.done { break; }
       }
       handle.await??;  // 展開 join error + generate error
     ↓
[7] 後端 parse JSON:
       serde_json::from_str::<AiSingleCommand>(&buf)
           .map_err(|e| AiError::ModelError {
               reason: e.to_string(),
               raw: buf.chars().take(200).collect(),
           })?
     ↓
[8] Ok(AiCommandReady { explanation, command })
    注意:risk_level 已被 serde 驗證存在且合法,但不放進 AiCommandReady 回傳前端
     ↓
[9] 前端 invoke Promise resolve:
      - term.write("\r\x1b[2K")  清掉 "→ asking AI..." 那行
      - setPreviewState({ loading: false, command, explanation, visible: true })
     ↓
[10] CommandPreview 顯示,listen keydown:
       Enter → invoke("pty_write", { sessionId, data: command + "\r" })
               setPreviewState({ visible: false })
       Esc   → setPreviewState({ visible: false })
```

### 4.2 幾個關鍵設計決策

**a. 用 invoke Promise 而非 Tauri event 回 AI 結果** — M1 是 buffered,單 request 單 response,Promise 自帶 correlation,不需發明 request id。Tauri event 留給 M4 多 chunk 串流。

**b. PTY 寫入時只送 `command + "\r"`** — 模擬使用者自己打這條命令的效果,shell history(`Get-History` / `history`)會正常收錄,使用者可以 ↑ 叫回來重跑或改。

**c. 清理 `/ai` 那行** — 步驟 [4] 和 [9] 各寫一次 `\r\x1b[2K` 把對應行清掉,避免 AI 往返過程在 terminal 留下視覺殘留。這是 M1 唯一跟 xterm.js 控制碼有關的地方。

**d. `session_id` 必帶** — 即使 M1 單分頁,`ai::context::snapshot()` 也要依 session_id 從 `PtyManager` 讀 cwd(見 §5),不能用程式自己的 `std::env::current_dir()`。

### 4.3 CommandPreview UI(M1 版本)

CommandPreview 是絕對定位、浮在 TerminalView 上方的一個卡片元件。**不改變 terminal 的 flex layout**。

```
┌──────────────────────────────────────────┐
│ Command:                                 │
│   Get-ChildItem -Recurse |               │
│     Where-Object { $_.Length -gt 100MB } │
│                                          │
│ Explanation:                             │
│   列出遞迴子目錄中大於 100MB 的檔案      │
│                                          │
│         [Enter] Execute   [Esc] Cancel   │
└──────────────────────────────────────────┘
```

**視覺規則:**
- 背景半透明深色疊加 terminal
- 等寬字型顯示 command,系統預設字型顯示 explanation
- command 過長時自動換行但不截斷
- Enter 和 Esc 的指示是純文字提示,不是可點擊按鈕 — M1 沒有滑鼠互動
- 預覽框顯示時 terminal 不 blur、不禁用輸入,但所有鍵盤事件優先給 preview(keydown listener on document + capture)

**Loading 與並發行為:**
- Loading 狀態**不使用** CommandPreview — 唯一的 loading 指示是 terminal 裡的 `→ asking AI...` 那行字。
- `loading: true` 期間,前端**忽略**後續的 `/ai` 前綴偵測(使用者打另一個 `/ai` 仍然會被攔截不送 PTY,但不會觸發新的 invoke,僅在 terminal 印一行紅字 `aiterm: already waiting for AI response`)。這避免雙送,M1 不做 cancel-and-retry。
- Loading 中使用者按 Esc:**不中止** invoke(M1 沒有可取消的 invoke,見 D6 及 §7.5)。Esc 只在 `visible: true` 的預覽框狀態下起作用。

---

## 5. cwd 追蹤(e2α)

### 5.1 原理

在 `PtySession::write` 裡掛一個 hook:維護一個 **line buffer**,每次寫入的 bytes 裡含 `\r`(使用者按 Enter)時 flush 那一行並嘗試 parse 是否為 `cd` 類命令。若是,更新 session 內部的 `cwd: Arc<Mutex<PathBuf>>`。

**初始 cwd**:session 建立時用 `portable-pty::CommandBuilder::cwd()` 指定的值,同時把該值寫入 `PtySession.cwd`。M0 當前沒設 cwd,M1 會改成明確設定(例如使用者 `std::env::current_dir()` 或 `$HOME`)。

**API 增加**:
```rust
impl PtyManager {
    pub fn get_cwd(&self, session_id: &str) -> Option<PathBuf>;
}
```

`ai::context::snapshot(pty_manager, session_id)` 呼叫這個取 cwd,取不到時回 fallback 到 `std::env::current_dir()`(僅 snapshot 邊界的防禦,parser 本身不會失敗導致 snapshot 失敗)。

### 5.2 Parser 覆蓋範圍(M1 必過)

| Shell | 命令模式 |
|---|---|
| **pwsh / powershell** | `cd <path>`, `Set-Location <path>`, `sl <path>`, `pushd <path>` |
| **cmd** | `cd <path>`, `cd /d <path>`, `chdir <path>` |
| **bash / zsh** | `cd <path>`, `cd`(無參數 → `$HOME`), `cd -`(上一個目錄) |

**共通處理:**
- 相對路徑:以當前 `cwd` 為基底 join,對 `.` 和 `..` 做路徑正規化(不走 `canonicalize` 以避免 I/O 與 symlink 解析差異)
- `~` 展開:Unix 家用目錄
- 引號:`cd "path with spaces"` 要正確解引號(用 `shell-words` crate 或自己寫一個小 tokenizer,M1 採後者以避免新增依賴)
- 空白:trim 前後空白

### 5.3 Parser 保守策略(失敗時 cwd 不變)

**不嘗試處理以下情況**,遇到時 cwd 狀態保持不變:

- 多子命令串接:`cd a && cd b`、`cd a; cd b`、`cd a | ...`
- 子 shell 內的 cd:`(cd a && ls)`、`$(cd a)` 不影響父 shell cwd,parser 直接忽略整行
- 變數展開:`cd $FOO`、`cd %USERPROFILE%` 若無法 literal 解析 → 不動 cwd
- Script 內部的 cd、function 內的 cd、source 的 file 內的 cd — parser 看不到
- `pushd`/`popd` 的完整 stack 語意(parser 可以追蹤 pushd 的 top,但不維護 stack — `popd` 完全不處理,cwd 不變)

**原則:parse 不出來就 cwd 不變,使用者拿到稍微過期的 cwd 絕對好過拿到隨機錯的 cwd。**

### 5.4 為什麼不做 OSC 7(e2β)

- cmd.exe 沒有乾淨的 per-prompt hook,會被迫退回 e2α 等於維護兩套機制
- 要侵入使用者的 `$PROFILE` / `.bashrc`,或犧牲他們的 profile 載入,兩個選項都糟
- OSC parser 要跟 PTY reader loop 耦合,測試成本高
- 留給 M5 的 `context_builder` 正式升級時處理,那時 e2α 會降級為 fallback 不會被丟掉

---

## 6. AI 回應格式與 system prompt

### 6.1 JSON schema(AI 必須回的形狀)

```json
{
  "explanation": "string, 繁體中文, 一句話說明這個命令做什麼",
  "command":     "string, 一條可直接執行的 shell 命令, 不含提示字元",
  "risk_level":  "safe" | "needs_confirm" | "dangerous"
}
```

### 6.2 System prompt(M1 固定內容)

```
You are an AI command generator for a cross-platform terminal application.
Your only job is to translate the user's natural-language request into ONE
executable shell command for their current environment.

Environment:
  OS: {os}              (e.g. windows, macos, linux)
  Shell: {shell}        (e.g. pwsh, powershell, cmd, bash, zsh)
  Cwd: {cwd}            (may be slightly stale; prefer relative paths or
                         shell variables over hardcoded absolute paths)

Rules:
1. Output ONLY a JSON object, no prose, no markdown fences, no extra keys.
2. Schema:
   {
     "explanation": "一句話說明這個命令做什麼 (use Traditional Chinese)",
     "command":     "a single shell command, no prompt prefix, no line breaks",
     "risk_level":  one of "safe", "needs_confirm", "dangerous"
   }
3. The command must be syntactically valid for {shell}. Do not mix shells.
4. If the request cannot be satisfied with one command, pick the most useful
   single command and explain the limitation in `explanation`.
5. Never produce destructive operations against system roots. If the user
   explicitly asks for one, set risk_level="dangerous".

User request: {query}
```

Prompt 的組裝在 `commands/ai.rs` 的一個內部函式 `build_single_command_prompt(snapshot: &EnvSnapshot) -> String`,純字串 format,可單元測試。

### 6.3 關鍵設計

- **explanation 強制繁中** — 沿用主規格 §13 的 UI 預設語言
- **command 不翻譯** — 必須是原生英文 shell 語法才能執行
- **「no markdown fences」是硬規則** — M1 的後端 parser 不做 fence 清理,違反就 `ModelError`
- **`response_format: { type: "json_object" }`** — OpenAI API 硬性保證 JSON 合法性
- **不使用 function calling / tool use** — 抽象成本高、`response_format` 已經夠用
- **model 寫死 `gpt-4o-mini`** — 簡單任務、便宜、快。M2 設定 UI 上線後變成 `providers[].model` 的預設值
- **context 只放 system message** — 不另開 user message 塞環境資訊,節省 token

### 6.4 M1 的 `risk_level` 處理(重要)

- **必須 parse 成功**:缺欄位或值不合法 → `AiError::ModelError`
- **M1 完全不使用欄位的值** — 預覽框永遠長一樣、永遠要求 Enter 確認
- **為什麼還要解**:讓 AI 從 M1 就習慣輸出此欄位,M3 接 `command_guard` 時 prompt 與 trait 都不用改

---

## 7. 錯誤處理

### 7.1 `AiError` enum

```rust
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiError {
    #[error("OPENAI_API_KEY environment variable is not set")]
    NotConfigured,

    #[error("network error: {message}")]
    Network { message: String },

    #[error("authentication failed (check your API key)")]
    AuthFailed,

    #[error("rate limit exceeded")]
    RateLimit { retry_after: Option<String> },

    #[error("AI returned invalid response: {reason}")]
    ModelError { reason: String, raw: String },
}
```

`#[serde(tag = "kind")]` 讓前端收到的 JSON 長這樣:
- `{ "kind": "not_configured" }`
- `{ "kind": "network", "message": "..." }`
- `{ "kind": "auth_failed" }`
- `{ "kind": "rate_limit", "retry_after": "20s" | null }`
- `{ "kind": "model_error", "reason": "...", "raw": "..." }`

前端以 `switch(err.kind)` 做 type-safe 處理,不靠字串比對。

### 7.2 錯誤來源映射

**OpenAiClient::generate():**
| 來源 | 對應 |
|---|---|
| `reqwest::Error`(transport) | `Network { message: e.to_string() }` |
| HTTP 401 | `AuthFailed` |
| HTTP 429 | `RateLimit { retry_after: parse_header("retry-after") }` |
| HTTP 其他 4xx/5xx | `Network { message: format!("http {status}: {body}") }` |
| SSE stream 結束但沒 `[DONE]` | `ModelError { reason: "stream ended early", raw: buf }` |

**`commands/ai.rs::ai_query`:**
| 來源 | 對應 |
|---|---|
| `env::var("OPENAI_API_KEY")` 缺 | `NotConfigured`(啟動時記錄,`/ai` 觸發時才回報)|
| `provider.generate()` 的 `Err(AiError)` | 直接向上 |
| `serde_json::from_str::<AiSingleCommand>` 失敗 | `ModelError { reason: serde_err.to_string(), raw: buf.chars().take(200).collect() }` |

### 7.3 `NotConfigured` 的延後回報策略

`lib.rs` 的 `.setup()` 嘗試讀 `OPENAI_API_KEY`;失敗時**不讓 setup 失敗**(那會讓整個應用不啟動),而是讓 `AiRouter` 持有 `Result<Arc<dyn AiProvider>, AiError::NotConfigured>`。每次 `ai_query` 被呼叫才回這個錯。使用者可以先正常使用 terminal,只有在打 `/ai` 時才會被告知沒設 key。

### 7.4 前端錯誤顯示

`src/ipc/ai.ts` 匯出:

```ts
export type AiError =
  | { kind: "not_configured" }
  | { kind: "network"; message: string }
  | { kind: "auth_failed" }
  | { kind: "rate_limit"; retry_after: string | null }
  | { kind: "model_error"; reason: string; raw: string };

export function formatAiError(e: AiError): string {
  switch (e.kind) {
    case "not_configured":
      return "aiterm: OPENAI_API_KEY not set. Set the env var and restart AITerm.";
    case "network":
      return `aiterm: network error — ${e.message}`;
    case "auth_failed":
      return "aiterm: authentication failed. Check your OPENAI_API_KEY.";
    case "rate_limit":
      return e.retry_after
        ? `aiterm: rate limit exceeded (retry after ${e.retry_after})`
        : "aiterm: rate limit exceeded, try again later";
    case "model_error":
      return `aiterm: AI returned invalid response (${e.reason})\n        raw: ${e.raw}`;
  }
}
```

**寫入 terminal:**
```ts
// in TerminalView.tsx
const writeError = (msg: string) => {
  const RED = "\x1b[31m";
  const RESET = "\x1b[0m";
  term.write(`\r\n${RED}${msg}${RESET}\r\n`);
};
```

`invokeAiQuery(...).catch((e) => writeError(formatAiError(e)))`。錯誤寫進 xterm 後,shell 的下一個 prompt 會自然重畫,使用者繼續操作 — 無需額外清理 UI 狀態。

**錯誤時 `→ asking AI...` 那行不清除** — 刻意保留當作視覺上的「請求上下文」,讓紅字錯誤訊息出現在「asking」那行下方,使用者一眼看得出「這個錯誤是剛才那次 `/ai` 引發的」。§4.1 步驟 [9] 清掉 asking 行只在 happy path 執行。`catch` 只做 `setPreviewState({ loading: false })`(允許下一次 `/ai`)+ 寫紅字,不動 asking 行。

### 7.5 M1 刻意不做的錯誤處理

- **不做 retry 按鈕** — 沒有「待處理的錯誤狀態」,錯了重打 `/ai`
- **不做錯誤 telemetry / 日誌檔** — 符合主規格 §9 隱私原則
- **不做「部分解析」** — JSON 少一欄位整個回應就是 `ModelError`,不嘗試搶救
- **不做 timeout 設定** — reqwest 預設(30 秒)即可,M1 不暴露設定

---

## 8. 測試策略

### 8.1 Rust 單元測試(`cargo test`)

| 測試對象 | 位置 | 覆蓋內容 |
|---|---|---|
| `ai/openai.rs` SSE 解析 | `#[cfg(test)] mod tests` | 多 chunk 拼接、`[DONE]` 標記、有 usage 的最後 chunk、中途 stream 截斷 |
| `ai/openai.rs` HTTP 錯誤映射 | 同檔 | 401 → AuthFailed、429 含 retry-after → RateLimit、500 → Network |
| `ai/openai.rs` JSON 解析 | 同檔 | valid schema、缺 command 欄位、risk_level 不合法、AI 回 markdown fence(應噴 ModelError)|
| `ai/context.rs` snapshot | 同檔 | mock PtyManager,驗證 EnvSnapshot 欄位 |
| `pty/session.rs` cd parser | 同檔 | **table-driven**:每種 shell 的每個 cd 變體至少 1 positive + 1 negative。覆蓋:相對路徑 resolve、`..` 正規化、引號(含空白路徑)、`~` 展開(Unix)、bash `cd` 無參數 → `$HOME`、bash `cd -` → 上一個目錄、parse 失敗保持 cwd 不變 |
| `pty/session.rs` line buffer | 同檔 | 部分寫入不 flush、含 `\r` flush、多行一次寫入逐行處理 |

**約束:**這層完全不碰網路、不 spawn shell。`openai.rs` 的 HTTP/SSE 測試用 `wiremock`(lifecycle 可控,歸類單元)。

### 8.2 Rust 整合測試(`src-tauri/tests/`)

| 測試檔 | 測什麼 |
|---|---|
| `tests/openai_client.rs` | 用 wiremock 起 fake OpenAI server,跑完整 `OpenAiClient::generate()`:`response_format`、SSE 串流、Authorization header、錯誤碼 contract |
| `tests/pty_cwd_tracking.rs` | 實際 spawn 子 shell(Windows: pwsh;fallback: cmd),送 `cd` 指令,呼叫 `PtyManager::get_cwd` 驗證跟真 shell 的 cwd 一致。**這是 e2α 的真金火煉** |
| `tests/ai_query_command.rs` | 用 mock `AiProvider` 驗證 `commands/ai.rs` 的組裝邏輯 + 錯誤向上傳遞 + EnvSnapshot 被正確帶入 |

**刻意不做:**
- 不打真實 OpenAI(費用 + CI 不穩)
- 不起完整 Tauri app 做 E2E(留 M6)

### 8.3 前端測試(TypeScript,新增 `vitest`)

| 測試對象 | 位置 |
|---|---|
| `parseAiPrefix` 純函式 | `src/components/TerminalView/parseAiPrefix.test.ts` |
| `formatAiError` 純函式 | `src/ipc/ai.test.ts` |

**parseAiPrefix 必測邊界:**
- `/ai` 後面無空格或空字串 → null
- `/ai ` 正常 → 抽出 query
- 前面有空白 → null(要求行首)
- 大寫 `/AI` → null(M1 只認小寫)
- 多空白 `/ai   hello` → query = "hello"

**刻意不做的前端測試:**
- CommandPreview 元件快照測試(純呈現,人工看一次即可)
- xterm.js 整合測試(信任 third-party)
- E2E framework(Playwright / Cypress 留 M6)

執行命令:`npm test`(新增,對應 vitest)、`cd src-tauri && cargo test`、`npx tsc --noEmit`。

### 8.4 M0 回歸保護

M1 會動 `pty/session.rs` 加 cwd 追蹤,也會動 `lib.rs` 注入 `AiRouter`。

**M1 完成的門檻:**
```
M0 所有 cargo test 通過
+ M1 新增的所有測試通過
+ §9 的 4 條手動驗收全過
```

動 cwd 追蹤如果弄壞 M0 的 PTY 既有測試,要**修 `pty_session.rs`** 讓兩邊都通過,**不是**改 M0 測試來迎合 M1。

---

## 9. 驗收條件(手動 E2E)

M1 完成必須通過以下 4 條,任一條失敗即 M1 未完成。

| # | 步驟 | 預期 |
|---|---|---|
| **1** | 設好 `OPENAI_API_KEY`,啟動 AITerm,`/ai list files` | 預覽框顯示合理的 `Get-ChildItem`/`dir` 命令 → Enter → 命令在 terminal 跑出結果 → shell history 能用 ↑ 叫回 |
| **2** | `/ai show disk usage` → 出預覽框 → Esc | 預覽框消失、terminal 空白、shell 繼續正常接收輸入(證明取消不會卡 PTY)|
| **3** | **未設** `OPENAI_API_KEY`,啟動 AITerm → `/ai anything` | terminal 顯示紅字 `aiterm: OPENAI_API_KEY not set. Set the env var and restart AITerm.`(證明延後回報策略有效,應用沒在啟動時就掛)|
| **4** | 在 pwsh 裡 `cd C:\Windows\System32` → `/ai list dll files` | 預覽框裡的命令使用 `C:\Windows\System32` 的上下文(證明 e2α 的 cwd 追蹤 → AI snapshot 這條線接通)|

**平台**:Windows 11 + pwsh。macOS / Linux 的驗收推遲到 M5/M6。

---

## 10. M0 影響與前向相容

### 10.1 M1 對 M0 的修改點

| 檔案 | 修改類型 | 風險 |
|---|---|---|
| `src-tauri/src/pty/session.rs` | **加** `cwd` 欄位、line buffer、cd parser hook | 中 — M0 既有測試必須繼續通過 |
| `src-tauri/src/pty/manager.rs` | **加** `get_cwd(session_id)` API | 低 — 純新增 |
| `src-tauri/src/lib.rs` | **加** `AiRouter` 到 state、註冊 `ai_query` command | 低 — 純新增 |
| `src-tauri/Cargo.toml` | **加** reqwest / async-trait / futures-util / thiserror | 低 |
| `src/components/TerminalView/TerminalView.tsx` | **加** `/ai` 前綴攔截邏輯 | 中 — 要不破壞既有 data 路徑 |
| `src/App.tsx` | **加** CommandPreview 疊加 | 低 |
| `package.json` | **加** `vitest` devDependency | 低 |

### 10.2 M2 以後會做的重構(M1 不做但要有意識地設計)

- M2 接 Anthropic/Gemini/Ollama/Compatible 時,`ai/router.rs` 會從「持有單一 provider」升級為「從 id 挑 provider + health check」。trait 不變。
- M2 接 `config_store` 時,`OPENAI_API_KEY` 環境變數讀取降級為 fallback(優先讀 keychain)。
- M3 接 `command_guard` 時,`AiSingleCommand.risk_level` 從「解析但不用」升級為「通過 guard 分類後決定預覽行為」。
- M4 接 AI 面板時,`GenerateChunk` 的串流介面從「M1 累積後批次回」升級為「逐 chunk emit Tauri event」。trait 不變。
- M5 接 `context_builder` 時,`ai/context.rs::snapshot()` 從三行升級為完整模組,e2α 降級為 cwd 追蹤的 fallback,主要 path 可能走 OSC 7。
