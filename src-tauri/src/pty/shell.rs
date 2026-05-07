use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
}

/// Return the first available shell on this platform.
pub fn default_shell() -> Option<ShellSpec> {
    #[cfg(windows)]
    {
        windows_default_shell()
    }
    #[cfg(not(windows))]
    {
        unix_default_shell()
    }
}

#[cfg(windows)]
fn windows_default_shell() -> Option<ShellSpec> {
    for candidate in ["pwsh.exe", "powershell.exe"] {
        if which_on_path(candidate).is_some() {
            return Some(inject_powershell_integration(PathBuf::from(candidate)));
        }
    }
    if which_on_path("cmd.exe").is_some() {
        return Some(inject_cmd_integration());
    }
    None
}

/// Inject OSC 133 shell integration into PowerShell (pwsh.exe / powershell.exe).
///
/// Overrides the `prompt` function to emit D (command finished) and A (prompt start)
/// markers. The prompt runs after every command — including those sent programmatically
/// via PTY — so no preexec hook is needed. The exit code is captured from `$?` and
/// `$LASTEXITCODE`. The user's original prompt function is preserved and called inside
/// our wrapper.
#[cfg(windows)]
fn inject_powershell_integration(program: PathBuf) -> ShellSpec {
    let temp_dir = std::env::temp_dir().join("aiterm_ps");
    let _ = std::fs::create_dir_all(&temp_dir);
    let script_path = temp_dir.join("shell_integration.ps1");

    let script = r#"
# ── AITerm Shell Integration (PowerShell) ──
$global:__aiterm_orig_prompt = if (Test-Path Function:\prompt) { ${function:prompt} } else { $null }

function global:prompt {
    # Capture success/exit code FIRST — later statements would overwrite $?
    $wasSuccess = $?
    $origExit = $global:LASTEXITCODE
    $ec = if ($wasSuccess) { 0 } else { if ($origExit) { $origExit } else { 1 } }

    [Console]::Write("$([char]27)]133;D;$ec$([char]7)")
    [Console]::Write("$([char]27)]133;A$([char]7)")

    if ($global:__aiterm_orig_prompt) {
        & $global:__aiterm_orig_prompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }

    # Restore so user scripts are not affected by our prompt logic
    $global:LASTEXITCODE = $origExit
}
"#;
    let _ = std::fs::write(&script_path, script);

    ShellSpec {
        program,
        // -NoExit keeps the session interactive; -Command runs AFTER the user's profile
        // loads, so our prompt wrapper overrides whatever the profile set.
        args: vec![
            "-NoExit".into(),
            "-Command".into(),
            format!(". '{}'", script_path.display()),
        ],
        envs: vec![],
    }
}

/// Inject OSC 133 markers into cmd.exe via the PROMPT environment variable.
///
/// cmd.exe has no preexec/precmd hooks, so D is emitted unconditionally on every
/// prompt. The frontend ignores D when no block is running, so this is safe.
/// Exit codes cannot be embedded in cmd.exe's PROMPT, so D carries no exit code
/// (the frontend defaults to 0).
/// `$E` = ESC; `$E\` = ESC + backslash = ST (String Terminator for OSC).
#[cfg(windows)]
fn inject_cmd_integration() -> ShellSpec {
    ShellSpec {
        program: PathBuf::from("cmd.exe"),
        args: vec![],
        envs: vec![
            ("PROMPT".into(), "$E]133;D$E\\$E]133;A$E\\$P$G".into()),
        ],
    }
}

#[cfg(not(windows))]
pub fn unix_default_shell() -> Option<ShellSpec> {
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.is_empty() {
            return Some(inject_shell_integration(PathBuf::from(shell)));
        }
    }
    for candidate in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        if std::path::Path::new(candidate).exists() {
            return Some(inject_shell_integration(PathBuf::from(candidate)));
        }
    }
    None
}

