//! 寄回覆信。`mail/` 只有收信，這裡補上寄信；帳號與密碼沿用郵件帳號設定與 keyring。
use lettre::message::header::{InReplyTo, References};
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

pub struct Outgoing<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub orig_subject: &'a str,
    /// 原信的 Message-ID（不含角括號）；空字串則不帶串信標頭。
    pub orig_message_id: &'a str,
    pub body: &'a str,
}

pub fn reply_subject(orig: &str) -> String {
    let t = orig.trim();
    if t.len() >= 3 && t[..3].eq_ignore_ascii_case("re:") { t.to_string() } else { format!("Re: {t}") }
}

pub fn build_reply(o: &Outgoing<'_>) -> Result<Message, String> {
    let from: Mailbox = o.from.parse().map_err(|e| format!("寄件地址無效：{e}"))?;
    let to: Mailbox = o.to.parse().map_err(|e| format!("收件地址無效：{e}"))?;
    let mut b = Message::builder().from(from).to(to).subject(reply_subject(o.orig_subject));
    if !o.orig_message_id.is_empty() {
        let id = format!("<{}>", o.orig_message_id.trim_matches(|c| c == '<' || c == '>'));
        b = b.header(InReplyTo::from(id.clone())).header(References::from(id));
    }
    b.body(o.body.to_string()).map_err(|e| e.to_string())
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
        let m = build_reply(&Outgoing { from: "me@home.com", to: "Bob <bob@corp.com>", orig_subject: "q", orig_message_id: "<abc@corp>", body: "答案" }).unwrap();
        let raw = String::from_utf8(m.formatted()).unwrap();
        assert!(raw.contains("In-Reply-To: <abc@corp>"), "{raw}");
        assert!(raw.contains("References: <abc@corp>"), "{raw}");
        assert!(raw.contains("Subject: Re: q"), "{raw}");
        assert!(raw.contains("bob@corp.com"));
    }

    #[test]
    fn invalid_address_is_an_error_not_a_panic() {
        assert!(build_reply(&Outgoing { from: "nope", to: "a@b.c", orig_subject: "", orig_message_id: "", body: "" }).is_err());
    }
}
