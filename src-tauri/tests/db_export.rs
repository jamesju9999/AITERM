//! 匯出／匯入純函式的測試。這些函式不碰 Tauri、不碰 Keychain，
//! 所以可以直接呼叫，不需要建立 AppHandle。

use aiterm_lib::commands::db_export::*;

#[test]
fn crypto_dependencies_have_the_expected_api() {
    let out = crypto_smoke_test();
    assert!(!out.is_empty());
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
