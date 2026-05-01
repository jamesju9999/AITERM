//! VCS (Version Control System) module.
//!
//! Provides repo detection, local git/svn operations, and GitHub API calls.

pub mod git;
pub mod svn;
pub mod types;

pub use types::*;

use std::process::Command;

use crate::config::types::VcsType;

pub struct VcsManager;

impl VcsManager {
    /// Detect whether `path` is inside a git or SVN working copy.
    /// Returns `VcsRepoInfo` with `connection_id = None` (caller fills it in).
    pub async fn detect_repo(path: &str) -> Result<VcsRepoInfo, String> {
        // Try git first
        let git_root = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(path)
            .output();

        if let Ok(out) = git_root {
            if out.status.success() {
                let root = String::from_utf8_lossy(&out.stdout).trim().to_string();

                // Get remote URL (best-effort)
                let remote_url = Command::new("git")
                    .args(["remote", "get-url", "origin"])
                    .current_dir(&root)
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|s| !s.is_empty());

                return Ok(VcsRepoInfo {
                    vcs_type: VcsType::Git,
                    root,
                    remote_url,
                    connection_id: None,
                });
            }
        }

        // Try SVN
        let svn_out = Command::new("svn")
            .args(["info", "--xml", "--non-interactive"])
            .current_dir(path)
            .output();

        if let Ok(out) = svn_out {
            if out.status.success() {
                let xml = String::from_utf8_lossy(&out.stdout).to_string();
                let root = extract_svn_wc_root(&xml).unwrap_or_else(|| path.to_string());
                let remote_url = extract_svn_url(&xml);

                return Ok(VcsRepoInfo {
                    vcs_type: VcsType::Svn,
                    root,
                    remote_url,
                    connection_id: None,
                });
            }
        }

        Err(format!(
            "No git or SVN repository found at or above '{path}'"
        ))
    }
}

fn extract_svn_wc_root(xml: &str) -> Option<String> {
    // <wcroot-abspath>...</wcroot-abspath>
    let start = xml.find("<wcroot-abspath>")? + "<wcroot-abspath>".len();
    let end = xml[start..].find("</wcroot-abspath>")? + start;
    Some(xml[start..end].to_string())
}

fn extract_svn_url(xml: &str) -> Option<String> {
    // <url>...</url>
    let start = xml.find("<url>")? + "<url>".len();
    let end = xml[start..].find("</url>")? + start;
    Some(xml[start..end].to_string())
}
