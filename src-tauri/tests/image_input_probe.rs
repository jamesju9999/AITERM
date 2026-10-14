//! 探勘測試（不驗證行為）：Ask AI 附圖時，各家供應商到底看不看得到圖。
//!
//! 圖是左黃右紫的 64×64 PNG，問左右各是什麼顏色。答對兩邊＝模型真的看到圖；
//! 說看不到、答錯、或報錯＝沒看到。只跑 `PROBE_IDS`（逗號分隔）指定的供應商，沒指定就全跑。
//!
//! A 組：走 App 現在的路徑（`AiRouter::resolve_by_id` → `generate`／`generate_with_tools`），
//!       訊息內容是前端送的 OpenAI 格式 `[{type:text},{type:image_url}]`。
//! B 組：同一張圖改用各家原生格式直接打端點（Anthropic image 區塊、Codex input_image、
//!       Gemini inlineData），看「換格式」能不能讓模型看到。
//!
//! 需要真實憑證與網路，整支 `#[ignore]`：
//!
//!   cargo test --test image_input_probe -- --ignored --nocapture --test-threads=1
//!
//! 結果 dump 到 `AITERM_PROBE_DIR`（預設 /tmp）。

use aiterm_lib::ai::router::{get_valid_codex_oauth_token, get_valid_google_oauth_token, get_valid_oauth_token, AiRouter};
use aiterm_lib::ai::{
    antigravity, codex, ChatMessage, EnvSnapshot, GenerateRequest, GenerateWithToolsResult, McpToolDefinition, QueryMode,
};
use aiterm_lib::config::ConfigStore;
use aiterm_lib::secret::SecretStore;
use aiterm_lib::usage::UsageStore;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

/// 64×64 PNG：左半黃（255,220,0）、右半紫（128,0,160）。純色會被猜中（第一輪用純藍，Codex 走「圖片變文字」的路徑也答 blue），兩色左右分割幾乎猜不到。
const BLUE_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAAVUlEQVR4nO3PQQ0AMAwDsUAvlAEblw1FPpWlA3DOu6k2OdW69wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAArAB9vhNlay+pm9gAAAABJRU5ErkJggg==";
const QUESTION: &str = "What color is the LEFT half of this image, and what color is the RIGHT half? Answer as `left: <color>, right: <color>`. If you cannot see any image, answer exactly NO_IMAGE.";

fn dump_dir() -> PathBuf {
    PathBuf::from(std::env::var("AITERM_PROBE_DIR").unwrap_or_else(|_| "/tmp".into()))
}

fn record(report: &mut String, label: &str, outcome: &str) {
    let short: String = outcome.chars().take(600).collect();
    let line = format!("== {label}\n{short}\n\n");
    println!("{line}");
    report.push_str(&line);
}

fn data_uri() -> String {
    format!("data:image/png;base64,{BLUE_PNG_B64}")
}

/// 前端 buildContentParts 產生的格式（OpenAI 風格）。
fn openai_style_message() -> ChatMessage {
    ChatMessage {
        role: "user".into(),
        content: serde_json::json!([
            {"type": "text", "text": QUESTION},
            {"type": "image_url", "image_url": {"url": data_uri()}}
        ]),
        tool_call_id: None,
        tool_calls: None,
    }
}

fn request(messages: Vec<ChatMessage>) -> GenerateRequest {
    GenerateRequest {
        system_prompt: "You are a helpful assistant.".into(),
        messages,
        context: EnvSnapshot::default(),
        mode: QueryMode::Chat,
        max_tokens: Some(200),
    }
}

fn dummy_tool() -> McpToolDefinition {
    McpToolDefinition {
        name: "probe__noop".into(),
        description: "Does nothing. Never call it.".into(),
        input_schema: serde_json::json!({"type": "object", "properties": {}}),
    }
}

async fn router() -> AiRouter {
    let usage = UsageStore::new_at(dump_dir().join("image_probe_usage.db")).await;
    AiRouter::new(Arc::new(ConfigStore::new()), Arc::new(SecretStore::new()), Arc::new(usage))
}

/// A 組：App 現在的兩條路徑（純聊天走 generate；⚡ 走 generate_with_tools）。
async fn current_paths(report: &mut String, provider_id: &str) {
    let r = router().await;
    let provider = match r.resolve_by_id(provider_id).await {
        Ok(p) => p,
        Err(e) => return record(report, &format!("A {provider_id} resolve"), &format!("ERR {e:?}")),
    };

    let (tx, mut rx) = mpsc::channel(256);
    let res = provider.generate(request(vec![openai_style_message()]), tx).await;
    let mut text = String::new();
    while let Ok(c) = rx.try_recv() {
        text.push_str(&c.delta);
    }
    record(report, &format!("A {provider_id} generate"), &format!("{res:?}\n{text}"));

    let (tx, mut rx) = mpsc::channel(256);
    let res = provider.generate_with_tools(request(vec![openai_style_message()]), vec![dummy_tool()], tx).await;
    let mut streamed = String::new();
    while let Ok(c) = rx.try_recv() {
        streamed.push_str(&c.delta);
    }
    let shown = match &res {
        Ok(GenerateWithToolsResult::Text(t)) => format!("Text: {t}"),
        Ok(GenerateWithToolsResult::ToolCalls { calls, .. }) => format!("ToolCalls: {calls:?}"),
        Ok(GenerateWithToolsResult::Unsupported) => "Unsupported".into(),
        Err(e) => format!("ERR {e:?}"),
    };
    record(report, &format!("A {provider_id} generate_with_tools"), &format!("{shown}\nstreamed: {streamed}"));
}

