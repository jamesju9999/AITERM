//! 匯出／匯入純函式的測試。這些函式不碰 Tauri、不碰 Keychain，
//! 所以可以直接呼叫，不需要建立 AppHandle。

use aiterm_lib::commands::db_export::*;

#[test]
fn crypto_dependencies_have_the_expected_api() {
    let out = crypto_smoke_test();
    assert!(!out.is_empty());
}
