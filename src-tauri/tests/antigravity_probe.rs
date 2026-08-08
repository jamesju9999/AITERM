//! M3 探勘測試（非常規測試，不驗證任何行為）——直接用真實 Antigravity
//! (Google 帳號訂閱制 OAuth) 憑證打私有端點
//! `cloudcode-pa.googleapis.com/v1internal:streamGenerateContent`，記錄：
//!
//! 1. 該端點接不接受 Gemini 原生格式的 `tools`
//!    （`request.tools = [{"functionDeclarations":[...]}]`）。
//! 2. 若接受，`functionCall` 在 SSE 事件裡實際長什麼樣（有沒有 id、參數
//!    一次到齊還是分片、`thought` part 長什麼樣）。
//! 3. ⚠️ 重點：有沒有 `thought_signature`（或其他不透明欄位）往返要求——
//!    M1 在 Gemini API key 路徑上踩過這個坑，Antigravity 底層也是 Gemini，
//!    做「原樣回送 vs 最小重建」對照實驗來確診。
//! 4. 多輪回送 `functionResponse` 的有效格式。
//! 5. `project` 欄位（`get_valid_google_oauth_token` 拿不拿得到）。
//!
//! 端點是無文件的逆向端點，所以本檔案只負責「打真實請求、把原始 SSE
//! dump 到檔案」，不對回應形狀做任何假設性斷言——那是 M3 adapter 實作
//! 階段的事，不是這裡的事。
//!
//! 需要真實憑證（provider id `Gemini2.5`）與網路，因此整支標 `#[ignore]`，
//! 不會進常規 `cargo test` 流程。執行方式：
//!
//!   cargo test --test antigravity_probe -- --ignored --nocapture
//!
//! Access token 續期窗口是 900 秒且 refresh token 不輪替，一律透過
//! `aiterm_lib::ai::router::get_valid_google_oauth_token` 取得/刷新，
//! 不自己重寫刷新邏輯。

use aiterm_lib::ai::antigravity::{build_request_body, AntigravityClient};
use aiterm_lib::ai::router::get_valid_google_oauth_token;
use aiterm_lib::ai::{ChatMessage, EnvSnapshot, GenerateRequest, QueryMode};
use aiterm_lib::bridge::anthropic::request::MessagesRequest;
use aiterm_lib::bridge::tool_meta::ToolMetaCache;
use aiterm_lib::bridge::upstream::antigravity::request::build_body;
use aiterm_lib::secret::SecretStore;
use std::path::PathBuf;

const PROVIDER_ID: &str = "Gemini2.5";
// 讀自使用者的 config.toml（[[providers]] id = "Gemini2.5"）。
const MODEL: &str = "gemini-2.5-flash";

const DUMP_DIR: &str =
    "/private/tmp/claude-501/-Users-jamesju-Documents-GitHub-AITERM/e08874e0-db22-4a64-aec9-86efc165d3c5/scratchpad";

fn dump_path(name: &str) -> PathBuf {
    PathBuf::from(DUMP_DIR).join(name)
}

fn write_dump(name: &str, content: &str) {
    let path = dump_path(name);
    std::fs::write(&path, content).unwrap_or_else(|e| panic!("寫入 dump 檔 {path:?} 失敗: {e}"));
    println!("[dump] {} ({} bytes)", path.display(), content.len());
}

/// 把一次 HTTP 回應的 status + raw body 記下來，回傳 body（給後續解析用）。
async fn dump_response(resp: reqwest::Response, dump_name: &str) -> (u16, String) {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_else(|e| format!("<讀取 body 失敗: {e}>"));
    write_dump(dump_name, &format!("HTTP {status}\n\n{body}"));
    (status, body)
}

/// 從一段 SSE 純文字裡把每個 `data: {...}` 行解析成 JSON。非 JSON / 空行
/// 直接跳過，不視為錯誤——這是探勘，不是嚴格 parser。
fn parse_sse_events(raw: &str) -> Vec<serde_json::Value> {
    raw.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter_map(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .collect()
}

/// Antigravity 把標準 Gemini payload 包在 `{"response": {...}, "traceId":
/// ..., "metadata": {}}` 信封裡（見 antigravity.rs 既有實作與其測試）。
/// 探勘要看的是信封拆開後的 response 本體。
fn unwrap_response<'a>(ev: &'a serde_json::Value) -> &'a serde_json::Value {
    ev.get("response").unwrap_or(ev)
}

fn env_ctx() -> EnvSnapshot {
    EnvSnapshot { os: "macos".into(), shell: "zsh".into(), cwd: PathBuf::from("/"), ..Default::default() }
}

/// Gemini 原生格式的 functionDeclarations 工具宣告（注意跟 OpenAI 相容/
/// Responses API 的扁平 `{"type":"function","name":...}` 形狀不同）。
fn weather_tools() -> serde_json::Value {
    serde_json::json!([{
        "functionDeclarations": [{
            "name": "get_weather",
            "description": "Get the current weather for a given city",
            "parameters": {
                "type": "object",
                "properties": {
                    "city": { "type": "string", "description": "City name" }
                },
                "required": ["city"]
            }
        }]
    }])
}

