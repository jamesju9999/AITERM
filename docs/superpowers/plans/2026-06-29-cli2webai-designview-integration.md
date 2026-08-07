# CLI2WebAI × DesignView 整合實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 DesignView 加入 Web AI 模式，允許使用者透過 CLI2WebAI 呼叫 ChatGPT/Claude/Gemini 網頁版進行規格設計對話，無需 API Key。

**Architecture:** 子進程呼叫方式。`design_chat` Tauri command 接收 `web_platform` 參數，若有則建立 `WebBrowserProvider`，spawn `cli2webai ask` binary 取得回應後以單一 chunk 送出。登入流程透過 spawn `cli2webai login` + 前端對話框 + stdin 訊號實現。

**Tech Stack:** Rust (tokio::process::Command, async-trait), TypeScript/React (useState, Tauri invoke), cli2webai binary (捆包進 src-tauri/binaries/)

**Design Spec:** `docs/superpowers/specs/2026-06-29-cli2webai-designview-integration-design.md`

---

## 檔案結構

**新增：**
- `src-tauri/src/ai/webbrowser.rs` — `WebBrowserProvider` 實作 + `build_web_prompt()`
- `src-tauri/src/commands/web_ai.rs` — `WebAiLoginState` + 3 個登入 IPC commands
- `src/ipc/webai.ts` — 前端登入 IPC bindings

**修改：**
- `src-tauri/src/ai/mod.rs` — 加入 `pub mod webbrowser`
- `src-tauri/src/commands/mod.rs` — 加入 `pub mod web_ai`
- `src-tauri/src/commands/design.rs` — `design_chat` 加入 `web_platform` + `web_new_session` 路由
- `src-tauri/src/lib.rs` — 管理 `WebAiLoginState`、註冊 3 個新 commands
- `src-tauri/tauri.conf.json` — `externalBin` 加入 `"binaries/cli2webai"`
- `src/ipc/design.ts` — `designChat()` 加入 `webPlatform?`, `webNewSession?`
- `src/components/DesignView/DesignView.tsx` — Web AI UI、登入 Dialog、stage 追蹤

---

## Task 1：WebBrowserProvider（Rust）

**Files:**
- Create: `src-tauri/src/ai/webbrowser.rs`
- Modify: `src-tauri/src/ai/mod.rs`

- [ ] **Step 1.1：在 `mod.rs` 加入模組宣告**

開啟 `src-tauri/src/ai/mod.rs`，在現有 `pub mod` 列表的末尾加入一行：

```rust
pub mod webbrowser;
```

- [ ] **Step 1.2：建立 `webbrowser.rs` 並寫 `build_web_prompt` 的測試**

建立 `src-tauri/src/ai/webbrowser.rs`，內容如下：

