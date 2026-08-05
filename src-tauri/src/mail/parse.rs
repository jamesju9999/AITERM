// src-tauri/src/mail/parse.rs
use mail_parser::MessageParser;

#[derive(Debug, Clone)]
pub struct ParsedMessage {
    pub sender: String,
    pub subject: String,
    pub date: Option<String>,
    pub body_text: String,
}

/// Parse a raw RFC822 byte slice into sender/subject/date/body. Returns
/// `None` only when mail-parser finds no headers at all (garbage input) —
/// see mail-parser's `MessageParser::parse` docs: "if no headers are found
/// None is returned".
pub fn parse_raw_message(raw: &[u8]) -> Option<ParsedMessage> {
    let message = MessageParser::default().parse(raw)?;

    let sender = message
        .from()
        .and_then(|addr| addr.first())
        .and_then(|a| a.address())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let subject = message
        .subject()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "(no subject)".to_string());

    let date = message.date().map(|d| d.to_rfc3339());

    let body_text = message
        .body_text(0)
        .or_else(|| message.body_html(0))
        .map(|s| s.to_string())
        .unwrap_or_default();

    Some(ParsedMessage { sender, subject, date, body_text })
}
