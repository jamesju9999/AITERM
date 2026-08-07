//! 資料庫連線匯出／匯入的檔案格式與加解密。
//!
//! 本檔案前半的純函式不依賴 Tauri，可單獨測試；後半的
//! `#[tauri::command]` 只負責讀寫檔案與接上 ConfigStore／SecretStore。

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// 目前支援的最高格式版本。讀到比這個大的檔案一律拒絕。
pub const EXPORT_VERSION: u32 = 1;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

/// 只用來確認相依的 API 形狀正確；Task 2 會被真正的加密取代。
pub fn crypto_smoke_test() -> Vec<u8> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);

    let params = Params::new(19456, 2, 1, Some(32)).expect("valid argon2 params");
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(b"pw", &salt, &mut key)
        .expect("argon2 hashes into a 32-byte key");

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), b"hello".as_ref())
        .expect("encrypt succeeds");
    B64.encode(ct).into_bytes()
}
