//! 用量／配額探勘測試（非常規測試，不驗證任何行為）。
//!
//! 目的：在寫任何「顯示使用量」的設計之前，先用真實憑證確認各家上游
//! **到底拿不拿得到訂閱配額資訊、欄位長什麼樣**。全部都是無文件的逆向
//! 端點，所以這裡只負責「打真實請求、把原始回應 dump 到檔案」，不對回
//! 應形狀做任何假設性斷言。
//!
//! 探勘三家（依使用者 config.toml 裡實際存在的訂閱型 provider）：
//!
//!   A. Anthropic OAuth (`anthropic-pro`) —— 候選 usage 端點 + 真實
//!      `/v1/messages` 回應的完整 header（找 `anthropic-ratelimit-*`）。
//!   B. Codex OAuth (`GPT5.6`) —— 候選 usage 端點 + 真實 responses 請求
//!      的完整 header 與 SSE（找 rate limit 快照）。
//!   C. GitHub Copilot (`Github-Sonet4.5`) —— token 交換端點的完整 body
//!      （找 quota snapshot）。
//!
//! 全部是唯讀查詢；A/B 會各送出一次 max_tokens 極小的真實請求，因為
//! rate-limit header 只在真正的推論請求上才會出現。
//!
//! 需要真實憑證與網路，因此整支標 `#[ignore]`：
//!
//!   cargo test --test usage_probe -- --ignored --nocapture

use aiterm_lib::ai::codex::CodexClient;
use aiterm_lib::ai::router::{get_valid_codex_oauth_token, get_valid_oauth_token};
use aiterm_lib::secret::SecretStore;
use std::path::PathBuf;

const ANTHROPIC_PROVIDER_ID: &str = "anthropic-pro";
const CODEX_PROVIDER_ID: &str = "GPT5.6";
const COPILOT_PROVIDER_ID: &str = "Github-Sonet4.5";

const DUMP_DIR: &str =
    "/private/tmp/claude-501/-Users-jamesju-Documents-GitHub-AITERM/48ba95dc-4071-4002-a91e-cd4845517537/scratchpad";

fn write_dump(name: &str, content: &str) {
    let path = PathBuf::from(DUMP_DIR).join(name);
    std::fs::write(&path, content).unwrap_or_else(|e| panic!("寫入 dump 檔 {path:?} 失敗: {e}"));
    println!("[dump] {} ({} bytes)", path.display(), content.len());
}

/// 把回應的 status + **完整 header** + body 格式化成一段可讀文字。
/// header 是這次探勘的主角（`anthropic-ratelimit-*` / `x-codex-*` 之類
/// 都藏在這裡），所以一個都不過濾。
async fn describe(resp: reqwest::Response) -> String {
    let status = resp.status().as_u16();
    let mut headers: Vec<String> = resp
        .headers()
        .iter()
        .map(|(k, v)| format!("  {k}: {}", v.to_str().unwrap_or("<non-utf8>")))
        .collect();
    headers.sort();
    let body = resp.text().await.unwrap_or_else(|e| format!("<讀取 body 失敗: {e}>"));
    format!("HTTP {status}\n--- headers ---\n{}\n--- body ---\n{body}\n", headers.join("\n"))
}

/// 依序打一串候選 GET 端點，把每個的 status/header/body 串成一份報告。
/// 404/401 一樣記錄下來——「哪些端點不存在」也是探勘結果。
async fn probe_candidates(
    client: &reqwest::Client,
    urls: &[&str],
    decorate: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
) -> String {
    let mut report = String::new();
    for url in urls {
        report.push_str(&format!("\n{}\nGET {url}\n{}\n", "=".repeat(72), "=".repeat(72)));
        let built = decorate(client.get(*url));
        match built.send().await {
            Ok(resp) => report.push_str(&describe(resp).await),
            Err(e) => report.push_str(&format!("<請求失敗: {e}>\n")),
        }
    }
    report
}

