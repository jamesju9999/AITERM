//! Verifies `ai_query_ctx`'s context assembly: `RemoteCtx` fields reach the
//! system prompt sent to the provider, and a missing shell/cwd degrades
//! gracefully (empty `Shell:`, `Cwd: .`) instead of panicking.
//!
//! `ai_query_ctx` itself is a `#[tauri::command]` needing an `AppHandle` +
//! `State`, which can't be built here — so we exercise its two pure building
//! blocks directly, the same pattern `ai_query_command.rs` uses.

use aiterm_lib::ai::context::snapshot_from_remote_ctx;
use aiterm_lib::commands::ai::build_single_command_prompt;
use aiterm_lib::ai::Locale;
use std::path::PathBuf;

#[test]
fn remote_ctx_fields_land_in_prompt() {
    let snap = snapshot_from_remote_ctx(
        "linux",
        None,
        None,
        Some("user@host:~/proj$ ls\nCargo.toml  src/".into()),
    );
    let prompt = build_single_command_prompt(&snap, Locale::En);
    assert!(prompt.contains("OS: linux"), "prompt: {prompt}");
    assert!(prompt.contains("~/proj$ ls"), "recent_output must reach the prompt: {prompt}");
    assert!(prompt.contains("Cwd: ."), "missing cwd degrades to \".\": {prompt}");
    assert!(prompt.contains("Shell: \n"), "missing shell renders as a blank field: {prompt}");
}

#[test]
fn remote_ctx_known_fields_land_in_prompt() {
    let snap = snapshot_from_remote_ctx(
        "windows",
        Some("pwsh"),
        Some("C:\\src"),
        None,
    );
    assert_eq!(snap.cwd, PathBuf::from("C:\\src"));
    let prompt = build_single_command_prompt(&snap, Locale::ZhTw);
    assert!(prompt.contains("OS: windows"));
    assert!(prompt.contains("Shell: pwsh"));
    assert!(prompt.contains("C:\\src"));
    // No recent_output supplied → no "Recent terminal output" section.
    assert!(!prompt.contains("Recent terminal output"), "prompt: {prompt}");
}
