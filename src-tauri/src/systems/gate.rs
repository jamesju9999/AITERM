use super::types::{Decision, DenyReason, Mode, Rule, RuleAction, SystemConnection};

pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?; // 去掉 user:pass@
    let host = authority.split(':').next()?; // 去掉埠
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn host_allowed(allowed: &[String], url: &str) -> bool {
    let Some(host) = host_of(url) else { return false };
    allowed.iter().any(|a| {
        let a = a.trim().trim_start_matches("*.").to_ascii_lowercase();
        !a.is_empty() && (host == a || host.ends_with(&format!(".{a}")))
    })
}

/// 只支援 `*`（任意長度）的萬用字元比對。
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return part.is_empty() || rest.ends_with(part);
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    true
}

/// 閘門判定。`is_navigation`：這是不是頂層頁面導覽（用來套網域白名單；子資源的 GET 可以去 CDN）。
pub fn decide(conn: &SystemConnection, method: &str, url: &str, is_navigation: bool) -> Decision {
    if !conn.enabled {
        return Decision::Deny(DenyReason::Disabled);
    }
    let host_ok = host_allowed(&conn.allowed_hosts, url);
    if is_navigation && !host_ok {
        return Decision::Deny(DenyReason::HostNotAllowed);
    }
    let m = method.to_ascii_uppercase();
    if matches!(m.as_str(), "GET" | "HEAD" | "OPTIONS") {
        return Decision::Allow;
    }
    if !host_ok {
        return Decision::Deny(DenyReason::HostNotAllowed);
    }
    match conn.mode {
        Mode::ReadOnly => Decision::Deny(DenyReason::ReadOnly),
        Mode::ApproveEach => Decision::Ask,
        Mode::Autonomous => Decision::Allow,
        Mode::Rules => conn
            .rules
            .iter()
            .find(|r: &&Rule| (r.method == "*" || r.method.eq_ignore_ascii_case(&m)) && glob_match(&r.url_pattern, url))
            .map(|r| match r.action {
                RuleAction::Allow => Decision::Allow,
                RuleAction::Ask => Decision::Ask,
            })
            .unwrap_or(Decision::Ask),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(mode: Mode) -> SystemConnection {
        SystemConnection {
            id: "s".into(),
            name: "S".into(),
            start_url: "https://app.example.com/".into(),
            allowed_hosts: vec!["example.com".into()],
            mode,
            rules: vec![],
            enabled: true, model: None, upload_dirs: vec![],
        }
    }

    const U: &str = "https://app.example.com/orders";

    #[test]
    fn reads_are_always_allowed_on_allowed_hosts() {
        for m in ["GET", "HEAD", "OPTIONS", "get"] {
            for mode in [Mode::ReadOnly, Mode::ApproveEach, Mode::Rules, Mode::Autonomous] {
                assert_eq!(decide(&conn(mode), m, U, false), Decision::Allow, "{m} {mode:?}");
            }
        }
    }

    #[test]
    fn writes_follow_the_mode() {
        assert_eq!(decide(&conn(Mode::ReadOnly), "POST", U, false), Decision::Deny(DenyReason::ReadOnly));
        assert_eq!(decide(&conn(Mode::ApproveEach), "DELETE", U, false), Decision::Ask);
        assert_eq!(decide(&conn(Mode::Autonomous), "PUT", U, false), Decision::Allow);
        assert_eq!(decide(&conn(Mode::Rules), "PATCH", U, false), Decision::Ask, "沒命中規則的一律問人");
    }

    #[test]
    fn rules_first_match_wins_and_method_is_case_insensitive() {
        let mut c = conn(Mode::Rules);
        c.rules = vec![
            Rule { method: "post".into(), url_pattern: "https://app.example.com/drafts/*".into(), action: RuleAction::Allow },
            Rule { method: "*".into(), url_pattern: "https://app.example.com/*".into(), action: RuleAction::Ask },
        ];
        assert_eq!(decide(&c, "POST", "https://app.example.com/drafts/new", false), Decision::Allow);
        assert_eq!(decide(&c, "POST", "https://app.example.com/orders", false), Decision::Ask);
        assert_eq!(decide(&c, "DELETE", "https://app.example.com/drafts/9", false), Decision::Ask, "方法不同不命中第一條");
    }

    #[test]
    fn navigation_outside_allowed_hosts_is_denied_even_for_get() {
        assert_eq!(
            decide(&conn(Mode::Autonomous), "GET", "https://evil.test/", true),
            Decision::Deny(DenyReason::HostNotAllowed)
        );
        // 子資源（圖片、腳本）的 GET 可以去 CDN。
        assert_eq!(decide(&conn(Mode::ReadOnly), "GET", "https://cdn.other.test/a.js", false), Decision::Allow);
    }

    #[test]
    fn writes_to_other_hosts_are_denied_in_every_mode() {
        for mode in [Mode::ReadOnly, Mode::ApproveEach, Mode::Rules, Mode::Autonomous] {
            assert_eq!(
                decide(&conn(mode), "POST", "https://evil.test/collect", false),
                Decision::Deny(DenyReason::HostNotAllowed),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn host_match_is_a_suffix_on_a_dot_boundary() {
        let c = conn(Mode::Autonomous);
        assert_eq!(decide(&c, "POST", "https://example.com/x", false), Decision::Allow);
        assert_eq!(decide(&c, "POST", "https://a.b.example.com/x", false), Decision::Allow);
        assert_eq!(
            decide(&c, "POST", "https://notexample.com/x", false),
            Decision::Deny(DenyReason::HostNotAllowed),
            "notexample.com 不是 example.com 的子網域"
        );
        assert_eq!(
            decide(&c, "POST", "https://example.com.evil.test/x", false),
            Decision::Deny(DenyReason::HostNotAllowed)
        );
    }

    #[test]
    fn a_disabled_connection_denies_everything() {
        let mut c = conn(Mode::Autonomous);
        c.enabled = false;
        assert_eq!(decide(&c, "GET", U, false), Decision::Deny(DenyReason::Disabled));
    }

    #[test]
    fn glob_match_handles_wildcards() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*c", "abbbc"));
        assert!(glob_match("a*c*e", "abcde"));
        assert!(!glob_match("a*c", "abd"));
        assert!(glob_match("abc", "abc"));
        assert!(!glob_match("abc", "abcd"));
    }
}
