//! 啟動外部命令（`npx` 之類）的方式。預設直接執行；桌面版在 macOS 要經過登入 shell 才找得到使用者的 PATH，
//! 所以由它在啟動時用 `set_builder` 換成自己的版本。

use std::sync::OnceLock;

/// 依命令與參數建立（還沒啟動的）子行程。
pub type Builder = fn(&str, &[String]) -> tokio::process::Command;

static BUILDER: OnceLock<Builder> = OnceLock::new();

#[cfg(target_os = "windows")]
fn default_builder(command: &str, args: &[String]) -> tokio::process::Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.arg("/C").arg(command);
    cmd.args(args);
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd
}

#[cfg(not(target_os = "windows"))]
fn default_builder(command: &str, args: &[String]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    cmd
}

/// 只能設一次（第二次以後忽略）。
pub fn set_builder(b: Builder) {
    let _ = BUILDER.set(b);
}

pub fn build_command(command: &str, args: &[String]) -> tokio::process::Command {
    BUILDER.get().copied().unwrap_or(default_builder)(command, args)
}

/// MCP 的 initialize 參數（代理 Playwright MCP 時送出）。
pub fn initialize_params() -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "clientInfo": { "name": "AITerm", "version": env!("CARGO_PKG_VERSION") }
    })
}
