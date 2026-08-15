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

/// Claude Code 自己送來、轉發時要沿用（而非被覆蓋）的標頭。
///
/// 只放這條轉發路徑需要的兩個欄位，不直接吃 axum 的 `HeaderMap`——那會讓
/// upstream 層依賴 axum。呼叫端（`bridge::server` / `bridge::stream`）
/// 負責從 `HeaderMap` 抽出字串。
#[derive(Debug, Default, Clone)]
pub struct ClientHeaders {
    pub beta: Option<String>,
    pub version: Option<String>,
}

/// OAuth 模式下我們必須送的 beta 旗標。定義在 `ai::anthropic`，全專案共用。
use crate::ai::anthropic::OAUTH_BETA_PARTS as REQUIRED_OAUTH_BETA;

/// 合併我們要求的 beta 旗標與客戶端自己宣告的旗標。
///
/// 規則：我們必需的在前、客戶端的接在後，去重且保留順序，用 `,` 連接。
/// `required` 傳空陣列（API key 模式）時就是「原樣保留客戶端的」。
/// 客戶端沒帶、且沒有必需項目時回 `None`（不送這個 header）。
fn merge_beta_header(required: &[&str], client: Option<&str>) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    for r in required {
        if !out.iter().any(|x| x == r) {
            out.push((*r).to_string());
        }
    }
    if let Some(c) = client {
        for part in c.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if !out.iter().any(|x| x == part) {
                out.push(part.to_string());
            }
        }
    }
    if out.is_empty() { None } else { Some(out.join(",")) }
}

pub fn messages_url(base_url: &str) -> String {
    format!("{}/v1/messages", base_url.trim_end_matches('/'))
}

/// 改寫請求 body。`raw` 是 Claude Code 原封不動的 JSON —— 我們不重建它，
/// 因為任何我們沒解析的欄位（cache_control、未來新增的參數）都要原樣送達。
pub fn rewrite_body(raw: &Value, model: &str, is_oauth: bool) -> Value {
    let mut body = raw.clone();
    body["model"] = json!(model);
    // Anthropic 的 Messages API 不接受 messages 裡有 role:"system"（實測回
    // 「role 'system' is not supported on this model」）。system 只能放頂層，
    // 所以把它摺過去——`ai/anthropic.rs` 那條路徑本來就這樣做，這裡補上。
    let folded = fold_system_messages(&mut body);
    if is_oauth {
        body["system"] = ensure_sentinel(body.get("system"));
    }
    if !folded.is_empty() {
        append_system_text(&mut body, &folded);
    }
    strip_unsigned_thinking(&mut body);
    body
}

/// 從 `messages` 取出所有 `role:"system"` 的訊息並回傳其文字（原訊息會被移除）。
fn fold_system_messages(body: &mut Value) -> Vec<String> {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return Vec::new();
    };
    let mut folded = Vec::new();
    messages.retain(|m| {
        if m.get("role").and_then(Value::as_str) != Some("system") {
            return true;
        }
        let text = match m.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if !text.is_empty() {
            folded.push(text);
        }
        false
    });
    folded
}

/// 把摺出來的文字接到頂層 `system` 後面。
///
/// OAuth 模式下 `system` 已經是 block 陣列且第一塊是哨兵——**接在後面**，
/// 插到前面會把哨兵擠掉，上游就會回一個假的 `rate_limit_error`。
fn append_system_text(body: &mut Value, folded: &[String]) {
    let extra: Vec<Value> = folded
        .iter()
        .map(|t| json!({"type": "text", "text": t}))
        .collect();
    match body.get_mut("system") {
        Some(Value::Array(blocks)) => blocks.extend(extra),
        Some(Value::String(s)) => {
            let joined = format!("{}\n\n{}", s, folded.join("\n\n"));
            body["system"] = json!(joined);
        }
        _ => body["system"] = json!(folded.join("\n\n")),
    }
}

