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
    /// 還不能用，而且這是 AITerm 的看板專案資料夾（有 `.aitprj`），不是程式碼。
    /// 不提供初始化——在這裡 init 會把 tasks.db 與派工紀錄收進版控。
    BoardFolder,
}

/// 資料夾最上層有 `.aitprj` 檔＝看板專案資料夾。
fn is_board_folder(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|rd| {
        rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "aitprj") && e.path().is_file())
    })
}

pub async fn check_repo(path: &str) -> RepoCheck {
    super::path_refresh::refresh().await;
    if path.trim().is_empty() || !Path::new(path).is_dir() {
        return RepoCheck::Missing;
    }
    match VcsManager::detect_repo(path).await {
        Ok(info) if info.vcs_type == VcsType::Git => {}
        _ if is_board_folder(Path::new(path)) => return RepoCheck::BoardFolder,
        _ => return RepoCheck::NotRepo,
    }
    let client = GitClient::new(path.to_string(), None);
    if client.head_sha().await.is_err() {
        if is_board_folder(Path::new(path)) {
            return RepoCheck::BoardFolder;
        }
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
        RepoCheck::BoardFolder => return Err(format!("這是看板專案資料夾，不是程式碼：{path}")),
        RepoCheck::NotRepo => GitClient::new(path.to_string(), None).init().await?,
        RepoCheck::NoCommits => {}
    }
    if let Some(board) = board {
        exclude_inside(Path::new(path), board)?;
    }
    GitClient::new(path.to_string(), None).initial_commit("Initial commit (AITerm Agent Canvas)").await
}

/// `inner` 在 `repo` 所屬的 git 工作樹裡面時，把它加進 `.git/info/exclude`（本機、不進版控）。
/// 回傳這次有沒有新加。不在裡面、就是工作樹最上層本身、或 `.git` 不是資料夾（例如這是
/// 別的 worktree）都不動。
///
/// `repo` 不一定是 git 最上層：Agent 的程式庫常是最上層底下的子資料夾，看板專案資料夾
/// 卻在子資料夾外面——所以先往上找到有 `.git` 的那一層，規則也相對於那一層。
pub fn exclude_inside(repo: &Path, inner: &Path) -> Result<bool, String> {
    let (Ok(repo), Ok(inner)) = (repo.canonicalize(), inner.canonicalize()) else { return Ok(false) };
    let Some(top) = repo.ancestors().find(|a| a.join(".git").exists()) else { return Ok(false) };
    let Ok(rel) = inner.strip_prefix(top) else { return Ok(false) };
    if rel.as_os_str().is_empty() {
        return Ok(false);
    }
    let git_dir = top.join(".git");
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

/// 專案記住的程式庫（套用團隊範本時選過一次就不再問），存在 `project_settings`。
const PROJECT_REPO_KEY: &str = "repo_dir";

pub async fn saved_project_repo(pool: &sqlx::SqlitePool) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM project_settings WHERE key = ?").bind(PROJECT_REPO_KEY).fetch_optional(pool).await
}

pub async fn save_project_repo(pool: &sqlx::SqlitePool, repo: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO project_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(PROJECT_REPO_KEY)
        .bind(repo)
        .execute(pool)
        .await?;
    Ok(())
}

/// 專案的程式庫，與它從哪裡來（`agents`＝桌布上 Agent 的、`saved`＝這個專案記住的）。
/// 兩者都是使用者自己指定過的；都沒有就是 `None`，由使用者選——系統不自己猜、不自己建立。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProjectRepo {
    pub path: String,
    pub source: &'static str,
}

