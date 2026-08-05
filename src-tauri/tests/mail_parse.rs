// src-tauri/tests/mail_parse.rs
use aiterm_lib::mail::parse::parse_raw_message;

const SAMPLE_EML: &str = "From: Alice <alice@example.com>\r\n\
Subject: Test Subject\r\n\
Date: Mon, 1 Jan 2026 12:00:00 +0000\r\n\
Content-Type: text/plain\r\n\
\r\n\
Hello world.\r\n";

#[test]
fn parses_sender_subject_date_and_body() {
    let parsed = parse_raw_message(SAMPLE_EML.as_bytes()).expect("should parse");
    assert_eq!(parsed.sender, "alice@example.com");
    assert_eq!(parsed.subject, "Test Subject");
    assert!(parsed.date.is_some());
    assert!(parsed.body_text.contains("Hello world."));
}

#[test]
fn falls_back_to_html_body_when_no_plain_text_part() {
    let html_only = "From: Bob <bob@example.com>\r\n\
Subject: HTML only\r\n\
Content-Type: text/html\r\n\
\r\n\
<p>Hi there</p>\r\n";
    let parsed = parse_raw_message(html_only.as_bytes()).expect("should parse");
    assert!(parsed.body_text.contains("Hi there"));
}

#[test]
fn missing_subject_falls_back_to_placeholder() {
    let no_subject = "From: Carol <carol@example.com>\r\n\
\r\n\
Body only.\r\n";
    let parsed = parse_raw_message(no_subject.as_bytes()).expect("should parse");
    assert_eq!(parsed.subject, "(no subject)");
}

#[test]
fn empty_bytes_return_none() {
    assert!(parse_raw_message(b"").is_none());
}