```rust
use std::path::PathBuf;
use async_trait::async_trait;
use tokio::process::Command;
use tokio::sync::mpsc;
use std::process::Stdio;

use super::{AiError, AiProvider, GenerateChunk, GenerateRequest};

pub struct WebBrowserProvider {
    pub platform: String,    // "chatgpt" | "claude" | "gemini"
    pub new_session: bool,
    pub binary_path: PathBuf,
}

/// 將 GenerateRequest 合併為 cli2webai 可接受的單一 prompt 字串。
pub fn build_web_prompt(req: &GenerateRequest) -> String {
    let msg_count = req.messages.len();
    let history: String = if msg_count > 1 {
        req.messages[..msg_count - 1]
            .iter()
            .map(|m| {
                let content = m.content.as_str().unwrap_or("");
                format!("{}: {}", m.role, content)
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    };

    let last = req.messages.last()
        .and_then(|m| m.content.as_str())
        .unwrap_or("");

    if history.is_empty() {
        format!(
            "===== SYSTEM INSTRUCTIONS =====\n{}\n\n===== CURRENT REQUEST =====\n{}",
            req.system_prompt, last
        )
    } else {
        format!(
            "===== SYSTEM INSTRUCTIONS =====\n{}\n\n===== CONVERSATION HISTORY =====\n{}\n\n===== CURRENT REQUEST =====\n{}",
            req.system_prompt, history, last
        )
    }
}

#[async_trait]
impl AiProvider for WebBrowserProvider {
    fn id(&self) -> &str { "cli2webai" }
    fn display_name(&self) -> &str { "CLI2WebAI (Web)" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let prompt = build_web_prompt(&req);

        let mut cmd = Command::new(&self.binary_path);
        cmd.args(["ask", &prompt, "--platform", &self.platform]);
        if self.new_session {
            cmd.arg("--new-session");
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(90),
            cmd.output(),
        )
        .await
        .map_err(|_| AiError::Network { message: "CLI2WebAI 回應逾時（90 秒）".into() })?
        .map_err(|e| AiError::Network { message: e.to_string() })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(AiError::ModelError {
                reason: stderr.clone(),
                raw: stderr,
            });
        }

        let response = String::from_utf8_lossy(&output.stdout).trim().to_string();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ChatMessage, EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn make_req(system: &str, msgs: Vec<(&str, &str)>) -> GenerateRequest {
        GenerateRequest {
            system_prompt: system.to_string(),
            messages: msgs.into_iter().map(|(role, content)| ChatMessage {
                role: role.to_string(),
                content: serde_json::Value::String(content.to_string()),
                tool_call_id: None,
                tool_calls: None,
            }).collect(),
            context: EnvSnapshot::default(),
            mode: QueryMode::Chat,
            max_tokens: None,
        }
    }

    #[test]
    fn single_message_omits_history_section() {
        let req = make_req("System prompt.", vec![("user", "Hello")]);
        let prompt = build_web_prompt(&req);
        assert!(prompt.contains("===== SYSTEM INSTRUCTIONS ====="));
        assert!(prompt.contains("System prompt."));
        assert!(!prompt.contains("===== CONVERSATION HISTORY ====="));
        assert!(prompt.contains("===== CURRENT REQUEST =====\nHello"));
    }

    #[test]
    fn multiple_messages_includes_history() {
        let req = make_req("Sys", vec![
            ("user", "first"),
            ("assistant", "reply"),
            ("user", "second"),
        ]);
        let prompt = build_web_prompt(&req);
        assert!(prompt.contains("===== CONVERSATION HISTORY ====="));
        assert!(prompt.contains("user: first"));
        assert!(prompt.contains("assistant: reply"));
        assert!(prompt.contains("===== CURRENT REQUEST =====\nsecond"));
        // 最後一條 user msg 不應出現在 HISTORY 區
        assert!(!prompt.contains("user: second\n"));
    }

    #[test]
    fn empty_messages_returns_gracefully() {
        let req = make_req("Sys", vec![]);
        let prompt = build_web_prompt(&req);
        assert!(prompt.contains("===== SYSTEM INSTRUCTIONS =====\nSys"));
        assert!(prompt.contains("===== CURRENT REQUEST =====\n"));
    }
}
```

- [ ] **Step 1.3：執行測試確認通過**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM/src-tauri && cargo test ai::webbrowser 2>&1 | tail -20
```

預期：3 個測試全部 PASS。

- [ ] **Step 1.4：Commit**

```bash
git add src-tauri/src/ai/webbrowser.rs src-tauri/src/ai/mod.rs
git commit -m "feat(ai): add WebBrowserProvider for cli2webai subprocess integration"
```

---

## Task 2：登入 IPC Commands（Rust）

**Files:**
- Create: `src-tauri/src/commands/web_ai.rs`
- Modify: `src-tauri/src/commands/mod.rs`

- [ ] **Step 2.1：在 `commands/mod.rs` 加入模組宣告**

開啟 `src-tauri/src/commands/mod.rs`，在現有列表末尾加入：

```rust
pub mod web_ai;
```

- [ ] **Step 2.2：建立 `web_ai.rs`**

建立 `src-tauri/src/commands/web_ai.rs`：

```rust
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use std::process::Stdio;
use tauri::{AppHandle, State};

