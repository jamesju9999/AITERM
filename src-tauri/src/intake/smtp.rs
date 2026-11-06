//! 寄回覆信。`mail/` 只有收信，這裡補上寄信；帳號與密碼沿用郵件帳號設定與 keyring。
use lettre::message::header::{InReplyTo, References};
use lettre::message::{Mailbox, MultiPart};
use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

pub struct Outgoing<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub orig_subject: &'a str,
    /// 原信的 Message-ID（不含角括號）；空字串則不帶串信標頭。
    pub orig_message_id: &'a str,
    pub body: &'a str,
    /// 回覆全部：原信的其他收件人與副本（已去掉自己、寄件人與重複）。
    pub also_to: &'a [String],
    pub cc: &'a [String],
}

fn key(a: &str) -> String {
    super::rules::normalize(a)
}

/// 算出回信的「其他收件人」與「副本」：原信 To／Cc 去掉寄件人、我們自己（收信帳號與別名）和重複；
/// To 優先於 Cc（同一人只出現一次）。
pub fn reply_all(sender: &str, to: &str, cc: &str, mine: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut seen: Vec<String> = std::iter::once(sender).chain(mine.iter().copied()).filter(|s| !s.trim().is_empty()).map(key).collect();
    let mut pick = |list: &str| -> Vec<String> {
        let mut out = vec![];
        for a in list.lines().map(str::trim).filter(|a| !a.is_empty()) {
            let k = key(a);
            if !seen.contains(&k) {
                seen.push(k);
                out.push(a.to_string());
            }
        }
        out
    };
    let t = pick(to);
    let c = pick(cc);
    (t, c)
}

pub fn reply_subject(orig: &str) -> String {
    let t = orig.trim();
    if t.len() >= 3 && t[..3].eq_ignore_ascii_case("re:") { t.to_string() } else { format!("Re: {t}") }
}

/// 一般信件（驗證碼等），不帶串信標頭、主旨原樣。
pub fn build_plain(from: &str, to: &str, subject: &str, body: &str) -> Result<Message, String> {
    let from: Mailbox = from.parse().map_err(|e| format!("寄件地址無效：{e}"))?;
    let to: Mailbox = to.parse().map_err(|e| format!("收件地址無效：{e}"))?;
    Message::builder().from(from).to(to).subject(subject).body(body.to_string()).map_err(|e| e.to_string())
}

pub fn build_reply(o: &Outgoing<'_>) -> Result<Message, String> {
    let from: Mailbox = o.from.parse().map_err(|e| format!("寄件地址無效：{e}"))?;
    let to: Mailbox = o.to.parse().map_err(|e| format!("收件地址無效：{e}"))?;
    let mut b = Message::builder().from(from).to(to).subject(reply_subject(o.orig_subject));
    // 其他收件人與副本：格式不對的地址略過，不讓一個壞地址擋住整封回信。
    for a in o.also_to.iter().filter_map(|a| a.parse::<Mailbox>().ok()) {
        b = b.to(a);
    }
    for a in o.cc.iter().filter_map(|a| a.parse::<Mailbox>().ok()) {
        b = b.cc(a);
    }
    if !o.orig_message_id.is_empty() {
        let id = format!("<{}>", o.orig_message_id.trim_matches(|c| c == '<' || c == '>'));
        b = b.header(InReplyTo::from(id.clone())).header(References::from(id));
    }
    // 回答多半是 Markdown：寄成「純文字＋HTML」兩種版本，支援 HTML 的信箱顯示排版，其餘看純文字。
    b.multipart(MultiPart::alternative_plain_html(o.body.to_string(), markdown_to_html(o.body))).map_err(|e| e.to_string())
}

fn safe_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://") || l.starts_with("mailto:")
}

/// Markdown → 信件用 HTML。內容來自 AI 與外部提問者，所以：原始 HTML 一律當文字跳脫、
/// 只放行 http(s)／mailto 連結、圖片不嵌入（只留替代文字，避免追蹤像素）。
pub fn markdown_to_html(md: &str) -> String {
    let events = Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS).filter_map(|e| match e {
        Event::Html(h) | Event::InlineHtml(h) => Some(Event::Text(h)),
        Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) => None,
        Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
            let dest_url = if safe_url(&dest_url) { dest_url } else { CowStr::Borrowed("") };
            Some(Event::Start(Tag::Link { link_type, dest_url, title, id }))
        }
        other => Some(other),
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    format!(
        "<div style=\"font-family:-apple-system,'Segoe UI',Roboto,'PingFang TC','Microsoft JhengHei',sans-serif;font-size:14px;line-height:1.6;color:#222\">{out}</div>"
    )
}

