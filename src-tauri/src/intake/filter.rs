//! 寄出前的機密過濾：遮蔽疑似金鑰與本機絕對路徑。
use regex::Regex;
use std::sync::OnceLock;

fn patterns() -> &'static [(Regex, &'static str)] {
    static P: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    P.get_or_init(|| {
        let mk = |p: &str, r: &'static str| (Regex::new(p).unwrap(), r);
        vec![
            mk(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----", "[已遮蔽：私鑰]"),
            mk(r"(?i)\bbearer\s+[A-Za-z0-9._\-]{16,}", "Bearer [已遮蔽]"),
            mk(r"\b(?:sk-[A-Za-z0-9_\-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9\-]{10,})", "[已遮蔽：金鑰]"),
            // 家目錄：保留其後的相對部分，只拿掉使用者名稱。
            mk(r"(?:/Users|/home)/[^/\s]+", "~"),
            mk(r"(?i)\b[A-Z]:\\Users\\[^\\\s]+", "~"),
        ]
    })
}

pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for (re, rep) in patterns() {
        out = re.replace_all(&out, *rep).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_api_keys() {
        let t = redact("key=sk-ant-abcdefghijklmnop1234 and ghp_abcdefghijklmnopqrstuvwx and AKIAABCDEFGHIJKLMNOP");
        assert!(!t.contains("sk-ant"), "{t}");
        assert!(!t.contains("ghp_"), "{t}");
        assert!(!t.contains("AKIA"), "{t}");
        assert_eq!(t.matches("[已遮蔽：金鑰]").count(), 3);
    }

    #[test]
    fn masks_pem_and_bearer() {
        let t = redact("-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----\nAuthorization: Bearer abcdefghijklmnop12345");
        assert!(!t.contains("MIIabc"));
        assert!(!t.contains("abcdefghijklmnop12345"));
    }

    #[test]
    fn masks_home_paths_on_all_platforms() {
        assert_eq!(redact("see /Users/jamesju/proj/a.rs"), "see ~/proj/a.rs");
        assert_eq!(redact("see /home/bob/x"), "see ~/x");
        assert_eq!(redact(r"see C:\Users\Bob\proj\a.rs"), r"see ~\proj\a.rs");
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let s = "The sk-learn skeleton, a /home page, and C:\\Windows\\System32 are fine. Use Bearer tokens.";
        assert_eq!(redact(s), s);
    }
}
