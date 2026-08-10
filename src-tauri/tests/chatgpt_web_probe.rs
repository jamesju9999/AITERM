//! ChatGPT 網頁版的端到端檢查。
//!
//! 這裡刻意**不**做需要網路的自動化整合測試——上游隨時變動，放進 CI 只會
//! 產生假紅燈。真正需要真實登入與 GUI 的驗證見本檔尾端的「手動驗證清單」。
//!
//! 這個檔案裡的測試不需要網路，但仍標成 `#[ignore]`：它們驗證的是跨模組的
//! 契約自洽性，屬於「發佈前跑一次」的性質，不是每次 `cargo test` 都要付的
//! 成本。手動執行：
//!
//! ```bash
//! cargo test --test chatgpt_web_probe -- --ignored --nocapture
//! ```

use aiterm_lib::ai::McpToolDefinition;
use aiterm_lib::chatgpt_web::protocol::{FlatTurn, SseOut, SseParser};
use aiterm_lib::chatgpt_web::tools;

fn defs() -> Vec<McpToolDefinition> {
    vec![McpToolDefinition {
        name: "filesystem__read_text_file".into(),
        description: "讀取一個文字檔".into(),
        input_schema: serde_json::json!({"type": "object"}),
    }]
}

/// 契約要求模型輸出的封套，剖析器必須認得。
///
/// 這兩段程式碼只靠字串常數對齊——任一邊漂移都不會有編譯錯誤，表現出來只會
/// 是「模型不會用工具」，而真正的原因藏在一個字串常數裡。
#[test]
#[ignore = "發佈前的跨模組契約檢查，不需要網路"]
fn contract_and_parser_agree_on_the_envelope() {
    let nonce = "test-nonce";
    let contract = tools::build_contract(&defs(), nonce);
    assert!(contract.contains(nonce), "契約要帶 nonce");

    // 模仿模型照契約輸出。
    let reply = format!(
        r#"好的。<tool>{{"name":"filesystem__read_text_file","arguments":{{"path":"a.txt"}},"_nonce":"{nonce}"}}</tool>"#
    );
    let (content, calls) = tools::parse_tool_calls(&reply, nonce);
    let calls = calls.expect("契約產生的封套必須被自己的剖析器認得");
    assert_eq!(calls[0].tool_name, "filesystem__read_text_file");
    assert_eq!(calls[0].args, serde_json::json!({"path": "a.txt"}));
    assert!(content.contains("好的"));
    assert!(!content.contains("<tool>"), "封套要從內容剝掉");
}

/// 完整的一輪：攤平 → 契約 → 模型回覆 → 剖析 → 結果回填 → 再攤平。
///
/// 守的是「第二輪的 prompt 裡不能再出現 `<tool>` 形狀」——那是 few-shot 教
/// 模型省略 nonce 的來源，會讓第二輪起的工具呼叫全被拒絕。
#[test]
#[ignore = "發佈前的跨模組契約檢查，不需要網路"]
fn second_turn_prompt_carries_no_envelope_shapes() {
    let nonce = "n1";
    let reply = format!(
        r#"我來讀檔。<tool>{{"name":"filesystem__read_text_file","arguments":{{"path":"a.txt"}},"_nonce":"{nonce}"}}</tool>"#
    );
    let (_, calls) = tools::parse_tool_calls(&reply, nonce).1.map_or_else(
        || panic!("第一輪應該剖析得出工具呼叫"),
        |c| (String::new(), c),
    );

    // 第二輪：把上一輪的呼叫與結果攤回歷史。
    let turns = vec![
        FlatTurn::User("請讀 a.txt".into()),
        FlatTurn::ToolCall {
            id: calls[0].id.clone(),
            name: calls[0].tool_name.clone(),
            args: calls[0].args.to_string(),
        },
        FlatTurn::ToolResult { id: calls[0].id.clone(), content: "檔案內容".into() },
    ];
    let text = aiterm_lib::chatgpt_web::protocol::flatten_history("你是助理", &turns);

    assert!(text.contains("[[tool_call:filesystem__read_text_file#"), "實際：{text}");
    assert!(text.contains("[[tool_result:"));
    assert!(!text.contains("<tool>"), "第二輪 prompt 不可出現封套形狀：{text}");
    assert!(!text.contains("_nonce"), "舊 nonce 不該被帶進新的 prompt：{text}");
}

/// SSE 快照差分在「chunk 切在行中間」時仍要正確——這是實測會發生的情況
/// （21 個 chunk、每個約 500 bytes，不保證切在 `\n` 上）。
#[test]
#[ignore = "發佈前的跨模組契約檢查，不需要網路"]
fn split_chunks_reassemble_into_incremental_deltas() {
    let mut p = SseParser::default();
    let mut got = String::new();
    // 一份累積快照被切成三段，最後一段才帶換行。
    for chunk in [
        r#"data: {"message":{"id":"m1","content":{"parts":["你好"#,
        r#"，世界"]}}}"#,
        "\n",
    ] {
        for out in p.feed_str(chunk) {
            if let SseOut::Text(d) = out {
                got.push_str(&d);
            }
        }
    }
    assert_eq!(got, "你好，世界", "切在行中間的 frame 要被接回來");
}

// ── 手動驗證清單（需要真實登入與 GUI，無法自動化）─────────────────────────
//
// 1. 設定頁新增 chatgpt-web 供應商 → 按「登入 ChatGPT」→ 視窗出現 → 登入完成
//    後視窗自動收起 → 模型下拉出現該帳號可用的清單。
//    - 若帳號用「Continue with Google」登入：Google 的 disallowed_useragent
//      政策會擋掉內嵌 webview 裡的 OAuth 流程。要確認這條路走不走得完，
//      走不完的話 watchLogin 會安靜地等到 10 分鐘 deadline。
//
// 2. `/ai` 下一句話 → 逐字串流出現 → 回答結尾完整（不少一截）。
//
// 3. 聊天面板帶 MCP 工具 → 第一個工具呼叫成功 → **第二、三輪仍然成功**。
//    第二輪起失效是這條路徑最容易出的問題（歷史裡的封套形狀）。
//
// 4. Claude Code 橋接指到這個供應商 → 多輪工具迴圈跑得完。
//
// 5. 錄一條真實的 SSE 串流存成 fixture，回頭補測試。探勘程式碼已移除，改用
//    在 `chatgpt_web::session::Session::push_chunk` 暫時加一行
//    `eprintln!("{data}")` 來擷取，錄完拿掉。要確認三件事：
//    a. 非內容 frame（moderation、`role:"system"`、只帶 conversation_id）
//       實際長什麼樣，`SseParser` 是否真的原封不動略過。
//    b. **兩則訊息會不會在同一條串流裡交錯**（思考區塊與答案）。目前
//       `SseParser` 只保留一份 `emitted`，交錯時每次切換都會整段重送；
//       若確認會交錯，`emitted` 要改成以 `message.id` 為鍵。
//    c. chunk 實際切在哪裡——確認行緩衝真的有派上用場。
//
// 6. 量一次 `[Object.keys(navigator).length, Object.keys(document).length,
//    Object.keys(window).length]`。前兩者在瀏覽器裡預期是 0（屬性都在
//    prototype 上），若屬實則 `pickKey(nav)` / `pickKey(document)` 在生產
//    環境是死碼，config[10]/[11] 恆為空字串——那與「18 格全是真值」這個
//    賣點不符，要決定是否改成 prototype 列舉。