#[tokio::test]
#[ignore = "探勘測試：需要真實 Antigravity OAuth 憑證與網路，不進常規 CI"]
async fn probe_antigravity_tools_support_and_multiturn() {
    // ---- 0. 拿有效 access token + project id（會自動刷新）----
    let secrets = SecretStore::new();
    let (access_token, project_id) = get_valid_google_oauth_token(PROVIDER_ID, &secrets)
        .await
        .unwrap_or_else(|e| panic!("BLOCKED: 拿不到 {PROVIDER_ID} 的有效 token/project_id: {e:?}"));
    println!(
        "[probe] got token (len={}), project_id={:?} (len={})",
        access_token.len(),
        &project_id[..project_id.len().min(6)],
        project_id.len()
    );
    write_dump(
        "antigravity_probe_project_id_confirmation.txt",
        &format!(
            "token_len={}\nproject_id_len={}\nproject_id_prefix={}\n",
            access_token.len(),
            project_id.len(),
            &project_id[..project_id.len().min(10)]
        ),
    );

    let client = AntigravityClient::new(access_token, project_id.clone(), MODEL.into());
    let http = reqwest::Client::new();

    // ==================== Q1+Q2: 帶 tools 的第一輪 ====================
    let round1_req = GenerateRequest {
        system_prompt: "You are a coding CLI assistant. When the user asks about the weather, you must call the get_weather tool rather than answering from memory.".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!("What's the weather in Taipei right now? Use the tool, don't guess."),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: env_ctx(),
        mode: QueryMode::Chat,
        max_tokens: Some(512),
    };

    let tools = weather_tools();
    let mut round1_body = build_request_body(MODEL, &project_id, &round1_req);
    round1_body["request"]["tools"] = tools.clone();

    write_dump("antigravity_probe_round1_request.json", &serde_json::to_string_pretty(&round1_body).unwrap());

    let resp = client
        .apply_headers(http.post(client.generate_content_url()))
        .json(&round1_body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round1 request failed to send: {e}"));

    let (status, body) = dump_response(resp, "antigravity_probe_round1_raw_sse.txt").await;
    println!("[probe] round1 status = {status}");

    if status >= 400 {
        println!(
            "[probe] === 端點拒絕帶 tools 的請求 (HTTP {status}) === 完整內容見 antigravity_probe_round1_raw_sse.txt"
        );
        // 端點回 4xx 本身就是這題的答案，探勘到此為止——不繼續測多輪。
        return;
    }

    let events = parse_sse_events(&body);
    println!("[probe] round1 SSE event count = {}", events.len());

    // Dump 全部 parts（不管認不認得的欄位），逐條列出每個 part 的 key 集合，
    // 方便肉眼確認 functionCall / thought / inlineData 分別長怎樣。
    let mut all_parts: Vec<serde_json::Value> = Vec::new();
    for ev in &events {
        let resp_body = unwrap_response(ev);
        if let Some(candidates) = resp_body.get("candidates").and_then(|c| c.as_array()) {
            for cand in candidates {
                if let Some(parts) = cand.get("content").and_then(|c| c.get("parts")).and_then(|p| p.as_array()) {
                    for part in parts {
                        all_parts.push(part.clone());
                    }
                }
            }
        }
    }
    write_dump("antigravity_probe_round1_all_parts.json", &serde_json::to_string_pretty(&all_parts).unwrap());
    println!("[probe] round1 collected {} parts across all events:", all_parts.len());
    for p in &all_parts {
        let keys: Vec<&String> = p.as_object().map(|o| o.keys().collect()).unwrap_or_default();
        println!("  - keys={keys:?} raw={}", serde_json::to_string(p).unwrap());
    }

    // 找出所有帶 functionCall 的 part（可能分散在多個串流事件裡）。
    let function_call_parts: Vec<serde_json::Value> =
        all_parts.iter().filter(|p| p.get("functionCall").is_some()).cloned().collect();

    write_dump(
        "antigravity_probe_round1_function_call_parts.json",
        &serde_json::to_string_pretty(&function_call_parts).unwrap(),
    );

    if function_call_parts.is_empty() {
        println!(
            "[probe] === 模型這輪沒有呼叫任何 tool（可能忽略了工具或直接用文字回答）。all_parts 見 dump 檔 ==="
        );
        return;
    }

    println!("[probe] found {} functionCall part(s)", function_call_parts.len());
    for fc in &function_call_parts {
        println!("  - {}", serde_json::to_string(fc).unwrap());
    }

    // 掃描整個事件流找 finishReason / usageMetadata 出現的位置。
    let finish_and_usage: Vec<serde_json::Value> = events
        .iter()
        .filter_map(|ev| {
            let resp_body = unwrap_response(ev);
            let finish = resp_body
                .get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|c| c.get("finishReason"));
            let usage = resp_body.get("usageMetadata");
            if finish.is_some() || usage.is_some() {
                Some(serde_json::json!({ "finishReason": finish, "usageMetadata": usage }))
            } else {
                None
            }
        })
        .collect();
    write_dump(
        "antigravity_probe_round1_finish_and_usage.json",
        &serde_json::to_string_pretty(&finish_and_usage).unwrap(),
    );
    println!("[probe] finishReason/usageMetadata events: {}", finish_and_usage.len());
    for e in &finish_and_usage {
        println!("  - {}", serde_json::to_string(e).unwrap());
    }

    // ==================== Q3+Q4: 多輪回送，帶 vs 不帶 thought_signature 對照 ====================
    // 取第一個 functionCall part 做多輪回送實驗。
    let first_fc_part = function_call_parts[0].clone();
    let fc_obj = first_fc_part.get("functionCall").cloned().unwrap_or(serde_json::Value::Null);
    let fn_name = fc_obj.get("name").and_then(|v| v.as_str()).unwrap_or("get_weather").to_string();

    let base_contents = round1_body["request"]["contents"].as_array().cloned().unwrap_or_default();

    // 對照組 A：原樣把 round1 回應裡「這個 candidate 的整組 parts」塞回去
    // 當作一個 model turn（含任何不透明欄位，例如可能存在的
    // thoughtSignature），再接一個 user turn 帶 functionResponse。
    let model_turn_full: serde_json::Value = serde_json::json!({ "role": "model", "parts": all_parts.clone() });
    let function_response_turn = serde_json::json!({
        "role": "user",
        "parts": [{
            "functionResponse": {
                "name": fn_name,
                "response": { "temperature_c": 29, "condition": "partly cloudy" }
            }
        }]
    });

    let mut contents_a = base_contents.clone();
    contents_a.push(model_turn_full);
    contents_a.push(function_response_turn.clone());
    let mut body_a = round1_body.clone();
    body_a["request"]["contents"] = serde_json::Value::Array(contents_a);
    body_a["request"]["tools"] = tools.clone();
    // requestId 要換新的，避免被當成重複請求。
    body_a["requestId"] = serde_json::json!(format!("agent/probe/{}", uuid_like()));

    write_dump("antigravity_probe_round2a_full_replay_request.json", &serde_json::to_string_pretty(&body_a).unwrap());
    let resp_a = client
        .apply_headers(http.post(client.generate_content_url()))
        .json(&body_a)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round2a request failed to send: {e}"));
    let (status_a, _body_a) = dump_response(resp_a, "antigravity_probe_round2a_full_replay_raw_sse.txt").await;
    println!("[probe] round2a (WITH full original parts replayed, incl. any opaque fields) status = {status_a}");

    // 對照組 B：最小重建——只留 functionCall{name, args}，丟掉任何其他欄位
    // （例如 thoughtSignature，如果有的話）。這組是用來確診「有沒有欄位是
    // 必須原樣回送的」。
    let minimal_fc_part = serde_json::json!({
        "functionCall": {
            "name": fn_name,
            "args": fc_obj.get("args").cloned().unwrap_or(serde_json::json!({}))
        }
    });
    let model_turn_minimal = serde_json::json!({ "role": "model", "parts": [minimal_fc_part] });

    let mut contents_b = base_contents.clone();
    contents_b.push(model_turn_minimal);
    contents_b.push(function_response_turn.clone());
    let mut body_b = round1_body.clone();
    body_b["request"]["contents"] = serde_json::Value::Array(contents_b);
    body_b["request"]["tools"] = tools.clone();
    body_b["requestId"] = serde_json::json!(format!("agent/probe/{}", uuid_like()));

    write_dump("antigravity_probe_round2b_minimal_request.json", &serde_json::to_string_pretty(&body_b).unwrap());
    let resp_b = client
        .apply_headers(http.post(client.generate_content_url()))
        .json(&body_b)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round2b request failed to send: {e}"));
    let (status_b, body_b_raw) = dump_response(resp_b, "antigravity_probe_round2b_minimal_raw_sse.txt").await;
    println!("[probe] round2b (minimal reconstructed functionCall, no opaque fields) status = {status_b}");

    let events_a_types_present = status_a < 400;
    let events_b_types_present = status_b < 400;

    let summary = format!(
        "round1 status={status}\n\
         round1 functionCall part(s) found: {}\n\
         round1 all distinct part key-sets: {:?}\n\
         round2a (full original parts replayed, incl. any opaque fields) status={status_a} ok={events_a_types_present}\n\
         round2b (minimal reconstructed: functionCall{{name,args}} only, no opaque fields) status={status_b} ok={events_b_types_present}\n\
         結論指引：若 status_a=200 且 status_b=400 → 有不透明欄位（例如 thoughtSignature）\
         是往返必須的，且它就在 functionCall part 本身（不是分開的頂層欄位）。\n\
         若兩者皆 200 → 這次探勘沒有觀察到 thought_signature 往返要求（不代表其他情境下不存在）。\n\
         見同目錄下 antigravity_probe_round1_raw_sse.txt / antigravity_probe_round2a_full_replay_raw_sse.txt / \
         antigravity_probe_round2b_minimal_raw_sse.txt 取得完整原始內容。",
        function_call_parts.len(),
        all_parts.iter().map(|p| p.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default()).collect::<Vec<_>>()
    );
    write_dump("antigravity_probe_SUMMARY.txt", &summary);
    println!("[probe] === SUMMARY ===\n{summary}");

    if status_b >= 400 {
        println!("[probe] round2b body (for inspecting the exact 4xx reason): {body_b_raw}");
    }
}

