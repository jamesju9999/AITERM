//! Local git operations (via `git` subprocess) and GitHub API calls (via reqwest).

use std::process::Command;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;

use super::types::{
    BlameEntry, BranchEntry, CommitEntry, IssueEntry, PrEntry, VcsResult, WorkflowRun,
};

pub struct GitClient {
    pub repo_root: String,
    pub token: Option<String>,
}

impl GitClient {
    pub fn new(repo_root: String, token: Option<String>) -> Self {
        Self { repo_root, token }
    }

    // ── Local git operations ─────────────────────────────────────────────────

    pub async fn log(
        &self,
        path: Option<&str>,
        author: Option<&str>,
        since: Option<&str>,
        max_count: u32,
    ) -> Result<VcsResult, String> {
        let max = max_count.max(1).min(200);
        let mut args = vec![
            "log".to_string(),
            format!("--max-count={max}"),
            "--format=%H|%an|%ai|%s".to_string(),
        ];
        if let Some(a) = author {
            args.push(format!("--author={a}"));
        }
        if let Some(s) = since {
            args.push(format!("--since={s}"));
        }
        args.push("--".to_string());
        if let Some(p) = path {
            args.push(p.to_string());
        }

        let out = self.git(&args)?;
        let mut commits = Vec::new();
        for line in out.lines().filter(|l| !l.is_empty()) {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 4 {
                continue;
            }
            let sha = parts[0].to_string();
            let files = self.diff_tree_files(&sha).unwrap_or_default();
            commits.push(CommitEntry {
                revision: sha,
                author: parts[1].to_string(),
                date: parts[2].to_string(),
                message: parts[3].to_string(),
                files_changed: files,
            });
        }
        let truncated = commits.len() == max as usize;
        Ok(VcsResult::Log { commits, truncated })
    }

    pub async fn show(&self, revision: &str) -> Result<VcsResult, String> {
        let out = self.git(&["show".to_string(), revision.to_string()])?;
        Ok(VcsResult::Diff {
            content: out,
            revision: revision.to_string(),
        })
    }

    pub async fn blame(&self, path: &str) -> Result<VcsResult, String> {
        let out = self.git(&[
            "blame".to_string(),
            "--porcelain".to_string(),
            path.to_string(),
        ])?;
        let lines = parse_blame_porcelain(&out);
        Ok(VcsResult::Blame { lines })
    }

    pub async fn branch_list(&self) -> Result<VcsResult, String> {
        let out = self.git(&[
            "branch".to_string(),
            "-a".to_string(),
            "--format=%(refname:short)|%(HEAD)".to_string(),
        ])?;
        let mut branches = Vec::new();
        for line in out.lines().filter(|l| !l.is_empty()) {
            let parts: Vec<&str> = line.splitn(2, '|').collect();
            if parts.is_empty() {
                continue;
            }
            let name = parts[0].trim().to_string();
            let is_current = parts.get(1).map(|s| s.trim() == "*").unwrap_or(false);
            let is_remote = name.starts_with("remotes/") || name.contains('/');
            branches.push(BranchEntry {
                name,
                is_current,
                is_remote,
            });
        }
        Ok(VcsResult::Branches { branches })
    }

    pub async fn revert(&self, revision: &str) -> Result<VcsResult, String> {
        self.git(&[
            "revert".to_string(),
            "--no-edit".to_string(),
            revision.to_string(),
        ])?;
        Ok(VcsResult::WriteSuccess {
            operation: "revert".to_string(),
            detail: format!("Reverted commit {revision}"),
        })
    }

    pub async fn cherry_pick(&self, revision: &str) -> Result<VcsResult, String> {
        self.git(&["cherry-pick".to_string(), revision.to_string()])?;
        Ok(VcsResult::WriteSuccess {
            operation: "cherry_pick".to_string(),
            detail: format!("Cherry-picked commit {revision}"),
        })
    }

    pub async fn create_branch(&self, name: &str, from: Option<&str>) -> Result<VcsResult, String> {
        let mut args = vec!["checkout".to_string(), "-b".to_string(), name.to_string()];
        if let Some(f) = from {
            args.push(f.to_string());
        }
        self.git(&args)?;
        Ok(VcsResult::WriteSuccess {
            operation: "create_branch".to_string(),
            detail: format!("Created and checked out branch '{name}'"),
        })
    }

    pub async fn delete_branch(&self, name: &str) -> Result<VcsResult, String> {
        self.git(&["branch".to_string(), "-d".to_string(), name.to_string()])?;
        Ok(VcsResult::WriteSuccess {
            operation: "delete_branch".to_string(),
            detail: format!("Deleted branch '{name}'"),
        })
    }

    pub async fn checkout_branch(&self, name: &str) -> Result<VcsResult, String> {
        self.git(&["checkout".to_string(), name.to_string()])?;
        Ok(VcsResult::WriteSuccess {
            operation: "checkout_branch".to_string(),
            detail: format!("Checked out branch '{name}'"),
        })
    }

    // ── GitHub API operations ────────────────────────────────────────────────