/// Setup OS-specific shell integration hooks for block reporting (OSC 133).
fn inject_shell_integration(program: PathBuf) -> ShellSpec {
    let mut envs = vec![];
    let mut args = vec![];
    
    // For Zsh, override ZDOTDIR to inject our own startup files.
    if program.ends_with("zsh") {
        let temp_dir = std::env::temp_dir().join("aiterm_zsh");
        let _ = std::fs::create_dir_all(&temp_dir);

        // Preserve original ZDOTDIR so proxy files can source the real ones.
        if let Ok(orig) = std::env::var("ZDOTDIR") {
            envs.push(("AITERM_ORIG_ZDOTDIR".into(), orig));
        }

        // .zshenv — sources user's real .zshenv so PATH / env vars are correct.
        // zsh reads this for every invocation (interactive + non-interactive).
        let zshenv_content = r#"
# ── AITerm: proxy user's .zshenv ──
if [[ -n "$AITERM_ORIG_ZDOTDIR" && -f "$AITERM_ORIG_ZDOTDIR/.zshenv" ]]; then
  source "$AITERM_ORIG_ZDOTDIR/.zshenv"
elif [[ -f "$HOME/.zshenv" ]]; then
  source "$HOME/.zshenv"
fi
"#;
        let _ = std::fs::write(temp_dir.join(".zshenv"), zshenv_content);

        // .zprofile — sources user's real .zprofile (login shell init).
        let zprofile_content = r#"
# ── AITerm: proxy user's .zprofile ──
if [[ -n "$AITERM_ORIG_ZDOTDIR" && -f "$AITERM_ORIG_ZDOTDIR/.zprofile" ]]; then
  source "$AITERM_ORIG_ZDOTDIR/.zprofile"
elif [[ -f "$HOME/.zprofile" ]]; then
  source "$HOME/.zprofile"
fi
"#;
        let _ = std::fs::write(temp_dir.join(".zprofile"), zprofile_content);

        // .zshrc — sources user's real .zshrc first, then appends AITerm hooks
        // via add-zsh-hook so that oh-my-zsh / starship prompt hooks are preserved.
        // The guard variable __aiterm_cmd_running prevents the initial precmd
        // (which fires before any command is typed) from sending a false D marker.
        let zshrc_content = r#"
# ── AITerm: proxy user's .zshrc ──
if [[ -n "$AITERM_ORIG_ZDOTDIR" && -f "$AITERM_ORIG_ZDOTDIR/.zshrc" ]]; then
  source "$AITERM_ORIG_ZDOTDIR/.zshrc"
elif [[ -f "$HOME/.zshrc" ]]; then
  source "$HOME/.zshrc"
fi

# Disable prompt_sp to prevent an inverted % when we output OSC markers.
unsetopt prompt_sp 2>/dev/null || true

# ── AITerm Shell Integration (OSC 133) ──
# Use add-zsh-hook instead of overriding preexec/precmd directly so that
# oh-my-zsh, starship, and other prompt frameworks keep their own hooks.
autoload -Uz add-zsh-hook

__aiterm_cmd_running=0

__aiterm_preexec() {
  __aiterm_cmd_running=1
  printf '\x1b]133;C\x07'
}

__aiterm_precmd() {
  local ec=$?
  if [[ $__aiterm_cmd_running -eq 1 ]]; then
    __aiterm_cmd_running=0
    printf '\x1b]133;D;%s\x07' "$ec"
  fi
  printf '\x1b]133;A\x07'
}

add-zsh-hook preexec __aiterm_preexec
add-zsh-hook precmd __aiterm_precmd
"#;
        let _ = std::fs::write(temp_dir.join(".zshrc"), zshrc_content);

        envs.push(("ZDOTDIR".into(), temp_dir.to_string_lossy().into_owned()));
    }
    // For Bash, inject via --rcfile
    else if program.ends_with("bash") {
        let temp_dir = std::env::temp_dir().join("aiterm_bash");
        let _ = std::fs::create_dir_all(&temp_dir);
        let rcfile = temp_dir.join(".bashrc");
        
        let bashrc_content = r#"
# Source the user's real bashrc
if [[ -f "$HOME/.bashrc" ]]; then
  source "$HOME/.bashrc"
fi

# ── AITerm Shell Integration ──
__aiterm_cmd_running=0

__aiterm_preexec() {
  if [[ $__aiterm_cmd_running -eq 0 ]]; then
    __aiterm_cmd_running=1
    printf '\x1b]133;C\x07'
  fi
}
trap '__aiterm_preexec' DEBUG

__aiterm_precmd() {
  local ec=$?
  if [[ $__aiterm_cmd_running -eq 1 ]]; then
    __aiterm_cmd_running=0
    printf '\x1b]133;D;%s\x07' "$ec"
  fi
  printf '\x1b]133;A\x07'
}
PROMPT_COMMAND="__aiterm_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
"#;
        let _ = std::fs::write(&rcfile, bashrc_content);
        args.push("--rcfile".into());
        args.push(rcfile.to_string_lossy().into_owned());
    }

    ShellSpec {
        program,
        args,
        envs,
    }
}

#[allow(dead_code)]
fn which_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let full = dir.join(program);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shell_returns_something_on_supported_platforms() {
        // On CI/dev boxes we run this on, at least one of cmd.exe/bash/sh is available.
        let shell = default_shell();
        assert!(shell.is_some(), "expected a default shell on this platform");
    }

    #[cfg(windows)]
    #[test]
    fn windows_default_shell_returns_exe_path() {
        let shell = default_shell().expect("shell present");
        let name = shell
            .program
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        assert!(
            matches!(name, "pwsh.exe" | "powershell.exe" | "cmd.exe"),
            "unexpected shell chosen: {name}"
        );
    }

    #[test]
    fn which_on_path_finds_nothing_for_garbage_name() {
        assert!(which_on_path("definitely-not-a-real-binary-xyzzy").is_none());
    }
}