// ── A. Anthropic OAuth ───────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn probe_anthropic_oauth_usage() {
    let secrets = SecretStore::new();
    let token = get_valid_oauth_token(ANTHROPIC_PROVIDER_ID, &secrets)
        .await
        .expect("取得 Anthropic OAuth token 失敗");

    let client = reqwest::Client::new();
    let oauth_headers = |b: reqwest::RequestBuilder| {
        b.header("Authorization", format!("Bearer {token}"))
            .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
            .header("anthropic-version", "2023-06-01")
            .header("x-app", "cli")
            .header("User-Agent", "claude-cli/2.0.0 (external, cli)")
    };

    // 候選 usage/profile 端點。全都是猜的——這正是探勘的重點。
    let candidates = [
        "https://api.anthropic.com/api/oauth/usage",
        "https://api.anthropic.com/api/oauth/profile",
        "https://api.anthropic.com/api/oauth/claude_cli/usage",
        "https://api.anthropic.com/v1/oauth/usage",
        "https://api.anthropic.com/api/claude_cli/usage",
        "https://api.anthropic.com/api/usage",
    ];
    let mut report = probe_candidates(&client, &candidates, oauth_headers).await;

    // 真實推論請求：rate-limit header 只有在這種請求上才會出現。
    // max_tokens 壓到最小，成本可忽略。
    report.push_str(&format!(
        "\n{}\nPOST /v1/messages (最小請求，為了看 header)\n{}\n",
        "=".repeat(72),
        "=".repeat(72)
    ));
    let body = serde_json::json!({
        "model": "claude-sonnet-4-5",
        "max_tokens": 1,
        "system": [{"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude."}],
        "messages": [{"role": "user", "content": "hi"}],
    });
    let built = oauth_headers(client.post("https://api.anthropic.com/v1/messages"))
        .header("content-type", "application/json")
        .json(&body);
    match built.send().await {
        Ok(resp) => report.push_str(&describe(resp).await),
        Err(e) => report.push_str(&format!("<請求失敗: {e}>\n")),
    }

    write_dump("probe_anthropic_usage.txt", &report);
}

// ── B. Codex OAuth ───────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn probe_codex_usage() {
    let secrets = SecretStore::new();
    let (token, account_id) = get_valid_codex_oauth_token(CODEX_PROVIDER_ID, &secrets)
        .await
        .expect("取得 Codex OAuth token 失敗");
    println!("[info] chatgpt-account-id: {account_id:?}");

    let client = reqwest::Client::new();
    let codex_headers = |b: reqwest::RequestBuilder| {
        let b = b
            .header("Authorization", format!("Bearer {token}"))
            .header("originator", "codex_cli_rs")
            .header("Openai-Beta", "responses=experimental");
        match &account_id {
            Some(id) => b.header("chatgpt-account-id", id.as_str()),
            None => b,
        }
    };

    let candidates = [
        "https://chatgpt.com/backend-api/codex/usage",
        "https://chatgpt.com/backend-api/codex/rate_limits",
        "https://chatgpt.com/backend-api/me",
        "https://chatgpt.com/backend-api/accounts/check/v4-2023-04-27",
        "https://api.openai.com/v1/usage",
    ];
    let mut report = probe_candidates(&client, &candidates, codex_headers).await;

    // 真實 responses 請求：Codex CLI 的 /status 會顯示 5h/週用量，訊息
    // 應該就藏在這種請求的 header 或 SSE 事件裡。用 CodexClient 自己的
    // header 組法，避免跟正式流程有落差。
    report.push_str(&format!(
        "\n{}\nPOST codex/responses (最小請求，為了看 header + SSE)\n{}\n",
        "=".repeat(72),
        "=".repeat(72)
    ));
    let codex = CodexClient::new(token.clone(), "gpt-5.6-luna".into(), account_id.clone());
    let body = serde_json::json!({
        "model": "gpt-5.6-luna",
        "instructions": "You are a helpful assistant.",
        "input": [{"role": "user", "content": [{"type": "input_text", "text": "hi"}]}],
        "stream": true,
        "store": false,
    });
    let built = codex
        .apply_headers(client.post(codex.responses_url()))
        .header("content-type", "application/json")
        .header("Accept", "text/event-stream")
        .json(&body);
    match built.send().await {
        Ok(resp) => report.push_str(&describe(resp).await),
        Err(e) => report.push_str(&format!("<請求失敗: {e}>\n")),
    }

    write_dump("probe_codex_usage.txt", &report);
}

// ── C. GitHub Copilot ────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn probe_copilot_quota() {
    let secrets = SecretStore::new();
    let gh_token = secrets
        .get(COPILOT_PROVIDER_ID)
        .expect("讀取 keychain 失敗")
        .expect("找不到 Copilot 的 GitHub token");

    let client = reqwest::Client::new();
    let gh_headers = |b: reqwest::RequestBuilder| {
        b.header("Authorization", format!("token {gh_token}"))
            .header("User-Agent", "AITerm/1.0")
            .header("Accept", "application/json")
    };

    // v2/token 的 body 近年開始帶 quota_snapshots（premium request 額度）。
    let candidates = [
        "https://api.github.com/copilot_internal/v2/token",
        "https://api.github.com/copilot_internal/user",
    ];
    let report = probe_candidates(&client, &candidates, gh_headers).await;

    write_dump("probe_copilot_quota.txt", &report);
}
