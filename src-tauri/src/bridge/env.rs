//! 注入終端機分頁的環境變數。

/// 啟用橋接的分頁必須清掉的變數。
///
/// Claude Code 在 `ANTHROPIC_AUTH_TOKEN` 與 `ANTHROPIC_API_KEY` 並存時的
/// 優先序未經我們驗證，與其依賴假設，不如把 API key 清掉 —— 症狀
/// 「設了橋接卻打到真的 Anthropic」極難追查。
pub const ENV_TO_REMOVE: &[&str] = &["ANTHROPIC_API_KEY"];

/// 產生要塞進 `ShellSpec.envs` 的鍵值對。
pub fn bridge_envs(port: u16, token: &str) -> Vec<(String, String)> {
    let pair = |k: &str, v: String| (k.to_string(), v);
    vec![
        // 不能帶 /v1 後綴：Claude Code 自己會接上 /v1/messages。
        pair("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}")),
        pair("ANTHROPIC_AUTH_TOKEN", token.to_string()),
        // 哨兵字串：server 直接用它判層級，比猜真實型號穩定。
        pair("ANTHROPIC_DEFAULT_OPUS_MODEL", "aiterm:opus".into()),
        pair("ANTHROPIC_DEFAULT_SONNET_MODEL", "aiterm:sonnet".into()),
        pair("ANTHROPIC_DEFAULT_HAIKU_MODEL", "aiterm:haiku".into()),
        // 本地模型冷啟動很慢，預設逾時會在第一次請求就砍掉連線。
        pair("API_TIMEOUT_MS", "3000000".into()),
        pair("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(envs: &'a [(String, String)], key: &str) -> Option<&'a str> {
        envs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn base_url_has_no_v1_suffix() {
        // Claude Code 自己會接上 /v1/messages；帶了 /v1 會變成 /v1/v1/messages。
        let e = bridge_envs(8317, "tok");
        assert_eq!(find(&e, "ANTHROPIC_BASE_URL"), Some("http://127.0.0.1:8317"));
    }

    #[test]
    fn injects_the_three_tier_sentinels() {
        let e = bridge_envs(8317, "tok");
        assert_eq!(find(&e, "ANTHROPIC_DEFAULT_OPUS_MODEL"), Some("aiterm:opus"));
        assert_eq!(find(&e, "ANTHROPIC_DEFAULT_SONNET_MODEL"), Some("aiterm:sonnet"));
        assert_eq!(find(&e, "ANTHROPIC_DEFAULT_HAIKU_MODEL"), Some("aiterm:haiku"));
    }

    #[test]
    fn injects_auth_token_and_timeouts() {
        let e = bridge_envs(9000, "secret-token");
        assert_eq!(find(&e, "ANTHROPIC_AUTH_TOKEN"), Some("secret-token"));
        assert_eq!(find(&e, "API_TIMEOUT_MS"), Some("3000000"));
        assert_eq!(find(&e, "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"), Some("1"));
    }

    #[test]
    fn api_key_is_listed_for_removal() {
        // 使用者環境本來就有的 ANTHROPIC_API_KEY 是難查的干擾源：症狀是
        // 「明明設了橋接卻打到真的 Anthropic」。
        assert!(ENV_TO_REMOVE.contains(&"ANTHROPIC_API_KEY"));
    }

    #[test]
    fn port_is_reflected_in_the_url() {
        let e = bridge_envs(12345, "t");
        assert_eq!(find(&e, "ANTHROPIC_BASE_URL"), Some("http://127.0.0.1:12345"));
    }
}