/// 存放登入子進程的 stdin handle，等待前端確認後寫入 "\n"。
pub struct WebAiLoginState {
    pub stdin: Mutex<Option<tokio::process::ChildStdin>>,
}

impl WebAiLoginState {
    pub fn new() -> Self {
        Self { stdin: Mutex::new(None) }
    }
}

/// 檢查指定平台是否已有登入的 session。
/// cli2webai 的 profile 路徑為 ~/.config/cli2webai/profiles/{platform}/
#[tauri::command]
pub async fn web_ai_check_login(platform: String) -> Result<bool, String> {
    let profile_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cli2webai")
        .join("profiles")
        .join(&platform);

    if !profile_dir.exists() {
        return Ok(false);
    }
    // 目錄存在且非空即視為已登入
    let has_files = std::fs::read_dir(&profile_dir)
        .map(|mut d| d.next().is_some())
        .unwrap_or(false);
    Ok(has_files)
}

/// 啟動 headed 瀏覽器登入流程。
/// spawn `cli2webai login {platform}`，將子進程的 stdin 存入 WebAiLoginState。
#[tauri::command]
pub async fn web_ai_login_start(
    platform: String,
    app: AppHandle,
    login_state: State<'_, WebAiLoginState>,
) -> Result<(), String> {
    let binary_path = resolve_cli2webai_binary(&app)
        .map_err(|_| "找不到 cli2webai binary，請確認安裝".to_string())?;

    let mut child = Command::new(&binary_path)
        .args(["login", &platform])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("無法啟動登入程序: {e}"))?;

    let stdin = child.stdin.take()
        .ok_or_else(|| "無法取得子進程 stdin".to_string())?;

    *login_state.stdin.lock().await = Some(stdin);

    // 讓子進程在背景執行，不阻塞 command
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    Ok(())
}

/// 使用者在瀏覽器完成登入後呼叫此 command。
/// 向子進程 stdin 寫入 "\n" 觸發 cli2webai 儲存 session 並退出。
#[tauri::command]
pub async fn web_ai_login_confirm(
    login_state: State<'_, WebAiLoginState>,
) -> Result<(), String> {
    let mut guard = login_state.stdin.lock().await;
    if let Some(mut stdin) = guard.take() {
        stdin.write_all(b"\n").await
            .map_err(|e| format!("寫入 stdin 失敗: {e}"))?;
        stdin.flush().await
            .map_err(|e| format!("flush stdin 失敗: {e}"))?;
    }
    Ok(())
}

/// 解析 cli2webai binary 路徑（比照 DB2 sidecar 模式，掃描候選路徑）。
pub fn resolve_cli2webai_binary(app: &AppHandle) -> Result<PathBuf, ()> {
    use tauri::Manager;
    let exe = std::env::current_exe().map_err(|_| ())?;
    let exe_dir = exe.parent().unwrap_or_else(|| std::path::Path::new("."));

    #[cfg(target_os = "macos")]
    {
        let triple = if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        };
        let name = format!("cli2webai-{triple}");
        let candidates = [
            exe_dir.join(&name),
            exe_dir.join("../Resources").join(&name),
            exe_dir.join("../binaries").join(&name),   // dev
        ];
        return candidates.iter().find(|p| p.exists()).map(|p| p.to_path_buf()).ok_or(());
    }
    #[cfg(target_os = "windows")]
    {
        let name = "cli2webai-x86_64-pc-windows-msvc.exe";
        let candidates = [
            exe_dir.join(name),
            exe_dir.join("binaries").join(name),
        ];
        return candidates.iter().find(|p| p.exists()).map(|p| p.to_path_buf()).ok_or(());
    }
    #[cfg(target_os = "linux")]
    {
        let triple = if cfg!(target_arch = "aarch64") {
            "aarch64-unknown-linux-gnu"
        } else {
            "x86_64-unknown-linux-gnu"
        };
        let name = format!("cli2webai-{triple}");
        let candidates = [
            exe_dir.join(&name),
            exe_dir.join("binaries").join(&name),
        ];
        return candidates.iter().find(|p| p.exists()).map(|p| p.to_path_buf()).ok_or(());
    }
    #[allow(unreachable_code)]
    Err(())
}
```

- [ ] **Step 2.3：編譯確認無誤**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM/src-tauri && cargo check 2>&1 | grep -E "error|warning: unused" | head -20
```

