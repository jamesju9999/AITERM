//! 「接著談」：已完成的卡片，用原本的 Claude session 重開一個分頁接回對話（`--resume`）。
//!
//! 只是一次追加對話：不改卡片狀態、不重跑審核。想改東西要走既有流程（建卡或回報問題）；
//! 這裡的對話改了檔案，那些改動會留在工程師的工作資料夾，下一張卡開工前會被自動存成 WIP。
//! 這個模組只做純判斷（能不能接、接哪裡、打什麼指令），開分頁在 `commands::tasks`。

use std::path::{Path, PathBuf};

use crate::tasks::store::TaskRow;

#[derive(Debug, PartialEq, Eq)]
pub struct ContinuePlan {
    /// 分頁的工作目錄（卡片當時跑的那個）。
    pub dir: PathBuf,
    pub session_id: String,
}

/// 判斷時需要的外部事實（由呼叫端查好，這裡才好測）。
pub struct Facts<'a> {
    pub claude_command: &'a str,
    /// 這位工程師現在有沒有卡片在跑：同一個工作資料夾不能同時開兩個 Claude。
    pub agent_has_running_card: bool,
    /// `~/.claude/projects` 底下找得到這個 session 的對話檔。
    pub session_log_exists: bool,
    /// Agent 卡的工作資料夾（`agents/<id>/worktree`）。
    pub agent_worktree: Option<&'a Path>,
}

