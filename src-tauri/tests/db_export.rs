//! 匯出／匯入純函式的測試。這些函式不碰 Tauri、不碰 Keychain，
//! 所以可以直接呼叫，不需要建立 AppHandle。

use aiterm_lib::commands::db_export::*;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

fn base64_decode(s: &str) -> Vec<u8> {
    B64.decode(s).unwrap()
}

fn base64_encode(b: &[u8]) -> String {
    B64.encode(b)
}

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
