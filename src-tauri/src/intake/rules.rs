//! 收信規則：辨識模式、白名單、SPF/DKIM 結果。全是純函式，不碰 I/O。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    Alias,
    SubjectTag,
    AliasOrTag,
    /// 專用信箱：整個帳號都是問題，不另外辨識。
    Dedicated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Internal,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    pub address: String,
    pub level: Level,
    /// 外部地址可略過 SPF/DKIM（設定頁有風險提示）。
    pub skip_auth: bool,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub mode: MatchMode,
    pub alias: String,
    pub tag: String,
    /// 非空時，驗證結果標頭的 authserv-id 必須含這段文字。
    pub auth_server: String,
    /// 內部寄件者在收信端沒給驗證標頭時是否放行（預設否）。
    pub internal_without_auth: bool,
}

/// 只含判斷需要的標頭。`auth_results` 依信件中出現的順序（最上面＝自己的信箱最後加的）。
#[derive(Debug, Clone, Default)]
pub struct Headers {
    pub from: String,
    pub recipients: Vec<String>, // To / Cc / Delivered-To
    pub subject: String,
    pub auth_results: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drop {
    NotForUs,
    NotAllowed,
    AuthFailed,
    NoAuthHeader,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Accept(Level),
    Drop(Drop),
}

pub fn normalize(addr: &str) -> String {
    let a = addr.trim();
    let a = match (a.rfind('<'), a.rfind('>')) {
        (Some(l), Some(r)) if l < r => &a[l + 1..r],
        _ => a,
    };
    a.trim().to_lowercase()
}

/// 白名單可以放電子郵件地址，或 Telegram 聊天室 `tg:<chat_id>`。
pub fn valid_address(addr: &str) -> bool {
    let a = normalize(addr);
    if let Some(id) = a.strip_prefix("tg:") {
        return id.parse::<i64>().is_ok();
    }
    let mut parts = a.split('@');
    matches!((parts.next(), parts.next(), parts.next()), (Some(l), Some(d), None) if !l.is_empty() && d.contains('.'))
}

fn domain_of(addr: &str) -> &str {
    addr.rsplit('@').next().unwrap_or("")
}

pub fn addressed_to_us(s: &Settings, h: &Headers) -> bool {
    let alias = normalize(&s.alias);
    let alias_hit = !alias.is_empty() && h.recipients.iter().any(|r| normalize(r) == alias);
    let tag = s.tag.trim().to_lowercase();
    let tag_hit = !tag.is_empty() && h.subject.trim_start().to_lowercase().starts_with(&tag);
    match s.mode {
        MatchMode::Dedicated => true,
        MatchMode::Alias => alias_hit,
        MatchMode::SubjectTag => tag_hit,
        MatchMode::AliasOrTag => alias_hit || tag_hit,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Auth {
    Pass,
    Fail,
    Missing,
}

fn aligned(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    !a.is_empty() && (a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}")))
}

/// 取最上面一筆驗證標頭（自己的信箱服務商最後加的；寄件者夾帶的在更下面）。
/// spf=pass 或 dkim=pass 任一，且驗證網域與 From 網域對齊才算通過。
pub fn check_auth(headers: &[String], auth_server: &str, from: &str) -> Auth {
    let Some(top) = headers.first() else { return Auth::Missing };
    let mut segs = top.split(';');
    let id = segs.next().unwrap_or("").trim().to_lowercase();
    if !auth_server.trim().is_empty() && !id.contains(&auth_server.trim().to_lowercase()) {
        return Auth::Missing;
    }
    let from_dom = domain_of(&normalize(from)).to_string();
    for seg in segs {
        let toks: Vec<&str> = seg.split_whitespace().collect();
        let Some(first) = toks.first() else { continue };
        let Some((method, result)) = first.split_once('=') else { continue };
        if !result.eq_ignore_ascii_case("pass") {
            continue;
        }
        let want: &[&str] = match method.to_lowercase().as_str() {
            "spf" => &["smtp.mailfrom", "smtp.helo"],
            "dkim" => &["header.d", "header.i"],
            _ => continue,
        };
        for t in &toks[1..] {
            if let Some((k, v)) = t.split_once('=') {
                if want.contains(&k.to_lowercase().as_str()) {
                    let v = v.trim_matches(|c| c == '"' || c == '<' || c == '>');
                    if aligned(domain_of(v).trim_start_matches('@'), &from_dom) {
                        return Auth::Pass;
                    }
                }
            }
        }
    }
    Auth::Fail
}

pub fn decide(s: &Settings, senders: &[Sender], h: &Headers) -> Decision {
    if !addressed_to_us(s, h) {
        return Decision::Drop(Drop::NotForUs);
    }
    let from = normalize(&h.from);
    let Some(sender) = senders.iter().find(|x| normalize(&x.address) == from) else {
        return Decision::Drop(Drop::NotAllowed);
    };
    let needs_auth = match sender.level {
        Level::External => !sender.skip_auth,
        Level::Internal => false,
    };
    if needs_auth {
        match check_auth(&h.auth_results, &s.auth_server, &h.from) {
            Auth::Pass => {}
            Auth::Fail => return Decision::Drop(Drop::AuthFailed),
            Auth::Missing => return Decision::Drop(Drop::NoAuthHeader),
        }
    } else if sender.level == Level::Internal && !s.internal_without_auth {
        // 內部寄件者：有標頭就必須不是明確失敗；沒標頭時依設定放行。
        match check_auth(&h.auth_results, &s.auth_server, &h.from) {
            Auth::Fail => return Decision::Drop(Drop::AuthFailed),
            Auth::Missing => return Decision::Accept(sender.level),
            Auth::Pass => {}
        }
    }
    Decision::Accept(sender.level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(mode: MatchMode) -> Settings {
        Settings { mode, alias: "Me+AITerm@gmail.com".into(), tag: "[AITerm]".into(), auth_server: String::new(), internal_without_auth: false }
    }
    fn ext(addr: &str) -> Sender { Sender { address: addr.into(), level: Level::External, skip_auth: false } }
    fn int(addr: &str) -> Sender { Sender { address: addr.into(), level: Level::Internal, skip_auth: false } }
    fn hdr(from: &str, to: &[&str], subject: &str, auth: &[&str]) -> Headers {
        Headers { from: from.into(), recipients: to.iter().map(|s| s.to_string()).collect(), subject: subject.into(), auth_results: auth.iter().map(|s| s.to_string()).collect() }
    }
    const PASS: &str = "mx.google.com; dkim=pass header.i=@corp.com header.s=x; spf=pass smtp.mailfrom=a@corp.com";

    #[test]
    fn alias_matches_case_insensitively_incl_display_name() {
        let h = hdr("a@corp.com", &["Me <me+aiterm@gmail.com>"], "hi", &[]);
        // 收件者含顯示名稱時要先正規化
        assert!(addressed_to_us(&settings(MatchMode::Alias), &h));
        assert!(!addressed_to_us(&settings(MatchMode::Alias), &hdr("a@corp.com", &["me@gmail.com"], "hi", &[])));
    }

    #[test]
    fn tag_matches_only_at_subject_start() {
        let s = settings(MatchMode::SubjectTag);
        assert!(addressed_to_us(&s, &hdr("a@b.c", &[], "  [aiterm] bug", &[])));
        assert!(!addressed_to_us(&s, &hdr("a@b.c", &[], "re: [AITerm] bug", &[])));
    }

    #[test]
    fn either_mode_and_dedicated() {
        let h = hdr("a@b.c", &["x@y.z"], "plain", &[]);
        assert!(!addressed_to_us(&settings(MatchMode::AliasOrTag), &h));
        assert!(addressed_to_us(&settings(MatchMode::AliasOrTag), &hdr("a@b.c", &[], "[AITerm] x", &[])));
        assert!(addressed_to_us(&settings(MatchMode::Dedicated), &h));
    }

    #[test]
    fn allowlist_addresses_are_emails_or_telegram_chats() {
        for ok in ["a@b.co", "Bob <bob@corp.com>", "tg:12345", "tg:-1001234"] {
            assert!(valid_address(ok), "{ok}");
        }
        for bad in ["", "nope", "a@b", "@b.co", "a@@b.co", "tg:", "tg:abc", "a@b.co@c.d"] {
            assert!(!valid_address(bad), "{bad}");
        }
    }

    #[test]
    fn unknown_sender_dropped() {
        let d = decide(&settings(MatchMode::Dedicated), &[ext("a@corp.com")], &hdr("evil@x.com", &[], "s", &[PASS]));
        assert_eq!(d, Decision::Drop(Drop::NotAllowed));
    }

    #[test]
    fn external_needs_aligned_pass() {
        let s = settings(MatchMode::Dedicated);
        let senders = [ext("a@corp.com")];
        assert_eq!(decide(&s, &senders, &hdr("a@corp.com", &[], "s", &[PASS])), Decision::Accept(Level::External));
        // 驗證網域與 From 不對齊（偽造）
        let forged = "mx.google.com; spf=pass smtp.mailfrom=bot@attacker.net";
        assert_eq!(decide(&s, &senders, &hdr("a@corp.com", &[], "s", &[forged])), Decision::Drop(Drop::AuthFailed));
        let failing = "mx.google.com; spf=fail smtp.mailfrom=a@corp.com; dkim=none";
        assert_eq!(decide(&s, &senders, &hdr("a@corp.com", &[], "s", &[failing])), Decision::Drop(Drop::AuthFailed));
        assert_eq!(decide(&s, &senders, &hdr("a@corp.com", &[], "s", &[])), Decision::Drop(Drop::NoAuthHeader));
    }

    #[test]
    fn only_topmost_auth_header_is_trusted() {
        let s = settings(MatchMode::Dedicated);
        let own_fail = "mx.google.com; spf=fail smtp.mailfrom=a@corp.com";
        let spoofed = "evil.net; spf=pass smtp.mailfrom=a@corp.com";
        let d = decide(&s, &[ext("a@corp.com")], &hdr("a@corp.com", &[], "s", &[own_fail, spoofed]));
        assert_eq!(d, Decision::Drop(Drop::AuthFailed));
    }

    #[test]
    fn auth_server_pin_rejects_foreign_header() {
        let mut s = settings(MatchMode::Dedicated);
        s.auth_server = "google.com".into();
        let foreign = "evil.net; spf=pass smtp.mailfrom=a@corp.com";
        assert_eq!(decide(&s, &[ext("a@corp.com")], &hdr("a@corp.com", &[], "s", &[foreign])), Decision::Drop(Drop::NoAuthHeader));
    }

    #[test]
    fn skip_auth_and_subdomain_alignment() {
        let s = settings(MatchMode::Dedicated);
        let mut e = ext("a@corp.com");
        e.skip_auth = true;
        assert_eq!(decide(&s, &[e], &hdr("a@corp.com", &[], "s", &[])), Decision::Accept(Level::External));
        let sub = "mx.google.com; dkim=pass header.d=mail.corp.com";
        assert_eq!(check_auth(&[sub.into()], "", "a@corp.com"), Auth::Pass);
    }

    #[test]
    fn internal_sender_not_blocked_without_header_but_blocked_on_explicit_fail() {
        let s = settings(MatchMode::Dedicated);
        let senders = [int("me@home.com")];
        assert_eq!(decide(&s, &senders, &hdr("me@home.com", &[], "s", &[])), Decision::Accept(Level::Internal));
        let fail = "mx.google.com; spf=fail smtp.mailfrom=me@home.com";
        assert_eq!(decide(&s, &senders, &hdr("me@home.com", &[], "s", &[fail])), Decision::Drop(Drop::AuthFailed));
    }
}
