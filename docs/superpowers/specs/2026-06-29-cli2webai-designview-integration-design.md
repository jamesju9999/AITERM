# CLI2WebAI × DesignView 整合設計

**日期**：2026-06-29  
**範圍**：DesignView 專屬，不影響全域 AI Provider 系統  
**CLI2WebAI 來源**：`/Users/jamesju/Documents/GitCodeBase/CLI2WebAI`

---

## 背景與動機

CLI2WebAI 是一個透過 Playwright 瀏覽器自動化操控 ChatGPT、Claude、Gemini 網頁版的工具，讓使用者可以利用現有的 Web 訂閱（Plus/Pro/Advanced）取得 AI 回應，而無需支付 API Token 費用。

AITerm 的 DesignView 是「規格驅動開發（OpenSpec）」的工作介面，流程為：對話 → Proposal → Spec → SDD → Tasks。它使用純文字對話，無 MCP tool calling，無 agent 模式，是 CLI2WebAI 唯一適合整合的功能入口。

其他功能（AiPanel、LoopStudio）因為有 MCP tool calling 和 agent 模式，與 CLI2WebAI 的能力不相容，不在本設計範圍內。

---

## 決策彙整

| 決定項目 | 結論 |
|---------|------|
| 整合範圍 | DesignView 專屬，不加進全域 Settings > AI Providers |
| 架構方式 | 子進程呼叫（spawn cli2webai binary），CLI2WebAI 自管 daemon |
| Browser session | 同一 Stage 內維持，Stage 切換時重啟（`--new-session`） |
| Binary 分發 | 捆包進 `src-tauri/binaries/`（比照 DB2 sidecar 模式） |
| 登入流程 | AITerm spawn `cli2webai login {platform}` + 對話框，使用者點確認後 AITerm 向 stdin 送 Enter |
| 串流 | 無即時串流，整份回應一次顯示；顯示「等待中」動畫 |
| Provider UI | 單一按鈕互斥切換：Web AI 啟用後 `🤖 預設模型` 變為 `🌐 ChatGPT Web ▾` |

---

## 架構概覽

```
DesignView 前端
  → designChat(sessionId, msgs, providerId?, webPlatform?, webNewSession?)
      ↓
Rust design_chat command
  if web_platform.is_some():
    → WebBrowserProvider::new(platform, new_session, binary_path)
    → 合併 prompt（system + history + latest）
    → spawn: cli2webai ask "{prompt}" --platform {platform} [--new-session]
    → 等待 stdout → 單一 GenerateChunk { delta: 全文, done: true }
  else:
    → router.resolve(provider_id)（現有邏輯不變）
```

CLI2WebAI binary 內嵌所有 JS 檔案（`include_str!`），首次執行時自動：
1. 解壓 JS 到 `~/.config/cli2webai/js/`
2. 執行 `npm install`
3. 下載 Playwright Chromium

因此 AITerm 只需捆包 cli2webai binary，使用者機器上需有 Node.js。

---

## 模組一：DesignView 前端

### 新增 State

```typescript
const [webPlatform, setWebPlatform] = useState<"chatgpt" | "claude" | "gemini" | null>(null);
const [lastSentStage, setLastSentStage] = useState<string | null>(null);
const [loginDialogOpen, setLoginDialogOpen] = useState(false);
const [loginStep, setLoginStep] = useState<"idle" | "waiting" | "done">("idle");
```

### Provider 按鈕（互斥切換）

現有的 `🤖 預設模型` 按鈕行為擴充：

- **Web AI 關閉**：顯示 `🤖 {providerName ?? '預設模型'}`，點擊開啟擴充後的 dropdown
- **Web AI 開啟**：顯示 `🌐 {platform} Web ▾`，點擊開啟同一 dropdown

Dropdown 內容（同一介面，互斥選擇）：
```
── Web AI ──────────────────
  🌐 ChatGPT Plus
  🌐 Claude Pro
  🌐 Gemini Advanced
── API Provider ────────────
  {現有 ProviderPalette 列表}
  關閉 Web AI（恢復 API）
```