    pub async fn pr_list(&self, state: Option<&str>) -> Result<VcsResult, String> {
        let token = self.require_token(2)?;
        let (owner, repo) = self.parse_remote()?;
        let state_val = state.unwrap_or("open");
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls?state={state_val}&per_page=30"
        );

        let resp: Vec<GhPr> = self
            .gh_get(&token, &url)
            .await?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        let prs = resp
            .into_iter()
            .map(|p| PrEntry {
                number: p.number,
                title: p.title,
                author: p.user.login,
                state: p.state,
                url: p.html_url,
                updated_at: p.updated_at,
            })
            .collect();

        Ok(VcsResult::PrList { prs })
    }

    pub async fn issue_list(&self, state: Option<&str>) -> Result<VcsResult, String> {
        let token = self.require_token(2)?;
        let (owner, repo) = self.parse_remote()?;
        let state_val = state.unwrap_or("open");
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/issues?state={state_val}&per_page=30"
        );

        let resp: Vec<GhIssue> = self
            .gh_get(&token, &url)
            .await?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        // Filter out pull requests (GitHub issues endpoint returns both)
        let issues = resp
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .map(|i| IssueEntry {
                number: i.number,
                title: i.title,
                author: i.user.login,
                state: i.state,
                url: i.html_url,
                created_at: i.created_at,
            })
            .collect();

        Ok(VcsResult::IssueList { issues })
    }

    pub async fn actions_list(&self) -> Result<VcsResult, String> {
        let token = self.require_token(2)?;
        let (owner, repo) = self.parse_remote()?;
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/actions/runs?per_page=20"
        );

        #[derive(Deserialize)]
        struct RunsResp {
            workflow_runs: Vec<GhRun>,
        }

        let resp: RunsResp = self
            .gh_get(&token, &url)
            .await?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        let runs = resp
            .workflow_runs
            .into_iter()
            .map(|r| WorkflowRun {
                id: r.id,
                name: r.name,
                status: r.status,
                conclusion: r.conclusion,
                created_at: r.created_at,
                html_url: r.html_url,
            })
            .collect();

        Ok(VcsResult::ActionsList { runs })
    }

    pub async fn create_pr(
        &self,
        title: &str,
        head: &str,
        base: &str,
        body: Option<&str>,
    ) -> Result<VcsResult, String> {
        let token = self.require_token(3)?;
        let (owner, repo) = self.parse_remote()?;
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls");

        let payload = serde_json::json!({
            "title": title,
            "head": head,
            "base": base,
            "body": body.unwrap_or(""),
        });

        let resp = self
            .gh_post(&token, &url, &payload)
            .await?;

        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let pr_url = json["html_url"].as_str().unwrap_or("").to_string();
        let number = json["number"].as_u64().unwrap_or(0);

        Ok(VcsResult::WriteSuccess {
            operation: "create_pr".to_string(),
            detail: format!("Created PR #{number}: {pr_url}"),
        })
    }

    pub async fn merge_pr(&self, pr_number: u64) -> Result<VcsResult, String> {
        let token = self.require_token(3)?;
        let (owner, repo) = self.parse_remote()?;
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls/{pr_number}/merge"
        );

        self.gh_put(&token, &url, &serde_json::json!({})).await?;

        Ok(VcsResult::WriteSuccess {
            operation: "merge_pr".to_string(),
            detail: format!("Merged PR #{pr_number}"),
        })
    }

    pub async fn create_issue(
        &self,
        title: &str,
        body: Option<&str>,
    ) -> Result<VcsResult, String> {
        let token = self.require_token(3)?;
        let (owner, repo) = self.parse_remote()?;
        let url = format!("https://api.github.com/repos/{owner}/{repo}/issues");

        let payload = serde_json::json!({
            "title": title,
            "body": body.unwrap_or(""),
        });

        let resp = self.gh_post(&token, &url, &payload).await?;
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let issue_url = json["html_url"].as_str().unwrap_or("").to_string();
        let number = json["number"].as_u64().unwrap_or(0);

        Ok(VcsResult::WriteSuccess {
            operation: "create_issue".to_string(),
            detail: format!("Created issue #{number}: {issue_url}"),
        })
    }

    pub async fn trigger_workflow(
        &self,
        workflow_id: &str,
        ref_name: &str,
    ) -> Result<VcsResult, String> {
        let token = self.require_token(3)?;
        let (owner, repo) = self.parse_remote()?;
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/actions/workflows/{workflow_id}/dispatches"
        );

        let payload = serde_json::json!({ "ref": ref_name });
        self.gh_post(&token, &url, &payload).await?;

        Ok(VcsResult::WriteSuccess {
            operation: "trigger_workflow".to_string(),
            detail: format!("Triggered workflow '{workflow_id}' on ref '{ref_name}'"),
        })
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn git(&self, args: &[String]) -> Result<String, String> {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(&self.repo_root);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let out = cmd.output()
            .map_err(|e| format!("git exec error: {e}"))?;

        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).to_string())
        }
    }

    fn diff_tree_files(&self, sha: &str) -> Result<Vec<String>, String> {
        let out = self.git(&[
            "diff-tree".to_string(),
            "--no-commit-id".to_string(),
            "-r".to_string(),
            "--name-only".to_string(),
            sha.to_string(),
        ])?;
        Ok(out
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }

    /// Parse remote URL and extract (owner, repo).
    /// Supports:
    ///   https://github.com/owner/repo
    ///   https://github.com/owner/repo.git
    ///   git@github.com:owner/repo.git
    fn parse_remote(&self) -> Result<(String, String), String> {
        let url = self
            .git(&["remote".to_string(), "get-url".to_string(), "origin".to_string()])
            .map(|s| s.trim().to_string())
            .map_err(|_| "No git remote 'origin' configured".to_string())?;

        parse_github_url(&url)
            .ok_or_else(|| format!("Cannot parse GitHub owner/repo from remote URL: {url}"))
    }

    fn require_token(&self, level: u8) -> Result<String, String> {
        self.token
            .clone()
            .ok_or_else(|| format!("no_token:{level}"))
    }

    fn gh_headers(&self, token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static("AITerm"));
        headers
    }

    async fn gh_get(
        &self,
        token: &str,
        url: &str,
    ) -> Result<reqwest::Response, String> {
        let client = reqwest::Client::new();
        let resp = client
            .get(url)
            .headers(self.gh_headers(token))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("GitHub API error {status}: {body}"));
        }
        Ok(resp)
    }

    async fn gh_post(
        &self,
        token: &str,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post(url)
            .headers(self.gh_headers(token))
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("GitHub API error {status}: {text}"));
        }
        Ok(resp)
    }

    async fn gh_put(
        &self,
        token: &str,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, String> {
        let client = reqwest::Client::new();
        let resp = client
            .put(url)
            .headers(self.gh_headers(token))
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("GitHub API error {status}: {text}"));
        }
        Ok(resp)
    }
}

