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
///
/// `board`：看板專案資料夾。它在程式庫裡面時先排除（`exclude_inside`），不讓
/// tasks.db 與 Agent 工作資料夾進到第一個 commit。
pub async fn init_repo(path: &str, board: Option<&Path>) -> Result<(), String> {
    match check_repo(path).await {
        RepoCheck::Ok { .. } => return Ok(()),
        RepoCheck::Missing => return Err(format!("資料夾不存在：{path}")),
        RepoCheck::NotRepo => GitClient::new(path.to_string(), None).init().await?,
        RepoCheck::NoCommits => {}
    }
    if let Some(board) = board {
        exclude_inside(Path::new(path), board)?;
    }
    GitClient::new(path.to_string(), None).initial_commit("Initial commit (AITerm Agent Canvas)").await
}

/// `inner` 在程式庫 `repo` 裡面時，把它加進 `.git/info/exclude`（本機、不進版控）。
/// 回傳這次有沒有新加。不在裡面、就是程式庫本身、或 `.git` 不是資料夾（例如這是
/// 別的 worktree）都不動。
pub fn exclude_inside(repo: &Path, inner: &Path) -> Result<bool, String> {
    let (Ok(repo), Ok(inner)) = (repo.canonicalize(), inner.canonicalize()) else { return Ok(false) };
    let Ok(rel) = inner.strip_prefix(&repo) else { return Ok(false) };
    if rel.as_os_str().is_empty() {
        return Ok(false);
    }
    let git_dir = repo.join(".git");
    if !git_dir.is_dir() {
        return Ok(false);
    }
    let rel: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let line = format!("/{}/", rel.join("/"));
    let file = git_dir.join("info").join("exclude");
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    if current.lines().any(|l| l.trim() == line) {
        return Ok(false);
    }
    std::fs::create_dir_all(git_dir.join("info")).map_err(|e| e.to_string())?;
    let sep = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
    std::fs::write(&file, format!("{current}{sep}# AITerm 看板專案資料夾\n{line}\n")).map_err(|e| e.to_string())?;
    Ok(true)
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

        init_repo(&p, None).await.unwrap();

        assert!(matches!(check_repo(&p).await, RepoCheck::Ok { .. }));
        let out = Command::new("git").args(["ls-files"]).current_dir(dir.path()).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "main.c", "現有檔案要進第一個 commit");
        // 已經 OK 的資料夾再按一次不會出事。
        init_repo(&p, None).await.unwrap();
    }

    #[tokio::test]
    async fn init_handles_an_empty_folder_and_a_repo_without_commits() {
        let empty = tempfile::tempdir().unwrap();
        init_repo(&empty.path().to_string_lossy(), None).await.unwrap();
        assert!(matches!(check_repo(&empty.path().to_string_lossy()).await, RepoCheck::Ok { .. }));

        let bare = tempfile::tempdir().unwrap();
        git(bare.path(), &["init", "-q"]);
        fs::write(bare.path().join("x.txt"), "x").unwrap();
        init_repo(&bare.path().to_string_lossy(), None).await.unwrap();
        assert!(matches!(check_repo(&bare.path().to_string_lossy()).await, RepoCheck::Ok { .. }));
    }

    #[tokio::test]
    async fn init_refuses_a_missing_folder() {
        assert!(init_repo("/definitely/not/here", None).await.is_err());
    }

    /// Pomodoro 實機：看板專案資料夾建在程式庫裡面，一鍵 init 把它的 tasks.db 也
    /// commit 進去，之後看板每次更新都讓程式庫「有未提交的改動」。
    #[tokio::test]
    async fn init_leaves_a_board_project_inside_the_repo_out_of_git() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.c"), "int main(){}").unwrap();
        let board = dir.path().join("Board");
        fs::create_dir_all(board.join("agents")).unwrap();
        fs::write(board.join("tasks.db"), "db").unwrap();
        let p = dir.path().to_string_lossy().into_owned();

        init_repo(&p, Some(&board)).await.unwrap();

        let out = Command::new("git").args(["ls-files"]).current_dir(dir.path()).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "main.c");
        fs::write(board.join("tasks.db"), "changed").unwrap();
        let st = Command::new("git").args(["status", "--porcelain"]).current_dir(dir.path()).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&st.stdout), "", "看板更新不能讓程式庫變髒");
    }

    #[test]
    fn exclude_is_added_once_and_only_for_folders_inside_the_repo() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        let inside = dir.path().join("Board");
        fs::create_dir_all(&inside).unwrap();
        assert!(exclude_inside(dir.path(), &inside).unwrap());
        assert!(!exclude_inside(dir.path(), &inside).unwrap(), "已經有就不重複加");
        let other = tempfile::tempdir().unwrap();
        assert!(!exclude_inside(dir.path(), other.path()).unwrap(), "不在程式庫裡面就不動");
        assert!(!exclude_inside(dir.path(), dir.path()).unwrap(), "程式庫本身不排除");
        let text = fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
        assert_eq!(text.matches("/Board/").count(), 1, "{text}");
    }
}
