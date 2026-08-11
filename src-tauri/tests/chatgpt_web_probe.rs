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
        r#"data: {"message":{"id":"m1","author":{"role":"assistant"},"content":{"parts":["你好"#,
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
// 2. ✅ `/ai` 可用（2026-08-10 實測）。注意 `/ai` 不會逐字顯示——那條路徑要收
//    完整份 JSON 才解析得出 command/explanation，然後一次顯示 CommandPreview。
//    逐字串流要在聊天面板（AiPanel）才看得到，那條**尚未實測**。
//
// 3. ✅ 聊天面板（右上角的「Ask AI」）帶 MCP 工具，2026-08-11 實測通過：
//    模型發出 `<tool>{"name":"mcp_brave_search__brave_web_search","arguments":
//    {...},"_nonce":"d2b8…"}</tool>`，nonce 正確、工具名的 `__` 前綴完整、
//    沒有包 code fence；工具執行後**第二輪正常用結果作答**，沒有踩到「歷史
//    裡的封套形狀」那個洞。
//
//    ⚠️ **重要前提：要點名工具它才會用。** 這個模型自己就有 web search，
//    問「查匯率」「查 Tauri 2.10」時它直接用內建搜尋回答（回覆裡帶
//    `citeturn0search…` 標記），完全不碰我們的契約；問本機檔案時則傾向回
//    `<cmd>`（那是 `build_chat_prompt` 教的格式）。實測要說「請用
//    `<server>__<tool>` 這個工具查…」才會走契約。
//
//    這不是契約失效——契約送達與否已用診斷確認過（has_contract=true、
//    nonce_in_text=true）。是模型有更方便的替代品時不會選陌生的純文字協定。
//    若要提高自發使用率，方向是 A：帶工具時不要在 system prompt 放 `<cmd>`
//    規則，讓 `<tool>` 成為唯一被授權的行動方式。目前**沒有**做這件事，
//    因為那會拿掉「點一下執行指令」這個既有功能。
//
// 4. ❌ Claude Code 橋接：**刻意不支援**（2026-08-11 實測後決定）。
//    路由與供應商解析都正常（log 有 `model=aiterm:sonnet → provider=Chatgpt-web`），
//    但每一發都被上游擋下：`message_length_exceeds_limit`。
//
//    量到的數字：總長 163,206 字元 = system 27,935 + 42 個工具的完整 schema
//    約 135,000。同一條傳輸在 `/ai` 下 24,092 字元是成功的，所以超標 5–7 倍，
//    主因是工具 schema。
//
//    成因是設計上的先天衝突：網頁版沒有結構化的 system role，所以必須把所有
//    東西攤平成單一則訊息，而網頁版又對單則訊息設了上限。
//
//    `kind_for` 已改回 `None`，並給了專屬的拒絕訊息（通用的「還不支援」會讓
//    人以為等一等就有）。`bridge/upstream/chatgpt_web.rs` 的實作與測試留著，
//    重新啟用只需要改 `kind_for` 那一行——條件見那裡的註解。
//
//    ✅ 拒絕訊息已端到端驗證（2026-08-11）：Claude Code 收到
//    `API Error: 400 invalid input:「…」不能用於 Claude Code 橋接：…`，
//    含原因、實測數字、以及「仍可用於 /ai 與 Ask AI 面板」的替代路徑。
//
// 5. ✅ 已錄過一次（2026-08-10），發現並修掉兩個 bug，fixture 已進
//    `protocol.rs` 的測試（`user_echo_frame_is_not_treated_as_the_answer` 等）：
//    a. ✅ 串流會**回放我們自己送出去的那則訊息**（`author.role == "user"`，
//       內容就是完整的 system prompt），還夾著若干 `role:"system"` 的空訊息與
//       `{"type":"input_message"}` 封套。原本沒有依 role 過濾，`/ai` 收到的
//       「回答」是自己的 prompt 原樣回傳，表現成「AI 回傳格式錯誤」。
//    b. ✅ 用量上限這類錯誤是 **HTTP 200 + SSE body 裡的 error 欄位**
//       （`{"message":null,"error":"你已達到上限。","error_code":"usage_limit"}`），
//       原本完全沒被偵測到。已加 `SseOut::Error` 與 `map_stream_error`。
//    c. ✅ **沒有觀察到交錯**。一次請求裡出現三個不同的 assistant message id：
//       前兩個沒有字串 `parts[0]`（content_type 不是 text，例如思考內容），
//       被 parts guard 攔下、不會碰到 `current_id`/`emitted`；第三個才是答案，
//       而且是逐步累積的（0 → 12 → 32 → 77 字元）。答案訊息一旦開始，後續
//       frame 全屬於它。**保留** `emitted` 單一份的設計。
//       ⚠️ 這只是一次取樣，而且非答案的那兩則剛好沒有可讀文字。若換成會輸出
//       思考文字的模型，交錯仍有可能——那時就要改成以 `message.id` 為鍵。
//       真實序列已寫成測試 `real_captured_frame_sequence_yields_only_incremental_answer`。
//    d. ✅ **chunk 真的會切在行中間**，實測抓到：
//         [len=4220 ends_nl=false] tail="…,\"conversation_id\":\"6a79"
//         [len=36   ends_nl=true ] tail="e841-de64-83ee-a31b-fd9e6eaafd61\"}\n\n"
//       `conversation_id` 被從中間切開，下一個 36 bytes 才補完。行緩衝是載重的
//       ——沒有它那個 frame 兩半都無法剖析、會被靜默丟掉。已寫成測試
//       `real_chunk_boundary_falls_mid_line`。
//
// 6. ✅ 已量（2026-08-11，在真實的 chatgpt.com 頁面上）：
//
//      Object.keys(navigator).length === 0        ← 恆為 0，假設成立
//      Object.keys(document).length   === 1..5    ← 只有頁面腳本自己加的
//      Object.keys(window).length     === 221..270
//      for (k in navigator) → 40   (hardwareConcurrency, gpu, clipboard, …)
//      for (k in document)  → 264
//
//    也就是 config[10] 恆為空字串、config[11] 送的是「這個頁面剛好被加了
//    什麼」（數量還會隨時間變）。已把 `pickKey` 改成 `for...in` 列舉。
//
//    ⚠️ 改的依據是推論不是實測：我們看不到 OpenAI 的腳本。支持的證據是
//    OmniRoute 硬編的是一份 `Navigator.prototype` 方法名清單——他們跑在
//    伺服器上、沒有瀏覽器，不會無故硬編那種形狀。改的理由是不對稱：對方
//    不檢查則改不改都一樣，對方會檢查則恆為空是很明顯的訊號。
//
//    **這個改動沒有任何回饋能驗證好壞**，改前改後上游都接受。若日後出現
//    疑似指紋偵測的症狀，這裡是第一個要回頭檢查的地方。