pub fn project_repo(agent_dirs: &[String], saved: Option<String>) -> Option<ProjectRepo> {
    if let Some(d) = agent_dirs.iter().find(|d| !d.trim().is_empty()) {
        return Some(ProjectRepo { path: d.clone(), source: "agents" });
    }
    saved.filter(|s| !s.trim().is_empty()).map(|path| ProjectRepo { path, source: "saved" })
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

    /// 實機：團隊範本選到看板專案資料夾（ARESGUI.aitprj＋tasks.db），只看得到
    /// 「不是 git repo」，照提示初始化就會把 tasks.db commit 進去。
    #[tokio::test]
    async fn a_board_project_folder_is_reported_and_never_initialised() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ARESGUI.aitprj"), "{}").unwrap();
        fs::write(dir.path().join("tasks.db"), "db").unwrap();
        let p = dir.path().to_string_lossy().into_owned();
        assert_eq!(check_repo(&p).await, RepoCheck::BoardFolder);
        assert!(init_repo(&p, None).await.is_err());
        assert!(!dir.path().join(".git").exists(), "不能 git init");

        git(dir.path(), &["init", "-q"]);
        assert_eq!(check_repo(&p).await, RepoCheck::BoardFolder, "init 過但沒 commit 也一樣");
        assert!(init_repo(&p, None).await.is_err());
        let head = Command::new("git").args(["rev-parse", "--verify", "HEAD"]).current_dir(dir.path()).output().unwrap();
        assert!(!head.status.success(), "不能建第一個 commit");

        // 同名但是資料夾的不算；已經是可用的程式庫也照常回 Ok（不擋已經在用的設定）。
        let code = tempfile::tempdir().unwrap();
        fs::create_dir_all(code.path().join("x.aitprj")).unwrap();
        assert_eq!(check_repo(&code.path().to_string_lossy()).await, RepoCheck::NotRepo);
    }

    /// ARES GUI To Web 實機：Agent 的程式庫（project_dir）是 git 最上層的子資料夾，
    /// 看板專案資料夾卻在最上層底下、project_dir 外面。只看 `project_dir/.git` 會找不到，
    /// 排除就靜靜沒做，程式庫永遠「有未提交的改動」、整合結果合不回去。
    #[test]
    fn exclude_works_when_the_agents_repo_is_a_subfolder_of_the_git_toplevel() {
        let top = tempfile::tempdir().unwrap();
        git(top.path(), &["init", "-q"]);
        let sub = top.path().join("Source");
        let board = top.path().join("Board");
        fs::create_dir_all(&sub).unwrap();
        fs::create_dir_all(&board).unwrap();

        assert!(exclude_inside(&sub, &board).unwrap(), "最上層底下、子資料夾外面的看板資料夾也要排除");
        let text = fs::read_to_string(top.path().join(".git/info/exclude")).unwrap();
        assert!(text.lines().any(|l| l == "/Board/"), "規則要相對於 git 最上層：{text}");
        assert!(!exclude_inside(&sub, &board).unwrap(), "已經有就不重複加");

        fs::write(board.join("tasks.db"), "db").unwrap();
        let st = Command::new("git").args(["status", "--porcelain"]).current_dir(&sub).output().unwrap();
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

    #[test]
    fn project_repo_only_uses_folders_the_user_chose() {
        let r = project_repo(&["/a".into()], Some("/s".into())).unwrap();
        assert_eq!((r.path.as_str(), r.source), ("/a", "agents"), "桌布上 Agent 的程式庫優先");
        let r = project_repo(&[], Some("/s".into())).unwrap();
        assert_eq!((r.path.as_str(), r.source), ("/s", "saved"), "其次是這個專案記住的");
        assert_eq!(project_repo(&[], None), None, "都沒有就由使用者選，不自己猜");
        assert_eq!(project_repo(&["  ".into()], Some(" ".into())), None);
    }

    #[tokio::test]
    async fn the_project_repo_is_remembered() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        assert_eq!(saved_project_repo(&pool).await.unwrap(), None);
        save_project_repo(&pool, "/r").await.unwrap();
        save_project_repo(&pool, "/r2").await.unwrap();
        assert_eq!(saved_project_repo(&pool).await.unwrap().as_deref(), Some("/r2"));
    }
}
