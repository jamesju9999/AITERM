//! 協作頁一進來就檢查 Claude Code／Codex／git 有沒有裝，見
//! `docs/superpowers/specs/2026-10-19-agent-tools-check-design.md`。
//!
//! 要用派工分頁同一個環境去找：GUI App 自己的 PATH 不含 Homebrew、npm、`~/.local/bin`，
//! 看它會說「沒裝」但派工其實跑得起來（或反過來）。

use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolsCheck {
    pub platform: &'static str,
    /// 設定裡的派工指令（原樣），前端顯示用。
    pub claude_command: String,
    /// 找到的路徑；`None` = 找不到。
    pub claude: Option<String>,
    pub codex: Option<String>,
    pub git: Option<String>,
}

const MARK: &str = "__AITERM_TOOL__";
const TIMEOUT: Duration = Duration::from_secs(10);

/// 派工指令的程式名稱：第一個字，可以用引號包住含空白的路徑。
pub fn program_of(command: &str) -> String {
    let c = command.trim();
    for q in ['"', '\''] {
        if let Some(rest) = c.strip_prefix(q) {
            if let Some(end) = rest.find(q) {
                return rest[..end].to_string();
            }
        }
    }
    c.split_whitespace().next().unwrap_or("claude").to_string()
}

/// 從 shell 輸出挑出 `__AITERM_TOOL__<名稱>=<路徑>` 行；其他行（歡迎訊息等）略過。
/// 路徑空白或沒有那一行都算找不到。
pub fn parse_marks(output: &str, name: &str) -> Option<String> {
    let prefix = format!("{MARK}{name}=");
    output
        .lines()
        .filter_map(|l| l.trim_end_matches('\r').strip_prefix(&prefix))
        .map(str::trim)
        .find(|p| !p.is_empty())
        .map(str::to_string)
}

#[cfg(not(windows))]
fn lookup_script(names: &[&str]) -> (String, Vec<String>) {
    // 名稱當成位置參數傳進去，不拼進指令字串，不用處理引號。
    let script = format!(r#"for c in "$@"; do printf '\n{MARK}%s=%s\n' "$c" "$(command -v "$c" 2>/dev/null)"; done"#);
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
    // `-i`：派工分頁是互動 shell，npm／claude 安裝程式常把 PATH 加在 .zshrc。
    let mut args = vec!["-ilc".to_string(), script, "aiterm-tools-check".to_string()];
    args.extend(names.iter().map(|n| n.to_string()));
    (shell, args)
}

#[cfg(windows)]
fn lookup_script(names: &[&str]) -> (String, Vec<String>) {
    let list = names.iter().map(|n| format!("'{}'", n.replace('\'', "''"))).collect::<Vec<_>>().join(",");
    // 腳本裡不用雙引號：Windows PowerShell 5.1 解析 -Command 參數時會吃掉內嵌的 `"`。
    let script = format!(
        "foreach ($n in @({list})) {{ $p = (Get-Command $n -ErrorAction SilentlyContinue | Select-Object -First 1).Source; \
         [Console]::Out.WriteLine([Environment]::NewLine + '{MARK}' + $n + '=' + $p) }}"
    );
    let shell = crate::pty::shell::find_powershell7()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "powershell.exe".into());
    (shell, vec!["-NoLogo".into(), "-NonInteractive".into(), "-Command".into(), script])
}

/// 在派工用的 shell 裡找這些程式。逾時或 shell 起不來回錯誤（前端就不顯示提示）。
pub async fn lookup(names: &[&str]) -> Result<String, String> {
    let (shell, args) = lookup_script(names);
    let mut cmd = tokio::process::Command::new(&shell);
    cmd.args(&args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(TIMEOUT, cmd.output())
        .await
        .map_err(|_| format!("{shell} 在 {} 秒內沒有回應", TIMEOUT.as_secs()))?
        .map_err(|e| format!("無法執行 {shell}：{e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub async fn check(claude_command: &str) -> Result<ToolsCheck, String> {
    // Windows：使用者剛裝完 git／claude，行程的 PATH 還是舊的。
    super::path_refresh::refresh().await;
    let claude = program_of(claude_command);
    let out = lookup(&[&claude, "codex", "git"]).await?;
    Ok(ToolsCheck {
        platform: if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" },
        claude_command: claude_command.to_string(),
        claude: parse_marks(&out, &claude),
        codex: parse_marks(&out, "codex"),
        git: parse_marks(&out, "git").filter(|p| !is_macos_git_stub(p, xcode_tools_installed)),
    })
}

/// 新的 Mac 上 `/usr/bin/git` 是個殼：執行它只會跳出「安裝命令列開發者工具」的視窗。
/// `command -v` 找得到它，所以要另外看命令列工具裝了沒（`xcode-select -p`，不會跳視窗）。
fn is_macos_git_stub(path: &str, tools_installed: fn() -> bool) -> bool {
    cfg!(target_os = "macos") && path == "/usr/bin/git" && !tools_installed()
}

fn xcode_tools_installed() -> bool {
    std::process::Command::new("xcode-select")
        .arg("-p")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_is_the_first_word_or_a_quoted_path() {
        assert_eq!(program_of("claude"), "claude");
        assert_eq!(program_of("  claude --verbose "), "claude");
        assert_eq!(program_of(r#""/Users/a b/bin/claude" --x"#), "/Users/a b/bin/claude");
        assert_eq!(program_of("'C:\\Program Files\\claude.exe'"), "C:\\Program Files\\claude.exe");
        assert_eq!(program_of("   "), "claude");
    }

    #[test]
    fn marks_are_picked_out_of_noisy_output() {
        let out = "Welcome!\nLast login: x\n\n__AITERM_TOOL__claude=/Users/a/.local/bin/claude\r\n\n__AITERM_TOOL__codex=\n\n__AITERM_TOOL__git=/usr/bin/git\n";
        assert_eq!(parse_marks(out, "claude").as_deref(), Some("/Users/a/.local/bin/claude"));
        assert_eq!(parse_marks(out, "codex"), None, "空路徑 = 找不到");
        assert_eq!(parse_marks(out, "git").as_deref(), Some("/usr/bin/git"));
        assert_eq!(parse_marks(out, "svn"), None, "沒有那一行 = 找不到");
        assert_eq!(parse_marks("echo __AITERM_TOOL__git=/x", "git"), None, "要在行首，回音的指令不算");
    }

    #[test]
    fn the_macos_git_stub_only_counts_when_the_command_line_tools_are_installed() {
        assert_eq!(is_macos_git_stub("/usr/bin/git", || false), cfg!(target_os = "macos"));
        assert!(!is_macos_git_stub("/usr/bin/git", || true));
        assert!(!is_macos_git_stub("/opt/homebrew/bin/git", || false), "Homebrew 的 git 是真的");
    }

    /// 真的跑一次使用者的 shell：git 在開發機與 CI 都有，亂取的名稱一定沒有。
    #[tokio::test]
    #[cfg_attr(windows, ignore = "PowerShell 版在 Windows 實機驗")]
    async fn the_users_shell_finds_git_and_not_a_made_up_name() {
        let out = lookup(&["git", "aiterm-no-such-cli-7f3e"]).await.unwrap();
        assert!(parse_marks(&out, "git").is_some(), "{out}");
        assert_eq!(parse_marks(&out, "aiterm-no-such-cli-7f3e"), None, "{out}");
    }
}
