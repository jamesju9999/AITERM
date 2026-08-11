//! ChatGPT 網頁版的協定轉換：歷史攤平與 SSE 解析。

/// 攤平前的一個回合。
///
/// 網頁版沒有 `role: "tool"` 這種結構化角色，工具回合只能以文字回填——
/// 這是此傳輸路徑的先天限制，不是實作取捨。見 spec 的「工具結果的回填」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlatTurn {
    User(String),
    Assistant(String),
    ToolCall { id: String, name: String, args: String },
    ToolResult { id: String, content: String },
}

/// 把 system prompt 與整段歷史攤平成單一 user turn 的內容。
///
/// 界定符 `[[tool_call:…]]` / `[[tool_result:…]]` 刻意與模型要輸出的
/// `<tool>` 封套不同：若共用同一組標籤，`tools::parse_tool_calls` 可能把
/// 歷史裡我們自己寫進去的回合誤判成模型發出的新呼叫。nonce 檢查雖然也會
/// 擋下（歷史回合沒有 `_nonce`），但不該把正確性建立在第二道防線上。
///
/// `system_prompt` 會先 `trim()` 再判斷是否為空、也用 trim 後的內容放進輸出，
/// 前後空白／換行不會殘留。system 區塊刻意**沒有** `System:` 前綴——這段文字
/// 要讀起來像框架敘述本身，加前綴會讓模型把它降格成轉錄稿裡的一個回合。
///
/// 輸出格式範例（system + user + assistant + tool_call + tool_result）：
///
/// ```text
/// 你是助理
///
/// User: 第一問
///
/// Assistant: 第一答
///
/// [[tool_call:Read#call_1]]
/// {"path":"a.txt"}
/// [[/tool_call]]
///
/// [[tool_result:call_1]]
/// 檔案內容
/// [[/tool_result]]
/// ```
pub fn flatten_history(system_prompt: &str, turns: &[FlatTurn]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !system_prompt.trim().is_empty() {
        parts.push(system_prompt.trim().to_string());
    }
    for turn in turns {
        parts.push(match turn {
            FlatTurn::User(text) => format!("User: {text}"),
            FlatTurn::Assistant(text) => format!("Assistant: {text}"),
            FlatTurn::ToolCall { id, name, args } => {
                format!("[[tool_call:{name}#{id}]]\n{args}\n[[/tool_call]]")
            }
            FlatTurn::ToolResult { id, content } => {
                format!("[[tool_result:{id}]]\n{content}\n[[/tool_result]]")
            }
        });
    }
    parts.join("\n\n")
}

/// 解析出的一個事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseOut {
    Text(String),
    /// 串流內的錯誤（HTTP 200，錯誤藏在 SSE body 裡）。`code` 例如
    /// `usage_limit`，用來對應到可行動的訊息。
    Error {
        message: String,
        code: String,
    },
    Done,
}

/// ChatGPT 網頁版 SSE 的解析器。
///
/// 呼叫端餵原始 chunk（`feed_str`），不需要自己切行——HTTP chunk 不保證切在
/// 行邊界上，殘缺的尾巴留在 `pending` 等下一個 chunk 接上。因為每個 frame 是
/// 累積快照，中段被丟的 frame 會被下一個補回來，但**最後一個內容 frame 沒有
/// 下一個**，所以少了行緩衝就是「回答結尾永久少一截且無聲無息」。
///
/// 每個 data frame 是「到目前為止的完整文字」快照而非增量，所以要保留已送出
/// 的內容自己算差分。
///
/// `Done` 不會終止解析器，呼叫端負責停止餵資料。
#[derive(Default)]
pub struct SseParser {
    pending: String,
    current_id: Option<String>,
    emitted: String,
}

