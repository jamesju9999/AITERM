# Claude Code 橋接 M1 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 AITerm 終端機裡的 Claude Code CLI 改打 AITerm 已設定的 OpenAI 相容或 Anthropic 相容供應商。

**Architecture:** 新增 `src-tauri/src/bridge/` 模組，內含一個綁 `127.0.0.1` 的 axum server，實作 Anthropic Messages API。三條上游路徑收斂到中立的 `UpstreamEvent`，由單一份 SSE 序列化器輸出。憑證解析復用既有 `ai/router.rs`，不改動 `AiProvider` trait。

**Tech Stack:** Rust / axum 0.8 / tokio / reqwest（已有）/ serde_json；前端 React 19 + Vitest。

**設計文件：** `docs/superpowers/specs/2026-08-07-claude-code-bridge-design.md`

---

## 階段總覽

| 階段 | 任務 | 產出 |
|---|---|---|
| A 基礎 | 1–4 | 依賴、模組骨架、config schema、token、層級判定 |
| B 協定層 | 5–7 | Anthropic 請求解析、SSE 序列化、中立事件型別 |
| C OpenAI 路徑 | 8–11 | 請求翻譯、串流 tool_calls 累積器、SSE 解析、adapter |
| D Anthropic 路徑 | 12 | Passthrough adapter |
| E server | 13–16 | 端點、生命週期、Tauri 指令 |
| F 前端與注入 | 17–20 | env 注入、IPC、設定 UI、分頁 UI |
| G 驗收 | 21 | 手動端到端 |

A–C 是純函式庫，每個任務都能單獨 `cargo test` 驗證。D 之後才接線。

---

## 階段 A：基礎

### Task 1: 加入 axum 依賴與模組骨架

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/bridge/mod.rs`
- Modify: `src-tauri/src/lib.rs:1-17`（模組宣告區）

- [ ] **Step 1: 加依賴**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 區塊，`reqwest` 那行後面加上：

```toml
# Claude Code 橋接的本地 HTTP server。hyper/tower/tower-http 已透過
# reqwest 與 tauri 進入 Cargo.lock，這裡只是把 axum 提為直接依賴。
axum = "0.8"
```

- [ ] **Step 2: 建立模組骨架**

建立 `src-tauri/src/bridge/mod.rs`：

```rust
//! Claude Code 橋接：一個綁 127.0.0.1 的 Anthropic Messages API 相容 server，
//! 把 Claude Code CLI 的請求翻譯到 AITerm 已設定的任一 AI 供應商。
//!
//! 為什麼不擴充 `ai::AiProvider`：Claude Code 需要串流 tool_use、thinking
//! 區塊與 cache_control，這些保真度 AITerm 自己的 UI 完全用不到。把它們塞
//! 進共用 trait 會讓 7 支 client、Agent loop 與 chat hook 全進入爆炸半徑，
//! 受益者卻只有一個消費者。因此另開模組，但憑證解析與端點常數共用
//! （見 `upstream/` 各 adapter）。

pub mod auth;
pub mod model_map;
```

- [ ] **Step 3: 註冊模組**

在 `src-tauri/src/lib.rs` 的模組宣告區，`pub mod appimage_env;` 之後插入一行（維持字母排序）：

```rust
pub mod bridge;
```

- [ ] **Step 4: 驗證編譯**

Run: `cd src-tauri && cargo check`
Expected: 成功。若出現 `bridge::auth` / `bridge::model_map` 找不到檔案，代表 Step 2 的兩個 `pub mod` 提前宣告了尚未建立的檔案 —— 先把那兩行註解掉，Task 2、3 完成後再解開。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/bridge/mod.rs src-tauri/src/lib.rs
git commit -m "feat(bridge): 加入 axum 依賴與 bridge 模組骨架"
```

---

### Task 2: Config schema

**Files:**
- Modify: `src-tauri/src/config/types.rs`（`AppConfig` struct，約 :97 之後）

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/src/config/types.rs` 檔案最末端加入（若已有 `#[cfg(test)] mod tests` 就併入）：

```rust
#[cfg(test)]
mod bridge_config_tests {
    use super::*;

    #[test]
    fn missing_section_gets_defaults() {
        // 舊的 config.toml 沒有 [claude_bridge] 區塊，必須照常載入。
        let cfg: AppConfig = toml::from_str("").expect("空 config 應可載入");
        assert!(!cfg.claude_bridge.enabled);
        assert_eq!(cfg.claude_bridge.port, 8317);
        assert!(cfg.claude_bridge.opus.is_none());
    }

    #[test]
    fn tier_mapping_round_trips() {
        let toml_src = r#"
[claude_bridge]
enabled = true
port = 9000

[claude_bridge.sonnet]
provider_id = "local-qwen"
model = "Qwen3.6-35B-A3B-4bit"
"#;
        let cfg: AppConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.claude_bridge.port, 9000);
        let sonnet = cfg.claude_bridge.sonnet.as_ref().unwrap();
        assert_eq!(sonnet.provider_id, "local-qwen");
        assert_eq!(sonnet.model, "Qwen3.6-35B-A3B-4bit");
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge_config_tests`
Expected: 編譯失敗，`no field 'claude_bridge' on type 'AppConfig'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/config/types.rs` 的 `AppConfig` struct 尾端（`python_index_url` 欄位之後、右大括號之前）加入：

```rust
    /// Claude Code 橋接設定。舊的 config.toml 沒有這個區塊，靠 `default` 補齊。
    #[serde(default)]
    pub claude_bridge: ClaudeBridgeConfig,
```

在同檔案 `fn default_max_agent_steps()` 附近加入型別定義：