預期：無 error。

- [ ] **Step 2.4：Commit**

```bash
git add src-tauri/src/commands/web_ai.rs src-tauri/src/commands/mod.rs
git commit -m "feat(commands): add web_ai login IPC commands and WebAiLoginState"
```

---

## Task 3：修改 `design_chat` 加入 Web AI 路由

**Files:**
- Modify: `src-tauri/src/commands/design.rs`

- [ ] **Step 3.1：在 `design.rs` 加入 import**

在 `src-tauri/src/commands/design.rs` 頂部現有 `use` 列表中加入：

```rust
use std::sync::Arc;
use crate::ai::webbrowser::WebBrowserProvider;
use crate::commands::web_ai::resolve_cli2webai_binary;
```

- [ ] **Step 3.2：修改 `design_chat` 函式簽名與路由邏輯**

找到 `design_chat` 函式的參數列（目前最後一個參數是 `router: State<'_, AiRouter>`），加入兩個新參數：

```rust
#[tauri::command]
pub async fn design_chat(
    session_id: String,
    messages: Vec<ChatMessage>,
    provider_id: Option<String>,
    web_platform: Option<String>,       // 新增
    web_new_session: Option<bool>,      // 新增
    app: AppHandle,
    design_db: State<'_, DesignDb>,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiChatReply, AiError> {
```

找到現有的 provider 解析邏輯（目前是 `let provider = match provider_id...`），替換為：

```rust
    // 3. Resolve AI provider
    let provider: Arc<dyn crate::ai::AiProvider> = match web_platform.as_deref() {
        Some(platform) => {
            let binary_path = resolve_cli2webai_binary(&app)
                .map_err(|_| AiError::NotConfigured)?;
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
```

- [ ] **Step 3.3：編譯確認**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM/src-tauri && cargo check 2>&1 | grep "error" | head -10
```

預期：無 error。

- [ ] **Step 3.4：Commit**

```bash
git add src-tauri/src/commands/design.rs
git commit -m "feat(design): route design_chat to WebBrowserProvider when web_platform set"
```

---

## Task 4：lib.rs + tauri.conf.json 設定

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 4.1：在 `lib.rs` import 新模組**

在 `lib.rs` 的 `use` 區塊找到 commands 相關的 import，加入：

```rust
use commands::web_ai::{WebAiLoginState, web_ai_check_login, web_ai_login_start, web_ai_login_confirm};
```

- [ ] **Step 4.2：在 `lib.rs` 管理 `WebAiLoginState`**

在 `.manage(AnthropicOAuthState::new())` 這行下方加入：

```rust
.manage(WebAiLoginState::new())
```

- [ ] **Step 4.3：在 `invoke_handler` 註冊三個新 command**

在 `tauri::generate_handler![...]` 的列表中，找到 `design_delete_session,` 這行，在其後加入：

```rust
web_ai_check_login,
web_ai_login_start,
web_ai_login_confirm,
```

- [ ] **Step 4.4：在 `tauri.conf.json` 加入 externalBin**

開啟 `src-tauri/tauri.conf.json`，找到：
```json
"externalBin": [
  "binaries/db2-sidecar"
]
```
改為：
```json
"externalBin": [
  "binaries/db2-sidecar",
  "binaries/cli2webai"
]
```

- [ ] **Step 4.5：放入 dev 用的 binary stub（讓 cargo check 通過）**

建立一個空的 placeholder binary，讓 Tauri dev 模式不報「找不到 binary」的警告：

```bash
touch /Users/jamesju/Documents/GitHub/AITERM/src-tauri/binaries/cli2webai-aarch64-apple-darwin
chmod +x /Users/jamesju/Documents/GitHub/AITERM/src-tauri/binaries/cli2webai-aarch64-apple-darwin
```

（正式的 binary 需從 CLI2WebAI 專案編譯後置換，見 Task 8。）

- [ ] **Step 4.6：完整編譯**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM/src-tauri && cargo build 2>&1 | grep "error" | head -20
```

