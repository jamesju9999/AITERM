//! 匯出／匯入純函式的測試。這些函式不碰 Tauri、不碰 Keychain，
//! 所以可以直接呼叫，不需要建立 AppHandle。

use aiterm_lib::commands::db_export::*;
use aiterm_lib::config::types::{DbConnection, DbType};

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

// ---- KDF 參數上界（Finding 1）----

#[test]
fn an_absurd_memory_cost_is_rejected_rather_than_allocated() {
    // 3.8 GiB。沒有上界的話 argon2 會真的去配置，配不到就 abort 整個行程。
    let bytes = tweak_header("kdf", serde_json::json!({
        "alg": "argon2id", "salt": "AAAAAAAAAAAAAAAAAAAAAA==",
        "m_cost": 4_000_000u32, "t_cost": 2, "p_cost": 1,
    }));
    assert_eq!(decrypt_payload(&bytes, "pw").unwrap_err(), ImportError::UnsupportedKdf);
}

#[test]
fn an_absurd_time_cost_is_rejected() {
    let bytes = tweak_header("kdf", serde_json::json!({
        "alg": "argon2id", "salt": "AAAAAAAAAAAAAAAAAAAAAA==",
        "m_cost": 19456, "t_cost": u32::MAX, "p_cost": 1,
    }));
    assert_eq!(decrypt_payload(&bytes, "pw").unwrap_err(), ImportError::UnsupportedKdf);
}

/// 上界檢查必須在 `check_import_file` 這一關就生效——UI 是在要求使用者
/// 輸入 passphrase 之前呼叫它的，晚一步擋就等於沒擋。
#[test]
fn absurd_kdf_params_are_caught_before_the_passphrase_is_needed() {
    let bytes = tweak_header("kdf", serde_json::json!({
        "alg": "argon2id", "salt": "AAAAAAAAAAAAAAAAAAAAAA==",
        "m_cost": 4_000_000u32, "t_cost": 2, "p_cost": 1,
    }));
    assert_eq!(check_import_file(&bytes).unwrap_err(), ImportError::UnsupportedKdf);
}

#[test]
fn our_own_export_is_within_the_kdf_bounds() {
    let bytes = encrypt_payload(&sample_payload(), "pw").unwrap();
    assert!(check_import_file(&bytes).is_ok());
    assert!(decrypt_payload(&bytes, "pw").is_ok());
}

// ---- 版本閘門優先於其餘 header 結構（Finding 2）----

/// v2 若改動了 header 的其餘結構，使用者該看到「請更新 AITerm」，
/// 而不是「這不是匯出檔」。
#[test]
fn a_newer_version_is_reported_as_such_even_when_the_rest_of_the_header_changed() {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "format": "aiterm-db-export",
        "version": 2,
        "kdf": { "alg": "scrypt", "salt": "AAAA", "log_n": 15 },  // v1 的欄位全不見了
        "aead": "chacha20-poly1305",                               // 連 cipher 都改名了
        "payload": "AAAA",
    })).unwrap();
    assert_eq!(check_import_file(&bytes).unwrap_err(), ImportError::UnsupportedVersion);
}

/// 認不得的版本表示法一律視為「比我們新」。
#[test]
fn a_non_numeric_version_is_treated_as_unsupported() {
    let bytes = tweak_header("version", serde_json::json!("2.0"));
    assert_eq!(check_import_file(&bytes).unwrap_err(), ImportError::UnsupportedVersion);
}

/// 沒有 format 欄位的 JSON 仍然要是「不是匯出檔」，不能因為版本閘門
/// 的改動而變成別的錯誤。
#[test]
fn json_without_a_format_field_is_still_not_an_export_file() {
    let bytes = serde_json::to_vec(&serde_json::json!({ "version": 1 })).unwrap();
    assert_eq!(check_import_file(&bytes).unwrap_err(), ImportError::NotAnExportFile);
}

// ---- 既有但未被測到的路徑（Finding 3）----

#[test]
fn an_unknown_kdf_algorithm_is_rejected() {
    let bytes = tweak_header("kdf", serde_json::json!({
        "alg": "scrypt", "salt": "AAAAAAAAAAAAAAAAAAAAAA==",
        "m_cost": 19456, "t_cost": 2, "p_cost": 1,
    }));
    assert_eq!(decrypt_payload(&bytes, "pw").unwrap_err(), ImportError::UnsupportedKdf);
}

#[test]
fn an_unknown_cipher_is_rejected() {
    let bytes = tweak_header("cipher", serde_json::json!("chacha20-poly1305"));
    assert_eq!(decrypt_payload(&bytes, "pw").unwrap_err(), ImportError::UnsupportedKdf);
}

