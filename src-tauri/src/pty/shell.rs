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
    for candidate in ["pwsh.exe", "powershell.exe", "cmd.exe"] {
        if which_on_path(candidate).is_some() {
            return Some(ShellSpec {
                program: PathBuf::from(candidate),
                args: vec![],
                envs: vec![],
            });
        }
    }
    None
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
    
    // For Zsh, override ZDOTDIR to inject our own .zshrc
    if program.ends_with("zsh") {
        let temp_dir = std::env::temp_dir().join("aiterm_zsh");
        let _ = std::fs::create_dir_all(&temp_dir);
        let zshrc_path = temp_dir.join(".zshrc");
        
        // The guard variable __aiterm_cmd_running prevents the initial precmd
        // (which fires before any command is typed) from sending a false D marker.
        let zshrc_content = r#"
# Source the user's real .zshrc if it exists
if [[ -n "$AITERM_ORIG_ZDOTDIR" && -f "$AITERM_ORIG_ZDOTDIR/.zshrc" ]]; then
  source "$AITERM_ORIG_ZDOTDIR/.zshrc"
elif [[ -f "$HOME/.zshrc" ]]; then
  source "$HOME/.zshrc"
fi

# Disable prompt_sp to prevent inverted % symbol from appearing when we output shell integration marks
unsetopt prompt_sp

# ── AITerm Shell Integration ──
__aiterm_cmd_running=0

preexec() {
  __aiterm_cmd_running=1
  printf '\x1b]133;C\x07'
}

precmd() {
  local ec=$?
  if [[ $__aiterm_cmd_running -eq 1 ]]; then
    __aiterm_cmd_running=0
    printf '\x1b]133;D;%s\x07' "$ec"
  fi
  printf '\x1b]133;A\x07'
}
"#;
        let _ = std::fs::write(&zshrc_path, zshrc_content);
        
        // Preserve original ZDOTDIR so the user's .zshrc in a custom location
        // can still be found.
        if let Ok(orig) = std::env::var("ZDOTDIR") {
            envs.push(("AITERM_ORIG_ZDOTDIR".into(), orig));
        }
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
