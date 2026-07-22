//! Shared VCS types used across git.rs, svn.rs, and commands.

use serde::{Deserialize, Serialize};

fn default_log_max_count() -> u32 { 20 }

/// Parsed intent from a natural-language VCS query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VcsIntent {
    LogQuery {
        path: Option<String>,
        author: Option<String>,
        since: Option<String>,
        #[serde(default = "default_log_max_count")]
        max_count: u32,
    },
    DiffView {
        revision: String,
    },
    Blame {
        path: String,
    },
    BranchList,
    PrList {
        state: Option<String>,
    },
    IssueList {
        state: Option<String>,
    },
    ActionsList,
    RevertCommit {
        revision: String,
    },
    CherryPick {
        revision: String,
    },
    CreatePr {
        title: String,
        head: String,
        base: String,
        body: Option<String>,
    },
    MergePr {
        pr_number: u64,
    },
    CreateIssue {
        title: String,
        body: Option<String>,
    },
    TriggerWorkflow {
        workflow_id: String,
        r#ref: String,
    },
    CreateBranch {
        name: String,
        from: Option<String>,
    },
    DeleteBranch {
        name: String,
    },
    CheckoutBranch {
        name: String,
    },
    SvnCommit {
        message: String,
        paths: Vec<String>,
    },
    SvnRevert {
        paths: Vec<String>,
    },
    SvnUpdate {
        path: Option<String>,
    },
}

/// Info about a detected repo, returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VcsRepoInfo {
    pub vcs_type: crate::config::types::VcsType,
    pub root: String,
    pub remote_url: Option<String>,
    pub connection_id: Option<String>,
}

/// A single commit entry for log results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitEntry {
    pub revision: String,
    pub author: String,
    pub date: String,
    pub message: String,
    pub files_changed: Vec<String>,
}

/// A branch entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchEntry {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
}

/// A PR from GitHub API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrEntry {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: String,
    pub url: String,
    pub updated_at: String,
}

/// A GitHub Actions workflow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub created_at: String,
    pub html_url: String,
}

/// A GitHub issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueEntry {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: String,
    pub url: String,
    pub created_at: String,
}

/// A blame line entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameEntry {
    pub line_number: u32,
    pub revision: String,
    pub author: String,
    pub date: String,
    pub content: String,
}

/// A single entry in the vcs_agent_step history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum VcsAgentHistoryEntry {
    User { text: String },
    Step {
        step_num: u32,
        operation: String,
        result_json: String,
        summary: String,
    },
}

/// The AI decision returned by vcs_agent_step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VcsAgentDecision {
    pub done: bool,
    pub intent: Option<VcsIntent>,
    pub summary: String,
    pub final_answer: Option<String>,
}

/// Structured result from a VCS operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VcsResult {
    Log {
        commits: Vec<CommitEntry>,
        truncated: bool,
    },
    Diff {
        content: String,
        revision: String,
    },
    Blame {
        lines: Vec<BlameEntry>,
    },
    Branches {
        branches: Vec<BranchEntry>,
    },
    PrList {
        prs: Vec<PrEntry>,
    },
    IssueList {
        issues: Vec<IssueEntry>,
    },
    ActionsList {
        runs: Vec<WorkflowRun>,
    },
    WriteConfirm {
        operation: String,
        preview: String,
        intent: VcsIntent,
    },
    WriteSuccess {
        operation: String,
        detail: String,
    },
    Error {
        message: String,
    },
    NoToken {
        required_level: u8,
    },
    SvnNotInstalled,
}

/// Branch + working-tree diff stats for a terminal block header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitBlockInfo {
    pub branch: String,
    pub insertions: u32,
    pub deletions: u32,
}

#[cfg(test)]
mod agent_tests {
    use super::*;

    #[test]
    fn test_history_entry_user_roundtrip() {
        let entry = VcsAgentHistoryEntry::User { text: "show me commits".into() };
        let json = serde_json::to_string(&entry).unwrap();
        let decoded: VcsAgentHistoryEntry = serde_json::from_str(&json).unwrap();
        match decoded {
            VcsAgentHistoryEntry::User { text } => assert_eq!(text, "show me commits"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_history_entry_step_roundtrip() {
        let entry = VcsAgentHistoryEntry::Step {
            step_num: 1,
            operation: "log_query".into(),
            result_json: r#"{"type":"log","commits":[],"truncated":false}"#.into(),
            summary: "Found 0 commits".into(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let decoded: VcsAgentHistoryEntry = serde_json::from_str(&json).unwrap();
        match decoded {
            VcsAgentHistoryEntry::Step { step_num, .. } => assert_eq!(step_num, 1),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_decision_done_roundtrip() {
        let d = VcsAgentDecision {
            done: true,
            intent: None,
            summary: "Goal achieved".into(),
            final_answer: Some("Found the file".into()),
        };
        let json = serde_json::to_string(&d).unwrap();
        let decoded: VcsAgentDecision = serde_json::from_str(&json).unwrap();
        assert!(decoded.done);
        assert_eq!(decoded.final_answer.as_deref(), Some("Found the file"));
    }

    #[test]
    fn test_decision_continue_roundtrip() {
        let d = VcsAgentDecision {
            done: false,
            intent: Some(VcsIntent::BranchList),
            summary: "Checking branches".into(),
            final_answer: None,
        };
        let json = serde_json::to_string(&d).unwrap();
        let decoded: VcsAgentDecision = serde_json::from_str(&json).unwrap();
        assert!(!decoded.done);
        assert!(matches!(decoded.intent, Some(VcsIntent::BranchList)));
    }
}