/// 上游明確指出「不支援」的參數 → 該從 body 移除的欄位。
///
/// 橋接原樣轉發 Claude Code 的 JSON，只換模型名稱。但 Claude Code 是依「它以為
/// 自己在用的模型」決定要送什麼參數的——使用者 `/model` 選 `aiterm:opus` 時它
/// 會啟用 adaptive thinking，而真正收到請求的可能是 Sonnet，於是 400。
///
/// 用「哪個模型支援什麼」的能力表去擋會過期（每次上游出新模型都要維護），所以
/// 改成信任上游的錯誤訊息：它說哪個參數不支援，就拿掉那個重試一次。
/// 實測 log：拿掉 `thinking` 之後上游改抱怨 `effort`——Claude Code 會依模型等級
/// 一次送出多個專屬參數，所以要一個一個剝，不是剝一次就好。
/// 收到特定的上游錯誤時要對 body 做的修正。
enum Fix {
    /// 移除某個欄位（路徑用 `.` 分層；父層被清空時一併移除）。
    RemovePath(&'static str),
    /// 移除陣列裡序列化後含 `needle` 的元素（陣列清空時連同父層一併移除）。
    RemoveArrayItem { path: &'static str, needle: &'static str },
}

/// 實測到的 body 輪廓：`model messages system tools metadata{user_id} max_tokens
/// output_config{effort} context_management{edits}`——要剝的東西不一定在頂層，
/// 也不一定是物件欄位。
///
/// 注意最後一條是**連鎖**：我們把 `thinking` 剝掉之後，依賴它的
/// `clear_thinking` 策略就變成非法的。剝一個參數可能讓另一個參數失去前提。
const UNSUPPORTED_PARAM_RULES: &[(&str, Fix)] = &[
    ("adaptive thinking is not supported", Fix::RemovePath("thinking")),
    ("does not support the effort parameter", Fix::RemovePath("output_config.effort")),
    (
        "strategy requires `thinking`",
        Fix::RemoveArrayItem { path: "context_management.edits", needle: "clear_thinking" },
    ),
];

/// 最多剝幾次。每次都必須移除一個確實存在的欄位，body 因此單調變小、不會來回
/// 震盪；這個上限只是防止規則表寫錯時打爆上游。
const MAX_PARAM_STRIPS: usize = 4;

/// body 的欄位輪廓（只有欄位名，不含內容）。
///
/// 上游說某個參數不支援、而我們的規則沒對上時，多半是那個欄位不在頂層。有這條
/// 就能一眼看出它藏在哪，不必再往返一輪加臨時 log。只印名字是刻意的——訊息內容
/// 可能含使用者的私密資料。
fn key_outline(body: &Value) -> String {
    let Some(obj) = body.as_object() else { return "(非物件)".into() };
    obj.iter()
        .map(|(k, v)| match v.as_object() {
            Some(inner) if !inner.is_empty() => {
                format!("{k}{{{}}}", inner.keys().cloned().collect::<Vec<_>>().join(","))
            }
            _ => k.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 依上游的 400 訊息移除對應欄位。回傳 `None` 代表沒有可套用的規則
/// （不相干的 400，或該欄位本來就不在 body 裡——後者若回 Some 會變成無謂重試）。
fn strip_unsupported_param(body: &Value, upstream_error: &str) -> Option<Value> {
    for (marker, fix) in UNSUPPORTED_PARAM_RULES {
        if !upstream_error.contains(marker) {
            continue;
        }
        // 目標不在 body 裡就跳過——剝了也沒變化，重試只是白送一次請求。
        let mut fixed = body.clone();
        let changed = match fix {
            Fix::RemovePath(path) => path_exists(body, path) && remove_path(&mut fixed, path),
            Fix::RemoveArrayItem { path, needle } => remove_array_item(&mut fixed, path, needle),
        };
        if changed {
            return Some(fixed);
        }
    }
    None
}

/// 移除 `path` 指到的陣列中、序列化後含 `needle` 的元素。
fn remove_array_item(body: &mut Value, path: &str, needle: &str) -> bool {
    let ptr = format!("/{}", path.replace('.', "/"));
    let Some(arr) = body.pointer_mut(&ptr).and_then(Value::as_array_mut) else {
        return false;
    };
    let before = arr.len();
    arr.retain(|item| !item.to_string().contains(needle));
    if arr.len() == before {
        return false;
    }
    if arr.is_empty() {
        // 陣列空了就連它自己與被清空的父層一起移除，沿用同一套規則。
        remove_path(body, path);
    }
    true
}

fn path_exists(body: &Value, path: &str) -> bool {
    let mut cur = body;
    for seg in path.split('.') {
        match cur.get(seg) {
            Some(v) => cur = v,
            None => return false,
        }
    }
    true
}

/// 移除 `path` 指到的欄位。父層被清空時一併移除——只剩空殼的 `output_config: {}`
/// 同樣可能被上游拒絕，留著等於沒剝乾淨。
fn remove_path(body: &mut Value, path: &str) -> bool {
    let Some((parent_path, leaf)) = path.rsplit_once('.') else {
        // 頂層欄位
        return body.as_object_mut().is_some_and(|o| o.remove(path).is_some());
    };

    let ptr = format!("/{}", parent_path.replace('.', "/"));
    let Some(parent) = body.pointer_mut(&ptr).and_then(Value::as_object_mut) else {
        return false;
    };
    if parent.remove(leaf).is_none() {
        return false;
    }
    if !parent.is_empty() {
        return true;
    }

    // 父層被清空 → 一併移除。
    match parent_path.rsplit_once('.') {
        Some((grand, key)) => {
            let gptr = format!("/{}", grand.replace('.', "/"));
            if let Some(g) = body.pointer_mut(&gptr).and_then(Value::as_object_mut) {
                g.remove(key);
            }
        }
        None => {
            if let Some(o) = body.as_object_mut() {
                o.remove(parent_path);
            }
        }
    }
    true
}

/// 移除歷史訊息裡沒有 `signature` 的 thinking 區塊。
///
/// 三層映射是各自獨立的，使用者可以 sonnet 走 OpenAI、opus 走 Anthropic。
/// 走 OpenAI 那條路徑時我們的序列化器產生的 thinking 區塊沒有簽章（我們簽
/// 不出來），而使用者一旦 /model 切到 opus，這段歷史就會被原樣 POST 到真的
/// api.anthropic.com —— 那邊會驗證 thinking 區塊的簽章。與其賭它會不會 400，
/// 不如轉發前就拿掉。
fn strip_unsigned_thinking(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for m in messages {
        let Some(blocks) = m.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        blocks.retain(|b| {
            let is_thinking = b.get("type").and_then(Value::as_str) == Some("thinking");
            !is_thinking || b.get("signature").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
        });
    }
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
    ///
    /// `client` 是 Claude Code 自己送的 `anthropic-beta` / `anthropic-version`
    /// ——不能整組覆蓋掉：客戶端送 `context_management` 之類欄位時，會在
    /// `anthropic-beta` 裡宣告對應的 beta 旗標，我們如果換成固定字串，上游
    /// 會看到沒宣告 beta 卻出現該欄位而 400。
    pub async fn send_raw(
        &self,
        raw: &Value,
        model: &str,
        client: &ClientHeaders,
    ) -> Result<UpstreamResponse, AiError> {
        let body = rewrite_body(raw, model, self.is_oauth);
        let required: &[&str] = if self.is_oauth { REQUIRED_OAUTH_BETA } else { &[] };
        let beta = merge_beta_header(required, client.beta.as_deref());
        let version = client.version.as_deref().unwrap_or("2023-06-01");

        let post = |payload: &Value| {
            let mut rb = self.client.post(messages_url(&self.base_url)).json(payload);
            rb = if self.is_oauth {
                rb.bearer_auth(&self.token).header("x-app", "cli")
            } else {
                rb.header("x-api-key", &self.token)
            };
            if let Some(beta) = beta.as_deref() {
                rb = rb.header("anthropic-beta", beta);
            }
            rb.header("anthropic-version", version)
        };

        // Claude Code 依「它以為自己在用的模型」送參數，真正的後端模型未必支援。
        // 上游有明講是哪個參數時就剝掉重送——而且要一個一個剝到它不再抱怨為止
        // （實測：拿掉 thinking 之後換 effort 被拒）。
        let mut payload = body;
        for _ in 0..=MAX_PARAM_STRIPS {
            let resp = post(&payload)
                .send()
                .await
                .map_err(|e| AiError::Network { message: e.to_string() })?;

            let status = resp.status();
            if status.is_success() {
                return Ok(UpstreamResponse::Passthrough(resp));
            }
            if status.as_u16() != 400 {
                return Err(crate::ai::sse::map_http_error(status, resp).await);
            }

            let text = resp.text().await.unwrap_or_default();
            match strip_unsupported_param(&payload, &text) {
                Some(fixed) => {
                    log::warn!(
                        "上游拒絕了 Claude Code 帶來的參數，移除後重試：{}",
                        crate::ai::sse::truncate(&text, 200)
                    );
                    payload = fixed;
                }
                None => {
                    // 沒有規則對上。印出欄位輪廓，讓「上游說不支援的那個參數究竟
                    // 在 body 的哪裡」下次一看就知道。
                    log::warn!(
                        "上游回 400 但沒有可套用的剝除規則。body 欄位：{}｜上游訊息：{}",
                        key_outline(&payload),
                        crate::ai::sse::truncate(&text, 300)
                    );
                    return Err(AiError::Network {
                        message: format!("http 400: {}", crate::ai::sse::truncate(&text, 300)),
                    });
                }
            }
        }
        Err(AiError::Network {
            message: format!("上游連續拒絕參數超過 {MAX_PARAM_STRIPS} 次，放棄重試"),
        })
    }
}

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
    fn unsigned_thinking_blocks_are_stripped_from_history() {
        // 走 OpenAI 路徑產生的 thinking 區塊沒有簽章，使用者切到 Anthropic
        // 層之後這段歷史會被原樣轉發，上游會驗簽。
        let body = rewrite_body(
            &json!({"model": "m", "messages": [{"role": "assistant", "content": [
                {"type": "thinking", "thinking": "沒簽章"},
                {"type": "text", "text": "保留"}
            ]}]}),
            "m",
            false,
        );
        let blocks = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
    }

    #[test]
    fn signed_thinking_blocks_survive() {
        let body = rewrite_body(
            &json!({"model": "m", "messages": [{"role": "assistant", "content": [
                {"type": "thinking", "thinking": "有簽章", "signature": "sig-abc"}
            ]}]}),
            "m",
            false,
        );
        assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn string_content_messages_are_left_alone() {
        let body = rewrite_body(
            &json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]}),
            "m",
            false,
        );
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn messages_url_appends_v1_messages() {
        assert_eq!(messages_url("https://api.anthropic.com"), "https://api.anthropic.com/v1/messages");
        assert_eq!(messages_url("https://x.test/"), "https://x.test/v1/messages");
    }

    #[test]
    fn oauth_without_client_beta_yields_only_required() {
        let merged = merge_beta_header(REQUIRED_OAUTH_BETA, None);
        assert_eq!(merged.as_deref(), Some("claude-code-20250219,oauth-2025-04-20"));
    }

    #[test]
    fn oauth_with_client_beta_appends_after_required_in_order() {
        let merged = merge_beta_header(REQUIRED_OAUTH_BETA, Some("context-management-2025-06-27"));
        assert_eq!(
            merged.as_deref(),
            Some("claude-code-20250219,oauth-2025-04-20,context-management-2025-06-27")
        );
    }

    #[test]
    fn oauth_does_not_duplicate_a_flag_the_client_already_sent() {
        // Claude Code 有時自己也會帶 oauth-2025-04-20，不能合併成兩份。
        let merged = merge_beta_header(REQUIRED_OAUTH_BETA, Some("oauth-2025-04-20"));
        assert_eq!(merged.as_deref(), Some("claude-code-20250219,oauth-2025-04-20"));
    }

    #[test]
    fn api_key_mode_keeps_client_beta_as_is_without_adding_ours() {
        let merged = merge_beta_header(&[], Some("context-management-2025-06-27"));
        assert_eq!(merged.as_deref(), Some("context-management-2025-06-27"));
    }

    #[test]
    fn api_key_mode_without_client_beta_sets_no_header() {
        assert_eq!(merge_beta_header(&[], None), None);
    }

    #[test]
    fn client_beta_list_is_trimmed_when_split() {
        let merged = merge_beta_header(&[], Some("a, b"));
        assert_eq!(merged.as_deref(), Some("a,b"));
    }
}
