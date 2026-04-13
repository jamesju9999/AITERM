//! OS keychain wrapper.
//!
//! All secrets are stored under the service name `"aiterm"` with the
//! provider id as the username, e.g. `"aiterm" / "claude-sonnet"`.
//!
//! Backed by:
//!   Windows: Windows Credential Manager
//!   macOS:   Keychain
//!   Linux:   libsecret / SecretService

use anyhow::{Context, Result};
use keyring::Entry;

pub struct SecretStore {
    service: String,
}

impl SecretStore {
    pub fn new() -> Self {
        Self { service: "aiterm".into() }
    }

    /// Store (or overwrite) the API key for a provider.
    pub fn set(&self, provider_id: &str, secret: &str) -> Result<()> {
        Entry::new(&self.service, provider_id)
            .with_context(|| format!("opening keychain entry for {provider_id}"))?
            .set_password(secret)
            .with_context(|| format!("writing keychain entry for {provider_id}"))
    }

    /// Retrieve the API key for a provider. Returns `None` if not found.
    pub fn get(&self, provider_id: &str) -> Result<Option<String>> {
        let entry = Entry::new(&self.service, provider_id)
            .with_context(|| format!("opening keychain entry for {provider_id}"))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("keychain read error for {provider_id}: {e}")),
        }
    }

    /// Remove the API key for a provider. Ignores "not found" errors.
    pub fn delete(&self, provider_id: &str) -> Result<()> {
        let entry = Entry::new(&self.service, provider_id)
            .with_context(|| format!("opening keychain entry for {provider_id}"))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("keychain delete error for {provider_id}: {e}")),
        }
    }

    /// Returns `true` if a secret exists for this provider (does not return the value).
    pub fn has(&self, provider_id: &str) -> bool {
        matches!(self.get(provider_id), Ok(Some(_)))
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
    #[ignore = "requires OS keychain"]
    fn delete_missing_is_ok() {
        let store = SecretStore::new();
        // Should not error even if the entry doesn't exist.
        store.delete("aiterm-nonexistent-provider-xyz").unwrap();
    }
}
