use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
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
            });
        }
    }
    None
}

#[cfg(not(windows))]
fn unix_default_shell() -> Option<ShellSpec> {
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.is_empty() {
            return Some(ShellSpec {
                program: PathBuf::from(shell),
                args: vec![],
            });
        }
    }
    for candidate in ["/bin/bash", "/bin/sh"] {
        if std::path::Path::new(candidate).exists() {
            return Some(ShellSpec {
                program: PathBuf::from(candidate),
                args: vec![],
            });
        }
    }
    None
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