```rust
/// 橋接 server 的預設埠。被占用時啟動失敗而非漂移 —— 環境變數只能在分頁
/// spawn 的瞬間決定，埠若會漂移，已開的分頁會指向死位址。
pub fn default_bridge_port() -> u16 { 8317 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeBridgeConfig {
    /// server 是否常駐。
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_bridge_port")]
    pub port: u16,

    /// 新開的終端機分頁是否預設注入橋接環境變數。
    #[serde(default)]
    pub default_on_new_tab: bool,

    #[serde(default)]
    pub opus: Option<TierMapping>,
    #[serde(default)]
    pub sonnet: Option<TierMapping>,
    #[serde(default)]
    pub haiku: Option<TierMapping>,
}

impl Default for ClaudeBridgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_bridge_port(),
            default_on_new_tab: false,
            opus: None,
            sonnet: None,
            haiku: None,
        }
    }
}

/// 一個 Claude Code 模型層級要打到哪個供應商的哪個模型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TierMapping {
    pub provider_id: String,
    pub model: String,
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge_config_tests`
Expected: 2 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config/types.rs
git commit -m "feat(bridge): AppConfig 加入 claude_bridge 設定區塊"
```

---

### Task 3: 橋接 token

**Files:**
- Create: `src-tauri/src/bridge/auth.rs`

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/src/bridge/auth.rs`，先只寫測試區塊：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_is_64_hex_chars() {
        let t = generate_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, generate_token(), "每次呼叫都要不同");
    }

    #[test]
    fn token_matches_is_exact() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc123", "abc124"));
        assert!(!token_matches("abc123", "abc1234"));
        assert!(!token_matches("abc123", ""));
    }

    #[test]
    fn extracts_bearer_token() {
        assert_eq!(extract_token(Some("Bearer xyz"), None).as_deref(), Some("xyz"));
        assert_eq!(extract_token(Some("bearer xyz"), None).as_deref(), Some("xyz"));
    }

    #[test]
    fn falls_back_to_x_api_key() {
        // Claude Code 在只設了 ANTHROPIC_API_KEY 時會送 x-api-key。
        assert_eq!(extract_token(None, Some("xyz")).as_deref(), Some("xyz"));
    }

    #[test]
    fn authorization_wins_over_x_api_key() {
        assert_eq!(extract_token(Some("Bearer a"), Some("b")).as_deref(), Some("a"));
    }

    #[test]
    fn no_credentials_yields_none() {
        assert_eq!(extract_token(None, None), None);
        assert_eq!(extract_token(Some("Basic xyz"), None), None);
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::auth`
Expected: 編譯失敗，`cannot find function 'generate_token'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/bridge/auth.rs` 的測試區塊**之前**加入：

```rust
//! 橋接 server 的 bearer token。存在 OS keychain，key 見 [`BRIDGE_TOKEN_KEY`]。

/// Keychain 的 key。沿用 `SecretStore` 既有的冒號子鍵慣例
/// （例如 `{provider_id}:oauth_refresh`）。
pub const BRIDGE_TOKEN_KEY: &str = "claude-bridge:token";

/// 產生 32 bytes 的隨機 token（64 個 hex 字元）。
///
/// 用兩個 UUIDv4 串接而非引入 `rand`：`uuid` 已是依賴，其 v4 走的是
/// `getrandom`，密碼學強度足夠，且省一個 crate。
pub fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// 常數時間比對，避免用回應時間逐字元猜出 token。
///
/// 長度不同時提早返回會洩漏長度，但長度是固定的 64，不算資訊。
pub fn token_matches(expected: &str, provided: &str) -> bool {
    let a = expected.as_bytes();
    let b = provided.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 從請求標頭取出 token。
///
/// Claude Code 設了 `ANTHROPIC_AUTH_TOKEN` 時送 `Authorization: Bearer`，
/// 只設 `ANTHROPIC_API_KEY` 時送 `x-api-key`。兩種都接受，前者優先。
pub fn extract_token(authorization: Option<&str>, x_api_key: Option<&str>) -> Option<String> {
    if let Some(value) = authorization {
        let trimmed = value.trim();
        if trimmed.len() > 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
            return Some(trimmed[7..].trim().to_string());
        }
    }
    x_api_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::auth`
Expected: 6 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bridge/auth.rs
git commit -m "feat(bridge): 橋接 token 產生、常數時間比對與標頭解析"
```

---

### Task 4: 模型層級判定

**Files:**
- Create: `src-tauri/src/bridge/model_map.rs`
- Modify: `src-tauri/src/bridge/mod.rs`（加 `pub mod model_map;`，若 Task 1 已註解掉則解開）

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/src/bridge/model_map.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{ClaudeBridgeConfig, TierMapping};

    fn cfg_with_sonnet() -> ClaudeBridgeConfig {
        ClaudeBridgeConfig {
            sonnet: Some(TierMapping {
                provider_id: "local-qwen".into(),
                model: "Qwen3.6-35B".into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn sentinel_strings_win() {
        assert_eq!(tier_for_model("aiterm:opus"), Some(Tier::Opus));
        assert_eq!(tier_for_model("aiterm:sonnet"), Some(Tier::Sonnet));
        assert_eq!(tier_for_model("aiterm:haiku"), Some(Tier::Haiku));
    }

    #[test]
    fn falls_back_to_substring_for_real_model_names() {
        // 使用者手動覆寫了環境變數，或未來版本的 Claude Code 送真實型號。
        assert_eq!(tier_for_model("claude-opus-4-20250514"), Some(Tier::Opus));
        assert_eq!(tier_for_model("claude-3-5-haiku-latest"), Some(Tier::Haiku));
    }

    #[test]
    fn haiku_checked_before_sonnet_and_opus() {
        // 假想的複合名稱不能因為比對順序而誤判。
        assert_eq!(tier_for_model("sonnet-and-haiku-mix"), Some(Tier::Haiku));
    }

    #[test]
    fn unknown_model_yields_none() {
        assert_eq!(tier_for_model("gpt-4o"), None);
    }

    #[test]
    fn resolve_returns_mapping_for_configured_tier() {
        let m = resolve(&cfg_with_sonnet(), "aiterm:sonnet").unwrap();
        assert_eq!(m.provider_id, "local-qwen");
        assert_eq!(m.model, "Qwen3.6-35B");
    }

    #[test]
    fn resolve_errors_when_tier_unmapped() {
        let err = resolve(&cfg_with_sonnet(), "aiterm:opus").unwrap_err();
        assert!(err.contains("opus"), "訊息要指出是哪一層沒設定：{err}");
    }

    #[test]
    fn resolve_errors_on_unrecognised_model() {
        let err = resolve(&cfg_with_sonnet(), "gpt-4o").unwrap_err();
        assert!(err.contains("gpt-4o"));
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::model_map`
Expected: 編譯失敗，`cannot find type 'Tier'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/bridge/model_map.rs` 測試區塊之前加入：

```rust
//! Claude Code 送來的 `model` 欄位 → AITerm 的 (provider_id, model)。
//!
//! 我們注入的是 `ANTHROPIC_DEFAULT_OPUS_MODEL=aiterm:opus` 這類哨兵字串，
//! Claude Code 會原樣放進請求的 `model`。這比比對 `claude-opus-4-...` 穩定
//! ——真實型號會隨 Claude Code 版本改變。子字串比對只是使用者手動覆寫時
//! 的後備路徑。

use crate::config::types::{ClaudeBridgeConfig, TierMapping};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Opus,
    Sonnet,
    Haiku,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Opus => "opus",
            Tier::Sonnet => "sonnet",
            Tier::Haiku => "haiku",
        }
    }
}

pub fn tier_for_model(model: &str) -> Option<Tier> {
    match model {
        "aiterm:opus" => return Some(Tier::Opus),
        "aiterm:sonnet" => return Some(Tier::Sonnet),
        "aiterm:haiku" => return Some(Tier::Haiku),
        _ => {}
    }
    // haiku 先比：它是最便宜的一層，誤判成 opus 的代價比反過來大。
    let lower = model.to_ascii_lowercase();
    if lower.contains("haiku") {
        Some(Tier::Haiku)
    } else if lower.contains("sonnet") {
        Some(Tier::Sonnet)
    } else if lower.contains("opus") {
        Some(Tier::Opus)
    } else {
        None
    }
}

/// 查出這個 model 字串該打到哪。錯誤訊息會直接回給 Claude Code 顯示，
/// 所以要寫成使用者看得懂、且指向設定頁的句子。
pub fn resolve<'a>(
    cfg: &'a ClaudeBridgeConfig,
    model: &str,
) -> Result<&'a TierMapping, String> {
    let Some(tier) = tier_for_model(model) else {
        return Err(format!(
            "AITerm 橋接無法判斷模型「{model}」屬於哪一層。請在設定 → Claude Code 橋接檢查模型映射。"
        ));
    };
    let slot = match tier {
        Tier::Opus => &cfg.opus,
        Tier::Sonnet => &cfg.sonnet,
        Tier::Haiku => &cfg.haiku,
    };
    slot.as_ref().ok_or_else(|| {
        format!(
            "AITerm 橋接的 {} 層尚未設定供應商。請到設定 → Claude Code 橋接指定。",
            tier.as_str()
        )
    })
}
```

在 `src-tauri/src/bridge/mod.rs` 確認有這兩行：

```rust
pub mod auth;
pub mod model_map;
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::model_map`
Expected: 7 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bridge/model_map.rs src-tauri/src/bridge/mod.rs
git commit -m "feat(bridge): 模型層級判定與供應商映射查詢"
```

---

## 階段 B：協定層

### Task 5: Anthropic 請求解析

**Files:**
- Create: `src-tauri/src/bridge/anthropic/mod.rs`
- Create: `src-tauri/src/bridge/anthropic/request.rs`
- Modify: `src-tauri/src/bridge/mod.rs`

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/src/bridge/anthropic/request.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_string_content() {
        let blocks = parse_content(&json!("hello"));
        assert_eq!(blocks, vec![ContentBlock::Text("hello".into())]);
    }

    #[test]
    fn parses_text_and_image_blocks() {
        let v = json!([
            {"type": "text", "text": "看這張"},
            {"type": "image", "source": {
                "type": "base64", "media_type": "image/png", "data": "AAAA"
            }}
        ]);
        assert_eq!(
            parse_content(&v),
            vec![
                ContentBlock::Text("看這張".into()),
                ContentBlock::Image { media_type: "image/png".into(), data: "AAAA".into() },
            ]
        );
    }

    #[test]
    fn parses_tool_use_block() {
        let v = json!([{
            "type": "tool_use", "id": "toolu_1", "name": "Read",
            "input": {"file_path": "/tmp/a"}
        }]);
        assert_eq!(
            parse_content(&v),
            vec![ContentBlock::ToolUse {
                id: "toolu_1".into(),
                name: "Read".into(),
                input: json!({"file_path": "/tmp/a"}),
            }]
        );
    }

    #[test]
    fn parses_tool_result_with_nested_blocks() {
        let v = json!([{
            "type": "tool_result", "tool_use_id": "toolu_1",
            "content": [{"type": "text", "text": "檔案內容"}]
        }]);
        assert_eq!(
            parse_content(&v),
            vec![ContentBlock::ToolResult {
                tool_use_id: "toolu_1".into(),
                content: vec![ContentBlock::Text("檔案內容".into())],
                is_error: false,
            }]
        );
    }

    #[test]
    fn tool_result_content_may_be_a_bare_string() {
        let v = json!([{
            "type": "tool_result", "tool_use_id": "t1", "content": "ok", "is_error": true
        }]);
        assert_eq!(
            parse_content(&v),
            vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: vec![ContentBlock::Text("ok".into())],
                is_error: true,
            }]
        );
    }

    #[test]
    fn unknown_block_types_are_dropped() {
        let v = json!([{"type": "future_thing", "x": 1}, {"type": "text", "text": "a"}]);
        assert_eq!(parse_content(&v), vec![ContentBlock::Text("a".into())]);
    }

    #[test]
    fn system_accepts_string_or_block_array() {
        assert_eq!(system_text(Some(&json!("你是助手"))), "你是助手");
        let arr = json!([{"type": "text", "text": "一"}, {"type": "text", "text": "二"}]);
        assert_eq!(system_text(Some(&arr)), "一\n\n二");
        assert_eq!(system_text(None), "");
    }

    #[test]
    fn deserializes_a_minimal_request() {
        let req: MessagesRequest = serde_json::from_value(json!({
            "model": "aiterm:sonnet",
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap();
        assert_eq!(req.model, "aiterm:sonnet");
        assert_eq!(req.max_tokens, Some(1024));
        assert_eq!(req.stream, Some(true));
        assert!(req.tools.is_none());
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::anthropic::request`
Expected: 編譯失敗，`cannot find type 'ContentBlock'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/bridge/anthropic/request.rs` 測試區塊之前加入：

```rust
//! 傳入的 Anthropic Messages API 請求。
//!
//! 只解析我們真的會用到的欄位；未知的 content block 型別直接丟棄而不是報錯
//! ——Claude Code 版本更新時新增區塊型別是常態，硬失敗會讓橋接整個不能用。

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesRequest {
    pub model: String,
    /// 字串或 block 陣列，用 [`system_text`] 取出純文字。
    #[serde(default)]
    pub system: Option<Value>,
    pub messages: Vec<InboundMessage>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub tools: Option<Vec<ToolDef>>,
    #[serde(default)]
    pub tool_choice: Option<Value>,
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InboundMessage {
    pub role: String,
    /// 字串或 block 陣列，用 [`parse_content`] 正規化。
    pub content: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThinkingConfig {
    #[serde(default)]
    pub budget_tokens: Option<u32>,
}

/// 正規化後的 content block。`cache_control` 刻意不保留 —— OpenAI 家族不支援，
/// Anthropic 家族走 passthrough 不經過這裡。
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    Text(String),
    Image { media_type: String, data: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult {
        tool_use_id: String,
        content: Vec<ContentBlock>,
        is_error: bool,
    },
    Thinking(String),
}

pub fn parse_content(v: &Value) -> Vec<ContentBlock> {
    match v {
        Value::String(s) => vec![ContentBlock::Text(s.clone())],
        Value::Array(items) => items.iter().filter_map(parse_block).collect(),
        _ => Vec::new(),
    }
}

fn parse_block(v: &Value) -> Option<ContentBlock> {
    let ty = v.get("type")?.as_str()?;
    match ty {
        "text" => Some(ContentBlock::Text(
            v.get("text")?.as_str().unwrap_or_default().to_string(),
        )),
        "thinking" => Some(ContentBlock::Thinking(
            v.get("thinking")?.as_str().unwrap_or_default().to_string(),
        )),
        "image" => {
            let source = v.get("source")?;
            Some(ContentBlock::Image {
                media_type: source.get("media_type")?.as_str()?.to_string(),
                data: source.get("data")?.as_str()?.to_string(),
            })
        }
        "tool_use" => Some(ContentBlock::ToolUse {
            id: v.get("id")?.as_str()?.to_string(),
            name: v.get("name")?.as_str()?.to_string(),
            input: v.get("input").cloned().unwrap_or(Value::Object(Default::default())),
        }),
        "tool_result" => Some(ContentBlock::ToolResult {
            tool_use_id: v.get("tool_use_id")?.as_str()?.to_string(),
            content: v.get("content").map(parse_content).unwrap_or_default(),
            is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        }),
        _ => None,
    }
}

/// 把 `system` 欄位攤平成一段純文字。
pub fn system_text(v: Option<&Value>) -> String {
    let Some(v) = v else { return String::new() };
    match v {
        Value::String(s) => s.clone(),
        Value::Array(_) => parse_content(v)
            .into_iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}
```

建立 `src-tauri/src/bridge/anthropic/mod.rs`：

```rust
//! Anthropic Messages API 的請求解析與回應序列化。

pub mod request;
```

在 `src-tauri/src/bridge/mod.rs` 加入：

```rust
pub mod anthropic;
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::anthropic::request`
Expected: 8 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bridge/anthropic/ src-tauri/src/bridge/mod.rs
git commit -m "feat(bridge): Anthropic Messages 請求解析與 content block 正規化"
```

---

### Task 6: 中立事件型別與上游 trait

**Files:**
- Create: `src-tauri/src/bridge/upstream/mod.rs`
- Modify: `src-tauri/src/bridge/mod.rs`

這個任務沒有行為可測（純型別定義），所以不寫測試，靠 `cargo check` 驗證。

- [ ] **Step 1: 實作**

建立 `src-tauri/src/bridge/upstream/mod.rs`：

```rust
//! 上游 adapter。
//!
//! 三條路徑收斂到 [`UpstreamEvent`]，Anthropic SSE 序列化器因此只需要寫一份；
//! 新增一個上游 = 寫一支 adapter，序列化端零改動。
//!
//! Anthropic 家族是特例：它本來就講同一個協定，解析再重組是純粹的損耗，
//! 所以走 [`UpstreamResponse::Passthrough`] 直接 pipe。

use async_trait::async_trait;
use futures_util::stream::BoxStream;

use crate::ai::AiError;
use crate::bridge::anthropic::request::MessagesRequest;

#[derive(Debug, Clone, PartialEq)]
pub enum UpstreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    /// 工具名稱確定後才發 —— Anthropic 的 `content_block_start` 就要帶 name，
    /// 而 OpenAI 是 name 先到、arguments 分片後到。
    ToolUseStart { id: String, name: String },
    /// 工具參數的 partial JSON 片段，不需要是完整的 JSON。
    ToolInputDelta(String),
    ToolUseEnd,
    Done { stop_reason: StopReason, usage: Usage },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            StopReason::EndTurn => "end_turn",
            StopReason::MaxTokens => "max_tokens",
            StopReason::ToolUse => "tool_use",
            StopReason::StopSequence => "stop_sequence",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub enum UpstreamResponse {
    /// SSE 原樣轉發，不解析。
    Passthrough(reqwest::Response),
    Events(BoxStream<'static, Result<UpstreamEvent, AiError>>),
}

#[async_trait]
pub trait BridgeUpstream: Send + Sync {
    /// `model` 是映射後的上游模型名，不是 Claude Code 送來的哨兵字串。
    async fn send(
        &self,
        req: &MessagesRequest,
        model: &str,
    ) -> Result<UpstreamResponse, AiError>;
}
```

在 `src-tauri/src/bridge/mod.rs` 加入：

```rust
pub mod upstream;
```

- [ ] **Step 2: 驗證編譯**

Run: `cd src-tauri && cargo check`
Expected: 成功（可能有 `unused` 警告，正常）。

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/bridge/upstream/mod.rs src-tauri/src/bridge/mod.rs
git commit -m "feat(bridge): 中立上游事件型別與 BridgeUpstream trait"
```

---

### Task 7: Anthropic SSE 序列化器

**Files:**
- Create: `src-tauri/src/bridge/anthropic/response.rs`
- Modify: `src-tauri/src/bridge/anthropic/mod.rs`

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/src/bridge/anthropic/response.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::upstream::{StopReason, Usage};

    fn encoder() -> SseEncoder {
        SseEncoder::new("msg_test".into(), "aiterm:sonnet".into())
    }

    /// 從一串 frame 裡抽出 `event:` 行的名稱，方便斷言事件序列。
    fn names(frames: &[String]) -> Vec<String> {
        frames
            .iter()
            .filter_map(|f| f.lines().next())
            .map(|l| l.trim_start_matches("event: ").to_string())
            .collect()
    }

    #[test]
    fn ping_frame_is_a_real_event_not_a_comment() {
        // Claude Code 在等上游時遇到 SSE 靜默會斷線，而 SSE 註解（": ping"）
        // 無效，必須是完整的 event frame。
        let f = ping_frame();
        assert!(f.starts_with("event: ping\n"), "實際：{f}");
        assert!(f.contains("\"type\":\"ping\""));
        assert!(f.ends_with("\n\n"));
    }

    #[test]
    fn start_emits_message_start_only() {
        let mut e = encoder();
        assert_eq!(names(&e.start()), vec!["message_start"]);
    }

    #[test]
    fn text_delta_opens_a_block_then_reuses_it() {
        let mut e = encoder();
        e.start();
        let first = e.push(UpstreamEvent::TextDelta("你".into()));
        assert_eq!(names(&first), vec!["content_block_start", "content_block_delta"]);
        let second = e.push(UpstreamEvent::TextDelta("好".into()));
        assert_eq!(names(&second), vec!["content_block_delta"]);
        assert!(second[0].contains("\"text_delta\""));
    }

    #[test]
    fn switching_from_text_to_tool_closes_the_text_block() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::TextDelta("先講話".into()));
        let frames = e.push(UpstreamEvent::ToolUseStart {
            id: "toolu_1".into(),
            name: "Read".into(),
        });
        assert_eq!(names(&frames), vec!["content_block_stop", "content_block_start"]);
        assert!(frames[1].contains("\"tool_use\""));
        assert!(frames[1].contains("\"Read\""));
    }

    #[test]
    fn tool_input_delta_uses_input_json_delta() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::ToolUseStart { id: "t1".into(), name: "Read".into() });
        let frames = e.push(UpstreamEvent::ToolInputDelta("{\"a\":".into()));
        assert_eq!(names(&frames), vec!["content_block_delta"]);
        assert!(frames[0].contains("\"input_json_delta\""));
    }

    #[test]
    fn thinking_delta_uses_thinking_block() {
        let mut e = encoder();
        e.start();
        let frames = e.push(UpstreamEvent::ThinkingDelta("嗯".into()));
        assert!(frames[0].contains("\"thinking\""));
        assert!(frames[1].contains("\"thinking_delta\""));
    }

    #[test]
    fn done_closes_open_block_then_emits_delta_and_stop() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::TextDelta("hi".into()));
        let frames = e.push(UpstreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 10, output_tokens: 3 },
        });
        assert_eq!(
            names(&frames),
            vec!["content_block_stop", "message_delta", "message_stop"]
        );
        assert!(frames[1].contains("\"tool_use\""));
        assert!(frames[1].contains("\"output_tokens\":3"));
    }

    #[test]
    fn done_without_any_block_skips_the_stop() {
        let mut e = encoder();
        e.start();
        let frames = e.push(UpstreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
        });
        assert_eq!(names(&frames), vec!["message_delta", "message_stop"]);
    }

    #[test]
    fn block_indices_increment() {
        let mut e = encoder();
        e.start();
        e.push(UpstreamEvent::TextDelta("a".into()));
        let frames = e.push(UpstreamEvent::ToolUseStart { id: "t".into(), name: "N".into() });
        assert!(frames[1].contains("\"index\":1"), "第二個區塊的 index 應為 1：{}", frames[1]);
    }

    #[test]
    fn error_frame_matches_anthropic_shape() {
        let f = error_frame("invalid_request_error", "壞掉了");
        assert!(f.starts_with("event: error\n"));
        assert!(f.contains("\"invalid_request_error\""));
        assert!(f.contains("壞掉了"));
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::anthropic::response`
Expected: 編譯失敗，`cannot find type 'SseEncoder'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/bridge/anthropic/response.rs` 測試區塊之前加入：

```rust
//! 把 [`UpstreamEvent`] 序列化成 Anthropic SSE frame。
//!
//! 這是整條翻譯管線唯一的輸出端 —— 每條上游路徑都收斂到 `UpstreamEvent`，
//! 事件序列的正確性只需要在這裡驗證一次。

use serde_json::{json, Value};

use crate::bridge::upstream::{StopReason, UpstreamEvent, Usage};

/// 一個完整的 SSE frame：`event: X\ndata: {...}\n\n`。
fn frame(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// Claude Code 在等上游第一個 byte 期間遇到靜默會斷線，且 SSE 註解
/// （`: ping`）不算資料。必須送完整的 event frame。
pub fn ping_frame() -> String {
    frame("ping", json!({"type": "ping"}))
}

pub fn error_frame(kind: &str, message: &str) -> String {
    frame(
        "error",
        json!({"type": "error", "error": {"type": kind, "message": message}}),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenBlock {
    Text,
    Thinking,
    ToolUse,
}

pub struct SseEncoder {
    message_id: String,
    model: String,
    next_index: usize,
    open: Option<OpenBlock>,
}

impl SseEncoder {
    pub fn new(message_id: String, model: String) -> Self {
        Self { message_id, model, next_index: 0, open: None }
    }

    pub fn start(&mut self) -> Vec<String> {
        vec![frame(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": self.message_id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": Value::Null,
                    "stop_sequence": Value::Null,
                    "usage": {"input_tokens": 0, "output_tokens": 0},
                }
            }),
        )]
    }

    pub fn push(&mut self, ev: UpstreamEvent) -> Vec<String> {
        match ev {
            UpstreamEvent::TextDelta(t) => {
                let mut out = self.ensure_block(OpenBlock::Text, json!({"type": "text", "text": ""}));
                out.push(self.delta(json!({"type": "text_delta", "text": t})));
                out
            }
            UpstreamEvent::ThinkingDelta(t) => {
                let mut out = self.ensure_block(
                    OpenBlock::Thinking,
                    json!({"type": "thinking", "thinking": ""}),
                );
                out.push(self.delta(json!({"type": "thinking_delta", "thinking": t})));
                out
            }
            UpstreamEvent::ToolUseStart { id, name } => {
                // 工具區塊一律開新的：同一個回合可能連續呼叫多個工具。
                let mut out = self.close_open();
                out.push(self.open_block(
                    OpenBlock::ToolUse,
                    json!({"type": "tool_use", "id": id, "name": name, "input": {}}),
                ));
                out
            }
            UpstreamEvent::ToolInputDelta(partial) => {
                vec![self.delta(json!({"type": "input_json_delta", "partial_json": partial}))]
            }
            UpstreamEvent::ToolUseEnd => self.close_open(),
            UpstreamEvent::Done { stop_reason, usage } => {
                let mut out = self.close_open();
                out.push(self.message_delta(stop_reason, usage));
                out.push(frame("message_stop", json!({"type": "message_stop"})));
                out
            }
        }
    }

    /// 目前開著的若已是同型別區塊就沿用，否則關掉舊的再開新的。
    fn ensure_block(&mut self, kind: OpenBlock, body: Value) -> Vec<String> {
        if self.open == Some(kind) {
            return Vec::new();
        }
        let mut out = self.close_open();
        out.push(self.open_block(kind, body));
        out
    }

    fn open_block(&mut self, kind: OpenBlock, body: Value) -> String {
        let index = self.next_index;
        self.next_index += 1;
        self.open = Some(kind);
        frame(
            "content_block_start",
            json!({"type": "content_block_start", "index": index, "content_block": body}),
        )
    }

    fn close_open(&mut self) -> Vec<String> {
        if self.open.take().is_none() {
            return Vec::new();
        }
        vec![frame(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": self.current_index()}),
        )]
    }

    fn delta(&self, body: Value) -> String {
        frame(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": self.current_index(), "delta": body}),
        )
    }

    /// 目前（或剛關閉的）區塊索引。`next_index` 永遠指向下一個。
    fn current_index(&self) -> usize {
        self.next_index.saturating_sub(1)
    }

    fn message_delta(&self, stop_reason: StopReason, usage: Usage) -> String {
        frame(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_reason.as_str(), "stop_sequence": Value::Null},
                "usage": {
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                }
            }),
        )
    }
}
```

在 `src-tauri/src/bridge/anthropic/mod.rs` 加入：

```rust
pub mod response;
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::anthropic::response`
Expected: 10 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bridge/anthropic/
git commit -m "feat(bridge): Anthropic SSE 序列化器"
```

---

## 階段 C：OpenAI 路徑

### Task 8: Anthropic → OpenAI 請求翻譯

**Files:**
- Create: `src-tauri/src/bridge/upstream/openai/request.rs`
- Create: `src-tauri/src/bridge/upstream/openai/mod.rs`
- Modify: `src-tauri/src/bridge/upstream/mod.rs`

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/src/bridge/upstream/openai/request.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(v: serde_json::Value) -> MessagesRequest {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn system_becomes_first_message() {
        let body = build_body(
            &req(json!({
                "model": "aiterm:sonnet", "system": "你是助手",
                "messages": [{"role": "user", "content": "hi"}]
            })),
            "qwen",
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "你是助手");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn empty_system_is_omitted() {
        let body = build_body(
            &req(json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]})),
            "qwen",
        );
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn model_and_stream_are_set() {
        let body = build_body(
            &req(json!({"model": "aiterm:opus", "messages": [{"role":"user","content":"x"}]})),
            "gpt-4o",
        );
        assert_eq!(body["model"], "gpt-4o", "要用映射後的模型名，不是哨兵字串");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn image_block_becomes_data_uri() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": "看"},
                    {"type": "image", "source": {"type":"base64","media_type":"image/png","data":"AAAA"}}
                ]}]
            })),
            "gpt-4o",
        );
        let parts = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn plain_text_content_stays_a_string() {
        // 沒有圖片時不要包成陣列，某些相容 server 只吃字串。
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]})),
            "m",
        );
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn tool_use_becomes_assistant_tool_calls() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"p": 1}}
                ]}]
            })),
            "m",
        );
        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["tool_calls"][0]["id"], "toolu_1");
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "Read");
        // arguments 必須是 JSON 字串，不是物件。
        assert_eq!(msg["tool_calls"][0]["function"]["arguments"], "{\"p\":1}");
    }

    #[test]
    fn tool_result_becomes_a_tool_message() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1",
                     "content": [{"type": "text", "text": "檔案內容"}]}
                ]}]
            })),
            "m",
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "tool");
        assert_eq!(msgs[0]["tool_call_id"], "toolu_1");
        assert_eq!(msgs[0]["content"], "檔案內容");
    }

    #[test]
    fn image_inside_tool_result_is_lifted_to_a_following_user_turn() {
        // OpenAI 的 tool 訊息不能帶圖片，圖片必須提到後面的 user turn。
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": [
                        {"type": "text", "text": "截圖如下"},
                        {"type": "image", "source": {"type":"base64","media_type":"image/png","data":"BBBB"}}
                    ]}
                ]}]
            })),
            "m",
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2, "應該多出一個 user turn 承載圖片");
        assert_eq!(msgs[0]["role"], "tool");
        assert_eq!(msgs[0]["content"], "截圖如下");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"][0]["image_url"]["url"], "data:image/png;base64,BBBB");
    }

    #[test]
    fn multiple_tool_results_in_one_turn_become_separate_messages() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "a"},
                    {"type": "tool_result", "tool_use_id": "t2", "content": "b"}
                ]}]
            })),
            "m",
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["tool_call_id"], "t1");
        assert_eq!(msgs[1]["tool_call_id"], "t2");
    }

    #[test]
    fn tools_are_translated_to_function_defs() {
        let body = build_body(
            &req(json!({
                "model": "m",
                "messages": [{"role":"user","content":"x"}],
                "tools": [{"name": "Read", "description": "讀檔",
                           "input_schema": {"type":"object","properties":{}}}]
            })),
            "m",
        );
        let t = &body["tools"][0];
        assert_eq!(t["type"], "function");
        assert_eq!(t["function"]["name"], "Read");
        assert_eq!(t["function"]["description"], "讀檔");
        assert_eq!(t["function"]["parameters"]["type"], "object");
        assert_eq!(body["tool_choice"], "auto", "沒指定時預設 auto");
    }

    #[test]
    fn tool_choice_is_translated_not_hardcoded() {
        let mk = |tc: serde_json::Value| {
            build_body(
                &req(json!({
                    "model": "m", "messages": [{"role":"user","content":"x"}],
                    "tools": [{"name":"Read","input_schema":{}}],
                    "tool_choice": tc
                })),
                "m",
            )["tool_choice"]
                .clone()
        };
        assert_eq!(mk(json!({"type": "auto"})), json!("auto"));
        assert_eq!(mk(json!({"type": "any"})), json!("required"));
        assert_eq!(mk(json!({"type": "none"})), json!("none"));
        assert_eq!(
            mk(json!({"type": "tool", "name": "Read"})),
            json!({"type": "function", "function": {"name": "Read"}})
        );
    }

    #[test]
    fn no_tools_means_no_tools_field() {
        let body = build_body(
            &req(json!({"model":"m","messages":[{"role":"user","content":"x"}]})),
            "m",
        );
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn sampling_params_pass_through() {
        let body = build_body(
            &req(json!({
                "model":"m","messages":[{"role":"user","content":"x"}],
                "max_tokens": 4096, "temperature": 0.3, "stop_sequences": ["END"]
            })),
            "m",
        );
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["stop"][0], "END");
    }

    #[test]
    fn thinking_budget_maps_to_reasoning_effort() {
        let mk = |budget: u32| {
            build_body(
                &req(json!({
                    "model":"m","messages":[{"role":"user","content":"x"}],
                    "thinking": {"type":"enabled","budget_tokens": budget}
                })),
                "m",
            )["reasoning_effort"]
                .clone()
        };
        assert_eq!(mk(1000), "low");
        assert_eq!(mk(8000), "medium");
        assert_eq!(mk(30000), "high");
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::upstream::openai::request`
Expected: 編譯失敗，`cannot find function 'build_body'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/bridge/upstream/openai/request.rs` 測試區塊之前加入：

```rust
//! Anthropic Messages 請求 → OpenAI chat.completions 請求。

use serde_json::{json, Map, Value};

use crate::bridge::anthropic::request::{
    parse_content, system_text, ContentBlock, MessagesRequest,
};

pub fn build_body(req: &MessagesRequest, model: &str) -> Value {
    let mut messages: Vec<Value> = Vec::new();

    let system = system_text(req.system.as_ref());
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system}));
    }
    for m in &req.messages {
        push_message(&mut messages, &m.role, &parse_content(&m.content));
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("messages".into(), Value::Array(messages));
    body.insert("stream".into(), json!(true));
    // 沒有這個旗標，多數 OpenAI 相容端點的串流回應不會帶 usage，
    // Claude Code 的 token 計數就會一直是 0。
    body.insert("stream_options".into(), json!({"include_usage": true}));

    if let Some(v) = req.max_tokens {
        body.insert("max_tokens".into(), json!(v));
    }
    if let Some(v) = req.temperature {
        body.insert("temperature".into(), json!(v));
    }
    if let Some(v) = &req.stop_sequences {
        if !v.is_empty() {
            body.insert("stop".into(), json!(v));
        }
    }
    if let Some(t) = &req.thinking {
        if let Some(budget) = t.budget_tokens {
            body.insert("reasoning_effort".into(), json!(reasoning_effort(budget)));
        }
    }
    if let Some(tools) = &req.tools {
        if !tools.is_empty() {
            let defs: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description.clone().unwrap_or_default(),
                            "parameters": t.input_schema,
                        }
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(defs));
            body.insert("tool_choice".into(), map_tool_choice(req.tool_choice.as_ref()));
        }
    }

    Value::Object(body)
}

/// Anthropic 的 `tool_choice` 是物件，OpenAI 大多是字串（指定工具時才是物件）。
///
/// 不能寫死 `"auto"`：Claude Code 在某些子代理流程會送 `{"type":"none"}` 來
/// 禁止工具呼叫，寫死會讓模型在不該用工具的回合硬呼叫工具。
fn map_tool_choice(tc: Option<&Value>) -> Value {
    let Some(tc) = tc else { return json!("auto") };
    match tc.get("type").and_then(Value::as_str) {
        Some("any") => json!("required"),
        Some("none") => json!("none"),
        Some("tool") => match tc.get("name").and_then(Value::as_str) {
            Some(name) => json!({"type": "function", "function": {"name": name}}),
            None => json!("auto"),
        },
        _ => json!("auto"),
    }
}

/// Anthropic 的 thinking budget 是 token 數，OpenAI 是三段式。粗略對應即可
/// ——這個欄位只影響推理深度，不影響正確性。
fn reasoning_effort(budget: u32) -> &'static str {
    if budget < 4096 {
        "low"
    } else if budget < 16384 {
        "medium"
    } else {
        "high"
    }
}

/// 把一個 Anthropic turn 攤成一或多個 OpenAI 訊息。
///
/// 一個 turn 可能含多個 `tool_result`，OpenAI 要求每個結果各自一則 `tool`
/// 訊息；而 `tool` 訊息不能帶圖片，所以圖片會被提到後面新增的 user turn。
fn push_message(out: &mut Vec<Value>, role: &str, blocks: &[ContentBlock]) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut images: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    let mut lifted_images: Vec<Value> = Vec::new();

    for b in blocks {
        match b {
            ContentBlock::Text(t) => text_parts.push(t.clone()),
            // thinking 區塊不回送給上游：它是模型自己的產出，重送只是浪費 token。
            ContentBlock::Thinking(_) => {}
            ContentBlock::Image { media_type, data } => images.push(image_part(media_type, data)),
            ContentBlock::ToolUse { id, name, input } => tool_calls.push(json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    // OpenAI 的 arguments 是 JSON 字串，不是物件。
                    "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
                }
            })),
            ContentBlock::ToolResult { tool_use_id, content, .. } => {
                let mut result_text: Vec<String> = Vec::new();
                for inner in content {
                    match inner {
                        ContentBlock::Text(t) => result_text.push(t.clone()),
                        ContentBlock::Image { media_type, data } => {
                            lifted_images.push(image_part(media_type, data))
                        }
                        _ => {}
                    }
                }
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": result_text.join("\n"),
                }));
            }
        }
    }

    let has_own_content = !text_parts.is_empty() || !images.is_empty() || !tool_calls.is_empty();
    if has_own_content {
        let mut msg = Map::new();
        msg.insert("role".into(), json!(role));
        msg.insert("content".into(), content_value(&text_parts, &images));
        if !tool_calls.is_empty() {
            msg.insert("tool_calls".into(), Value::Array(tool_calls));
        }
        out.push(Value::Object(msg));
    }

    if !lifted_images.is_empty() {
        out.push(json!({"role": "user", "content": Value::Array(lifted_images)}));
    }
}

/// 沒有圖片時回純字串 —— 部分 OpenAI 相容 server 不接受 content 陣列。
fn content_value(text_parts: &[String], images: &[Value]) -> Value {
    if images.is_empty() {
        return json!(text_parts.join("\n"));
    }
    let mut parts: Vec<Value> = Vec::new();
    let joined = text_parts.join("\n");
    if !joined.is_empty() {
        parts.push(json!({"type": "text", "text": joined}));
    }
    parts.extend(images.iter().cloned());
    Value::Array(parts)
}

fn image_part(media_type: &str, data: &str) -> Value {
    json!({
        "type": "image_url",
        "image_url": {"url": format!("data:{media_type};base64,{data}")}
    })
}
```

建立 `src-tauri/src/bridge/upstream/openai/mod.rs`：

```rust
//! OpenAI chat.completions 家族的 adapter。
//!
//! 覆蓋 openai / openai-compatible / ollama / openrouter / deepseek / kimi /
//! xai / github-copilot，以及 API key 模式的 google-ai。

pub mod request;
```

在 `src-tauri/src/bridge/upstream/mod.rs` 加入：

```rust
pub mod openai;
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::upstream::openai::request`
Expected: 14 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bridge/upstream/
git commit -m "feat(bridge): Anthropic → OpenAI 請求翻譯"
```

---

### Task 9: 串流 tool_calls 累積器

這是 M1 最容易寫錯的一塊。`ai/sse.rs:120` 的 `OpenAiSseDelta` 只宣告 `content`，串流的工具呼叫目前是被整包丟掉的 —— 這裡從零建立。

**Files:**
- Create: `src-tauri/src/bridge/upstream/openai/tool_calls.rs`
- Modify: `src-tauri/src/bridge/upstream/openai/mod.rs`

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/src/bridge/upstream/openai/tool_calls.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn acc() -> ToolCallAccumulator {
        ToolCallAccumulator::default()
    }

    #[test]
    fn name_and_args_in_one_chunk() {
        let mut a = acc();
        let ev = a.push(&[json!({
            "index": 0, "id": "call_1",
            "function": {"name": "Read", "arguments": "{\"p\":1}"}
        })]);
        assert_eq!(
            ev,
            vec![
                UpstreamEvent::ToolUseStart { id: "call_1".into(), name: "Read".into() },
                UpstreamEvent::ToolInputDelta("{\"p\":1}".into()),
            ]
        );
    }

    #[test]
    fn start_is_deferred_until_the_name_arrives() {
        // 有些 server 先送 id、名稱下一片才到。此時不能發 ToolUseStart，
        // 因為 Anthropic 的 content_block_start 就要帶 name。
        let mut a = acc();
        assert_eq!(a.push(&[json!({"index": 0, "id": "call_1"})]), vec![]);
        assert_eq!(
            a.push(&[json!({"index": 0, "function": {"name": "Read"}})]),
            vec![UpstreamEvent::ToolUseStart { id: "call_1".into(), name: "Read".into() }]
        );
    }

    #[test]
    fn name_split_across_chunks_is_concatenated() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "Re"}})]);
        // 名稱補完之前不該發事件；補完後用完整名稱。
        let ev = a.push(&[json!({"index": 0, "function": {"arguments": "{}"}})]);
        assert_eq!(
            ev,
            vec![
                UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Re".into() },
                UpstreamEvent::ToolInputDelta("{}".into()),
            ],
            "arguments 一到就代表名稱已完結，此時才 flush"
        );
    }

    #[test]
    fn argument_fragments_are_forwarded_verbatim() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "N", "arguments": "{\"a\""}})]);
        let ev = a.push(&[json!({"index": 0, "function": {"arguments": ":1}"}})]);
        assert_eq!(ev, vec![UpstreamEvent::ToolInputDelta(":1}".into())]);
    }

    #[test]
    fn empty_argument_fragment_emits_nothing() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "N", "arguments": ""}})]);
        assert_eq!(a.push(&[json!({"index": 0, "function": {"arguments": ""}})]), vec![]);
    }

    #[test]
    fn switching_index_closes_the_previous_tool() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "A", "arguments": "{}"}})]);
        let ev = a.push(&[json!({"index": 1, "id": "c2", "function": {"name": "B", "arguments": "{}"}})]);
        assert_eq!(
            ev,
            vec![
                UpstreamEvent::ToolUseEnd,
                UpstreamEvent::ToolUseStart { id: "c2".into(), name: "B".into() },
                UpstreamEvent::ToolInputDelta("{}".into()),
            ]
        );
    }

    #[test]
    fn two_tools_in_one_chunk_are_sequenced() {
        let mut a = acc();
        let ev = a.push(&[
            json!({"index": 0, "id": "c1", "function": {"name": "A", "arguments": "{}"}}),
            json!({"index": 1, "id": "c2", "function": {"name": "B", "arguments": "{}"}}),
        ]);
        assert_eq!(
            ev,
            vec![
                UpstreamEvent::ToolUseStart { id: "c1".into(), name: "A".into() },
                UpstreamEvent::ToolInputDelta("{}".into()),
                UpstreamEvent::ToolUseEnd,
                UpstreamEvent::ToolUseStart { id: "c2".into(), name: "B".into() },
                UpstreamEvent::ToolInputDelta("{}".into()),
            ]
        );
    }

    #[test]
    fn finish_closes_the_open_tool() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "A", "arguments": "{}"}})]);
        assert_eq!(a.finish(), vec![UpstreamEvent::ToolUseEnd]);
        assert_eq!(a.finish(), vec![], "重複呼叫不應再發事件");
    }

    #[test]
    fn finish_flushes_a_tool_that_never_got_arguments() {
        let mut a = acc();
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "NoArgs"}})]);
        assert_eq!(
            a.finish(),
            vec![
                UpstreamEvent::ToolUseStart { id: "c1".into(), name: "NoArgs".into() },
                UpstreamEvent::ToolUseEnd,
            ],
            "無參數的工具呼叫不能被吞掉"
        );
    }

    #[test]
    fn missing_index_defaults_to_zero() {
        // 少數相容端點不送 index。
        let mut a = acc();
        let ev = a.push(&[json!({"id": "c1", "function": {"name": "A", "arguments": "{}"}})]);
        assert_eq!(ev.len(), 2);
    }

    #[test]
    fn reports_whether_any_tool_was_seen() {
        let mut a = acc();
        assert!(!a.saw_any());
        a.push(&[json!({"index": 0, "id": "c1", "function": {"name": "A"}})]);
        assert!(a.saw_any());
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::upstream::openai::tool_calls`
Expected: 編譯失敗，`cannot find type 'ToolCallAccumulator'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/bridge/upstream/openai/tool_calls.rs` 測試區塊之前加入：

```rust
//! OpenAI 串流工具呼叫的累積器。
//!
//! OpenAI 把一個工具呼叫拆成多個 `delta.tool_calls[]` 片段：`id` 與
//! `function.name` 通常先到，`function.arguments` 分很多片後到。Anthropic 的
//! `content_block_start` 卻要求一開始就帶完整的 name，所以必須緩衝到名稱
//! 確定才發第一個事件。
//!
//! 「名稱確定」的判準：收到第一個非空的 arguments 片段，或串流結束。

use std::collections::BTreeMap;

use serde_json::Value;

use crate::bridge::upstream::UpstreamEvent;

#[derive(Debug, Default)]
struct Slot {
    id: String,
    name: String,
    started: bool,
}

#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    slots: BTreeMap<u64, Slot>,
    /// 目前已發出 `ToolUseStart` 但還沒 `ToolUseEnd` 的槽位。
    active: Option<u64>,
    saw_any: bool,
}

impl ToolCallAccumulator {
    /// 餵進一個 SSE chunk 的 `delta.tool_calls` 陣列，回傳要發出的事件。
    pub fn push(&mut self, tool_calls: &[Value]) -> Vec<UpstreamEvent> {
        let mut out = Vec::new();
        for tc in tool_calls {
            let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
            self.saw_any = true;

            let slot = self.slots.entry(index).or_default();
            if let Some(id) = tc.get("id").and_then(Value::as_str) {
                if !id.is_empty() {
                    slot.id = id.to_string();
                }
            }
            if let Some(name) = tc.get("function").and_then(|f| f.get("name")).and_then(Value::as_str) {
                slot.name.push_str(name);
            }

            let args = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or_default();

            if args.is_empty() {
                continue;
            }
            // 有參數進來就代表名稱已經完結，可以開區塊了。
            out.extend(self.activate(index));
            out.push(UpstreamEvent::ToolInputDelta(args.to_string()));
        }
        out
    }

    /// 串流結束時呼叫：把還沒開的槽位補開、關掉開著的區塊。
    pub fn finish(&mut self) -> Vec<UpstreamEvent> {
        let mut out = Vec::new();
        let pending: Vec<u64> = self
            .slots
            .iter()
            .filter(|(_, s)| !s.started)
            .map(|(i, _)| *i)
            .collect();
        for index in pending {
            out.extend(self.activate(index));
        }
        out.extend(self.close_active());
        out
    }

    pub fn saw_any(&self) -> bool {
        self.saw_any
    }

    /// 讓 `index` 成為開著的區塊：先關掉別的，需要時發 `ToolUseStart`。
    fn activate(&mut self, index: u64) -> Vec<UpstreamEvent> {
        if self.active == Some(index) {
            return Vec::new();
        }
        let mut out = self.close_active();
        let Some(slot) = self.slots.get_mut(&index) else {
            return out;
        };
        if !slot.started {
            slot.started = true;
            out.push(UpstreamEvent::ToolUseStart {
                id: slot.id.clone(),
                name: slot.name.clone(),
            });
        }
        self.active = Some(index);
        out
    }

    fn close_active(&mut self) -> Vec<UpstreamEvent> {
        if self.active.take().is_some() {
            vec![UpstreamEvent::ToolUseEnd]
        } else {
            Vec::new()
        }
    }
}
```

在 `src-tauri/src/bridge/upstream/openai/mod.rs` 加入：

```rust
pub mod tool_calls;
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::upstream::openai::tool_calls`
Expected: 11 passed。

若 `finish_flushes_a_tool_that_never_got_arguments` 失敗且輸出多了一個 `ToolUseEnd`，檢查 `activate` 在 `close_active` 之後才查 slot —— 順序反了會對已關閉的槽位重複發事件。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bridge/upstream/openai/
git commit -m "feat(bridge): OpenAI 串流工具呼叫累積器"
```

---

### Task 10: OpenAI SSE → UpstreamEvent

**Files:**
- Modify: `src-tauri/src/ai/sse.rs`（把行切分工具提為 `pub(crate)`）
- Create: `src-tauri/src/bridge/upstream/openai/stream.rs`
- Modify: `src-tauri/src/bridge/upstream/openai/mod.rs`

- [ ] **Step 1: 把既有的行切分工具提為 `pub(crate)`**

在 `src-tauri/src/ai/sse.rs` 找到 `fn find_line_end` 與 `fn separator_len` 兩個私有函式，各自把 `fn` 改成 `pub(crate) fn`。不要複製一份到 bridge —— SSE 的斷行處理（`\n` 與 `\r\n` 混用、跨 chunk 的半行）是踩過的坑，兩份實作遲早分歧。

Run: `cd src-tauri && cargo check`
Expected: 成功。

- [ ] **Step 2: 寫失敗測試**

建立 `src-tauri/src/bridge/upstream/openai/stream.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 把幾個 SSE 資料行餵進解析器，收集全部事件。
    fn run(lines: &[&str]) -> Vec<UpstreamEvent> {
        let mut p = StreamParser::default();
        let mut out = Vec::new();
        for l in lines {
            out.extend(p.feed_line(l));
        }
        out.extend(p.finish());
        out
    }

    #[test]
    fn text_deltas_are_forwarded() {
        let ev = run(&[
            r#"data: {"choices":[{"delta":{"content":"你"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"好"}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("你".into()));
        assert_eq!(ev[1], UpstreamEvent::TextDelta("好".into()));
    }

    #[test]
    fn non_data_lines_are_ignored() {
        let ev = run(&["", ": comment", "event: whatever", "data: [DONE]"]);
        assert!(matches!(ev.as_slice(), [UpstreamEvent::Done { .. }]));
    }

    #[test]
    fn malformed_json_is_skipped_not_fatal() {
        let ev = run(&[
            "data: {not json",
            r#"data: {"choices":[{"delta":{"content":"ok"}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(ev[0], UpstreamEvent::TextDelta("ok".into()));
    }

    #[test]
    fn reasoning_content_becomes_thinking_delta() {
        let ev = run(&[
            r#"data: {"choices":[{"delta":{"reasoning_content":"嗯"}}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(ev[0], UpstreamEvent::ThinkingDelta("嗯".into()));
    }

    #[test]
    fn tool_calls_produce_tool_events() {
        let ev = run(&[
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"Read","arguments":"{}"}}]}}]}"#,
            r#"data: {"choices":[{"finish_reason":"tool_calls"}]}"#,
            "data: [DONE]",
        ]);
        assert_eq!(ev[0], UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Read".into() });
        assert_eq!(ev[1], UpstreamEvent::ToolInputDelta("{}".into()));
        assert_eq!(ev[2], UpstreamEvent::ToolUseEnd);
        assert!(matches!(
            ev[3],
            UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }
        ));
    }

    #[test]
    fn finish_reason_maps_to_stop_reason() {
        let mk = |fr: &str| {
            let line = format!(r#"data: {{"choices":[{{"finish_reason":"{fr}"}}]}}"#);
            match run(&[&line, "data: [DONE]"]).into_iter().next().unwrap() {
                UpstreamEvent::Done { stop_reason, .. } => stop_reason,
                other => panic!("預期 Done，實際 {other:?}"),
            }
        };
        assert_eq!(mk("stop"), StopReason::EndTurn);
        assert_eq!(mk("length"), StopReason::MaxTokens);
        assert_eq!(mk("tool_calls"), StopReason::ToolUse);
        assert_eq!(mk("content_filter"), StopReason::EndTurn);
    }

    #[test]
    fn usage_is_captured_even_when_it_arrives_after_finish_reason() {
        // stream_options.include_usage 會讓 usage 出現在最後一個 chunk，
        // 也就是 finish_reason 之後。
        let ev = run(&[
            r#"data: {"choices":[{"finish_reason":"stop"}]}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34}}"#,
            "data: [DONE]",
        ]);
        let done = ev.iter().find(|e| matches!(e, UpstreamEvent::Done { .. })).unwrap();
        match done {
            UpstreamEvent::Done { usage, .. } => {
                assert_eq!(usage.input_tokens, 12);
                assert_eq!(usage.output_tokens, 34);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            ev.iter().filter(|e| matches!(e, UpstreamEvent::Done { .. })).count(),
            1,
            "Done 只能發一次"
        );
    }

    #[test]
    fn stream_ending_without_done_still_emits_done() {
        let ev = run(&[r#"data: {"choices":[{"delta":{"content":"a"}}]}"#]);
        assert!(matches!(ev.last(), Some(UpstreamEvent::Done { .. })));
    }
}
```

- [ ] **Step 3: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::upstream::openai::stream`
Expected: 編譯失敗，`cannot find type 'StreamParser'`。

- [ ] **Step 4: 實作**

在 `src-tauri/src/bridge/upstream/openai/stream.rs` 測試區塊之前加入：

```rust
//! OpenAI chat.completions SSE → [`UpstreamEvent`]。
//!
//! 為什麼不用 `ai::sse::consume_openai_sse`：那支的輸出型別是
//! `GenerateChunk { delta: String }`，工具呼叫在型別上就表達不出來。行切分
//! 的工具函式仍然共用。

use serde_json::Value;

use super::tool_calls::ToolCallAccumulator;
use crate::bridge::upstream::{StopReason, UpstreamEvent, Usage};

/// 逐行餵入的 SSE 解析器。呼叫端負責把 byte 串切成行
/// （見 `ai::sse::find_line_end`）。
#[derive(Default)]
pub struct StreamParser {
    tools: ToolCallAccumulator,
    /// 收到 finish_reason 後暫存，等 usage 那一片到了才發 Done。
    pending_stop: Option<StopReason>,
    usage: Usage,
    done_sent: bool,
}

impl StreamParser {
    pub fn feed_line(&mut self, line: &str) -> Vec<UpstreamEvent> {
        let line = line.trim();
        let Some(payload) = line.strip_prefix("data:") else {
            return Vec::new();
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            return self.finish();
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            // 壞掉的一行不該終止整個串流：部分端點會夾雜心跳或非標準行。
            return Vec::new();
        };

        let mut out = Vec::new();
        if let Some(u) = v.get("usage") {
            self.usage = Usage {
                input_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
                output_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            };
        }

        let Some(choice) = v.get("choices").and_then(Value::as_array).and_then(|c| c.first()) else {
            return out;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(t) = delta.get("content").and_then(Value::as_str) {
                if !t.is_empty() {
                    out.push(UpstreamEvent::TextDelta(t.to_string()));
                }
            }
            // DeepSeek 用 reasoning_content，部分相容端點用 reasoning。
            for key in ["reasoning_content", "reasoning"] {
                if let Some(t) = delta.get(key).and_then(Value::as_str) {
                    if !t.is_empty() {
                        out.push(UpstreamEvent::ThinkingDelta(t.to_string()));
                    }
                }
            }
            if let Some(tc) = delta.get("tool_calls").and_then(Value::as_array) {
                out.extend(self.tools.push(tc));
            }
        }
        if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
            self.pending_stop = Some(map_stop_reason(fr));
        }
        out
    }

    /// 串流結束（收到 `[DONE]` 或連線關閉）時呼叫。冪等。
    pub fn finish(&mut self) -> Vec<UpstreamEvent> {
        if self.done_sent {
            return Vec::new();
        }
        self.done_sent = true;
        let mut out = self.tools.finish();
        // 上游若沒給 finish_reason，用工具呼叫的有無來推斷。
        let stop_reason = self.pending_stop.unwrap_or(if self.tools.saw_any() {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        });
        out.push(UpstreamEvent::Done { stop_reason, usage: self.usage });
        out
    }
}

fn map_stop_reason(fr: &str) -> StopReason {
    match fr {
        "length" => StopReason::MaxTokens,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "stop_sequence" => StopReason::StopSequence,
        // content_filter 等其餘一律當正常結束 —— Anthropic 沒有對應的
        // stop_reason，回未知值會讓 Claude Code 解析失敗。
        _ => StopReason::EndTurn,
    }
}
```

在 `src-tauri/src/bridge/upstream/openai/mod.rs` 加入：

```rust
pub mod stream;
```

- [ ] **Step 5: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::upstream::openai::stream`
Expected: 8 passed。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/sse.rs src-tauri/src/bridge/upstream/openai/
git commit -m "feat(bridge): OpenAI SSE 解析成中立上游事件"
```

---

### Task 11: OpenAI adapter 接線

**Files:**
- Create: `src-tauri/src/bridge/upstream/openai/client.rs`
- Modify: `src-tauri/src/bridge/upstream/openai/mod.rs`
- Create: `src-tauri/tests/bridge_openai_upstream.rs`

- [ ] **Step 1: 寫失敗的整合測試**

建立 `src-tauri/tests/bridge_openai_upstream.rs`：

```rust
//! OpenAI 上游 adapter 的端到端測試（假上游）。

use futures_util::StreamExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::anthropic::request::MessagesRequest;
use aiterm_lib::bridge::upstream::openai::client::OpenAiUpstream;
use aiterm_lib::bridge::upstream::{BridgeUpstream, StopReason, UpstreamEvent, UpstreamResponse};

fn req(v: serde_json::Value) -> MessagesRequest {
    serde_json::from_value(v).unwrap()
}

async fn collect(resp: UpstreamResponse) -> Vec<UpstreamEvent> {
    match resp {
        UpstreamResponse::Events(mut s) => {
            let mut out = Vec::new();
            while let Some(item) = s.next().await {
                out.push(item.expect("串流不該出錯"));
            }
            out
        }
        UpstreamResponse::Passthrough(_) => panic!("OpenAI 路徑不應回 Passthrough"),
    }
}

#[tokio::test]
async fn streams_text_and_tool_calls() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"開始\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"Read\",\"arguments\":\"{}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse, "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = OpenAiUpstream::new(server.uri(), "sk-test".into());
    let resp = up
        .send(
            &req(serde_json::json!({
                "model": "aiterm:sonnet",
                "messages": [{"role": "user", "content": "hi"}]
            })),
            "qwen",
        )
        .await
        .unwrap();

    let ev = collect(resp).await;
    assert_eq!(ev[0], UpstreamEvent::TextDelta("開始".into()));
    assert_eq!(ev[1], UpstreamEvent::ToolUseStart { id: "c1".into(), name: "Read".into() });
    assert_eq!(ev[2], UpstreamEvent::ToolInputDelta("{}".into()));
    assert_eq!(ev[3], UpstreamEvent::ToolUseEnd);
    assert!(matches!(ev[4], UpstreamEvent::Done { stop_reason: StopReason::ToolUse, .. }));
}

#[tokio::test]
async fn http_error_is_mapped_to_ai_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("no key"))
        .mount(&server)
        .await;

    let up = OpenAiUpstream::new(server.uri(), "bad".into());
    let err = up
        .send(&req(serde_json::json!({
            "model": "m", "messages": [{"role":"user","content":"x"}]
        })), "m")
        .await
        .unwrap_err();
    assert!(format!("{err:?}").contains("Auth"), "實際：{err:?}");
}

#[tokio::test]
async fn base_url_already_ending_in_v1_is_not_doubled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: [DONE]\n\n", "text/event-stream"))
        .mount(&server)
        .await;

    let up = OpenAiUpstream::new(format!("{}/v1", server.uri()), "k".into());
    let resp = up
        .send(&req(serde_json::json!({
            "model": "m", "messages": [{"role":"user","content":"x"}]
        })), "m")
        .await
        .unwrap();
    // 沒有 panic 就代表打到了 /v1/chat/completions 而不是 /v1/v1/...
    assert!(matches!(collect(resp).await.last(), Some(UpstreamEvent::Done { .. })));
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test bridge_openai_upstream`
Expected: 編譯失敗，`could not find 'client' in 'openai'`。

- [ ] **Step 3: 實作**

建立 `src-tauri/src/bridge/upstream/openai/client.rs`：

```rust
//! OpenAI chat.completions 上游。

use async_trait::async_trait;
use futures_util::StreamExt;

use super::{request::build_body, stream::StreamParser};
use crate::ai::sse::{find_line_end, separator_len};
use crate::ai::AiError;
use crate::bridge::anthropic::request::MessagesRequest;
use crate::bridge::upstream::{BridgeUpstream, UpstreamEvent, UpstreamResponse};

pub struct OpenAiUpstream {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiUpstream {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            client: reqwest::Client::new(),
        }
    }

    /// 使用者填的 base_url 有人帶 `/v1` 有人不帶，兩種都要能用。
    fn completions_url(&self) -> String {
        if self.base_url.ends_with("/v1") {
            format!("{}/chat/completions", self.base_url)
        } else {
            format!("{}/v1/chat/completions", self.base_url)
        }
    }
}

#[async_trait]
impl BridgeUpstream for OpenAiUpstream {
    async fn send(
        &self,
        req: &MessagesRequest,
        model: &str,
    ) -> Result<UpstreamResponse, AiError> {
        let resp = self
            .client
            .post(self.completions_url())
            .bearer_auth(&self.api_key)
            .json(&build_body(req, model))
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(crate::ai::sse::map_http_error(status, &body));
        }

        Ok(UpstreamResponse::Events(Box::pin(into_events(resp))))
    }
}

/// 把 HTTP 回應的 byte 串轉成 [`UpstreamEvent`] 串流。
///
/// 用 `unfold` 而非 `async_stream`：後者要多一個依賴，而狀態機只有三個欄位。
fn into_events(
    resp: reqwest::Response,
) -> impl futures_util::Stream<Item = Result<UpstreamEvent, AiError>> {
    struct State {
        bytes: std::pin::Pin<Box<dyn futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
        buf: Vec<u8>,
        parser: StreamParser,
        queued: std::collections::VecDeque<UpstreamEvent>,
        ended: bool,
    }

    let state = State {
        bytes: Box::pin(resp.bytes_stream()),
        buf: Vec::new(),
        parser: StreamParser::default(),
        queued: std::collections::VecDeque::new(),
        ended: false,
    };

    futures_util::stream::unfold(state, |mut s| async move {
        loop {
            if let Some(ev) = s.queued.pop_front() {
                return Some((Ok(ev), s));
            }
            if s.ended {
                return None;
            }
            // 先把緩衝區裡完整的行處理掉。
            if let Some(pos) = find_line_end(&s.buf) {
                let line_bytes: Vec<u8> = s.buf.drain(..pos).collect();
                let sep = separator_len(&s.buf);
                s.buf.drain(..sep);
                if let Ok(line) = std::str::from_utf8(&line_bytes) {
                    s.queued.extend(s.parser.feed_line(line));
                }
                continue;
            }
            match s.bytes.next().await {
                Some(Ok(chunk)) => s.buf.extend_from_slice(&chunk),
                Some(Err(e)) => {
                    s.ended = true;
                    return Some((Err(AiError::Network { message: e.to_string() }), s));
                }
                None => {
                    s.ended = true;
                    s.queued.extend(s.parser.finish());
                }
            }
        }
    })
}
```

在 `src-tauri/src/bridge/upstream/openai/mod.rs` 加入：

```rust
pub mod client;
```

`bytes` crate 需要提為直接依賴 —— 在 `src-tauri/Cargo.toml` 的 `axum` 那行後面加：

```toml
bytes = "1"
```

同時確認 `src-tauri/src/ai/sse.rs` 的 `map_http_error` 是 `pub`；若不是，改成 `pub`。

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test bridge_openai_upstream`
Expected: 3 passed。

- [ ] **Step 5: 跑全部測試確認沒弄壞既有功能**

Run: `cd src-tauri && cargo test`
Expected: 全綠。特別注意 `compatible_client` 與 `openai_client` 兩支既有測試 —— Step 1 改動了 `ai/sse.rs` 的可見性，行為不應改變。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/ src-tauri/tests/bridge_openai_upstream.rs
git commit -m "feat(bridge): OpenAI 上游 adapter 與整合測試"
```

---

## 階段 D：Anthropic 路徑

### Task 12: Anthropic passthrough adapter

**Files:**
- Create: `src-tauri/src/bridge/upstream/anthropic.rs`
- Modify: `src-tauri/src/bridge/upstream/mod.rs`
- Create: `src-tauri/tests/bridge_anthropic_upstream.rs`

- [ ] **Step 1: 寫失敗的單元測試**

建立 `src-tauri/src/bridge/upstream/anthropic.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_is_replaced_with_the_mapped_one() {
        let body = rewrite_body(
            &json!({"model": "aiterm:sonnet", "messages": [], "stream": true}),
            "claude-sonnet-4-5",
            false,
        );
        assert_eq!(body["model"], "claude-sonnet-4-5");
    }

    #[test]
    fn api_key_mode_leaves_system_untouched() {
        let body = rewrite_body(
            &json!({"model": "m", "system": "你是助手", "messages": []}),
            "m",
            false,
        );
        assert_eq!(body["system"], "你是助手");
    }

    #[test]
    fn oauth_mode_prepends_the_sentinel_when_missing() {
        // sk-ant-oat* token 要求第一個 system block 必須是這句，否則上游會回
        // 假的 rate_limit_error（見 ai/anthropic.rs:342）。
        let body = rewrite_body(&json!({"model": "m", "system": "自訂", "messages": []}), "m", true);
        let blocks = body["system"].as_array().expect("OAuth 模式要轉成 block 陣列");
        assert_eq!(blocks[0]["text"], CLAUDE_CODE_SENTINEL);
        assert_eq!(blocks[1]["text"], "自訂");
    }

    #[test]
    fn oauth_mode_does_not_duplicate_an_existing_sentinel() {
        // Claude Code 自己送的 system prompt 第一句正好就是那句，多半天然滿足。
        let body = rewrite_body(
            &json!({
                "model": "m", "messages": [],
                "system": [{"type": "text", "text": CLAUDE_CODE_SENTINEL}]
            }),
            "m",
            true,
        );
        let blocks = body["system"].as_array().unwrap();
        assert_eq!(blocks.len(), 1, "不能重複插入：{blocks:?}");
    }

    #[test]
    fn oauth_mode_handles_absent_system() {
        let body = rewrite_body(&json!({"model": "m", "messages": []}), "m", true);
        let blocks = body["system"].as_array().unwrap();
        assert_eq!(blocks[0]["text"], CLAUDE_CODE_SENTINEL);
    }

    #[test]
    fn messages_url_appends_v1_messages() {
        assert_eq!(messages_url("https://api.anthropic.com"), "https://api.anthropic.com/v1/messages");
        assert_eq!(messages_url("https://x.test/"), "https://x.test/v1/messages");
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::upstream::anthropic`
Expected: 編譯失敗，`cannot find function 'rewrite_body'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/bridge/upstream/anthropic.rs` 測試區塊之前加入：

```rust
//! Anthropic 家族轉發。
//!
//! 上游本來就講 Messages API，解析再重組是純粹的損耗，所以 SSE 原樣 pipe
//! （[`UpstreamResponse::Passthrough`]）。只改三處：模型名、auth 標頭、
//! OAuth 模式的 system 哨兵。

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ai::AiError;
use crate::bridge::anthropic::request::MessagesRequest;
use crate::bridge::upstream::{BridgeUpstream, UpstreamResponse};

/// OAuth（`sk-ant-oat*`）token 要求第一個 system block 必須是這句，否則上游
/// 回一個假的 `rate_limit_error`。與 `ai/anthropic.rs:342` 是同一個常數，
/// 那邊已 `pub`，這裡直接 re-export 而非重打一份。
pub use crate::ai::anthropic::CLAUDE_CODE_SENTINEL;

pub struct AnthropicUpstream {
    base_url: String,
    token: String,
    is_oauth: bool,
    client: reqwest::Client,
}

impl AnthropicUpstream {
    pub fn new(base_url: String, token: String, is_oauth: bool) -> Self {
        Self { base_url, token, is_oauth, client: reqwest::Client::new() }
    }
}

pub fn messages_url(base_url: &str) -> String {
    format!("{}/v1/messages", base_url.trim_end_matches('/'))
}

/// 改寫請求 body。`raw` 是 Claude Code 原封不動的 JSON —— 我們不重建它，
/// 因為任何我們沒解析的欄位（cache_control、未來新增的參數）都要原樣送達。
pub fn rewrite_body(raw: &Value, model: &str, is_oauth: bool) -> Value {
    let mut body = raw.clone();
    body["model"] = json!(model);
    if is_oauth {
        body["system"] = ensure_sentinel(raw.get("system"));
    }
    body
}

fn ensure_sentinel(system: Option<&Value>) -> Value {
    let sentinel = json!({"type": "text", "text": CLAUDE_CODE_SENTINEL});
    let mut blocks: Vec<Value> = match system {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => vec![json!({"type": "text", "text": s})],
        Some(Value::Array(a)) => a.clone(),
        Some(other) => vec![other.clone()],
    };
    let already = blocks
        .first()
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
        .map(|t| t.starts_with(CLAUDE_CODE_SENTINEL))
        .unwrap_or(false);
    if !already {
        blocks.insert(0, sentinel);
    }
    Value::Array(blocks)
}

#[async_trait]
impl BridgeUpstream for AnthropicUpstream {
    async fn send(
        &self,
        _req: &MessagesRequest,
        _model: &str,
    ) -> Result<UpstreamResponse, AiError> {
        // 這條路徑需要原始 JSON，走 `send_raw`。trait 方法保留是為了讓
        // 工廠能回傳統一的 Box<dyn BridgeUpstream>。
        Err(AiError::ModelError {
            reason: "Anthropic 轉發路徑請呼叫 send_raw".into(),
            raw: String::new(),
        })
    }
}

impl AnthropicUpstream {
    /// Anthropic 路徑專用：吃原始 JSON，回未解析的 HTTP 回應。
    pub async fn send_raw(&self, raw: &Value, model: &str) -> Result<UpstreamResponse, AiError> {
        let body = rewrite_body(raw, model, self.is_oauth);
        let mut rb = self.client.post(messages_url(&self.base_url)).json(&body);
        rb = if self.is_oauth {
            rb.bearer_auth(&self.token)
                .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
                .header("x-app", "cli")
        } else {
            rb.header("x-api-key", &self.token)
        };
        let resp = rb
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::ai::sse::map_http_error(status, &text));
        }
        Ok(UpstreamResponse::Passthrough(resp))
    }
}
```

在 `src-tauri/src/bridge/upstream/mod.rs` 加入：

```rust
pub mod anthropic;
```

若 `crate::ai::anthropic::CLAUDE_CODE_SENTINEL` 不是 `pub`，把 `ai/anthropic.rs:342` 的 `const` 改成 `pub const`。不要複製一份字串到 bridge。

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::upstream::anthropic`
Expected: 6 passed。

- [ ] **Step 5: 寫轉發的整合測試**

建立 `src-tauri/tests/bridge_anthropic_upstream.rs`：

```rust
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aiterm_lib::bridge::upstream::anthropic::AnthropicUpstream;
use aiterm_lib::bridge::upstream::UpstreamResponse;

#[tokio::test]
async fn api_key_mode_sets_x_api_key_and_passes_body_through() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("event: message_stop\ndata: {}\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-test".into(), false);
    let raw = serde_json::json!({
        "model": "aiterm:sonnet", "stream": true, "messages": [],
        // 我們沒解析的欄位必須原樣送達。
        "metadata": {"user_id": "x"}
    });
    let resp = up.send_raw(&raw, "claude-sonnet-4-5").await.unwrap();
    match resp {
        UpstreamResponse::Passthrough(r) => {
            let body = r.text().await.unwrap();
            assert!(body.contains("message_stop"));
        }
        _ => panic!("Anthropic 路徑必須回 Passthrough"),
    }
}

#[tokio::test]
async fn oauth_mode_sets_bearer_and_beta_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer sk-ant-oat-x"))
        .and(header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20"))
        .and(header("x-app", "cli"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: {}\n\n", "text/event-stream"))
        .mount(&server)
        .await;

    let up = AnthropicUpstream::new(server.uri(), "sk-ant-oat-x".into(), true);
    let resp = up.send_raw(&serde_json::json!({"model": "m", "messages": []}), "m").await;
    assert!(resp.is_ok());
}
```

- [ ] **Step 6: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test bridge_anthropic_upstream`
Expected: 2 passed。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/ src-tauri/tests/bridge_anthropic_upstream.rs
git commit -m "feat(bridge): Anthropic 家族轉發 adapter"
```

---

## 階段 E：server

### Task 13: 上游工廠

把 `provider_id` 解析成一個可用的上游。憑證解析**完全復用** `ai/router.rs` 既有的函式，不重寫續期邏輯。

**Files:**
- Create: `src-tauri/src/bridge/factory.rs`
- Modify: `src-tauri/src/bridge/mod.rs`
- Modify: `src-tauri/src/ai/router.rs`（把 `get_valid_oauth_token` 提為 `pub(crate)` 或 `pub`）

- [ ] **Step 1: 確認可見性**

在 `src-tauri/src/ai/router.rs:48` 找到 `get_valid_oauth_token`，確認它是 `pub` 或 `pub(crate)`；若是私有就改成 `pub(crate)`。

Run: `cd src-tauri && cargo check`
Expected: 成功。

- [ ] **Step 2: 寫失敗測試**

建立 `src-tauri/src/bridge/factory.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{ProviderConfig, ProviderType};

    fn provider(ty: ProviderType, auth: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            id: "p1".into(),
            display_name: "P".into(),
            provider_type: ty,
            base_url: Some("https://x.test".into()),
            oauth_client_id: None,
            model: "m".into(),
            supports_json_mode: false,
            auth_method: auth.map(str::to_string),
        }
    }

    #[test]
    fn openai_family_maps_to_openai_kind() {
        for ty in [
            ProviderType::Openai,
            ProviderType::OpenaiCompatible,
            ProviderType::Ollama,
            ProviderType::Openrouter,
            ProviderType::Deepseek,
            ProviderType::Kimi,
            ProviderType::Xai,
            ProviderType::GithubCopilot,
        ] {
            assert_eq!(kind_for(&provider(ty, None)), Some(UpstreamKind::OpenAi), "{ty:?}");
        }
    }

    #[test]
    fn anthropic_family_maps_to_anthropic_kind() {
        assert_eq!(
            kind_for(&provider(ProviderType::Anthropic, None)),
            Some(UpstreamKind::Anthropic)
        );
        assert_eq!(
            kind_for(&provider(ProviderType::AnthropicCompatible, None)),
            Some(UpstreamKind::Anthropic)
        );
    }

    #[test]
    fn google_ai_splits_on_auth_method() {
        // 沿用 router.rs:463 的判斷：oauth 走 Antigravity，其餘走 OpenAI 相容。
        assert_eq!(
            kind_for(&provider(ProviderType::GoogleAi, None)),
            Some(UpstreamKind::OpenAi)
        );
        assert_eq!(kind_for(&provider(ProviderType::GoogleAi, Some("oauth"))), None);
    }

    #[test]
    fn codex_is_not_supported_in_m1() {
        assert_eq!(kind_for(&provider(ProviderType::Codex, None)), None);
    }

    #[test]
    fn unsupported_message_names_the_provider() {
        let err = unsupported_message(&provider(ProviderType::Codex, None));
        assert!(err.contains("P"), "訊息要含顯示名稱：{err}");
    }
}
```

- [ ] **Step 3: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::factory`
Expected: 編譯失敗，`cannot find type 'UpstreamKind'`。

- [ ] **Step 4: 實作**

在 `src-tauri/src/bridge/factory.rs` 測試區塊之前加入：

```rust
//! provider_id → 可用的上游實例。
//!
//! 憑證每個請求重新解析，不快取：Codex 的 access token 300 秒就過期
//! （`ai/router.rs:244`），Antigravity 每請求都要帶 project。M1 雖然還沒接
//! 這兩條路徑，但介面先照這個約束設計，M2/M3 才不用回頭改。

use std::sync::Arc;

use crate::ai::AiError;
use crate::config::types::{ProviderConfig, ProviderType};
use crate::config::ConfigStore;
use crate::secret::SecretStore;

use super::upstream::anthropic::AnthropicUpstream;
use super::upstream::openai::client::OpenAiUpstream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamKind {
    OpenAi,
    Anthropic,
}

/// M1 支援的上游種類；不支援的回 `None`。
pub fn kind_for(p: &ProviderConfig) -> Option<UpstreamKind> {
    match p.provider_type {
        ProviderType::Openai
        | ProviderType::OpenaiCompatible
        | ProviderType::Ollama
        | ProviderType::Openrouter
        | ProviderType::Deepseek
        | ProviderType::Kimi
        | ProviderType::Xai
        | ProviderType::GithubCopilot => Some(UpstreamKind::OpenAi),

        ProviderType::Anthropic | ProviderType::AnthropicCompatible => Some(UpstreamKind::Anthropic),

        // router.rs:463 的同一個判斷：oauth 走 Antigravity（M3），其餘是
        // OpenAI 相容端點。
        ProviderType::GoogleAi => match p.auth_method.as_deref() {
            Some("oauth") => None,
            _ => Some(UpstreamKind::OpenAi),
        },

        // M2 / M3。
        ProviderType::Codex => None,
    }
}

/// 回給 Claude Code 顯示的訊息，所以要寫成使用者看得懂的句子。
pub fn unsupported_message(p: &ProviderConfig) -> String {
    format!(
        "AITerm 橋接目前還不支援供應商「{}」。請到設定 → Claude Code 橋接改選其他供應商。",
        p.display_name
    )
}

pub enum Upstream {
    OpenAi(OpenAiUpstream),
    Anthropic(AnthropicUpstream),
}

/// 建立上游實例。每個請求呼叫一次。
pub async fn build(
    config: &Arc<ConfigStore>,
    secrets: &Arc<SecretStore>,
    provider_id: &str,
) -> Result<Upstream, AiError> {
    let cfg = config.get();
    let p = cfg.find_provider(provider_id).ok_or_else(|| AiError::NotConfigured {
        message: format!("AITerm 橋接找不到供應商 id「{provider_id}」，它可能已被刪除。"),
    })?;

    let kind = kind_for(p).ok_or_else(|| AiError::NotConfigured {
        message: unsupported_message(p),
    })?;

    match kind {
        UpstreamKind::OpenAi => {
            let base = p.base_url.clone().unwrap_or_default();
            let key = secrets.get(provider_id).ok().flatten().unwrap_or_default();
            Ok(Upstream::OpenAi(OpenAiUpstream::new(base, key)))
        }
        UpstreamKind::Anthropic => {
            let base = p
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            let is_oauth = p.auth_method.as_deref() == Some("oauth");
            let token = if is_oauth {
                crate::ai::router::get_valid_oauth_token(provider_id, secrets).await?
            } else {
                secrets.get(provider_id).ok().flatten().unwrap_or_default()
            };
            Ok(Upstream::Anthropic(AnthropicUpstream::new(base, token, is_oauth)))
        }
    }
}
```

在 `src-tauri/src/bridge/mod.rs` 加入：

```rust
pub mod factory;
```

- [ ] **Step 5: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::factory`
Expected: 5 passed。

若 `kind_for` 的 `match` 因為新增了 `ProviderType` 變體而編譯失敗，**不要**加 `_ => None` 萬用分支 —— 保留窮舉，讓未來新增供應商時編譯器強迫你在這裡做決定。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/bridge/factory.rs src-tauri/src/bridge/mod.rs src-tauri/src/ai/router.rs
git commit -m "feat(bridge): 上游工廠與 M1 供應商支援矩陣"
```

---

### Task 14: axum server 與端點

**Files:**
- Create: `src-tauri/src/bridge/server.rs`
- Modify: `src-tauri/src/bridge/mod.rs`

- [ ] **Step 1: 寫失敗測試（純函式部分）**

建立 `src-tauri/src/bridge/server.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn token_estimate_counts_all_text_fields() {
        let req: crate::bridge::anthropic::request::MessagesRequest =
            serde_json::from_value(json!({
                "model": "m",
                "system": "1234",
                "messages": [{"role": "user", "content": "12345678"}]
            }))
            .unwrap();
        // 4 + 8 = 12 字元 → 12/4 = 3
        assert_eq!(estimate_input_tokens(&req), 3);
    }

    #[test]
    fn token_estimate_never_returns_zero_for_nonempty_input() {
        let req: crate::bridge::anthropic::request::MessagesRequest =
            serde_json::from_value(json!({"model": "m", "messages": [{"role":"user","content":"a"}]}))
                .unwrap();
        assert_eq!(estimate_input_tokens(&req), 1);
    }

    #[test]
    fn token_estimate_includes_tool_result_text() {
        let req: crate::bridge::anthropic::request::MessagesRequest =
            serde_json::from_value(json!({
                "model": "m",
                "messages": [{"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t", "content": "12345678"}
                ]}]
            }))
            .unwrap();
        assert_eq!(estimate_input_tokens(&req), 2);
    }

    #[test]
    fn error_status_maps_from_ai_error() {
        use crate::ai::AiError;
        assert_eq!(status_for(&AiError::AuthFailed { message: "x".into() }), 401);
        assert_eq!(status_for(&AiError::RateLimit { message: "x".into() }), 429);
        assert_eq!(status_for(&AiError::NotConfigured { message: "x".into() }), 400);
        assert_eq!(status_for(&AiError::Network { message: "x".into() }), 502);
    }
}
```

`AiError` 各變體的實際欄位名以 `src-tauri/src/ai/mod.rs:28-51` 為準；若欄位不是 `message`，把測試裡的建構改成實際的形狀。

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::server`
Expected: 編譯失敗，`cannot find function 'estimate_input_tokens'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/bridge/server.rs` 測試區塊之前加入：

```rust
//! 綁 127.0.0.1 的 Anthropic Messages API server。

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::ai::AiError;
use crate::bridge::anthropic::request::{parse_content, system_text, ContentBlock, MessagesRequest};
use crate::bridge::anthropic::response::{error_frame, ping_frame, SseEncoder};
use crate::bridge::factory::{build, Upstream};
use crate::bridge::upstream::UpstreamResponse;
use crate::bridge::{auth, model_map};
use crate::config::ConfigStore;
use crate::secret::SecretStore;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub secrets: Arc<SecretStore>,
    pub token: Arc<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .with_state(state)
}

fn authorize(state: &AppState, headers: &HeaderMap) -> bool {
    let authorization = headers.get("authorization").and_then(|v| v.to_str().ok());
    let x_api_key = headers.get("x-api-key").and_then(|v| v.to_str().ok());
    match auth::extract_token(authorization, x_api_key) {
        Some(provided) => auth::token_matches(&state.token, &provided),
        None => false,
    }
}

/// 非串流的錯誤回應（連 SSE 都還沒開始時用）。
fn json_error(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"type": "error", "error": {"type": kind, "message": message}})),
    )
        .into_response()
}

pub fn status_for(err: &AiError) -> u16 {
    match err {
        AiError::AuthFailed { .. } => 401,
        AiError::RateLimit { .. } => 429,
        AiError::NotConfigured { .. } | AiError::InvalidInput { .. } => 400,
        AiError::Network { .. } => 502,
        AiError::ModelError { .. } => 500,
    }
}

fn error_text(err: &AiError) -> String {
    // AiError 是 #[serde(tag="kind")] 的判別聯集，直接序列化再取訊息欄位
    // 會比逐變體 match 更耐得住未來新增變體。
    serde_json::to_value(err)
        .ok()
        .and_then(|v| {
            v.get("message")
                .or_else(|| v.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("{err:?}"))
}

fn error_kind(err: &AiError) -> &'static str {
    match err {
        AiError::AuthFailed { .. } => "authentication_error",
        AiError::RateLimit { .. } => "rate_limit_error",
        AiError::NotConfigured { .. } | AiError::InvalidInput { .. } => "invalid_request_error",
        _ => "api_error",
    }
}

async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorize(&state, &headers) {
        return json_error(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "AITerm 橋接的 token 不正確。請確認終端機分頁是由 AITerm 開啟的。",
        );
    }

    let Ok(raw) = serde_json::from_slice::<Value>(&body) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_request_error", "請求不是合法的 JSON。");
    };
    let req: MessagesRequest = match serde_json::from_value(raw.clone()) {
        Ok(r) => r,
        Err(e) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                &format!("無法解析 Messages 請求：{e}"),
            )
        }
    };
    if req.stream != Some(true) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "AITerm 橋接只支援串流請求（stream: true）。",
        );
    }

    let cfg = state.config.get();
    let mapping = match model_map::resolve(&cfg.claude_bridge, &req.model) {
        Ok(m) => m.clone(),
        Err(msg) => return json_error(StatusCode::BAD_REQUEST, "invalid_request_error", &msg),
    };

    let message_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(Body::from_stream(super::stream::run(
            state, mapping, req, raw, message_id,
        )))
        .expect("建立 SSE 回應不應失敗")
}