fn sse_text(raw: &str, pick: impl Fn(&serde_json::Value) -> Option<String>) -> String {
    raw.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|s| serde_json::from_str::<serde_json::Value>(s.trim()).ok())
        .filter_map(|v| pick(&v))
        .collect()
}

#[tokio::test]
#[ignore]
async fn probe_image_input_all_providers() {
    let mut report = String::new();
    let wanted = std::env::var("PROBE_IDS").ok();
    let want = |id: &str| wanted.as_deref().map_or(true, |w| w.split(',').any(|x| x == id));
    for id in ["anthropic-pro", "GPT5.6", "Gemini2.5", "gemma-4-26b-a4b-it-4bit", "Github-Sonet4.5", "Gemini-3.5-flash"] {
        if want(id) {
            current_paths(&mut report, id).await;
        }
    }

    let secrets = SecretStore::new();
    let http = reqwest::Client::new();

    // ── B1 Anthropic 原生 image 區塊（OAuth；header 與 usage_probe 相同）──
    if want("anthropic-pro") { match get_valid_oauth_token("anthropic-pro", &secrets).await {
        Ok(token) => {
            let body = serde_json::json!({
                "model": "claude-sonnet-4-5",
                "max_tokens": 50,
                "system": [{"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude."}],
                "messages": [{"role": "user", "content": [
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": BLUE_PNG_B64}},
                    {"type": "text", "text": QUESTION}
                ]}],
            });
            let resp = http
                .post("https://api.anthropic.com/v1/messages")
                .header("Authorization", format!("Bearer {token}"))
                .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
                .header("anthropic-version", "2023-06-01")
                .header("x-app", "cli")
                .header("User-Agent", "claude-cli/2.0.0 (external, cli)")
                .json(&body)
                .send()
                .await;
            let out = match resp {
                Ok(r) => format!("HTTP {}\n{}", r.status(), r.text().await.unwrap_or_default()),
                Err(e) => format!("ERR {e}"),
            };
            record(&mut report, "B anthropic native image block", &out);
        }
        Err(e) => record(&mut report, "B anthropic token", &format!("ERR {e:?}")),
    } }

    // ── B2 Codex 原生 input_image ──
    if want("GPT5.6") { match get_valid_codex_oauth_token("GPT5.6", &secrets).await {
        Ok((token, account)) => {
            let client = codex::CodexClient::new(token, "gpt-5.6-luna".into(), account);
            let mut body = codex::build_request_body("gpt-5.6-luna", &request(vec![ChatMessage {
                role: "user".into(),
                content: serde_json::json!("placeholder"),
                tool_call_id: None,
                tool_calls: None,
            }]));
            body["input"][0]["content"] = serde_json::json!([
                {"type": "input_text", "text": QUESTION},
                {"type": "input_image", "image_url": data_uri()}
            ]);
            let resp = client
                .apply_headers(http.post(client.responses_url()))
                .header("Accept", "text/event-stream")
                .json(&body)
                .send()
                .await;
            let out = match resp {
                Ok(r) => {
                    let status = r.status();
                    let raw = r.text().await.unwrap_or_default();
                    let text = sse_text(&raw, |v| {
                        (v["type"] == "response.output_text.delta").then(|| v["delta"].as_str().unwrap_or("").to_string())
                    });
                    format!("HTTP {status}\ntext: {text}\nraw head: {}", raw.chars().take(400).collect::<String>())
                }
                Err(e) => format!("ERR {e}"),
            };
            record(&mut report, "B codex native input_image", &out);
        }
        Err(e) => record(&mut report, "B codex token", &format!("ERR {e:?}")),
    } }

    // ── B3 Antigravity（Gemini）原生 inlineData ──
    if want("Gemini2.5") { match get_valid_google_oauth_token("Gemini2.5", &secrets).await {
        Ok((token, project)) => {
            let client = antigravity::AntigravityClient::new(token, project.clone(), "gemini-2.5-flash".into());
            let mut body = antigravity::build_request_body("gemini-2.5-flash", &project, &request(vec![ChatMessage {
                role: "user".into(),
                content: serde_json::json!("placeholder"),
                tool_call_id: None,
                tool_calls: None,
            }]));
            body["request"]["contents"][0]["parts"] = serde_json::json!([
                {"text": QUESTION},
                {"inlineData": {"mimeType": "image/png", "data": BLUE_PNG_B64}}
            ]);
            let resp = client.apply_headers(http.post(client.generate_content_url())).json(&body).send().await;
            let out = match resp {
                Ok(r) => {
                    let status = r.status();
                    let raw = r.text().await.unwrap_or_default();
                    let text = sse_text(&raw, |v| {
                        v["response"]["candidates"][0]["content"]["parts"].as_array().map(|ps| {
                            ps.iter().filter_map(|p| p["text"].as_str()).collect::<String>()
                        })
                    });
                    format!("HTTP {status}\ntext: {text}\nraw head: {}", raw.chars().take(400).collect::<String>())
                }
                Err(e) => format!("ERR {e}"),
            };
            record(&mut report, "B antigravity native inlineData", &out);
        }
        Err(e) => record(&mut report, "B antigravity token", &format!("ERR {e:?}")),
    } }

    let path = dump_dir().join("image_input_probe.txt");
    std::fs::write(&path, &report).unwrap();
    println!("[dump] {}", path.display());
}
