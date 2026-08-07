//! 橋接 server 的 bearer token。存在 OS keychain，key 見 [`BRIDGE_TOKEN_KEY`]。

/// Keychain 的 key。沿用 `SecretStore` 既有的冒號子鍵慣例
/// （例如 `{provider_id}:oauth_refresh`）。
pub const BRIDGE_TOKEN_KEY: &str = "claude-bridge:token";

/// 產生 32 bytes 的隨機 token（64 個 hex 字元）。
///
/// 用兩個 UUIDv4 串接而非引入 `rand`：`uuid` 已是依賴，其 v4 走的是
/// `getrandom`，密碼學強度足夠，且省一個 crate。
pub fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// 常數時間比對，避免用回應時間逐字元猜出 token。
///
/// 長度不同時提早返回會洩漏長度，但長度是固定的 64，不算資訊。
pub fn token_matches(expected: &str, provided: &str) -> bool {
    let a = expected.as_bytes();
    let b = provided.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 從請求標頭取出 token。
///
/// Claude Code 設了 `ANTHROPIC_AUTH_TOKEN` 時送 `Authorization: Bearer`，
/// 只設 `ANTHROPIC_API_KEY` 時送 `x-api-key`。兩種都接受，前者優先。
pub fn extract_token(authorization: Option<&str>, x_api_key: Option<&str>) -> Option<String> {
    if let Some(value) = authorization {
        let trimmed = value.trim();
        if trimmed.len() > 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
            return Some(trimmed[7..].trim().to_string());
        }
    }
    x_api_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_is_64_hex_chars() {
        let t = generate_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, generate_token(), "每次呼叫都要不同");
    }

    #[test]
    fn token_matches_is_exact() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc123", "abc124"));
        assert!(!token_matches("abc123", "abc1234"));
        assert!(!token_matches("abc123", ""));
    }

    #[test]
    fn extracts_bearer_token() {
        assert_eq!(extract_token(Some("Bearer xyz"), None).as_deref(), Some("xyz"));
        assert_eq!(extract_token(Some("bearer xyz"), None).as_deref(), Some("xyz"));
    }

    #[test]
    fn falls_back_to_x_api_key() {
        // Claude Code 在只設了 ANTHROPIC_API_KEY 時會送 x-api-key。
        assert_eq!(extract_token(None, Some("xyz")).as_deref(), Some("xyz"));
    }

    #[test]
    fn authorization_wins_over_x_api_key() {
        assert_eq!(extract_token(Some("Bearer a"), Some("b")).as_deref(), Some("a"));
    }

    #[test]
    fn no_credentials_yields_none() {
        assert_eq!(extract_token(None, None), None);
        assert_eq!(extract_token(Some("Basic xyz"), None), None);
    }
}
