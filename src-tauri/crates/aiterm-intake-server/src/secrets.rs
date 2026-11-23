//! 檔案式祕密儲存（server 沒有系統鑰匙圈）：`secrets.key` 放 32 byte 隨機金鑰，
//! 祕密用 AES-256-GCM 加密後存成 `secrets.json`。金鑰檔權限 0600。
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use aiterm_intake_core::host::Secrets;
use base64::Engine;
use parking_lot::Mutex;
use rand::RngCore;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct FileSecrets {
    key: [u8; 32],
    path: PathBuf,
    lock: Mutex<()>,
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// 金鑰檔不存在就產生；存在就讀（不覆蓋）。
pub fn load_or_create_key(path: &Path) -> std::io::Result<[u8; 32]> {
    if let Ok(raw) = std::fs::read(path) {
        if raw.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&raw);
            return Ok(k);
        }
    }
    let mut k = [0u8; 32];
    rand::rng().fill_bytes(&mut k);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, k)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(k)
}

impl FileSecrets {
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        let key = load_or_create_key(&dir.join("secrets.key"))?;
        Ok(Self { key, path: dir.join("secrets.json"), lock: Mutex::new(()) })
    }

    fn read_map(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(&self.path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn write_map(&self, m: &BTreeMap<String, String>) -> Result<(), String> {
        let json = serde_json::to_string(m).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600)).ok();
        }
        Ok(())
    }

    fn seal(&self, plain: &str) -> Result<String, String> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(|e| e.to_string())?;
        let mut nonce = [0u8; 12];
        rand::rng().fill_bytes(&mut nonce);
        let ct = cipher.encrypt(Nonce::from_slice(&nonce), plain.as_bytes()).map_err(|e| e.to_string())?;
        let mut out = nonce.to_vec();
        out.extend(ct);
        Ok(b64().encode(out))
    }

    fn open_sealed(&self, sealed: &str) -> Option<String> {
        let raw = b64().decode(sealed).ok()?;
        if raw.len() < 13 {
            return None;
        }
        let cipher = Aes256Gcm::new_from_slice(&self.key).ok()?;
        let pt = cipher.decrypt(Nonce::from_slice(&raw[..12]), &raw[12..]).ok()?;
        String::from_utf8(pt).ok()
    }
}

impl Secrets for FileSecrets {
    fn get(&self, id: &str) -> Option<String> {
        let _g = self.lock.lock();
        self.read_map().get(id).and_then(|s| self.open_sealed(s))
    }

    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        let _g = self.lock.lock();
        let mut m = self.read_map();
        m.insert(id.to_string(), self.seal(value)?);
        self.write_map(&m)
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        let _g = self.lock.lock();
        let mut m = self.read_map();
        m.remove(id);
        self.write_map(&m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_and_are_not_stored_in_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let s = FileSecrets::open(dir.path()).unwrap();
        assert_eq!(s.get("line"), None);
        s.set("line", "SUPER-SECRET-TOKEN").unwrap();
        assert_eq!(s.get("line").as_deref(), Some("SUPER-SECRET-TOKEN"));
        let on_disk = std::fs::read_to_string(dir.path().join("secrets.json")).unwrap();
        assert!(!on_disk.contains("SUPER-SECRET-TOKEN"), "{on_disk}");
        s.delete("line").unwrap();
        assert_eq!(s.get("line"), None);
    }

    #[test]
    fn a_second_open_reuses_the_key_and_a_wrong_key_reads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        FileSecrets::open(dir.path()).unwrap().set("a", "1").unwrap();
        assert_eq!(FileSecrets::open(dir.path()).unwrap().get("a").as_deref(), Some("1"), "金鑰不可被覆蓋");
        std::fs::write(dir.path().join("secrets.key"), [9u8; 32]).unwrap();
        assert_eq!(FileSecrets::open(dir.path()).unwrap().get("a"), None, "換了金鑰就解不開");
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        FileSecrets::open(dir.path()).unwrap();
        let mode = std::fs::metadata(dir.path().join("secrets.key")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
