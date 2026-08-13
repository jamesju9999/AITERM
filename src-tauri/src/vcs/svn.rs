//! SVN operations via the `svn` command-line tool.

use std::ffi::OsString;
use std::process::Command;
#[cfg(windows)]
use std::path::PathBuf;
use std::sync::OnceLock;

use super::types::{BlameEntry, CommitEntry, VcsResult};

/// Resolves the `svn` executable, falling back to the user's login-shell PATH
/// (macOS/Linux) or common install locations (Windows) when it isn't visible
/// on this process's own PATH.
///
/// AITerm's own process PATH is captured once at launch time (from launchd
/// on macOS, or the parent process on Windows) and never picks up entries
/// added afterward by Homebrew/winget/etc. — restarting the app doesn't help
/// unless it's re-launched from a shell that already has the updated PATH.
/// The embedded terminal doesn't hit this because it runs a real login shell
/// that re-sources the user's profile (see `pty/shell.rs`); this mirrors
/// that fallback for one-off `svn` invocations.
pub fn svn_program() -> &'static OsString {
    static PROGRAM: OnceLock<OsString> = OnceLock::new();
    PROGRAM.get_or_init(|| {
        if on_current_path("svn") {
            return OsString::from("svn");
        }
        match fallback_svn_path() {
            Some(found) => found.into_os_string(),
            None => OsString::from("svn"),
        }
    })
}

fn on_current_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let exe_name = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_string()
    };
    std::env::split_paths(&path).any(|dir| dir.join(&exe_name).is_file())
}