預期：無 error。

- [ ] **Step 4.7：Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/tauri.conf.json src-tauri/binaries/cli2webai-aarch64-apple-darwin
git commit -m "feat: register WebAiLoginState and web_ai commands, add cli2webai to externalBin"
```

---

## Task 5：前端 IPC Bindings

**Files:**
- Create: `src/ipc/webai.ts`
- Modify: `src/ipc/design.ts`

- [ ] **Step 5.1：建立 `src/ipc/webai.ts`**

```typescript
import { invoke } from "@tauri-apps/api/core";

export type WebAiPlatform = "chatgpt" | "claude" | "gemini";

/** 檢查指定平台是否已有登入 session */
export const webAiCheckLogin = (platform: WebAiPlatform): Promise<boolean> =>
  invoke("web_ai_check_login", { platform });

/** 啟動登入瀏覽器（spawn cli2webai login）*/
export const webAiLoginStart = (platform: WebAiPlatform): Promise<void> =>
  invoke("web_ai_login_start", { platform });

/** 使用者確認登入完成，送出 Enter 給子進程 */
export const webAiLoginConfirm = (): Promise<void> =>
  invoke("web_ai_login_confirm");

export const WEB_AI_PLATFORM_LABELS: Record<WebAiPlatform, string> = {
  chatgpt: "ChatGPT Plus",
  claude: "Claude Pro",
  gemini: "Gemini Advanced",
};
```

- [ ] **Step 5.2：修改 `src/ipc/design.ts` 的 `designChat()`**

找到現有的 `designChat` 函式：

```typescript
export async function designChat(
  sessionId: string,
  messages: ChatMessage[],
  providerId?: string
): Promise<DesignChatReply> {
  return invoke('design_chat', { sessionId, messages, providerId: providerId ?? null });
}
```

替換為：

```typescript
export async function designChat(
  sessionId: string,
  messages: ChatMessage[],
  providerId?: string,
  webPlatform?: string,
  webNewSession?: boolean,
): Promise<DesignChatReply> {
  return invoke('design_chat', {
    sessionId,
    messages,
    providerId: providerId ?? null,
    webPlatform: webPlatform ?? null,
    webNewSession: webNewSession ?? null,
  });
}
```

- [ ] **Step 5.3：Type check**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit 2>&1 | grep "error" | head -20
```

預期：無 error。

- [ ] **Step 5.4：Commit**

```bash
git add src/ipc/webai.ts src/ipc/design.ts
git commit -m "feat(ipc): add Web AI IPC bindings and extend designChat with webPlatform"
```

---

## Task 6：DesignView — Provider 按鈕 + Web AI Dropdown

**Files:**
- Modify: `src/components/DesignView/DesignView.tsx`

- [ ] **Step 6.1：加入 import 和新 state**

在 `DesignView.tsx` 頂部找到現有 import 區，加入：

```typescript
import { webAiCheckLogin, webAiLoginStart, webAiLoginConfirm, WEB_AI_PLATFORM_LABELS, type WebAiPlatform } from '../../ipc/webai';
```

在現有 `const [providerId, setProviderId]` state 附近加入：

```typescript
const [webPlatform, setWebPlatform] = useState<WebAiPlatform | null>(null);
const [showWebAiDropdown, setShowWebAiDropdown] = useState(false);
const [loginDialogOpen, setLoginDialogOpen] = useState(false);
const [loginStep, setLoginStep] = useState<"idle" | "waiting" | "done">("idle");
const [loginPlatform, setLoginPlatform] = useState<WebAiPlatform | null>(null);
const [lastSentStage, setLastSentStage] = useState<string | null>(null);
```

- [ ] **Step 6.2：實作 `handleSelectWebAi` 函式**

在 `handleSendMessage` 函式上方加入：

