//! 模型把工具呼叫寫成一般文字時的過濾。
//!
//! 有兩種情況會發生：
//!   1. 供應商不支援原生工具呼叫，我們把工具描述注入系統提示，模型照約定吐出
//!      `<tool_call>{…}</tool_call>`（見 `commands::ai::build_tool_prompt_injection`）
//!   2. 本地模型即使有原生工具呼叫，仍偶爾改用 `<function=…>` 這種方言
//!
//! 兩種都是**指令**，不是講給使用者聽的話。但串流是在解析之前就把文字送到畫面
//! 的，不擋的話整條指令會印在答案裡（實測回報過）。

/// 工具呼叫寫成文字時的兩種開頭。
const TOOL_MARKERS: [&str; 2] = ["<tool_call>", "<function="];

/// 這段文字裡可以安全顯示給使用者的前綴長度（byte offset）。
///
/// 切在第一個標記之前；也會扣掉結尾那截「只收到一半的標記開頭」——串流時標記是
/// 一個 delta 一個 delta 拼出來的，不擋的話畫面會先閃出 `<func` 再被蓋掉。
pub fn visible_prefix_len(text: &str) -> usize {
    let mut cut = text.len();
    for m in TOOL_MARKERS {
        if let Some(i) = text.find(m) {
            cut = cut.min(i);
        }
    }
    if cut == text.len() {
        for m in TOOL_MARKERS {
            for len in (1..m.len()).rev() {
                if text.ends_with(&m[..len]) {
                    cut = cut.min(text.len() - len);
                    break;
                }
            }
        }
    }
    cut
}

/// 串流時逐段決定「這次新增的內容裡，有多少可以送到畫面」。
///
/// `seen` 是目前累積到的全文，`emitted` 是已經送出去的長度。回傳新的可送片段與
/// 更新後的 offset；沒有東西可送時回 `None`。
pub fn next_visible_delta(seen: &str, emitted: usize) -> Option<(String, usize)> {
    let safe = visible_prefix_len(seen);
    if safe > emitted {
        Some((seen[emitted..safe].to_string(), safe))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_at_tool_call_tag() {
        let text = "我來看看這個檔案：<tool_call>{\"name\":\"read_file\"}</tool_call>";
        assert_eq!(&text[..visible_prefix_len(text)], "我來看看這個檔案：");
    }

    #[test]
    fn stops_at_function_attribute_tag() {
        let text = "先讀檔：<function=read_file> <parameter=path> a.java </parameter>";
        assert_eq!(&text[..visible_prefix_len(text)], "先讀檔：");
    }

    // 串流時標記是一個 delta 一個 delta 拼出來的，中間會經過 "<"、"<fun"…
    #[test]
    fn holds_back_partial_marker() {
        assert_eq!(&"先讀檔：<func"[..visible_prefix_len("先讀檔：<func")], "先讀檔：");
        assert_eq!(&"先讀檔：<"[..visible_prefix_len("先讀檔：<")], "先讀檔：");
    }

    #[test]
    fn keeps_plain_text_untouched() {
        let text = "這個專案採用 DDD 分層，a < b 的比較在 domain 層。";
        assert_eq!(visible_prefix_len(text), text.len());
    }

    #[test]
    fn cuts_at_the_first_marker() {
        let text = "說明<function=a>中間<tool_call>後面";
        assert_eq!(&text[..visible_prefix_len(text)], "說明");
    }

    #[test]
    fn next_delta_emits_only_the_newly_safe_part() {
        let (delta, offset) = next_visible_delta("我來看", 0).unwrap();
        assert_eq!(delta, "我來看");
        let (delta2, offset2) = next_visible_delta("我來看一下", offset).unwrap();
        assert_eq!(delta2, "一下");
        // 標記出現之後不再有東西可送。
        assert!(next_visible_delta("我來看一下<tool_call>{}", offset2).is_none());
    }
}
