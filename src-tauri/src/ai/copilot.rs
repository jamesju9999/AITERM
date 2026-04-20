//! GitHub Copilot token exchange.
//!
//! The raw GitHub OAuth token (from Device Flow) cannot be used directly
//! with `api.githubcopilot.com/chat/completions`.  It must first be
//! exchanged for a short-lived Copilot session token via
//! `https://api.github.com/copilot_internal/v2/token`.
//!
//! This module handles the exchange and caches the result so we don't
//! call the endpoint on every single AI request.

use once_cell::sync::Lazy;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Cached Copilot session token.
struct CachedToken {
    token: String,
    /// Unix timestamp (seconds) at which `token` expires.
    expires_at: i64,
    /// Hash of the GitHub OAuth token that produced this session token.
    /// If the user re-authenticates we invalidate automatically.
    source_hash: u64,
}

static CACHE: Lazy<Mutex<Option<CachedToken>>> = Lazy::new(|| Mutex::new(None));

/// Return a valid Copilot session token, using the cache when possible.
///
/// `github_token` is the OAuth access token stored in the keychain.
pub async fn get_copilot_session_token(github_token: &str) -> Result<String, String> {
    let now = now_secs();
    let hash = hash_str(github_token);

    // ── Check cache ──────────────────────────────────────────────────────
    {
        let guard = CACHE.lock().unwrap();
        if let Some(ref c) = *guard {
            // Refresh 60 s before actual expiry to avoid races.
            if c.source_hash == hash && c.expires_at - 60 > now {
                return Ok(c.token.clone());
            }
        }
    }

    // ── Exchange ─────────────────────────────────────────────────────────
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/copilot_internal/v2/token")
        .header("Authorization", format!("token {github_token}"))
        .header("User-Agent", "AITerm/1.0")
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Copilot token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Copilot token exchange returned {status}: {body}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Copilot token response: {e}"))?;

    let token = json["token"]
        .as_str()
        .ok_or_else(|| "Copilot token response missing 'token' field".to_string())?
        .to_string();

    // `expires_at` may be an integer (unix ts) or absent.
    let expires_at = json["expires_at"]
        .as_i64()
        .unwrap_or(now + 1500); // fallback: 25 min

    // ── Update cache ─────────────────────────────────────────────────────
    {
        let mut guard = CACHE.lock().unwrap();
        *guard = Some(CachedToken {
            token: token.clone(),
            expires_at,
            source_hash: hash,
        });
    }

    Ok(token)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}