impl SseParser {
    /// 餵一段原始 chunk。只有完整的行（遇到 `\n`）會被處理，殘缺的尾巴留著。
    ///
    /// 呼叫端必須確保串流最後一行有換行結尾，否則它會永遠留在緩衝裡——
    /// 注入腳本送的 `data: [DONE]` 因此要帶 `\n`。
    pub fn feed_str(&mut self, chunk: &str) -> Vec<SseOut> {
        self.pending.push_str(chunk);
        let mut out = Vec::new();
        while let Some(nl) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=nl).collect();
            out.extend(self.feed_line(line.trim_end_matches(['\r', '\n'])));
        }
        out
    }

    fn feed_line(&mut self, line: &str) -> Vec<SseOut> {
        let Some(payload) = line.strip_prefix("data:") else {
            return Vec::new();
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            return vec![SseOut::Done];
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
            return Vec::new();
        };
        // 串流裡的錯誤 frame：`{"message":null,"error":"…","error_code":"usage_limit"}`。
        // 這種 frame HTTP 狀態是 200，錯誤藏在 SSE body 裡——不辨識的話會被
        // 當成「沒有內容的 frame」略過，使用者最後看到的是一個跟真正原因無關
        // 的下游錯誤（實測：用量上限被表現成「AI 回傳格式錯誤」）。
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            let code = v.get("error_code").and_then(|c| c.as_str()).unwrap_or("");
            return vec![SseOut::Error { message: err.to_string(), code: code.to_string() }];
        }

        let Some(msg) = v.get("message") else {
            return Vec::new();
        };
        // **只收 assistant 的訊息。**
        //
        // `/backend-api/conversation` 的串流會回放整段對話，包括我們自己剛送
        // 出去的那則（`author.role == "user"`，內容就是完整的 system prompt）
        // 與若干 `role:"system"` 的空訊息。不過濾的話，第一個被當成「回答」
        // 收下的就是我們自己的 prompt——實測時 `/ai` 收到的正是它原樣回傳，
        // 表現成「AI 回傳格式錯誤」。
        //
        // 用「不是 assistant 就跳過」而不是「排除 user/system」：上游隨時可能
        // 加新的 role（tool、moderation…），白名單才不會漏。
        if msg.get("author").and_then(|a| a.get("role")).and_then(|r| r.as_str())
            != Some("assistant")
        {
            return Vec::new();
        }
        // 沒有可讀文字的 frame（moderation、role:"system" 的空訊息、只帶
        // conversation_id 的 frame）在串流裡是常態。這種 frame 必須完全不碰
        // 差分狀態：先前用 `.unwrap_or("")` 把它壓成空字串，會讓 emitted 被
        // 重設，下一個真正的快照就整段重送，使用者看到重複的文字。
        let Some(full) = msg
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .and_then(|s| s.as_str())
        else {
            return Vec::new();
        };
        // message.id 換了就是換了一則訊息（例如思考區塊結束、答案開始），
        // 重新起算。用 id 而不是「內容對不上」來判斷，是因為後者無法區分
        // 「換訊息」與其他情況。
        //
        // 注意：這假設兩則訊息不會在同一條串流裡交錯（交錯時每次切換都會
        // 整段重送）。Task 16 的端到端探針要錄一條真實串流確認這件事。
        if let Some(id) = msg.get("id").and_then(|s| s.as_str()) {
            if self.current_id.as_deref() != Some(id) {
                self.current_id = Some(id.to_string());
                self.emitted.clear();
            }
        }
        // 用 strip_prefix 而非 `full[self.emitted.len()..]`：後者按 byte 切字串，
        // 當這個 frame 不是前一個的延續時，切點會落在多位元組字元的中間而
        // panic。實測：emitted="ok"、full="你好世界" 會炸在
        // 「byte index 2 is not a char boundary」。中文內容下這不是理論風險。
        match full.strip_prefix(self.emitted.as_str()) {
            // 沒有新增內容（上游重送同一份快照）。
            Some("") => Vec::new(),
            // 正常的累積快照：只回新增的那一段。
            Some(delta) => {
                let delta = delta.to_string();
                self.emitted = full.to_string();
                vec![SseOut::Text(delta)]
            }
            // 不是延續，而且 id 也沒告訴我們換了訊息。整段當新內容送出——
            // 不能沿用「比較長度」的判斷：新訊息比舊的短時會被整段靜默吃掉。
            None => {
                // 上游送了一個空的 parts[0] 但 id 沒變：不是新內容，忽略。
                // 這跟上面那個 `Some(full) = ... else` guard 攔的不是同一類
                // 輸入——那個攔的是「結構上就沒有 content/parts 欄位」（例如
                // `message:null`），這裡攔的是「有 parts[0]，但它是空字串」
                // （例如 `parts:[""]`）。單獨拿掉這裡這道防護（改回「先
                // reset 再判斷」的舊順序）就足以讓
                // `frame_with_empty_parts_mid_stream_does_not_reset_state`
                // 變紅；`mid_stream_frame_without_text_does_not_reset_state`
                // 用的 `message:null` 則會在更早的 `Some(full) = ... else`
                // 就被整段攔下，不會走到這裡，所以單獨拿掉上面那道 guard
                // 不會讓任何測試變紅——要兩道防護都拿掉，`message:null` 那條
                // 測試才會變紅。兩者不是重複，不要合併刪掉。
                if full.is_empty() {
                    return Vec::new();
                }
                let owned = full.to_string();
                self.emitted = owned.clone();
                vec![SseOut::Text(owned)]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_system_and_turns_in_order() {
        let out = flatten_history(
            "你是助理",
            &[
                FlatTurn::User("第一問".into()),
                FlatTurn::Assistant("第一答".into()),
                FlatTurn::User("第二問".into()),
            ],
        );
        assert!(out.starts_with("你是助理"), "system 要在最前面，實際：{out}");
        let first = out.find("第一問").unwrap();
        let second = out.find("第二問").unwrap();
        assert!(first < second, "順序要保留");
        assert!(out.contains("第一答"), "實際：{out}");
    }

    /// 鎖住 `trim()`：純空白（非空字串）的 system prompt 也要被視為「無 system」，
    /// 不能只靠 `is_empty()` 判斷，否則會在輸出前面留下一段空白／空行。
    #[test]
    fn whitespace_only_system_prompt_omitted() {
        let out = flatten_history("   \n\t", &[FlatTurn::User("只有這句".into())]);
        assert!(
            !out.starts_with(char::is_whitespace),
            "純空白 system 不該留下開頭空白，實際：{out:?}"
        );
        assert_eq!(
            out, "User: 只有這句",
            "純空白 system 應該被整段省略，實際：{out:?}"
        );
    }

    /// 界定符刻意與模型要輸出的 `<tool>` 封套不同——共用會讓剖析器把歷史
    /// 誤判成模型發出的新呼叫。
    #[test]
    fn tool_turns_use_distinct_delimiters() {
        let out = flatten_history(
            "",
            &[
                FlatTurn::ToolCall { id: "call_1".into(), name: "Read".into(),
                                     args: r#"{"path":"a.txt"}"#.into() },
                FlatTurn::ToolResult { id: "call_1".into(), content: "檔案內容".into() },
            ],
        );
        assert!(out.contains("[[tool_call:Read#call_1]]"), "實際：{out}");
        assert!(out.contains(r#"{"path":"a.txt"}"#), "實際：{out}");
        assert!(out.contains("[[/tool_call]]"), "實際：{out}");
        assert!(out.contains("[[tool_result:call_1]]"), "實際：{out}");
        assert!(out.contains("檔案內容"), "實際：{out}");
        assert!(out.contains("[[/tool_result]]"), "實際：{out}");
        assert!(!out.contains("<tool>"), "不可使用模型輸出用的封套標籤，實際：{out}");
        assert!(!out.contains("<tool_call"), "不可使用模型輸出用的封套標籤，實際：{out}");
    }

    /// 精確比對整段攤平後的輸出。這是線上格式本身（後續 Task 3–5 的剖析器要
    /// 靠它吃回來），涵蓋全部四種 `FlatTurn` 變體加一個帶前後空白／尾端換行的
    /// system prompt——Claude Code 送來的 system 區塊幾乎必然帶尾端換行，若
    /// 只用乾淨字串測試，格式鎖對最常見的真實輸入其實不成立。一次鎖住：
    /// system 位置與 trim、角色前綴的確切文字、`\n\n` 分隔符、工具界定符、
    /// 以及工具區塊的先後順序。上面幾個測試表達的是個別意圖，各自的錯誤訊息
    /// 在壞掉時比較好讀；這個測試是額外一道格式鎖，不是取代品。
    #[test]
    fn flattened_format_is_exact() {
        let out = flatten_history(
            "\n你是助理\n",
            &[
                FlatTurn::User("第一問".into()),
                FlatTurn::Assistant("第一答".into()),
                FlatTurn::ToolCall {
                    id: "call_1".into(),
                    name: "Read".into(),
                    args: r#"{"path":"a.txt"}"#.into(),
                },
                FlatTurn::ToolResult { id: "call_1".into(), content: "檔案內容".into() },
            ],
        );
        let expected = "你是助理\n\n\
            User: 第一問\n\n\
            Assistant: 第一答\n\n\
            [[tool_call:Read#call_1]]\n{\"path\":\"a.txt\"}\n[[/tool_call]]\n\n\
            [[tool_result:call_1]]\n檔案內容\n[[/tool_result]]";
        assert_eq!(out, expected);
    }

    /// 使用者內容本身可能就含有界定符字面文字——例如用 `Read` 讀進本 repo 的
    /// spec 檔（docs/superpowers/specs/2026-08-10-chatgpt-web-provider-design.md）
    /// 本身就寫著字面的 `[[tool_call:<name>#<id>]]`。這裡刻意**不逃逸／不改寫**：
    /// 沒有任何剖析器會把這段歷史文字吃回去重新剖析——Task 5 的剖析器只掃
    /// 模型輸出裡的 `<tool>` / `<tool_call`，對 `[[…]]` 完全惰性，騙不到；
    /// 加逃逸只會讓模型多看到一堆雜訊，沒有實際的安全效益。
    #[test]
    fn delimiters_inside_content_pass_through_verbatim() {
        let literal = "spec 裡寫著 [[tool_call:Read#call_1]] ... [[/tool_call]]";
        let out = flatten_history(
            "",
            &[FlatTurn::ToolResult { id: "call_1".into(), content: literal.into() }],
        );
        assert!(out.contains(literal), "應原樣通過、不逃逸，實際：{out}");
    }

    /// 網頁版送的是「到目前為止的完整文字」，不是增量。直接轉發會讓
    /// 使用者看到重複累加的內容，必須自己算差分。
    #[test]
    fn snapshot_frames_become_incremental_deltas() {
        let mut p = SseParser::default();
        let a = p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["你好"]}}}"#);
        let b = p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["你好，世界"]}}}"#);
        assert_eq!(a, vec![SseOut::Text("你好".into())]);
        assert_eq!(b, vec![SseOut::Text("，世界".into())], "只能回新增的部分");
    }

    #[test]
    fn done_marker_ends_the_stream() {
        let mut p = SseParser::default();
        assert_eq!(p.feed_line("data: [DONE]"), vec![SseOut::Done]);
    }

    #[test]
    fn non_data_and_blank_lines_are_ignored() {
        let mut p = SseParser::default();
        assert!(p.feed_line("").is_empty());
        assert!(p.feed_line("event: delta").is_empty());
        assert!(p.feed_line(": keep-alive").is_empty());
    }

    /// 上游偶爾夾雜非 JSON 或缺欄位的 frame，不能因此中斷整條串流。
    #[test]
    fn malformed_frames_are_skipped_not_fatal() {
        let mut p = SseParser::default();
        assert!(p.feed_line("data: {不是 JSON").is_empty());
        assert!(p.feed_line(r#"data: {"message":null}"#).is_empty());
        assert_eq!(
            p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["還活著"]}}}"#),
            vec![SseOut::Text("還活著".into())],
        );
    }

    /// 上游換一則新訊息時，這個 frame 不是前一個的延續。按 byte 位移切字串
    /// 會讓切點落在多位元組字元中間而 panic——中文內容下這不是理論風險。
    #[test]
    fn non_continuation_frame_does_not_panic_on_multibyte() {
        let mut p = SseParser::default();
        assert_eq!(
            p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["ok"]}}}"#),
            vec![SseOut::Text("ok".into())],
        );
        // "ok" 是 2 bytes，"你好世界" 的 byte 2 在 '你' 的中間。
        assert_eq!(
            p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["你好世界"]}}}"#),
            vec![SseOut::Text("你好世界".into())],
            "換訊息時要整段送出，不是 panic 也不是丟掉",
        );
    }

    /// 新訊息比舊的短時，用長度比較會把它整段靜默吃掉。
    #[test]
    fn shorter_new_message_is_not_swallowed() {
        let mut p = SseParser::default();
        p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["很長的第一則回答"]}}}"#);
        assert_eq!(
            p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["短"]}}}"#),
            vec![SseOut::Text("短".into())],
        );
    }

    /// 上游重送同一份快照時不該重複輸出。
    #[test]
    fn identical_resend_emits_nothing() {
        let mut p = SseParser::default();
        let frame = r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["你好"]}}}"#;
        assert_eq!(p.feed_line(frame), vec![SseOut::Text("你好".into())]);
        assert!(p.feed_line(frame).is_empty(), "重送不該再輸出一次");
    }

    // ── 以下 fixture 是從真實串流錄下來的（`/backend-api/conversation`），
    //    不是手寫的簡化 JSON。手寫 fixture 會把「我以為的上游長相」固化成
    //    測試——這幾個 bug 全都是因此溜過先前 8 個綠燈測試的。

    /// **上游會把我們自己送出去的訊息回放一遍。**
    ///
    /// 實測：`/ai` 收到的「回答」是我們自己的 system prompt 原樣回傳，表現成
    /// 「AI 回傳格式錯誤」。真實 frame 的 `author.role` 是 `"user"`。
    #[test]
    fn user_echo_frame_is_not_treated_as_the_answer() {
        let mut p = SseParser::default();
        let echo = r#"data: {"message":{"id":"805ccd7d","author":{"role":"user","name":null,"metadata":{}},"content":{"content_type":"text","parts":["You are an AI command generator"]},"status":"finished_successfully"},"conversation_id":"6a79dfd3"}"#;
        assert!(
            p.feed_line(echo).is_empty(),
            "使用者訊息的回放不是回答，不可當成內容送出"
        );
        // 真正的 assistant frame 仍要正常收下，而且不受剛才那則影響。
        let real = r#"data: {"message":{"id":"aaa","author":{"role":"assistant"},"content":{"content_type":"text","parts":["好的"]}},"conversation_id":"x"}"#;
        assert_eq!(p.feed_line(real), vec![SseOut::Text("好的".into())]);
    }

    /// 串流開頭會夾幾則 `role:"system"` 的空訊息（`parts:[""]`）。
    #[test]
    fn system_frames_are_skipped() {
        let mut p = SseParser::default();
        let sys = r#"data: {"message":{"id":"ce85b27b","author":{"role":"system","name":null,"metadata":{}},"content":{"content_type":"text","parts":[""]},"status":"finished_successfully"}}"#;
        assert!(p.feed_line(sys).is_empty());
    }

    /// `{"type":"input_message","input_message":{…}}` 是另一種回放封套，
    /// 沒有 `message` 鍵。
    #[test]
    fn input_message_envelope_is_skipped() {
        let mut p = SseParser::default();
        let f = r#"data: {"type":"input_message","input_message":{"id":"805ccd7d","author":{"role":"user"},"content":{"content_type":"text","parts":["整段 prompt"]}}}"#;
        assert!(p.feed_line(f).is_empty());
    }

    /// 用量上限這類錯誤是 **HTTP 200 + SSE body 裡的 error 欄位**。
    ///
    /// 不辨識的話它會被當成「沒有內容的 frame」略過，使用者最後看到的是一個
    /// 跟真正原因無關的下游錯誤——實測時「你已達到上限」被表現成
    /// 「AI 回傳格式錯誤（expected value at line 4 column 21）」。
    #[test]
    fn in_stream_error_frame_is_surfaced() {
        let mut p = SseParser::default();
        let f = r#"data: {"message":null,"conversation_id":"6a79dfd3","error":"你已達到上限。請稍後再試一次。","error_code":"usage_limit"}"#;
        let out = p.feed_line(f);
        match out.first() {
            Some(SseOut::Error { message, code }) => {
                assert!(message.contains("上限"), "實際：{message}");
                assert_eq!(code, "usage_limit");
            }
            other => panic!("串流內的錯誤要被辨識出來，實際：{other:?}"),
        }
    }

    /// `resume_conversation_token` 這類沒有 `message` 的控制 frame。
    #[test]
    fn control_frames_without_message_are_skipped() {
        let mut p = SseParser::default();
        let f = r#"data: {"type":"resume_conversation_token","kind":"topic","token":"eyJhbGciOi"}"#;
        assert!(p.feed_line(f).is_empty());
    }

    /// **一次真實請求的完整 frame 序列**（2026-08-10 錄下）。
    ///
    /// 這條測試存在的理由是：先前所有 SSE 測試都是一次餵一種 frame，而真實
    /// 串流是這些 frame 交織成的一長串。兩個實測抓到的 bug（回放自己的 prompt、
    /// 串流內錯誤）都只在「照真實順序走一遍」時才顯現。
    ///
    /// 實測觀察到的重點：
    /// - 一次請求裡出現**三個不同的 assistant message id**。前兩個沒有字串
    ///   `parts[0]`（略過，且不可碰到差分狀態），第三個才是答案。
    /// - 答案訊息是**逐步累積**的（0 → 12 → 32 → 77 字元），差分要正確。
    /// - 答案訊息一旦開始，後續 frame 全屬於它——**沒有觀察到交錯**。
    #[test]
    fn real_captured_frame_sequence_yields_only_incremental_answer() {
        let mut p = SseParser::default();
        let mut got = String::new();
        let feed = |p: &mut SseParser, got: &mut String, line: &str| {
            for out in p.feed_line(line) {
                if let SseOut::Text(d) = out {
                    got.push_str(&d);
                }
            }
        };

        // 控制 frame（沒有 message 鍵）。
        for f in [
            r#"data: {"type":"resume_conversation_token","kind":"topic","token":"eyJ"}"#,
            r#"data: {"type":"message_marker","marker":"user_visible_token","event":"first"}"#,
            r#"data: {"type":"title_generation","title":"專案"}"#,
        ] {
            feed(&mut p, &mut got, f);
        }
        // 兩則 role:"system" 的空訊息。
        for id in ["ed2b42f9", "dea9978c"] {
            feed(&mut p, &mut got, &format!(
                r#"data: {{"message":{{"id":"{id}","author":{{"role":"system"}},"content":{{"content_type":"text","parts":[""]}},"status":"finished_successfully"}}}}"#
            ));
        }
        // 我們自己的 prompt 被回放（實測 2258 字元）。
        feed(&mut p, &mut got, r#"data: {"message":{"id":"507b2e02","author":{"role":"user"},"content":{"content_type":"text","parts":["You are an AI command generator"]},"status":"finished_successfully"}}"#);
        feed(&mut p, &mut got, r#"data: {"type":"input_message","input_message":{"author":{"role":"user"},"content":{"parts":["同一份 prompt 再回放一次"]}}}"#);

        // 兩則 assistant 訊息，但 parts[0] 不是字串（實測 parts_len=None）。
        // 這兩則**不可以**碰到 current_id / emitted——否則下面的答案會被當成
        // 「換了訊息」而整段重送。
        for id in ["7a02b6d6", "a8bcb6d8"] {
            feed(&mut p, &mut got, &format!(
                r#"data: {{"message":{{"id":"{id}","author":{{"role":"assistant"}},"content":{{"content_type":"thoughts","thoughts":[{{"summary":"想一下"}}]}},"status":"finished_successfully"}}}}"#
            ));
        }

        // 答案訊息：逐步累積 0 → 12 → 32 → 77。
        for text in [
            "",
            "{\"explanation\"",
            "{\"explanation\":\"列出最近修改的檔案\"",
            "{\"explanation\":\"列出最近修改的檔案\",\"command\":\"ls -lt | head -5\",\"risk_level\":\"safe\"}",
        ] {
            feed(&mut p, &mut got, &format!(
                r#"data: {{"message":{{"id":"3e875844","author":{{"role":"assistant"}},"content":{{"content_type":"text","parts":[{}]}},"status":"in_progress"}}}}"#,
                serde_json::Value::String(text.to_string())
            ));
        }

        assert_eq!(
            got,
            "{\"explanation\":\"列出最近修改的檔案\",\"command\":\"ls -lt | head -5\",\"risk_level\":\"safe\"}",
            "收到的應該恰好是答案本身：不含我們自己的 prompt、不含重複"
        );
    }

    /// 實測（2026-08-10）確認 chunk 真的會切在行中間：
    ///
    /// ```text
    /// [len=4220 ends_nl=false] tail="…,\"conversation_id\":\"6a79"
    /// [len=36   ends_nl=true ] tail="e841-de64-83ee-a31b-fd9e6eaafd61\"}\n\n"
    /// ```
    ///
    /// `conversation_id` 被從中間切開，下一個 36 bytes 的 chunk 才補完。沒有
    /// 行緩衝的話那個 frame 兩半都無法剖析、會被靜默丟掉——而若被切開的是
    /// **最後一個內容 frame**，回答結尾就永久少一截。
    #[test]
    fn real_chunk_boundary_falls_mid_line() {
        let mut p = SseParser::default();
        let head = r#"data: {"message":{"id":"m1","author":{"role":"assistant"},"content":{"parts":["答案"]}},"conversation_id":"6a79"#;
        assert!(p.feed_str(head).is_empty(), "半行不可提早輸出");
        let tail = "e841-de64\"}\n";
        assert_eq!(p.feed_str(tail), vec![SseOut::Text("答案".into())]);
    }

    /// 規格明確要求文字取自 `message.content.parts[0]`。多個 parts 時如何合併
    /// 明確不在本任務範圍內，這裡只固化「取索引 0」這個決定。用兩個以上、
    /// 彼此不同的元素，才能區分出取值走的是第一個還是別的位置——單元素陣列上
    /// `first()`／`last()`／任何索引結果都相同，測不出取值路徑選對沒有。
    #[test]
    fn text_comes_from_the_first_part() {
        let mut p = SseParser::default();
        assert_eq!(
            p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["第一","第二","第三"]}}}"#),
            vec![SseOut::Text("第一".into())],
        );
    }

    /// Bug 1：moderation、role:"system" 的空訊息、只帶 conversation_id 的
    /// frame 在串流裡是常態。這種沒有可讀文字的 frame 若被壓成空字串（舊寫法
    /// `.unwrap_or("")`），會讓 `emitted` 被重設，下一個真正的快照就整段重送。
    /// 這個測試刻意放在串流**中段**——放在開頭時 `emitted` 本來就是空的，
    /// 重設沒有可觀察後果，測不出問題。
    #[test]
    fn mid_stream_frame_without_text_does_not_reset_state() {
        let mut p = SseParser::default();
        assert_eq!(
            p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["你好"]}}}"#),
            vec![SseOut::Text("你好".into())],
        );
        assert!(p.feed_line(r#"data: {"message":null}"#).is_empty());
        assert_eq!(
            p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["你好，世界"]}}}"#),
            vec![SseOut::Text("，世界".into())],
            "中段沒有文字的 frame 不該把 emitted 清空，否則這裡會整段重送",
        );
    }

    /// 同上，但用 `parts:[""]`（欄位存在、內容是空字串）而不是整個 message
    /// 缺欄位。兩種情況都要保留差分狀態，不能因此重送。
    #[test]
    fn frame_with_empty_parts_mid_stream_does_not_reset_state() {
        let mut p = SseParser::default();
        assert_eq!(
            p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["你好"]}}}"#),
            vec![SseOut::Text("你好".into())],
        );
        assert!(p
            .feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":[""]}}}"#)
            .is_empty());
        assert_eq!(
            p.feed_line(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["你好，世界"]}}}"#),
            vec![SseOut::Text("，世界".into())],
            "中段的空 parts[0] 不該把 emitted 清空，否則這裡會整段重送",
        );
    }

    /// Bug 2：HTTP chunk 不保證切在行邊界上。用 `feed_str` 把一行拆成兩段
    /// 餵，斷言殘缺的前半段不輸出，後半段接上後才輸出完整內容。
    #[test]
    fn chunks_split_mid_line_are_reassembled() {
        let mut p = SseParser::default();
        let first = p.feed_str(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["完整答案在這"#);
        assert!(first.is_empty(), "殘缺的前半段不該提早輸出");
        let second = p.feed_str("裡\"]}}}\n");
        assert_eq!(second, vec![SseOut::Text("完整答案在這裡".into())]);
    }

    /// 沒有換行結尾的完整 JSON 應該被留在緩衝裡等下一個 chunk，不能因為
    /// JSON 本身可解析就提早當成一行處理。
    #[test]
    fn trailing_partial_line_is_held_not_emitted() {
        let mut p = SseParser::default();
        let out = p.feed_str(r#"data: {"message":{"author":{"role":"assistant"},"content":{"parts":["還沒送出"]}}}"#);
        assert!(out.is_empty(), "沒有換行結尾時應該還在等，不能提早輸出");
    }

    /// `message.id` 換了代表換了一則訊息（例如思考區塊結束、答案開始）。這個
    /// 情境是真的、不是為測試而測試：思考區塊與答案是兩則不同的 `message`，
    /// 它們的開頭恰好重疊並不罕見（例如思考寫「我來看看」、答案也以「我來
    /// 看看」開頭）。
    ///
    /// 這裡刻意讓第二則的內容是第一則 `emitted` 的**字串延續**——這是唯一能
    /// 區分「靠 `strip_prefix` 回 `None`（內容對不上）」和「靠 `id` 重設」的
    /// 情境：若只看內容是否為前綴延續，第二則會被誤判成第一則的延續，
    /// 使用者只會看到後半段「，世界」而不是完整的第二則訊息。沒有這個測試，
    /// `current_id`／`self.emitted.clear()` 整套機制形同沒有理由的複雜度：
    /// 拿掉它其餘測試依然全綠。
    #[test]
    fn new_message_id_restarts_the_diff_even_when_content_overlaps() {
        let mut p = SseParser::default();
        assert_eq!(
            p.feed_line(r#"data: {"message":{"id":"msg_1","author":{"role":"assistant"},"content":{"parts":["你好"]}}}"#),
            vec![SseOut::Text("你好".into())],
        );
        assert_eq!(
            p.feed_line(r#"data: {"message":{"id":"msg_2","author":{"role":"assistant"},"content":{"parts":["你好，世界"]}}}"#),
            vec![SseOut::Text("你好，世界".into())],
            "新 id 的訊息即使文字上恰好是舊 emitted 的延續，也該整段重送而非只送差異",
        );
    }

    /// `id` 不變時仍照常算差分——確認 id 檢查不會誤傷正常的累積快照。
    #[test]
    fn same_message_id_keeps_diffing() {
        let mut p = SseParser::default();
        assert_eq!(
            p.feed_line(r#"data: {"message":{"id":"msg_1","author":{"role":"assistant"},"content":{"parts":["你好"]}}}"#),
            vec![SseOut::Text("你好".into())],
        );
        assert_eq!(
            p.feed_line(r#"data: {"message":{"id":"msg_1","author":{"role":"assistant"},"content":{"parts":["你好，世界"]}}}"#),
            vec![SseOut::Text("，世界".into())],
        );
    }

    /// 部分環境可能送 `\r\n` 結尾。這條鎖的是**端到端的 CRLF 行為**——
    /// `feed_str` 餵 `\r\n` 結尾的一行要能正常解析——而不是單獨鎖
    /// `trim_end_matches(['\r', '\n'])` 那一行本身：`feed_line` 裡的
    /// `payload.trim()` 也會參與（`\r` 屬於 Unicode 空白字元，會被
    /// `trim()` 一併吃掉），所以單獨把 `trim_end_matches` 改壞成只留
    /// `'\n'` 並不會讓這條測試變紅。保留 `trim_end_matches(['\r', '\n'])`
    /// 是因為「一行不該包含它自己的行終止符」是 `feed_str` 這一層自己的
    /// 契約，不該依賴下游 `.trim()` 的副作用來成立。
    #[test]
    fn crlf_line_endings_are_handled() {
        let mut p = SseParser::default();
        let out = p.feed_str("data: {\"message\":{\"author\":{\"role\":\"assistant\"},\"content\":{\"parts\":[\"你好\"]}}}\r\n");
        assert_eq!(out, vec![SseOut::Text("你好".into())]);
    }
}
