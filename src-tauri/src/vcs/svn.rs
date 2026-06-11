//! SVN operations via the `svn` command-line tool.

use std::process::Command;

use super::types::{BlameEntry, CommitEntry, VcsResult};

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
        let mut cmd = Command::new("svn");
        cmd.arg("--version");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    }

    fn svn(&self, args: &[String]) -> Result<String, String> {
        let mut cmd = Command::new("svn");
        cmd.args(args).current_dir(&self.working_copy_root);
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
