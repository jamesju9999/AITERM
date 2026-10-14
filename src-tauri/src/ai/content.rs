//! 把 `ChatMessage.content`（字串或 OpenAI 格式的內容陣列）解析成文字與圖片段落，
//! 讓各家供應商在組請求時轉成自己的原生格式。
//!
//! 前端附圖一律送 `{"type":"image_url","image_url":{"url":"data:<mime>;base64,<data>"}}`；
//! 只有 OpenAI 風格的端點吃得下，其他家要在這裡拆開再重組。

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    /// `data` 是 base64，不含 `data:…;base64,` 前綴。
    Image { media_type: String, data: String },
}

/// 字串 → 一個 Text；陣列 → 依序解析 text／image_url（data URI）；
/// 不是 data URI 的 image_url、不認得的段落 → 略過；null → 空。
/// 其他型別（數字、物件）→ 一個 Text（`to_string()`，維持舊行為的保底）。
pub fn parts(content: &Value) -> Vec<Part> {
    match content {
        Value::Null => Vec::new(),
        Value::String(s) => vec![Part::Text(s.clone())],
        Value::Array(items) => items.iter().filter_map(part_from_item).collect(),
        other => vec![Part::Text(other.to_string())],
    }
}

fn part_from_item(item: &Value) -> Option<Part> {
    match item.get("type").and_then(Value::as_str)? {
        "text" => Some(Part::Text(item.get("text")?.as_str()?.to_string())),
        "image_url" => {
            let url = item.get("image_url")?.get("url")?.as_str()?;
            parse_data_uri(url)
        }
        _ => None,
    }
}

/// `data:<mime>;base64,<data>` → Image；其他形式（http 網址、少了 `;base64,`、空 mime）→ None。
fn parse_data_uri(url: &str) -> Option<Part> {
    let rest = url.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    if media_type.is_empty() {
        return None;
    }
    Some(Part::Image { media_type: media_type.to_string(), data: data.to_string() })
}

pub fn has_image(content: &Value) -> bool {
    parts(content).iter().any(|p| matches!(p, Part::Image { .. }))
}

/// 只取文字段落、用 "\n\n" 接起來（給只吃純文字的地方）。
pub fn text_only(content: &Value) -> String {
    parts(content)
        .into_iter()
        .filter_map(|p| match p {
            Part::Text(t) => Some(t),
            Part::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn img(url: &str) -> Value {
        json!({"type": "image_url", "image_url": {"url": url}})
    }

    #[test]
    fn string_becomes_single_text() {
        assert_eq!(parts(&json!("hello")), vec![Part::Text("hello".into())]);
        assert_eq!(parts(&json!("")), vec![Part::Text(String::new())]);
    }

    #[test]
    fn null_is_empty() {
        assert_eq!(parts(&Value::Null), vec![]);
    }

    #[test]
    fn array_keeps_order_and_splits_data_uri() {
        let c = json!([
            {"type": "text", "text": "what colour?"},
            img("data:image/jpeg;base64,QUJD"),
            {"type": "text", "text": "and this"},
        ]);
        assert_eq!(
            parts(&c),
            vec![
                Part::Text("what colour?".into()),
                Part::Image { media_type: "image/jpeg".into(), data: "QUJD".into() },
                Part::Text("and this".into()),
            ]
        );
    }

    #[test]
    fn http_image_url_is_skipped() {
        let c = json!([{"type": "text", "text": "hi"}, img("https://example.com/a.png")]);
        assert_eq!(parts(&c), vec![Part::Text("hi".into())]);
    }

    #[test]
    fn malformed_data_uri_is_skipped() {
        let c = json!([
            {"type": "text", "text": "hi"},
            img("data:image/png,QUJD"),
            img("data:;base64,QUJD"),
        ]);
        assert_eq!(parts(&c), vec![Part::Text("hi".into())]);
    }

    #[test]
    fn unknown_type_is_skipped() {
        let c = json!([{"type": "audio", "data": "x"}, {"type": "text", "text": "hi"}]);
        assert_eq!(parts(&c), vec![Part::Text("hi".into())]);
    }

    #[test]
    fn other_values_fall_back_to_to_string() {
        assert_eq!(parts(&json!(42)), vec![Part::Text("42".into())]);
    }

    #[test]
    fn has_image_true_and_false() {
        assert!(has_image(&json!([{"type": "text", "text": "a"}, img("data:image/png;base64,QUJD")])));
        assert!(!has_image(&json!([{"type": "text", "text": "a"}])));
        assert!(!has_image(&json!("data:image/png;base64,QUJD")));
    }

    #[test]
    fn text_only_joins_text_parts() {
        let c = json!([
            {"type": "text", "text": "one"},
            img("data:image/png;base64,QUJD"),
            {"type": "text", "text": "two"},
        ]);
        assert_eq!(text_only(&c), "one\n\ntwo");
        assert_eq!(text_only(&json!("plain")), "plain");
    }
}
