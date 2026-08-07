//! M2 探勘測試（非常規測試，不驗證任何行為）——直接用真實 Codex OAuth
//! 憑證打私有端點 `chatgpt.com/backend-api/codex/responses`，記錄：
//!
//! 1. 該端點接不接受扁平格式的 `tools` 欄位。
//! 2. 若接受，工具呼叫相關的 SSE 事件實際長什麼樣（型別名稱、欄位）。
//! 3. 多輪對話裡，工具結果要怎麼回送才有效（`function_call_output` 慣例
//!    是否成立、有沒有必須原樣回送的不透明欄位）。
//!
//! 端點是無文件的逆向端點，所以本檔案只負責「打真實請求、把原始 SSE
//! dump 到檔案」，不對回應形狀做任何假設性斷言 —— 那是 M2 adapter 實作
//! 階段的事，不是這裡的事。
//!
//! 需要真實憑證（provider id `GPT5.6`）與網路，因此整支標 `#[ignore]`，
//! 不會進常規 `cargo test` 流程。執行方式：
//!
//!   cargo test --test codex_probe -- --ignored --nocapture
//!
//! Access token 存活僅 300 秒且 refresh token 會輪替，一律透過
//! `aiterm_lib::ai::router::get_valid_codex_oauth_token` 取得/刷新，
//! 不自己重寫刷新邏輯。

use aiterm_lib::ai::codex::{build_request_body, CodexClient};
use aiterm_lib::ai::router::get_valid_codex_oauth_token;
use aiterm_lib::ai::{ChatMessage, EnvSnapshot, GenerateRequest, QueryMode};
use aiterm_lib::secret::SecretStore;
use std::path::PathBuf;

const PROVIDER_ID: &str = "GPT5.6";
// 讀自使用者的 config.toml（[[providers]] id = "GPT5.6"）。
const MODEL: &str = "gpt-5.6-luna";

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
/// （如 `[DONE]`）直接跳過，不視為錯誤 —— 這是探勘，不是嚴格 parser。
fn parse_sse_events(raw: &str) -> Vec<serde_json::Value> {
    raw.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "[DONE]")
        .filter_map(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .collect()
}

fn event_type(ev: &serde_json::Value) -> &str {
    ev.get("type").and_then(|v| v.as_str()).unwrap_or("<no type>")
}

