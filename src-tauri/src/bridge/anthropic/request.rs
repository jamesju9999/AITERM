//! 傳入的 Anthropic Messages API 請求。
//!
//! 只解析我們真的會用到的欄位；未知的 content block 型別直接丟棄而不是報錯
//! ——Claude Code 版本更新時新增區塊型別是常態，硬失敗會讓橋接整個不能用。

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesRequest {
    pub model: String,
    /// 字串或 block 陣列，用 [`system_text`] 取出純文字。
    #[serde(default)]
    pub system: Option<Value>,
    pub messages: Vec<InboundMessage>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub tools: Option<Vec<ToolDef>>,
    #[serde(default)]
    pub tool_choice: Option<Value>,
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InboundMessage {
    pub role: String,
    /// 字串或 block 陣列，用 [`parse_content`] 正規化。
    pub content: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThinkingConfig {
    #[serde(default)]
    pub budget_tokens: Option<u32>,
}

/// 正規化後的 content block。`cache_control` 刻意不保留 —— OpenAI 家族不支援，
/// Anthropic 家族走 passthrough 不經過這裡。
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    Text(String),
    Image { media_type: String, data: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult {
        tool_use_id: String,
        content: Vec<ContentBlock>,
        is_error: bool,
    },
    Thinking(String),
}

pub fn parse_content(v: &Value) -> Vec<ContentBlock> {
    match v {
        Value::String(s) => vec![ContentBlock::Text(s.clone())],
        Value::Array(items) => items.iter().filter_map(parse_block).collect(),
        _ => Vec::new(),
    }
}

fn parse_block(v: &Value) -> Option<ContentBlock> {
    let ty = v.get("type")?.as_str()?;
    match ty {
        "text" => Some(ContentBlock::Text(
            v.get("text")?.as_str().unwrap_or_default().to_string(),
        )),
        "thinking" => Some(ContentBlock::Thinking(
            v.get("thinking")?.as_str().unwrap_or_default().to_string(),
        )),
        "image" => {
            let source = v.get("source")?;
            Some(ContentBlock::Image {
                media_type: source.get("media_type")?.as_str()?.to_string(),
                data: source.get("data")?.as_str()?.to_string(),
            })
        }
        "tool_use" => Some(ContentBlock::ToolUse {
            id: v.get("id")?.as_str()?.to_string(),
            name: v.get("name")?.as_str()?.to_string(),
            input: v.get("input").cloned().unwrap_or(Value::Object(Default::default())),
        }),
        "tool_result" => Some(ContentBlock::ToolResult {
            tool_use_id: v.get("tool_use_id")?.as_str()?.to_string(),
            content: v.get("content").map(parse_content).unwrap_or_default(),
            is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        }),
        _ => None,
    }
}

/// 把 `system` 欄位攤平成一段純文字。
pub fn system_text(v: Option<&Value>) -> String {
    let Some(v) = v else { return String::new() };
    match v {
        Value::String(s) => s.clone(),
        Value::Array(_) => parse_content(v)
            .into_iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_string_content() {
        let blocks = parse_content(&json!("hello"));
        assert_eq!(blocks, vec![ContentBlock::Text("hello".into())]);
    }

    #[test]
    fn parses_text_and_image_blocks() {
        let v = json!([
            {"type": "text", "text": "看這張"},
            {"type": "image", "source": {
                "type": "base64", "media_type": "image/png", "data": "AAAA"
            }}
        ]);
        assert_eq!(
            parse_content(&v),
            vec![
                ContentBlock::Text("看這張".into()),
                ContentBlock::Image { media_type: "image/png".into(), data: "AAAA".into() },
            ]
        );
    }

    #[test]
    fn parses_tool_use_block() {
        let v = json!([{
            "type": "tool_use", "id": "toolu_1", "name": "Read",
            "input": {"file_path": "/tmp/a"}
        }]);
        assert_eq!(
            parse_content(&v),
            vec![ContentBlock::ToolUse {
                id: "toolu_1".into(),
                name: "Read".into(),
                input: json!({"file_path": "/tmp/a"}),
            }]
        );
    }

    #[test]
    fn parses_tool_result_with_nested_blocks() {
        let v = json!([{
            "type": "tool_result", "tool_use_id": "toolu_1",
            "content": [{"type": "text", "text": "檔案內容"}]
        }]);
        assert_eq!(
            parse_content(&v),
            vec![ContentBlock::ToolResult {
                tool_use_id: "toolu_1".into(),
                content: vec![ContentBlock::Text("檔案內容".into())],
                is_error: false,
            }]
        );
    }

    #[test]
    fn tool_result_content_may_be_a_bare_string() {
        let v = json!([{
            "type": "tool_result", "tool_use_id": "t1", "content": "ok", "is_error": true
        }]);
        assert_eq!(
            parse_content(&v),
            vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: vec![ContentBlock::Text("ok".into())],
                is_error: true,
            }]
        );
    }

    #[test]
    fn unknown_block_types_are_dropped() {
        let v = json!([{"type": "future_thing", "x": 1}, {"type": "text", "text": "a"}]);
        assert_eq!(parse_content(&v), vec![ContentBlock::Text("a".into())]);
    }

    #[test]
    fn system_accepts_string_or_block_array() {
        assert_eq!(system_text(Some(&json!("你是助手"))), "你是助手");
        let arr = json!([{"type": "text", "text": "一"}, {"type": "text", "text": "二"}]);
        assert_eq!(system_text(Some(&arr)), "一\n\n二");
        assert_eq!(system_text(None), "");
    }

    #[test]
    fn deserializes_a_minimal_request() {
        let req: MessagesRequest = serde_json::from_value(json!({
            "model": "aiterm:sonnet",
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap();
        assert_eq!(req.model, "aiterm:sonnet");
        assert_eq!(req.max_tokens, Some(1024));
        assert_eq!(req.stream, Some(true));
        assert!(req.tools.is_none());
    }
}