/// Parse `https://github.com/owner/repo[.git]` or `git@github.com:owner/repo[.git]`
/// into `(owner, repo)`.
pub fn parse_github_url(url: &str) -> Option<(String, String)> {
    let url = url.trim().trim_end_matches('/');

    // HTTPS format: https://github.com/owner/repo[.git]
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        return split_owner_repo(rest);
    }
    if let Some(rest) = url.strip_prefix("http://github.com/") {
        return split_owner_repo(rest);
    }

    // SSH format: git@github.com:owner/repo[.git]
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        return split_owner_repo(rest);
    }

    None
}

fn split_owner_repo(s: &str) -> Option<(String, String)> {
    let s = s.trim_end_matches(".git");
    let mut parts = s.splitn(2, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

// ── Blame porcelain parser ────────────────────────────────────────────────────

fn parse_blame_porcelain(output: &str) -> Vec<BlameEntry> {
    let mut entries = Vec::new();
    let mut lines = output.lines().peekable();

    while let Some(line) = lines.next() {
        // Commit header line: "<sha> <orig_line> <final_line> [<num_lines>]"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 || parts[0].len() != 40 {
            continue;
        }
        let sha = parts[0].to_string();
        let line_number: u32 = parts[2].parse().unwrap_or(0);

        let mut author = String::new();
        let mut date = String::new();
        let mut content = String::new();

        // Read header key-value pairs until the content line (starts with '\t')
        while let Some(hdr) = lines.peek() {
            if hdr.starts_with('\t') {
                content = lines.next().unwrap().trim_start_matches('\t').to_string();
                break;
            }
            let hdr = lines.next().unwrap();
            if let Some(val) = hdr.strip_prefix("author ") {
                author = val.to_string();
            } else if let Some(val) = hdr.strip_prefix("author-time ") {
                date = val.to_string();
            }
        }

        entries.push(BlameEntry {
            line_number,
            revision: sha,
            author,
            date,
            content,
        });
    }

    entries
}

// ── GitHub JSON response shapes ───────────────────────────────────────────────

#[derive(Deserialize)]
struct GhUser {
    login: String,
}

#[derive(Deserialize)]
struct GhPr {
    number: u64,
    title: String,
    user: GhUser,
    state: String,
    html_url: String,
    updated_at: String,
}

#[derive(Deserialize)]
struct GhIssue {
    number: u64,
    title: String,
    user: GhUser,
    state: String,
    html_url: String,
    created_at: String,
    pull_request: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct GhRun {
    id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
    created_at: String,
    html_url: String,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_https_url() {
        assert_eq!(
            parse_github_url("https://github.com/owner/repo"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parse_https_url_with_git_suffix() {
        assert_eq!(
            parse_github_url("https://github.com/owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parse_ssh_url() {
        assert_eq!(
            parse_github_url("git@github.com:owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parse_non_github_url_returns_none() {
        assert_eq!(parse_github_url("https://gitlab.com/owner/repo"), None);
    }

    #[test]
    fn parse_blame_basic() {
        let porcelain = "\
abcdef1234567890abcdef1234567890abcdef12 1 1 1\nauthor Alice\nauthor-time 1700000000\n\thello world\n";
        let entries = parse_blame_porcelain(porcelain);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].author, "Alice");
        assert_eq!(entries[0].content, "hello world");
        assert_eq!(entries[0].line_number, 1);
    }
}
