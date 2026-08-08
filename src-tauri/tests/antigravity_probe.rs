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