/// 遞迴掃描整段事件流，找出任何帶 `call_id` 欄位的 JSON 物件（不管巢狀
/// 在哪一層）——這是探勘用的「不管欄位名稱猜對沒，先把所有像 tool call
/// 的東西都撈出來看」策略。
fn find_call_id_objects(events: &[serde_json::Value]) -> Vec<serde_json::Value> {
    fn walk(v: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
        match v {
            serde_json::Value::Object(map) => {
                if map.contains_key("call_id") {
                    out.push(v.clone());
                }
                for val in map.values() {
                    walk(val, out);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    walk(item, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for ev in events {
        walk(ev, &mut out);
    }
    out
}

fn env_ctx() -> EnvSnapshot {
    EnvSnapshot { os: "macos".into(), shell: "zsh".into(), cwd: PathBuf::from("/"), ..Default::default() }
}

#[tokio::test]
#[ignore = "探勘測試：需要真實 Codex OAuth 憑證與網路，不進常規 CI"]
async fn probe_codex_tools_support_and_multiturn() {
    // ---- 0. 拿有效 access token（會自動刷新）----
    let secrets = SecretStore::new();
    let (access_token, account_id) = get_valid_codex_oauth_token(PROVIDER_ID, &secrets)
        .await
        .unwrap_or_else(|e| panic!("BLOCKED: 拿不到 {PROVIDER_ID} 的有效 token: {e:?}"));
    println!("[probe] got token (len={}), account_id={:?}", access_token.len(), account_id);

    let client =
        CodexClient::with_base_url(access_token, MODEL.into(), account_id, "https://chatgpt.com".into());
    // CodexClient 內部的 reqwest::Client 欄位是 private（沒理由公開），探勘
    // 測試自己開一個就好 —— responses_url()/apply_headers() 才是需要照抄
    // 的既有行為，HTTP client 本身沒有特殊狀態。
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

    // Responses API 的 tool 格式是扁平的：{"type":"function","name":...,
    // "description":...,"parameters":{...}}，跟 chat.completions 巢狀的
    // {"type":"function","function":{...}} 不同。
    let tools = serde_json::json!([{
        "type": "function",
        "name": "get_weather",
        "description": "Get the current weather for a given city",
        "parameters": {
            "type": "object",
            "properties": {
                "city": { "type": "string", "description": "City name" }
            },
            "required": ["city"]
        }
    }]);

    let mut round1_body = build_request_body(MODEL, &round1_req);
    round1_body["tools"] = tools.clone();

    write_dump("codex_probe_round1_request.json", &serde_json::to_string_pretty(&round1_body).unwrap());

    let resp = client
        .apply_headers(http.post(client.responses_url()))
        .json(&round1_body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round1 request failed to send: {e}"));

    let (status, body) = dump_response(resp, "codex_probe_round1_raw_sse.txt").await;
    println!("[probe] round1 status = {status}");

    if status >= 400 {
        println!(
            "[probe] === 端點拒絕帶 tools 的請求 (HTTP {status}) === 完整內容見 codex_probe_round1_raw_sse.txt"
        );
        // 端點回 4xx 本身就是這題的答案，探勘到此為止 —— 不繼續測多輪。
        return;
    }

    let events = parse_sse_events(&body);
    let types: std::collections::BTreeSet<&str> = events.iter().map(event_type).collect();
    println!("[probe] round1 distinct event types: {types:?}");

    let call_id_objs = find_call_id_objects(&events);
    write_dump(
        "codex_probe_round1_call_id_objects.json",
        &serde_json::to_string_pretty(&call_id_objs).unwrap(),
    );
    println!("[probe] round1 objects containing call_id: {}", call_id_objs.len());
    for obj in &call_id_objs {
        println!("  - {}", serde_json::to_string(obj).unwrap());
    }

    // 第一次探勘跑出來的實測結果：response.completed.response.output
    // 是空陣列 `[]`，即使這輪確實呼叫了工具 —— 跟 OpenAI 官方文件對
    // Responses API 的一般描述不符（文件說 completed 帶完整 output）。
    // 每個輸出 item 的權威內容實際落在各自的 response.output_item.done
    // 事件的 `item` 欄位裡，用這個來源重建多輪回送要用的 item 列表。
    let output_items: Vec<serde_json::Value> = events
        .iter()
        .filter(|ev| event_type(ev) == "response.output_item.done")
        .filter_map(|ev| ev.get("item").cloned())
        .collect();

    write_dump(
        "codex_probe_round1_output_items_from_stream.json",
        &serde_json::to_string_pretty(&output_items).unwrap(),
    );

    let function_call_item = output_items.iter().find(|item| item.get("type").and_then(|t| t.as_str()) == Some("function_call"));

    let Some(call_item) = function_call_item else {
        println!("[probe] 模型這輪沒有呼叫任何 tool（可能忽略了工具或直接用文字回答），output items: {output_items:?}");
        return;
    };

    let call_id = call_item.get("call_id").and_then(|v| v.as_str()).unwrap_or("MISSING_CALL_ID").to_string();
    println!("[probe] found function_call: call_id={call_id}, item={}", serde_json::to_string(call_item).unwrap());

    // ==================== Q3: 多輪回送，帶 vs 不帶原始 output items 對照 ====================
    let base_input = round1_body["input"].as_array().cloned().unwrap_or_default();
    let tool_output = serde_json::json!({
        "type": "function_call_output",
        "call_id": call_id,
        "output": "{\"temperature_c\": 29, \"condition\": \"partly cloudy\"}"
    });

    // 對照組 A：原樣把 round1 的所有 output items（含任何不透明欄位）塞回去，
    // 再接 function_call_output。
    let mut input_a = base_input.clone();
    input_a.extend(output_items.iter().cloned());
    input_a.push(tool_output.clone());
    let mut body_a = round1_body.clone();
    body_a["input"] = serde_json::Value::Array(input_a);
    body_a["tools"] = tools.clone();

    write_dump("codex_probe_round2a_request.json", &serde_json::to_string_pretty(&body_a).unwrap());
    let resp_a = client
        .apply_headers(http.post(client.responses_url()))
        .json(&body_a)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round2a request failed to send: {e}"));
    let (status_a, body_raw_a) = dump_response(resp_a, "codex_probe_round2a_raw_sse.txt").await;
    println!("[probe] round2a (WITH original output items replayed) status = {status_a}");

    // 對照組 B：不把 round1 的 output items 塞回去，只接 function_call_output。
    let mut input_b = base_input.clone();
    input_b.push(tool_output.clone());
    let mut body_b = round1_body.clone();
    body_b["input"] = serde_json::Value::Array(input_b);
    body_b["tools"] = tools.clone();

    write_dump("codex_probe_round2b_request.json", &serde_json::to_string_pretty(&body_b).unwrap());
    let resp_b = client
        .apply_headers(http.post(client.responses_url()))
        .json(&body_b)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round2b request failed to send: {e}"));
    let (status_b, body_raw_b) = dump_response(resp_b, "codex_probe_round2b_raw_sse.txt").await;
    println!("[probe] round2b (WITHOUT original output items replayed) status = {status_b}");

    let events_a = parse_sse_events(&body_raw_a);
    let events_b = parse_sse_events(&body_raw_b);
    println!(
        "[probe] round2a distinct event types: {:?}",
        events_a.iter().map(event_type).collect::<std::collections::BTreeSet<_>>()
    );
    println!(
        "[probe] round2b distinct event types: {:?}",
        events_b.iter().map(event_type).collect::<std::collections::BTreeSet<_>>()
    );

    // 對照組 C：round2a 證明「原樣回送 function_call item」有效、round2b
    // 證明「完全不回送」無效——但還不知道是不是每個欄位都要原樣保留，還是
    // 只要形狀對（type/call_id/name/arguments）就好。這組故意去掉 round1
    // 產生的不透明 `id` 欄位（fc_...），只保留看起來語意必要的欄位，測試
    // `id` 是不是像 Gemini `thought_signature` 那種「必須原樣回送」的欄位。
    let minimal_call_item = serde_json::json!({
        "type": "function_call",
        "call_id": call_id,
        "name": call_item.get("name").cloned().unwrap_or(serde_json::Value::Null),
        "arguments": call_item.get("arguments").cloned().unwrap_or(serde_json::Value::Null)
    });
    let mut input_c = base_input.clone();
    input_c.push(minimal_call_item);
    input_c.push(tool_output.clone());
    let mut body_c = round1_body.clone();
    body_c["input"] = serde_json::Value::Array(input_c);
    body_c["tools"] = tools.clone();

    write_dump("codex_probe_round2c_request.json", &serde_json::to_string_pretty(&body_c).unwrap());
    let resp_c = client
        .apply_headers(http.post(client.responses_url()))
        .json(&body_c)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round2c request failed to send: {e}"));
    let (status_c, _body_raw_c) = dump_response(resp_c, "codex_probe_round2c_raw_sse.txt").await;
    println!("[probe] round2c (minimal reconstructed function_call item, no opaque `id`) status = {status_c}");

    let summary = format!(
        "round1 status={status}\nround2a (with replayed output items, full item incl. opaque `id`) status={status_a}\n\
         round2b (without replaying the function_call item at all) status={status_b}\n\
         round2c (minimal reconstructed item: type/call_id/name/arguments, no `id`) status={status_c}\n\
         見同目錄下 codex_probe_round1_raw_sse.txt / codex_probe_round2a_raw_sse.txt / codex_probe_round2b_raw_sse.txt / codex_probe_round2c_raw_sse.txt 取得完整原始內容。"
    );
    write_dump("codex_probe_SUMMARY.txt", &summary);
    println!("[probe] === SUMMARY ===\n{summary}");
}

// ============================================================================
// 第二輪探勘：第一輪留下的兩個未觀察區域 —— reasoning/thinking 事件、
// 並行工具呼叫。沿用上面同一批 helper（dump_path / write_dump /
// dump_response / parse_sse_events / event_type / env_ctx），不重寫。
// ============================================================================

/// 問題 A：reasoning / thinking 事件。
///
/// 第一輪送出的請求沒有明確帶 `reasoning` 欄位，但後端在 response 物件裡
/// 回填了 `"reasoning":{"context":"all_turns","effort":"medium","mode":
/// "standard","summary":null}` 這個預設值，且 usage 顯示
/// `reasoning_tokens:0`——那次的 prompt（單一工具呼叫）太簡單，模型可能
/// 根本沒有內部推理可摘要。這一輪明確把 `summary` 設成
/// "auto"/"detailed"/"concise" 依序嘗試，並換一個真的需要多步驟權衡的
/// prompt（2 個 worker、5 個任務的排程最佳化），看會不會冒出 reasoning
/// 相關的 SSE 事件。
#[tokio::test]
#[ignore = "探勘測試：需要真實 Codex OAuth 憑證與網路，不進常規 CI"]
async fn probe_codex_reasoning_summary_events() {
    let secrets = SecretStore::new();
    let (access_token, account_id) = get_valid_codex_oauth_token(PROVIDER_ID, &secrets)
        .await
        .unwrap_or_else(|e| panic!("BLOCKED: 拿不到 {PROVIDER_ID} 的有效 token: {e:?}"));
    println!("[probe-r2a] got token (len={}), account_id={:?}", access_token.len(), account_id);

    let client =
        CodexClient::with_base_url(access_token, MODEL.into(), account_id, "https://chatgpt.com".into());
    let http = reqwest::Client::new();

    // 真的需要權衡取捨的排程題，不是問天氣那種一步到位的問題。
    let hard_prompt = "You are scheduling 5 tasks on 2 workers to minimize the makespan. \
        Task durations in minutes are: A=7, B=3, C=9, D=5, E=6. Each worker processes tasks \
        sequentially, one at a time, with no preemption. Find an assignment of tasks to the \
        two workers that minimizes the time at which the last task finishes. Think through \
        the tradeoffs of different assignments step by step before giving your final answer, \
        then state the final assignment and the resulting makespan.";

    let req = GenerateRequest {
        system_prompt: "You are a careful reasoning assistant. Think step by step.".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!(hard_prompt),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: env_ctx(),
        mode: QueryMode::Chat,
        max_tokens: Some(1024),
    };

    // 依序試 summary 的幾個值；第一個真的冒出 reasoning 相關內容的就停下來，
    // 拿它去做「要不要回送」的多輪對照。
    let summary_variants = ["auto", "detailed", "concise"];
    let mut chosen: Option<(&str, serde_json::Value, Vec<serde_json::Value>)> = None;

    for summary in summary_variants {
        let mut body = build_request_body(MODEL, &req);
        body["reasoning"] = serde_json::json!({ "effort": "medium", "summary": summary });

        write_dump(
            &format!("codex_probe_r2_reasoning_{summary}_request.json"),
            &serde_json::to_string_pretty(&body).unwrap(),
        );

        let resp = client
            .apply_headers(http.post(client.responses_url()))
            .json(&body)
            .send()
            .await
            .unwrap_or_else(|e| panic!("reasoning(summary={summary}) request failed to send: {e}"));

        let (status, raw) =
            dump_response(resp, &format!("codex_probe_r2_reasoning_{summary}_raw_sse.txt")).await;
        println!("[probe-r2a] summary={summary} status={status}");

        if status >= 400 {
            println!("[probe-r2a] summary={summary} 被拒絕 (HTTP {status})，換下一個值");
            continue;
        }

        let events = parse_sse_events(&raw);
        let types: std::collections::BTreeSet<&str> = events.iter().map(event_type).collect();
        println!("[probe-r2a] summary={summary} distinct event types: {types:?}");

        // 掃描所有 type 含 "reasoning" 字樣的事件（不預設實際名稱）。
        let reasoning_events: Vec<serde_json::Value> =
            events.iter().filter(|ev| event_type(ev).contains("reasoning")).cloned().collect();
        write_dump(
            &format!("codex_probe_r2_reasoning_{summary}_reasoning_events.json"),
            &serde_json::to_string_pretty(&reasoning_events).unwrap(),
        );
        println!("[probe-r2a] summary={summary} reasoning-typed SSE events: {}", reasoning_events.len());
        for ev in &reasoning_events {
            println!("  - {}", serde_json::to_string(ev).unwrap());
        }

        let output_items: Vec<serde_json::Value> = events
            .iter()
            .filter(|ev| event_type(ev) == "response.output_item.done")
            .filter_map(|ev| ev.get("item").cloned())
            .collect();
        let reasoning_items: Vec<&serde_json::Value> = output_items
            .iter()
            .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("reasoning"))
            .collect();
        println!("[probe-r2a] summary={summary} output items of type=reasoning: {}", reasoning_items.len());
        for it in &reasoning_items {
            println!("  - {}", serde_json::to_string(it).unwrap());
        }

        if !reasoning_events.is_empty() || !reasoning_items.is_empty() {
            chosen = Some((summary, body.clone(), output_items));
            break;
        }
    }

    let Some((summary, round1_body, output_items)) = chosen else {
        let msg = "試過 summary=auto/detailed/concise，effort=medium，皆未觀察到任何 type 含 \
            \"reasoning\" 字樣的 SSE 事件，也沒有 output item type=reasoning。結論：在這個 \
            prompt/帳號/模型組合下，reasoning 內容對 client 不可見（後端可能仍在內部推理，\
            但不外洩摘要）。";
        write_dump("codex_probe_r2_reasoning_SUMMARY.txt", msg);
        println!("[probe-r2a] === {msg} ===");
        return;
    };

    println!("[probe-r2a] === 用 summary={summary} 觀察到 reasoning 相關內容，繼續做多輪回送對照 ===");

    // ---- 多輪回送對照：reasoning item 要不要塞回去？----
    let base_input = round1_body["input"].as_array().cloned().unwrap_or_default();
    let follow_up = serde_json::json!({
        "type": "message",
        "role": "user",
        "content": [{ "type": "input_text", "text": "Thanks. Now double-check your makespan is optimal and explain why in one sentence." }]
    });

    // 對照 A：round1 所有 output items（含 reasoning item）原樣回送。
    let mut input_a = base_input.clone();
    input_a.extend(output_items.iter().cloned());
    input_a.push(follow_up.clone());
    let mut body_a = round1_body.clone();
    body_a["input"] = serde_json::Value::Array(input_a);

    write_dump(
        "codex_probe_r2_reasoning_round2_with_item_request.json",
        &serde_json::to_string_pretty(&body_a).unwrap(),
    );
    let resp_a = client
        .apply_headers(http.post(client.responses_url()))
        .json(&body_a)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round2(with reasoning item) request failed to send: {e}"));
    let (status_a, _) =
        dump_response(resp_a, "codex_probe_r2_reasoning_round2_with_item_raw_sse.txt").await;
    println!("[probe-r2a] round2 WITH reasoning item replayed: status={status_a}");

    // 對照 B：只塞非 reasoning 的 output items，reasoning item 拿掉。
    let non_reasoning_items: Vec<serde_json::Value> = output_items
        .iter()
        .filter(|item| item.get("type").and_then(|t| t.as_str()) != Some("reasoning"))
        .cloned()
        .collect();
    let mut input_b = base_input.clone();
    input_b.extend(non_reasoning_items);
    input_b.push(follow_up.clone());
    let mut body_b = round1_body.clone();
    body_b["input"] = serde_json::Value::Array(input_b);

    write_dump(
        "codex_probe_r2_reasoning_round2_without_item_request.json",
        &serde_json::to_string_pretty(&body_b).unwrap(),
    );
    let resp_b = client
        .apply_headers(http.post(client.responses_url()))
        .json(&body_b)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round2(without reasoning item) request failed to send: {e}"));
    let (status_b, _) =
        dump_response(resp_b, "codex_probe_r2_reasoning_round2_without_item_raw_sse.txt").await;
    println!("[probe-r2a] round2 WITHOUT reasoning item replayed: status={status_b}");

    let summary_txt = format!(
        "chosen summary variant = {summary}\n\
         round2 WITH reasoning item replayed: status={status_a}\n\
         round2 WITHOUT reasoning item replayed: status={status_b}\n"
    );
    write_dump("codex_probe_r2_reasoning_SUMMARY.txt", &summary_txt);
    println!("[probe-r2a] === SUMMARY ===\n{summary_txt}");
}

/// 問題 B：並行工具呼叫。
///
/// 第一輪只驗證了單一工具呼叫；這一輪給兩個明顯獨立的工具，並在 prompt
/// 裡明確要求「同時」呼叫兩者，觀察：
/// 1. 模型會不會真的在同一輪發出 ≥2 個 function_call。
/// 2. `response.output_item.added` 與 `response.function_call_arguments.delta`
///    如何交錯，delta 靠 `item_id` 還是 `output_index` 歸屬到正確的呼叫。
/// 3. 多輪回送 2 組 function_call/function_call_output 時，排列順序（成對
///    相鄰 vs 先全部 call 再全部 output）是否都被接受。
#[tokio::test]
#[ignore = "探勘測試：需要真實 Codex OAuth 憑證與網路，不進常規 CI"]
async fn probe_codex_parallel_tool_calls() {
    let secrets = SecretStore::new();
    let (access_token, account_id) = get_valid_codex_oauth_token(PROVIDER_ID, &secrets)
        .await
        .unwrap_or_else(|e| panic!("BLOCKED: 拿不到 {PROVIDER_ID} 的有效 token: {e:?}"));
    println!("[probe-r2b] got token (len={}), account_id={:?}", access_token.len(), account_id);

    let client =
        CodexClient::with_base_url(access_token, MODEL.into(), account_id, "https://chatgpt.com".into());
    let http = reqwest::Client::new();

    let tools = serde_json::json!([
        {
            "type": "function",
            "name": "get_weather",
            "description": "Get the current weather for a given city",
            "parameters": {
                "type": "object",
                "properties": { "city": { "type": "string", "description": "City name" } },
                "required": ["city"]
            }
        },
        {
            "type": "function",
            "name": "list_directory",
            "description": "List the files in a given directory path",
            "parameters": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Absolute directory path" } },
                "required": ["path"]
            }
        }
    ]);

    let req = GenerateRequest {
        system_prompt: "You are a coding CLI assistant with access to tools. When a request \
            has multiple independent sub-tasks that each map to a distinct tool, call ALL the \
            relevant tools in the same turn rather than one at a time.".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!(
                "Do these two independent things right now, in parallel: (1) get the current \
                 weather in Taipei, and (2) list the files in /tmp. Call both tools in this \
                 same turn."
            ),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: env_ctx(),
        mode: QueryMode::Chat,
        max_tokens: Some(512),
    };

    let mut round1_body = build_request_body(MODEL, &req);
    round1_body["tools"] = tools.clone();

    write_dump(
        "codex_probe_r2_parallel_round1_request.json",
        &serde_json::to_string_pretty(&round1_body).unwrap(),
    );

    let resp = client
        .apply_headers(http.post(client.responses_url()))
        .json(&round1_body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("parallel round1 request failed to send: {e}"));
    let (status, raw) = dump_response(resp, "codex_probe_r2_parallel_round1_raw_sse.txt").await;
    println!("[probe-r2b] parallel round1 status={status}");

    if status >= 400 {
        println!("[probe-r2b] 端點拒絕 (HTTP {status})，探勘到此為止");
        return;
    }

    let events = parse_sse_events(&raw);

    let added_events: Vec<&serde_json::Value> =
        events.iter().filter(|ev| event_type(ev) == "response.output_item.added").collect();
    let delta_events: Vec<&serde_json::Value> = events
        .iter()
        .filter(|ev| event_type(ev) == "response.function_call_arguments.delta")
        .collect();
    let done_item_events: Vec<&serde_json::Value> =
        events.iter().filter(|ev| event_type(ev) == "response.output_item.done").collect();

    println!("[probe-r2b] output_item.added count={}", added_events.len());
    for ev in &added_events {
        println!(
            "  added: output_index={:?} item.id={:?} item.call_id={:?} item.name={:?} item.type={:?}",
            ev.get("output_index"),
            ev.get("item").and_then(|i| i.get("id")),
            ev.get("item").and_then(|i| i.get("call_id")),
            ev.get("item").and_then(|i| i.get("name")),
            ev.get("item").and_then(|i| i.get("type")),
        );
    }
    println!("[probe-r2b] function_call_arguments.delta count={}", delta_events.len());
    for ev in delta_events.iter().take(30) {
        println!(
            "  delta: item_id={:?} output_index={:?} delta={:?}",
            ev.get("item_id"),
            ev.get("output_index"),
            ev.get("delta"),
        );
    }

    write_dump(
        "codex_probe_r2_parallel_added_and_delta_events.json",
        &serde_json::to_string_pretty(&serde_json::json!({
            "added": added_events,
            "deltas": delta_events,
        }))
        .unwrap(),
    );

    let function_call_items: Vec<serde_json::Value> = done_item_events
        .iter()
        .filter_map(|ev| ev.get("item").cloned())
        .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("function_call"))
        .collect();

    write_dump(
        "codex_probe_r2_parallel_function_call_items.json",
        &serde_json::to_string_pretty(&function_call_items).unwrap(),
    );
    println!(
        "[probe-r2b] function_call output items (from output_item.done): {}",
        function_call_items.len()
    );
    for item in &function_call_items {
        println!("  - {}", serde_json::to_string(item).unwrap());
    }

    if function_call_items.len() < 2 {
        let msg = format!(
            "模型這輪只呼叫了 {} 個工具（未觀察到並行呼叫），儘管 parallel_tool_calls=true \
             是預設值且 prompt 明確要求同時呼叫兩個。可能原因：這個 prompt/模型組合下模型 \
             選擇序列化呼叫，或先呼叫一個、看到結果後才呼叫下一個。因為呼叫數 <2，跳過多輪 \
             回送排列順序對照（沒有意義）。",
            function_call_items.len()
        );
        write_dump("codex_probe_r2_parallel_SUMMARY.txt", &msg);
        println!("[probe-r2b] === {msg} ===");
        return;
    }

    // ==================== 多輪回送排列順序對照 ====================
    let base_input = round1_body["input"].as_array().cloned().unwrap_or_default();

    fn tool_output_for(call_id: &str, name: &str) -> serde_json::Value {
        let output = if name == "get_weather" {
            "{\"temperature_c\": 29, \"condition\": \"partly cloudy\"}"
        } else {
            "{\"files\": [\"a.txt\", \"b.txt\"]}"
        };
        serde_json::json!({ "type": "function_call_output", "call_id": call_id, "output": output })
    }

    let call_ids: Vec<(String, String)> = function_call_items
        .iter()
        .map(|item| {
            (
                item.get("call_id").and_then(|v| v.as_str()).unwrap_or("MISSING").to_string(),
                item.get("name").and_then(|v| v.as_str()).unwrap_or("MISSING").to_string(),
            )
        })
        .collect();

    // 排列 A：成對相鄰（call1, output1, call2, output2, ...）。
    let mut input_paired = base_input.clone();
    for (item, (call_id, name)) in function_call_items.iter().zip(call_ids.iter()) {
        input_paired.push(item.clone());
        input_paired.push(tool_output_for(call_id, name));
    }
    let mut body_paired = round1_body.clone();
    body_paired["input"] = serde_json::Value::Array(input_paired);

    write_dump(
        "codex_probe_r2_parallel_round2_paired_request.json",
        &serde_json::to_string_pretty(&body_paired).unwrap(),
    );
    let resp_paired = client
        .apply_headers(http.post(client.responses_url()))
        .json(&body_paired)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round2 paired request failed to send: {e}"));
    let (status_paired, _) =
        dump_response(resp_paired, "codex_probe_r2_parallel_round2_paired_raw_sse.txt").await;
    println!("[probe-r2b] round2 排列=成對相鄰 (call1,output1,call2,output2): status={status_paired}");

    // 排列 B：先全部 call 再全部 output。
    let mut input_grouped = base_input.clone();
    for item in &function_call_items {
        input_grouped.push(item.clone());
    }
    for (call_id, name) in &call_ids {
        input_grouped.push(tool_output_for(call_id, name));
    }
    let mut body_grouped = round1_body.clone();
    body_grouped["input"] = serde_json::Value::Array(input_grouped);

    write_dump(
        "codex_probe_r2_parallel_round2_grouped_request.json",
        &serde_json::to_string_pretty(&body_grouped).unwrap(),
    );
    let resp_grouped = client
        .apply_headers(http.post(client.responses_url()))
        .json(&body_grouped)
        .send()
        .await
        .unwrap_or_else(|e| panic!("round2 grouped request failed to send: {e}"));
    let (status_grouped, _) =
        dump_response(resp_grouped, "codex_probe_r2_parallel_round2_grouped_raw_sse.txt").await;
    println!("[probe-r2b] round2 排列=先全部 call 再全部 output: status={status_grouped}");

    let summary = format!(
        "round1 status={status}\nfunction_call items observed={}\ncall_ids={call_ids:?}\n\
         round2 排列=成對相鄰 status={status_paired}\n\
         round2 排列=先全部call再全部output status={status_grouped}\n",
        function_call_items.len()
    );
    write_dump("codex_probe_r2_parallel_SUMMARY.txt", &summary);
    println!("[probe-r2b] === SUMMARY ===\n{summary}");
}
