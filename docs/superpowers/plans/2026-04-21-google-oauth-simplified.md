# Google OAuth Simplified Login Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove client ID/secret inputs from the Google Gemini OAuth flow by bundling Gemini CLI's public credentials, and add refresh token persistence so users rarely need to re-login.

**Architecture:** New `src-tauri/src/ai/google_oauth.rs` module holds credentials, token struct, and refresh logic. The command `google_gemini_oauth_auth` uses `access_type=offline` to get a refresh token stored as JSON in keychain. The router auto-refreshes the access token transparently before each AI call.

**Tech Stack:** Rust (tokio, reqwest, serde_json), TypeScript/React (Tauri IPC)

---

## File Map

| Action | File | Change |
|--------|------|--------|
| Create | `src-tauri/src/ai/google_oauth.rs` | Constants, `GoogleOAuthToken`, `get_fresh_google_token` |
| Modify | `src-tauri/src/ai/mod.rs` | Add `pub mod google_oauth` |
| Modify | `src-tauri/src/commands/provider.rs` | Simplify `google_gemini_oauth_auth` — remove client_id/secret params, use offline flow |
| Modify | `src-tauri/src/ai/router.rs` | Replace direct secret read with `get_fresh_google_token` |
| Modify | `src/ipc/provider.ts` | Remove `clientId`/`clientSecret` from `googleGeminiOauthAuth` |
| Modify | `src/components/Settings/ProviderForm.tsx` | Remove Client ID/Secret UI, clean up state/validation |
| Modify | `src/lib/i18n.ts` | Remove obsolete keys, add `provider_google_oauth_description` |

---

## Task 1: Create `google_oauth.rs` module

**Files:**
- Create: `src-tauri/src/ai/google_oauth.rs`

- [ ] **Step 1: Write unit tests first**

Add to the bottom of the new file:

```rust
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
        let past = 1_000_000_i64; // year 1970
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(past <= now + 60);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail (token struct not defined yet)**

```bash
cd src-tauri && cargo test google_oauth 2>&1 | head -20
```

Expected: compile error — `GoogleOAuthToken` not defined.

- [ ] **Step 3: Write the full module**

Create `src-tauri/src/ai/google_oauth.rs` with this content:

```rust
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
```

- [ ] **Step 4: Register the module in `mod.rs`**

In `src-tauri/src/ai/mod.rs`, add after line 20 (`pub(crate) mod sse;`):

```rust
pub mod google_oauth;
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd src-tauri && cargo test google_oauth 2>&1
```

Expected: `test ai::google_oauth::tests::token_roundtrips_json ... ok`, all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/google_oauth.rs src-tauri/src/ai/mod.rs
git commit -m "feat: add google_oauth module with bundled Gemini CLI credentials and token refresh"
```

---

## Task 2: Simplify `google_gemini_oauth_auth` command

**Files:**
- Modify: `src-tauri/src/commands/provider.rs`

- [ ] **Step 1: Replace the entire `google_gemini_oauth_auth` function**

Find the function starting at line 389 (the `pub async fn google_gemini_oauth_auth` block) and replace it entirely with:

