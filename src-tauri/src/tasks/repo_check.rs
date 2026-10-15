//! Agent 的程式庫檢查與一鍵初始化。Agent 的隔離、依序累積與合併都建立在
//! git 分支上，而且要有至少一個 commit 才能 `git worktree add`——所以「是
//! git repo」還不夠，還要「有 HEAD」。對話框在使用者選資料夾時就先檢查，
//! 不讓問題拖到派工時才爆。

use serde::Serialize;
use std::path::Path;

use crate::config::types::VcsType;
use crate::vcs::git::GitClient;
use crate::vcs::VcsManager;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RepoCheck {
    /// 可以用。`branch` 是目前分支（detached HEAD 時為 None）。
    Ok { branch: Option<String> },
    /// 資料夾不存在。
    Missing,
    /// 不是 git repo（或是 SVN）。可以一鍵初始化。
    NotRepo,
    /// 是 git repo 但還沒有任何 commit。可以一鍵建立第一個 commit。
    NoCommits,
}

pub async fn check_repo(path: &str) -> RepoCheck {
    if path.trim().is_empty() || !Path::new(path).is_dir() {
        return RepoCheck::Missing;
    }
    match VcsManager::detect_repo(path).await {
        Ok(info) if info.vcs_type == VcsType::Git => {}
        _ => return RepoCheck::NotRepo,
    }
    let client = GitClient::new(path.to_string(), None);
    if client.head_sha().await.is_err() {
        return RepoCheck::NoCommits;
    }
    RepoCheck::Ok { branch: client.current_branch().await.ok().filter(|b| !b.is_empty() && b != "HEAD") }
}

/// 不是 repo 就 `git init`；沒有 commit 就把資料夾現有檔案建成第一個 commit
/// （沒有任何檔案時建空 commit）。使用者沒設定 git 身分時，這一個 commit
/// 用 AITerm 的身分，不去改使用者的全域設定。
pub async fn init_repo(path: &str) -> Result<(), String> {
    match check_repo(path).await {
        RepoCheck::Ok { .. } => return Ok(()),
        RepoCheck::Missing => return Err(format!("資料夾不存在：{path}")),
        RepoCheck::NotRepo => GitClient::new(path.to_string(), None).init().await?,
        RepoCheck::NoCommits => {}
    }
    GitClient::new(path.to_string(), None).initial_commit("Initial commit (AITerm Agent Canvas)").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        assert!(Command::new("git").args(args).current_dir(dir).status().unwrap().success());
    }

    #[tokio::test]
    async fn reports_missing_not_repo_no_commits_and_ok() {
        assert_eq!(check_repo("/definitely/not/here").await, RepoCheck::Missing);
        assert_eq!(check_repo("").await, RepoCheck::Missing);

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().to_string_lossy().into_owned();
        assert_eq!(check_repo(&p).await, RepoCheck::NotRepo);

        git(dir.path(), &["init", "-q"]);
        assert_eq!(check_repo(&p).await, RepoCheck::NoCommits);

        git(dir.path(), &["config", "user.email", "t@t"]);
        git(dir.path(), &["config", "user.name", "T"]);
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "c"]);
        match check_repo(&p).await {
            RepoCheck::Ok { branch } => assert!(branch.is_some()),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn init_turns_a_plain_folder_into_a_repo_with_its_files_committed() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.c"), "int main(){}").unwrap();
        let p = dir.path().to_string_lossy().into_owned();

        init_repo(&p).await.unwrap();

        assert!(matches!(check_repo(&p).await, RepoCheck::Ok { .. }));
        let out = Command::new("git").args(["ls-files"]).current_dir(dir.path()).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "main.c", "現有檔案要進第一個 commit");
        // 已經 OK 的資料夾再按一次不會出事。
        init_repo(&p).await.unwrap();
    }

    #[tokio::test]
    async fn init_handles_an_empty_folder_and_a_repo_without_commits() {
        let empty = tempfile::tempdir().unwrap();
        init_repo(&empty.path().to_string_lossy()).await.unwrap();
        assert!(matches!(check_repo(&empty.path().to_string_lossy()).await, RepoCheck::Ok { .. }));

        let bare = tempfile::tempdir().unwrap();
        git(bare.path(), &["init", "-q"]);
        fs::write(bare.path().join("x.txt"), "x").unwrap();
        init_repo(&bare.path().to_string_lossy()).await.unwrap();
        assert!(matches!(check_repo(&bare.path().to_string_lossy()).await, RepoCheck::Ok { .. }));
    }

    #[tokio::test]
    async fn init_refuses_a_missing_folder() {
        assert!(init_repo("/definitely/not/here").await.is_err());
    }
}
