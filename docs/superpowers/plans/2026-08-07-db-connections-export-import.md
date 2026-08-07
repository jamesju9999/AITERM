# 資料庫連線匯出／匯入 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓使用者把資料庫連線設定（含密碼）匯出成一份 passphrase 加密的 JSON 檔，並在另一台機器上勾選匯入。

**Architecture:** 新增 `src-tauri/src/commands/db_export.rs`，前半是不依賴 Tauri 的純函式（檔案格式、AES-256-GCM 加解密、衝突判定），後半是四個 `#[tauri::command]` 接線。前端把匯出／匯入 UI 放進新的 `src/components/Settings/DbConnectionTransfer.tsx`（兩個各自獨立的面板元件），`DatabaseConnectionsPage.tsx` 只負責加兩顆按鈕與切換顯示。明文密碼只在 Rust 內部流動，不跨 IPC。

**Tech Stack:** Rust（`aes-gcm` 0.10、`argon2` 0.5、serde_json、base64）、React 19 + TypeScript、Vitest + React Testing Library、`@tauri-apps/plugin-dialog`。

**設計依據：** `docs/superpowers/specs/2026-08-07-db-connections-export-import-design.md`

---

## 開始前的環境前提

`src-tauri/binaries/` 是 gitignored，而 `tauri-build` 的 `build.rs` 會在**編譯期**驗證 `externalBin` 的每個項目都存在於磁碟上。沒有先跑 setup script 的話，連 `cargo test` 都會失敗（不只是 `tauri:dev`）。

- [ ] **前置：確認 sidecar 二進位檔存在**

```bash
ls src-tauri/binaries/
```

若是空的或缺少 `uv`，先跑對應平台的 setup script（macOS：`bash scripts/setup-uv-mac.sh`），再繼續。

驗證：`cd src-tauri && cargo check` 應成功（第一次會編很久）。

---

## 檔案結構

**新增：**

| 檔案 | 職責 |
|---|---|
| `src-tauri/src/commands/db_export.rs` | 匯出檔格式、加解密、衝突判定（純函式）＋ 四個 Tauri 指令 |
| `src-tauri/tests/db_export.rs` | 純函式的整合測試 |
| `src/components/Settings/DbConnectionTransfer.tsx` | `DbExportPanel`、`DbImportPanel`、`translateDbTransferError` |
| `src/components/Settings/DbConnectionTransfer.test.tsx` | 前端測試 |

**修改：**

| 檔案 | 改動 |
|---|---|
| `src-tauri/Cargo.toml` | 新增 `aes-gcm`、`argon2` |
| `src-tauri/src/commands/mod.rs` | `pub mod db_export;` |
| `src-tauri/src/commands/db.rs:54` | `secret_key` 改為 `pub(crate)` |
| `src-tauri/src/lib.rs` | `use` 與 `generate_handler!` 註冊四個新指令 |
| `src/ipc/db.ts` | 四個 wrapper 與型別 |
| `src/lib/i18n.ts` | zh-TW／en 兩份新字串 |
| `src/components/Settings/DatabaseConnectionsPage.tsx` | 兩顆按鈕與面板切換 |

**設計偏離說明（相對於 spec）：** spec 列了 `rand` 作為新相依，實作改用 `aes_gcm::aead::OsRng`（`aead` crate 已再匯出 `rand_core`）。少一個相依，也避免 `rand` 與 `aes-gcm` 內部 `rand_core` 版本不一致。

---

### Task 1: 加入加密相依並確認 API 可用

先用一個最小的 smoke test 釘住 `aes-gcm` / `argon2` 的實際 API 形狀。這兩個 crate 跨大版本改過 API，先確認能編過，後面的任務才不會卡在相依問題上。

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/commands/db_export.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Test: `src-tauri/tests/db_export.rs`

- [ ] **Step 1: 加入相依**

在 `src-tauri/Cargo.toml` 的 `base64 = "0.22"`（第 38 行）後面加：

```toml
# 資料庫連線匯出檔的加密（passphrase → Argon2id → AES-256-GCM）
aes-gcm = "0.10"
argon2 = "0.5"
```

- [ ] **Step 2: 建立模組骨架**

建立 `src-tauri/src/commands/db_export.rs`：

```rust
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
```

在 `src-tauri/src/commands/mod.rs` 第 6 行 `pub mod db;` 後面加一行：

```rust
pub mod db_export;
```

- [ ] **Step 3: 寫測試**

建立 `src-tauri/tests/db_export.rs`：

```rust
//! 匯出／匯入純函式的測試。這些函式不碰 Tauri、不碰 Keychain，
//! 所以可以直接呼叫，不需要建立 AppHandle。

use aiterm_lib::commands::db_export::*;

#[test]
fn crypto_dependencies_have_the_expected_api() {
    let out = crypto_smoke_test();
    assert!(!out.is_empty());
}
```

- [ ] **Step 4: 執行測試**

```bash
cd src-tauri && cargo test --test db_export
```

Expected: PASS。若編譯失敗且訊息指向 `aes_gcm::aead::rand_core` 或 `OsRng` 路徑不存在，執行 `cargo tree -p aes-gcm` 確認 `aead` 版本，並依該版本文件調整 `use`——**不要**改成加入 `rand` crate 繞過。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/commands/db_export.rs src-tauri/src/commands/mod.rs src-tauri/tests/db_export.rs
git commit -m "feat(db-export): 加入 aes-gcm 與 argon2 相依"
```

---

### Task 2: 檔案格式型別

**Files:**
- Modify: `src-tauri/src/commands/db_export.rs`
- Test: `src-tauri/tests/db_export.rs`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/tests/db_export.rs` 加：

```rust
#[test]
fn envelope_serializes_with_the_documented_field_names() {
    let env = Envelope {
        format: "aiterm-db-export".into(),
        version: 1,
        kdf: KdfParams {
            alg: "argon2id".into(),
            salt: "c2FsdA==".into(),
            m_cost: 19456,
            t_cost: 2,
            p_cost: 1,
        },
        cipher: "aes-256-gcm".into(),
        nonce: "bm9uY2U=".into(),
        data: "ZGF0YQ==".into(),
    };
    let json = serde_json::to_value(&env).unwrap();
    assert_eq!(json["format"], "aiterm-db-export");
    assert_eq!(json["version"], 1);
    assert_eq!(json["kdf"]["alg"], "argon2id");
    assert_eq!(json["kdf"]["m_cost"], 19456);
    assert_eq!(json["cipher"], "aes-256-gcm");
}

#[test]
fn exported_connection_defaults_password_and_schema_when_absent() {
    let e: ExportedConnection = serde_json::from_str(
        r#"{"id":"a","name":"n","db_type":"sqlite","host":"/tmp/x.db",
            "port":0,"database":"","username":""}"#,
    )
    .unwrap();
    assert_eq!(e.password, "");
    assert_eq!(e.default_schema, None);
}
```

`src-tauri/tests/` 需要 `serde_json`——它已是 `src-tauri/Cargo.toml` 的一般相依（第 21 行），整合測試可直接使用。

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --test db_export
```

Expected: 編譯失敗，`cannot find type 'Envelope' in this scope`。

- [ ] **Step 3: 實作**

在 `db_export.rs` 的 `use` 之後、`crypto_smoke_test` 之前加：

```rust
use serde::{Deserialize, Serialize};

use crate::config::types::DbType;

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
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --test db_export
```

Expected: 3 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/db_export.rs src-tauri/tests/db_export.rs
git commit -m "feat(db-export): 定義匯出檔信封與 payload 型別"
```

---

### Task 3: 加解密 round-trip