async fn count_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorize(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "authentication_error", "token 不正確。");
    }
    let Ok(req) = serde_json::from_slice::<MessagesRequest>(&body) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_request_error", "請求格式錯誤。");
    };
    Json(json!({"input_tokens": estimate_input_tokens(&req)})).into_response()
}

/// 粗估 token 數：字元數 ÷ 4。
///
/// Claude Code 用這個端點做 context 管理，不需要精確值。引入 tiktoken 對
/// 非 OpenAI 模型也只是另一種估算，不值得多一個依賴與詞彙表體積。
pub fn estimate_input_tokens(req: &MessagesRequest) -> u32 {
    let mut chars = system_text(req.system.as_ref()).chars().count();
    for m in &req.messages {
        chars += count_blocks(&parse_content(&m.content));
    }
    let est = chars / 4;
    if chars > 0 { est.max(1) as u32 } else { 0 }
}

fn count_blocks(blocks: &[ContentBlock]) -> usize {
    blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Text(t) | ContentBlock::Thinking(t) => t.chars().count(),
            // 圖片按 Anthropic 文件的粗略公式無法在不解碼下算準，
            // 用 base64 長度的固定比例當下限即可。
            ContentBlock::Image { data, .. } => data.len() / 750,
            ContentBlock::ToolUse { name, input, .. } => {
                name.chars().count() + input.to_string().chars().count()
            }
            ContentBlock::ToolResult { content, .. } => count_blocks(content),
        })
        .sum()
}

