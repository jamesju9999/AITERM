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
    Done,
}

/// ChatGPT 網頁版 SSE 的逐行解析器。
///
/// 每個 data frame 是「到目前為止的完整文字」快照而非增量，所以要保留
/// 已送出的內容自己算差分。
#[derive(Default)]
pub struct SseParser {
    emitted: String,
}

impl SseParser {
    pub fn feed_line(&mut self, line: &str) -> Vec<SseOut> {
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
        let full = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())
            .and_then(|s| s.as_str())
            .unwrap_or("");
        // 用 strip_prefix 而非 `full[self.emitted.len()..]`：後者按 byte 切字串，
        // 當這個 frame 不是前一個的延續時（上游換了一則訊息），切點會落在多位元組
        // 字元的中間而 panic。實測：emitted="ok"、full="你好世界" 會炸在
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
            // 不是延續——上游換了一則訊息。重新起算並把整段當新內容送出。
            // 不能沿用「比較長度」的判斷：新訊息比舊的短時會被整段靜默吃掉。
            None => {
                self.emitted = full.to_string();
                if full.is_empty() {
                    Vec::new()
                } else {
                    vec![SseOut::Text(full.to_string())]
                }
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
        let a = p.feed_line(r#"data: {"message":{"content":{"parts":["你好"]}}}"#);
        let b = p.feed_line(r#"data: {"message":{"content":{"parts":["你好，世界"]}}}"#);
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
            p.feed_line(r#"data: {"message":{"content":{"parts":["還活著"]}}}"#),
            vec![SseOut::Text("還活著".into())],
        );
    }

    /// 上游換一則新訊息時，這個 frame 不是前一個的延續。按 byte 位移切字串
    /// 會讓切點落在多位元組字元中間而 panic——中文內容下這不是理論風險。
    #[test]
    fn non_continuation_frame_does_not_panic_on_multibyte() {
        let mut p = SseParser::default();
        assert_eq!(
            p.feed_line(r#"data: {"message":{"content":{"parts":["ok"]}}}"#),
            vec![SseOut::Text("ok".into())],
        );
        // "ok" 是 2 bytes，"你好世界" 的 byte 2 在 '你' 的中間。
        assert_eq!(
            p.feed_line(r#"data: {"message":{"content":{"parts":["你好世界"]}}}"#),
            vec![SseOut::Text("你好世界".into())],
            "換訊息時要整段送出，不是 panic 也不是丟掉",
        );
    }

    /// 新訊息比舊的短時，用長度比較會把它整段靜默吃掉。
    #[test]
    fn shorter_new_message_is_not_swallowed() {
        let mut p = SseParser::default();
        p.feed_line(r#"data: {"message":{"content":{"parts":["很長的第一則回答"]}}}"#);
        assert_eq!(
            p.feed_line(r#"data: {"message":{"content":{"parts":["短"]}}}"#),
            vec![SseOut::Text("短".into())],
        );
    }

    /// 上游重送同一份快照時不該重複輸出。
    #[test]
    fn identical_resend_emits_nothing() {
        let mut p = SseParser::default();
        let frame = r#"data: {"message":{"content":{"parts":["你好"]}}}"#;
        assert_eq!(p.feed_line(frame), vec![SseOut::Text("你好".into())]);
        assert!(p.feed_line(frame).is_empty(), "重送不該再輸出一次");
    }
}
