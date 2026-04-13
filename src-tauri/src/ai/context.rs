//! Environment snapshot for AI prompts.
//!
//! Collects OS/shell/cwd from the PTY session, plus two optional rich-context
//! fields: recent terminal output (ring-buffered from PTY) and a directory
//! listing. Both are best-effort — failures degrade gracefully to None.

use std::path::{Path, PathBuf};

use crate::ai::EnvSnapshot;
use crate::pty::cd_parser::ShellVariant;
use crate::pty::PtyManager;

/// Build an enriched snapshot for a given PTY session.
pub fn snapshot(pty_manager: &PtyManager, session_id: &str) -> EnvSnapshot {
    let cwd = pty_manager
        .get_cwd(session_id)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let shell = pty_manager
        .get_shell_variant(session_id)
        .map(shell_variant_to_str)
        .unwrap_or_else(default_shell_name)
        .to_string();

    let recent_output = pty_manager
        .get_recent_output(session_id, 4096)
        .map(|s| trim_to_last_n_lines(&s, 50));

    let dir_listing = collect_dir_listing(&cwd);

    EnvSnapshot {
        os: std::env::consts::OS.to_string(),
        shell,
        cwd,
        recent_output,
        dir_listing,
    }
}

/// Test-visible helper so we can build a snapshot from already-resolved parts.
/// Context fields default to None — supply them explicitly in tests that need them.
pub fn snapshot_from_parts(os: &str, shell: &str, cwd: PathBuf) -> EnvSnapshot {
    EnvSnapshot {
        os: os.to_string(),
        shell: shell.to_string(),
        cwd,
        recent_output: None,
        dir_listing: None,
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

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

/// Keep only the last `n` non-empty lines of `s`.
fn trim_to_last_n_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// List up to 50 entries in `cwd`, sorted, with trailing `/` for dirs and `@`
/// for symlinks. Returns `None` on permission errors or empty directories.
fn collect_dir_listing(cwd: &Path) -> Option<String> {
    let read = std::fs::read_dir(cwd).ok()?;
    let mut names: Vec<String> = read
        .filter_map(|e| e.ok())
        .take(50)
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let suffix = match e.file_type() {
                Ok(t) if t.is_symlink() => "@",
                Ok(t) if t.is_dir() => "/",
                _ => "",
            };
            format!("{name}{suffix}")
        })
        .collect();

    if names.is_empty() {
        return None;
    }
    names.sort();
    Some(names.join("\n"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_from_parts_sets_all_fields() {
        let s = snapshot_from_parts("linux", "bash", PathBuf::from("/home/u"));
        assert_eq!(s.os, "linux");
        assert_eq!(s.shell, "bash");
        assert_eq!(s.cwd, PathBuf::from("/home/u"));
        assert!(s.recent_output.is_none());
        assert!(s.dir_listing.is_none());
    }

    #[test]
    fn shell_variant_mapping_is_stable() {
        assert_eq!(shell_variant_to_str(ShellVariant::Pwsh), "pwsh");
        assert_eq!(shell_variant_to_str(ShellVariant::Cmd), "cmd");
        assert_eq!(shell_variant_to_str(ShellVariant::Bash), "bash");
        assert_eq!(shell_variant_to_str(ShellVariant::Unknown), "unknown");
    }

    #[test]
    fn trim_to_last_n_lines_works() {
        let s = "a\nb\nc\nd\ne";
        assert_eq!(trim_to_last_n_lines(s, 3), "c\nd\ne");
        assert_eq!(trim_to_last_n_lines(s, 10), "a\nb\nc\nd\ne");
        assert_eq!(trim_to_last_n_lines(s, 0), "");
    }

    #[test]
    fn collect_dir_listing_returns_none_for_missing_dir() {
        assert!(collect_dir_listing(Path::new("/nonexistent/path/xyz")).is_none());
    }

    #[test]
    fn collect_dir_listing_returns_some_for_real_dir() {
        // Use the crate root, which always has files.
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let listing = collect_dir_listing(&manifest_dir);
        assert!(listing.is_some());
        let text = listing.unwrap();
        assert!(text.contains("src/") || text.contains("Cargo.toml"));
    }
}