// ============================================================================
// M3 驗收 bug 修正驗證：真實 Claude Code 打過來時，上游回 400
// "Unknown name \"$schema\" ... Cannot find field"——Gemini 的
// Schema 型別只接受 OpenAPI Schema 的受限子集，Claude Code（zod-to-json-
// schema 輸出）帶的 $schema、additionalProperties 等欄位不被接受。
// bridge/upstream/antigravity/request.rs 的 build_body 現在會用白名單遞迴
// 清洗 input_schema 再組進 functionDeclarations。
//
// 這裡直接打真實端點做對照：同一份「貼近 Claude Code 真實形狀」的巢狀
// schema（帶 $schema、頂層與巢狀 additionalProperties），
//   - 未清洗（模擬修正前）→ 預期 400，把完整錯誤訊息 dump 出來——這是
//     bug 報告裡看不到的部分（原本的 400 訊息在 ai/sse.rs 被截斷在 300
//     字元）。
//   - 清洗後（build_body 現在的行為）→ 預期 200。
// ============================================================================

/// 貼近 Claude Code（zod-to-json-schema 輸出）真實形狀的工具 schema：帶
/// `$schema`、頂層與巢狀 `properties` 都帶 `additionalProperties`，用來
/// 同時驗證清洗有沒有生效、遞迴有沒有真的下探到巢狀層。
fn claude_code_like_tool_def() -> serde_json::Value {
    serde_json::json!({
        "name": "Read",
        "description": "Read a file from the local filesystem",
        "input_schema": {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "The path to read" },
                "limit": { "type": "number", "description": "Line limit" },
                "options": {
                    "type": "object",
                    "properties": { "deep": { "type": "boolean" } },
                    "additionalProperties": false
                }
            },
            "required": ["file_path"],
            "additionalProperties": false
        }
    })
}