選擇 Web AI 平台時觸發登入檢查（見模組三）。

### Stage 切換偵測

```typescript
async function handleSendMessage() {
  // Web AI 剛啟用（從 null 切換過來）也視為需要新 session
  const needsNewSession =
    webPlatform !== null &&
    (session?.status !== lastSentStage || lastSentStage === null);

  await designChat(sessionId, msgs, providerId,
    webPlatform ?? undefined,
    needsNewSession ? true : undefined
  );
  setLastSentStage(session?.status ?? null);
}

// 關閉 Web AI 時重置 lastSentStage，確保下次重新啟用強制開新 session
function handleDisableWebAi() {
  setWebPlatform(null);
  setLastSentStage(null);
}
```

---

## 模組二：後端 — WebBrowserProvider

**新增檔案**：`src-tauri/src/ai/webbrowser.rs`

```rust
pub struct WebBrowserProvider {
    platform: String,      // "chatgpt" | "claude" | "gemini"
    new_session: bool,
    binary_path: PathBuf,
}

#[async_trait]
impl AiProvider for WebBrowserProvider {
    fn id(&self) -> &str { "cli2webai" }
    fn display_name(&self) -> &str { "CLI2WebAI (Web)" }

    async fn generate(&self, req: GenerateRequest, tx: Sender<GenerateChunk>) -> Result<(), AiError> {
        let prompt = build_web_prompt(&req);
        let mut cmd = Command::new(&self.binary_path);
        cmd.args(["ask", &prompt, "--platform", &self.platform]);
        if self.new_session { cmd.arg("--new-session"); }

        let output = cmd.output().await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(AiError::ModelError { reason: stderr.clone(), raw: stderr });
        }

        let response = String::from_utf8_lossy(&output.stdout).to_string();
        let _ = tx.send(GenerateChunk { delta: response, done: true, usage: None }).await;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), AiError> {
        if !self.binary_path.exists() {
            return Err(AiError::NotConfigured);
        }
        Ok(())
    }
}
```

### Prompt 合併格式

```rust
fn build_web_prompt(req: &GenerateRequest) -> String {
    let history = req.messages.iter()
        .take(req.messages.len().saturating_sub(1))
        .map(|m| format!("{}: {}", m.role, m.content.as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n");

    let last = req.messages.last()
        .and_then(|m| m.content.as_str())
        .unwrap_or("");

    format!(
        "===== SYSTEM INSTRUCTIONS =====\n{}\n\n===== CONVERSATION HISTORY =====\n{}\n\n===== CURRENT REQUEST =====\n{}",
        req.system_prompt, history, last
    )
}
```

---

## 模組三：後端 — design_chat 修改

**修改檔案**：`src-tauri/src/commands/design.rs`

```rust
#[tauri::command]
pub async fn design_chat(
    // ... 現有參數 ...
    web_platform: Option<String>,     // 新增
    web_new_session: Option<bool>,    // 新增
) -> Result<AiChatReply, AiError> {
    // ... 現有邏輯（session 載入、prompt 建構）...

    let provider: Arc<dyn AiProvider> = match web_platform.as_deref() {
        Some(platform) => {
            let binary_path = resolve_cli2webai_binary(&app)?;
            Arc::new(WebBrowserProvider {
                platform: platform.to_string(),
                new_session: web_new_session.unwrap_or(false),
                binary_path,
            })
        }
        None => match provider_id.as_deref() {
            Some(id) => router.resolve_by_id(id).await?,
            None => router.resolve().await?,
        },
    };

    // ... 現有 streaming 邏輯不變 ...
}

fn resolve_cli2webai_binary(app: &AppHandle) -> Result<PathBuf, AiError> {
    // Tauri sidecar 解析路徑
    app.path().resource_dir()
        .map(|d| d.join("binaries").join(format!("cli2webai-{}", tauri::utils::platform::target_triple().unwrap_or_default())))
        .map_err(|_| AiError::NotConfigured)
}
```