/// `Nonce::from_slice` 對長度不符會 panic。長度檢查是唯一擋在它前面的
/// 東西，刪掉它所有其他測試依然全綠——所以這條測試要釘住它。
#[test]
fn a_wrong_length_nonce_is_rejected_instead_of_panicking() {
    let bytes = tweak_header("nonce", serde_json::json!("AAAAAAAAAAA=")); // 8 bytes，不是 12
    assert_eq!(decrypt_payload(&bytes, "pw").unwrap_err(), ImportError::NotAnExportFile);
}

#[test]
fn invalid_base64_in_the_header_is_not_an_export_file() {
    for field in ["nonce", "data"] {
        let bytes = tweak_header(field, serde_json::json!("!!!not base64!!!"));
        assert_eq!(
            decrypt_payload(&bytes, "pw").unwrap_err(),
            ImportError::NotAnExportFile,
            "field={field}"
        );
    }
    let bytes = tweak_header("kdf", serde_json::json!({
        "alg": "argon2id", "salt": "!!!not base64!!!",
        "m_cost": 19456, "t_cost": 2, "p_cost": 1,
    }));
    assert_eq!(decrypt_payload(&bytes, "pw").unwrap_err(), ImportError::NotAnExportFile);
}

// ---- 同一份匯出檔內部的目標衝突 ----

/// 兩筆匯出資料分別以 id 和名稱命中同一筆現有連線。若兩筆都套用，
/// 後者會把前者剛寫進去的內容默默蓋掉，而回傳筆數會說「覆蓋了 2 筆」。
#[test]
fn a_second_entry_claiming_the_same_target_is_marked_duplicate() {
    let r = resolve_conflicts(
        &[incoming("local-1", "Staging"), incoming("exp-9", "Prod")],
        &[existing("local-1", "Prod")],
    );
    assert_eq!(r[0].kind, ConflictKind::Overwrite);
    assert_eq!(r[0].target_id, "local-1");
    assert_eq!(r[1].kind, ConflictKind::Duplicate);
    assert_eq!(r[1].target_id, "local-1", "仍然指向同一個目標，只是不套用");
}

/// 匯出檔裡兩筆同名、目標機器全新。第二筆若也判 New，就會產生兩筆
/// 同名連線——正是名稱比對規則想避免的情況。
#[test]
fn two_same_named_new_entries_do_not_both_get_created() {
    let r = resolve_conflicts(&[incoming("x", "Dupe"), incoming("y", "Dupe")], &[]);
    assert_eq!(r[0].kind, ConflictKind::New);
    assert_eq!(r[1].kind, ConflictKind::Duplicate);
}

/// 手動合併兩份匯出檔就會產生重複 id。`add_db_connection` 只是 push、
/// 沒有重複檢查，兩筆都建立的話設定裡會出現兩筆相同 id。
#[test]
fn two_entries_sharing_an_id_do_not_both_get_created() {
    let r = resolve_conflicts(&[incoming("same", "A"), incoming("same", "B")], &[]);
    assert_eq!(r[0].kind, ConflictKind::New);
    assert_eq!(r[1].kind, ConflictKind::Duplicate);
}

/// 去重不能誤傷正常情況：目標各不相同時，全部照舊。
#[test]
fn distinct_targets_are_all_still_applied() {
    let r = resolve_conflicts(
        &[incoming("a", "Hit"), incoming("x", "Fresh1"), incoming("y", "Fresh2")],
        &[existing("a", "Hit")],
    );
    assert_eq!(r[0].kind, ConflictKind::Overwrite);
    assert_eq!(r[1].kind, ConflictKind::New);
    assert_eq!(r[2].kind, ConflictKind::New);
}

/// 認領是照輸入順序先到先得——第一筆保留，後面的才被降級。
#[test]
fn the_first_entry_wins_the_target() {
    let r = resolve_conflicts(
        &[incoming("x", "Same"), incoming("y", "Same"), incoming("z", "Same")],
        &[],
    );
    assert_eq!(r[0].kind, ConflictKind::New);
    assert_eq!(r[0].target_id, "x");
    assert_eq!(r[1].kind, ConflictKind::Duplicate);
    assert_eq!(r[2].kind, ConflictKind::Duplicate);
}

/// `ConflictKind` 會序列化送到前端，字串值是介面契約的一部分。
#[test]
fn conflict_kind_serializes_in_snake_case() {
    assert_eq!(serde_json::to_value(ConflictKind::New).unwrap(), "new");
    assert_eq!(serde_json::to_value(ConflictKind::Overwrite).unwrap(), "overwrite");
    assert_eq!(serde_json::to_value(ConflictKind::Duplicate).unwrap(), "duplicate");
}