```typescript
async function handleSelectWebAi(platform: WebAiPlatform) {
  setShowWebAiDropdown(false);
  const loggedIn = await webAiCheckLogin(platform);
  if (loggedIn) {
    setWebPlatform(platform);
    setLastSentStage(null); // 強制下次開新 session
  } else {
    setLoginPlatform(platform);
    setLoginDialogOpen(true);
  }
}

function handleDisableWebAi() {
  setWebPlatform(null);
  setLastSentStage(null);
  setShowWebAiDropdown(false);
}
```

- [ ] **Step 6.3：替換 provider 按鈕 JSX**

找到現有的 provider 按鈕：
```tsx
<button
  className="design-provider-btn"
  onClick={() => setShowProviderPalette(true)}
>
  🤖 {providerName ? `模型: ${providerName}` : '預設模型'}
</button>
```

替換為：

```tsx
<div style={{ position: 'relative' }}>
  <button
    className="design-provider-btn"
    onClick={() => setShowWebAiDropdown(o => !o)}
  >
    {webPlatform
      ? `🌐 ${WEB_AI_PLATFORM_LABELS[webPlatform]} ▾`
      : `🤖 ${providerName ? `模型: ${providerName}` : '預設模型'} ▾`}
  </button>

  {showWebAiDropdown && (
    <div
      className="design-webai-dropdown"
      style={{
        position: 'absolute', bottom: '100%', left: 0,
        background: '#1e1e2e', border: '1px solid #7c3aed44',
        borderRadius: 6, padding: '6px 0', minWidth: 180, zIndex: 100,
      }}
    >
      <div style={{ padding: '4px 12px', color: '#666', fontSize: 10 }}>Web AI</div>
      {(['chatgpt', 'claude', 'gemini'] as WebAiPlatform[]).map(p => (
        <button
          key={p}
          onClick={() => handleSelectWebAi(p)}
          style={{
            display: 'block', width: '100%', textAlign: 'left',
            padding: '5px 12px', background: webPlatform === p ? '#7c3aed22' : 'transparent',
            color: webPlatform === p ? '#c4b5fd' : '#ccc',
            border: 'none', cursor: 'pointer', fontSize: 11,
          }}
        >
          🌐 {WEB_AI_PLATFORM_LABELS[p]} {webPlatform === p ? '✓' : ''}
        </button>
      ))}
      <hr style={{ border: 'none', borderTop: '1px solid #333', margin: '4px 0' }} />
      <div style={{ padding: '4px 12px', color: '#666', fontSize: 10 }}>API Provider</div>
      <button
        onClick={() => { setShowWebAiDropdown(false); setShowProviderPalette(true); }}
        style={{
          display: 'block', width: '100%', textAlign: 'left',
          padding: '5px 12px', background: !webPlatform ? '#7c3aed22' : 'transparent',
          color: !webPlatform ? '#c4b5fd' : '#ccc',
          border: 'none', cursor: 'pointer', fontSize: 11,
        }}
      >
        🤖 {providerName ?? '預設模型'} {!webPlatform ? '✓' : ''}
      </button>
      {webPlatform && (
        <button
          onClick={handleDisableWebAi}
          style={{
            display: 'block', width: '100%', textAlign: 'left',
            padding: '5px 12px', background: 'transparent',
            color: '#888', border: 'none', cursor: 'pointer', fontSize: 11,
          }}
        >
          關閉 Web AI
        </button>
      )}
    </div>
  )}
</div>
```

同時在 backdrop（點擊外部關閉 dropdown）：在 DesignView 根 `div` 的 `onClick` 加入：
```tsx
onClick={() => setShowWebAiDropdown(false)}
```

- [ ] **Step 6.4：Type check**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit 2>&1 | grep "error" | head -20
```

- [ ] **Step 6.5：Commit**

```bash
git add src/components/DesignView/DesignView.tsx
git commit -m "feat(design-view): add Web AI provider toggle with mutually exclusive dropdown"
```

---

## Task 7：DesignView — Stage 追蹤 + handleSendMessage 修改

**Files:**
- Modify: `src/components/DesignView/DesignView.tsx`

- [ ] **Step 7.1：修改 `handleSendMessage` 加入 stage 追蹤**

`handleSendMessage` 在 `DesignView.tsx:227` 是一個 `useCallback`。找到它呼叫 `designChat` 的那一行（約 line 248）：

```typescript
const response = await designChat(session.id, combinedMessages, providerId);
```

替換為：

```typescript
// Stage 變更或 Web AI 剛啟用時，強制開新 browser session
const needsNewSession =
  webPlatform !== null &&
  (session?.status !== lastSentStage || lastSentStage === null);