#[tokio::test]
#[ignore = "探勘測試：需要真實 Antigravity OAuth 憑證與網路，不進常規 CI"]
async fn probe_antigravity_tool_schema_sanitization() {
    let secrets = SecretStore::new();
    let (access_token, project_id) = get_valid_google_oauth_token(PROVIDER_ID, &secrets)
        .await
        .unwrap_or_else(|e| panic!("BLOCKED: 拿不到 {PROVIDER_ID} 的有效 token/project_id: {e:?}"));
    println!("[probe-schema] got token (len={}), project_id_len={}", access_token.len(), project_id.len());

    let client = AntigravityClient::new(access_token, project_id.clone(), MODEL.into());
    let http = reqwest::Client::new();

    // 走真正的 bridge 路徑（Claude Code → AITerm axum server → build_body）
    // 組出請求，而不是自己手刻信封——這樣測的才是真正會跑的程式碼，不是
    // 「看起來像」的等價物。
    let messages_req: MessagesRequest = serde_json::from_value(serde_json::json!({
        "model": "aiterm:sonnet",
        "messages": [{ "role": "user", "content": "Read the file /tmp/foo.txt" }],
        "tools": [claude_code_like_tool_def()]
    }))
    .expect("MessagesRequest 反序列化失敗");

    // build_body 現在一定會清洗，所以「清洗後」直接是它的正常輸出。
    let sanitized_body = build_body(&messages_req, MODEL, &project_id, &ToolMetaCache::new(512));

    // 「未清洗」版本：拿清洗後的 body 當骨架，把 parameters 換回原始、未清
    // 洗的 input_schema——這樣兩邊除了 parameters 之外完全一致，才是乾淨
    // 的對照組（不會因為其他欄位不同而干擾判讀）。
    let mut unsanitized_body = sanitized_body.clone();
    unsanitized_body["request"]["tools"][0]["functionDeclarations"][0]["parameters"] =
        claude_code_like_tool_def()["input_schema"].clone();
    unsanitized_body["requestId"] = serde_json::json!(format!("agent/probe/{}", uuid_like()));

    write_dump(
        "antigravity_probe_schema_unsanitized_request.json",
        &serde_json::to_string_pretty(&unsanitized_body).unwrap(),
    );
    let resp_unsan = client
        .apply_headers(http.post(client.generate_content_url()))
        .json(&unsanitized_body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("unsanitized request failed to send: {e}"));
    let (status_unsan, body_unsan) =
        dump_response(resp_unsan, "antigravity_probe_schema_unsanitized_raw_response.txt").await;
    println!("[probe-schema] unsanitized status = {status_unsan}");
    println!("[probe-schema] unsanitized FULL error body:\n{body_unsan}");

    write_dump(
        "antigravity_probe_schema_sanitized_request.json",
        &serde_json::to_string_pretty(&sanitized_body).unwrap(),
    );
    let resp_san = client
        .apply_headers(http.post(client.generate_content_url()))
        .json(&sanitized_body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("sanitized request failed to send: {e}"));
    let (status_san, body_san) = dump_response(resp_san, "antigravity_probe_schema_sanitized_raw_response.txt").await;
    println!("[probe-schema] sanitized status = {status_san}");
    if status_san >= 400 {
        println!("[probe-schema] sanitized FULL error body (unexpected — 白名單還有問題):\n{body_san}");
    }

    let summary = format!(
        "未清洗（原封不動 input_schema，含頂層 $schema、頂層與巢狀 additionalProperties）status={status_unsan}\n\
         清洗後（build_body 白名單遞迴清洗）status={status_san}\n\n\
         結論指引：若 unsan=400 且 san=200 → 修正確認有效，bug 重現且白名單可行。\n\
         若 san 也是 400 → 白名單還有問題，見上面「sanitized FULL error body」找出還漏了什麼欄位。\n\
         完整錯誤內容見 antigravity_probe_schema_unsanitized_raw_response.txt / \
         antigravity_probe_schema_sanitized_raw_response.txt。",
    );
    write_dump("antigravity_probe_schema_SUMMARY.txt", &summary);
    println!("[probe-schema] === SUMMARY ===\n{summary}");

    assert_eq!(status_unsan, 400, "預期未清洗 schema 被上游拒絕（400）——若不是，這個 bug 沒有如預期重現，見上面 dump");
    assert_eq!(status_san, 200, "預期清洗後的 schema 通過上游驗證（200）——若不是，白名單還有問題，不要調整斷言，回報完整錯誤");
}

/// 探勘用途的簡易亂數字串（避免每次 requestId 撞號），不追求密碼學品質。
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{nanos:x}")
}

// ============================================================================
// 第二輪探勘：第一輪的 prompt（單一工具呼叫、答案顯而易見）沒有觀察到獨立的
// `thought: true` part——只看到 thoughtSignature 附掛在 functionCall / text
// part 上。比照 M2 Codex 探勘的教訓（太簡單的 prompt 漏掉了 reasoning 事
// 件，補了需要多步驟權衡的 prompt 才觀察到），這裡換一個真的需要權衡的排
// 程題（帶工具，逼模型在呼叫前思考），看有沒有冒出獨立的 thought part。
// ============================================================================
#[tokio::test]
#[ignore = "探勘測試：需要真實 Antigravity OAuth 憑證與網路，不進常規 CI"]
async fn probe_antigravity_thought_part_with_harder_prompt() {
    let secrets = SecretStore::new();
    let (access_token, project_id) = get_valid_google_oauth_token(PROVIDER_ID, &secrets)
        .await
        .unwrap_or_else(|e| panic!("BLOCKED: 拿不到 {PROVIDER_ID} 的有效 token/project_id: {e:?}"));
    println!("[probe-r2] got token (len={}), project_id_len={}", access_token.len(), project_id.len());

    let client = AntigravityClient::new(access_token, project_id.clone(), MODEL.into());
    let http = reqwest::Client::new();

    // 排程最佳化題，且明確要求「呼叫工具前先想清楚」，逼模型在呼叫工具前
    // 有內部推理可摘要——單純問天氣那種一步到位的請求觀察不到這個。
    let hard_prompt = "You are scheduling 5 tasks on 2 workers to minimize the makespan. \
        Task durations in minutes are: A=7, B=3, C=9, D=5, E=6. Each worker processes tasks \
        sequentially, one at a time, with no preemption. Think through the tradeoffs of \
        different assignments step by step. Once you've decided the optimal assignment, call \
        the record_schedule tool with your final answer — do not just answer in text.";

    let tools = serde_json::json!([{
        "functionDeclarations": [{
            "name": "record_schedule",
            "description": "Record the final optimal task-to-worker assignment and the resulting makespan",
            "parameters": {
                "type": "object",
                "properties": {
                    "worker_1_tasks": { "type": "array", "items": { "type": "string" } },
                    "worker_2_tasks": { "type": "array", "items": { "type": "string" } },
                    "makespan_minutes": { "type": "integer" }
                },
                "required": ["worker_1_tasks", "worker_2_tasks", "makespan_minutes"]
            }
        }]
    }]);

    let req = GenerateRequest {
        system_prompt: "You are a careful reasoning assistant with access to tools.".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!(hard_prompt),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: env_ctx(),
        mode: QueryMode::Chat,
        max_tokens: Some(2048),
    };

    let mut body = build_request_body(MODEL, &project_id, &req);
    body["request"]["tools"] = tools;

    write_dump("antigravity_probe_r2_thought_request.json", &serde_json::to_string_pretty(&body).unwrap());

    let resp = client
        .apply_headers(http.post(client.generate_content_url()))
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("r2 thought-probe request failed to send: {e}"));

    let (status, raw) = dump_response(resp, "antigravity_probe_r2_thought_raw_sse.txt").await;
    println!("[probe-r2] status = {status}");
    if status >= 400 {
        println!("[probe-r2] 端點拒絕 (HTTP {status})，探勘到此為止");
        return;
    }

    let events = parse_sse_events(&raw);
    let mut all_parts: Vec<serde_json::Value> = Vec::new();
    for ev in &events {
        let resp_body = unwrap_response(ev);
        if let Some(candidates) = resp_body.get("candidates").and_then(|c| c.as_array()) {
            for cand in candidates {
                if let Some(parts) = cand.get("content").and_then(|c| c.get("parts")).and_then(|p| p.as_array()) {
                    for part in parts {
                        all_parts.push(part.clone());
                    }
                }
            }
        }
    }
    write_dump("antigravity_probe_r2_thought_all_parts.json", &serde_json::to_string_pretty(&all_parts).unwrap());

    println!("[probe-r2] collected {} parts total. key-sets present:", all_parts.len());
    let key_sets: std::collections::BTreeSet<Vec<String>> = all_parts
        .iter()
        .map(|p| {
            let mut ks: Vec<String> = p.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
            ks.sort();
            ks
        })
        .collect();
    for ks in &key_sets {
        println!("  - {ks:?}");
    }

    // 明確找有沒有 `"thought": true` 這個欄位的 part（不管跟什麼欄位共存）。
    let explicit_thought_parts: Vec<&serde_json::Value> =
        all_parts.iter().filter(|p| p.get("thought").and_then(|v| v.as_bool()) == Some(true)).collect();
    println!("[probe-r2] parts with thought:true field: {}", explicit_thought_parts.len());
    for p in &explicit_thought_parts {
        println!("  - {}", serde_json::to_string(p).unwrap());
    }

    let summary = if explicit_thought_parts.is_empty() {
        format!(
            "換了需要多步驟權衡的排程題（帶工具，明確要求先想清楚再呼叫），\
             仍未觀察到任何帶 `\"thought\": true` 欄位的 part。觀察到的 part key-sets: {key_sets:?}\n\
             結論：這次探勘沒有觀察到獨立的 thought part 形式；thoughtSignature（見第一輪探勘）\
             是唯一觀察到的、跟推理相關的欄位，且它是附掛在 functionCall/text part 上的不透明\
             簽章，不是可讀的推理摘要文字。不代表其他 prompt/模型組合下一定不存在，只是「試過\
             更難的 prompt 仍沒觀察到」。"
        )
    } else {
        format!(
            "觀察到 {} 個帶 thought:true 的 part，完整內容見 antigravity_probe_r2_thought_all_parts.json",
            explicit_thought_parts.len()
        )
    };
    write_dump("antigravity_probe_r2_thought_SUMMARY.txt", &summary);
    println!("[probe-r2] === SUMMARY ===\n{summary}");
}

