//! Environment snapshot for AI prompts. M1 minimal: os + shell + cwd.
//! Historical command context is intentionally deferred to M5.

use std::path::PathBuf;

use crate::ai::EnvSnapshot;
use crate::pty::cd_parser::ShellVariant;
use crate::pty::PtyManager;

/// Build a snapshot for a given PTY session. Falls back to the process cwd
/// if the manager has no record of the session (should not happen in normal
/// use, but we degrade gracefully).
pub fn snapshot(pty_manager: &PtyManager, session_id: &str) -> EnvSnapshot {
    let cwd = pty_manager
        .get_cwd(session_id)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let shell = pty_manager
        .get_shell_variant(session_id)
        .map(shell_variant_to_str)
        .unwrap_or_else(|| default_shell_name())
        .to_string();
    EnvSnapshot {
        os: std::env::consts::OS.to_string(),
        shell,
        cwd,
    }
}

/// Test-visible helper so we can build a snapshot from already-resolved parts.
pub fn snapshot_from_parts(os: &str, shell: &str, cwd: PathBuf) -> EnvSnapshot {
    EnvSnapshot {
        os: os.to_string(),
        shell: shell.to_string(),
        cwd,
    }
}

fn shell_variant_to_str(v: ShellVariant) -> &'static str {
    match v {
        ShellVariant::Pwsh => "pwsh",
        ShellVariant::Cmd => "cmd",
        ShellVariant::Bash => "bash",
        ShellVariant::Unknown => "unknown",
    }
}

fn default_shell_name() -> &'static str {
    #[cfg(windows)]
    { "pwsh" }
    #[cfg(not(windows))]
    { "bash" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_from_parts_sets_all_fields() {
        let s = snapshot_from_parts("linux", "bash", PathBuf::from("/home/u"));
        assert_eq!(s.os, "linux");
        assert_eq!(s.shell, "bash");
        assert_eq!(s.cwd, PathBuf::from("/home/u"));
    }

    #[test]
    fn shell_variant_mapping_is_stable() {
        assert_eq!(shell_variant_to_str(ShellVariant::Pwsh), "pwsh");
        assert_eq!(shell_variant_to_str(ShellVariant::Cmd), "cmd");
        assert_eq!(shell_variant_to_str(ShellVariant::Bash), "bash");
        assert_eq!(shell_variant_to_str(ShellVariant::Unknown), "unknown");
    }
}