/// 上游錯誤 → 一則 SSE error frame。串流已經開始時只能用這個回報。
pub fn error_frame_for(err: &AiError) -> String {
    error_frame(error_kind(err), &error_text(err))
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::server`
Expected: 4 passed。此時 `super::stream::run` 尚未存在，編譯會失敗 —— 先把 `messages` 的 body 換成 `Body::empty()` 讓測試跑過，Task 15 再接回來。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bridge/server.rs src-tauri/src/bridge/mod.rs
git commit -m "feat(bridge): axum 路由、授權、錯誤映射與 count_tokens"
```

---

### Task 15: 串流管線（含 ping keepalive）

**Files:**
- Create: `src-tauri/src/bridge/stream.rs`
- Modify: `src-tauri/src/bridge/server.rs`（把 `Body::empty()` 換回 `super::stream::run(...)`）
- Modify: `src-tauri/src/bridge/mod.rs`

- [ ] **Step 1: 實作**

建立 `src-tauri/src/bridge/stream.rs`：

```rust
//! 把上游回應接成送給 Claude Code 的 SSE byte 串流。
//!
//! 開場的 ping：Claude Code 在等上游第一個 byte 期間遇到 SSE 靜默會斷線，
//! 而 SSE 註解（`: ping`）不算資料。所以在等待上游回應時每 3 秒送一個真的
//! `event: ping` frame。本地模型冷啟動可能要好幾十秒，這條保護是必要的。

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use futures_util::StreamExt;
use serde_json::Value;

use crate::bridge::anthropic::request::MessagesRequest;
use crate::bridge::anthropic::response::{ping_frame, SseEncoder};
use crate::bridge::factory::{build, Upstream};
use crate::bridge::server::{error_frame_for, AppState};
use crate::bridge::upstream::UpstreamResponse;
use crate::config::types::TierMapping;

/// 每隔多久補一個 ping。3 秒遠低於任何合理的客戶端逾時，成本可忽略。
const PING_INTERVAL: Duration = Duration::from_secs(3);

pub fn run(
    state: AppState,
    mapping: TierMapping,
    req: MessagesRequest,
    raw: Value,
    message_id: String,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(64);

    tokio::spawn(async move {
        let send = |b: Bytes| {
            let tx = tx.clone();
            async move { tx.send(b).await.is_ok() }
        };

        // ── 解析上游憑證並發出請求，期間持續送 ping ──────────────────
        let upstream_fut = async {
            let up = build(&state.config, &state.secrets, &mapping.provider_id).await?;
            match up {
                Upstream::Anthropic(a) => a.send_raw(&raw, &mapping.model).await,
                Upstream::OpenAi(o) => {
                    use crate::bridge::upstream::BridgeUpstream;
                    o.send(&req, &mapping.model).await
                }
            }
        };
        tokio::pin!(upstream_fut);

        let mut ticker = tokio::time::interval(PING_INTERVAL);
        let resp = loop {
            tokio::select! {
                r = &mut upstream_fut => break r,
                _ = ticker.tick() => {
                    if !send(Bytes::from(ping_frame())).await { return; }
                }
            }
        };

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                log::warn!("bridge upstream 失敗 provider={} err={e:?}", mapping.provider_id);
                let _ = send(Bytes::from(error_frame_for(&e))).await;
                return;
            }
        };

        match resp {
            // Anthropic 家族：原樣 pipe，不解析也不重組。
            UpstreamResponse::Passthrough(r) => {
                let mut bytes = r.bytes_stream();
                while let Some(chunk) = bytes.next().await {
                    match chunk {
                        Ok(b) => {
                            if !send(b).await { return; }
                        }
                        Err(e) => {
                            log::warn!("bridge passthrough 串流中斷：{e}");
                            return;
                        }
                    }
                }
            }
            UpstreamResponse::Events(mut events) => {
                let mut enc = SseEncoder::new(message_id, req.model.clone());
                for f in enc.start() {
                    if !send(Bytes::from(f)).await { return; }
                }
                while let Some(item) = events.next().await {
                    match item {
                        Ok(ev) => {
                            for f in enc.push(ev) {
                                if !send(Bytes::from(f)).await { return; }
                            }
                        }
                        Err(e) => {
                            log::warn!("bridge 串流中斷 provider={} err={e:?}", mapping.provider_id);
                            let _ = send(Bytes::from(error_frame_for(&e))).await;
                            return;
                        }
                    }
                }
            }
        }
    });

    futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|b| (Ok(b), rx))
    })
}
```

在 `src-tauri/src/bridge/mod.rs` 加入：

```rust
pub mod server;
pub mod stream;
```

把 `src-tauri/src/bridge/server.rs` 中 Task 14 Step 4 暫時改成 `Body::empty()` 的地方換回：

```rust
        .body(Body::from_stream(super::stream::run(
            state, mapping, req, raw, message_id,
        )))
```

- [ ] **Step 2: 驗證編譯**

Run: `cd src-tauri && cargo check`
Expected: 成功。

`tokio::time::interval` 的第一個 tick 會立刻完成，所以連線一建立就會先收到一個 ping —— 這是想要的行為，不要改成 `interval_at`。

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/bridge/
git commit -m "feat(bridge): SSE 串流管線與開場 ping keepalive"
```

---

### Task 16: server 生命週期與 Tauri 指令

**Files:**
- Modify: `src-tauri/src/bridge/mod.rs`
- Create: `src-tauri/src/commands/bridge.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`（`.manage()` 與 `invoke_handler!`）

- [ ] **Step 1: 實作 BridgeState**

在 `src-tauri/src/bridge/mod.rs` 的模組宣告之後加入：

```rust
use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::ConfigStore;
use crate::secret::SecretStore;

pub struct BridgeState {
    running: Mutex<Option<Running>>,
}

struct Running {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl Default for BridgeState {
    fn default() -> Self {
        Self { running: Mutex::new(None) }
    }
}

impl BridgeState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.port)
    }

    /// 啟動 server。已經在跑就先停掉（換 port 時會用到）。
    ///
    /// 埠被占用時回錯誤而不是換一個 —— 環境變數只能在分頁 spawn 的瞬間決定，
    /// 埠若會漂移，已開的分頁會指向死位址。
    pub async fn start(
        &self,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        token: String,
        port: u16,
    ) -> anyhow::Result<()> {
        self.stop();

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            anyhow::anyhow!("無法綁定 127.0.0.1:{port}（{e}）。請在設定裡換一個埠。")
        })?;

        let app = server::router(server::AppState {
            config,
            secrets,
            token: Arc::new(token),
        });
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let served = axum::serve(listener, app)
                .with_graceful_shutdown(async { let _ = rx.await; })
                .await;
            if let Err(e) = served {
                log::error!("bridge server 結束於錯誤：{e}");
            }
        });

        *self.running.lock() = Some(Running { port, shutdown: tx });
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
        }
    }
}
```

- [ ] **Step 2: 實作 Tauri 指令**

建立 `src-tauri/src/commands/bridge.rs`：

```rust
//! Claude Code 橋接的前端指令。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::bridge::{auth, BridgeState};
use crate::config::ConfigStore;
use crate::secret::SecretStore;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub running: bool,
    pub port: Option<u16>,
    /// 已設定的 token（給「複製手動命令」用）。未啟用時為 None。
    pub token: Option<String>,
    pub error: Option<String>,
}