**Files:**
- Modify: `src-tauri/src/commands/db_export.rs`
- Test: `src-tauri/tests/db_export.rs`

- [ ] **Step 1: 寫失敗的測試**

Task 1 的 smoke test 階梯已經用完，先把 `src-tauri/tests/db_export.rs` 裡的 `crypto_dependencies_have_the_expected_api` **整個刪掉**（Step 3 會把它測的 `crypto_smoke_test` 從實作移除，留著會編譯失敗）。

然後加（含一個共用的測試資料 helper，後續任務會重複用到）：

```rust
fn sample_payload() -> ExportPayload {
    ExportPayload {
        connections: vec![
            ExportedConnection {
                id: "id-db2".into(),
                name: "總行LBOTHODB".into(),
                db_type: aiterm_lib::config::types::DbType::Db2,
                host: "172.19.2.83".into(),
                port: 25000,
                database: "LBOTHODB".into(),
                username: "nuntio".into(),
                default_schema: Some("NUNTIO".into()),
                password: "s3cret".into(),
            },
            ExportedConnection {
                id: "id-sqlite".into(),
                name: "AITERM知識庫".into(),
                db_type: aiterm_lib::config::types::DbType::Sqlite,
                host: "/tmp/knowledge_base.db".into(),
                port: 0,
                database: String::new(),
                username: String::new(),
                default_schema: None,
                password: String::new(),
            },
        ],
    }
}

#[test]
fn encrypt_then_decrypt_restores_the_payload() {
    let payload = sample_payload();
    let bytes = encrypt_payload(&payload, "correct horse").unwrap();
    let back = decrypt_payload(&bytes, "correct horse").unwrap();
    assert_eq!(back, payload);
}

#[test]
fn the_ciphertext_contains_no_plaintext_connection_data() {
    let bytes = encrypt_payload(&sample_payload(), "pw").unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("172.19.2.83"));
    assert!(!text.contains("nuntio"));
    assert!(!text.contains("s3cret"));
    assert!(!text.contains("LBOTHODB"));
    // 明文 header 仍然要看得到，版本檢查才能在解密前做。
    assert!(text.contains("aiterm-db-export"));
}

#[test]
fn encrypting_the_same_payload_twice_gives_different_bytes() {
    let a = encrypt_payload(&sample_payload(), "pw").unwrap();
    let b = encrypt_payload(&sample_payload(), "pw").unwrap();
    assert_ne!(a, b, "salt 與 nonce 必須每次重新隨機產生");
}
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --test db_export
```

Expected: 編譯失敗，`cannot find function 'encrypt_payload'`。

- [ ] **Step 3: 實作**

把 `db_export.rs` 裡的 `crypto_smoke_test` **整個刪掉**（Task 1 的階梯已經用完），換成：

```rust
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
```

`check_envelope` 在 Task 4 實作。這一步先加一個最小版本讓它編得過，Task 4 會補上版本檢查：

```rust
fn check_envelope(bytes: &[u8]) -> Result<Envelope, ImportError> {
    serde_json::from_slice(bytes).map_err(|_| ImportError::NotAnExportFile)
}
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --test db_export
```

Expected: 5 passed（Task 2 的 2 個 + 本任務的 3 個；smoke test 已移除）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/db_export.rs src-tauri/tests/db_export.rs
git commit -m "feat(db-export): Argon2id + AES-256-GCM 加解密"
```

---

### Task 4: 格式與版本檢查（在解密之前）

**Files:**
- Modify: `src-tauri/src/commands/db_export.rs`
- Test: `src-tauri/tests/db_export.rs`

- [ ] **Step 1: 寫失敗的測試**

```rust
/// 把一份合法匯出檔的某個 header 欄位換掉，用來製造各種壞檔。
fn tweak_header(field: &str, value: serde_json::Value) -> Vec<u8> {
    let bytes = encrypt_payload(&sample_payload(), "pw").unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json[field] = value;
    serde_json::to_vec(&json).unwrap()
}

#[test]
fn a_wrong_passphrase_is_reported_as_such() {
    let bytes = encrypt_payload(&sample_payload(), "right").unwrap();
    assert_eq!(
        decrypt_payload(&bytes, "wrong").unwrap_err(),
        ImportError::WrongPassphrase
    );
}

#[test]
fn tampering_with_the_ciphertext_is_detected() {
    let bytes = encrypt_payload(&sample_payload(), "pw").unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut ct = base64_decode(json["data"].as_str().unwrap());
    ct[0] ^= 0x01; // 翻一個位元
    json["data"] = serde_json::Value::String(base64_encode(&ct));
    let tampered = serde_json::to_vec(&json).unwrap();

    assert_eq!(
        decrypt_payload(&tampered, "pw").unwrap_err(),
        ImportError::WrongPassphrase
    );
}