const response = await designChat(
  session.id,
  combinedMessages,
  providerId,
  webPlatform ?? undefined,
  needsNewSession || undefined,
);

// 記錄本次送出時的 stage，用於下次偵測變更
setLastSentStage(session?.status ?? null);
```

同時更新 `useCallback` 的 deps array，加入 `webPlatform` 和 `lastSentStage`：

```typescript
// 找到 handleSendMessage 的 deps array（目前大約是這樣）
}, [inputValue, session, messages, isStreaming, refreshSession, providerId]);
// 改為：
}, [inputValue, session, messages, isStreaming, refreshSession, providerId, webPlatform, lastSentStage]);
```

- [ ] **Step 7.2：Type check + lint**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit 2>&1 | grep "error" | head -10
npm run lint 2>&1 | grep "error" | head -10
```

預期：無 error。

- [ ] **Step 7.3：Commit**

```bash
git add src/components/DesignView/DesignView.tsx
git commit -m "feat(design-view): detect stage changes to trigger new cli2webai browser session"
```

---

## Task 8：DesignView — Login Dialog

**Files:**
- Modify: `src/components/DesignView/DesignView.tsx`

- [ ] **Step 8.1：加入 Login Dialog JSX**

在 `DesignView` 的 return JSX 末尾，在 `{showProviderPalette && <ProviderPalette ... />}` 之後加入：

```tsx
{loginDialogOpen && loginPlatform && (
  <div
    style={{
      position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)',
      display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 200,
    }}
  >
    <div style={{
      background: '#1e1e2e', border: '1px solid #7c3aed55',
      borderRadius: 8, padding: 20, maxWidth: 360, width: '90%', color: '#ccc',
    }}>
      <div style={{ fontWeight: 'bold', marginBottom: 8, color: '#c4b5fd' }}>
        🌐 {WEB_AI_PLATFORM_LABELS[loginPlatform]} 登入
      </div>

      {loginStep === 'idle' && (
        <>
          <p style={{ fontSize: 12, color: '#888', marginBottom: 12 }}>
            需要先登入 {WEB_AI_PLATFORM_LABELS[loginPlatform]} 才能使用 Web AI 模式。
            點擊「開啟瀏覽器」後，請在 Chromium 視窗中完成登入。
          </p>
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button
              onClick={() => { setLoginDialogOpen(false); setLoginStep('idle'); }}
              style={{ padding: '4px 12px', background: '#333', border: 'none', borderRadius: 4, color: '#aaa', cursor: 'pointer' }}
            >
              取消
            </button>
            <button
              onClick={async () => {
                setLoginStep('waiting');
                await webAiLoginStart(loginPlatform);
              }}
              style={{ padding: '4px 12px', background: '#7c3aed', border: 'none', borderRadius: 4, color: '#fff', cursor: 'pointer' }}
            >
              開啟瀏覽器
            </button>
          </div>
        </>
      )}

      {loginStep === 'waiting' && (
        <>
          <p style={{ fontSize: 12, color: '#888', marginBottom: 12 }}>
            ⏳ 瀏覽器已開啟，請在 Chromium 視窗中完成 {WEB_AI_PLATFORM_LABELS[loginPlatform]} 登入。
            完成後點擊「我已登入完成」。
          </p>
          <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
            <button
              onClick={() => { setLoginDialogOpen(false); setLoginStep('idle'); }}
              style={{ padding: '4px 12px', background: '#333', border: 'none', borderRadius: 4, color: '#aaa', cursor: 'pointer' }}
            >
              取消
            </button>
            <button
              onClick={async () => {
                await webAiLoginConfirm();
                setLoginStep('done');
                setWebPlatform(loginPlatform);
                setLastSentStage(null);
                setLoginDialogOpen(false);
                setLoginStep('idle');
              }}
              style={{ padding: '4px 12px', background: '#22c55e', border: 'none', borderRadius: 4, color: '#fff', cursor: 'pointer' }}
            >
              ✓ 我已登入完成
            </button>
          </div>
        </>
      )}
    </div>
  </div>
)}
```