/// 465 走隱含 TLS，其餘（587 等）走 STARTTLS。
pub async fn send(host: &str, port: u16, username: &str, password: &str, msg: Message) -> Result<(), String> {
    let builder = if port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(host)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
    }
    .map_err(|e| e.to_string())?;
    let t = builder.port(port).credentials(Credentials::new(username.to_string(), password.to_string())).build();
    t.send(msg).await.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_gets_single_re_prefix() {
        assert_eq!(reply_subject("[AITerm] bug"), "Re: [AITerm] bug");
        assert_eq!(reply_subject("RE: x"), "RE: x");
        assert_eq!(reply_subject("  re: x "), "re: x");
    }

    #[test]
    fn reply_threads_onto_original() {
        let m = build_reply(&Outgoing { from: "me@home.com", to: "Bob <bob@corp.com>", orig_subject: "q", orig_message_id: "<abc@corp>", body: "答案", also_to: &[], cc: &[] }).unwrap();
        let raw = String::from_utf8(m.formatted()).unwrap();
        assert!(raw.contains("In-Reply-To: <abc@corp>"), "{raw}");
        assert!(raw.contains("References: <abc@corp>"), "{raw}");
        assert!(raw.contains("Subject: Re: q"), "{raw}");
        assert!(raw.contains("bob@corp.com"));
    }

    #[test]
    fn reply_all_drops_sender_self_and_duplicates_and_keeps_to_before_cc() {
        let (to, cc) = reply_all(
            "Bob <bob@corp.com>",
            "me+aiterm@gmail.com\nAlice@Corp.com\nbob@corp.com",
            "carol@corp.com\nalice@corp.com\n\nme@gmail.com",
            &["me@gmail.com", "me+aiterm@gmail.com"],
        );
        assert_eq!(to, vec!["Alice@Corp.com"]);
        assert_eq!(cc, vec!["carol@corp.com"], "Alice 已在收件人，不重複放副本；自己不寄給自己");
    }

    #[test]
    fn reply_carries_all_recipients_and_cc_and_skips_malformed_ones() {
        let (also, cc) = (vec!["alice@corp.com".to_string(), "not an address".to_string()], vec!["carol@corp.com".to_string()]);
        let m = build_reply(&Outgoing { from: "me@home.com", to: "bob@corp.com", orig_subject: "q", orig_message_id: "", body: "答", also_to: &also, cc: &cc }).unwrap();
        let raw = String::from_utf8(m.formatted()).unwrap();
        assert!(raw.contains("bob@corp.com") && raw.contains("alice@corp.com"), "{raw}");
        assert!(raw.contains("Cc: carol@corp.com"), "{raw}");
        let env = m.envelope();
        let rcpt: Vec<String> = env.to().iter().map(|a| a.to_string()).collect();
        assert_eq!(rcpt.len(), 3, "信封收件人要包含 To 與 Cc：{rcpt:?}");
    }

    #[test]
    fn plain_mail_keeps_subject_and_has_no_threading_headers() {
        let m = build_plain("me@home.com", "bob@corp.com", "驗證碼", "123456").unwrap();
        let raw = String::from_utf8(m.formatted()).unwrap();
        assert!(raw.contains("bob@corp.com") && !raw.contains("In-Reply-To"), "{raw}");
        assert!(build_plain("x", "bob@corp.com", "s", "b").is_err());
    }

    #[test]
    fn markdown_becomes_html_and_unsafe_parts_are_neutralised() {
        let h = markdown_to_html("**左欄**\n\n- 項目一\n- `code`\n\n[好](https://a.com) [壞](javascript:alert(1)) <script>x</script> ![追蹤](http://t.com/p.png)");
        assert!(h.contains("<strong>左欄</strong>") && h.contains("<li>項目一</li>") && h.contains("<code>code</code>"), "{h}");
        assert!(h.contains("href=\"https://a.com\""), "{h}");
        assert!(!h.contains("javascript:"), "{h}");
        assert!(!h.contains("<script>") && h.contains("&lt;script&gt;"), "{h}");
        assert!(!h.contains("<img"), "{h}");
    }

    #[test]
    fn reply_carries_both_a_plain_and_an_html_part() {
        let m = build_reply(&Outgoing { from: "me@home.com", to: "bob@corp.com", orig_subject: "q", orig_message_id: "", body: "**粗體**", also_to: &[], cc: &[] }).unwrap();
        let raw = String::from_utf8(m.formatted()).unwrap();
        assert!(raw.contains("multipart/alternative"), "{raw}");
        assert!(raw.contains("text/plain") && raw.contains("text/html"), "{raw}");
    }

    #[test]
    fn invalid_address_is_an_error_not_a_panic() {
        assert!(build_reply(&Outgoing { from: "nope", to: "a@b.c", orig_subject: "", orig_message_id: "", body: "", also_to: &[], cc: &[] }).is_err());
    }
}
