//! 訂閱登入（OAuth）共用程式：token 交換、續期與一次性呼叫。
//!
//! 這裡的常數與線上格式**以 AITerm 的實作為準**（`commands/provider.rs`、`ai/anthropic.rs`、
//! `ai/codex.rs`、`ai/antigravity.rs`，已用真實帳號驗證過）；AITerm 端有測試斷言兩邊一致，
//! 日後改了一邊會立刻紅燈。不依賴 tauri、pty，Server 與 AITerm 的「為 server 登入」共用。
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub mod anthropic;
pub mod antigravity;
pub mod codex;

/// 一次登入的 PKCE 資料。`state` 與 `verifier` 只存在記憶體，登入完成就丟掉。
#[derive(Debug, Clone, PartialEq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl Pkce {
    pub fn new() -> Self {
        let verifier = URL_SAFE_NO_PAD.encode([Uuid::new_v4().as_bytes().as_slice(), Uuid::new_v4().as_bytes().as_slice()].concat());
        let challenge = challenge_of(&verifier);
        // 24 位元組 → 32 個 base64url 字元，與其他 OAuth 用戶端相同的格式。
        let (b1, b2) = (Uuid::new_v4(), Uuid::new_v4());
        let mut bytes = Vec::with_capacity(24);
        bytes.extend_from_slice(b1.as_bytes());
        bytes.extend_from_slice(&b2.as_bytes()[..8]);
        Self { verifier, challenge, state: URL_SAFE_NO_PAD.encode(bytes) }
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

pub fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// 取得的憑證。`id_token` 只有 Codex 會給（要從裡面取帳號 id）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Tokens {
    #[serde(rename = "access_token")]
    pub access: String,
    #[serde(rename = "refresh_token", default)]
    pub refresh: Option<String>,
    #[serde(rename = "expires_in", default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub id_token: Option<String>,
}

/// 供應商回的錯誤原因，截短後附在錯誤訊息後面。
pub(crate) fn detail(v: &serde_json::Value) -> String {
    let msg = v["error"]["message"].as_str().or_else(|| v["error"].as_str()).or_else(|| v["detail"].as_str()).or_else(|| v["message"].as_str()).unwrap_or("");
    if msg.trim().is_empty() { String::new() } else { format!("：{}", msg.trim().chars().take(200).collect::<String>()) }
}

pub(crate) fn client(timeout: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout)).build().map_err(|e| e.to_string())
}

/// 取 SSE 文字裡每個 `data:` 行的內容（已去掉前綴與空白）。
pub(crate) fn sse_data_lines(body: &str) -> impl Iterator<Item = &str> {
    body.lines().filter_map(|l| l.trim().strip_prefix("data:")).map(str::trim).filter(|d| !d.is_empty() && *d != "[DONE]")
}
