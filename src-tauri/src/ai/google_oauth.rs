//! Bundled Gemini CLI OAuth credentials and token refresh logic.
//!
//! Credentials taken from the public Gemini CLI open-source repository:
//! https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/oauth2.ts
//! The client_secret for desktop OAuth apps is considered non-secret by Google.

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::{ai::AiError, secret::SecretStore};

pub const GEMINI_CLI_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
pub const GEMINI_CLI_CLIENT_SECRET: &str = "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";

/// OAuth scopes matching Gemini CLI registration.
pub const GEMINI_OAUTH_SCOPES: &str =
    "https://www.googleapis.com/auth/cloud-platform \
     https://www.googleapis.com/auth/userinfo.email \
     https://www.googleapis.com/auth/userinfo.profile";

/// Token blob stored in keychain as JSON.
#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleOAuthToken {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) after which the access_token is expired.
    pub expires_at: i64,
}

/// Returns a valid access token, refreshing it first if it has expired.
/// Reads/writes keychain via `secrets` under `provider_id`.
///
/// Errors:
/// - `NotConfigured` — no token stored or JSON is unparseable (user must re-login)
/// - `AuthFailed`    — refresh token has been revoked (user must re-login)
/// - `Network`       — transient HTTP error during refresh
pub async fn get_fresh_google_token(
    secrets: &Arc<SecretStore>,
    provider_id: &str,
) -> Result<String, AiError> {
    let raw = secrets
        .get(provider_id)
        .map_err(|_| AiError::NotConfigured)?
        .ok_or(AiError::NotConfigured)?;

    let token: GoogleOAuthToken =
        serde_json::from_str(&raw).map_err(|_| AiError::NotConfigured)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Return early if access token is still valid (with 60s buffer).
    if token.expires_at > now + 60 {
        return Ok(token.access_token);
    }

    // Access token expired — use refresh token to get a new one.
    let http = reqwest::Client::new();
    let resp = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", GEMINI_CLI_CLIENT_ID),
            ("client_secret", GEMINI_CLI_CLIENT_SECRET),
            ("refresh_token", token.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| AiError::Network { message: e.to_string() })?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(AiError::AuthFailed);
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AiError::Network {
            message: format!("token refresh failed (HTTP {status}): {body}"),
        });
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AiError::Network { message: e.to_string() })?;

    let new_access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| AiError::Network {
            message: "no access_token in refresh response".into(),
        })?
        .to_string();

    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let new_expires_at = now + expires_in - 60;

    // Persist updated token (refresh_token unchanged).
    let updated = GoogleOAuthToken {
        access_token: new_access_token.clone(),
        refresh_token: token.refresh_token,
        expires_at: new_expires_at,
    };
    let _ = secrets.set(provider_id, &serde_json::to_string(&updated).unwrap());

    Ok(new_access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrips_json() {
        let t = GoogleOAuthToken {
            access_token: "ya29.abc".into(),
            refresh_token: "1//xyz".into(),
            expires_at: 9999999999,
        };
        let s = serde_json::to_string(&t).unwrap();
        let parsed: GoogleOAuthToken = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.access_token, "ya29.abc");
        assert_eq!(parsed.refresh_token, "1//xyz");
        assert_eq!(parsed.expires_at, 9999999999);
    }

    #[test]
    fn non_expired_token_detected() {
        let far_future = 9_999_999_999_i64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(far_future > now + 60);
    }

    #[test]
    fn expired_token_detected() {
        let past = 1_000_000_i64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(past <= now + 60);
    }
}
