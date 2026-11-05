//! 修復驗證：派工前記下 git HEAD，完成後看「真的改了什麼」並跑使用者指定的測試指令。
//! Agent 說「做完了」不等於修好了。
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

async fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(cwd).stdin(Stdio::null()).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 目前工作區的內容指紋：把所有未被忽略的檔案（含未追蹤）寫進一個**暫時的** index 再 `write-tree`，
/// 回傳 tree sha。不動使用者真正的 index、不產生 commit。
///
/// 用內容指紋而不是 HEAD：前一個問題留下的未 commit 修改、或 Agent 自己 commit 了，
/// 都不會被算到這一題頭上——兩個指紋的差才是「這一題真正改了什麼」。
pub async fn snapshot(cwd: &Path) -> Option<String> {
    git(cwd, &["rev-parse", "--is-inside-work-tree"]).await?;
    let idx = std::env::temp_dir().join(format!("aiterm-intake-index-{}", uuid::Uuid::new_v4()));
    let run = |args: &'static [&'static str]| {
        let idx = idx.clone();
        let cwd = cwd.to_path_buf();
        async move {
            let out = Command::new("git").args(args).current_dir(&cwd).env("GIT_INDEX_FILE", &idx).stdin(Stdio::null()).output().await.ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        }
    };
    let tree = async {
        run(&["add", "-A"]).await?;
        run(&["write-tree"]).await.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }
    .await;
    let _ = std::fs::remove_file(&idx);
    tree
}

/// 自 `base`（先前的 `snapshot`）以來內容有變的檔案。
pub async fn changed_files(cwd: &Path, base: &str) -> Vec<String> {
    let Some(now) = snapshot(cwd).await else { return vec![] };
    git(cwd, &["diff-tree", "-r", "--name-only", base, &now])
        .await
        .map(|d| d.lines().map(str::to_string).filter(|l| !l.trim().is_empty()).collect())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRun {
    pub command: String,
    pub passed: bool,
    pub timed_out: bool,
    pub tail: String,
}

fn tail(s: &str, max: usize) -> String {
    let t = s.trim_end();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() <= max { t.to_string() } else { format!("…{}", chars[chars.len() - max..].iter().collect::<String>()) }
}

/// 用各平台的預設 shell 跑一行指令，合併 stdout/stderr 取尾端。
pub async fn run_command(cwd: &Path, command: &str, timeout: Duration) -> TestRun {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let fail = |msg: String| TestRun { command: command.into(), passed: false, timed_out: false, tail: msg };
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(format!("無法執行：{e}")),
    };
    let (mut so, mut se) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let read = async {
        let (mut a, mut b) = (String::new(), String::new());
        let _ = tokio::join!(so.read_to_string(&mut a), se.read_to_string(&mut b));
        let status = child.wait().await;
        (status, format!("{a}{b}"))
    };
    match tokio::time::timeout(timeout, read).await {
        Ok((Ok(st), out)) => TestRun { command: command.into(), passed: st.success(), timed_out: false, tail: tail(&out, 1500) },
        Ok((Err(e), _)) => fail(format!("等待結束失敗：{e}")),
        Err(_) => TestRun { command: command.into(), passed: false, timed_out: true, tail: format!("超過 {} 秒仍未結束，已中止", timeout.as_secs()) },
    }
}

#[derive(Debug, Clone)]
pub struct Report {
    pub is_repo: bool,
    pub files: Vec<String>,
    pub tests: Option<TestRun>,
    /// 使用者有沒有設定測試指令（沒設定與「沒變更所以略過」要分開說）。
    pub test_command_set: bool,
}

impl Report {
    /// 通過＝有實際變更，而且（有設定測試的話）測試通過。
    pub fn passed(&self) -> bool {
        self.is_repo && !self.files.is_empty() && self.tests.as_ref().map_or(true, |t| t.passed)
    }

    pub fn text(&self) -> String {
        if !self.is_repo {
            return "【驗證】這個終端機不在 git 專案內，無法確認程式是否真的有被修改。".into();
        }
        let mut s = String::from("【驗證】\n");
        if self.files.is_empty() {
            s.push_str("沒有偵測到任何檔案變更，這個問題可能尚未被修復。\n");
        } else {
            s.push_str(&format!("已變更 {} 個檔案：\n", self.files.len()));
            for f in self.files.iter().take(20) {
                s.push_str(&format!("  - {f}\n"));
            }
            if self.files.len() > 20 {
                s.push_str(&format!("  …另有 {} 個\n", self.files.len() - 20));
            }
        }
        match &self.tests {
            None if !self.test_command_set => s.push_str("未設定測試指令，未執行自動測試。\n"),
            None => {} // 有設定但沒有變更可測，上面已說明沒有變更
            Some(t) if t.passed => s.push_str(&format!("測試通過：{}\n", t.command)),
            Some(t) if t.timed_out => s.push_str(&format!("測試逾時：{}\n{}\n", t.command, t.tail)),
            Some(t) => s.push_str(&format!("測試未通過：{}\n{}\n", t.command, t.tail)),
        }
        s.trim_end().to_string()
    }
}