#[test]
fn arbitrary_json_is_not_an_export_file() {
    assert_eq!(
        check_import_file(br#"{"hello":"world"}"#).unwrap_err(),
        ImportError::NotAnExportFile
    );
    assert_eq!(
        check_import_file(b"not json at all").unwrap_err(),
        ImportError::NotAnExportFile
    );
}

#[test]
fn a_wrong_format_tag_is_rejected() {
    let bytes = tweak_header("format", serde_json::json!("some-other-tool"));
    assert_eq!(
        check_import_file(&bytes).unwrap_err(),
        ImportError::NotAnExportFile
    );
}

#[test]
fn a_newer_version_is_rejected() {
    let bytes = tweak_header("version", serde_json::json!(2));
    assert_eq!(
        check_import_file(&bytes).unwrap_err(),
        ImportError::UnsupportedVersion
    );
}

/// 版本檢查必須發生在解密之前，UI 才能在要求輸入 passphrase 前就擋下來。
/// 用一個絕對錯誤的 passphrase 驗證：仍應得到版本錯誤，而不是 passphrase 錯誤。
#[test]
fn the_version_check_happens_before_decryption() {
    let bytes = tweak_header("version", serde_json::json!(2));
    assert_eq!(
        decrypt_payload(&bytes, "definitely-not-the-passphrase").unwrap_err(),
        ImportError::UnsupportedVersion
    );
}

#[test]
fn the_current_version_is_accepted() {
    let bytes = encrypt_payload(&sample_payload(), "pw").unwrap();
    assert_eq!(check_import_file(&bytes).unwrap(), EXPORT_VERSION);
}
```

在測試檔頂端加 base64 helper（測試自己用，不要動到 production 程式）：

```rust
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

fn base64_decode(s: &str) -> Vec<u8> {
    B64.decode(s).unwrap()
}

fn base64_encode(b: &[u8]) -> String {
    B64.encode(b)
}
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --test db_export
```

Expected: 編譯失敗，`cannot find function 'check_import_file'`。

- [ ] **Step 3: 實作**

把 Task 3 Step 3 那個暫時的 `check_envelope` 換成：

```rust
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
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --test db_export
```

Expected: 12 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/db_export.rs src-tauri/tests/db_export.rs
git commit -m "feat(db-export): 在解密前完成格式與版本檢查"
```

---

### Task 5: 衝突判定

**Files:**
- Modify: `src-tauri/src/commands/db_export.rs`
- Test: `src-tauri/tests/db_export.rs`

- [ ] **Step 1: 寫失敗的測試**

```rust
use aiterm_lib::config::types::{DbConnection, DbType};

fn existing(id: &str, name: &str) -> DbConnection {
    DbConnection {
        id: id.into(),
        name: name.into(),
        db_type: DbType::Postgresql,
        host: "localhost".into(),
        port: 5432,
        database: "db".into(),
        username: "u".into(),
        default_schema: None,
    }
}

fn incoming(id: &str, name: &str) -> ExportedConnection {
    ExportedConnection {
        id: id.into(),
        name: name.into(),
        db_type: DbType::Postgresql,
        host: "localhost".into(),
        port: 5432,
        database: "db".into(),
        username: "u".into(),
        default_schema: None,
        password: "pw".into(),
    }
}

#[test]
fn an_unknown_connection_is_new_and_keeps_its_exported_id() {
    let r = resolve_conflicts(&[incoming("x", "Fresh")], &[existing("a", "Other")]);
    assert_eq!(r[0].kind, ConflictKind::New);
    // 沿用匯出檔的 id，讓同一份檔案重複匯入是冪等的。
    assert_eq!(r[0].target_id, "x");
    assert_eq!(r[0].existing_name, None);
}

#[test]
fn a_matching_id_overwrites_that_connection() {
    let r = resolve_conflicts(&[incoming("a", "Renamed")], &[existing("a", "Original")]);
    assert_eq!(r[0].kind, ConflictKind::Overwrite);
    assert_eq!(r[0].target_id, "a");
    assert_eq!(r[0].existing_name.as_deref(), Some("Original"));
}

#[test]
fn a_matching_name_overwrites_even_when_the_id_differs() {
    // 同事在他的機器上手動建了同名連線——id 不同但實際是同一筆。
    let r = resolve_conflicts(&[incoming("x", "總行LBOTHODB")], &[existing("a", "總行LBOTHODB")]);
    assert_eq!(r[0].kind, ConflictKind::Overwrite);
    assert_eq!(r[0].target_id, "a", "要覆蓋現有那筆，沿用它的 id");
}

#[test]
fn name_matching_ignores_case_and_surrounding_whitespace() {
    let r = resolve_conflicts(&[incoming("x", "  MyDb  ")], &[existing("a", "mydb")]);
    assert_eq!(r[0].kind, ConflictKind::Overwrite);
    assert_eq!(r[0].target_id, "a");
}

#[test]
fn an_id_match_wins_over_a_name_match_on_a_different_connection() {
    let r = resolve_conflicts(
        &[incoming("a", "Beta")],
        &[existing("a", "Alpha"), existing("b", "Beta")],
    );
    assert_eq!(r[0].target_id, "a", "id 比對優先於名稱比對");
    assert_eq!(r[0].existing_name.as_deref(), Some("Alpha"));
}

#[test]
fn resolutions_come_back_in_input_order_with_matching_indexes() {
    let r = resolve_conflicts(
        &[incoming("x", "New1"), incoming("a", "Hit"), incoming("y", "New2")],
        &[existing("a", "Hit")],
    );
    assert_eq!(r.len(), 3);
    assert_eq!(r.iter().map(|x| x.index).collect::<Vec<_>>(), vec![0, 1, 2]);
    assert_eq!(r[1].kind, ConflictKind::Overwrite);
}
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
cd src-tauri && cargo test --test db_export
```

Expected: 編譯失敗，`cannot find function 'resolve_conflicts'`。

- [ ] **Step 3: 實作**

在 `db_export.rs` 加（`use` 區塊補上 `use crate::config::types::DbConnection;`）：

```rust
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
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --test db_export
```

Expected: 18 passed。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/db_export.rs src-tauri/tests/db_export.rs
git commit -m "feat(db-export): 匯入衝突判定（id 優先、再比名稱）"
```

---

### Task 6: 匯出與檔案檢查的 Tauri 指令

這兩個指令需要 `AppHandle` / `State`，不做自動化測試——邏輯都已經在前面的純函式裡驗證過了，這裡只有接線。接線的正確性由 Task 13 的手動驗證涵蓋。

**Files:**
- Modify: `src-tauri/src/commands/db.rs:54`
- Modify: `src-tauri/src/commands/db_export.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 開放 `secret_key` 給同 crate 使用**

`src-tauri/src/commands/db.rs` 第 54 行：

```rust
fn secret_key(id: &str) -> String {
```

改成：

```rust
pub(crate) fn secret_key(id: &str) -> String {
```

- [ ] **Step 2: 加入兩個指令**

在 `db_export.rs` 檔案最後加：

```rust
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
```

- [ ] **Step 3: 註冊指令**

`src-tauri/src/lib.rs` 第 47-51 行的 `db::{...}` 區塊後面（第 51 行 `},` 之後）加一個新的 use 項目：

```rust
    db_export::{db_check_import_file, db_export_connections},
```

然後在 `generate_handler!` 的 `db_preview_table,`（Database 區塊結尾附近，約第 375 行後）之後加：

```rust
            db_check_import_file,
            db_export_connections,
```

- [ ] **Step 4: 確認編譯與既有測試不受影響**

```bash
cd src-tauri && cargo test
```

Expected: 全部通過（含既有測試）。若出現 `secret_key` 的 `dead_code` 警告請忽略——它兩邊都在用。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/db.rs src-tauri/src/commands/db_export.rs src-tauri/src/lib.rs
git commit -m "feat(db-export): 加入 db_check_import_file 與 db_export_connections 指令"
```

---

### Task 7: 匯入預覽與套用的 Tauri 指令

**Files:**
- Modify: `src-tauri/src/commands/db_export.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 加入型別與兩個指令**

在 `db_export.rs` 最後加：

```rust
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
```

- [ ] **Step 2: 註冊指令**

`src-tauri/src/lib.rs`，把 Task 6 加的那行 use 擴充成：

```rust
    db_export::{
        db_check_import_file, db_export_connections, db_import_connections, db_preview_import,
    },
```

`generate_handler!` 裡在 `db_export_connections,` 之後加：

```rust
            db_preview_import,
            db_import_connections,
```

- [ ] **Step 3: 確認編譯**

```bash
cd src-tauri && cargo test
```

Expected: 全部通過。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/db_export.rs src-tauri/src/lib.rs
git commit -m "feat(db-export): 加入 db_preview_import 與 db_import_connections 指令"
```

---

### Task 8: 前端 IPC wrapper

**Files:**
- Modify: `src/ipc/db.ts`

- [ ] **Step 1: 加入型別與 wrapper**

在 `src/ipc/db.ts` 最後加：

```ts
export type ConflictKind = "new" | "overwrite";

/** 匯入預覽的單筆。後端刻意不送密碼過來。 */
export interface ImportPreviewItem {
  id: string;
  name: string;
  db_type: DbType;
  host: string;
  port: number;
  database: string;
  username: string;
  conflict: ConflictKind;
  existing_name?: string | null;
}

export interface ImportFailure {
  name: string;
  reason: string;
}

export interface ImportResult {
  added: number;
  overwritten: number;
  failures: ImportFailure[];
}

/** 只檢查明文 header，回傳檔案的格式版本。不需要 passphrase。 */
export function dbCheckImportFile(path: string): Promise<number> {
  return invoke("db_check_import_file", { path });
}

/** 回傳實際匯出的筆數。 */
export function dbExportConnections(path: string, ids: string[], passphrase: string): Promise<number> {
  return invoke("db_export_connections", { path, ids, passphrase });
}

export function dbPreviewImport(path: string, passphrase: string): Promise<ImportPreviewItem[]> {
  return invoke("db_preview_import", { path, passphrase });
}

export function dbImportConnections(path: string, passphrase: string, ids: string[]): Promise<ImportResult> {
  return invoke("db_import_connections", { path, passphrase, ids });
}
```

- [ ] **Step 2: 型別檢查**

```bash
npx tsc -b
```

Expected: 無輸出（成功）。**注意不要用 `tsc --noEmit`**——根目錄的 `tsconfig.json` 是 solution file（`"files": []`），那個指令什麼都不檢查、永遠回傳 0。

- [ ] **Step 3: Commit**

```bash
git add src/ipc/db.ts
git commit -m "feat(db-export): 前端 IPC wrapper"
```

---

### Task 9: i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 加入 zh-TW 字串**

在 `src/lib/i18n.ts` 的 zhTW 區塊，`no_connections`（第 40 行）之後加：

```ts
    // 資料庫連線匯出／匯入
    db_export: "匯出",
    db_import: "匯入",
    db_export_title: "匯出資料庫連線",
    db_import_title: "匯入資料庫連線",
    db_transfer_passphrase: "加密密碼",
    db_transfer_passphrase_confirm: "確認加密密碼",
    db_transfer_passphrase_mismatch: "兩次輸入的加密密碼不一致",
    db_transfer_passphrase_hint: "匯入時需要這組密碼。密碼遺失後檔案無法復原。",
    db_transfer_choosing_file: "選擇檔案中…",
    db_transfer_next: "下一步",
    db_export_done: (n: number) => `已匯出 ${n} 筆連線`,
    db_import_done: (added: number, overwritten: number) =>
      `新增 ${added} 筆、覆蓋 ${overwritten} 筆`,
    db_import_new: "新增",
    db_import_overwrite: (name: string) => `覆蓋（原：${name}）`,
    db_err_not_an_export_file: "這不是 AITerm 的資料庫匯出檔",
    db_err_unsupported_version: "此檔案由較新版本的 AITerm 匯出，請先更新 AITerm",
    db_err_wrong_passphrase: "加密密碼錯誤，或檔案已損毀",
    db_err_unsupported_kdf: "不支援的加密方式",
```

- [ ] **Step 2: 加入對應的 en 字串**

在 `enRaw` 區塊，`no_connections`（第 1148 行）之後加：

```ts
    // Database connection export / import
    db_export: "Export",
    db_import: "Import",
    db_export_title: "Export Database Connections",
    db_import_title: "Import Database Connections",
    db_transfer_passphrase: "Passphrase",
    db_transfer_passphrase_confirm: "Confirm passphrase",
    db_transfer_passphrase_mismatch: "The two passphrases do not match",
    db_transfer_passphrase_hint: "You will need this passphrase to import. The file cannot be recovered without it.",
    db_transfer_choosing_file: "Choosing file…",
    db_transfer_next: "Next",
    db_export_done: (n: number) => `Exported ${n} connection${n === 1 ? "" : "s"}`,
    db_import_done: (added: number, overwritten: number) =>
      `Added ${added}, overwrote ${overwritten}`,
    db_import_new: "New",
    db_import_overwrite: (name: string) => `Overwrite (was: ${name})`,
    db_err_not_an_export_file: "This is not an AITerm database export file",
    db_err_unsupported_version: "This file was exported by a newer version of AITerm. Please update AITerm.",
    db_err_wrong_passphrase: "Wrong passphrase, or the file is corrupted",
    db_err_unsupported_kdf: "Unsupported encryption method",
```

- [ ] **Step 3: 型別檢查**

```bash
npx tsc -b
```

Expected: 無輸出。`Translations` 型別由 zh-TW 推導，en 缺 key 或簽章不符會在這裡報錯。

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(db-export): 匯出／匯入的 i18n 字串"
```

---

### Task 10: 錯誤碼對應到 i18n

**Files:**
- Create: `src/components/Settings/DbConnectionTransfer.tsx`
- Create: `src/components/Settings/DbConnectionTransfer.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/components/Settings/DbConnectionTransfer.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { translateDbTransferError } from "./DbConnectionTransfer";
import { translations } from "../../lib/i18n";

const t = translations["zh-TW"];

describe("translateDbTransferError", () => {
  it("maps a known error code to its localized message", () => {
    expect(translateDbTransferError(t, "wrong_passphrase")).toBe(t.db_err_wrong_passphrase);
    expect(translateDbTransferError(t, "not_an_export_file")).toBe(t.db_err_not_an_export_file);
    expect(translateDbTransferError(t, "unsupported_version")).toBe(t.db_err_unsupported_version);
    expect(translateDbTransferError(t, "unsupported_kdf")).toBe(t.db_err_unsupported_kdf);
  });

  it("falls back to the raw text for unknown errors", () => {
    expect(translateDbTransferError(t, "io_error: no such file")).toBe("io_error: no such file");
  });

  it("stringifies non-string rejections", () => {
    expect(translateDbTransferError(t, new Error("boom"))).toContain("boom");
  });
});
```

`translations` 已經是 `src/lib/i18n.ts:2126` 的具名匯出，直接 import 即可，不需要改動 i18n。

- [ ] **Step 2: 執行測試確認失敗**

```bash
npx vitest run src/components/Settings/DbConnectionTransfer.test.tsx
```

Expected: FAIL，找不到模組 `./DbConnectionTransfer`。

- [ ] **Step 3: 實作**

建立 `src/components/Settings/DbConnectionTransfer.tsx`：

```tsx
import type { Translations } from "../../lib/i18n";

/** 後端 `ImportError` 的變體名稱 → i18n key。這些字串是介面契約的一部分，
 *  改動時要同步 `src-tauri/src/commands/db_export.rs` 的 `#[error(...)]`。 */
const ERROR_KEYS = {
  not_an_export_file: "db_err_not_an_export_file",
  unsupported_version: "db_err_unsupported_version",
  wrong_passphrase: "db_err_wrong_passphrase",
  unsupported_kdf: "db_err_unsupported_kdf",
} as const;

/** 已知錯誤碼轉成本地化訊息；其餘（例如 `io_error: ...`）原樣顯示。 */
export function translateDbTransferError(t: Translations, e: unknown): string {
  const raw = String(e);
  const key = ERROR_KEYS[raw as keyof typeof ERROR_KEYS];
  return key ? (t[key] as string) : raw;
}
```

- [ ] **Step 4: 執行測試確認通過**

```bash
npx vitest run src/components/Settings/DbConnectionTransfer.test.tsx
```

Expected: 3 passed。

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/DbConnectionTransfer.tsx src/components/Settings/DbConnectionTransfer.test.tsx src/lib/i18n.ts
git commit -m "feat(db-export): 錯誤碼對應到 i18n 訊息"
```

---

### Task 11: 匯出面板

**Files:**
- Modify: `src/components/Settings/DbConnectionTransfer.tsx`
- Modify: `src/components/Settings/DbConnectionTransfer.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

在測試檔頂端加 mock（放在所有 `import` 之後、`describe` 之前）：

```tsx
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { DbExportPanel } from "./DbConnectionTransfer";
import type { DbConnectionInfo } from "../../ipc/db";

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(), open: vi.fn() }));
vi.mock("../../ipc/db", () => ({
  dbCheckImportFile: vi.fn(),
  dbExportConnections: vi.fn(),
  dbPreviewImport: vi.fn(),
  dbImportConnections: vi.fn(),
}));

