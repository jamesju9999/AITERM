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
use serde::{Deserialize, Serialize};

use crate::config::types::{DbConnection, DbType};

const FORMAT_TAG: &str = "aiterm-db-export";

// Argon2id 參數，採 OWASP 建議值（m=19 MiB, t=2, p=1）。
const ARGON2_M_COST: u32 = 19456;
const ARGON2_T_COST: u32 = 2;
const ARGON2_P_COST: u32 = 1;

/// 匯出檔的明文 header。刻意不含任何連線資訊——所有連線資料都在
/// `data` 的密文裡。留這層明文是為了讓「檔案不對」和「passphrase 錯誤」
/// 成為兩種可分辨的錯誤，並讓版本檢查能在解密前完成。
#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub format: String,
    pub version: u32,
    pub kdf: KdfParams,
    pub cipher: String,
    pub nonce: String,
    pub data: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KdfParams {
    pub alg: String,
    pub salt: String,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

/// 密文解開後的內容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportPayload {
    pub connections: Vec<ExportedConnection>,
}

/// 匯出檔中的單筆連線。與 `DbConnection` 的差別只在多一個 `password`——
/// `DbConnection` 的密碼存在 Keychain，不在設定檔裡。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedConnection {
    pub id: String,
    pub name: String,
    pub db_type: DbType,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(default)]
    pub default_schema: Option<String>,
    /// 空字串代表這筆本來就沒有密碼（例如 SQLite），
    /// 匯入時不會拿它去覆寫既有的 Keychain 內容。
    #[serde(default)]
    pub password: String,
}

/// 目前支援的最高格式版本。讀到比這個大的檔案一律拒絕。
pub const EXPORT_VERSION: u32 = 1;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

/// 匯入失敗的原因。變體名稱會以 `to_string()` 送到前端當作錯誤碼，
/// 由前端對應到 i18n 字串——所以這些字串是介面的一部分，不要隨意改。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImportError {
    #[error("not_an_export_file")]
    NotAnExportFile,
    #[error("unsupported_version")]
    UnsupportedVersion,
    #[error("wrong_passphrase")]
    WrongPassphrase,
    #[error("unsupported_kdf")]
    UnsupportedKdf,
}

fn derive_key(passphrase: &str, salt: &[u8], kdf: &KdfParams) -> Result<[u8; 32], ImportError> {
    if kdf.alg != "argon2id" {
        return Err(ImportError::UnsupportedKdf);
    }
    let params = Params::new(kdf.m_cost, kdf.t_cost, kdf.p_cost, Some(32))
        .map_err(|_| ImportError::UnsupportedKdf)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|_| ImportError::UnsupportedKdf)?;
    Ok(key)
}

/// 把 payload 加密成一份完整的匯出檔位元組。
pub fn encrypt_payload(payload: &ExportPayload, passphrase: &str) -> anyhow::Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);

    let kdf = KdfParams {
        alg: "argon2id".into(),
        salt: B64.encode(salt),
        m_cost: ARGON2_M_COST,
        t_cost: ARGON2_T_COST,
        p_cost: ARGON2_P_COST,
    };
    let key = derive_key(passphrase, &salt, &kdf)
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;

    let plaintext = serde_json::to_vec(payload)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;

    let envelope = Envelope {
        format: FORMAT_TAG.into(),
        version: EXPORT_VERSION,
        kdf,
        cipher: "aes-256-gcm".into(),
        nonce: B64.encode(nonce),
        data: B64.encode(ciphertext),
    };
    Ok(serde_json::to_vec_pretty(&envelope)?)
}

/// 解開一份匯出檔。GCM 的驗證標籤讓「passphrase 錯誤」與「檔案遭竄改」
/// 都表現為 `WrongPassphrase`——兩者對使用者而言是同一件事：這份檔案
/// 配這組密碼打不開。
pub fn decrypt_payload(bytes: &[u8], passphrase: &str) -> Result<ExportPayload, ImportError> {
    let envelope = check_envelope(bytes)?;
    if envelope.cipher != "aes-256-gcm" {
        return Err(ImportError::UnsupportedKdf);
    }
    let salt = B64
        .decode(&envelope.kdf.salt)
        .map_err(|_| ImportError::NotAnExportFile)?;
    let nonce = B64
        .decode(&envelope.nonce)
        .map_err(|_| ImportError::NotAnExportFile)?;
    let ciphertext = B64
        .decode(&envelope.data)
        .map_err(|_| ImportError::NotAnExportFile)?;
    if nonce.len() != NONCE_LEN {
        return Err(ImportError::NotAnExportFile);
    }

    let key = derive_key(passphrase, &salt, &envelope.kdf)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| ImportError::WrongPassphrase)?;

    serde_json::from_slice(&plaintext).map_err(|_| ImportError::WrongPassphrase)
}