pub async fn verify(cwd: &Path, base: Option<&str>, test_command: &str, timeout: Duration) -> Report {
    let Some(base) = base else { return Report { is_repo: false, files: vec![], tests: None, test_command_set: !test_command.trim().is_empty() } };
    let files = changed_files(cwd, base).await;
    let cmd = test_command.trim();
    // 沒有任何變更時不必浪費時間跑測試：結論已經是「沒修」。
    let tests = if cmd.is_empty() || files.is_empty() { None } else { Some(run_command(cwd, cmd, timeout).await) };
    Report { is_repo: true, files, tests, test_command_set: !cmd.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Sync;

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let run = |a: &[&str]| assert!(Sync::new("git").args(a).current_dir(d.path()).output().unwrap().status.success(), "{a:?}");
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(d.path().join("a.txt"), "1").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        d
    }
    const T: Duration = Duration::from_secs(20);

    #[tokio::test]
    async fn no_changes_is_not_a_pass_and_tests_are_skipped() {
        let d = repo();
        let base = snapshot(d.path()).await.unwrap();
        let r = verify(d.path(), Some(&base), "exit 1", T).await;
        assert!(r.files.is_empty() && r.tests.is_none() && !r.passed());
        assert!(r.text().contains("沒有偵測到任何檔案變更"));
        assert!(!r.text().contains("未設定測試指令"), "有設定測試指令時不可說「未設定」：{}", r.text());
    }

    #[tokio::test]
    async fn sees_modified_untracked_and_committed_changes() {
        let d = repo();
        let base = snapshot(d.path()).await.unwrap();
        std::fs::write(d.path().join("a.txt"), "2").unwrap();
        std::fs::write(d.path().join("new.txt"), "n").unwrap();
        assert_eq!(changed_files(d.path(), &base).await, vec!["a.txt".to_string(), "new.txt".to_string()]);
        // commit 後仍然看得到（以 base 為準，不是看工作區髒不髒）
        let run = |a: &[&str]| { Sync::new("git").args(a).current_dir(d.path()).output().unwrap(); };
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "fix"]);
        assert_eq!(changed_files(d.path(), &base).await.len(), 2);
    }

    #[tokio::test]
    async fn an_earlier_items_uncommitted_changes_are_not_blamed_on_the_next_item() {
        let d = repo();
        std::fs::write(d.path().join("from_item_1.txt"), "x").unwrap(); // 上一題留下的，沒 commit
        let base = snapshot(d.path()).await.unwrap();
        // 這一題什麼都沒改
        assert!(changed_files(d.path(), &base).await.is_empty());
        let r = verify(d.path(), Some(&base), "echo fine", T).await;
        assert!(!r.passed(), "{}", r.text());
        // 這一題改了一個檔：只算這一個
        std::fs::write(d.path().join("from_item_2.txt"), "y").unwrap();
        assert_eq!(changed_files(d.path(), &base).await, vec!["from_item_2.txt".to_string()]);
    }

    #[tokio::test]
    async fn snapshot_leaves_the_real_index_and_history_alone_and_respects_gitignore() {
        let d = repo();
        std::fs::write(d.path().join(".gitignore"), "ignored.log\n").unwrap();
        let base = snapshot(d.path()).await.unwrap();
        std::fs::write(d.path().join("ignored.log"), "noise").unwrap();
        assert!(changed_files(d.path(), &base).await.is_empty(), "被忽略的檔案不算變更");
        let status = git(d.path(), &["status", "--porcelain"]).await.unwrap();
        assert!(status.contains(".gitignore") && status.contains("??"), "真正的 index 沒被動到，.gitignore 仍是未追蹤：{status}");
        assert_eq!(git(d.path(), &["rev-list", "--count", "HEAD"]).await.unwrap().trim(), "1", "沒有產生 commit");
    }

    #[tokio::test]
    async fn passing_and_failing_tests_decide_the_verdict() {
        let d = repo();
        let base = snapshot(d.path()).await.unwrap();
        std::fs::write(d.path().join("a.txt"), "2").unwrap();
        let ok = verify(d.path(), Some(&base), "echo fine", T).await;
        assert!(ok.passed(), "{}", ok.text());
        assert!(ok.text().contains("測試通過"));
        let bad = verify(d.path(), Some(&base), "echo boom && exit 3", T).await;
        assert!(!bad.passed());
        assert!(bad.text().contains("測試未通過") && bad.text().contains("boom"));
        let none = verify(d.path(), Some(&base), "", T).await;
        assert!(none.passed(), "沒設測試指令時，有變更就算通過但會註明未測試");
        assert!(none.text().contains("未設定測試指令"));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn a_hanging_test_command_times_out_instead_of_hanging_the_queue() {
        let d = repo();
        let r = run_command(d.path(), "sleep 30", Duration::from_millis(300)).await;
        assert!(r.timed_out && !r.passed);
    }

    #[tokio::test]
    async fn outside_a_git_repo_is_reported_not_passed() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(snapshot(d.path()).await, None);
        let r = verify(d.path(), None, "echo x", T).await;
        assert!(!r.passed() && r.text().contains("不在 git 專案內"));
    }
}