import { save } from "@tauri-apps/plugin-dialog";
import { dbExportConnections } from "../../ipc/db";

const CONNS: DbConnectionInfo[] = [
  { id: "a", name: "總行LBOTHODB", db_type: "db2", host: "172.19.2.83", port: 25000,
    database: "LBOTHODB", username: "nuntio", default_schema: "NUNTIO", is_connected: true },
  { id: "b", name: "MSSQL-Docker", db_type: "mssql", host: "192.168.1.30", port: 1433,
    database: "master", username: "sa", default_schema: null, is_connected: false },
];

beforeEach(() => {
  vi.clearAllMocks();
  Object.defineProperty(window, "localStorage", {
    value: { getItem: vi.fn(() => null), setItem: vi.fn(), removeItem: vi.fn(), clear: vi.fn(), key: vi.fn(), length: 0 },
    writable: true,
  });
});

function renderExport(onDone = vi.fn(), onClose = vi.fn()) {
  render(
    <LocaleProvider>
      <DbExportPanel connections={CONNS} onClose={onClose} onDone={onDone} />
    </LocaleProvider>,
  );
  return { onDone, onClose };
}

function typePassphrases(pw: string, confirm: string) {
  fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: pw } });
  fireEvent.change(screen.getByLabelText("確認加密密碼"), { target: { value: confirm } });
}
```

再加測試：

```tsx
describe("DbExportPanel", () => {
  it("checks every connection by default", () => {
    renderExport();
    const boxes = screen.getAllByRole("checkbox");
    expect(boxes).toHaveLength(2);
    boxes.forEach((b) => expect(b).toBeChecked());
  });

  it("disables export until both passphrases match", () => {
    renderExport();
    const btn = screen.getByRole("button", { name: "匯出" });
    expect(btn).toBeDisabled();

    typePassphrases("hunter2", "hunter3");
    expect(btn).toBeDisabled();
    expect(screen.getByText("兩次輸入的加密密碼不一致")).toBeInTheDocument();

    typePassphrases("hunter2", "hunter2");
    expect(btn).toBeEnabled();
  });

  it("disables export when nothing is selected", () => {
    renderExport();
    typePassphrases("pw", "pw");
    screen.getAllByRole("checkbox").forEach((b) => fireEvent.click(b));
    expect(screen.getByRole("button", { name: "匯出" })).toBeDisabled();
  });

  it("sends only the checked ids", async () => {
    vi.mocked(save).mockResolvedValue("/tmp/out.json");
    vi.mocked(dbExportConnections).mockResolvedValue(1);
    const { onDone } = renderExport();

    typePassphrases("pw", "pw");
    fireEvent.click(screen.getAllByRole("checkbox")[1]); // 取消勾選 MSSQL-Docker
    fireEvent.click(screen.getByRole("button", { name: "匯出" }));

    await waitFor(() =>
      expect(dbExportConnections).toHaveBeenCalledWith("/tmp/out.json", ["a"], "pw"),
    );
    await waitFor(() => expect(onDone).toHaveBeenCalledWith("已匯出 1 筆連線"));
  });

  it("does not call the backend when the save dialog is cancelled", async () => {
    vi.mocked(save).mockResolvedValue(null);
    renderExport();
    typePassphrases("pw", "pw");
    fireEvent.click(screen.getByRole("button", { name: "匯出" }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(dbExportConnections).not.toHaveBeenCalled();
  });

  it("shows a localized message when the backend fails", async () => {
    vi.mocked(save).mockResolvedValue("/tmp/out.json");
    vi.mocked(dbExportConnections).mockRejectedValue("io_error: disk full");
    renderExport();
    typePassphrases("pw", "pw");
    fireEvent.click(screen.getByRole("button", { name: "匯出" }));
    await waitFor(() => expect(screen.getByText("io_error: disk full")).toBeInTheDocument());
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
npx vitest run src/components/Settings/DbConnectionTransfer.test.tsx
```

Expected: FAIL，`DbExportPanel` 不是匯出的成員。

- [ ] **Step 3: 實作**

在 `DbConnectionTransfer.tsx` 頂端補 import——**只放本任務用得到的**，Task 12 再擴充（先 import 未使用的東西會被 `npm run lint` 擋下）：

```tsx
import { useState, type CSSProperties } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { dbExportConnections, DB_TYPE_LABELS, type DbConnectionInfo } from "../../ipc/db";
import { useLocale } from "../../contexts/LocaleContext";
```

在 `translateDbTransferError` 之後加：

```tsx
const DEFAULT_EXPORT_NAME = "aiterm-db-connections.json";

export function DbExportPanel({
  connections, onClose, onDone,
}: {
  connections: DbConnectionInfo[];
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const { t } = useLocale();
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set(connections.map((c) => c.id)),
  );
  const [passphrase, setPassphrase] = useState("");
  const [confirmPass, setConfirmPass] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const mismatch = confirmPass.length > 0 && passphrase !== confirmPass;
  const canExport =
    selected.size > 0 && passphrase.length > 0 && passphrase === confirmPass && !busy;

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const handleExport = async () => {
    const path = await save({
      defaultPath: DEFAULT_EXPORT_NAME,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return; // 使用者取消對話框——不呼叫任何 IPC
    setBusy(true);
    setError("");
    try {
      const n = await dbExportConnections(path, [...selected], passphrase);
      onDone(t.db_export_done(n));
    } catch (e) {
      setError(translateDbTransferError(t, e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={panelStyle}>
      <h3 style={headingStyle}>{t.db_export_title}</h3>

      <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 16 }}>
        {connections.map((c) => (
          <label key={c.id} style={rowStyle}>
            <input
              type="checkbox"
              checked={selected.has(c.id)}
              onChange={() => toggle(c.id)}
            />
            <span style={{ flex: 1, minWidth: 0 }}>
              <span style={{ color: "#e6e6e6", fontSize: 13 }}>{c.name}</span>
              <span style={{ color: "#888", fontSize: 11, marginLeft: 8 }}>
                {DB_TYPE_LABELS[c.db_type]} · {c.host}:{c.port}
              </span>
            </span>
          </label>
        ))}
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "120px 1fr", gap: "10px 12px", alignItems: "center" }}>
        <label style={labelStyle} htmlFor="db-export-pass">{t.db_transfer_passphrase}</label>
        <input
          id="db-export-pass"
          type="password"
          value={passphrase}
          onChange={(e) => setPassphrase(e.target.value)}
          style={inputStyle}
        />
        <label style={labelStyle} htmlFor="db-export-pass2">{t.db_transfer_passphrase_confirm}</label>
        <input
          id="db-export-pass2"
          type="password"
          value={confirmPass}
          onChange={(e) => setConfirmPass(e.target.value)}
          style={inputStyle}
        />
      </div>

      <div style={{ color: "#888", fontSize: 11, marginTop: 8 }}>{t.db_transfer_passphrase_hint}</div>
      {mismatch && <div style={errorStyle}>{t.db_transfer_passphrase_mismatch}</div>}
      {error && <div style={errorStyle}>{error}</div>}

      <div style={{ display: "flex", gap: 8, marginTop: 16, justifyContent: "flex-end" }}>
        <button onClick={onClose} className="aiterm-btn aiterm-btn--secondary">{t.cancel}</button>
        <button onClick={handleExport} disabled={!canExport} className="aiterm-btn aiterm-btn--primary">
          {t.db_export}
        </button>
      </div>
    </div>
  );
}

const panelStyle: CSSProperties = {
  background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 8, padding: 20,
};
const headingStyle: CSSProperties = { margin: "0 0 16px", fontSize: 14, color: "#e6e6e6" };
const rowStyle: CSSProperties = {
  display: "flex", alignItems: "center", gap: 10, padding: "8px 10px",
  background: "#141414", border: "1px solid #2a2a2a", borderRadius: 5, cursor: "pointer",
};
const labelStyle: CSSProperties = { color: "#888", fontSize: 12 };
const inputStyle: CSSProperties = {
  background: "#0f0f0f", border: "1px solid #2a2a2a", color: "#e6e6e6",
  borderRadius: 4, padding: "6px 8px", fontSize: 13, width: "100%", boxSizing: "border-box",
};
const errorStyle: CSSProperties = { color: "#f87171", fontSize: 12, marginTop: 8 };
```

- [ ] **Step 4: 執行測試確認通過**

```bash
npx vitest run src/components/Settings/DbConnectionTransfer.test.tsx
```

Expected: 9 passed。

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/DbConnectionTransfer.tsx src/components/Settings/DbConnectionTransfer.test.tsx
git commit -m "feat(db-export): 匯出面板"
```

---

### Task 12: 匯入面板

**Files:**
- Modify: `src/components/Settings/DbConnectionTransfer.tsx`
- Modify: `src/components/Settings/DbConnectionTransfer.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

在測試檔加（沿用 Task 11 的 mock 區塊；再補 import）：

```tsx
import { DbImportPanel } from "./DbConnectionTransfer";
import { open } from "@tauri-apps/plugin-dialog";
import { dbCheckImportFile, dbPreviewImport, dbImportConnections } from "../../ipc/db";
import type { ImportPreviewItem } from "../../ipc/db";

const PREVIEW: ImportPreviewItem[] = [
  { id: "a", name: "總行LBOTHODB", db_type: "db2", host: "172.19.2.83", port: 25000,
    database: "LBOTHODB", username: "nuntio", conflict: "overwrite", existing_name: "舊的總行" },
  { id: "z", name: "新連線", db_type: "mysql", host: "10.0.0.5", port: 3306,
    database: "app", username: "root", conflict: "new", existing_name: null },
];

function renderImport(onDone = vi.fn(), onClose = vi.fn()) {
  render(
    <LocaleProvider>
      <DbImportPanel onClose={onClose} onDone={onDone} />
    </LocaleProvider>,
  );
  return { onDone, onClose };
}

describe("DbImportPanel", () => {
  it("closes without any IPC when the open dialog is cancelled", async () => {
    vi.mocked(open).mockResolvedValue(null);
    const { onClose } = renderImport();
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(dbCheckImportFile).not.toHaveBeenCalled();
  });

  it("shows the error and no passphrase field when the file is rejected", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/bad.json");
    vi.mocked(dbCheckImportFile).mockRejectedValue("unsupported_version");
    renderImport();

    await waitFor(() =>
      expect(
        screen.getByText("此檔案由較新版本的 AITerm 匯出，請先更新 AITerm"),
      ).toBeInTheDocument(),
    );
    // 使用者不該為一個注定被拒的檔案白打一次密碼
    expect(screen.queryByLabelText("加密密碼")).not.toBeInTheDocument();
    expect(dbPreviewImport).not.toHaveBeenCalled();
  });

  it("asks for the passphrase once the file passes the header check", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    renderImport();
    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
  });

  it("renders new and overwrite labels in the preview", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockResolvedValue(PREVIEW);
    renderImport();

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    await waitFor(() => expect(screen.getByText("覆蓋（原：舊的總行）")).toBeInTheDocument());
    expect(screen.getByText("新增")).toBeInTheDocument();
  });

  it("shows a localized message for a wrong passphrase", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockRejectedValue("wrong_passphrase");
    renderImport();

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "nope" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    await waitFor(() =>
      expect(screen.getByText("加密密碼錯誤，或檔案已損毀")).toBeInTheDocument(),
    );
  });

  it("imports only the checked ids and reports the counts", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockResolvedValue(PREVIEW);
    vi.mocked(dbImportConnections).mockResolvedValue({ added: 1, overwritten: 0, failures: [] });
    const { onDone } = renderImport();

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    await waitFor(() => expect(screen.getAllByRole("checkbox")).toHaveLength(2));
    fireEvent.click(screen.getAllByRole("checkbox")[0]); // 取消勾選覆蓋那筆
    fireEvent.click(screen.getByRole("button", { name: "匯入" }));

    await waitFor(() =>
      expect(dbImportConnections).toHaveBeenCalledWith("/tmp/ok.json", "pw", ["z"]),
    );
    await waitFor(() => expect(onDone).toHaveBeenCalledWith("新增 1 筆、覆蓋 0 筆"));
  });

  it("lists per-item failures alongside the summary", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockResolvedValue(PREVIEW);
    vi.mocked(dbImportConnections).mockResolvedValue({
      added: 1, overwritten: 1,
      failures: [{ name: "總行LBOTHODB", reason: "secret_write_failed: denied" }],
    });
    renderImport();

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    await waitFor(() => expect(screen.getAllByRole("checkbox")).toHaveLength(2));
    fireEvent.click(screen.getByRole("button", { name: "匯入" }));

    await waitFor(() =>
      expect(screen.getByText(/secret_write_failed: denied/)).toBeInTheDocument(),
    );
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

```bash
npx vitest run src/components/Settings/DbConnectionTransfer.test.tsx
```

Expected: FAIL，`DbImportPanel` 不是匯出的成員。

- [ ] **Step 3: 實作**

先把頂端的 import 擴充成完整版（Task 11 只 import 了匯出用得到的）：

```tsx
import { useState, useEffect, type CSSProperties } from "react";
import { save, open } from "@tauri-apps/plugin-dialog";
import {
  dbCheckImportFile, dbExportConnections, dbPreviewImport, dbImportConnections,
  DB_TYPE_LABELS,
  type DbConnectionInfo, type ImportPreviewItem,
} from "../../ipc/db";
import { useLocale } from "../../contexts/LocaleContext";
```

然後加入元件（放在 `DbExportPanel` 之後、style 常數之前）：

```tsx
export function DbImportPanel({
  onClose, onDone,
}: {
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const { t } = useLocale();
  const [path, setPath] = useState<string | null>(null);
  const [passphrase, setPassphrase] = useState("");
  const [preview, setPreview] = useState<ImportPreviewItem[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [failures, setFailures] = useState<{ name: string; reason: string }[]>([]);
  const [summary, setSummary] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  // 一掛載就開檔案對話框。使用者取消就直接關掉面板——沒有檔案就沒有
  // 後續流程可言。`cancelled` 擋住 unmount 後的 setState（Tauri 對話框
  // 是非同步的，使用者可能在期間就離開設定頁）。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const picked = await open({
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (cancelled) return;
      if (typeof picked !== "string") {
        onClose();
        return;
      }
      try {
        // 先只看明文 header：格式或版本不合就在這裡結束，不必讓
        // 使用者為一個注定被拒的檔案白打一次密碼。
        await dbCheckImportFile(picked);
        if (!cancelled) setPath(picked);
      } catch (e) {
        if (!cancelled) setError(translateDbTransferError(t, e));
      }
    })();
    return () => { cancelled = true; };
    // 只在掛載時跑一次；t / onClose 變動不該重新開檔案對話框。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const handlePreview = async () => {
    if (!path) return;
    setBusy(true);
    setError("");
    try {
      const items = await dbPreviewImport(path, passphrase);
      setPreview(items);
      setSelected(new Set(items.map((i) => i.id)));
    } catch (e) {
      setError(translateDbTransferError(t, e));
    } finally {
      setBusy(false);
    }
  };

  const handleImport = async () => {
    if (!path) return;
    setBusy(true);
    setError("");
    try {
      const r = await dbImportConnections(path, passphrase, [...selected]);
      setFailures(r.failures);
      setSummary(t.db_import_done(r.added, r.overwritten));
      onDone(t.db_import_done(r.added, r.overwritten));
    } catch (e) {
      setError(translateDbTransferError(t, e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={panelStyle}>
      <h3 style={headingStyle}>{t.db_import_title}</h3>

      {!path && !error && <div style={{ color: "#888", fontSize: 12 }}>{t.db_transfer_choosing_file}</div>}

      {path && !preview && (
        <div style={{ display: "grid", gridTemplateColumns: "120px 1fr", gap: "10px 12px", alignItems: "center" }}>
          <label style={labelStyle} htmlFor="db-import-pass">{t.db_transfer_passphrase}</label>
          <input
            id="db-import-pass"
            type="password"
            value={passphrase}
            onChange={(e) => setPassphrase(e.target.value)}
            style={inputStyle}
          />
        </div>
      )}

      {preview && (
        <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 12 }}>
          {preview.map((item) => (
            <label key={item.id} style={rowStyle}>
              <input type="checkbox" checked={selected.has(item.id)} onChange={() => toggle(item.id)} />
              <span style={{ flex: 1, minWidth: 0 }}>
                <span style={{ color: "#e6e6e6", fontSize: 13 }}>{item.name}</span>
                <span style={{ color: "#888", fontSize: 11, marginLeft: 8 }}>
                  {DB_TYPE_LABELS[item.db_type]} · {item.host}:{item.port}
                </span>
              </span>
              <span style={{ fontSize: 11, color: item.conflict === "new" ? "#34d399" : "#f9a825" }}>
                {item.conflict === "new"
                  ? t.db_import_new
                  : t.db_import_overwrite(item.existing_name ?? item.name)}
              </span>
            </label>
          ))}
        </div>
      )}

      {summary && <div style={{ color: "#34d399", fontSize: 12, marginTop: 8 }}>{summary}</div>}
      {failures.map((f) => (
        <div key={`${f.name}:${f.reason}`} style={errorStyle}>{f.name}：{f.reason}</div>
      ))}
      {error && <div style={errorStyle}>{error}</div>}

      <div style={{ display: "flex", gap: 8, marginTop: 16, justifyContent: "flex-end" }}>
        <button onClick={onClose} className="aiterm-btn aiterm-btn--secondary">{t.cancel}</button>
        {path && !preview && (
          <button
            onClick={handlePreview}
            disabled={busy || passphrase.length === 0}
            className="aiterm-btn aiterm-btn--primary"
          >
            {t.db_transfer_next}
          </button>
        )}
        {preview && (
          <button
            onClick={handleImport}
            disabled={busy || selected.size === 0}
            className="aiterm-btn aiterm-btn--primary"
          >
            {t.db_import}
          </button>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 執行測試確認通過**

```bash
npx vitest run src/components/Settings/DbConnectionTransfer.test.tsx
```

Expected: 16 passed。

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/DbConnectionTransfer.tsx src/components/Settings/DbConnectionTransfer.test.tsx src/lib/i18n.ts
git commit -m "feat(db-export): 匯入面板"
```

---

### Task 13: 接進設定頁

**Files:**
- Modify: `src/components/Settings/DatabaseConnectionsPage.tsx:82-125`
- Modify: `src/components/Settings/DatabaseConnectionsPage.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

`DatabaseConnectionsPage.test.tsx` 現有的 `vi.mock("../../ipc/db", ...)`（第 18-25 行）需要補上新 wrapper，否則元件 import 會失敗。把它整段換成：

```tsx
vi.mock("../../ipc/db", () => ({
  dbListConnections: vi.fn().mockResolvedValue([]),
  dbAddConnection: vi.fn().mockResolvedValue("new-id"),
  dbUpdateConnection: vi.fn().mockResolvedValue(undefined),
  dbRemoveConnection: vi.fn().mockResolvedValue(undefined),
  dbTestConnection: vi.fn().mockResolvedValue(undefined),
  dbCheckImportFile: vi.fn(),
  dbExportConnections: vi.fn(),
  dbPreviewImport: vi.fn(),
  dbImportConnections: vi.fn(),
  DB_TYPE_LABELS: { postgresql: "PostgreSQL", mysql: "MySQL", sqlite: "SQLite", mssql: "MSSQL", db2: "DB2" },
  DB_DEFAULT_PORTS: { postgresql: 5432, mysql: 3306, sqlite: 0, mssql: 1433, db2: 50000 },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(), open: vi.fn() }));
```

現有的 `beforeEach` 只 stub 了 `localStorage`，沒有清 mock，於是呼叫次數會跨測試累積、讓 `toHaveBeenCalledTimes` 讀到前面測試留下的數字。在 `beforeEach` 開頭補一行（`clearAllMocks` 只清呼叫紀錄，不會清掉 `vi.mock` factory 裡設定的 `mockResolvedValue`）：

```tsx
beforeEach(() => {
  vi.clearAllMocks();
  Object.defineProperty(window, "localStorage", { /* 既有內容不動 */ });
});
```

再加測試：

```tsx
import { open } from "@tauri-apps/plugin-dialog";
import {
  dbListConnections, dbCheckImportFile, dbPreviewImport, dbImportConnections,
} from "../../ipc/db";

const ONE_CONN = [{
  id: "a", name: "總行LBOTHODB", db_type: "db2" as const, host: "172.19.2.83", port: 25000,
  database: "LBOTHODB", username: "nuntio", default_schema: "NUNTIO", is_connected: true,
}];

describe("DatabaseConnectionsPage transfer buttons", () => {
  it("disables export when there are no connections", async () => {
    vi.mocked(dbListConnections).mockResolvedValue([]);
    renderPage();
    await waitFor(() => expect(screen.getByRole("button", { name: "匯出" })).toBeDisabled());
    expect(screen.getByRole("button", { name: "匯入" })).toBeEnabled();
  });

  it("opens the export panel and hides the connection list", async () => {
    vi.mocked(dbListConnections).mockResolvedValue(ONE_CONN);
    renderPage();
    // 必須先等清單載入完。「匯出」鈕在 connections 還是空陣列時是 disabled，
    // 而 waitFor 的第一次檢查是同步跑的——getByRole 當下就找得到那顆
    // （存在，只是 disabled），callback 不 throw 就立刻 resolve，於是點擊
    // 會落在 disabled 按鈕上變成 no-op。要等的是「鈕變成可按」，不是「鈕存在」。
    await waitFor(() => expect(screen.getByRole("button", { name: "匯出" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "匯出" }));
    expect(screen.getByText("匯出資料庫連線")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "+ 新增連線" })).not.toBeInTheDocument();
  });

  // 這是真正的回歸風險：匯入完成後若沒有重新拉一次清單，畫面會停在舊資料。
  it("reloads the connection list after an import finishes", async () => {
    vi.mocked(dbListConnections).mockResolvedValue(ONE_CONN);
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockResolvedValue([{
      id: "z", name: "新連線", db_type: "mysql", host: "10.0.0.5", port: 3306,
      database: "app", username: "root", conflict: "new", existing_name: null,
    }]);
    vi.mocked(dbImportConnections).mockResolvedValue({ added: 1, overwritten: 0, failures: [] });

    renderPage();
    await waitFor(() => expect(dbListConnections).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "匯入" }));

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    await waitFor(() => expect(screen.getByRole("checkbox")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "匯入" }));

    await waitFor(() => expect(dbListConnections).toHaveBeenCalledTimes(2));
    // 匯入面板刻意留在畫面上，讓使用者看得到結果與失敗清單
    expect(screen.getByText("新增 1 筆、覆蓋 0 筆")).toBeInTheDocument();
  });
});
```

第三個測試裡有兩顆名稱都是「匯入」的按鈕（頁面標題列的、面板底部的）。點下第一次時面板還沒開，`getByRole` 只找得到標題列那顆；面板開啟後標題列那顆會因為 `!transfer` 條件消失，所以第二次 `getByRole` 仍然只會命中一顆。這是實作必須維持的性質——若把標題列按鈕留著，這個測試會以「找到多個」失敗。

- [ ] **Step 2: 執行測試確認失敗**

```bash
npx vitest run src/components/Settings/DatabaseConnectionsPage.test.tsx
```

Expected: FAIL，找不到名稱為「匯出」的按鈕。

- [ ] **Step 3: 實作**

`DatabaseConnectionsPage.tsx` 加 import：

```tsx
import { DbExportPanel, DbImportPanel } from "./DbConnectionTransfer";
```

加 state（放在 `confirmingDelete` 那行之後，第 25 行後）：

```tsx
  const [transfer, setTransfer] = useState<"export" | "import" | null>(null);
  const [notice, setNotice] = useState("");
```

把標題列（第 84-94 行）換成：

```tsx
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
        <h2 style={{ margin: 0, fontSize: 16, color: "#e6e6e6" }}>{t.db_connections}</h2>
        {!showForm && !transfer && (
          <div style={{ display: "flex", gap: 8 }}>
            <button
              onClick={() => { setTransfer("export"); setNotice(""); }}
              disabled={connections.length === 0}
              className="aiterm-btn aiterm-btn--secondary"
            >
              {t.db_export}
            </button>
            <button
              onClick={() => { setTransfer("import"); setNotice(""); }}
              className="aiterm-btn aiterm-btn--secondary"
            >
              {t.db_import}
            </button>
            <button
              onClick={() => { setForm(EMPTY_FORM); setShowForm(true); setTestStatus("idle"); }}
              className="aiterm-btn aiterm-btn--primary"
            >
              {t.add_connection}
            </button>
          </div>
        )}
      </div>

      {notice && <div style={{ color: "#34d399", fontSize: 12, marginBottom: 12 }}>{notice}</div>}

      {transfer === "export" && (
        <DbExportPanel
          connections={connections}
          onClose={() => setTransfer(null)}
          onDone={(msg) => { setNotice(msg); setTransfer(null); }}
        />
      )}
      {transfer === "import" && (
        <DbImportPanel
          onClose={() => setTransfer(null)}
          onDone={() => load()}
        />
      )}
```

把連線清單的顯示條件（第 96 行）從 `{!showForm && (` 改成：

```tsx
      {!showForm && !transfer && (
```

匯入的 `onDone` 刻意**不**關閉面板——`ImportResult.failures` 要留在畫面上讓使用者看到哪幾筆失敗。使用者自己按「取消」離開。

也因此匯入的 `onDone` 只呼叫 `load()`，**不**設 `notice`：面板留在畫面上、已經自己顯示了結果摘要與失敗清單，頁面再顯示一次會讓同一句話同時出現在畫面上兩處。`notice` 只服務匯出流程——那個面板完成後會關閉，需要頁面接手報告結果。

- [ ] **Step 4: 執行測試確認通過**

```bash
npx vitest run src/components/Settings/DatabaseConnectionsPage.test.tsx
```

Expected: 6 passed（3 個既有 + 3 個新增）。

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/DatabaseConnectionsPage.tsx src/components/Settings/DatabaseConnectionsPage.test.tsx
git commit -m "feat(db-export): 設定頁接上匯出／匯入面板"
```

---

### Task 14: 全量驗證與手動確認

自動化測試涵蓋了純函式與 UI 行為，但**沒有**涵蓋 Tauri 接線（`State` 注入、Keychain 實際讀寫、檔案實際落地）。那一段只能手動驗。

- [ ] **Step 1: 跑完整驗證**

```bash
npm run lint
npx tsc -b
npm run test
cd src-tauri && cargo test
```

Expected: 四個指令全部通過。任何一個失敗都要修到過，不要跳過。

- [ ] **Step 2: 啟動 app**

```bash
npm run tauri:dev
```

- [ ] **Step 3: 手動驗證清單**

在設定 → 資料庫連線頁逐項確認：

1. 勾掉其中一筆後匯出到桌面，輸入兩次相同密碼 → 顯示「已匯出 N 筆連線」，且 N 等於勾選數
2. 用文字編輯器打開該檔 → 看得到 `aiterm-db-export`，**看不到**任何主機 IP、帳號或密碼
3. 匯入同一份檔、輸入正確密碼 → 每筆都標「覆蓋（原：…）」（因為 id 都對得上）
4. 全選匯入 → 顯示「新增 0 筆、覆蓋 N 筆」，連線清單筆數不變（驗證冪等）
5. 匯入同一份檔、故意輸入錯誤密碼 → 顯示「加密密碼錯誤，或檔案已損毀」
6. 隨便挑一個 `.json`（例如 `package.json`）匯入 → 立刻顯示「這不是 AITerm 的資料庫匯出檔」，且**沒有**跳出密碼輸入框
7. 手動把匯出檔的 `"version": 1` 改成 `2` 再匯入 → 顯示「此檔案由較新版本的 AITerm 匯出，請先更新 AITerm」
8. 刪掉一筆連線後重新匯入該檔 → 顯示「新增 1 筆」，且該連線**不需重新輸入密碼**就能成功連線（驗證 Keychain 確實寫回去了）
9. 切到 English 介面，重跑步驟 1 → 所有文字都是英文，沒有殘留中文

- [ ] **Step 4: 手動驗證「空密碼不覆寫既有 Keychain 密碼」**

這條規則寫在 `db_import_connections` 裡（`if !e.password.is_empty()`），因為需要 `State` 注入而無法單元測試，只能手動驗。步驟（macOS）：

1. 新增一筆 SQLite 連線，名稱取 `EmptyPassProbe`（SQLite 表單沒有密碼欄位，所以 Keychain 會存空字串）
2. 只勾這一筆匯出成 `~/Desktop/probe.json`
3. 刪除該連線，改新增一筆**同名** `EmptyPassProbe` 的 MSSQL 連線，設好密碼並測試連線成功
4. 從設定檔找出這筆的新 id：

```bash
grep -B2 -A6 'EmptyPassProbe' ~/Library/Application\ Support/AITerm/config.toml
```

5. 確認 Keychain 裡有密碼（`SecretStore` 的 service 是 `aiterm`，account 是 `db:{id}`）：

```bash
security find-generic-password -s aiterm -a "db:<上一步的 id>" -w
```

6. 匯入 `probe.json`（名稱相符 → 判為「覆蓋」），勾選並匯入
7. 再跑一次步驟 5 的指令

Expected: 密碼**仍然存在且未改變**。若回傳空字串或找不到，代表空密碼把既有密碼清掉了——這是 bug，回頭檢查 `db_import_connections` 的 `if !e.password.is_empty()` 判斷。

驗證完把 `EmptyPassProbe` 刪掉。

- [ ] **Step 5: Commit（若手動驗證過程有修）**

```bash
git add -A
git commit -m "fix(db-export): 手動驗證修正"
```

---

## 已知限制

衝突判定是對**匯入前的設定快照**做的，不會把迴圈中前幾筆剛套用的結果算進去。這帶來兩個邊界行為：

1. 同一份匯出檔內部若有兩筆同名連線，且兩者在現有設定中都沒有對應，兩筆都會判為 `New`，結果是兩筆同名連線並存。
2. 反之，若兩筆都對應到**同一個**現有連線，兩筆會拿到相同的 `target_id`，第二筆覆蓋掉第一筆，但 `overwritten` 會計為 2 而實際只有一列。

兩者都需要使用者刻意製造出「一份匯出檔內含重複名稱」的檔案才會發生，不值得為它們增加複雜度。
