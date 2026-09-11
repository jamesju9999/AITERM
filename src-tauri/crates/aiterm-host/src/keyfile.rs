//! 金鑰檔的讀取、產生與權限檢查。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use aiterm_core::share::auth::KEY_LEN;

/// 預設的金鑰檔位置。
pub fn default_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("找不到設定檔目錄")?;
    Ok(dir.join("aiterm-host").join("key"))
}

/// 讀出金鑰；檔案不存在就產生一組新的。
///
/// 回傳 `(金鑰, 是否為這次新產生的)`——呼叫端用第二個值決定要不要在啟動訊息
/// 裡特別提醒使用者「這是新金鑰，記得複製到 GUI」。
pub fn load_or_create(path: &Path) -> Result<(Vec<u8>, bool)> {
    if path.exists() {
        check_permissions(path)?;
        let hex = std::fs::read_to_string(path)
            .with_context(|| format!("讀不到金鑰檔 {}", path.display()))?;
        let key = aiterm_core::share::tls::decode_hex(hex.trim())
            .with_context(|| format!("金鑰檔 {} 的內容不是合法的 hex", path.display()))?;
        if key.len() != KEY_LEN {
            bail!("金鑰長度是 {} bytes，應該是 {KEY_LEN}：{}", key.len(), path.display());
        }
        return Ok((key, false));
    }

    let key = aiterm_core::share::auth::generate_key();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("建不出目錄 {}", parent.display()))?;
    }
    write_private(path, &aiterm_core::share::tls::hex_of(&key))
        .with_context(|| format!("寫不進金鑰檔 {}", path.display()))?;
    Ok((key.to_vec(), true))
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    // 先建成 0600 再寫，不要先寫完再 chmod——那之間有一個任何人都讀得到的窗口。
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

/// Unix 上檢查權限；比 0600 寬就拒絕，比照 ssh。
///
/// Windows 不做這個檢查——ACL 的語意跟 mode bits 不同，硬套會得到一個既
/// 擋不住真正的問題、又會誤擋正常設定的檢查。改為在啟動訊息裡明確指出
/// 金鑰檔的位置。
#[cfg(unix)]
pub fn check_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!(
            "金鑰檔 {} 的權限是 {mode:o}，其他人讀得到。請執行：chmod 600 {}",
            path.display(),
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn check_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creating_a_key_produces_one_of_the_right_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("key");
        let (key, created) = load_or_create(&path).unwrap();
        assert!(created);
        assert_eq!(key.len(), KEY_LEN);
        assert!(path.exists());
    }

    #[test]
    fn reading_an_existing_key_returns_the_same_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        let (first, created_first) = load_or_create(&path).unwrap();
        let (second, created_second) = load_or_create(&path).unwrap();
        assert!(created_first);
        assert!(!created_second);
        assert_eq!(first, second, "重啟後金鑰必須不變，否則 GUI 存的連線會失效");
    }

    #[test]
    fn a_malformed_key_file_is_an_error_rather_than_a_silent_zero_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "not hex at all").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(load_or_create(&path).is_err());
    }

    #[test]
    fn a_key_of_the_wrong_length_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "abcd").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(load_or_create(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_world_readable_key_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        let (key, _) = load_or_create(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_or_create(&path).unwrap_err();
        assert!(
            err.to_string().contains("chmod 600"),
            "錯誤訊息要直接給出修法，got: {err}"
        );
        // 不是因為讀不出來才失敗——金鑰本身是好的。
        assert_eq!(key.len(), KEY_LEN);
    }

    #[cfg(unix)]
    #[test]
    fn a_newly_created_key_file_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        load_or_create(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "產生金鑰時就要是 0600，不能事後補 chmod");
    }
}