/// 取得（必要時產生）橋接 token。
fn ensure_token(secrets: &Arc<SecretStore>) -> anyhow::Result<String> {
    if let Some(t) = secrets.get(auth::BRIDGE_TOKEN_KEY)? {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let t = auth::generate_token();
    secrets.set(auth::BRIDGE_TOKEN_KEY, &t)?;
    Ok(t)
}

#[tauri::command]
pub async fn bridge_status(
    bridge: State<'_, Arc<BridgeState>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<BridgeStatus, String> {
    let port = bridge.port();
    let token = if port.is_some() {
        secrets.get(auth::BRIDGE_TOKEN_KEY).ok().flatten()
    } else {
        None
    };
    Ok(BridgeStatus { running: port.is_some(), port, token, error: None })
}

/// 依目前 config 啟動或停止 server。設定頁存檔後呼叫。
#[tauri::command]
pub async fn bridge_apply(
    bridge: State<'_, Arc<BridgeState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<BridgeStatus, String> {
    let cfg = config.get().claude_bridge;
    if !cfg.enabled {
        bridge.stop();
        return Ok(BridgeStatus { running: false, port: None, token: None, error: None });
    }

    let token = ensure_token(&secrets).map_err(|e| e.to_string())?;
    match bridge
        .start(config.inner().clone(), secrets.inner().clone(), token.clone(), cfg.port)
        .await
    {
        Ok(()) => Ok(BridgeStatus {
            running: true,
            port: Some(cfg.port),
            token: Some(token),
            error: None,
        }),
        Err(e) => Ok(BridgeStatus {
            running: false,
            port: None,
            token: None,
            // 回成 status.error 而非 Err：埠被占用是使用者要處理的狀態，
            // 不是程式錯誤，UI 要能把它顯示在區塊裡。
            error: Some(e.to_string()),
        }),
    }
}
```

在 `src-tauri/src/commands/mod.rs` 加入 `pub mod bridge;`。

- [ ] **Step 3: 接線到 lib.rs**

在 `src-tauri/src/lib.rs` 的 `use commands::{...}` 區塊加入：

```rust
    bridge::{bridge_apply, bridge_status},
```

在 `.manage(AnthropicOAuthState::new())` 後面加入：

```rust
        .manage(Arc::new(bridge::BridgeState::new()))
```

在 `invoke_handler![...]` 的清單裡（Provider management 區塊附近）加入：

```rust
            bridge_status,
            bridge_apply,
```

在 `.setup(|app| {` 區塊內，加入啟動時自動拉起 server 的邏輯：

```rust
        // 橋接 server：設定為 enabled 時隨 app 啟動。失敗只記 log 不擋啟動
        // ——埠被占用不該讓整個 app 起不來，設定頁會顯示錯誤。
        {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Manager;
                let bridge = handle.state::<Arc<bridge::BridgeState>>().inner().clone();
                let config = handle.state::<Arc<ConfigStore>>().inner().clone();
                let secrets = handle.state::<Arc<SecretStore>>().inner().clone();
                let cfg = config.get().claude_bridge;
                if !cfg.enabled {
                    return;
                }
                let token = match secrets.get(bridge::auth::BRIDGE_TOKEN_KEY) {
                    Ok(Some(t)) if !t.is_empty() => t,
                    _ => {
                        let t = bridge::auth::generate_token();
                        if let Err(e) = secrets.set(bridge::auth::BRIDGE_TOKEN_KEY, &t) {
                            log::error!("bridge token 寫入 keychain 失敗：{e}");
                            return;
                        }
                        t
                    }
                };
                if let Err(e) = bridge.start(config, secrets, token, cfg.port).await {
                    log::error!("bridge server 啟動失敗：{e}");
                }
            });
        }
```

`ConfigStore` 與 `SecretStore` 在 `.manage()` 時的實際包裝型別以 `lib.rs:246-247` 為準（若是 `Arc<ConfigStore>` 就照上面寫；若是裸值，`state::<ConfigStore>()` 後自行 `Arc::new` 會產生第二份 —— 這時改成把 `.manage()` 的型別統一成 `Arc<...>`）。

- [ ] **Step 4: 驗證編譯**

Run: `cd src-tauri && cargo check`
Expected: 成功。

- [ ] **Step 5: 寫 server 的端到端整合測試**

建立 `src-tauri/tests/bridge_server.rs`：

```rust
//! 直接對 axum router 發請求，驗證授權與錯誤路徑。
//! 不啟動真的 TcpListener，用 tower 的 oneshot。

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use aiterm_lib::bridge::server::{router, AppState};
use aiterm_lib::config::ConfigStore;
use aiterm_lib::secret::SecretStore;

fn state(dir: &tempfile::TempDir) -> AppState {
    AppState {
        config: Arc::new(ConfigStore::new_at(dir.path().join("config.toml"))),
        secrets: Arc::new(SecretStore::new()),
        token: Arc::new("t0ken".into()),
    }
}

fn post(uri: &str, token: Option<&str>, body: serde_json::Value) -> Request<Body> {
    let mut b = Request::builder().method("POST").uri(uri).header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn rejects_missing_token() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post("/v1/messages", None, serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_wrong_token() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post("/v1/messages", Some("nope"), serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_non_streaming_request() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post(
            "/v1/messages",
            Some("t0ken"),
            serde_json::json!({"model": "aiterm:sonnet", "messages": [], "stream": false}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unmapped_tier_returns_400_pointing_at_settings() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post(
            "/v1/messages",
            Some("t0ken"),
            serde_json::json!({"model": "aiterm:sonnet", "messages": [], "stream": true}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("設定"), "訊息要指向設定頁：{text}");
}

#[tokio::test]
async fn count_tokens_returns_an_estimate() {
    let dir = tempfile::tempdir().unwrap();
    let resp = router(state(&dir))
        .oneshot(post(
            "/v1/messages/count_tokens",
            Some("t0ken"),
            serde_json::json!({"model": "m", "messages": [{"role":"user","content":"12345678"}]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["input_tokens"], 2);
}
```

需要在 `[dev-dependencies]` 加 `tower = "0.5"`。`ConfigStore::new_at` 若不存在，用 `ConfigStore` 實際提供的建構子（見 `src-tauri/src/config/mod.rs:17`），必要時加一個只給測試用的 `pub fn new_at(path: PathBuf)`。

- [ ] **Step 6: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test bridge_server`
Expected: 5 passed。

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/ src-tauri/tests/bridge_server.rs
git commit -m "feat(bridge): server 生命週期、Tauri 指令與端到端測試"
```

---

## 階段 F：環境變數注入與前端

### Task 17: 環境變數注入

**Files:**
- Create: `src-tauri/src/bridge/env.rs`
- Modify: `src-tauri/src/bridge/mod.rs`
- Modify: `src-tauri/src/pty/manager.rs:26`
- Modify: `src-tauri/src/pty/commands.rs:25-34`
- Modify: `src-tauri/src/pty/session.rs`（`spawn_with_id` 的 `env_remove`）

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/src/bridge/env.rs`，先只寫測試：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(envs: &'a [(String, String)], key: &str) -> Option<&'a str> {
        envs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn base_url_has_no_v1_suffix() {
        // Claude Code 自己會接上 /v1/messages；帶了 /v1 會變成 /v1/v1/messages。
        let e = bridge_envs(8317, "tok");
        assert_eq!(find(&e, "ANTHROPIC_BASE_URL"), Some("http://127.0.0.1:8317"));
    }

    #[test]
    fn injects_the_three_tier_sentinels() {
        let e = bridge_envs(8317, "tok");
        assert_eq!(find(&e, "ANTHROPIC_DEFAULT_OPUS_MODEL"), Some("aiterm:opus"));
        assert_eq!(find(&e, "ANTHROPIC_DEFAULT_SONNET_MODEL"), Some("aiterm:sonnet"));
        assert_eq!(find(&e, "ANTHROPIC_DEFAULT_HAIKU_MODEL"), Some("aiterm:haiku"));
    }

    #[test]
    fn injects_auth_token_and_timeouts() {
        let e = bridge_envs(9000, "secret-token");
        assert_eq!(find(&e, "ANTHROPIC_AUTH_TOKEN"), Some("secret-token"));
        assert_eq!(find(&e, "API_TIMEOUT_MS"), Some("3000000"));
        assert_eq!(find(&e, "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"), Some("1"));
    }

    #[test]
    fn api_key_is_listed_for_removal() {
        // 使用者環境本來就有的 ANTHROPIC_API_KEY 是難查的干擾源：症狀是
        // 「明明設了橋接卻打到真的 Anthropic」。
        assert!(ENV_TO_REMOVE.contains(&"ANTHROPIC_API_KEY"));
    }

    #[test]
    fn port_is_reflected_in_the_url() {
        let e = bridge_envs(12345, "t");
        assert_eq!(find(&e, "ANTHROPIC_BASE_URL"), Some("http://127.0.0.1:12345"));
    }
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --lib bridge::env`
Expected: 編譯失敗，`cannot find function 'bridge_envs'`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/bridge/env.rs` 測試區塊之前加入：

```rust
//! 注入終端機分頁的環境變數。

/// 啟用橋接的分頁必須清掉的變數。
///
/// Claude Code 在 `ANTHROPIC_AUTH_TOKEN` 與 `ANTHROPIC_API_KEY` 並存時的
/// 優先序未經我們驗證，與其依賴假設，不如把 API key 清掉 —— 症狀
/// 「設了橋接卻打到真的 Anthropic」極難追查。
pub const ENV_TO_REMOVE: &[&str] = &["ANTHROPIC_API_KEY"];

/// 產生要塞進 `ShellSpec.envs` 的鍵值對。
pub fn bridge_envs(port: u16, token: &str) -> Vec<(String, String)> {
    let pair = |k: &str, v: String| (k.to_string(), v);
    vec![
        // 不能帶 /v1 後綴：Claude Code 自己會接上 /v1/messages。
        pair("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}")),
        pair("ANTHROPIC_AUTH_TOKEN", token.to_string()),
        // 哨兵字串：server 直接用它判層級，比猜真實型號穩定。
        pair("ANTHROPIC_DEFAULT_OPUS_MODEL", "aiterm:opus".into()),
        pair("ANTHROPIC_DEFAULT_SONNET_MODEL", "aiterm:sonnet".into()),
        pair("ANTHROPIC_DEFAULT_HAIKU_MODEL", "aiterm:haiku".into()),
        // 本地模型冷啟動很慢，預設逾時會在第一次請求就砍掉連線。
        pair("API_TIMEOUT_MS", "3000000".into()),
        pair("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
    ]
}
```

在 `src-tauri/src/bridge/mod.rs` 加入 `pub mod env;`。

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib bridge::env`
Expected: 5 passed。

- [ ] **Step 5: 讓 ShellSpec 能帶移除清單**

在 `src-tauri/src/pty/shell.rs:4-8` 的 `ShellSpec` 加一個欄位：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
    /// 要從繼承環境中移除的變數名。
    pub env_removals: Vec<String>,
}
```

編譯器會指出所有建構 `ShellSpec` 的地方（`shell.rs` 內的三個平台分支）；每處補上 `env_removals: Vec::new()`。

在 `src-tauri/src/pty/session.rs` 的 `spawn` 與 `spawn_with_id` 兩處，在既有的 `for (k, v) in shell.envs { cmd.env(k, v); }` **之後**加入：

```rust
        for k in &shell.env_removals {
            cmd.env_remove(k);
        }
```

順序重要：移除要在設定之後，這樣清單與注入清單若不小心重疊時，移除會贏。

- [ ] **Step 6: 讓 PtyManager 接受橋接參數**

修改 `src-tauri/src/pty/manager.rs:26` 的簽名與內容：

```rust
    /// High-level: spawn a session and wire its output to a Tauri event.
    ///
    /// `bridge_env` 非 None 時，把 Claude Code 橋接的環境變數注入這個分頁。
    /// 環境變數只能在 spawn 的瞬間決定，所以事後無法對已開的分頁切換。
    pub fn create_with_app(
        &self,
        app: AppHandle,
        size: PtySize,
        cwd: Option<PathBuf>,
        bridge_env: Option<(u16, String)>,
    ) -> PtyResult<String> {
        let mut shell: ShellSpec = default_shell().ok_or(PtyError::NoShellAvailable)?;

        if let Some((port, token)) = bridge_env {
            shell.envs.extend(crate::bridge::env::bridge_envs(port, &token));
            shell
                .env_removals
                .extend(crate::bridge::env::ENV_TO_REMOVE.iter().map(|s| s.to_string()));
        }
```

（其餘內容不變。）

`create_with_callback`（`manager.rs:48`）不需要改 —— 它是測試專用的低階入口。

- [ ] **Step 7: 讓指令層決定要不要注入**

修改 `src-tauri/src/pty/commands.rs:25-34`：

```rust
#[tauri::command]
pub fn pty_create(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    bridge: State<'_, std::sync::Arc<crate::bridge::BridgeState>>,
    secrets: State<'_, std::sync::Arc<crate::secret::SecretStore>>,
    size: PtySizeArg,
    cwd: Option<String>,
    claude_bridge: Option<bool>,
) -> Result<String, PtyError> {
    let cwd = cwd.map(std::path::PathBuf::from);
    // server 沒在跑就不注入 —— 注入指向死埠的位址比不注入更難除錯。
    let bridge_env = match (claude_bridge.unwrap_or(false), bridge.port()) {
        (true, Some(port)) => secrets
            .get(crate::bridge::auth::BRIDGE_TOKEN_KEY)
            .ok()
            .flatten()
            .map(|t| (port, t)),
        _ => None,
    };
    manager.create_with_app(app, size.into(), cwd, bridge_env)
}
```

編譯器會指出 `create_with_app` 的其他呼叫端（若有），補上 `None`。

- [ ] **Step 8: 驗證編譯與既有測試**

Run: `cd src-tauri && cargo test`
Expected: 全綠。特別注意 `pty_integration` 與 `pty_cwd_tracking` 兩支 —— `ShellSpec` 加欄位可能讓它們的 fixture 編不過。

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/
git commit -m "feat(bridge): 分頁層級的環境變數注入"
```

---

### Task 18: 前端 IPC 與字串

**Files:**
- Create: `src/ipc/bridge.ts`
- Modify: `src/ipc/pty.ts:13-15`
- Modify: `src/lib/i18n.ts`
- Modify: `src/components/TerminalView.tsx:807`
- Modify: `src/components/LoopStudio/index.tsx:131`

- [ ] **Step 1: 新增 IPC 包裝**

建立 `src/ipc/bridge.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";

export interface BridgeStatus {
  running: boolean;
  port: number | null;
  /** 已產生的 token，供「複製手動命令」使用。未啟動時為 null。 */
  token: string | null;
  /** 啟動失敗的原因（例如埠被占用）。這是使用者要處理的狀態，不是例外。 */
  error: string | null;
}

export function bridgeStatus(): Promise<BridgeStatus> {
  return invoke<BridgeStatus>("bridge_status");
}

/** 依目前 config 啟動或停止 server。設定存檔後呼叫。 */
export function bridgeApply(): Promise<BridgeStatus> {
  return invoke<BridgeStatus>("bridge_apply");
}
```

- [ ] **Step 2: 讓 createPty 能帶橋接旗標**

修改 `src/ipc/pty.ts:13-15`：

```ts
export function createPty(
  size: PtySize,
  cwd?: string,
  claudeBridge?: boolean,
): Promise<string> {
  return invoke<string>("pty_create", {
    size,
    cwd: cwd ?? null,
    claudeBridge: claudeBridge ?? false,
  });
}
```

`src/components/LoopStudio/index.tsx:131` 的呼叫不需要改（第三個參數可省略）。

- [ ] **Step 3: 加入 i18n 字串**

在 `src/lib/i18n.ts` 的 `zhTW` 物件裡，設定相關字串附近加入：

```ts
    // Claude Code 橋接
    bridge_title: "Claude Code 橋接",
    bridge_desc: "讓終端機裡的 Claude Code CLI 改用 AITerm 設定的供應商。",
    bridge_enable: "啟用橋接 server",
    bridge_port: "連接埠",
    bridge_default_on_new_tab: "新分頁預設啟用",
    bridge_tier_opus: "Opus 層（規劃與困難任務）",
    bridge_tier_sonnet: "Sonnet 層（一般任務）",
    bridge_tier_haiku: "Haiku 層（背景小任務）",
    bridge_tier_provider: "供應商",
    bridge_tier_model: "模型",
    bridge_tier_unset: "（未設定）",
    bridge_unsupported_suffix: "（尚未支援）",
    bridge_status_running: "執行中",
    bridge_status_stopped: "未啟動",
    bridge_copy_command: "複製手動命令",
    bridge_copied: "已複製",
    bridge_new_tab: "新增 Claude Code 分頁",
    bridge_tab_badge: "CC",
```

在同檔案的 `en` 物件裡加入對應英文：

```ts
    bridge_title: "Claude Code bridge",
    bridge_desc: "Point the Claude Code CLI in your terminal at a provider configured in AITerm.",
    bridge_enable: "Enable bridge server",
    bridge_port: "Port",
    bridge_default_on_new_tab: "Enable on new tabs by default",
    bridge_tier_opus: "Opus tier (planning, hard tasks)",
    bridge_tier_sonnet: "Sonnet tier (general work)",
    bridge_tier_haiku: "Haiku tier (small background tasks)",
    bridge_tier_provider: "Provider",
    bridge_tier_model: "Model",
    bridge_tier_unset: "(not set)",
    bridge_unsupported_suffix: " (not supported yet)",
    bridge_status_running: "Running",
    bridge_status_stopped: "Stopped",
    bridge_copy_command: "Copy manual command",
    bridge_copied: "Copied",
    bridge_new_tab: "New Claude Code tab",
    bridge_tab_badge: "CC",
```

- [ ] **Step 4: 驗證型別**

Run: `npx tsc -b`
Expected: 成功。注意不是 `tsc --noEmit` —— 根目錄的 `tsconfig.json` 是 solution file（`"files": []`），那個指令什麼都不檢查且永遠回 0。

- [ ] **Step 5: Commit**

```bash
git add src/ipc/bridge.ts src/ipc/pty.ts src/lib/i18n.ts
git commit -m "feat(bridge): 前端 IPC 包裝與 i18n 字串"
```

---

### Task 19: 設定頁

**Files:**
- Modify: `src-tauri/src/commands/bridge.rs`（新增存檔指令）
- Modify: `src-tauri/src/lib.rs`（註冊指令）
- Modify: `src/ipc/bridge.ts`
- Create: `src/components/settings/ClaudeBridgePage.tsx`
- Create: `src/components/settings/ClaudeBridgePage.css`
- Create: `src/components/settings/ClaudeBridgePage.test.tsx`
- Modify: `src/components/settings/SettingsView.tsx:3-10, 22, 78-111`

- [ ] **Step 1: 後端新增存檔指令**

在 `src-tauri/src/commands/bridge.rs` 加入：

```rust
use crate::config::types::ClaudeBridgeConfig;

/// 存下橋接設定並立刻套用（啟動或停止 server）。
#[tauri::command]
pub async fn bridge_set_config(
    bridge: State<'_, Arc<BridgeState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    value: ClaudeBridgeConfig,
) -> Result<BridgeStatus, String> {
    config
        .update(|c| c.claude_bridge = value.clone())
        .map_err(|e| e.to_string())?;
    bridge_apply(bridge, config, secrets).await
}
```

`ClaudeBridgeConfig` 已經 derive 了 `Deserialize`，可以直接當指令參數。`ConfigStore::update` 的實際簽名見 `src-tauri/src/config/mod.rs:44`；若它回的是 `anyhow::Result<()>`，上面的 `map_err` 就是對的。

在 `src-tauri/src/lib.rs` 的 use 區塊與 `invoke_handler!` 清單各加上 `bridge_set_config`。

- [ ] **Step 2: 前端補 IPC**

在 `src/ipc/bridge.ts` 加入：

```ts
export interface TierMapping {
  provider_id: string;
  model: string;
}

export interface ClaudeBridgeConfig {
  enabled: boolean;
  port: number;
  default_on_new_tab: boolean;
  opus: TierMapping | null;
  sonnet: TierMapping | null;
  haiku: TierMapping | null;
}

export function bridgeSetConfig(value: ClaudeBridgeConfig): Promise<BridgeStatus> {
  return invoke<BridgeStatus>("bridge_set_config", { value });
}
```

欄位名用 snake_case：`ClaudeBridgeConfig` 的 serde 沒有 `rename_all`，所以序列化出來就是 Rust 的欄位名。（`BridgeStatus` 有 `rename_all = "camelCase"`，兩者不同是刻意的 —— 前者是 config 檔的真實形狀。）

- [ ] **Step 3: 寫失敗的元件測試**

建立 `src/components/settings/ClaudeBridgePage.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ClaudeBridgePage } from "./ClaudeBridgePage";

vi.mock("../../ipc/bridge", () => ({
  bridgeStatus: vi.fn(),
  bridgeSetConfig: vi.fn(),
}));
vi.mock("../../ipc/config", () => ({ getConfig: vi.fn() }));

import { bridgeStatus, bridgeSetConfig } from "../../ipc/bridge";
import { getConfig } from "../../ipc/config";

const PROVIDERS = [
  { id: "qwen", display_name: "本地 Qwen", type: "openai-compatible", model: "Qwen3.6-35B" },
  { id: "cdx", display_name: "Codex", type: "codex", model: "gpt-5" },
];

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getConfig).mockResolvedValue({
    providers: PROVIDERS,
    claude_bridge: {
      enabled: false, port: 8317, default_on_new_tab: false,
      opus: null, sonnet: null, haiku: null,
    },
  } as never);
  vi.mocked(bridgeStatus).mockResolvedValue({
    running: false, port: null, token: null, error: null,
  });
  vi.mocked(bridgeSetConfig).mockResolvedValue({
    running: true, port: 8317, token: "tok", error: null,
  });
});

describe("ClaudeBridgePage", () => {
  it("顯示停止中的狀態", async () => {
    render(<ClaudeBridgePage />);
    expect(await screen.findByText(/未啟動|Stopped/)).toBeInTheDocument();
  });

  it("把不支援的供應商標示出來且不可選", async () => {
    render(<ClaudeBridgePage />);
    const select = await screen.findByLabelText(/Opus/);
    const codex = Array.from(select.querySelectorAll("option")).find((o) =>
      o.textContent?.includes("Codex"),
    );
    expect(codex).toBeDefined();
    expect(codex).toBeDisabled();
  });

  it("選了供應商就帶入它的預設模型", async () => {
    const user = userEvent.setup();
    render(<ClaudeBridgePage />);
    const select = await screen.findByLabelText(/Sonnet/);
    await user.selectOptions(select, "qwen");
    await waitFor(() => {
      expect(screen.getByDisplayValue("Qwen3.6-35B")).toBeInTheDocument();
    });
  });

  it("存檔時把設定送給後端", async () => {
    const user = userEvent.setup();
    render(<ClaudeBridgePage />);
    // 等資料載入完成，按鈕才會 enable —— 直接點會點在停用的按鈕上。
    const save = await screen.findByRole("button", { name: /儲存|Save/ });
    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);
    await waitFor(() => expect(bridgeSetConfig).toHaveBeenCalledTimes(1));
  });

  it("啟動失敗時顯示錯誤而不是拋例外", async () => {
    vi.mocked(bridgeStatus).mockResolvedValue({
      running: false, port: null, token: null,
      error: "無法綁定 127.0.0.1:8317",
    });
    render(<ClaudeBridgePage />);
    expect(await screen.findByText(/無法綁定/)).toBeInTheDocument();
  });
});
```

`getConfig` 回傳型別以 `src/ipc/config.ts` 既有的定義為準；上面的 `as never` 是為了讓 mock 不用湊齊 `AppConfig` 的所有欄位。

- [ ] **Step 4: 執行測試確認失敗**

Run: `npm run test -- ClaudeBridgePage`
Expected: 失敗，找不到 `./ClaudeBridgePage`。

- [ ] **Step 5: 實作元件**

建立 `src/components/settings/ClaudeBridgePage.tsx`：

```tsx
import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { getConfig } from "../../ipc/config";
import {
  bridgeSetConfig,
  bridgeStatus,
  type BridgeStatus,
  type ClaudeBridgeConfig,
  type TierMapping,
} from "../../ipc/bridge";
import "./ClaudeBridgePage.css";

/** M1 支援的 provider type，與 src-tauri/src/bridge/factory.rs 的 kind_for 對齊。 */
const SUPPORTED_TYPES = new Set([
  "openai", "openai-compatible", "ollama", "openrouter",
  "deepseek", "kimi", "xai", "github-copilot",
  "anthropic", "anthropic-compatible",
]);

interface Provider {
  id: string;
  display_name: string;
  type: string;
  model: string;
  auth_method?: string | null;
}

function isSupported(p: Provider): boolean {
  // google-ai 的 oauth 模式走 Antigravity（M3），API key 模式是 OpenAI 相容。
  if (p.type === "google-ai") return p.auth_method !== "oauth";
  return SUPPORTED_TYPES.has(p.type);
}

const TIERS = ["opus", "sonnet", "haiku"] as const;
type TierKey = (typeof TIERS)[number];

export function ClaudeBridgePage() {
  const { t } = useLocale();
  const [providers, setProviders] = useState<Provider[]>([]);
  const [cfg, setCfg] = useState<ClaudeBridgeConfig | null>(null);
  const [status, setStatus] = useState<BridgeStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    void (async () => {
      const [c, s] = await Promise.all([getConfig(), bridgeStatus()]);
      setProviders((c as unknown as { providers: Provider[] }).providers ?? []);
      setCfg((c as unknown as { claude_bridge: ClaudeBridgeConfig }).claude_bridge);
      setStatus(s);
    })();
  }, []);

  const setTier = useCallback(
    (tier: TierKey, providerId: string) => {
      setCfg((prev) => {
        if (!prev) return prev;
        if (!providerId) return { ...prev, [tier]: null };
        const p = providers.find((x) => x.id === providerId);
        const mapping: TierMapping = { provider_id: providerId, model: p?.model ?? "" };
        return { ...prev, [tier]: mapping };
      });
    },
    [providers],
  );

  const setTierModel = useCallback((tier: TierKey, model: string) => {
    setCfg((prev) => {
      const current = prev?.[tier];
      if (!prev || !current) return prev;
      return { ...prev, [tier]: { ...current, model } };
    });
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    setSaving(true);
    try {
      setStatus(await bridgeSetConfig(cfg));
    } finally {
      setSaving(false);
    }
  }, [cfg]);

  const manualCommand = (): string => {
    const port = status?.port ?? cfg?.port ?? 8317;
    const token = status?.token ?? "<token>";
    return [
      `ANTHROPIC_BASE_URL='http://127.0.0.1:${port}'`,
      `ANTHROPIC_AUTH_TOKEN='${token}'`,
      `ANTHROPIC_DEFAULT_OPUS_MODEL='aiterm:opus'`,
      `ANTHROPIC_DEFAULT_SONNET_MODEL='aiterm:sonnet'`,
      `ANTHROPIC_DEFAULT_HAIKU_MODEL='aiterm:haiku'`,
      `API_TIMEOUT_MS=3000000`,
      `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`,
      `claude`,
    ].join(" ");
  };

  if (!cfg) return <div className="bridge-page">…</div>;

  return (
    <div className="bridge-page">
      <h2>{t("bridge_title")}</h2>
      <p className="bridge-desc">{t("bridge_desc")}</p>

      <div className="bridge-status">
        <span className={status?.running ? "bridge-dot bridge-dot--on" : "bridge-dot"} />
        {status?.running ? t("bridge_status_running") : t("bridge_status_stopped")}
        {status?.port ? ` · :${status.port}` : ""}
      </div>
      {status?.error && <div className="bridge-error">{status.error}</div>}

      <label className="bridge-row">
        <input
          type="checkbox"
          checked={cfg.enabled}
          onChange={(e) => setCfg({ ...cfg, enabled: e.target.checked })}
        />
        {t("bridge_enable")}
      </label>

      <label className="bridge-row">
        {t("bridge_port")}
        <input
          type="number"
          value={cfg.port}
          onChange={(e) => setCfg({ ...cfg, port: Number(e.target.value) || 8317 })}
        />
      </label>

      <label className="bridge-row">
        <input
          type="checkbox"
          checked={cfg.default_on_new_tab}
          onChange={(e) => setCfg({ ...cfg, default_on_new_tab: e.target.checked })}
        />
        {t("bridge_default_on_new_tab")}
      </label>

      {TIERS.map((tier) => (
        <div className="bridge-tier" key={tier}>
          <label htmlFor={`bridge-${tier}`}>{t(`bridge_tier_${tier}`)}</label>
          <select
            id={`bridge-${tier}`}
            value={cfg[tier]?.provider_id ?? ""}
            onChange={(e) => setTier(tier, e.target.value)}
          >
            <option value="">{t("bridge_tier_unset")}</option>
            {providers.map((p) => (
              <option key={p.id} value={p.id} disabled={!isSupported(p)}>
                {p.display_name}
                {isSupported(p) ? "" : t("bridge_unsupported_suffix")}
              </option>
            ))}
          </select>
          {cfg[tier] && (
            <input
              aria-label={`${t(`bridge_tier_${tier}`)} ${t("bridge_tier_model")}`}
              value={cfg[tier]!.model}
              onChange={(e) => setTierModel(tier, e.target.value)}
            />
          )}
        </div>
      ))}

      <div className="bridge-actions">
        <button onClick={() => void save()} disabled={saving}>
          {t("common_confirm")}
        </button>
        <button
          onClick={() => {
            void navigator.clipboard.writeText(manualCommand());
            setCopied(true);
          }}
        >
          {copied ? t("bridge_copied") : t("bridge_copy_command")}
        </button>
      </div>
    </div>
  );
}
```

建立 `src/components/settings/ClaudeBridgePage.css`，比照 `GeneralPage.css` 的既有樣式風格；至少要有 `.bridge-page`、`.bridge-row`、`.bridge-tier`、`.bridge-status`、`.bridge-dot`、`.bridge-dot--on`、`.bridge-error`、`.bridge-actions` 這幾個 class。

- [ ] **Step 6: 掛進設定頁**

修改 `src/components/settings/SettingsView.tsx`：

- 第 3–10 行的 import 區塊加入 `import { ClaudeBridgePage } from "./ClaudeBridgePage";`
- 第 22 行的 `SettingsTab` 型別加上 `| "bridge"`
- 側邊欄在 `mcp` 那一項之後加入一個 `bridge` 項目，沿用既有的 `sidebar-item` class 寫法
- 內容區加入 `{tab === "bridge" && <ClaudeBridgePage />}`

- [ ] **Step 7: 執行測試確認通過**

Run: `npm run test -- ClaudeBridgePage`
Expected: 5 passed。

若「存檔時把設定送給後端」失敗且錯誤是點在停用的按鈕上，檢查測試裡的 `waitFor(() => expect(save).toBeEnabled())` 有沒有漏 —— `waitFor` 的第一次檢查是同步的，把 `click` 包在 `waitFor` 裡反而會競態。

- [ ] **Step 8: 型別檢查與 lint**

Run: `npx tsc -b && npm run lint`
Expected: 都成功。

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/ src/ipc/bridge.ts src/components/settings/
git commit -m "feat(bridge): Claude Code 橋接設定頁"
```

---

### Task 20: 分頁 UI

**Files:**
- Modify: `src/components/TerminalView.tsx:807`
- Modify: `src/components/TerminalApp.tsx`
- Modify: `src/components/NewTabPicker/`（新增選項）
- Modify: `src/components/TabBar/`（顯示標記）

- [ ] **Step 1: 讓 TerminalView 接受並傳遞旗標**

在 `src/components/TerminalView.tsx` 的 props 介面加入：

```tsx
  /**
   * 這個分頁是否注入 Claude Code 橋接環境變數。
   * 環境變數只能在 PTY spawn 的瞬間決定，所以這個值在分頁建立後改變沒有效果。
   */
  claudeBridge?: boolean;
```

把第 807 行改成：

```tsx
        const id = await createPty({ rows, cols }, lastCwd, claudeBridge);
```

- [ ] **Step 2: 讓 TerminalApp 記住每個分頁的旗標**

在 `src/components/TerminalApp.tsx` 的分頁狀態型別加入 `claudeBridge?: boolean`，建立分頁的函式多收一個參數，並把它傳給 `<TerminalView>`。新分頁的預設值取自 config 的 `claude_bridge.default_on_new_tab`（用 `getConfig()` 讀一次並存在 state 裡）。

`Ctrl+T` 走預設值；「新增 Claude Code 分頁」則強制傳 `true`。

- [ ] **Step 3: 新分頁選單加入選項**

在 `src/components/NewTabPicker/` 的選項清單加入一項，文字用 `t("bridge_new_tab")`，點下去呼叫建立分頁並帶 `claudeBridge: true`。

橋接未啟動時（`bridgeStatus().running === false`）把這個選項停用，並在 title 提示要先到設定頁啟用 —— 建立一個注入了死埠位址的分頁比不給選更難除錯。

- [ ] **Step 4: 分頁標題顯示標記**

在 `src/components/TabBar/` 渲染分頁標題的地方，`claudeBridge` 為真時加一個小 badge，文字用 `t("bridge_tab_badge")`。

- [ ] **Step 5: 寫測試**

在 `src/components/TerminalApp.tsx` 對應的既有測試檔（若無則建立 `src/components/TerminalApp.bridge.test.tsx`）加入：

```tsx
it("新增 Claude Code 分頁時，createPty 帶 claudeBridge=true", async () => {
  // mock createPty，觸發「新增 Claude Code 分頁」，斷言第三個參數為 true
  expect(vi.mocked(createPty).mock.calls.at(-1)?.[2]).toBe(true);
});

it("一般 Ctrl+T 分頁沿用 default_on_new_tab 設定", async () => {
  // default_on_new_tab: false 時，第三個參數應為 false
  expect(vi.mocked(createPty).mock.calls.at(-1)?.[2]).toBe(false);
});
```

測試骨架照 `src/components/TerminalApp.tsx` 既有測試的 mock 方式撰寫（該檔若尚無測試，參考 `src/components/WarpInput.test.tsx` 的 mock 風格）。

- [ ] **Step 6: 驗證**

Run: `npm run test && npx tsc -b && npm run lint`
Expected: 全綠。

- [ ] **Step 7: Commit**

```bash
git add src/components/
git commit -m "feat(bridge): 分頁層級的橋接開關與標記"
```

---

## 階段 G：驗收

### Task 21: 手動端到端驗收

自動化測試證明不了這件事能用 —— Claude Code 是外部程式，它對 SSE 事件序列的實際要求只有真的跑起來才知道。

**前置：** 準備一個 OpenAI 相容端點（本地 llama.cpp / LM Studio / Ollama 都可以），在 AITerm 設定成一個 provider。

- [ ] **Step 1: 全套自動化驗證**

```bash
cd src-tauri && cargo test && cd .. && npm run test && npx tsc -b && npm run lint
```
Expected: 全綠。任何一項紅的就停下來修，不要帶著紅燈做手動驗收。

- [ ] **Step 2: 設定橋接**

啟動 `npm run tauri:dev`，到設定 → Claude Code 橋接：啟用、三層都指向那個 OpenAI 相容 provider、儲存。確認狀態顯示「執行中」。

- [ ] **Step 3: 驗證 server 活著**

```bash
curl -s http://127.0.0.1:8317/health
```
Expected: `ok`

```bash
curl -s -o /dev/null -w '%{http_code}' -X POST http://127.0.0.1:8317/v1/messages \
  -H 'content-type: application/json' -d '{}'
```
Expected: `401`（沒帶 token）

- [ ] **Step 4: 驗證環境變數注入**

用「新增 Claude Code 分頁」開一個分頁，在裡面執行：

```bash
echo "$ANTHROPIC_BASE_URL / ${ANTHROPIC_API_KEY:-(已清除)}"
```
Expected: `http://127.0.0.1:8317 / (已清除)`

- [ ] **Step 5: 跑完整的工具循環**

在同一個分頁執行 `claude`，然後下一個需要多步工具呼叫的指令，例如：

```
讀 package.json，告訴我 scripts 裡有哪些指令，然後執行 npm run lint
```

Expected：Claude Code 依序呼叫 Read → 回報內容 → 呼叫 Bash 執行 lint → 回報結果。整個過程沒有斷線、沒有 JSON 解析錯誤。

**這一步通過才算 M1 完成。**

- [ ] **Step 6: 驗證慢速上游不斷線**

把三層映射改到一個需要冷啟動的大模型（第一個 token 要等 20 秒以上），重複 Step 5。Expected：Claude Code 耐心等待而非斷線 —— 這驗證的是開場 ping keepalive。

若這裡失敗，`src-tauri/src/bridge/stream.rs` 的 `PING_INTERVAL` 與 ping frame 格式是第一嫌疑；用 `curl -N` 直接看原始 frame 比對。

- [ ] **Step 7: 驗證 Anthropic 轉發路徑**

把三層映射改到一個 `anthropic` 或 `anthropic-compatible` 的 provider，重複 Step 5。

- [ ] **Step 8: 更新 spec 的待驗證假設**

在 `docs/superpowers/specs/2026-08-07-claude-code-bridge-design.md` 的「待驗證假設彙總」一節，把第 1 項（ping frame）與第 4 項（count_tokens 誤差）依實測結果標註為已驗證或已推翻。推翻的話記下實際觀察到的行為。

- [ ] **Step 9: Commit**

```bash
git add docs/superpowers/specs/2026-08-07-claude-code-bridge-design.md
git commit -m "docs(bridge): 依 M1 實測結果更新待驗證假設"
```

---

## M1 完成條件

- [ ] `cd src-tauri && cargo test` 全綠
- [ ] `npm run test` 全綠
- [ ] `npx tsc -b` 成功
- [ ] `npm run lint` 無錯誤
- [ ] Task 21 的 Step 5 手動驗收通過（OpenAI 相容路徑跑完整工具循環）
- [ ] Task 21 的 Step 7 手動驗收通過（Anthropic 轉發路徑）

M2（Codex）與 M3（Antigravity）各自另寫計畫，第一個任務都是「用真憑證 dump 原始 SSE」的探勘測試。

