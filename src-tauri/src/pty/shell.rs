use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
    /// 要從繼承環境中移除的變數名。
    pub env_removals: Vec<String>,
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
pub(crate) fn inject_powershell_integration(program: PathBuf) -> ShellSpec {
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

    $rendered = if ($global:__aiterm_orig_prompt) {
        & $global:__aiterm_orig_prompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }

    # Restore so user scripts are not affected by our prompt logic
    $global:LASTEXITCODE = $origExit

    # B marker: appended to the actual rendered prompt text, so it's
    # guaranteed to arrive AFTER the visible prompt characters — unlike A
    # (printed above), which fires before this function's return value is
    # ever echoed to the screen.
    "$rendered$([char]27)]133;B$([char]7)"
}

# C marker: PowerShell has no preexec-equivalent hook, so this overrides the
# Enter key itself. AcceptLine (or the user's own Enter binding, preserved
# below) runs FIRST — that's what produces the newline echo — and C is
# printed AFTER it, once the cursor has actually moved to the new line. This
# matches zsh/bash's ordering (preexec fires after the line editor's own
# newline echo), which recoverUntrackedCommand's cursor-position math in
# useTerminalBlocks.ts relies on being consistent across all three shells.
$global:__aiterm_orig_enter_handler = (Get-PSReadLineKeyHandler -Chord Enter -Bound `
    -ErrorAction SilentlyContinue).Function

Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
    param($key, $arg)
    if ($global:__aiterm_orig_enter_handler -and
        $global:__aiterm_orig_enter_handler -ne "AcceptLine") {
        [Microsoft.PowerShell.PSConsoleReadLine]::($global:__aiterm_orig_enter_handler)($key, $arg)
    } else {
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine($key, $arg)
    }
    [Console]::Write("$([char]27)]133;C$([char]7)")
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
        envs: vec![
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
            ("PYTHONIOENCODING".into(), "utf-8".into()),
        ],
        env_removals: Vec::new(),
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
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
            ("PYTHONIOENCODING".into(), "utf-8".into()),
        ],
        env_removals: Vec::new(),
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
#[cfg(not(windows))]
pub(crate) fn inject_shell_integration(program: PathBuf) -> ShellSpec {
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

        // .zshrc — sources user's real .zprofile first (since this is a non-login
        // interactive shell, zsh won't read .zprofile automatically; sourcing it
        // here ensures PATH entries like Homebrew are available), then .zshrc.
        // Uses add-zsh-hook so that oh-my-zsh / starship prompt hooks are preserved.
        // The guard variable __aiterm_cmd_running prevents the initial precmd
        // (which fires before any command is typed) from sending a false D marker.
        let zshrc_content = r#"
# ── AITerm: proxy user's .zshrc ──
# Source .zprofile first: this shell is interactive non-login, so zsh skips
# .zprofile normally. We source it explicitly so that Homebrew PATH (and
# other login-shell env setup) is available exactly as in a login shell.
if [[ -n "$AITERM_ORIG_ZDOTDIR" && -f "$AITERM_ORIG_ZDOTDIR/.zprofile" ]]; then
  source "$AITERM_ORIG_ZDOTDIR/.zprofile"
elif [[ -f "$HOME/.zprofile" ]]; then
  source "$HOME/.zprofile"
fi

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
  local b_marker=$'%{\e]133;B\a%}'
  if [[ "$PS1" != *"$b_marker"* ]]; then
    PS1="${PS1}${b_marker}"
  fi
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

__aiterm_append_b_marker() {
  local b_marker=$'\[\e]133;B\a\]'
  if [[ "$PS1" != *"$b_marker"* ]]; then
    PS1="${PS1}${b_marker}"
  fi
}

PROMPT_COMMAND="__aiterm_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND};__aiterm_append_b_marker"
"#;
        let _ = std::fs::write(&rcfile, bashrc_content);
        args.push("--rcfile".into());
        args.push(rcfile.to_string_lossy().into_owned());
    }

    // Ensure color output works regardless of how AITerm was launched.
    // When launched as a .app from Dock/Finder, launchd does not set TERM,
    // so programs default to no-color output.
    envs.push(("TERM".into(), "xterm-256color".into()));
    envs.push(("COLORTERM".into(), "truecolor".into()));

    // Ensure UTF-8 locale so multi-byte characters (CJK, etc.) are not mangled.
    // Dock/Finder launches do not inherit the login-shell LANG; without it the
    // C locale is used, which replaces non-ASCII bytes with '?'.
    if std::env::var("LANG").unwrap_or_default().is_empty() {
        envs.push(("LANG".into(), "en_US.UTF-8".into()));
        envs.push(("LC_ALL".into(), "en_US.UTF-8".into()));
    }

    ShellSpec {
        program,
        args,
        envs,
        env_removals: Vec::new(),
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

    #[cfg(windows)]
    #[test]
    fn powershell_integration_emits_c_via_enter_override_and_b_after_rendered_prompt() {
        let spec = inject_powershell_integration(PathBuf::from("pwsh.exe"));
        let script_path = std::env::temp_dir().join("aiterm_ps").join("shell_integration.ps1");
        let content = std::fs::read_to_string(&script_path).expect("script should have been written");

        assert!(
            content.contains("Set-PSReadLineKeyHandler -Chord Enter"),
            "expected an Enter key handler override to emit the C marker"
        );
        assert!(
            content.contains(r#"[Console]::Write("$([char]27)]133;C$([char]7)")"#),
            "expected the Enter override to emit the C marker after AcceptLine runs"
        );
        assert!(
            content.contains(r#""$rendered$([char]27)]133;B$([char]7)""#),
            "expected the prompt function to append a B marker after the rendered prompt text"
        );

        // spec.program 本身已經被既有的 windows_default_shell_returns_exe_path 測試涵蓋，
        // 這裡只是避免 unused 警告。
        assert_eq!(spec.program, PathBuf::from("pwsh.exe"));
    }

    #[test]
    fn which_on_path_finds_nothing_for_garbage_name() {
        assert!(which_on_path("definitely-not-a-real-binary-xyzzy").is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn zsh_integration_appends_b_marker_to_ps1_in_precmd() {
        let _ = inject_shell_integration(PathBuf::from("/bin/zsh"));
        let content = std::fs::read_to_string(
            std::env::temp_dir().join("aiterm_zsh").join(".zshrc"),
        )
        .expect("zshrc should have been written");

        assert!(
            content.contains(r#"printf '\x1b]133;A\x07'"#),
            "expected the existing A marker printf to still be present"
        );
        assert!(
            content.contains(r#"local b_marker=$'%{\e]133;B\a%}'"#),
            "expected the precmd hook to compute a B marker to append to PS1"
        );
        assert!(
            content.contains(r#"if [[ "$PS1" != *"$b_marker"* ]]; then"#),
            "expected a guard against appending the B marker more than once"
        );
        assert!(
            content.contains(r#"PS1="${PS1}${b_marker}""#),
            "expected PS1 to be extended with the B marker"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn bash_integration_appends_b_marker_to_ps1_after_prompt_command_chain() {
        let spec = inject_shell_integration(PathBuf::from("/bin/bash"));
        let rcfile_idx = spec
            .args
            .iter()
            .position(|a| a == "--rcfile")
            .expect("bash spec should pass --rcfile");
        let rcfile = PathBuf::from(&spec.args[rcfile_idx + 1]);
        let content = std::fs::read_to_string(rcfile).expect("bashrc should have been written");

        assert!(
            content.contains(r#"printf '\x1b]133;A\x07'"#),
            "expected the existing A marker printf to still be present"
        );
        assert!(
            content.contains("__aiterm_append_b_marker() {"),
            "expected a dedicated function for appending the B marker, kept separate from __aiterm_precmd"
        );
        assert!(
            content.contains(r#"local b_marker=$'\[\e]133;B\a\]'"#),
            "expected the B marker append function to compute a B marker"
        );
        assert!(
            content.contains(r#"if [[ "$PS1" != *"$b_marker"* ]]; then"#),
            "expected a guard against appending the B marker more than once"
        );
        assert!(
            content.contains(r#"PS1="${PS1}${b_marker}""#),
            "expected PS1 to be extended with the B marker"
        );
        assert!(
            content.contains(
                r#"PROMPT_COMMAND="__aiterm_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND};__aiterm_append_b_marker""#
            ),
            "expected __aiterm_append_b_marker to run LAST in PROMPT_COMMAND — after any \
             framework's own PROMPT_COMMAND entries (which __aiterm_precmd is prepended \
             before, to capture $? correctly) have already finalized PS1 for this cycle, \
             so the B marker survives even if a framework fully reassigns PS1 rather than \
             appending to it"
        );
    }
}