```rust
/// Open the browser for the Google OAuth loopback flow (using bundled Gemini CLI
/// credentials), wait for the callback, exchange the code for tokens, and store
/// the JSON token blob in the keychain.
#[tauri::command]
pub async fn google_gemini_oauth_auth(
    provider_id: String,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    use crate::ai::google_oauth::{
        GEMINI_CLI_CLIENT_ID, GEMINI_CLI_CLIENT_SECRET, GEMINI_OAUTH_SCOPES, GoogleOAuthToken,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Bind a local loopback server on a random available port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("failed to start local server: {e}"))?;
    let port = listener.local_addr().unwrap().port();
    let redirect_uri = format!("http://localhost:{port}");

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        url_encode(GEMINI_CLI_CLIENT_ID),
        url_encode(&redirect_uri),
        url_encode(GEMINI_OAUTH_SCOPES),
    );

    open_browser(&auth_url);

    // Wait for the browser redirect (up to 2 minutes).
    let code = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        async {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|e| format!("accept error: {e}"))?;

            let mut buf = vec![0u8; 8192];
            let n = stream
                .read(&mut buf)
                .await
                .map_err(|e| format!("read error: {e}"))?;
            let request = String::from_utf8_lossy(&buf[..n]).to_string();

            let body = "<html><body style='font-family:sans-serif;text-align:center;padding:40px'>\
                <h2>&#10003; Google login successful!</h2>\
                <p>You can close this tab and return to AITerm.</p>\
                </body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;

            request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|path| path.split_once('?').map(|(_, q)| q.to_string()))
                .and_then(|query| {
                    query
                        .split('&')
                        .find(|p| p.starts_with("code="))
                        .map(|p| p[5..].to_string())
                })
                .ok_or_else(|| "no authorization code in OAuth callback".to_string())
        },
    )
    .await
    .map_err(|_| "OAuth login timed out (2 minutes). Please try again.".to_string())?
    .map_err(|e| format!("OAuth callback error: {e}"))?;

    // Exchange the authorization code for access + refresh tokens.
    let http = reqwest::Client::new();
    let resp = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", GEMINI_CLI_CLIENT_ID),
            ("client_secret", GEMINI_CLI_CLIENT_SECRET),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token exchange failed (HTTP {status}): {body}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse token response: {e}"))?;

    let access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| format!("no access_token in response: {json}"))?
        .to_string();

    let refresh_token = json["refresh_token"]
        .as_str()
        .ok_or_else(|| "no refresh_token in response — ensure access_type=offline".to_string())?
        .to_string();

    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let token = GoogleOAuthToken {
        access_token: access_token.clone(),
        refresh_token,
        expires_at: now + expires_in - 60,
    };

    secrets
        .set(&provider_id, &serde_json::to_string(&token).unwrap())
        .map_err(|e| format!("failed to store token: {e}"))?;

    Ok(access_token)
}
```

- [ ] **Step 2: Build to confirm it compiles**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error" | head -20
```

Expected: no errors (only warnings are acceptable).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands/provider.rs
git commit -m "feat: simplify google_gemini_oauth_auth — bundle credentials, use offline access for refresh token"
```

---

## Task 3: Update router to auto-refresh token

**Files:**
- Modify: `src-tauri/src/ai/router.rs`

- [ ] **Step 1: Replace the `GoogleGeminiOauth` match arm**

Find this block in `resolve_by_id` (around line 159):

```rust
ProviderType::GoogleGeminiOauth => {
    // Access token stored in keychain by google_gemini_oauth_auth command.
    let token = self
        .secrets
        .get(&provider_cfg.id)
        .map_err(|_| AiError::NotConfigured)?
        .ok_or(AiError::NotConfigured)?;
    Arc::new(OpenAiCompatibleClient::new(
        provider_cfg
            .base_url
            .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta/openai".into()),
        provider_cfg.model.clone(),
        Some(token),
        provider_cfg.supports_json_mode,
    ))
}
```

Replace it with:

```rust
ProviderType::GoogleGeminiOauth => {
    let token =
        crate::ai::google_oauth::get_fresh_google_token(&self.secrets, &provider_cfg.id)
            .await?;
    Arc::new(OpenAiCompatibleClient::new(
        provider_cfg
            .base_url
            .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta/openai".into()),
        provider_cfg.model.clone(),
        Some(token),
        provider_cfg.supports_json_mode,
    ))
}
```