/// 只讀明文 header：確認這是 AITerm 的匯出檔，且版本在支援範圍內。
/// 完全不碰密文，所以可以在要求使用者輸入 passphrase 之前呼叫。
fn check_envelope(bytes: &[u8]) -> Result<Envelope, ImportError> {
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|_| ImportError::NotAnExportFile)?;
    if envelope.format != FORMAT_TAG {
        return Err(ImportError::NotAnExportFile);
    }
    // 高版本一律擋下。v1 無法分辨 v2 是「只新增欄位」還是「改了既有
    // 欄位的語意」，硬讀會安靜地匯入錯誤資料。反之低版本必須支援。
    if envelope.version > EXPORT_VERSION {
        return Err(ImportError::UnsupportedVersion);
    }
    Ok(envelope)
}

/// 對外的檔案檢查入口，回傳檔案的格式版本。
pub fn check_import_file(bytes: &[u8]) -> Result<u32, ImportError> {
    check_envelope(bytes).map(|e| e.version)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    New,
    Overwrite,
}

/// 匯出檔中某一筆該如何套用。
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    /// 在匯出檔 `connections` 陣列中的索引。
    pub index: usize,
    pub kind: ConflictKind,
    /// Overwrite 時是現有那筆的 id；New 時是匯出檔裡的 id。
    pub target_id: String,
    /// Overwrite 時填現有那筆的名稱，供 UI 顯示「覆蓋（原：xxx）」。
    pub existing_name: Option<String>,
}

/// 先比 id，沒中再比名稱（trim + 忽略大小寫）。比名稱是為了處理
/// 「同事在自己機器上手動建了同名連線」——id 不同但實際是同一筆，
/// 不比名稱的話會變成兩筆同名連線並存。
pub fn resolve_conflicts(
    exported: &[ExportedConnection],
    existing: &[DbConnection],
) -> Vec<Resolution> {
    exported
        .iter()
        .enumerate()
        .map(|(index, e)| {
            let hit = existing.iter().find(|x| x.id == e.id).or_else(|| {
                let key = e.name.trim().to_lowercase();
                existing.iter().find(|x| x.name.trim().to_lowercase() == key)
            });
            match hit {
                Some(x) => Resolution {
                    index,
                    kind: ConflictKind::Overwrite,
                    target_id: x.id.clone(),
                    existing_name: Some(x.name.clone()),
                },
                None => Resolution {
                    index,
                    kind: ConflictKind::New,
                    target_id: e.id.clone(),
                    existing_name: None,
                },
            }
        })
        .collect()
}

// ---- Tauri 接線 ----
// 以下只負責讀寫檔案並接上 ConfigStore／SecretStore；所有邏輯都在上面的純函式。

use std::sync::Arc;
use tauri::State;

use crate::commands::db::secret_key;
use crate::config::ConfigStore;
use crate::secret::SecretStore;

/// 只檢查明文 header，讓 UI 能在要求輸入 passphrase 之前就擋掉不合的檔案。
#[tauri::command]
pub async fn db_check_import_file(path: String) -> Result<u32, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("io_error: {e}"))?;
    check_import_file(&bytes).map_err(|e| e.to_string())
}

/// 把選取的連線加密寫到 `path`，回傳實際匯出的筆數。
#[tauri::command]
pub async fn db_export_connections(
    path: String,
    ids: Vec<String>,
    passphrase: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<usize, String> {
    let connections: Vec<ExportedConnection> = config
        .get()
        .db_connections
        .into_iter()
        .filter(|c| ids.contains(&c.id))
        .map(|c| ExportedConnection {
            // Keychain 讀不到就以空密碼匯出。匯入端會把空字串視為
            // 「這筆本來就沒有密碼」，不會拿它去清掉既有密碼。
            password: secrets
                .get(&secret_key(&c.id))
                .ok()
                .flatten()
                .unwrap_or_default(),
            id: c.id,
            name: c.name,
            db_type: c.db_type,
            host: c.host,
            port: c.port,
            database: c.database,
            username: c.username,
            default_schema: c.default_schema,
        })
        .collect();

    let count = connections.len();
    let bytes =
        encrypt_payload(&ExportPayload { connections }, &passphrase).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| format!("io_error: {e}"))?;
    Ok(count)
}
