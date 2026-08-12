//! OS keychain wrapper.
//!
//! All secrets are stored under the service name `"aiterm"` with the
//! provider id as the username, e.g. `"aiterm" / "claude-sonnet"`.
//!
//! Backed by:
//!   Windows: Windows Credential Manager
//!   macOS:   Keychain
//!   Linux:   libsecret / SecretService
//!
//! Secrets too long for a single credential (see [`MAX_CHUNK_UTF16_UNITS`]) are
//! transparently split across `{provider_id}:chunk:{i}` entries; callers of
//! `set`/`get` never see the difference.

use anyhow::{Context, Result};
use keyring::Entry;

/// Longest secret a single credential can hold, in UTF-16 units.
///
/// Windows Credential Manager caps a credential blob at
/// `CRED_MAX_CREDENTIAL_BLOB_SIZE` (2560 bytes) and the keyring crate stores
/// passwords there as UTF-16, so 1280 units is the hard ceiling — anything
/// longer is rejected before the OS is even called. ChatGPT/Codex OAuth access
/// tokens are ~1900-character JWTs, so they must be split across several
/// credentials. 1200 leaves headroom under the cap.
const MAX_CHUNK_UTF16_UNITS: usize = 1200;

/// Written to the primary entry in place of a split secret, followed by the
/// chunk count. Chunks themselves live under [`chunk_key`].
const CHUNK_HEADER: &str = "aiterm:chunked:v1:";

/// Key of the `i`-th chunk (1-based) of a split secret. Matches the existing
/// sub-key convention (`{provider_id}:oauth_refresh`).
fn chunk_key(provider_id: &str, i: usize) -> String {
    format!("{provider_id}:chunk:{i}")
}

/// Split a secret into pieces that each fit a single credential.
///
/// Returns one piece — the whole secret — when no split is needed, so short
/// secrets (every API key, and Anthropic/Google tokens) stay stored verbatim
/// and existing entries keep reading back unchanged. Splits land on `char`
/// boundaries, and surrogate pairs count as the 2 UTF-16 units they occupy.
fn split_secret(secret: &str) -> Vec<&str> {
    if secret.encode_utf16().count() <= MAX_CHUNK_UTF16_UNITS {
        return vec![secret];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut units = 0;
    for (idx, ch) in secret.char_indices() {
        if units + ch.len_utf16() > MAX_CHUNK_UTF16_UNITS {
            chunks.push(&secret[start..idx]);
            start = idx;
            units = 0;
        }
        units += ch.len_utf16();
    }
    chunks.push(&secret[start..]);
    chunks
}

pub struct SecretStore {
    service: String,
}

impl SecretStore {
    pub fn new() -> Self {
        Self { service: "aiterm".into() }
    }

    /// Store (or overwrite) the API key for a provider.
    pub fn set(&self, provider_id: &str, secret: &str) -> Result<()> {
        let chunks = split_secret(secret);
        // Clear whatever the previous value left behind: a now-unsplit secret
        // orphans every chunk, a shorter split one orphans the tail.
        let stale_from = if let [whole] = chunks[..] {
            self.write_entry(provider_id, whole)?;
            1
        } else {
            // Chunks first, header last: a failure part-way through never
            // leaves a header pointing at chunks that don't exist yet.
            for (i, chunk) in chunks.iter().enumerate() {
                self.write_entry(&chunk_key(provider_id, i + 1), chunk)?;
            }
            self.write_entry(provider_id, &format!("{CHUNK_HEADER}{}", chunks.len()))?;
            chunks.len() + 1
        };
        self.delete_chunks_from(provider_id, stale_from);
        Ok(())
    }

    /// Retrieve the API key for a provider. Returns `None` if not found.
    pub fn get(&self, provider_id: &str) -> Result<Option<String>> {
        let Some(value) = self.read_entry(provider_id)? else {
            return Ok(None);
        };
        let Some(count) = value.strip_prefix(CHUNK_HEADER) else {
            return Ok(Some(value));
        };
        let count: usize = count
            .parse()
            .with_context(|| format!("bad chunk header for {provider_id}"))?;
        let mut secret = String::new();
        for i in 1..=count {
            let part = self.read_entry(&chunk_key(provider_id, i))?.ok_or_else(|| {
                anyhow::anyhow!("missing keychain chunk {i}/{count} for {provider_id}")
            })?;
            secret.push_str(&part);
        }
        Ok(Some(secret))
    }

    /// Remove the API key for a provider. Ignores "not found" errors.
    pub fn delete(&self, provider_id: &str) -> Result<()> {
        self.delete_chunks_from(provider_id, 1);
        self.delete_entry(provider_id)
    }

    /// Returns `true` if a secret exists for this provider (does not return the value).
    pub fn has(&self, provider_id: &str) -> bool {
        matches!(self.get(provider_id), Ok(Some(_)))
    }

    /// Delete chunk `from` onwards, stopping at the first one that's absent.
    /// Chunks are always written as a contiguous 1..n run, so this clears both
    /// a whole split secret and the leftovers of a previously longer value.
    fn delete_chunks_from(&self, provider_id: &str, from: usize) {
        for i in from.. {
            let key = chunk_key(provider_id, i);
            if !matches!(self.read_entry(&key), Ok(Some(_))) {
                break;
            }
            let _ = self.delete_entry(&key);
        }
    }

    fn write_entry(&self, key: &str, value: &str) -> Result<()> {
        self.entry(key)?
            .set_password(value)
            // Inline the keyring error the way the readers below do: callers
            // render this with `{e}`, and anyhow's `{e}` prints only the
            // outermost context — a bare `with_context` here hid *why* the
            // write failed (that's what made the Codex token-length failure
            // read as an unexplained "writing keychain entry" error).
            .map_err(|e| anyhow::anyhow!("keychain write error for {key}: {e} [{e:?}]"))
    }

    fn read_entry(&self, key: &str) -> Result<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => {
                // 「這一筆讀不到」與「根本沒有可用的憑證儲存區」是兩件事，前者要
                // 讓使用者去看鑰匙圈授權，後者其實等於「什麼都沒設定過」——對無頭
                // Linux 或 CI 說「讀不到已儲存的憑證」是新的誤導。
                //
                // 不去硬編各平台的錯誤碼（會過期、也各家不同），改用探測：拿一個
                // 一定不存在的 key 去讀，回 NoEntry 就代表儲存區是好的。
                if self.store_unavailable() {
                    log::warn!("憑證儲存區不可用（{e}）——視同尚未設定");
                    return Ok(None);
                }
                Err(anyhow::anyhow!("keychain read error for {key}: {e}"))
            }
        }
    }

    /// 憑證儲存區本身是否不可用（而不是某一筆讀不到）。
    fn store_unavailable(&self) -> bool {
        match Entry::new(&self.service, "__aiterm_probe_does_not_exist__") {
            Ok(probe) => !matches!(probe.get_password(), Err(keyring::Error::NoEntry)),
            Err(_) => true,
        }
    }

    fn delete_entry(&self, key: &str) -> Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("keychain delete error for {key}: {e}")),
        }
    }

    fn entry(&self, key: &str) -> Result<Entry> {
        Entry::new(&self.service, key)
            .with_context(|| format!("opening keychain entry for {key}"))
    }
}