---

## 模組四：登入流程

**新增 AppState**（存放登入子進程的 stdin handle）：

```rust
// src-tauri/src/lib.rs 加入
pub struct WebAiLoginState {
    pub stdin: Mutex<Option<tokio::process::ChildStdin>>,
}
```

**新增 IPC commands**：

```rust
// 檢查是否已登入（~/.config/cli2webai/profiles/{platform}/ 目錄非空即視為已登入）
#[tauri::command]
pub async fn web_ai_check_login(platform: String) -> Result<bool, String> { ... }

// 啟動 headed 瀏覽器登入流程（spawn cli2webai login {platform}）
// 將子進程 stdin handle 存入 WebAiLoginState
#[tauri::command]
pub async fn web_ai_login_start(
    platform: String,
    login_state: State<'_, WebAiLoginState>,
    app: AppHandle,
) -> Result<(), String> { ... }

// 使用者確認完成登入 → 向子進程 stdin 寫入 "\n" → 子進程儲存 session 並退出
#[tauri::command]
pub async fn web_ai_login_confirm(
    login_state: State<'_, WebAiLoginState>,
) -> Result<(), String> { ... }
```

**前端登入 Dialog 流程**：

```
選擇 Web AI 平台
  → web_ai_check_login(platform)
      → true：直接啟用 webPlatform
      → false：開啟 LoginDialog
          → 使用者點「開啟瀏覽器」
              → web_ai_login_start(platform)
              → Dialog 顯示「請在 Chromium 中完成登入...」
              → 使用者點「我已登入完成」
                  → web_ai_login_confirm()
                  → Dialog 關閉，webPlatform 設定完成
```

---

## 模組五：錯誤處理

| 錯誤情境 | 處理方式 |
|---------|---------|
| cli2webai binary 不存在 | 顯示「Web AI 元件未找到」錯誤 |
| Node.js 未安裝 | stderr 包含 npm 錯誤 → 提示「請先安裝 Node.js」 |
| 首次執行（npm install + Playwright 下載） | 顯示進度對話框「正在初始化環境，約 1-2 分鐘...」 |
| 未登入 | stderr 包含「請先執行登入」→ 觸發 LoginDialog |
| 回應逾時（>90 秒） | `tokio::time::timeout` 包裹 spawn → 顯示錯誤附「重試」 |

---

## Binary 捆包

在 `src-tauri/tauri.conf.json` 加入：

```json
{
  "bundle": {
    "externalBin": ["binaries/cli2webai"]
  }
}
```

各平台需分別編譯 cli2webai binary：
- macOS：`cli2webai-aarch64-apple-darwin` / `cli2webai-x86_64-apple-darwin`
- Windows：`cli2webai-x86_64-pc-windows-msvc.exe`
- Linux：`cli2webai-x86_64-unknown-linux-gnu`

---

## 已知限制與風險

| 風險 | 說明 |
|------|------|
| Prompt 長度 | `build_design_prompt` 輸出約 1,000–2,000 字，加上對話歷史可能相當長。ChatGPT/Claude/Gemini 網頁版接受度通常沒問題，但若歷史過長（> 50 輪）應考慮截斷早期訊息 |
| Playwright selector 異動 | 平台若更動網頁結構，CLI2WebAI 可能失效，需更新 cli2webai binary |
| Cloudflare 封鎖 | CLI2WebAI 內建 stealth 防偵測，但無法 100% 保證 |
| Node.js 版本相容性 | 需要 Node.js ≥ 18（Playwright 要求），版本過舊會在 npm install 時失敗 |

## 不在範圍內

- AiPanel、LoopStudio、VcsView 等其他功能不整合 CLI2WebAI
- Web AI 不加入全域 Settings > AI Providers
- 不支援 MCP tool calling（CLI2WebAI 架構限制）
- 不支援即時串流（瀏覽器自動化架構限制）
