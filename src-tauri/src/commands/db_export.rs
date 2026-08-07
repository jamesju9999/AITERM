//! 資料庫連線匯出／匯入的檔案格式與加解密。
//!
//! 本檔案前半的純函式不依賴 Tauri，可單獨測試；後半的
//! `#[tauri::command]` 只負責讀寫檔案與接上 ConfigStore／SecretStore。

use std::collections::HashSet;

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

// 這三個參數來自匯入檔。argon2 的 `Params::new` 只擋到 u32::MAX，而
// `hash_password_into` 內部是不可失敗配置——配不到記憶體就 abort 整個
// 行程，使用者連錯誤訊息都看不到。m_cost 以 KiB 計，1 GiB 對合法檔案
// 綽綽有餘（我們自己寫出去的是 19 MiB），對惡意檔案則把記憶體壓在
// 可承受範圍內。
const ARGON2_MAX_M_COST: u32 = 1024 * 1024; // 1 GiB
const ARGON2_MAX_T_COST: u32 = 16;
const ARGON2_MAX_P_COST: u32 = 4;

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

fn check_kdf_bounds(kdf: &KdfParams) -> Result<(), ImportError> {
    if kdf.m_cost > ARGON2_MAX_M_COST
        || kdf.t_cost > ARGON2_MAX_T_COST
        || kdf.p_cost > ARGON2_MAX_P_COST
    {
        return Err(ImportError::UnsupportedKdf);
    }
    Ok(())
}

/// 版本閘門只解析這兩個欄位。刻意獨立於 `Envelope`：若用完整結構解，
/// 一份改動過 header 結構的 v2 檔案會先失敗在 serde，使用者拿到的是
/// 「這不是匯出檔」而不是「請先更新 AITerm」——版本欄位就白設了。
/// `format` 與 `version` 是這個格式唯一承諾永遠不變的兩個欄位。
#[derive(Deserialize)]
struct VersionGate {
    format: String,
    /// 用 `Value` 而非 `u32`：未來版本若把它寫成字串或超出 u32，
    /// 應該回報「版本不支援」，而不是「這不是匯出檔」。
    version: serde_json::Value,
}