/// 能接就回計畫，不能接就回「為什麼」（直接給使用者看的一句話）。
pub fn plan(task: &TaskRow, facts: &Facts) -> Result<ContinuePlan, String> {
    if task.status != "done" {
        return Err("這張卡還沒結束，請直接到它的分頁裡對話。".into());
    }
    use crate::tasks::workflow::{KIND_FIX, KIND_MERGE, KIND_PLAN, KIND_REVIEW, KIND_VERIFY, KIND_WORK};
    // 審核／驗收／拆解卡的工作資料夾是用完即刪的 detached worktree，卡結案後已不存在；
    // 它們也不佔用 Agent 的工作資料夾。接回對話只需要 session，換到程式庫根開即可。
    let throwaway = [KIND_REVIEW, KIND_VERIFY, KIND_PLAN].contains(&task.card_kind.as_str());
    if !throwaway && ![KIND_WORK, KIND_FIX, KIND_MERGE].contains(&task.card_kind.as_str()) {
        return Err("只有模組卡、修正卡、合併卡與審核／驗收／拆解卡可以接著談。".into());
    }
    if task.cli.as_deref().is_some_and(|c| !c.eq_ignore_ascii_case("claude")) {
        return Err("這張卡不是用 Claude Code 跑的，沒辦法接回對話。".into());
    }
    if !crate::tasks::dispatch::looks_like_claude(facts.claude_command) {
        return Err(format!("接著談需要 Claude Code：目前的派工指令是 `{}`。", facts.claude_command));
    }
    let Some(session_id) = task.session_id.clone().filter(|s| !s.is_empty()) else {
        return Err("這張卡沒有記錄對話編號，沒辦法接回對話。".into());
    };
    if !facts.session_log_exists {
        return Err("找不到這張卡當時的對話檔（可能被清掉了），沒辦法接回對話。".into());
    }
    if !throwaway && facts.agent_has_running_card {
        return Err("這位工程師正在做別的卡，同一個工作資料夾不能同時開兩個 Claude。請等它做完，或先停止那張卡。".into());
    }
    let dir = match (&task.worktree_path, facts.agent_worktree) {
        _ if throwaway => PathBuf::from(&task.project_dir),
        (Some(p), _) if !p.is_empty() => PathBuf::from(p),
        (_, Some(wt)) if task.agent_id.is_some() => wt.to_path_buf(),
        _ => PathBuf::from(&task.project_dir),
    };
    if !dir.is_dir() {
        return Err(format!("卡片當時的工作資料夾已經不在了：{}", dir.display()));
    }
    Ok(ContinuePlan { dir, session_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done_card(dir: &Path) -> TaskRow {
        TaskRow {
            status: "done".into(),
            card_kind: "work".into(),
            session_id: Some("sess-1".into()),
            project_dir: dir.to_string_lossy().into_owned(),
            ..crate::tasks::workflow::tests::test_row("c1")
        }
    }

    fn facts<'a>(wt: Option<&'a Path>) -> Facts<'a> {
        Facts { claude_command: "claude", agent_has_running_card: false, session_log_exists: true, agent_worktree: wt }
    }

    #[test]
    fn a_finished_module_card_with_a_session_can_be_continued_in_its_own_directory() {
        let d = tempfile::tempdir().unwrap();
        let p = plan(&done_card(d.path()), &facts(None)).unwrap();
        assert_eq!(p.session_id, "sess-1");
        assert_eq!(p.dir, d.path());
    }

    #[test]
    fn agent_cards_continue_in_the_agents_worktree() {
        let proj = tempfile::tempdir().unwrap();
        let wt = tempfile::tempdir().unwrap();
        let card = TaskRow { agent_id: Some("a1".into()), ..done_card(proj.path()) };
        assert_eq!(plan(&card, &facts(Some(wt.path()))).unwrap().dir, wt.path());
    }

    #[test]
    fn a_recorded_worktree_wins_over_the_default() {
        let proj = tempfile::tempdir().unwrap();
        let own = tempfile::tempdir().unwrap();
        let card = TaskRow { worktree_path: Some(own.path().to_string_lossy().into_owned()), ..done_card(proj.path()) };
        assert_eq!(plan(&card, &facts(None)).unwrap().dir, own.path());
    }

    #[test]
    fn a_merge_card_continues_in_the_agents_worktree_and_needs_the_agent_free() {
        let proj = tempfile::tempdir().unwrap();
        let wt = tempfile::tempdir().unwrap();
        let card = TaskRow { card_kind: "merge".into(), agent_id: Some("orc".into()), ..done_card(proj.path()) };
        assert_eq!(plan(&card, &facts(Some(wt.path()))).unwrap().dir, wt.path());
        let busy = Facts { agent_has_running_card: true, ..facts(Some(wt.path())) };
        assert!(plan(&card, &busy).unwrap_err().contains("同時開兩個"));
    }

    #[test]
    fn review_verify_and_plan_cards_continue_in_the_repo_root_even_when_the_agent_is_busy() {
        // 它們的工作資料夾是用完即刪的 detached worktree：記錄的路徑早就不在了，不能回頭用。
        let proj = tempfile::tempdir().unwrap();
        let wt = tempfile::tempdir().unwrap();
        for kind in ["review", "verify", "plan"] {
            let card = TaskRow {
                card_kind: kind.into(),
                agent_id: Some("rev".into()),
                worktree_path: Some("/no/such/dir/xyz".into()),
                ..done_card(proj.path())
            };
            let busy = Facts { agent_has_running_card: true, ..facts(Some(wt.path())) };
            let p = plan(&card, &busy).unwrap_or_else(|e| panic!("{kind} 應該能接著談：{e}"));
            assert_eq!(p.dir, proj.path(), "{kind} 要用程式庫根");
            assert_eq!(p.session_id, "sess-1");
        }
    }

    #[test]
    fn a_throwaway_card_still_obeys_the_other_refusals() {
        let proj = tempfile::tempdir().unwrap();
        let card = TaskRow { card_kind: "review".into(), ..done_card(proj.path()) };
        assert!(plan(&TaskRow { session_id: None, ..card.clone() }, &facts(None)).unwrap_err().contains("對話編號"));
        assert!(plan(&TaskRow { status: "running".into(), ..card.clone() }, &facts(None)).unwrap_err().contains("還沒結束"));
        let gone = TaskRow { project_dir: "/no/such/dir/xyz".into(), ..card };
        assert!(plan(&gone, &facts(None)).unwrap_err().contains("已經不在了"));
    }

    #[test]
    fn each_refusal_says_why() {
        let d = tempfile::tempdir().unwrap();
        let ok = done_card(d.path());
        let err = |card: TaskRow, f: Facts| plan(&card, &f).unwrap_err();

        assert!(err(TaskRow { status: "running".into(), ..done_card(d.path()) }, facts(None)).contains("還沒結束"));
        assert!(err(TaskRow { card_kind: "ask".into(), ..done_card(d.path()) }, facts(None)).contains("可以接著談"));
        assert!(err(TaskRow { cli: Some("codex".into()), ..done_card(d.path()) }, facts(None)).contains("不是用 Claude"));
        assert!(err(TaskRow { session_id: None, ..done_card(d.path()) }, facts(None)).contains("對話編號"));
        assert!(err(TaskRow { session_id: Some(String::new()), ..done_card(d.path()) }, facts(None)).contains("對話編號"));
        assert!(err(ok.clone(), Facts { claude_command: "codex", ..facts(None) }).contains("需要 Claude Code"));
        assert!(err(ok.clone(), Facts { session_log_exists: false, ..facts(None) }).contains("對話檔"));
        assert!(err(ok.clone(), Facts { agent_has_running_card: true, ..facts(None) }).contains("同時開兩個"));
        let gone = TaskRow { worktree_path: Some("/no/such/dir/xyz".into()), ..done_card(d.path()) };
        assert!(err(gone, facts(None)).contains("已經不在了"));
    }
}