impl Default for SecretStore {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests touch the real OS keychain. They are marked `#[ignore]`
    /// so they don't run in CI environments that may not have a keychain.
    /// Run manually with: `cargo test secret -- --ignored`

    #[test]
    #[ignore = "requires OS keychain"]
    fn set_get_delete_roundtrip() {
        let store = SecretStore::new();
        let id = "aiterm-test-provider-roundtrip";

        store.set(id, "super-secret-key").unwrap();
        assert_eq!(store.get(id).unwrap(), Some("super-secret-key".into()));
        assert!(store.has(id));

        store.delete(id).unwrap();
        assert_eq!(store.get(id).unwrap(), None);
        assert!(!store.has(id));
    }

    #[test]
    fn short_secret_is_not_split() {
        assert_eq!(split_secret("sk-ant-oat01-short-token"), vec!["sk-ant-oat01-short-token"]);
    }

    #[test]
    fn codex_sized_secret_splits_into_windows_safe_chunks() {
        // A real ChatGPT/Codex access token is ~1900 chars — 3800 bytes as
        // UTF-16, well over the 2560-byte Windows credential blob cap.
        let token = format!("header.{}.signature", "p".repeat(1875));
        let chunks = split_secret(&token);
        assert!(chunks.len() > 1, "a 1900-char token must be split");
        for chunk in &chunks {
            assert!(
                chunk.encode_utf16().count() * 2 <= 2560,
                "chunk of {} UTF-16 units exceeds the Windows blob cap",
                chunk.encode_utf16().count()
            );
        }
        assert_eq!(chunks.concat(), token);
    }

    #[test]
    fn split_keeps_multibyte_chars_intact() {
        // BMP chars are 1 UTF-16 unit, emoji are 2 (surrogate pair) — neither
        // may be cut in half, and both must respect the byte cap.
        for secret in ["鑰".repeat(1500), "🔑".repeat(900)] {
            let chunks = split_secret(&secret);
            assert!(chunks.len() > 1);
            for chunk in &chunks {
                assert!(chunk.encode_utf16().count() * 2 <= 2560);
            }
            assert_eq!(chunks.concat(), secret);
        }
    }

    #[test]
    #[ignore = "requires OS keychain"]
    fn long_secret_roundtrip() {
        let store = SecretStore::new();
        let id = "aiterm-test-provider-long";
        let token = format!("header.{}.signature", "p".repeat(1875));

        store.set(id, &token).unwrap();
        assert_eq!(store.get(id).unwrap(), Some(token));

        // Overwriting with a short value must not leave orphaned chunks behind.
        store.set(id, "short").unwrap();
        assert_eq!(store.get(id).unwrap(), Some("short".into()));
        assert!(!store.has(&chunk_key(id, 1)));

        store.delete(id).unwrap();
        assert_eq!(store.get(id).unwrap(), None);
    }

    #[test]
    #[ignore = "requires OS keychain"]
    fn delete_removes_every_chunk() {
        let store = SecretStore::new();
        let id = "aiterm-test-provider-chunk-cleanup";
        store.set(id, &format!("header.{}.signature", "p".repeat(1875))).unwrap();
        assert!(store.has(&chunk_key(id, 1)));

        store.delete(id).unwrap();
        assert_eq!(store.get(id).unwrap(), None);
        assert!(!store.has(&chunk_key(id, 1)));
        assert!(!store.has(&chunk_key(id, 2)));
    }

    #[test]
    #[ignore = "requires OS keychain"]
    fn delete_missing_is_ok() {
        let store = SecretStore::new();
        // Should not error even if the entry doesn't exist.
        store.delete("aiterm-nonexistent-provider-xyz").unwrap();
    }
}