/// 只讀明文 header：確認這是 AITerm 的匯出檔，且版本在支援範圍內。
/// 完全不碰密文，所以可以在要求使用者輸入 passphrase 之前呼叫。
fn check_envelope(bytes: &[u8]) -> Result<Envelope, ImportError> {
    let gate: VersionGate =
        serde_json::from_slice(bytes).map_err(|_| ImportError::NotAnExportFile)?;
    if gate.format != FORMAT_TAG {
        return Err(ImportError::NotAnExportFile);
    }
    // 認不得的版本表示法一律當成「比我們新」——我們無法理解它。
    let version = gate.version.as_u64().unwrap_or(u64::MAX);
    if version > EXPORT_VERSION as u64 {
        return Err(ImportError::UnsupportedVersion);
    }

    // 版本確定在支援範圍內，才用完整結構解。
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|_| ImportError::NotAnExportFile)?;
    check_kdf_bounds(&envelope.kdf)?;
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
    /// 目標已被同一份匯出檔中前面某一筆認領。照樣套用會把前一筆寫進去的
    /// 內容默默蓋掉，而回傳筆數還會把兩筆都算成成功——所以這種筆數直接
    /// 略過，並在預覽裡標示出來讓使用者知道為什麼。
    Duplicate,
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
///
/// 每個目標只能被認領一次。同一份匯出檔裡第二筆指向同一個目標的資料
/// 會被判為 `Duplicate`——若照樣套用，它會把前一筆剛寫進去的內容默默
/// 蓋掉，而 `ImportResult` 還會把兩筆都算成成功。逐筆誠實回報正是
/// 「不做 rollback」這個決策所倚賴的東西。
///
/// id／名稱比對的對象不只是匯入前的 `existing` 快照，還包含同一份
/// 匯出檔裡「前面已經處理過」的那些筆——否則兩筆全新、彼此同名的
/// 連線（誰都沒命中 `existing`）會被雙雙判成 New，恰好是名稱比對這條
/// 規則原本想擋下的情況。
pub fn resolve_conflicts(
    exported: &[ExportedConnection],
    existing: &[DbConnection],
) -> Vec<Resolution> {
    let mut claimed: HashSet<String> = HashSet::new();
    // 從既有連線出發，每處理完一筆就把它的目標 id／名稱也併進來，
    // 讓後面的筆數可以比對到「前面這份檔案裡剛認領的目標」。
    let mut pool: Vec<(String, String)> =
        existing.iter().map(|x| (x.id.clone(), x.name.clone())).collect();

    exported
        .iter()
        .enumerate()
        .map(|(index, e)| {
            let hit = pool.iter().find(|(id, _)| *id == e.id).or_else(|| {
                let key = e.name.trim().to_lowercase();
                pool.iter().find(|(_, name)| name.trim().to_lowercase() == key)
            });
            let (target_id, existing_name) = match hit {
                Some((id, name)) => (id.clone(), Some(name.clone())),
                None => (e.id.clone(), None),
            };
            // `insert` 回傳 false 表示這個目標稍早已被認領。
            let kind = if !claimed.insert(target_id.clone()) {
                ConflictKind::Duplicate
            } else if hit.is_some() {
                ConflictKind::Overwrite
            } else {
                ConflictKind::New
            };
            // 只有真的會被套用的筆數才進池。Duplicate 不會套用，把它的
            // 名稱放進去會讓後面某筆比對到一個匯入後根本不存在的名稱，
            // 於是也被誤判成 Duplicate 而漏掉。
            if kind != ConflictKind::Duplicate {
                pool.push((target_id.clone(), e.name.clone()));
            }
            Resolution { index, kind, target_id, existing_name }
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

/// 匯入預覽的單筆。**刻意不含 password**——現有的 `DbConnectionInfo`
/// 就從不外送密碼，這裡維持同樣的界線：明文密碼只在 Rust 內部流動。
/// 代價是套用時要再解密一次，換取密碼不跨 IPC 邊界。
#[derive(Debug, Serialize)]
pub struct ImportPreviewItem {
    /// 匯出檔裡的 id，前端用它當勾選的 key。
    pub id: String,
    pub name: String,
    pub db_type: DbType,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub conflict: ConflictKind,
    pub existing_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportFailure {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Default)]
pub struct ImportResult {
    pub added: usize,
    pub overwritten: usize,
    pub failures: Vec<ImportFailure>,
}

#[tauri::command]
pub async fn db_preview_import(
    path: String,
    passphrase: String,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<Vec<ImportPreviewItem>, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("io_error: {e}"))?;
    let payload = decrypt_payload(&bytes, &passphrase).map_err(|e| e.to_string())?;
    let existing = config.get().db_connections;

    Ok(resolve_conflicts(&payload.connections, &existing)
        .into_iter()
        .map(|r| {
            let e = &payload.connections[r.index];
            ImportPreviewItem {
                id: e.id.clone(),
                name: e.name.clone(),
                db_type: e.db_type,
                host: e.host.clone(),
                port: e.port,
                database: e.database.clone(),
                username: e.username.clone(),
                conflict: r.kind,
                existing_name: r.existing_name,
            }
        })
        .collect())
}

/// 套用勾選的項目。逐筆進行、不做全有全無——`ConfigStore` 沒有交易
/// 語意，硬做 rollback 需要自行實作快照與還原，而還原本身也可能失敗。
#[tauri::command]
pub async fn db_import_connections(
    path: String,
    passphrase: String,
    ids: Vec<String>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<ImportResult, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("io_error: {e}"))?;
    let payload = decrypt_payload(&bytes, &passphrase).map_err(|e| e.to_string())?;
    let existing = config.get().db_connections;
    let resolutions = resolve_conflicts(&payload.connections, &existing);

    let mut result = ImportResult::default();
    for r in resolutions {
        let e = &payload.connections[r.index];
        if !ids.contains(&e.id) {
            continue;
        }

        // 目標已被同一份檔案裡前面那筆認領。預覽已經標示過，這裡直接跳過——
        // 不計入任何一個計數器，`ImportResult` 才不會謊報。
        if r.kind == ConflictKind::Duplicate {
            continue;
        }

        let conn = DbConnection {
            id: r.target_id.clone(),
            name: e.name.clone(),
            db_type: e.db_type,
            host: e.host.clone(),
            port: e.port,
            database: e.database.clone(),
            username: e.username.clone(),
            default_schema: e.default_schema.clone(),
        };
        let applied = match r.kind {
            ConflictKind::Overwrite => config.update_db_connection(conn),
            ConflictKind::New => config.add_db_connection(conn),
            ConflictKind::Duplicate => unreachable!("Duplicate 在迴圈開頭就 continue 了"),
        };
        if let Err(err) = applied {
            result.failures.push(ImportFailure {
                name: e.name.clone(),
                reason: err.to_string(),
            });
            continue;
        }
        match r.kind {
            ConflictKind::Overwrite => result.overwritten += 1,
            ConflictKind::New => result.added += 1,
            ConflictKind::Duplicate => unreachable!("Duplicate 在迴圈開頭就 continue 了"),
        }

        // 空密碼代表匯出時這筆本來就沒有密碼，不能拿它清掉既有的。
        if !e.password.is_empty() {
            if let Err(err) = secrets.set(&secret_key(&r.target_id), &e.password) {
                result.failures.push(ImportFailure {
                    name: e.name.clone(),
                    reason: format!("secret_write_failed: {err}"),
                });
            }
        }
    }
    Ok(result)
}
