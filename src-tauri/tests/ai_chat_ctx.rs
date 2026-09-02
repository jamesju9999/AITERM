//! ai_chat_ctx 的情境組裝：RemoteCtx 欄位進 build_chat_prompt，缺 shell/cwd degrade。
//! ai_chat_ctx 本身是 #[tauri::command]（要 AppHandle/State），無法在整合測試建構，
//! 所以測它的純建構塊：snapshot_from_remote_ctx + build_chat_prompt。

use aiterm_lib::ai::context::snapshot_from_remote_ctx;
use aiterm_lib::ai::Locale;
use aiterm_lib::commands::ai::build_chat_prompt;
use std::path::PathBuf;

#[test]
fn remote_ctx_fields_land_in_chat_prompt() {
    let snap = snapshot_from_remote_ctx(
        "linux",
        None,
        None,
        Some("user@host:~/proj$ ls\nCargo.toml  src/".into()),
    );
    let prompt = build_chat_prompt(&snap, Locale::En, false);
    assert!(prompt.contains("OS: linux"), "{prompt}");
    assert!(prompt.contains("~/proj$ ls"), "recent_output must reach the prompt: {prompt}");
    assert!(prompt.contains("Cwd: ."), "missing cwd degrades to \".\": {prompt}");
    assert!(prompt.contains("Shell: \n"), "missing shell renders as a blank field: {prompt}");
}

#[test]
fn known_fields_land_in_chat_prompt() {
    let snap = snapshot_from_remote_ctx("windows", Some("pwsh"), Some("C:\\src"), None);
    assert_eq!(snap.cwd, PathBuf::from("C:\\src"));
    let prompt = build_chat_prompt(&snap, Locale::ZhTw, false);
    assert!(prompt.contains("Shell: pwsh"));
    assert!(prompt.contains("C:\\src"));
}