// ============================================================================
// 第二輪探勘（延續）：M2 在 Codex 上補第二輪才發現 reasoning 事件；這裡的
// 風險對稱點是「並行工具呼叫」——Claude Code 是重度並行工具呼叫的客戶端，
// 第一輪只測了單一 functionCall，這裡專門測：
//
//   問題 A：模型會不會在同一輪回多個 functionCall part？在同一個
//           candidates[0].content.parts[] 裡還是分散在不同 SSE chunk？
//           id 與 thoughtSignature 是否一對一？args 會不會跨 chunk 分片？
//   問題 B（最重要）：並行時 functionResponse 要不要帶 id 才能正確對應？
//           對照「都不帶 id」vs「都帶 id」，並檢查模型有沒有張冠李戴。
//   問題 C：thoughtSignature 的回送粒度——並行時是否每個 functionCall part
//           都要帶自己的 thoughtSignature？只帶其中一個會怎樣？
//
// 為了让模型有機會出現「同一函式被呼叫兩次、參數不同」這種必須靠 id 才能
// 消歧義的情境，工具刻意設計成同一個 get_weather 對兩個不同城市，外加一個
// 完全不同的 list_files 工具，逼模型至少要並行呼叫兩個不同工具。
// ============================================================================

/// 送一次 generateContent 請求，回傳 (status, raw_body, all_parts,
/// function_call_parts)。集中處理共用的送出/dump/parse 邏輯，避免三個對照
/// 組各自重複一份。
async fn send_and_collect(
    http: &reqwest::Client,
    client: &AntigravityClient,
    body: &serde_json::Value,
    dump_prefix: &str,
) -> (u16, String, Vec<serde_json::Value>, Vec<serde_json::Value>) {
    write_dump(&format!("{dump_prefix}_request.json"), &serde_json::to_string_pretty(body).unwrap());
    let resp = client
        .apply_headers(http.post(client.generate_content_url()))
        .json(body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{dump_prefix} request failed to send: {e}"));
    let (status, raw) = dump_response(resp, &format!("{dump_prefix}_raw_sse.txt")).await;
    println!("[probe-r2-parallel] {dump_prefix} status = {status}");

    if status >= 400 {
        return (status, raw, Vec::new(), Vec::new());
    }

    let events = parse_sse_events(&raw);
    let mut all_parts: Vec<serde_json::Value> = Vec::new();
    for ev in &events {
        let resp_body = unwrap_response(ev);
        if let Some(candidates) = resp_body.get("candidates").and_then(|c| c.as_array()) {
            for cand in candidates {
                if let Some(parts) = cand.get("content").and_then(|c| c.get("parts")).and_then(|p| p.as_array()) {
                    for part in parts {
                        all_parts.push(part.clone());
                    }
                }
            }
        }
    }
    write_dump(&format!("{dump_prefix}_all_parts.json"), &serde_json::to_string_pretty(&all_parts).unwrap());

    let function_call_parts: Vec<serde_json::Value> =
        all_parts.iter().filter(|p| p.get("functionCall").is_some()).cloned().collect();
    write_dump(
        &format!("{dump_prefix}_function_call_parts.json"),
        &serde_json::to_string_pretty(&function_call_parts).unwrap(),
    );

    (status, raw, all_parts, function_call_parts)
}

/// 把 all_parts 裡所有 text part 串接起來，方便肉眼檢查模型最終回答有沒有
/// 張冠李戴（例如把台北的天氣講成東京的、把檔名對錯目錄）。
fn concat_text_parts(all_parts: &[serde_json::Value]) -> String {
    all_parts.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join("")
}

#[tokio::test]
#[ignore = "探勘測試：需要真實 Antigravity OAuth 憑證與網路，不進常規 CI"]
async fn probe_antigravity_parallel_tool_calls() {
    let secrets = SecretStore::new();
    let (access_token, project_id) = get_valid_google_oauth_token(PROVIDER_ID, &secrets)
        .await
        .unwrap_or_else(|e| panic!("BLOCKED: 拿不到 {PROVIDER_ID} 的有效 token/project_id: {e:?}"));
    println!("[probe-r2-parallel] got token (len={}), project_id_len={}", access_token.len(), project_id.len());

    let client = AntigravityClient::new(access_token, project_id.clone(), MODEL.into());
    let http = reqwest::Client::new();

    // 兩個不同工具：get_weather + list_files。刻意在 prompt 裡明講「兩個城
    // 市的天氣都要查、還要列目錄」，逼模型有機會並行呼叫 3 次（含同一工具
    // 呼叫兩次、參數不同——這是最需要 id 消歧義的情境）。
    let tools = serde_json::json!([{
        "functionDeclarations": [
            {
                "name": "get_weather",
                "description": "Get the current weather for a given city",
                "parameters": {
                    "type": "object",
                    "properties": { "city": { "type": "string", "description": "City name" } },
                    "required": ["city"]
                }
            },
            {
                "name": "list_files",
                "description": "List files in a given directory path",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "Directory path" } },
                    "required": ["path"]
                }
            }
        ]
    }]);

    // 幾種不同措辭的 prompt 依序試，只要有一次拿到 >=2 個 functionCall part
    // 就停止——「試過幾種都誘不出並行」本身也是有用的結論，屆時全部 dump。
    let prompts = [
        "Call get_weather for Taipei AND get_weather for Tokyo AND list_files for /private/tmp \
         — all three calls right now, in this same turn, in parallel. Do not answer in text, \
         do not call them one at a time waiting for results — issue all three tool calls together.",
        "I need three things simultaneously: the weather in Taipei, the weather in Tokyo, and a \
         directory listing of /private/tmp. Use the available tools to fetch all three at once \
         in a single turn rather than sequentially.",
        "Please invoke get_weather(city=\"Taipei\"), get_weather(city=\"Tokyo\"), and \
         list_files(path=\"/private/tmp\") together in one response, in parallel.",
    ];

    let mut chosen: Option<(serde_json::Value, Vec<serde_json::Value>, Vec<serde_json::Value>)> = None;
    for (i, prompt) in prompts.iter().enumerate() {
        let req = GenerateRequest {
            system_prompt: "You are a coding CLI assistant with access to tools. When multiple \
                independent pieces of information are requested, call all the relevant tools in \
                the same turn instead of one at a time."
                .into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: serde_json::json!(*prompt),
                tool_call_id: None,
                tool_calls: None,
            }],
            context: env_ctx(),
            mode: QueryMode::Chat,
            max_tokens: Some(1024),
        };
        let mut body = build_request_body(MODEL, &project_id, &req);
        body["request"]["tools"] = tools.clone();

        let (status, _raw, all_parts, fc_parts) =
            send_and_collect(&http, &client, &body, &format!("antigravity_probe_r2_parallel_attempt{i}")).await;

        println!("[probe-r2-parallel] attempt{i}: status={status} functionCall parts={}", fc_parts.len());
        if status < 400 && fc_parts.len() >= 2 {
            chosen = Some((body, all_parts, fc_parts));
            println!("[probe-r2-parallel] attempt{i} produced >=2 functionCall parts, stopping prompt search");
            break;
        }
    }

    let Some((round1_body, all_parts, fc_parts)) = chosen else {
        write_dump(
            "antigravity_probe_r2_parallel_SUMMARY.txt",
            "試過 3 種不同措辭的 prompt（明講同時查兩個城市天氣 + 列目錄、要求平行），\
             沒有任何一次拿到 >=2 個 functionCall part。可能是這個模型/端點組合在這個情境下\
             傾向序列呼叫，或每次只挑一個工具呼叫。完整內容見各 attemptN_* dump 檔。\
             結論：本輪探勘沒有觀察到並行 functionCall——不代表『不存在』，只是『試過這些\
             prompt 誘不出來』。問題 B/C（functionResponse id、thoughtSignature 粒度）因此\
             無法在『真正並行』的情境下驗證，只能等以後找到更可靠的誘發方式再補。",
        );
        println!("[probe-r2-parallel] === 無法誘發並行呼叫，探勘到此為止（本身是有效結論）===");
        return;
    };

    // ---- 問題 A：分析拿到的 functionCall part 們 ----
    println!("[probe-r2-parallel] === 問題 A：並行 functionCall 的形狀 ===");
    println!("[probe-r2-parallel] 總計 {} 個 functionCall part（在 all_parts 裡的位置關係見 dump）", fc_parts.len());

    let mut per_call_info = Vec::new();
    for (idx, fc_part) in fc_parts.iter().enumerate() {
        let fc = fc_part.get("functionCall").cloned().unwrap_or(serde_json::Value::Null);
        let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("<none>").to_string();
        let id = fc.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let has_sig = fc_part.get("thoughtSignature").is_some();
        let sig_len = fc_part.get("thoughtSignature").and_then(|v| v.as_str()).map(|s| s.len());
        let args = fc.get("args").cloned().unwrap_or(serde_json::Value::Null);
        println!(
            "  [{idx}] name={name} id={id:?} has_thoughtSignature={has_sig} sig_len={sig_len:?} args={args}"
        );
        per_call_info.push(serde_json::json!({
            "index": idx, "name": name, "id": id, "has_thoughtSignature": has_sig,
            "thoughtSignature_len": sig_len, "args": args,
        }));
    }
    write_dump(
        "antigravity_probe_r2_parallel_per_call_analysis.json",
        &serde_json::to_string_pretty(&per_call_info).unwrap(),
    );

    // id 是否每個都有、是否互不相同（1:1 而非共用）。
    let ids: Vec<Option<String>> =
        fc_parts.iter().map(|p| p.get("functionCall").and_then(|fc| fc.get("id")).and_then(|v| v.as_str()).map(String::from)).collect();
    let all_have_id = ids.iter().all(|id| id.is_some());
    let distinct_ids: std::collections::HashSet<&Option<String>> = ids.iter().collect();
    let ids_all_distinct = distinct_ids.len() == ids.len();

    // thoughtSignature 是否每個都有、是否互不相同。
    let sigs: Vec<Option<String>> =
        fc_parts.iter().map(|p| p.get("thoughtSignature").and_then(|v| v.as_str()).map(String::from)).collect();
    let all_have_sig = sigs.iter().all(|s| s.is_some());
    let distinct_sigs: std::collections::HashSet<&Option<String>> = sigs.iter().collect();
    let sigs_all_distinct = distinct_sigs.len() == sigs.len();

    // 是否所有 functionCall part 都落在同一個 SSE event（同一個 candidates[0]
    // .content.parts[] 陣列）裡，還是分散在多個 chunk。用「這個 part 在
    // all_parts 裡是否緊鄰其他 functionCall part」粗略判斷；精確答案要看
    // per-event dump（antigravity_probe_r2_parallel_attemptN_raw_sse.txt）。
    let fc_indices_in_all_parts: Vec<usize> = all_parts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.get("functionCall").is_some())
        .map(|(i, _)| i)
        .collect();

    let a_summary = format!(
        "拿到 {} 個 functionCall part。\n\
         - 每個都有 id: {all_have_id}；id 互不相同（1:1 非共用）: {ids_all_distinct}；原始 ids={ids:?}\n\
         - 每個都有 thoughtSignature: {all_have_sig}；簽章互不相同: {sigs_all_distinct}\n\
         - functionCall part 在 all_parts 陣列中的位置索引: {fc_indices_in_all_parts:?}\n\
         - 精確的『同一 event 還是跨 chunk』要對照 antigravity_probe_r2_parallel_attemptN_raw_sse.txt \
           裡每個 `data:` 行各自帶了幾個 part 來判斷。\n\
         - 詳細每筆 id/thoughtSignature/args 見 antigravity_probe_r2_parallel_per_call_analysis.json",
        fc_parts.len()
    );
    println!("[probe-r2-parallel] {a_summary}");

    // ---- 問題 B + C：多輪回送對照實驗 ----
    // base_contents 是「round1 request 的 contents」；round2 的 body 都從
    // round1_body clone 出來改 contents/requestId，這樣 systemInstruction /
    // generationConfig / userAgent / requestType 等既有欄位會原樣沿用，不
    // 用手刻一份可能漏欄位的 skeleton。
    let base_contents: Vec<serde_json::Value> =
        round1_body["request"]["contents"].as_array().cloned().unwrap_or_default();

    // 準備刻意可分辨、彼此不該混淆的假回應內容，方便事後檢查模型有沒有張冠
    // 李戴：兩個 get_weather（不同城市）內容差異很大，list_files 回傳一組
    // 一望即知不是天氣的檔名。
    fn fake_response_for(name: &str, args: &serde_json::Value) -> serde_json::Value {
        match name {
            "get_weather" => {
                let city = args.get("city").and_then(|v| v.as_str()).unwrap_or("");
                if city.eq_ignore_ascii_case("taipei") {
                    serde_json::json!({ "temperature_c": 31, "condition": "monsoon rain" })
                } else {
                    serde_json::json!({ "temperature_c": -4, "condition": "heavy snow" })
                }
            }
            "list_files" => serde_json::json!({ "files": ["zephyr.cfg", "glimmer.dat", "catnip.log"] }),
            _ => serde_json::json!({ "result": "ok" }),
        }
    }

    let model_turn_full = serde_json::json!({ "role": "model", "parts": all_parts.clone() });

    // 變體 1：functionResponse 都不帶 id（只有 name + response）。
    let fr_parts_no_id: Vec<serde_json::Value> = fc_parts
        .iter()
        .map(|p| {
            let fc = p.get("functionCall").cloned().unwrap_or_default();
            let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
            serde_json::json!({ "functionResponse": { "name": name, "response": fake_response_for(&name, &args) } })
        })
        .collect();
    let mut contents_no_id = base_contents.clone();
    contents_no_id.push(model_turn_full.clone());
    contents_no_id.push(serde_json::json!({ "role": "user", "parts": fr_parts_no_id }));
    let mut body_no_id = round1_body.clone();
    body_no_id["request"]["contents"] = serde_json::Value::Array(contents_no_id);
    body_no_id["request"]["tools"] = tools.clone();
    body_no_id["requestId"] = serde_json::json!(format!("agent/probe/{}", uuid_like()));

    let (status_no_id, _raw_no_id, all_parts_no_id, _fc_no_id) =
        send_and_collect(&http, &client, &body_no_id, "antigravity_probe_r2_parallel_B_no_id").await;
    let text_no_id = concat_text_parts(&all_parts_no_id);
    println!("[probe-r2-parallel] B(no id) status={status_no_id} final_text={text_no_id:?}");

    // 變體 2：functionResponse 都帶 id（對應各自 functionCall 的 id，若有）。
    let fr_parts_with_id: Vec<serde_json::Value> = fc_parts
        .iter()
        .map(|p| {
            let fc = p.get("functionCall").cloned().unwrap_or_default();
            let name = fc.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
            let id = fc.get("id").cloned();
            let mut obj = serde_json::json!({ "name": name, "response": fake_response_for(&name, &args) });
            if let Some(id) = id {
                obj["id"] = id;
            }
            serde_json::json!({ "functionResponse": obj })
        })
        .collect();
    let mut contents_with_id = base_contents.clone();
    contents_with_id.push(model_turn_full.clone());
    contents_with_id.push(serde_json::json!({ "role": "user", "parts": fr_parts_with_id }));
    let mut body_with_id = round1_body.clone();
    body_with_id["request"]["contents"] = serde_json::Value::Array(contents_with_id);
    body_with_id["request"]["tools"] = tools.clone();
    body_with_id["requestId"] = serde_json::json!(format!("agent/probe/{}", uuid_like()));

    let (status_with_id, _raw_with_id, all_parts_with_id, _fc_with_id) =
        send_and_collect(&http, &client, &body_with_id, "antigravity_probe_r2_parallel_B_with_id").await;
    let text_with_id = concat_text_parts(&all_parts_with_id);
    println!("[probe-r2-parallel] B(with id) status={status_with_id} final_text={text_with_id:?}");

    let b_summary = format!(
        "變體 1（不帶 id）status={status_no_id}\n最終文字回答：{text_no_id}\n\n\
         變體 2（帶 id）status={status_with_id}\n最終文字回答：{text_with_id}\n\n\
         人工檢查重點：兩段最終文字是否都正確把台北天氣（31°C/monsoon rain）、東京天氣\
         （-4°C/heavy snow，若第二個 get_weather 真的存在）、目錄檔名（zephyr.cfg/\
         glimmer.dat/catnip.log）各自對應正確，沒有互相調換。",
    );
    write_dump("antigravity_probe_r2_parallel_B_SUMMARY.txt", &b_summary);
    println!("[probe-r2-parallel] === 問題 B SUMMARY ===\n{b_summary}");

    // ---- 問題 C：thoughtSignature 回送粒度 ----
    // 實測發現（問題 A 已確認）：並行呼叫時「不是每個 functionCall part 都
    // 有自己的 thoughtSignature」——3 個 functionCall part 裡只有 1 個帶
    // signature（兩次重跑都是同一個位置：第 1 個 get_weather）。所以「每個
    // part 各自帶一個簽章」這個假設本身在這次觀察中不成立，C1/C2「拿掉某個
    // 位置的簽章」這組對照失去意義（因為多數位置本來就沒有簽章）。
    //
    // 改測真正對得上實測情境的問題：伺服器實際附掛的那個（些）簽章，回送時
    // 拿掉會不會 400？這是問題 A 的觀察結果直接導出的、真正決定 adapter 快
    // 取粒度的問題——若答案是「會 400」，代表 adapter 只需要在有簽章的那個
    // part 上快取/回送，其餘沒簽章的 part 原樣（不帶 thoughtSignature 欄位）
    // 回送即可，不需要幫每個 functionCall 都合成一個。
    let sig_bearing_indices: Vec<usize> =
        fc_parts.iter().enumerate().filter(|(_, p)| p.get("thoughtSignature").is_some()).map(|(i, _)| i).collect();

    if sig_bearing_indices.is_empty() {
        write_dump(
            "antigravity_probe_r2_parallel_C_SUMMARY.txt",
            "跳過問題 C 的對照實驗：這次並行呼叫拿到的 functionCall part 裡，沒有任何一個帶 \
             thoughtSignature（不同於單一呼叫時第一輪的觀察）。無簽章可拿掉來做對照。",
        );
    } else {
        // 把「有帶 thoughtSignature 的那幾個 functionCall part」的簽章全部
        // 拿掉，其餘 part（包含本來就沒簽章的）維持原樣，其他都跟基準（問題
        // B 變體 2，已知 200）完全一樣。
        let mut parts_sig_stripped = all_parts.clone();
        let mut fc_seen = 0usize;
        for p in parts_sig_stripped.iter_mut() {
            if p.get("functionCall").is_some() {
                if sig_bearing_indices.contains(&fc_seen) {
                    if let Some(obj) = p.as_object_mut() {
                        obj.remove("thoughtSignature");
                    }
                }
                fc_seen += 1;
            }
        }
        let model_turn_sig_stripped = serde_json::json!({ "role": "model", "parts": parts_sig_stripped });
        let mut contents_c = base_contents.clone();
        contents_c.push(model_turn_sig_stripped);
        contents_c.push(serde_json::json!({ "role": "user", "parts": fr_parts_with_id.clone() }));
        let mut body_c = round1_body.clone();
        body_c["request"]["contents"] = serde_json::Value::Array(contents_c);
        body_c["request"]["tools"] = tools.clone();
        body_c["requestId"] = serde_json::json!(format!("agent/probe/{}", uuid_like()));
        let (status_c, raw_c, _, _) =
            send_and_collect(&http, &client, &body_c, "antigravity_probe_r2_parallel_C_strip_present_sigs").await;
        println!(
            "[probe-r2-parallel] C(拿掉實際存在的 {} 個 thoughtSignature，其餘 part 不變) status={status_c}",
            sig_bearing_indices.len()
        );

        let c_summary = format!(
            "問題 A 已確認：3 個 functionCall part 中只有 index {sig_bearing_indices:?} 帶 \
             thoughtSignature，其餘完全沒有這個欄位（不是『每個都有、只是不同簽章』）。\n\n\
             基準（原樣回送，含那 {} 個 part 各自的簽章狀態，即問題 B 變體 2）status={status_with_id}\n\
             把『實際存在的』{} 個 thoughtSignature 全部拿掉（其餘本來就沒有的 part 不受影響）：\
             status={status_c}\n\n\
             結論指引：若基準 200、拿掉後 400 → 證實 M1/round1 的『不透明簽章必須原樣回送』\
             規則在並行情境下依然成立，但粒度是『伺服器給了簽章的那個 part 才需要回送』，不是\
             『每個 functionCall 都要有一個』。adapter 只需在有 thoughtSignature 的 part 上快取\
             (id → signature)，沒有的 part 直接不帶這個欄位即可。\n\
             若拿掉後仍是 200 → 這次觀察到的唯一簽章其實不是必須的，需要更多樣本才能下定論。\n\
             400 情境完整錯誤訊息見 antigravity_probe_r2_parallel_C_strip_present_sigs_raw_sse.txt。",
            sig_bearing_indices.len(),
            sig_bearing_indices.len(),
        );
        write_dump("antigravity_probe_r2_parallel_C_SUMMARY.txt", &c_summary);
        println!("[probe-r2-parallel] === 問題 C SUMMARY ===\n{c_summary}");
        if status_c >= 400 {
            println!("[probe-r2-parallel] C strip-present-sigs raw body: {raw_c}");
        }
    }

    // ---- 總結 dump（含問題 A 的結論文字）----
    write_dump("antigravity_probe_r2_parallel_A_SUMMARY.txt", &a_summary);
}
