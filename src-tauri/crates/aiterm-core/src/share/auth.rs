//! CLI host 的預共享金鑰互證。
//!
//! GUI 主控端的身分保證來自「觀看端唸出 4 位 SAS、主控端的人核對」——那需要
//! 一個人在旁邊。headless 的 CLI host 沒有這個人，所以改用一組長期金鑰。
//!
//! **金鑰不上線。** 兩端各自對「自己那條 TLS 連線的 exporter material」做
//! HMAC，只把結果送出去。中間人終止 TLS 之後手上是兩條不同的連線、兩份不同
//! 的 exporter，所以它既算不出正確的證明，原封轉發也不成立。
//!
//! **互證是必要的，不是加分。** 只驗觀看端的話，中間人雖然偽造不出觀看端的
//! 證明，但它可以乾脆自己扮演主控端：直接回 `Granted`、餵假畫面、收走使用者
//! 打的每一個鍵。觀看端不驗憑證（見 `share::viewer` 的
//! `SasIsTheOnlyIdentityCheck`），所以「對面真的握有金鑰」必須由主控端那份
//! 證明提供。

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// 金鑰長度。32 bytes = 256 bit，暴力搜尋不成立。
pub const KEY_LEN: usize = 32;

/// 觀看端證明的 domain separator。
const VIEWER_LABEL: &[u8] = b"aiterm-viewer-v1";
/// 主控端證明的 domain separator。
///
/// **兩個方向必須用不同的 label。** 相同的話，中間人可以把主控端送來的證明
/// 原封當成觀看端的證明送回去（反射攻擊），不需要知道金鑰就能通過。
const HOST_LABEL: &[u8] = b"aiterm-host-v1";

fn proof(key: &[u8], label: &[u8], exporter: &[u8]) -> String {
    let mut mac = <Hmac<Sha256>>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(label);
    mac.update(exporter);
    super::tls::hex_of(&mac.finalize().into_bytes())
}

fn verify(key: &[u8], label: &[u8], exporter: &[u8], candidate: &str) -> bool {
    let Some(bytes) = super::tls::decode_hex(candidate) else { return false };
    let mut mac = <Hmac<Sha256>>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(label);
    mac.update(exporter);
    // `verify_slice` 是 constant-time 的，不要換成 `==`。
    mac.verify_slice(&bytes).is_ok()
}

/// 觀看端送給主控端的證明。
pub fn viewer_proof(key: &[u8], exporter: &[u8]) -> String {
    proof(key, VIEWER_LABEL, exporter)
}

/// 主控端驗證觀看端的證明。
pub fn verify_viewer_proof(key: &[u8], exporter: &[u8], candidate: &str) -> bool {
    verify(key, VIEWER_LABEL, exporter, candidate)
}

/// 主控端送給觀看端的證明。
pub fn host_proof(key: &[u8], exporter: &[u8]) -> String {
    proof(key, HOST_LABEL, exporter)
}

/// 觀看端驗證主控端的證明。
pub fn verify_host_proof(key: &[u8], exporter: &[u8], candidate: &str) -> bool {
    verify(key, HOST_LABEL, exporter, candidate)
}

/// 產生一組新金鑰。
pub fn generate_key() -> [u8; KEY_LEN] {
    use rand::RngCore;
    let mut key = [0u8; KEY_LEN];
    rand::rng().fill_bytes(&mut key);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
    const OTHER_KEY: &[u8] = b"fedcba9876543210fedcba9876543210";
    const EXPORTER: &[u8] = b"exporter-material-of-connection-1";
    const OTHER_EXPORTER: &[u8] = b"exporter-material-of-connection-2";

    #[test]
    fn a_correct_viewer_proof_verifies() {
        let p = viewer_proof(KEY, EXPORTER);
        assert!(verify_viewer_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_correct_host_proof_verifies() {
        let p = host_proof(KEY, EXPORTER);
        assert!(verify_host_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn the_wrong_key_is_rejected() {
        let p = viewer_proof(OTHER_KEY, EXPORTER);
        assert!(!verify_viewer_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_proof_from_another_connection_is_rejected() {
        // 這是整個機制的核心：中間人終止 TLS 之後，它跟觀看端那條連線的
        // exporter 跟它跟主控端那條連線的 exporter 不同，所以原封轉發不成立。
        // 這條測試若壞了，防中間人保證整個歸零而不會有任何其他徵兆。
        let p = viewer_proof(KEY, OTHER_EXPORTER);
        assert!(!verify_viewer_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_host_proof_cannot_be_replayed_as_a_viewer_proof() {
        // 反射攻擊：把主控端送來的證明原封當成觀看端的證明送回去。
        // 兩個方向用相同的 label 就會通過——那不需要知道金鑰。
        let p = host_proof(KEY, EXPORTER);
        assert!(!verify_viewer_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_viewer_proof_cannot_be_replayed_as_a_host_proof() {
        let p = viewer_proof(KEY, EXPORTER);
        assert!(!verify_host_proof(KEY, EXPORTER, &p));
    }

    #[test]
    fn a_malformed_proof_is_rejected_rather_than_panicking() {
        assert!(!verify_viewer_proof(KEY, EXPORTER, "not-hex"));
        assert!(!verify_viewer_proof(KEY, EXPORTER, ""));
        assert!(!verify_viewer_proof(KEY, EXPORTER, "ab"));
    }

    #[test]
    fn two_generated_keys_differ() {
        assert_ne!(generate_key(), generate_key());
    }
}