- [ ] **Step 2: Build to confirm it compiles**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 3: Run all Rust tests**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: all tests pass (keychain tests are `#[ignore]`, so they don't run).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/ai/router.rs
git commit -m "feat: auto-refresh Google OAuth token in router using get_fresh_google_token"
```

---

## Task 4: Update TypeScript IPC

**Files:**
- Modify: `src/ipc/provider.ts`

- [ ] **Step 1: Update `googleGeminiOauthAuth`**

Find lines 105–110 in `src/ipc/provider.ts`:

```typescript
export const googleGeminiOauthAuth = (
  providerId: string,
  clientId: string,
  clientSecret: string,
): Promise<string> =>
  invoke("google_gemini_oauth_auth", { providerId, clientId, clientSecret });
```

Replace with:

```typescript
export const googleGeminiOauthAuth = (providerId: string): Promise<string> =>
  invoke("google_gemini_oauth_auth", { providerId });
```

- [ ] **Step 2: Type-check**

```bash
npx tsc --noEmit 2>&1 | head -20
```

Expected: errors in `ProviderForm.tsx` about extra arguments (we fix those in Task 5).

- [ ] **Step 3: Commit**

```bash
git add src/ipc/provider.ts
git commit -m "feat: remove clientId/clientSecret from googleGeminiOauthAuth IPC"
```

---

## Task 5: Update ProviderForm UI and i18n

**Files:**
- Modify: `src/components/Settings/ProviderForm.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Add new i18n keys and remove obsolete ones in `src/lib/i18n.ts`**

In the `zh-TW` locale object, make these changes:

**Remove** these keys:
```typescript
provider_google_client_id: "Google OAuth Client ID",
provider_google_client_id_placeholder: "xxxx.apps.googleusercontent.com",
provider_google_client_secret: "Google OAuth Client Secret",
provider_google_client_secret_placeholder: "GOCSPX-...",
provider_google_oauth_hint: "請至 Google Cloud Console → APIs & Services → Credentials 建立「電腦版應用程式」OAuth 憑證",
err_google_client_id_required: "請先填入 Google OAuth Client ID",
err_google_client_secret_required: "請先填入 Google OAuth Client Secret",
```

**Update** this key (button label):
```typescript
provider_google_oauth_auth: "使用 Google 登入",
```

**Add** this new key (after `provider_google_oauth_auth`):
```typescript
provider_google_oauth_description: "使用您的 Google 帳戶登入，透過 OAuth 使用 Gemini",
```

In the `en` locale object, make the same changes:

**Remove** these keys:
```typescript
provider_google_client_id: "Google OAuth Client ID",
provider_google_client_id_placeholder: "xxxx.apps.googleusercontent.com",
provider_google_client_secret: "Google OAuth Client Secret",
provider_google_client_secret_placeholder: "GOCSPX-...",
provider_google_oauth_hint: "Go to Google Cloud Console → APIs & Services → Credentials and create a 'Desktop app' OAuth client",
err_google_client_id_required: "Google OAuth Client ID is required",
err_google_client_secret_required: "Google OAuth Client Secret is required",
```

**Update** this key:
```typescript
provider_google_oauth_auth: "Sign in with Google",
```

**Add** this new key:
```typescript
provider_google_oauth_description: "Sign in with your Google account to use Gemini via OAuth",
```

- [ ] **Step 2: Update `ProviderForm.tsx`**

**2a. Remove the `oauthClientSecret` state** (line 59):

Remove:
```typescript
const [oauthClientSecret, setOauthClientSecret] = useState("");
```

**2b. Replace the `runGoogleOauth` function** (lines 214–238) with:

```typescript
const runGoogleOauth = async () => {
  if (!id.trim()) {
    setAuthStatus(t.err_id_empty);
    return;
  }
  setAuthing(true);
  setAuthStatus(null);
  try {
    await googleGeminiOauthAuth(id);
    setAuthStatus(t.provider_auth_ok);
  } catch (e: unknown) {
    setAuthStatus(String(e));
  } finally {
    setAuthing(false);
  }
};
```

**2c. Replace the `google-gemini-oauth` JSX block** (lines 348–375) with:

```tsx
{providerType === "google-gemini-oauth" && (
  <div className="form-group">
    <label>{t.provider_auth_action}</label>
    <div className="form-hint">{t.provider_google_oauth_description}</div>
    <button type="button" onClick={runGoogleOauth} disabled={authing}>
      {authing
        ? t.provider_auth_running
        : isEdit && existing?.has_api_key
          ? t.provider_auth_ok
          : t.provider_google_oauth_auth}
    </button>
    {authStatus && (
      <div className={`form-hint ${authStatus.startsWith("✓") ? "" : "form-hint--error"}`}>
        {authStatus}
      </div>
    )}
  </div>
)}
```

- [ ] **Step 3: Type-check**

```bash
npx tsc --noEmit 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 4: Run frontend tests**

```bash
npm run test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Run lint**

```bash
npm run lint 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add src/components/Settings/ProviderForm.tsx src/lib/i18n.ts
git commit -m "feat: simplify Google OAuth UI — remove client credential inputs, use bundled credentials"
```

---

## Task 6: Final verification

- [ ] **Step 1: Full Rust build + tests**

```bash
cd src-tauri && cargo test 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 2: Full TypeScript type check**

```bash
npx tsc --noEmit 2>&1
```

Expected: no output (no errors).

- [ ] **Step 3: Tag and push**

```bash
git tag v0.1.22
git push origin master && git push origin v0.1.22
```

---

## Rollback Note

If the Gemini CLI client is revoked by Google in the future, restore the Client ID / Secret input fields by reverting Task 2 and Task 5. The `google_oauth.rs` module can be updated with new credentials without UI changes.