#[cfg(windows)]
fn fallback_svn_path() -> Option<PathBuf> {
    let bases: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"]
        .iter()
        .filter_map(|var| std::env::var_os(var))
        .map(PathBuf::from)
        .collect();
    let install_dirs = ["SlikSvn", "TortoiseSVN", "Subversion"];
    for base in &bases {
        for dir in &install_dirs {
            let candidate = base.join(dir).join("bin").join("svn.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn fallback_svn_path() -> Option<std::path::PathBuf> {
    // Ask the user's login shell for its PATH — this sources .zprofile /
    // .bash_profile, which is where Homebrew's shellenv (and similar PATH
    // extensions) typically live.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let output = std::process::Command::new(&shell)
        .arg("-lc")
        .arg("echo -n \"$PATH\"")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path_str = String::from_utf8_lossy(&output.stdout);
    std::env::split_paths(path_str.trim())
        .map(|dir| dir.join("svn"))
        .find(|candidate| candidate.is_file())
}

pub struct SvnClient {
    pub working_copy_root: String,
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl SvnClient {
    pub fn new(
        working_copy_root: String,
        url: String,
        username: Option<String>,
        password: Option<String>,
    ) -> Self {
        Self {
            working_copy_root,
            url,
            username,
            password,
        }
    }

    pub async fn log(
        &self,
        path: Option<&str>,
        max_count: u32,
    ) -> Result<VcsResult, String> {
        if !self.is_available() {
            return Ok(VcsResult::SvnNotInstalled);
        }

        let limit = max_count.max(1).min(200);
        let mut args = vec![
            "log".to_string(),
            "--xml".to_string(),
            format!("--limit={limit}"),
            "--non-interactive".to_string(),
        ];
        self.push_auth(&mut args);
        if let Some(p) = path {
            args.push(p.to_string());
        }

        let out = self.svn(&args)?;
        let commits = parse_svn_log_xml(&out)?;
        let truncated = commits.len() == limit as usize;
        Ok(VcsResult::Log { commits, truncated })
    }

    pub async fn diff(&self, revision: &str) -> Result<VcsResult, String> {
        if !self.is_available() {
            return Ok(VcsResult::SvnNotInstalled);
        }

        let mut args = vec![
            "diff".to_string(),
            format!("-c{revision}"),
            "--non-interactive".to_string(),
        ];
        self.push_auth(&mut args);

        let out = self.svn(&args)?;
        Ok(VcsResult::Diff {
            content: out,
            revision: revision.to_string(),
        })
    }

    pub async fn blame(&self, path: &str) -> Result<VcsResult, String> {
        if !self.is_available() {
            return Ok(VcsResult::SvnNotInstalled);
        }

        let mut args = vec![
            "blame".to_string(),
            "--xml".to_string(),
            "--non-interactive".to_string(),
            path.to_string(),
        ];
        self.push_auth(&mut args);

        let out = self.svn(&args)?;
        let lines = parse_svn_blame_xml(&out, path)?;
        Ok(VcsResult::Blame { lines })
    }

    /// Returns raw `svn info` text (used by VcsManager to detect SVN root/URL).
    pub async fn info(&self) -> Result<String, String> {
        if !self.is_available() {
            return Err("svn not installed".into());
        }
        let mut args = vec!["info".to_string(), "--xml".to_string(), "--non-interactive".to_string()];
        self.push_auth(&mut args);
        self.svn(&args)
    }

    pub async fn revert(&self, paths: &[String]) -> Result<VcsResult, String> {
        if !self.is_available() {
            return Ok(VcsResult::SvnNotInstalled);
        }

        let mut args = vec!["revert".to_string(), "--non-interactive".to_string()];
        self.push_auth(&mut args);
        for p in paths {
            args.push(p.clone());
        }

        self.svn(&args)?;
        Ok(VcsResult::WriteSuccess {
            operation: "svn_revert".to_string(),
            detail: format!("Reverted {} path(s)", paths.len()),
        })
    }

    pub async fn commit(&self, message: &str, paths: &[String]) -> Result<VcsResult, String> {
        if !self.is_available() {
            return Ok(VcsResult::SvnNotInstalled);
        }

        let mut args = vec![
            "commit".to_string(),
            format!("-m{message}"),
            "--non-interactive".to_string(),
        ];
        self.push_auth(&mut args);
        for p in paths {
            args.push(p.clone());
        }

        let out = self.svn(&args)?;
        Ok(VcsResult::WriteSuccess {
            operation: "svn_commit".to_string(),
            detail: out.trim().to_string(),
        })
    }

    pub async fn update(&self, path: Option<&str>) -> Result<VcsResult, String> {
        if !self.is_available() {
            return Ok(VcsResult::SvnNotInstalled);
        }

        let mut args = vec!["update".to_string(), "--non-interactive".to_string()];
        self.push_auth(&mut args);
        if let Some(p) = path {
            args.push(p.to_string());
        }

        let out = self.svn(&args)?;
        Ok(VcsResult::WriteSuccess {
            operation: "svn_update".to_string(),
            detail: out.trim().to_string(),
        })
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn is_available(&self) -> bool {
        let mut cmd = Command::new(svn_program());
        cmd.arg("--version");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    }

    fn svn(&self, args: &[String]) -> Result<String, String> {
        let mut cmd = Command::new(svn_program());
        cmd.args(args)
            // Trust unknown-CA server certs (e.g. internal/self-signed) since
            // --non-interactive can't show the usual accept-cert prompt.
            .arg("--trust-server-cert-failures=unknown-ca")
            .current_dir(&self.working_copy_root);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let out = cmd.output()
            .map_err(|e| format!("svn exec error: {e}"))?;

        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).to_string())
        }
    }

    fn push_auth(&self, args: &mut Vec<String>) {
        if let Some(u) = &self.username {
            args.push("--username".to_string());
            args.push(u.clone());
        }
        if let Some(p) = &self.password {
            args.push("--password".to_string());
            args.push(p.clone());
        }
    }
}

// ── SVN XML parsers ───────────────────────────────────────────────────────────

fn parse_svn_log_xml(xml: &str) -> Result<Vec<CommitEntry>, String> {
    let mut commits = Vec::new();
    // Minimal XML parsing without pulling in an XML crate.
    // Each <logentry> block looks like:
    // <logentry revision="N">
    //   <author>...</author>
    //   <date>...</date>
    //   <msg>...</msg>
    // </logentry>
    for block in xml.split("<logentry") {
        if !block.contains("revision=") {
            continue;
        }
        let revision = extract_attr(block, "revision").unwrap_or_default();
        let author = extract_tag(block, "author").unwrap_or_default();
        let date = extract_tag(block, "date").unwrap_or_default();
        let message = extract_tag(block, "msg").unwrap_or_default();

        if revision.is_empty() {
            continue;
        }

        commits.push(CommitEntry {
            revision,
            author,
            date,
            message,
            files_changed: vec![], // SVN log --xml doesn't include paths by default
        });
    }
    Ok(commits)
}

fn parse_svn_blame_xml(xml: &str, _path: &str) -> Result<Vec<BlameEntry>, String> {
    let mut entries = Vec::new();
    let mut line_number: u32 = 0;

    for block in xml.split("<entry") {
        if !block.contains("line-number=") {
            continue;
        }
        line_number += 1;
        let revision = extract_tag(block, "revision").unwrap_or_default();
        let author = extract_tag(block, "author").unwrap_or_default();
        let date = extract_tag(block, "date").unwrap_or_default();

        entries.push(BlameEntry {
            line_number,
            revision,
            author,
            date,
            content: String::new(), // SVN blame XML doesn't include content
        });
    }
    Ok(entries)
}

/// Extract the value of a simple XML attribute like `key="value"`.
fn extract_attr(s: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = s.find(&needle)? + needle.len();
    let end = s[start..].find('"')? + start;
    Some(s[start..end].to_string())
}

/// Extract inner text of a simple XML tag (no nesting).
fn extract_tag(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&close)? + start;
    Some(decode_xml_entities(&s[start..end]))
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_svn_log_basic() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<log>
<logentry revision="42">
<author>alice</author>
<date>2024-01-01T00:00:00.000000Z</date>
<msg>Fix bug</msg>
</logentry>
</log>"#;
        let commits = parse_svn_log_xml(xml).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].revision, "42");
        assert_eq!(commits[0].author, "alice");
        assert_eq!(commits[0].message, "Fix bug");
    }

    #[test]
    fn xml_entity_decoding() {
        assert_eq!(decode_xml_entities("foo &amp; bar"), "foo & bar");
        assert_eq!(decode_xml_entities("&lt;tag&gt;"), "<tag>");
    }
}