- [ ] **Step 8.2：Type check**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM && npx tsc --noEmit 2>&1 | grep "error" | head -20
```

預期：無 error。

- [ ] **Step 8.3：Commit**

```bash
git add src/components/DesignView/DesignView.tsx
git commit -m "feat(design-view): add Web AI login dialog with browser spawn and confirm flow"
```

---

## Task 9：編譯真正的 cli2webai binary

**Files:**
- `src-tauri/binaries/cli2webai-aarch64-apple-darwin`（或其他平台）

- [ ] **Step 9.1：從 CLI2WebAI 專案編譯**

```bash
cd /Users/jamesju/Documents/GitCodeBase/CLI2WebAI
cargo build --release
```

- [ ] **Step 9.2：複製 binary 到 AITerm binaries 目錄**

macOS (Apple Silicon)：
```bash
cp /Users/jamesju/Documents/GitCodeBase/CLI2WebAI/target/release/cli2webai \
   /Users/jamesju/Documents/GitHub/AITERM/src-tauri/binaries/cli2webai-aarch64-apple-darwin
chmod +x /Users/jamesju/Documents/GitHub/AITERM/src-tauri/binaries/cli2webai-aarch64-apple-darwin
```

確認：
```bash
ls -lh /Users/jamesju/Documents/GitHub/AITERM/src-tauri/binaries/cli2webai-*
```

預期：看到 binary 大小 > 1MB（代表不是 stub）。

- [ ] **Step 9.3：確認 binary 可執行**

```bash
/Users/jamesju/Documents/GitHub/AITERM/src-tauri/binaries/cli2webai-aarch64-apple-darwin --help 2>&1 | head -5
```

預期：輸出 cli2webai help 資訊。

- [ ] **Step 9.4：Commit**

```bash
git add src-tauri/binaries/cli2webai-aarch64-apple-darwin
git commit -m "chore: add compiled cli2webai binary for macOS Apple Silicon"
```

---

## Task 10：手動驗收測試

- [ ] **Step 10.1：啟動 dev server**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM && npm run tauri:dev
```

- [ ] **Step 10.2：測試 provider 按鈕 dropdown**

1. 開啟 DesignView（側邊欄 → 設計工作室）
2. 點擊輸入區的 provider 按鈕，確認 dropdown 顯示 Web AI 選項 + 現有 API providers
3. 選擇 ChatGPT Plus → 若尚未登入，確認 Login Dialog 出現

- [ ] **Step 10.3：測試登入流程**

1. Login Dialog 出現後點「開啟瀏覽器」
2. 確認 Chromium 視窗開啟並導向 ChatGPT 登入頁
3. 完成登入後點「我已登入完成」
4. 確認 Dialog 關閉，provider 按鈕顯示 `🌐 ChatGPT Plus ▾`

- [ ] **Step 10.4：測試 Web AI 對話**

1. 輸入「我想要做一個任務管理系統」並送出
2. 確認 DesignView 顯示「等待中」動畫
3. 確認回應出現在對話區（完整 markdown，不是逐字）

- [ ] **Step 10.5：測試 Stage 切換**

1. 在 Draft 階段完成 Proposal 後核准
2. 進入 Spec 階段後送出第一條訊息
3. 在 Rust log 確認（或透過 verbose flag）這次呼叫帶了 `--new-session`

- [ ] **Step 10.6：測試關閉 Web AI**

1. 點 provider 按鈕 → 選「關閉 Web AI」
2. 確認按鈕恢復為 `🤖 ...`
3. 確認下一次對話走 API provider（有即時串流）
