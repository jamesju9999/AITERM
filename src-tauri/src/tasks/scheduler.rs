//! The dispatch loop. `pick_next` is the pure selection rule (heavily
//! tested); `run` is the long-lived `tokio` task that ties it to `tasks.db`,
//! a `Notify` wake signal, `dispatch`, and `monitor`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{oneshot, Notify};

use crate::config::types::VcsType;
use crate::config::ConfigStore;
use crate::pty::PtyManager;
use crate::tasks::store::{self};
use crate::tasks::store::TaskRow;
use crate::projects::{ProjectHandle, ProjectRegistry};
use crate::tasks::{dispatch, monitor};
use crate::vcs::VcsManager;

/// Choose the next `queued` card to promote to `running`, or `None` if
/// nothing should start right now. `queued` MUST be sorted by `sort_order`
/// ascending (oldest first). Rules (see the design doc §5):
/// - a `parallel_ok = false` card currently running blocks all starts;
/// - respect the global `max_concurrent` cap;
/// - consider only the head of the queue (strict priority — never skip a
///   card to start a later one);
/// - a head card that is `parallel_ok = false` starts only when nothing is
///   running.
pub fn pick_next<'a>(
    running: &[TaskRow],
    queued: &'a [TaskRow],
    max_concurrent: u32,
) -> Option<&'a TaskRow> {
    if running.iter().any(|r| !r.parallel_ok) {
        return None;
    }
    if running.len() as u32 >= max_concurrent.max(1) {
        return None;
    }
    let head = queued.first()?;
    if !head.parallel_ok && !running.is_empty() {
        return None;
    }
    Some(head)
}

/// 一個專案這一輪可以考慮派工的隊首卡片（Agent 協作，見
/// `docs/superpowers/specs/2026-10-15-agent-canvas-design.md` §1）。
///
/// - 未指派卡片：維持原本的嚴格優先序，只交出 `sort_order` 最小的一張
///   非互動卡片（互動卡片另走 `drain_once` 的第一輪）。
/// - 每個 Agent：交出自己待執行卡片中最前面那張——**包括互動卡片**，
///   Agent 卡片一律依序執行。Agent 已有卡片在跑（`running` 裡有同一個
///   `agent_id`）或暫停中（`paused`）就不交出。
///
/// - `blocked`：還在等別位工程師前置的卡（`workflow::blocked_cards`）。跳過它、
///   改交出同一個 Agent 佇列裡的下一張，不讓一張等待中的卡擋住整條佇列。
///
/// 每個 Agent 各有一條佇列，所以一個忙碌的 Agent 不會擋住另一個 Agent
/// 或未指派的卡片——這正是「嚴格只看隊首」在這裡必須放寬的地方。
/// `queued`/`running` 只能是**同一個專案**的卡片（Agent 屬於專案）。
/// 回傳依 `sort_order` 排序。
pub fn candidate_heads(
    queued: &[TaskRow],
    running: &[TaskRow],
    paused: &std::collections::HashSet<String>,
    blocked: &std::collections::HashSet<String>,
) -> Vec<TaskRow> {
    let mut sorted: Vec<&TaskRow> = queued.iter().collect();
    sorted.sort_by(|a, b| a.sort_order.partial_cmp(&b.sort_order).unwrap_or(std::cmp::Ordering::Equal));

    let busy: std::collections::HashSet<&str> =
        running.iter().filter_map(|t| t.agent_id.as_deref()).collect();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut out: Vec<TaskRow> = Vec::new();
    let mut unassigned_taken = false;
    for t in sorted {
        match t.agent_id.as_deref() {
            None => {
                if !unassigned_taken && !t.interactive {
                    unassigned_taken = true;
                    out.push(t.clone());
                }
            }
            Some(aid) => {
                if blocked.contains(&t.id) {
                    continue;
                }
                if seen.insert(aid) && !busy.contains(aid) && !paused.contains(aid) {
                    out.push(t.clone());
                }
            }
        }
    }
    out
}

/// 各專案的隊首卡片依 `created_at` 由舊到新排序 —— 跨專案「先到先得」。
///
/// 刻意**不**把所有專案的 queued 卡片混在一起依 `sort_order` 排序：
/// `sort_order` 是各專案獨立的浮點數，混排毫無意義，而且會破壞使用者
/// 在單一專案內用拖曳決定的順序。每個專案只交出自己的隊首，專案內部
/// 的嚴格優先序因此完整保留。
pub fn order_heads(mut heads: Vec<(String, TaskRow)>) -> Vec<(String, TaskRow)> {
    heads.sort_by(|a, b| a.1.created_at.cmp(&b.1.created_at).then(a.1.id.cmp(&b.1.id)));
    heads
}

/// 決定這張卡片實際要用哪個工作目錄，以及要不要建立隔離用的 worktree。
///
/// `project_dir` 是 git repo 就在 `<project_storage_path>/tasks/<task_id>/worktree`
/// 建一個新 worktree + 分支（`aiterm-task/<task_id>`），從目前 HEAD 分支
/// 出去，不 fetch、不需要遠端。回傳的第二個值非 `None` 時，呼叫端要把
/// 這組 (path, branch) 寫進 `store::set_worktree`。
///
/// 不是 git repo（`VcsManager::detect_repo` 回 Err，或偵測到是 SVN——
/// `detect_repo` 對 SVN repo 回傳 `Ok`，不是 Err，所以要額外檢查
/// `vcs_type`），或 `git worktree add` 本身失敗（磁碟空間不足、舊版 git
/// 沒有 worktree 支援等），一律退回直接用 `project_dir`，不擋派工。
/// 這張卡片這次派工到底要不要隔離。
///
/// 卡片有明確覆寫就用它，否則沿用全域設定。**刻意在派工當下才解析**（而不
/// 是建立卡片時就固定下來），這樣調整全域設定會影響所有還在等待、又沒有
/// 個別覆寫的卡片——那才是「預設值」該有的語意。
///
/// 拆成獨立函式是為了能單獨測優先順序：`dispatch()` 需要 `AppHandle`，
/// 在測試裡建不出來。
fn resolve_isolation(card: Option<bool>, global: bool) -> bool {
    card.unwrap_or(global)
}

async fn prepare_worktree(
    project_storage_path: &std::path::Path,
    task_id: &str,
    task_project_dir: &str,
    isolate: bool,
) -> (std::path::PathBuf, Option<(String, String)>) {
    let fallback = std::path::PathBuf::from(task_project_dir);

    // 關掉隔離時直接走跟「這個目錄不是 git repo」完全一樣的回傳值——那條
    // 路徑本來就存在而且驗證過，不需要新的邏輯。回傳 None 也讓卡片不會有
    // worktree_branch，前端的「合併回原分支」按鈕自然不會出現。
    if !isolate {
        return (fallback, None);
    }

    match VcsManager::detect_repo(task_project_dir).await {
        Ok(info) if info.vcs_type == VcsType::Git => {}
        _ => return (fallback, None),
    }

    let worktree_path = crate::tasks::task_dir(project_storage_path, task_id).join("worktree");
    let branch_name = format!("aiterm-task/{task_id}");
    let client = crate::vcs::git::GitClient::new(task_project_dir.to_string(), None);
    match client.create_worktree(&worktree_path.to_string_lossy(), &branch_name).await {
        Ok(_) => {
            let path_str = worktree_path.to_string_lossy().into_owned();
            (worktree_path, Some((path_str, branch_name)))
        }
        Err(e) => {
            eprintln!("worktree create failed for task {task_id}: {e}");
            (fallback, None)
        }
    }
}

/// 角色前綴＋卡片提示詞。前綴全空白就原樣回傳，不留多餘的空行。
pub fn with_role_prefix(prefix: &str, prompt: &str) -> String {
    let p = prefix.trim();
    if p.is_empty() {
        prompt.to_string()
    } else {
        format!("{p}\n\n{prompt}")
    }
}

/// 非互動的 Agent 卡：旁邊通常沒有人，停下來問問題就會讓整條流程停住（Pomodoro 實機）。
pub const AUTONOMY_NOTE: &str = "（這是自動派工，旁邊沒有人會即時回答。遇到不確定的地方，採用最合理的做法，\
把假設寫進文件或交件說明的「待確認事項」，然後直接把工作做完，不要停下來詢問。）";

/// 派工指令的 `--model <name>`。名稱只接受英數與 `._-[]:`——指令是打進使用者的
/// shell 的，其他字元在各 shell 的意義不同，不安全就不加（沿用預設模型）。
pub fn model_flag(model: Option<&str>) -> Option<String> {
    let m = model?.trim();
    if m.is_empty() || !m.chars().all(|c| c.is_ascii_alphanumeric() || "._-[]:".contains(c)) {
        return None;
    }
    Some(format!("--model {m}"))
}

/// 把有值的參數片段用空白接起來；全部都沒有就是 `None`。
pub fn join_args<const N: usize>(parts: [Option<String>; N]) -> Option<String> {
    let v: Vec<String> = parts.into_iter().flatten().collect();
    (!v.is_empty()).then(|| v.join(" "))
}

/// 掛上 `ask_human` 時的版本：需要人決定的事用工具問。
pub const AUTONOMY_NOTE_ASK: &str = "（這是自動派工，旁邊沒有人會即時回答。需要人做決定時（需求不清、取捨會影響成果），\
用 ask_human 工具提問，送出後結束這一輪回覆等答案，不要自己先決定；小細節自己採合理做法，記在文件或交件說明的「待確認事項」。）";

/// Codex 卡片的提問版自主說明：ask_human 會等到有人回答才回傳（`codex exec` 結束這一輪就退出了）。
pub const AUTONOMY_NOTE_ASK_WAIT: &str = "（這是自動派工，旁邊沒有人會即時回答。需要人做決定時（需求不清、取捨會影響成果），\
用 ask_human 工具提問；工具會等到有人回答才回傳答案，照答案繼續做完。不要自己先決定；小細節自己採合理做法，記在文件或交件說明的「待確認事項」。）";

/// Codex 卡片：把提問版說明換成「工具會等答案」的版本。
pub fn codex_autonomy(prompt: &str) -> String {
    prompt.replace(AUTONOMY_NOTE_ASK, AUTONOMY_NOTE_ASK_WAIT)
}

/// Codex 卡片結束時的判斷：額度用完就回佇列，否則照結果走（錯誤訊息換成 Codex 的）。
pub(crate) enum CodexVerdict {
    Requeue,
    Outcome(monitor::TaskOutcome),
}

pub(crate) async fn codex_verdict(pool: &sqlx::SqlitePool, task_id: &str, outcome: monitor::TaskOutcome) -> CodexVerdict {
    if !matches!(outcome, monitor::TaskOutcome::Failed(_)) {
        return CodexVerdict::Outcome(outcome);
    }
    let path = store::get_task(pool, task_id).await.ok().flatten().and_then(|t| t.session_path);
    let err = path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| crate::tasks::codex::last_error(&t));
    match err {
        Some(crate::tasks::codex::CodexError::UsageLimit(_)) => {
            match requeue_for_quota(pool, task_id, chrono::Utc::now().timestamp()).await {
                Ok(()) => CodexVerdict::Requeue,
                Err(e) => {
                    eprintln!("requeue for quota {task_id}: {e}");
                    CodexVerdict::Outcome(outcome)
                }
            }
        }
        Some(crate::tasks::codex::CodexError::Other(m)) => CodexVerdict::Outcome(monitor::TaskOutcome::Failed(m)),
        None => CodexVerdict::Outcome(outcome),
    }
}

/// 額度用完的 Codex 卡回到該 Agent 佇列最前面、標成接著做、記下等待時間；
/// 排程器在 `RETRY_AFTER` 之內不會再派它（見 `quota_cooling`）。
pub(crate) async fn requeue_for_quota(pool: &sqlx::SqlitePool, task_id: &str, now: i64) -> Result<(), sqlx::Error> {
    let t = store::get_task(pool, task_id).await?.ok_or(sqlx::Error::RowNotFound)?;
    let order = match t.agent_id.as_deref() {
        Some(a) => crate::tasks::agents::first_queued_order(pool, a).await?.map(|f| f - 1.0).unwrap_or(t.sort_order),
        None => t.sort_order,
    };
    sqlx::query(
        "UPDATE tasks SET status = ?, resume_session = ?, sort_order = ?, tab_id = NULL,
             reply_wait_at = NULL, quota_wait_at = ? WHERE id = ?",
    )
    .bind(store::STATUS_QUEUED)
    .bind(t.session_id.is_some())
    .bind(order)
    .bind(now)
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 佇列裡等額度的卡：記下的時間還沒滿 `RETRY_AFTER` 就先不派。
pub(crate) fn quota_cooling(quota_wait_at: Option<i64>, now: i64) -> bool {
    quota_wait_at.is_some_and(|at| now - at < crate::tasks::quota::RETRY_AFTER.as_secs() as i64)
}

/// Agent 卡的完整提示：角色前綴＋卡片內容；非互動卡最後再加自主說明（有 `ask_human` 時用提問版）。
pub fn agent_prompt(prefix: &str, prompt: &str, interactive: bool, can_ask: bool) -> String {
    let p = with_role_prefix(prefix, prompt);
    match (interactive, can_ask) {
        (true, _) => p,
        (false, true) => format!("{p}\n\n{AUTONOMY_NOTE_ASK}"),
        (false, false) => format!("{p}\n\n{AUTONOMY_NOTE}"),
    }
}

/// Agent 的 worktree：已經有就沿用，沒有就在 `<專案>/agents/<id>/worktree`
/// 以 `aiterm-agent/<id>` 從 `agent.project_dir` 目前的 HEAD 建立，並寫回
/// agents 表。
///
/// **失敗就回 Err，不退回原資料夾。** 一般卡片的 `prepare_worktree` 會退回
/// 直接在 `project_dir` 跑，但 Agent 不行：兩個 Coder 會同時改同一個資料夾，
/// 而且之後的 `start_sha..end_sha` 範圍也就失去意義。
///
/// worktree 資料夾不見了但分支還在（使用者手動刪了資料夾）時，重新接上
/// 既有分支——之前的 commit 一個都不能丟。
pub(crate) async fn prepare_agent_worktree(
    pool: &sqlx::SqlitePool,
    project_storage_path: &std::path::Path,
    agent: &crate::tasks::agents::AgentRow,
) -> Result<std::path::PathBuf, String> {
    // 看板專案資料夾（含各 Agent 的工作資料夾）在程式庫裡面時，不能讓它把程式庫弄髒。
    if let Err(e) = crate::tasks::repo_check::exclude_inside(std::path::Path::new(&agent.project_dir), project_storage_path) {
        eprintln!("exclude board folder: {e}");
    }
    if let Some(p) = &agent.worktree_path {
        let pb = std::path::PathBuf::from(p);
        if pb.is_dir() {
            return Ok(pb);
        }
    }
    match VcsManager::detect_repo(&agent.project_dir).await {
        Ok(info) if info.vcs_type == VcsType::Git => {}
        _ => return Err(format!("Agent 的程式庫不是 git repo：{}", agent.project_dir)),
    }
    let path = crate::tasks::agents::agent_dir(project_storage_path, &agent.id).join("worktree");
    let path_str = path.to_string_lossy().into_owned();
    let branch = crate::tasks::agents::branch_name(&agent.id);
    let client = crate::vcs::git::GitClient::new(agent.project_dir.clone(), None);
    if let Err(create_err) = client.create_worktree(&path_str, &branch).await {
        // 最常見的原因是分支已經存在（資料夾被刪、分支還在）。先 prune 掉
        // 指向不存在資料夾的登記，再接上既有分支。
        let _ = client.prune_worktrees().await;
        if let Err(attach_err) = client.attach_worktree(&path_str, &branch).await {
            return Err(format!("建立 Agent worktree 失敗：{create_err}；{attach_err}"));
        }
    }
    crate::tasks::agents::set_agent_worktree(pool, &agent.id, &path_str, &branch)
        .await
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// Agent 卡片的 watch 結束後、`finish_task` 之前呼叫。
///
/// - 成功：worktree 有未提交改動就 `Module: <標題>` commit 掉，記下 `end_sha`。
///   沒有改動仍算成功（`end_sha` 會等於 `start_sha`）。commit 失敗（例如
///   pre-commit hook 擋下）→ 改判失敗，錯誤訊息帶 git 的輸出。
/// - 失敗／取消（含卡住、手動停止）：暫停 Agent，讓使用者決定重試或跳過——
///   不能讓下一張卡建立在半成品上。
///
/// 回傳最終要寫進資料庫的結果。
pub(crate) async fn finalize_agent_card(
    pool: &sqlx::SqlitePool,
    agent_id: &str,
    task_id: &str,
    commit_message: &str,
    worktree: &std::path::Path,
    outcome: monitor::TaskOutcome,
) -> monitor::TaskOutcome {
    let pause = || async {
        if let Err(e) = crate::tasks::agents::set_paused(pool, agent_id, Some(task_id)).await {
            eprintln!("pause agent {agent_id}: {e}");
        }
    };
    if !matches!(outcome, monitor::TaskOutcome::Success) {
        pause().await;
        return outcome;
    }
    let client = crate::vcs::git::GitClient::new(worktree.to_string_lossy().into_owned(), None);
    let commit = async {
        if client.has_uncommitted_changes().await? {
            client.commit_all(commit_message).await?;
        }
        client.head_sha().await
    };
    match commit.await {
        Ok(sha) => {
            if let Err(e) = store::set_end_sha(pool, task_id, &sha).await {
                eprintln!("set_end_sha {task_id}: {e}");
            }
            monitor::TaskOutcome::Success
        }
        Err(e) => {
            pause().await;
            monitor::TaskOutcome::Failed(format!("自動 commit 失敗：{e}"))
        }
    }
}

/// 工程師卡開工前，把協調者最後一次成功合併的結果合進來（通常是快轉）：一輪合併完、
/// 同一個專案開新一輪時，工程師要從整合後的版本開始，而不是自己的舊分支。
/// 回傳有沒有合進新東西。衝突：還原、回 Err（派工失敗、工程師暫停）。
pub(crate) async fn sync_with_integration(
    pool: &sqlx::SqlitePool,
    task: &TaskRow,
    worktree: &std::path::Path,
) -> Result<bool, String> {
    let integrated: Option<String> = sqlx::query_scalar(
        "SELECT end_sha FROM tasks WHERE card_kind = 'merge' AND outcome = 'success' AND end_sha IS NOT NULL
         ORDER BY finished_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some(sha) = integrated else { return Ok(false) };
    let _ = task;
    let client = crate::vcs::git::GitClient::new(worktree.to_string_lossy().into_owned(), None);
    let before = client.head_sha().await?;
    if let Err(e) = client.merge_rev(&sha, "Merge 整合結果（上一輪合併）").await {
        let files = client.conflicted_files().await.unwrap_or_default();
        if client.is_merge_in_progress().await {
            let _ = client.merge_abort().await;
        }
        return Err(if files.is_empty() {
            format!("開工前合併上一輪的整合結果失敗：{e}")
        } else {
            format!("開工前合併上一輪的整合結果有衝突：{}。請在工作資料夾解決後按重試", files.join("、"))
        });
    }
    Ok(client.head_sha().await? != before)
}

/// 開工前把別位工程師的前置（審過的版本）合進這張卡的工作資料夾。
/// 回傳合進來的前置卡 id。衝突或其他失敗：還原合併、回 Err（派工失敗、工程師暫停）。
pub(crate) async fn premerge_prerequisites(
    pool: &sqlx::SqlitePool,
    task: &TaskRow,
    worktree: &std::path::Path,
) -> Result<Vec<String>, String> {
    let plan = crate::tasks::workflow::premerge_plan(pool, task).await.map_err(|e| e.to_string())?;
    let client = crate::vcs::git::GitClient::new(worktree.to_string_lossy().into_owned(), None);
    let mut merged = Vec::new();
    for (id, title, sha) in plan {
        if let Err(e) = client.merge_rev(&sha, &format!("Merge 前置：{title}")).await {
            let files = client.conflicted_files().await.unwrap_or_default();
            if client.is_merge_in_progress().await {
                let _ = client.merge_abort().await;
            }
            return Err(if files.is_empty() {
                format!("開工前合併前置「{title}」失敗：{e}")
            } else {
                format!("開工前合併前置「{title}」有衝突：{}。請在工作資料夾解決後按重試", files.join("、"))
            });
        }
        merged.push(id);
    }
    Ok(merged)
}

/// Agent 卡片的自動 commit 訊息：模組、修正、整合各一種前綴，git log 才分得出來。
pub(crate) fn commit_message_for(card_kind: &str, title: &str) -> String {
    match card_kind {
        crate::tasks::workflow::KIND_FIX => format!("Fix: {title}"),
        crate::tasks::workflow::KIND_MERGE => format!("Integrate: {title}"),
        _ => format!("Module: {title}"),
    }
}

/// Reviewer 的受審目錄：`<專案>/agents/<reviewer>/reviews/<review_id>`，以
/// detached worktree 指向被審版本（`task.end_sha`）。重試時舊的目錄先清掉。
/// 派工準備階段的 git 操作一次一個，見 `RealDispatcher::dispatch`。
static GIT_PREP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 驗收卡提示最後附上的「可用的埠」。
fn verify_ports_note(ports: &[u16]) -> String {
    let list = ports.iter().map(u16::to_string).collect::<Vec<_>>().join("、");
    let first = ports.first().map(u16::to_string).unwrap_or_default();
    format!(
        "\n\n可用的埠：{list}。伺服器只能用這些埠（例如 PORT={first}，或工具的 --port {first}），\
         不要用預設埠，以免跟使用者正在跑的服務衝突。"
    )
}

pub(crate) async fn prepare_review_worktree(
    project_storage_path: &std::path::Path,
    reviewer_id: &str,
    task: &TaskRow,
) -> Result<std::path::PathBuf, String> {
    let sha = task.end_sha.as_deref().ok_or_else(|| "審查卡沒有受審版本（end_sha）".to_string())?;
    let path = crate::tasks::agents::agent_dir(project_storage_path, reviewer_id).join("reviews").join(&task.id);
    let path_str = path.to_string_lossy().into_owned();
    let repo = crate::vcs::git::GitClient::new(task.project_dir.clone(), None);
    if path.exists() {
        let _ = repo.remove_worktree_force(&path_str).await;
        let _ = std::fs::remove_dir_all(&path);
        let _ = repo.prune_worktrees().await;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    repo.add_detached_worktree(&path_str, sha).await.map_err(|e| format!("建立審查目錄失敗：{e}"))?;
    // review.json 寫在卡片資料夾，先確保它存在。
    let _ = std::fs::create_dir_all(crate::tasks::task_dir(project_storage_path, &task.id));
    Ok(path)
}

/// Reviewer 卡片結案（`finish_task` 之前）：讀 review.json 寫入判定，失敗就
/// 暫停 Reviewer，最後清掉受審目錄。回傳最終結果。
pub(crate) async fn finalize_review_card(
    pool: &sqlx::SqlitePool,
    project_storage_path: &std::path::Path,
    reviewer_id: &str,
    task_id: &str,
    repo_dir: &str,
    worktree: &std::path::Path,
    outcome: monitor::TaskOutcome,
) -> monitor::TaskOutcome {
    let outcome = if matches!(outcome, monitor::TaskOutcome::Success) {
        let path = crate::tasks::workflow::review_json_path(project_storage_path, task_id);
        match std::fs::read_to_string(&path)
            .map_err(|e| format!("找不到 {}：{e}", path.display()))
            .and_then(|t| crate::tasks::workflow::parse_review_json(&t))
        {
            Ok(v) => match crate::tasks::workflow::set_verdict(pool, task_id, &v).await {
                Ok(()) => monitor::TaskOutcome::Success,
                Err(e) => monitor::TaskOutcome::Failed(format!("寫入審查結果失敗：{e}")),
            },
            Err(e) => monitor::TaskOutcome::Failed(format!("審查沒有產出有效的 review.json：{e}")),
        }
    } else {
        outcome
    };
    if !matches!(outcome, monitor::TaskOutcome::Success) {
        if let Err(e) = crate::tasks::agents::set_paused(pool, reviewer_id, Some(task_id)).await {
            eprintln!("pause reviewer {reviewer_id}: {e}");
        }
    }
    let repo = crate::vcs::git::GitClient::new(repo_dir.to_string(), None);
    if let Err(e) = repo.remove_worktree_force(&worktree.to_string_lossy()).await {
        eprintln!("remove review worktree {}: {e}", worktree.display());
        let _ = std::fs::remove_dir_all(worktree);
        let _ = repo.prune_worktrees().await;
    }
    outcome
}

/// 驗收卡結案（`finish_task` 之前）：讀 verify.json 寫入判定（reasons = 沒通過的
/// 條件），失敗就暫停驗收者；收掉還佔著派工時給的埠的行程；最後清掉驗收目錄。
pub(crate) async fn finalize_verify_card(
    pool: &sqlx::SqlitePool,
    project_storage_path: &std::path::Path,
    verifier_id: &str,
    task_id: &str,
    repo_dir: &str,
    worktree: &std::path::Path,
    outcome: monitor::TaskOutcome,
) -> monitor::TaskOutcome {
    let outcome = if matches!(outcome, monitor::TaskOutcome::Success) {
        let path = crate::tasks::workflow::verify_json_path(project_storage_path, task_id);
        match std::fs::read_to_string(&path)
            .map_err(|e| format!("找不到 {}：{e}", path.display()))
            .and_then(|t| crate::tasks::workflow::parse_verify_json(&t))
        {
            Ok(report) => {
                let verdict = crate::tasks::workflow::ReviewVerdict {
                    verdict: report.verdict.clone(),
                    reasons: report.items.iter().filter(|i| i.status == "fail").map(|i| i.criterion.clone()).collect(),
                };
                match crate::tasks::workflow::set_verdict(pool, task_id, &verdict).await {
                    Ok(()) => monitor::TaskOutcome::Success,
                    Err(e) => monitor::TaskOutcome::Failed(format!("寫入驗收結果失敗：{e}")),
                }
            }
            Err(e) => monitor::TaskOutcome::Failed(format!("驗收沒有產出有效的 verify.json：{e}")),
        }
    } else {
        outcome
    };
    if !matches!(outcome, monitor::TaskOutcome::Success) {
        if let Err(e) = crate::tasks::agents::set_paused(pool, verifier_id, Some(task_id)).await {
            eprintln!("pause verifier {verifier_id}: {e}");
        }
    }
    let ports = crate::tasks::verify_ports::load_ports(&crate::tasks::task_dir(project_storage_path, task_id));
    let stopped = tokio::task::spawn_blocking(move || crate::tasks::verify_ports::stop_listeners(&ports)).await.unwrap_or_default();
    if !stopped.is_empty() {
        log::info!("verify {task_id}: stopped leftover servers {stopped:?}");
    }
    let repo = crate::vcs::git::GitClient::new(repo_dir.to_string(), None);
    if let Err(e) = repo.remove_worktree_force(&worktree.to_string_lossy()).await {
        eprintln!("remove verify worktree {}: {e}", worktree.display());
        let _ = std::fs::remove_dir_all(worktree);
        let _ = repo.prune_worktrees().await;
    }
    outcome
}

/// 拆解卡結案：驗證 tasks.json（實際建卡在 workflow::after_card_finished），
/// 不合格就判失敗並暫停 Orchestrator，最後清掉唯讀目錄。
pub(crate) async fn finalize_plan_card(
    pool: &sqlx::SqlitePool,
    project_storage_path: &std::path::Path,
    orchestrator_id: &str,
    task_id: &str,
    repo_dir: &str,
    worktree: &std::path::Path,
    outcome: monitor::TaskOutcome,
) -> monitor::TaskOutcome {
    let outcome = if matches!(outcome, monitor::TaskOutcome::Success) {
        let path = crate::tasks::workflow::tasks_json_path(project_storage_path, task_id);
        match std::fs::read_to_string(&path)
            .map_err(|e| format!("找不到 {}：{e}", path.display()))
            .and_then(|t| crate::tasks::workflow::parse_plan_json(&t))
        {
            Ok(_) => monitor::TaskOutcome::Success,
            Err(e) => monitor::TaskOutcome::Failed(format!("拆解沒有產出有效的 tasks.json：{e}")),
        }
    } else {
        outcome
    };
    if !matches!(outcome, monitor::TaskOutcome::Success) {
        if let Err(e) = crate::tasks::agents::set_paused(pool, orchestrator_id, Some(task_id)).await {
            eprintln!("pause orchestrator {orchestrator_id}: {e}");
        }
    }
    let repo = crate::vcs::git::GitClient::new(repo_dir.to_string(), None);
    if let Err(e) = repo.remove_worktree_force(&worktree.to_string_lossy()).await {
        eprintln!("remove plan worktree {}: {e}", worktree.display());
        let _ = std::fs::remove_dir_all(worktree);
        let _ = repo.prune_worktrees().await;
    }
    outcome
}

/// 審查不通過超過上限時送給前端的通知。
#[derive(Clone, serde::Serialize)]
struct ModuleAttentionEvent {
    project_id: String,
    project_name: String,
    module_id: String,
    module_title: String,
}

/// 驗收沒通過、要使用者決定時送給前端的通知。
#[derive(Clone, serde::Serialize)]
struct VerifyAttentionEvent {
    project_id: String,
    project_name: String,
    verify_id: String,
    /// 找不到負責模組的失敗項數。
    unowned: usize,
}

/// `task-finished` 的酬載：明確指出剛到達終局的是哪個專案的哪張卡片、
/// 用的是哪個分頁。前端靠它把對話記錄換成 xterm 序列化出來的乾淨版本
/// （那需要活著的 xterm 實例，只有前端有）。
#[derive(Clone, serde::Serialize)]
struct TaskFinishedEvent {
    project_id: String,
    task_id: String,
    tab_id: String,
    outcome: String,
    title: String,
    project_name: String,
    error_message: Option<String>,
}

/// Abstracts "actually run this card" so the loop is testable without an
/// `AppHandle`. Production impl is `RealDispatcher`.
#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync {
    async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String>;
}

/// The production dispatcher: spawn a visible `claude` tab, type the prompt,
/// mark the card running, then spawn a `monitor::watch` task that finishes
/// the card and re-pokes the scheduler when it ends.
pub struct RealDispatcher {
    pub app: AppHandle,
    pub pty: Arc<PtyManager>,
    pub config: Arc<ConfigStore>,
    pub bridge: Arc<crate::bridge::BridgeState>,
    pub secrets: Arc<crate::secret::SecretStore>,
    pub wake: Arc<Notify>,
    /// task_id → cancel sender, so `tasks_stop` can abort a running watch.
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<monitor::WatchControl>>>>,
    /// 額度用完自動續跑（見 `tasks::quota`）。
    pub quota: Arc<crate::tasks::quota::QuotaState>,
}

impl RealDispatcher {
    /// Codex 卡片的整行指令；提示詞寫進 `tasks/<id>/prompt.md`。
    #[allow(clippy::too_many_arguments)]
    async fn codex_launch(
        &self,
        project: &ProjectHandle,
        task: &TaskRow,
        agent: Option<&crate::tasks::agents::AgentRow>,
        prompt: &str,
        resume: Option<&str>,
        ask_url: Option<&str>,
        evidence: Option<&std::path::Path>,
        browser: crate::tasks::ask::Browser,
    ) -> Result<String, String> {
        let dir = crate::tasks::task_dir(&project.path, &task.id);
        let file = dir.join("prompt.md");
        std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&file, prompt))
            .map_err(|e| format!("寫入 Codex 提示詞失敗：{e}"))?;
        let hook = self.quota.hook_base(&task.id).await.map(|b| crate::tasks::codex::hook_command(&b));
        let playwright = evidence.map(|e| crate::tasks::ask::playwright_args(e, browser));
        let off = if self.config.get().task_board.lean_agent_dispatch {
            crate::tasks::codex::codex_home()
                .and_then(|h| std::fs::read_to_string(h.join("config.toml")).ok())
                .map(|t| crate::tasks::codex::user_mcp_servers(&t))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        crate::tasks::codex::launch_command(&crate::tasks::codex::CodexLaunch {
            model: agent.and_then(|a| a.model.as_deref()),
            prompt_file: &file,
            resume_session: resume,
            hook_command: hook.as_deref(),
            ask_url,
            playwright_args: playwright.as_deref(),
            disable_mcp: &off,
        })
        .ok_or_else(|| format!("專案路徑含有單引號，無法派給 Codex：{}", file.display()))
    }
}

#[async_trait::async_trait]
impl Dispatcher for RealDispatcher {
    async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
        // 派工會同時進行（見 drain_once），但準備階段的 git 操作（建 worktree、開工前合併）
        // 一次一個：同一個 repo 的多個 worktree 共用 .git，同時寫會撞到 git 的鎖檔。
        // 等 claude 啟動那段（最久的部分）才放開。
        let git_prep = GIT_PREP.lock().await;
        let attachments = store::list_attachments(&project.pool, &task.id)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| {
                crate::tasks::resolve_stored_path(&project.path, &a.stored_path)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        let mut prompt = dispatch::build_prompt(&task.body, &attachments);
        let claude_cmd = self.config.get().task_board.claude_command;
        // Agent 卡片：在 Agent 自己的 worktree 跑、提示詞加角色前綴，並且
        // **不**寫卡片的 worktree 欄位——那兩欄是每卡隔離用的，有值會讓
        // 看板顯示「合併回原分支」，而 Agent 的成果是整個分支一起合併。
        let agent = match &task.agent_id {
            Some(aid) => Some(
                crate::tasks::agents::get_agent(&project.pool, aid)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("找不到指派的 Agent：{aid}"))?,
            ),
            None => None,
        };
        // 選了 Codex 的 Agent 的非互動卡片：用 `codex exec` 做（見 tasks::codex）。
        let codex = !task.interactive && agent.as_ref().is_some_and(|a| a.cli == crate::tasks::agents::CLI_CODEX);
        // 只有指令確實是 claude 時才給 session id——別的指令接上這個旗標
        // 會直接啟動失敗（見 dispatch::looks_like_claude）。
        // 重啟時被中斷的卡片：接回原本的對話（同一個 session id）。
        // Codex 自己產生 session id（SessionStart hook 回報），接著做時用卡片上記的那個。
        let resume = task.resume_session
            && task.session_id.is_some()
            && (codex || dispatch::looks_like_claude(&claude_cmd));
        let codex_resume = if codex && resume { task.session_id.clone() } else { None };
        let session_id = if codex {
            None
        } else if resume {
            task.session_id.clone()
        } else {
            dispatch::looks_like_claude(&claude_cmd).then(|| uuid::Uuid::new_v4().to_string())
        };
        // 能掛 ask_human／瀏覽器的卡：claude（有 session id）或 Codex。
        let managed = session_id.is_some() || codex;
        // 非互動的 Agent 卡掛上 ask_human（需要人決定時提問）；驗收卡另外掛上
        // 瀏覽器工具。做不到就照舊派工（驗收者會把需要瀏覽器的條件標成 manual）。
        let is_verify = task.card_kind == crate::tasks::workflow::KIND_VERIFY && task.agent_id.is_some();
        let ask_url = if managed && !task.interactive && task.agent_id.is_some() {
            self.quota.mcp_url(&task.id).await
        } else {
            None
        };
        let evidence = (managed && is_verify).then(|| crate::tasks::workflow::evidence_dir(&project.path, &task.id));
        let browser = if evidence.is_some() { crate::tasks::ask::detect_browser() } else { crate::tasks::ask::Browser::SystemChrome };
        if let Some(dir) = evidence.as_ref().filter(|_| codex) {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("evidence dir for {}: {e}", task.id);
            }
        }
        // Claude 用 `--mcp-config` 檔；Codex 的 MCP 從 `-c` 給（見 codex::launch_command）。
        let claude_mcp = if codex { None } else { crate::tasks::ask::mcp_config_json_for(ask_url.as_deref(), evidence.as_deref(), browser) };
        let ask_args = match claude_mcp {
            Some(json) => {
                let dir = crate::tasks::task_dir(&project.path, &task.id);
                let path = dir.join("mcp.json");
                let written = std::fs::create_dir_all(&dir)
                    .and_then(|_| evidence.as_ref().map_or(Ok(()), std::fs::create_dir_all))
                    .and_then(|_| std::fs::write(&path, json));
                match written {
                    Ok(()) => crate::tasks::quota::path_flag("--mcp-config", &path).and_then(|f| {
                        crate::tasks::ask::allowed_tools(ask_url.is_some(), evidence.is_some())
                            .map(|t| format!("{f} --allowedTools {t}"))
                    }),
                    Err(e) => {
                        eprintln!("mcp config for {}: {e}", task.id);
                        None
                    }
                }
            }
            None => None,
        };
        let can_ask = if codex { ask_url.is_some() } else { ask_args.is_some() && ask_url.is_some() };

        let bridge_env = dispatch::resolve_bridge_env(
            &self.config,
            task,
            self.bridge.port(),
            self.secrets.get(crate::bridge::auth::BRIDGE_TOKEN_KEY).ok().flatten(),
        );
        let readonly = task.card_kind == crate::tasks::workflow::KIND_REVIEW
            || task.card_kind == crate::tasks::workflow::KIND_PLAN
            || task.card_kind == crate::tasks::workflow::KIND_VERIFY;
        let (effective_dir, worktree_info) = if let Some(agent) = agent.as_ref().filter(|_| readonly) {
            // 審查卡、拆解卡、驗收卡：在受審版本的唯讀目錄跑，不記 start_sha（那兩欄存的是受審範圍）。
            let wt = prepare_review_worktree(&project.path, &agent.id, task).await?;
            prompt = agent_prompt(&agent.prompt_prefix, &prompt, task.interactive, can_ask);
            if is_verify {
                // 埠在這時才挑：建卡到開工之間可能隔很久。結案時收掉還佔著這些埠的行程。
                let ports = crate::tasks::verify_ports::pick_free_ports(3);
                crate::tasks::verify_ports::save_ports(&crate::tasks::task_dir(&project.path, &task.id), &ports);
                prompt.push_str(&verify_ports_note(&ports));
            }
            (wt, None)
        } else if let Some(agent) = &agent {
            let wt = prepare_agent_worktree(&project.pool, &project.path, agent).await?;
            // 先跟上上一輪的整合結果、再合別位工程師的前置；之後才記 start_sha，
            // 審查範圍只含這張卡自己的改動。
            // 接著做的卡片跳過：半成品還沒 commit，同步與合併會動到它，start_sha 也記過了。
            if !resume && (task.card_kind == crate::tasks::workflow::KIND_WORK || task.card_kind == crate::tasks::workflow::KIND_FIX) {
                sync_with_integration(&project.pool, task, &wt).await?;
            }
            let merged = if resume { Vec::new() } else { premerge_prerequisites(&project.pool, task, &wt).await? };
            if !merged.is_empty() {
                let json = serde_json::to_string(&merged).unwrap_or_default();
                if let Err(e) = sqlx::query("UPDATE tasks SET premerged = ? WHERE id = ?")
                    .bind(&json)
                    .bind(&task.id)
                    .execute(&project.pool)
                    .await
                {
                    eprintln!("set premerged {}: {e}", task.id);
                }
            }
            if !resume {
                match crate::vcs::git::GitClient::new(wt.to_string_lossy().into_owned(), None).head_sha().await {
                    Ok(sha) => {
                        if let Err(e) = store::set_start_sha(&project.pool, &task.id, &sha).await {
                            eprintln!("set_start_sha {}: {e}", task.id);
                        }
                    }
                    Err(e) => eprintln!("head_sha for agent card {}: {e}", task.id),
                }
            }
            prompt = agent_prompt(&agent.prompt_prefix, &prompt, task.interactive, can_ask);
            (wt, None)
        } else {
            let isolate = resolve_isolation(
                task.isolate_worktree,
                self.config.get().task_board.isolate_with_worktree,
            );
            prepare_worktree(&project.path, &task.id, &task.project_dir, isolate).await
        };
        // 非互動的 claude 卡片掛上額度 hook：撞到用量上限時卡片改成「等待額度」，
        // 換帳號或過一段時間自動續跑。做不到（接收器起不來、路徑有特殊字元）就照舊派工。
        // 精簡啟動（非互動的 Agent 卡）：關掉使用者的外掛、只用我們給的 MCP、不載入技能。
        let lean = session_id.is_some() && !task.interactive && task.agent_id.is_some() && self.config.get().task_board.lean_agent_dispatch;
        let plugins_off = if lean {
            dirs::home_dir()
                .and_then(|h| std::fs::read_to_string(h.join(".claude").join("settings.json")).ok())
                .map(|t| crate::tasks::quota::user_enabled_plugins(&t))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let quota_args = if session_id.is_some() && !task.interactive {
            match self.quota.hook_base(&task.id).await {
                Some(base) => {
                    let dir = crate::tasks::task_dir(&project.path, &task.id);
                    let path = dir.join("claude-settings.json");
                    match std::fs::create_dir_all(&dir)
                        .and_then(|_| std::fs::write(&path, crate::tasks::quota::hook_settings_json_with(&base, &plugins_off)))
                    {
                        Ok(()) => crate::tasks::quota::settings_flag(&path),
                        Err(e) => {
                            eprintln!("quota settings for {}: {e}", task.id);
                            None
                        }
                    }
                }
                None => None,
            }
        } else {
            None
        };
        // 依角色指定的模型（只對 claude 指令加，看不出是 claude 就不動）。
        let model_args = if session_id.is_some() { model_flag(agent.as_ref().and_then(|a| a.model.as_deref())) } else { None };
        let lean_args = lean.then(|| crate::tasks::quota::LEAN_FLAGS.to_string());
        let extra_args = join_args([model_args, quota_args.clone(), ask_args, lean_args]);
        if resume {
            // 卡片內容已經在對話裡了，只要叫它接著做。驗收卡的埠剛重新挑過，要再告訴它一次。
            let ports_note = prompt.rfind("\n\n可用的埠：").map(|i| prompt[i..].to_string()).unwrap_or_default();
            let base = if codex { dispatch::RESUME_PROMPT_CODEX } else { dispatch::RESUME_PROMPT };
            prompt = format!("{base}{ports_note}");
        } else if codex {
            prompt = codex_autonomy(&prompt);
        }
        // Codex：提示詞寫成檔案，指令只叫它去讀；連同 hook、MCP 一起組成整行指令。
        let codex_line = if codex {
            Some(self.codex_launch(project, task, agent.as_ref(), &prompt, codex_resume.as_deref(), ask_url.as_deref(), evidence.as_deref(), browser).await?)
        } else {
            None
        };
        let agent_ctx = agent.map(|a| (a.id, a.kind, effective_dir.clone()));
        let card_kind = task.card_kind.clone();
        let repo_dir = task.project_dir.clone();
        let effective_dir_str = effective_dir.to_string_lossy().into_owned();
        // 額度 hook 與 ask_human 都要先登記才收得到；登記發生在 claude 啟動之前
        // （見 spawn_and_run 的 on_spawned）。
        let register = (quota_args.is_some() || can_ask || codex).then(|| {
            let quota = self.quota.clone();
            let task_id = task.id.clone();
            let pool = project.pool.clone();
            let label = crate::tasks::quota::ReplyLabel {
                title: task.title.clone(),
                project_name: project.name.clone(),
                project_id: project.id.clone(),
                agent_id: task.agent_id.clone(),
            };
            move |tab_id: &str| {
                quota.register(&task_id, tab_id, pool, label);
                if codex {
                    quota.set_blocking_ask(&task_id);
                }
            }
        });
        drop(git_prep);
        let on_spawned = |tab_id: &str| {
            if let Some(register) = register {
                register(tab_id);
            }
        };
        let spawned = match &codex_line {
            Some(line) => {
                let env = self
                    .secrets
                    .get(crate::tasks::codex::CODEX_ENV_KEY)
                    .ok()
                    .flatten()
                    .map(|t| dispatch::parse_env_lines(&t))
                    .unwrap_or_default();
                dispatch::spawn_command(&self.app, &self.pty, &effective_dir_str, line, env, on_spawned).await
            }
            None => {
                dispatch::spawn_and_run(
                    &self.app,
                    &self.pty,
                    &effective_dir_str,
                    &claude_cmd,
                    session_id.as_deref(),
                    &prompt,
                    !task.interactive,
                    bridge_env,
                    extra_args.as_deref(),
                    resume,
                    on_spawned,
                )
                .await
            }
        };
        let (tab_id, disp) = match spawned {
            Ok(v) => v,
            Err(e) => {
                self.quota.unregister(&task.id);
                return Err(e);
            }
        };
        if resume {
            if let Err(e) = store::clear_resume(&project.pool, &task.id).await {
                eprintln!("clear_resume {}: {e}", task.id);
            }
            // 中斷前還有沒回答的問題：恢復「等決策」，完成標記先不算結案。
            if crate::tasks::questions::has_open(&project.pool, &task.id).await.unwrap_or(false) {
                self.quota.set_asking(&task.id, true);
            }
        }
        // 卡片在進到這裡之前就已經被 `claim_for_dispatch` 標成 running 了
        // （見 drain_once）——這裡只補上分頁 id。順序不能倒過來：spawn 要
        // 等 claude 的 TUI 起來、最久 30 秒，那段期間卡片若還是 queued，
        // 下一次掃描就會再派一次同一張。
        store::set_tab_id(&project.pool, &task.id, &tab_id)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(sid) = session_id.as_deref() {
            if let Err(e) = store::set_session_id(&project.pool, &task.id, sid).await {
                // 記不起來只代表這張卡片拿不到完整記錄，不值得讓派工失敗。
                eprintln!("set_session_id {}: {e}", task.id);
            }
        }
        if let Some((wt_path, wt_branch)) = &worktree_info {
            if let Err(e) = store::set_worktree(&project.pool, &task.id, wt_path, wt_branch).await {
                // 記不起來只代表「合併回原分支」按鈕不會出現，worktree
                // 本身已經建好、Claude Code 照樣在裡面跑——不值得讓派工失敗。
                eprintln!("set_worktree {}: {e}", task.id);
            }
        }
        let _ = self.app.emit("tasks-updated", ());

        let (cancel_tx, cancel_rx) = oneshot::channel::<monitor::WatchControl>();
        self.cancels.lock().insert(task.id.clone(), cancel_tx);

        // 這個 async block 捕獲該專案的 pool 與資料夾路徑的複本，所以
        // watch 結束後的寫回完全不需要回頭查 registry。
        let pool = project.pool.clone();
        let project_path = project.path.clone();
        let project_id = project.id.clone();
        let pty = self.pty.clone();
        let app = self.app.clone();
        let wake = self.wake.clone();
        let cancels = self.cancels.clone();
        let task_id = task.id.clone();
        let task_title = task.title.clone();
        let project_name = project.name.clone();
        // move 而不是 clone——上面兩處用的都是 `as_deref()`，`session_id`
        // 之後不再被碰。
        let session_id_for_watch = session_id;
        let baselines = monitor::Baselines { bell: disp.bell_baseline, marker: disp.marker_baseline };
        let watch_mode = if task.interactive { monitor::WatchMode::Interactive } else { monitor::WatchMode::Auto };
        // 非互動卡片連續多久沒輸出才判定「疑似卡住」，來自使用者可調的
        // 設定——見 monitor.rs 對這個誤判成因的說明（Claude Code 自己派
        // subagent 在背景工作時，終端機常常安靜超過原本寫死的 120 秒）。
        let stuck_timeout_ms = (self.config.get().task_board.stuck_timeout_secs as u64) * 1000;
        let thresholds = monitor::Thresholds {
            quiet_stuck_ms: stuck_timeout_ms,
            min_run_ms: stuck_timeout_ms,
            ..monitor::Thresholds::default()
        };
        let quota = self.quota.clone();
        tauri::async_runtime::spawn(async move {
            let waiting = {
                let quota = quota.clone();
                let task_id = task_id.clone();
                move || quota.is_waiting(&task_id)
            };
            let holding = {
                let quota = quota.clone();
                let task_id = task_id.clone();
                move || quota.is_holding_marker(&task_id)
            };
            let outcome = monitor::watch_with_quota(
                &pty, &tab_id, cancel_rx, baselines, thresholds, watch_mode, &waiting, &holding,
            ).await;
            quota.unregister(&task_id);
            // Codex：結束碼非 0 時從對話記錄判斷是不是額度用完（不會觸發任何 hook）。
            let outcome = if codex {
                match codex_verdict(&pool, &task_id, outcome).await {
                    CodexVerdict::Requeue => {
                        if let Err(e) = crate::tasks::questions::cancel_open_for_task(&pool, &task_id).await {
                            eprintln!("cancel questions {task_id}: {e}");
                        }
                        let _ = write_transcript(&pty, &project_path, &task_id, &tab_id);
                        cancels.lock().remove(&task_id);
                        let _ = app.emit("tasks-updated", ());
                        wake.notify_one();
                        return;
                    }
                    CodexVerdict::Outcome(o) => o,
                }
            } else {
                outcome
            };
            // 卡片結束了，它還沒回答的問題作廢（收件匣與 Telegram 會跟著更新）。
            match crate::tasks::questions::cancel_open_for_task(&pool, &task_id).await {
                Ok(cancelled) if !cancelled.is_empty() => {
                    let _ = app.emit("questions-updated", ());
                    for q in cancelled {
                        if let Some(mid) = q.telegram_message_id {
                            crate::telegram::mark_question_closed(&app, mid, &q, None);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("cancel questions {task_id}: {e}"),
            }
            let outcome = match &agent_ctx {
                Some((agent_id, _, worktree)) if card_kind == crate::tasks::workflow::KIND_REVIEW => {
                    finalize_review_card(&pool, &project_path, agent_id, &task_id, &repo_dir, worktree, outcome).await
                }
                Some((agent_id, _, worktree)) if card_kind == crate::tasks::workflow::KIND_PLAN => {
                    finalize_plan_card(&pool, &project_path, agent_id, &task_id, &repo_dir, worktree, outcome).await
                }
                Some((agent_id, _, worktree)) if card_kind == crate::tasks::workflow::KIND_VERIFY => {
                    finalize_verify_card(&pool, &project_path, agent_id, &task_id, &repo_dir, worktree, outcome).await
                }
                Some((agent_id, _, worktree)) => {
                    let msg = commit_message_for(&card_kind, &task_title);
                    finalize_agent_card(&pool, agent_id, &task_id, &msg, worktree, outcome).await
                }
                None => outcome,
            };
            let transcript = write_transcript(&pty, &project_path, &task_id, &tab_id);
            persist_outcome(
                &pool,
                crate::tasks::session_log::claude_projects_root().as_deref(),
                session_id_for_watch.as_deref(),
                &project_path,
                &task_id,
                transcript.as_deref(),
                &outcome,
            ).await;
            cancels.lock().remove(&task_id);
            let _ = app.emit("tasks-updated", ());
            // 帶資料的完成事件，給對話記錄乾淨化用。不能只靠 tasks-updated：
            // 那個沒有酬載，接收端得自己比對前後狀態才知道「哪一張剛完成」，
            // 而唯一在做這件事的地方是該專案的看板——但看板只有在該專案是
            // 當前分頁時才掛載，別的專案完成時根本沒人在聽。
            let _ = app.emit(
                "task-finished",
                TaskFinishedEvent {
                    project_id: project_id.clone(),
                    task_id: task_id.clone(),
                    tab_id: tab_id.clone(),
                    outcome: outcome.as_str().to_string(),
                    title: task_title.clone(),
                    project_name: project_name.clone(),
                    error_message: outcome.error_message().map(str::to_string),
                },
            );
            // Agent 協作第二期：建立後續的審查／修正／合併卡。
            if agent_ctx.is_some() {
                match crate::tasks::workflow::after_card_finished(&pool, &project_path, &task_id).await {
                    Ok(events) => {
                        for ev in events {
                            match ev {
                                crate::tasks::workflow::WorkflowEvent::Attention { module_id, module_title } => {
                                    let _ = app.emit(
                                        "agent-module-attention",
                                        ModuleAttentionEvent {
                                            project_id: project_id.clone(),
                                            project_name: project_name.clone(),
                                            module_id,
                                            module_title,
                                        },
                                    );
                                }
                                crate::tasks::workflow::WorkflowEvent::VerifyAttention { verify_id, unowned } => {
                                    let _ = app.emit(
                                        "agent-verify-attention",
                                        VerifyAttentionEvent {
                                            project_id: project_id.clone(),
                                            project_name: project_name.clone(),
                                            verify_id,
                                            unowned,
                                        },
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("workflow after {task_id}: {e}"),
                }
                let _ = app.emit("tasks-updated", ());
            }
            wake.notify_one();
        });
        Ok(())
    }
}

/// App 重啟後怎麼處理上次還在執行的卡片（見
/// docs/superpowers/specs/2026-10-17-agent-resume-after-restart-design.md）。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RecoverSummary {
    /// 找得到對話記錄，下次派工用 `claude --resume` 接著做。
    pub resumed: u64,
    /// claude 沒啟動起來，回到佇列從頭再來。
    pub restarted: u64,
    /// 互動卡、看板一般卡片：照舊中斷。
    pub cancelled: u64,
}

/// 非互動的 Agent 卡回到該 Agent 佇列的最前面（有對話記錄就標成接著做），Agent 不暫停；
/// 其他卡片照舊收成 cancelled 並暫停 Agent。`projects_root` 是 `~/.claude/projects`。
pub(crate) async fn recover_after_restart(
    pool: &sqlx::SqlitePool,
    projects_root: Option<&std::path::Path>,
) -> Result<RecoverSummary, sqlx::Error> {
    let running: Vec<TaskRow> = sqlx::query_as("SELECT * FROM tasks WHERE status = 'running'").fetch_all(pool).await?;
    let mut out = RecoverSummary::default();
    for t in running.iter().filter(|t| !t.interactive) {
        let Some(agent) = t.agent_id.as_deref() else { continue };
        let codex = crate::tasks::agents::get_agent(pool, agent)
            .await?
            .is_some_and(|a| a.cli == crate::tasks::agents::CLI_CODEX);
        let resume = if codex {
            // Codex 的記錄路徑由 SessionStart hook 記在卡片上；沒答的問題作廢（沒有人在等它了）。
            crate::tasks::questions::cancel_open_for_task(pool, &t.id).await?;
            t.session_id.is_some() && t.session_path.as_deref().is_some_and(|p| std::path::Path::new(p).is_file())
        } else {
            match (t.session_id.as_deref(), projects_root) {
                (Some(sid), Some(root)) => crate::tasks::session_log::find_session_log(root, sid).is_some(),
                _ => false,
            }
        };
        let order = match crate::tasks::agents::first_queued_order(pool, agent).await? {
            Some(first) => first - 1.0,
            None => t.sort_order,
        };
        sqlx::query(
            "UPDATE tasks SET status = ?, resume_session = ?, sort_order = ?, tab_id = NULL,
                 reply_wait_at = NULL, quota_wait_at = NULL WHERE id = ?",
        )
        .bind(store::STATUS_QUEUED)
        .bind(resume)
        .bind(order)
        .bind(&t.id)
        .execute(pool)
        .await?;
        if resume {
            out.resumed += 1;
        } else {
            out.restarted += 1;
        }
    }
    // 剩下的（互動卡、看板一般卡片）照舊。暫停一定要在 recover 之前：recover 之後就分不出
    // 哪些卡片是「被重啟中斷的執行中」了。
    crate::tasks::agents::pause_agents_of_orphaned_cards(pool).await?;
    out.cancelled = store::recover_orphaned_running(pool).await?;
    Ok(out)
}

/// Snapshot the tab's recent output to `<task_dir>/transcript.txt`. Best
/// effort — returns the path on success, `None` (and logs) on failure.
fn write_transcript(
    pty: &PtyManager,
    project_path: &std::path::Path,
    task_id: &str,
    tab_id: &str,
) -> Option<String> {
    let text = pty.get_recent_output(tab_id, 200_000).unwrap_or_default();
    let dir = crate::tasks::task_dir(project_path, task_id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("task transcript dir {dir:?}: {e}");
        return None;
    }
    let path = dir.join("transcript.txt");
    match std::fs::write(&path, text) {
        Ok(()) => Some(path.to_string_lossy().into_owned()),
        Err(e) => {
            eprintln!("write transcript {path:?}: {e}");
            None
        }
    }
}

/// 把一次執行的結果落地：完整的 session 記錄、然後結案。
///
/// 從 `RealDispatcher::dispatch` 的 watch closure 裡抽出來，因為那個 closure
/// 需要 `AppHandle`（`tauri` 依賴沒開 `test` feature，見 dispatch.rs 測試模組
/// 裡關於 `mock_builder` 的註解），而這一段不需要——抽出來就測得到。
///
/// `projects_root` 收 `Option<&Path>` 而不是自己呼叫 `claude_projects_root()`，
/// 純粹是為了讓測試能注入一棵假的樹。production 傳的就是那個函式的結果。
///
/// 順序是有意義的：`session_path` 必須在 `finish_task` 之前寫進去，因為
/// 呼叫端在 `finish_task` 之後緊接著發 `tasks-updated`，前端重載時該列
/// 必須已經是完整的。
async fn persist_outcome(
    pool: &sqlx::SqlitePool,
    projects_root: Option<&std::path::Path>,
    session_id: Option<&str>,
    project_path: &std::path::Path,
    task_id: &str,
    transcript: Option<&str>,
    outcome: &monitor::TaskOutcome,
) {
    // 完整的逐輪記錄。找不到就什麼都不做——上面剛寫好的 transcript.txt
    // 還在，沒有東西壞掉。
    if let (Some(sid), Some(root)) = (session_id, projects_root) {
        if let Some(path) = crate::tasks::session_log::copy_session_log(
            root, sid, project_path, task_id,
        ) {
            // 這一條比 `set_session_id` 的失敗嚴重得多，所以同樣要留下
            // 線索：檔案此刻已經躺在卡片資料夾裡了，路徑寫不進去的話它
            // 就變成沒有任何東西指向的孤兒，而使用者只會看到記錄莫名其妙
            // 退回終端機擷取。`set_session_id` 失敗則無害——watch closure
            // 手上本來就有自己的複本。
            // 存相對於專案資料夾的路徑（見 `to_stored_path`），專案搬走後才讀得到。
            let stored = crate::tasks::to_stored_path(project_path, std::path::Path::new(&path));
            if let Err(e) = store::set_session_path(pool, task_id, &stored).await {
                log::error!("set_session_path {task_id}: {e}");
            }
        }
    }
    // 用量（含子代理）存進卡片資料夾：`~/.claude` 裡的記錄 30 天後會被清掉。
    if let (Some(sid), Some(root)) = (session_id, projects_root) {
        if let Some(u) = crate::tasks::session_log::find_session_log(root, sid)
            .and_then(|main| crate::tasks::usage::session_usage(&main))
        {
            crate::tasks::usage::save(project_path, task_id, &u);
        }
    }
    // Codex 卡片：用量在它自己的 rollout 裡（路徑由 SessionStart hook 記在 session_path）。
    if let Ok(Some(t)) = store::get_task(pool, task_id).await {
        if let Some(path) = t.session_path.filter(|p| crate::tasks::codex::is_rollout_path(p)) {
            if let Some(u) = std::fs::read_to_string(&path).ok().and_then(|x| crate::tasks::codex::parse_usage(&x)) {
                crate::tasks::usage::save(project_path, task_id, &u);
            }
        }
    }
    let _ = store::finish_task(
        pool, task_id, outcome.as_str(), outcome.error_message(), transcript,
    ).await;
}

/// Promote as many queued cards as the rules allow, right now. Shared by the
/// loop and by tests. 跨所有已開啟的專案運作。
pub async fn drain_once(reg: &ProjectRegistry, dispatcher: &dyn Dispatcher, max_concurrent: u32) {
    // 資料夾被外部刪掉/搬走（或外接磁碟還沒掛上）的專案，這一輪整個跳過。
    //
    // 驅逐 registry 裡的 handle 是 `projects_list` 在做的，但那只有看板開著
    // 時才會跑，所以這裡不能假設 `reg.all()` 裡的每一個都還健在。而且下面
    // 收集 running 的迴圈遇到查詢失敗是 `return`——不先濾掉的話，**一個讀
    // 不到的專案會讓整個排程器停擺，連好的專案都不派工**。
    //
    // 這裡刻意只跳過、不驅逐：暫時讀不到（磁碟還沒掛上）之後回來時，
    // 下一輪就自動恢復。真正的驅逐留給 `projects_list`。
    let projects: Vec<ProjectHandle> = reg
        .all()
        .into_iter()
        .filter(|p| {
            let alive = p.path.is_dir();
            if !alive {
                eprintln!("scheduler: 專案資料夾不存在，這一輪跳過：{}", p.path.display());
            }
            alive
        })
        .collect();

    // Interactive cards bypass the concurrency cap and the solo-blocking
    // rule entirely — dispatch every queued one, unconditionally, first.
    // 逐專案處理即可：它們既不算並行額度也不受任何東西阻擋。
    for project in &projects {
        loop {
            let queued = match store::list_by_status(&project.pool, store::STATUS_QUEUED).await {
                Ok(q) => q,
                Err(e) => {
                    eprintln!("scheduler list queued (interactive pass, {}): {e}", project.name);
                    break;
                }
            };
            // Agent 的互動卡片不走這條：它們跟同一個 Agent 的其他卡片一樣
            // 要依序執行、占全域名額（見 `candidate_heads`）。
            let Some(next) = queued.into_iter().find(|t| t.interactive && t.agent_id.is_none()) else {
                break;
            };
            // 先原子地認領再去 spawn——理由見 store::claim_for_dispatch。
            // 認領不到代表別人先拿走了，直接重掃。
            match store::claim_for_dispatch(&project.pool, &next.id).await {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    eprintln!("claim {} failed ({}): {e}", next.id, project.name);
                    break;
                }
            }
            if let Err(e) = dispatcher.dispatch(project, &next).await {
                eprintln!("dispatch {} failed: {e}", next.id);
                let _ = store::finish_task(&project.pool, &next.id, "failed", Some(&e), None).await;
            }
        }
    }

    // 一般卡片：running 是跨專案的聯集（全域上限），queued 則每個專案
    // 交出 `candidate_heads`（未指派的隊首＋每個 Agent 的隊首）。專案之間
    // 依各自第一張候選的建立時間排序，專案內部維持 `sort_order`。
    // 認領後的派工先收著，等這一輪再也認領不到卡片時一起跑：每張派工都要等 claude
    // 啟動 15–20 秒，一張等一張的話好幾位工程師要很久才全部開工。認領當下卡片就已經是
    // running，所以下一輪掃描照樣算進上限、同一位 Agent 也不會再拿到第二張。
    loop {
        let mut pending: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>> = Vec::new();
        'scan: loop {
            let mut running: Vec<TaskRow> = Vec::new();
            let mut per_project: Vec<(String, Vec<TaskRow>)> = Vec::new();
            for project in &projects {
                // 閒著的工程師先接走別人還沒開始的實作卡，下面的隊首才會算到它。
                if let Err(e) = crate::tasks::workflow::rebalance_idle_coders(&project.pool).await {
                    eprintln!("scheduler rebalance ({}): {e}", project.name);
                }
                let project_running = match store::list_by_status(&project.pool, store::STATUS_RUNNING).await {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("scheduler list running ({}): {e}", project.name);
                        break 'scan;
                    }
                };
                let queued = match store::list_by_status(&project.pool, store::STATUS_QUEUED).await {
                    Ok(q) => q,
                    Err(e) => {
                        eprintln!("scheduler list queued ({}): {e}", project.name);
                        break 'scan;
                    }
                };
                let paused: std::collections::HashSet<String> =
                    match crate::tasks::agents::paused_agent_ids(&project.pool).await {
                        Ok(v) => v.into_iter().collect(),
                        Err(e) => {
                            eprintln!("scheduler list paused agents ({}): {e}", project.name);
                            break 'scan;
                        }
                    };
                let blocked = match crate::tasks::workflow::blocked_cards(&project.pool).await {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("scheduler blocked cards ({}): {e}", project.name);
                        break 'scan;
                    }
                };
                let mut heads = candidate_heads(&queued, &project_running, &paused, &blocked);
                // 等額度的 Codex 卡：冷卻時間內這位 Agent 先不派（它的隊首就是那張卡）。
                let now = chrono::Utc::now().timestamp();
                heads.retain(|h| !quota_cooling(h.quota_wait_at, now));
                // 用到 token 上限：Agent 卡先不派（已經在跑的照樣跑完），看板一般卡片不受影響。
                match crate::tasks::budget::over_budget(&project.pool, &project.path).await {
                    Ok(true) => heads.retain(|h| h.agent_id.is_none()),
                    Ok(false) => {}
                    Err(e) => eprintln!("scheduler budget ({}): {e}", project.name),
                }
                per_project.push((project.id.clone(), heads));
                // 互動卡片不占名額——但 Agent 的互動卡片要占，見 `candidate_heads`。
                running.extend(project_running.into_iter().filter(|t| !t.interactive || t.agent_id.is_some()));
            }

            let firsts = order_heads(
                per_project
                    .iter()
                    .filter_map(|(pid, hs)| hs.first().map(|h| (pid.clone(), h.clone())))
                    .collect(),
            );
            let heads: Vec<(String, TaskRow)> = firsts
                .into_iter()
                .flat_map(|(pid, _)| {
                    per_project
                        .iter()
                        .find(|(p, _)| *p == pid)
                        .map(|(_, hs)| hs.clone())
                        .unwrap_or_default()
                        .into_iter()
                        .map(move |h| (pid.clone(), h))
                })
                .collect();
            let mut dispatched = false;
            for (project_id, head) in heads {
                if pick_next(&running, std::slice::from_ref(&head), max_concurrent).is_none() {
                    continue;
                }
                let Some(project) = reg.get(&project_id) else {
                    continue; // 專案在這一輪之間被關閉了
                };
                // 先原子地認領再去 spawn——理由見 store::claim_for_dispatch。
                match store::claim_for_dispatch(&project.pool, &head.id).await {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(e) => {
                        eprintln!("claim {} failed ({}): {e}", head.id, project.name);
                        break 'scan;
                    }
                }
                pending.push(Box::pin(async move {
                    if let Err(e) = dispatcher.dispatch(&project, &head).await {
                        eprintln!("dispatch {} failed: {e}", head.id);
                        let _ = store::finish_task(&project.pool, &head.id, "failed", Some(&e), None).await;
                        // Agent 卡片派工失敗（例如 worktree 建不起來）跟執行失敗一樣
                        // 要暫停 Agent，否則下一輪會拿同一個壞掉的狀態派下一張。
                        if let Some(aid) = &head.agent_id {
                            let _ = crate::tasks::agents::set_paused(&project.pool, aid, Some(&head.id)).await;
                        }
                    }
                }));
                dispatched = true;
                break;
            }
            if !dispatched {
                break;
            }
        }
        // 這一批都派出去了再重看一次：派工期間可能有卡片已經結束（測試用的假派工會立刻
        // 結案），空出名額。一張都認領不到才收工。
        if pending.is_empty() {
            break;
        }
        futures_util::future::join_all(pending).await;
    }
}

/// Handed to the command layer so `tasks_move` / `tasks_stop` can wake the
/// scheduler and abort running watches.
#[derive(Clone)]
pub struct SchedulerHandle {
    pub wake: Arc<Notify>,
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<monitor::WatchControl>>>>,
}

impl SchedulerHandle {
    pub fn poke(&self) {
        self.wake.notify_one();
    }
    /// Fire the cancel signal for a running task, if present. Returns true
    /// if a watch was signalled.
    pub fn cancel(&self, task_id: &str) -> bool {
        self.send(task_id, monitor::WatchControl::Cancel)
    }
    /// Fire the manual-completion signal for a running task, if present.
    /// Returns true if a watch was signalled — see `commands::tasks::tasks_mark_done`
    /// for the fallback when this returns false (no active watch to signal).
    pub fn mark_done(&self, task_id: &str) -> bool {
        self.send(task_id, monitor::WatchControl::MarkDone)
    }
    fn send(&self, task_id: &str, msg: monitor::WatchControl) -> bool {
        if let Some(tx) = self.cancels.lock().remove(task_id) {
            let _ = tx.send(msg);
            true
        } else {
            false
        }
    }
}

/// 沒人回答的問題：每分鐘檢查一次，該提醒的（見 `questions::due_reminders`）再通知一次。
fn spawn_question_reminders(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let now = store::now_secs();
            let projects = app.state::<ProjectRegistry>().all();
            for p in projects {
                let Ok(due) = crate::tasks::questions::due_reminders(&p.pool, now).await else { continue };
                for q in due {
                    let _ = crate::tasks::questions::mark_reminded(&p.pool, q.id, now).await;
                    let title = store::get_task(&p.pool, &q.task_id).await.ok().flatten().map(|t| t.title).unwrap_or_default();
                    let _ = app.emit(
                        "agent-question",
                        serde_json::json!({
                            "project_id": p.id, "project_name": p.name, "title": title,
                            "id": q.id, "question": q.question, "reminder": true,
                        }),
                    );
                    if let Err(e) = crate::telegram::questions::send_reminder(&app, &q).await {
                        eprintln!("telegram reminder {}: {e}", q.id);
                    }
                }
            }
        }
    });
}

/// Start the long-lived scheduler. Called once from `lib.rs` `.setup()`.
pub fn spawn(app: AppHandle) -> SchedulerHandle {
    let wake = Arc::new(Notify::new());
    let cancels = Arc::new(parking_lot::Mutex::new(HashMap::new()));
    let handle = SchedulerHandle { wake: wake.clone(), cancels: cancels.clone() };

    tauri::async_runtime::spawn(async move {
        let config = app.state::<Arc<ConfigStore>>().inner().clone();
        let pty = app.state::<Arc<PtyManager>>().inner().clone();
        let bridge = app.state::<Arc<crate::bridge::BridgeState>>().inner().clone();
        let secrets = app.state::<Arc<crate::secret::SecretStore>>().inner().clone();

        // Startup recovery: clear orphaned `running` cards (their PTY died
        // with the previous process). 每個已開啟的專案各掃一次。
        // 非互動的 Agent 卡接著做，其他照舊中斷。見 recover_after_restart。
        {
            let reg = app.state::<ProjectRegistry>();
            let root = crate::tasks::session_log::claude_projects_root();
            let mut any = false;
            for project in reg.all() {
                match recover_after_restart(&project.pool, root.as_deref()).await {
                    Ok(s) if s != RecoverSummary::default() => {
                        any = true;
                        eprintln!(
                            "task board ({}): after restart {} resumed, {} restarted, {} cancelled",
                            project.name, s.resumed, s.restarted, s.cancelled
                        );
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("task board recovery scan ({}): {e}", project.name),
                }
            }
            if any {
                let _ = app.emit("tasks-updated", ());
            }
        }

        let quota = crate::tasks::quota::QuotaState::new();
        {
            let app = app.clone();
            quota.set_on_change(move || {
                let _ = app.emit("tasks-updated", ());
            });
        }
        {
            let app = app.clone();
            // 前端（永遠掛載的通知 hook）依「派工完成時通知」的設定發系統通知。
            quota.set_on_reply_wait(move |label| {
                let _ = app.emit(
                    "agent-reply-wait",
                    serde_json::json!({ "title": label.title, "project_name": label.project_name }),
                );
            });
        }
        {
            let app = app.clone();
            // 有新問題：前端收件匣／通知，同時送到 Telegram（有設定的話）。
            quota.set_on_question(move |label, q| {
                let _ = app.emit(
                    "agent-question",
                    serde_json::json!({
                        "project_id": label.project_id, "project_name": label.project_name,
                        "title": label.title, "id": q.id, "question": q.question, "reminder": false,
                    }),
                );
                let _ = app.emit("questions-updated", ());
                let (app, label, q) = (app.clone(), label.clone(), q.clone());
                tauri::async_runtime::spawn(async move {
                    match crate::telegram::questions::send_question(&app, &label.project_id, &label.project_name, &label.title, &q).await {
                        Ok(Some(mid)) => {
                            let reg = app.state::<ProjectRegistry>();
                            if let Some(p) = reg.get(&label.project_id) {
                                let _ = crate::tasks::questions::set_telegram_message(&p.pool, q.id, mid).await;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => eprintln!("telegram question {}: {e}", q.id),
                    }
                });
            });
        }
        // 答案要能從收件匣與 Telegram 送回來：讓指令拿得到這份狀態（解除等決策）。
        app.manage(quota.clone());
        spawn_question_reminders(app.clone());
        crate::tasks::quota::spawn_resumer(quota.clone(), pty.clone());
        let dispatcher = RealDispatcher {
            app: app.clone(),
            pty,
            config: config.clone(),
            bridge,
            secrets,
            wake: wake.clone(),
            cancels,
            quota,
        };

        loop {
            let max = config.get().task_board.max_concurrent;
            {
                let reg = app.state::<ProjectRegistry>();
                drain_once(&reg, &dispatcher, max).await;
            }
            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
        }
    });

    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::store::TaskRow;

    fn row(id: &str, parallel_ok: bool, sort_order: f64) -> TaskRow {
        TaskRow {
            id: id.into(),
            title: id.into(),
            body: String::new(),
            project_dir: "/r".into(),
            status: "queued".into(),
            parallel_ok,
            interactive: false,
            sort_order,
            outcome: None,
            tab_id: None,
            transcript_path: None,
            error_message: None,
            created_at: String::new(),
            dispatched_at: None,
            finished_at: None,
            ai_summary: None,
            archived_at: None,
            session_id: None,
            session_path: None,
            use_bridge: false,
            bridge_tiers: None,
            label: None,
            worktree_path: None,
            worktree_branch: None,
            isolate_worktree: None,
            agent_id: None,
            start_sha: None,
            end_sha: None,
            card_kind: "work".into(),
            module_id: None,
            review_of: None,
            verdict: None,
            review_reasons: None,
            module_override: false,
            is_spec: false,
            from_spec: None,
            quota_wait_at: None, depends_on: None, assigned_from: None, assigned_at: None, premerged: None, reply_wait_at: None, resume_session: false,
        }
    }

    fn agent_row(id: &str, agent: Option<&str>, sort_order: f64, interactive: bool) -> TaskRow {
        let mut r = row(id, true, sort_order);
        r.agent_id = agent.map(str::to_string);
        r.interactive = interactive;
        r
    }

    fn ids(v: &[TaskRow]) -> Vec<&str> {
        v.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn candidate_heads_gives_one_head_per_agent_plus_the_unassigned_head() {
        let queued = vec![
            agent_row("u1", None, 1.0, false),
            agent_row("a1", Some("A"), 2.0, false),
            agent_row("u2", None, 3.0, false),
            agent_row("b1", Some("B"), 4.0, false),
            agent_row("a2", Some("A"), 5.0, false),
        ];
        let heads = candidate_heads(&queued, &[], &Default::default(), &Default::default());
        assert_eq!(ids(&heads), vec!["u1", "a1", "b1"]);
    }

    #[test]
    fn candidate_heads_blocks_only_the_busy_agents_own_queue() {
        let queued = vec![agent_row("a2", Some("A"), 1.0, false), agent_row("b1", Some("B"), 2.0, false)];
        let running = vec![agent_row("a1", Some("A"), 0.0, false)];
        assert_eq!(ids(&candidate_heads(&queued, &running, &Default::default(), &Default::default())), vec!["b1"]);
    }

    #[test]
    fn candidate_heads_skips_paused_agents() {
        let queued = vec![agent_row("a1", Some("A"), 1.0, false), agent_row("u1", None, 2.0, false)];
        let paused = ["A".to_string()].into_iter().collect();
        assert_eq!(ids(&candidate_heads(&queued, &[], &paused, &Default::default())), vec!["u1"]);
    }

    #[test]
    fn candidate_heads_keeps_interactive_agent_cards_but_not_interactive_unassigned_ones() {
        let queued = vec![agent_row("chat", None, 1.0, true), agent_row("a1", Some("A"), 2.0, true), agent_row("u1", None, 3.0, false)];
        assert_eq!(ids(&candidate_heads(&queued, &[], &Default::default(), &Default::default())), vec!["a1", "u1"]);
    }

    #[test]
    fn candidate_heads_skips_a_card_waiting_for_prerequisites_and_offers_the_agents_next_one() {
        let queued = vec![
            agent_row("a1", Some("A"), 1.0, false),
            agent_row("a2", Some("A"), 2.0, false),
            agent_row("b1", Some("B"), 3.0, false),
        ];
        let blocked = ["a1".to_string(), "b1".to_string()].into_iter().collect();
        assert_eq!(ids(&candidate_heads(&queued, &[], &Default::default(), &blocked)), vec!["a2"]);
    }

    #[test]
    fn candidate_heads_orders_by_sort_order_even_if_input_is_not() {
        let queued = vec![agent_row("b1", Some("B"), 9.0, false), agent_row("a1", Some("A"), 2.0, false)];
        assert_eq!(ids(&candidate_heads(&queued, &[], &Default::default(), &Default::default())), vec!["a1", "b1"]);
    }

    #[test]
    fn picks_oldest_queued_when_a_slot_is_free() {
        let running: Vec<TaskRow> = vec![];
        let queued = vec![row("a", true, 1.0), row("b", true, 2.0)];
        assert_eq!(pick_next(&running, &queued, 2).map(|r| r.id.as_str()), Some("a"));
    }

    #[test]
    fn respects_the_global_cap() {
        let running = vec![row("x", true, 0.0), row("y", true, 0.0)];
        let queued = vec![row("a", true, 1.0)];
        assert!(pick_next(&running, &queued, 2).is_none());
    }

    #[test]
    fn a_solo_card_running_blocks_everything() {
        let running = vec![row("solo", false, 0.0)];
        let queued = vec![row("a", true, 1.0)];
        assert!(pick_next(&running, &queued, 4).is_none());
    }

    #[test]
    fn a_solo_card_at_the_head_waits_for_an_empty_running_set() {
        let running = vec![row("x", true, 0.0)];
        let queued = vec![row("solo", false, 1.0), row("b", true, 2.0)];
        // Strict priority: do NOT skip the solo card to run `b`.
        assert!(pick_next(&running, &queued, 4).is_none());
    }

    #[test]
    fn a_solo_card_at_the_head_runs_when_nothing_else_is() {
        let running: Vec<TaskRow> = vec![];
        let queued = vec![row("solo", false, 1.0)];
        assert_eq!(pick_next(&running, &queued, 4).map(|r| r.id.as_str()), Some("solo"));
    }

    #[test]
    fn empty_queue_picks_nothing() {
        let running: Vec<TaskRow> = vec![];
        assert!(pick_next(&running, &[], 4).is_none());
    }

    #[test]
    fn mark_done_sends_the_control_signal_and_returns_true_when_a_watch_is_registered() {
        let cancels = Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let handle = SchedulerHandle { wake: Arc::new(Notify::new()), cancels: cancels.clone() };
        let (tx, mut rx) = oneshot::channel::<monitor::WatchControl>();
        cancels.lock().insert("t1".to_string(), tx);
        assert!(handle.mark_done("t1"));
        assert!(matches!(rx.try_recv().unwrap(), monitor::WatchControl::MarkDone));
    }

    #[test]
    fn mark_done_returns_false_when_nothing_is_registered() {
        let handle = SchedulerHandle {
            wake: Arc::new(Notify::new()),
            cancels: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        };
        assert!(!handle.mark_done("nope"));
    }

    fn queued_row(id: &str, created_at: &str, parallel_ok: bool) -> TaskRow {
        TaskRow {
            id: id.to_string(),
            title: id.to_string(),
            body: String::new(),
            project_dir: "/r".to_string(),
            status: store::STATUS_QUEUED.to_string(),
            parallel_ok,
            interactive: false,
            sort_order: 1.0,
            outcome: None,
            tab_id: None,
            transcript_path: None,
            error_message: None,
            created_at: created_at.to_string(),
            dispatched_at: None,
            finished_at: None,
            ai_summary: None,
            archived_at: None,
            session_id: None,
            session_path: None,
            use_bridge: false,
            bridge_tiers: None,
            label: None,
            worktree_path: None,
            worktree_branch: None,
            isolate_worktree: None,
            agent_id: None,
            start_sha: None,
            end_sha: None,
            card_kind: "work".into(),
            module_id: None,
            review_of: None,
            verdict: None,
            review_reasons: None,
            module_override: false,
            is_spec: false,
            from_spec: None,
            quota_wait_at: None, depends_on: None, assigned_from: None, assigned_at: None, premerged: None, reply_wait_at: None, resume_session: false,
        }
    }

    #[test]
    fn cross_project_heads_are_tried_oldest_first() {
        // 兩個專案各有一張 head，B 比較早建立 → 先選 B
        let heads = vec![
            ("proj-a".to_string(), queued_row("a1", "2026-09-04T10:00:00Z", true)),
            ("proj-b".to_string(), queued_row("b1", "2026-09-04T09:00:00Z", true)),
        ];
        let ordered = order_heads(heads);
        assert_eq!(ordered[0].1.id, "b1");
        assert_eq!(ordered[1].1.id, "a1");
    }

    #[test]
    fn a_blocked_solo_head_lets_another_project_start() {
        // 已有東西在跑 → A 的獨佔 head 起不來，但 B 的一般 head 可以
        let running = vec![queued_row("r1", "2026-09-04T08:00:00Z", true)];
        let heads = order_heads(vec![
            ("proj-a".to_string(), queued_row("a1", "2026-09-04T09:00:00Z", false)),
            ("proj-b".to_string(), queued_row("b1", "2026-09-04T10:00:00Z", true)),
        ]);

        let mut chosen = None;
        for (project_id, head) in &heads {
            let slice = std::slice::from_ref(head);
            if pick_next(&running, slice, 5).is_some() {
                chosen = Some((project_id.clone(), head.id.clone()));
                break;
            }
        }
        assert_eq!(chosen, Some(("proj-b".to_string(), "b1".to_string())));
    }

    #[test]
    fn a_running_solo_card_blocks_every_project() {
        // D6：獨佔卡片執行中時，所有專案都不派新工作
        let running = vec![queued_row("r1", "2026-09-04T08:00:00Z", false)];
        let heads = order_heads(vec![
            ("proj-a".to_string(), queued_row("a1", "2026-09-04T09:00:00Z", true)),
            ("proj-b".to_string(), queued_row("b1", "2026-09-04T10:00:00Z", true)),
        ]);
        for (_, head) in &heads {
            assert!(pick_next(&running, std::slice::from_ref(head), 5).is_none());
        }
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use crate::tasks::store;

    /// 一個建在暫存目錄裡的單專案 registry。`TempDir` 一併回傳，
    /// 呼叫端必須持有它直到測試結束，否則資料夾會被提前刪掉。
    async fn one_project_registry() -> (ProjectRegistry, ProjectHandle, tempfile::TempDir) {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let handle = reg.create(parent.path(), "test", "").await.unwrap();
        (reg, handle, parent)
    }

    /// 讓一個專案的資料夾「消失」。
    ///
    /// 必須先關掉連線池再刪資料夾：Windows 不允許刪除還有檔案被開著的
    /// 目錄，而 `tasks.db` 正被這個池子開著——直接 `remove_dir_all` 在
    /// Windows CI 上會失敗（實際踩過）。關掉池子不影響這幾個測試要驗的
    /// 東西：`drain_once` 是先看資料夾在不在，根本還沒碰到池子。
    ///
    /// `pool.close().await` 回傳不保證 Windows 那邊的 OS 檔案控制代碼
    /// 當下就真的放開——CI 上還是踩過一次 `remove_dir_all` 噴
    /// `Os { code: 32 }`（檔案被另一個行程用著）。跟 monitor.rs 的
    /// `settle_exit_code` 同一種做法：重試而不是相信「呼叫完就代表生效」。
    async fn vanish(project: &ProjectHandle) {
        project.pool.close().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match std::fs::remove_dir_all(&project.path) {
                Ok(()) => return,
                Err(e) if tokio::time::Instant::now() < deadline => {
                    let _ = e;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => panic!("remove_dir_all({}) 一直失敗: {e}", project.path.display()),
            }
        }
    }

    /// 只記錄「誰被派工了」，不碰資料庫。資料夾已經被刪掉的專案沒辦法
    /// 事後查它的資料庫，所以這幾個測試改看派工紀錄。
    #[derive(Default)]
    struct RecordingDispatcher {
        dispatched: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingDispatcher {
        fn ids(&self) -> Vec<String> {
            self.dispatched.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Dispatcher for RecordingDispatcher {
        async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
            self.dispatched.lock().unwrap().push(task.id.clone());
            store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                .await
                .map_err(|e| e.to_string())?;
            store::finish_task(&project.pool, &task.id, "success", None, None)
                .await
                .map_err(|e| e.to_string())
        }
    }

    /// A `Dispatcher` that never really spawns a PTY — it immediately marks
    /// the card running with a fake tab id and finishes it `success`, so we
    /// can assert the loop's promotion behaviour.
    struct FakeDispatcher;
    #[async_trait::async_trait]
    impl Dispatcher for FakeDispatcher {
        async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
            store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                .await
                .map_err(|e| e.to_string())?;
            store::finish_task(&project.pool, &task.id, "success", None, None)
                .await
                .map_err(|e| e.to_string())
        }
    }

    /// 標成執行中但永遠不結束——模擬一張還在跑的長工作。
    struct NeverFinishes;
    #[async_trait::async_trait]
    impl Dispatcher for NeverFinishes {
        async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
            store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                .await
                .map_err(|e| e.to_string())
        }
    }

    async fn agent_in(project: &ProjectHandle, name: &str) -> String {
        crate::tasks::agents::create_agent(
            &project.pool,
            &crate::tasks::agents::NewAgent {
                kind: "coder", name, role: "r", prompt_prefix: "", project_dir: "/r",
                canvas_x: 0.0, canvas_y: 0.0,
            },
        )
        .await
        .unwrap()
    }

    async fn card_for(project: &ProjectHandle, title: &str, agent: &str, interactive: bool) -> String {
        let id = store::create_task(&project.pool, title, "", "/r", true, interactive).await.unwrap();
        crate::tasks::agents::assign_task(&project.pool, &id, Some(agent)).await.unwrap();
        id
    }

    fn titles(all: &[TaskRow], status: &str) -> Vec<String> {
        let mut v: Vec<String> = all.iter().filter(|r| r.status == status).map(|r| r.title.clone()).collect();
        v.sort();
        v
    }

    /// 你的例子：A/B 給 Coder1、D/E 給 Coder2，名額充足——兩個 Agent 各自只跑
    /// 自己佇列的第一張，第二張等自己的 Agent 有空。
    #[tokio::test]
    async fn drain_once_runs_one_card_per_agent_even_with_spare_capacity() {
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Coder1").await;
        let c2 = agent_in(&project, "Coder2").await;
        card_for(&project, "A", &c1, false).await;
        card_for(&project, "D", &c2, false).await;
        card_for(&project, "B", &c1, false).await;
        card_for(&project, "E", &c2, false).await;

        drain_once(&reg, &NeverFinishes, 5).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        assert_eq!(titles(&all, "running"), vec!["A", "D"]);
        assert_eq!(titles(&all, "queued"), vec!["B", "E"]);
    }

    #[tokio::test]
    async fn drain_once_lets_an_idle_coder_take_an_unstarted_spec_card() {
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Coder1").await;
        let c2 = agent_in(&project, "Coder2").await;
        for t in ["S1", "S2", "S3"] {
            let id = card_for(&project, t, &c1, false).await;
            sqlx::query("UPDATE tasks SET from_spec = 'spec' WHERE id = ?").bind(&id).execute(&project.pool).await.unwrap();
        }

        drain_once(&reg, &NeverFinishes, 5).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        let owner = |title: &str| all.iter().find(|t| t.title == title).unwrap().agent_id.clone().unwrap();
        assert_eq!(titles(&all, "running"), vec!["S1", "S2"], "Coder2 接走 S2 並同時開跑");
        assert_eq!(owner("S2"), c2);
        assert_eq!(owner("S3"), c1);
    }

    #[tokio::test]
    async fn drain_once_skips_a_card_waiting_for_another_coders_prerequisite() {
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Coder1").await;
        let c2 = agent_in(&project, "Coder2").await;
        let y = card_for(&project, "Y 前置", &c2, false).await;
        let x = card_for(&project, "X 整合", &c1, false).await;
        card_for(&project, "W 獨立", &c1, false).await;
        sqlx::query("UPDATE tasks SET depends_on = ? WHERE id = ?")
            .bind(serde_json::to_string(&[&y]).unwrap())
            .bind(&x)
            .execute(&project.pool)
            .await
            .unwrap();

        drain_once(&reg, &NeverFinishes, 5).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        // X 排在 Coder1 最前面，但 Coder2 的 Y 還沒做完 → 先做 W，X 留在佇列。
        assert_eq!(titles(&all, "running"), vec!["W 獨立", "Y 前置"]);
        assert_eq!(titles(&all, "queued"), vec!["X 整合"]);
    }

    #[tokio::test]
    async fn drain_once_finishes_an_agents_queue_in_order_when_cards_complete() {
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Coder1").await;
        card_for(&project, "A", &c1, false).await;
        card_for(&project, "B", &c1, false).await;
        let rec = RecordingDispatcher::default();
        drain_once(&reg, &rec, 5).await;
        let all = store::list_tasks(&project.pool).await.unwrap();
        assert_eq!(titles(&all, "done"), vec!["A", "B"]);
        let order: Vec<String> = rec
            .ids()
            .iter()
            .map(|id| all.iter().find(|r| &r.id == id).unwrap().title.clone())
            .collect();
        assert_eq!(order, vec!["A", "B"]);
    }

    #[tokio::test]
    async fn drain_once_skips_a_paused_agent_but_not_others() {
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Coder1").await;
        let c2 = agent_in(&project, "Coder2").await;
        card_for(&project, "A", &c1, false).await;
        card_for(&project, "D", &c2, false).await;
        crate::tasks::agents::set_paused(&project.pool, &c1, Some("x")).await.unwrap();

        drain_once(&reg, &NeverFinishes, 5).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        assert_eq!(titles(&all, "running"), vec!["D"]);
        assert_eq!(titles(&all, "queued"), vec!["A"]);
    }

    /// 額度用完回到佇列的 Codex 卡：冷卻 10 分鐘內這位 Agent 不派，其他 Agent 照常。
    #[tokio::test]
    async fn drain_once_lets_a_quota_waiting_codex_card_cool_down() {
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Codex").await;
        let c2 = agent_in(&project, "Claude").await;
        let a = card_for(&project, "A", &c1, false).await;
        card_for(&project, "B", &c1, false).await;
        card_for(&project, "D", &c2, false).await;
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE tasks SET quota_wait_at = ? WHERE id = ?").bind(now - 60).bind(&a).execute(&project.pool).await.unwrap();

        drain_once(&reg, &NeverFinishes, 5).await;
        let all = store::list_tasks(&project.pool).await.unwrap();
        assert_eq!(titles(&all, "running"), vec!["D"], "冷卻中的 Agent 連後面的 B 也不能先跑");

        sqlx::query("UPDATE tasks SET quota_wait_at = ? WHERE id = ?").bind(now - 601).bind(&a).execute(&project.pool).await.unwrap();
        drain_once(&reg, &NeverFinishes, 5).await;
        let all = store::list_tasks(&project.pool).await.unwrap();
        assert_eq!(titles(&all, "running"), vec!["A", "D"], "滿 10 分鐘就重試");
    }

    /// Agent 的互動卡片不能走「互動卡片無條件派工」那條捷徑，否則同一個
    /// Agent 會在同一個 worktree 裡同時跑兩個 claude。
    #[tokio::test]
    async fn drain_once_makes_an_interactive_agent_card_wait_for_its_agent() {
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Coder1").await;
        card_for(&project, "A", &c1, false).await;
        card_for(&project, "chat", &c1, true).await;

        drain_once(&reg, &NeverFinishes, 5).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        assert_eq!(titles(&all, "running"), vec!["A"]);
        assert_eq!(titles(&all, "queued"), vec!["chat"]);
    }

    /// 全域上限仍然有效：上限 1 時兩個 Agent 也只能有一個在跑。
    #[tokio::test]
    async fn drain_once_still_respects_the_global_cap_across_agents() {
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Coder1").await;
        let c2 = agent_in(&project, "Coder2").await;
        card_for(&project, "A", &c1, false).await;
        card_for(&project, "D", &c2, false).await;

        drain_once(&reg, &NeverFinishes, 1).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        assert_eq!(titles(&all, "running"), vec!["A"]);
    }

    /// Agent 的互動卡片在跑時要占全域名額——它背後一樣是一個 claude 行程。
    #[tokio::test]
    async fn a_running_interactive_agent_card_counts_against_the_cap() {
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Coder1").await;
        let c2 = agent_in(&project, "Coder2").await;
        card_for(&project, "chat", &c1, true).await;
        card_for(&project, "D", &c2, false).await;
        let chat = store::list_tasks(&project.pool).await.unwrap().into_iter().find(|r| r.title == "chat").unwrap();
        // 讓 chat 排在前面先跑起來
        sqlx::query("UPDATE tasks SET sort_order = 0 WHERE id = ?").bind(&chat.id).execute(&project.pool).await.unwrap();
        store::dispatch_for_test(&project.pool, &chat.id, "tab-chat").await;

        drain_once(&reg, &NeverFinishes, 1).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        assert_eq!(titles(&all, "running"), vec!["chat"]);
        assert_eq!(titles(&all, "queued"), vec!["D"]);
    }

    /// 派工本身失敗（例如 Agent 的 worktree 建不起來）也要暫停 Agent。
    #[tokio::test]
    async fn a_failed_agent_dispatch_pauses_the_agent() {
        struct Fails;
        #[async_trait::async_trait]
        impl Dispatcher for Fails {
            async fn dispatch(&self, _p: &ProjectHandle, _t: &TaskRow) -> Result<(), String> {
                Err("worktree boom".into())
            }
        }
        let (reg, project, _parent) = one_project_registry().await;
        let c1 = agent_in(&project, "Coder1").await;
        card_for(&project, "A", &c1, false).await;
        card_for(&project, "B", &c1, false).await;

        drain_once(&reg, &Fails, 5).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        let a = all.iter().find(|r| r.title == "A").unwrap();
        assert_eq!(a.outcome.as_deref(), Some("failed"));
        assert_eq!(titles(&all, "queued"), vec!["B"], "暫停後 B 不能被接著派出去");
        let agent = crate::tasks::agents::get_agent(&project.pool, &c1).await.unwrap().unwrap();
        assert_eq!(agent.paused_task_id.as_deref(), Some(a.id.as_str()));
    }

    #[tokio::test]
    async fn drain_once_promotes_the_whole_queue_when_the_fake_dispatcher_finishes_instantly() {
        let (reg, project, _parent) = one_project_registry().await;
        for (t, par, ord) in [("a", true, 1.0), ("b", true, 2.0), ("solo", false, 3.0), ("c", true, 4.0)] {
            let id = store::create_task(&project.pool, t, "", "/r", par, false).await.unwrap();
            store::move_task(&project.pool, &id, store::STATUS_QUEUED, ord).await.unwrap();
        }
        drain_once(&reg, &FakeDispatcher, 2).await;
        let all = store::list_tasks(&project.pool).await.unwrap();
        assert!(all.iter().all(|r| r.status == "done"), "{all:#?}");
    }

    #[tokio::test]
    async fn drain_once_stops_at_a_solo_head_while_something_runs() {
        let (reg, project, _parent) = one_project_registry().await;
        // A dispatcher that marks running but never finishes (simulates a
        // long job), so the running set stays non-empty.
        struct StuckDispatcher;
        #[async_trait::async_trait]
        impl Dispatcher for StuckDispatcher {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                    .await.map_err(|e| e.to_string())
            }
        }
        for (t, par, ord) in [("p1", true, 1.0), ("solo", false, 2.0), ("p2", true, 3.0)] {
            let id = store::create_task(&project.pool, t, "", "/r", par, false).await.unwrap();
            store::move_task(&project.pool, &id, store::STATUS_QUEUED, ord).await.unwrap();
        }
        drain_once(&reg, &StuckDispatcher, 5).await;
        // `create_task` assigns a UUID id; match on the human title instead.
        let by = |s: &str, all: &[TaskRow]| all.iter().filter(|r| r.status == s).map(|r| r.title.clone()).collect::<Vec<_>>();
        let all = store::list_tasks(&project.pool).await.unwrap();
        // p1 promoted; solo is the head and something's running → stop; p2 not skipped.
        assert_eq!(by("running", &all), vec!["p1"]);
        assert_eq!(by("queued", &all), vec!["solo", "p2"]);
    }

    #[tokio::test]
    async fn drain_once_dispatches_interactive_cards_even_when_the_auto_lane_is_full() {
        let (reg, project, _parent) = one_project_registry().await;
        // Seed one auto card already running — fills a cap of 1.
        let running_id = store::create_task(&project.pool, "already-running", "", "/r", true, false).await.unwrap();
        store::move_task(&project.pool, &running_id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&project.pool, &running_id, "tab-already").await;

        // A second auto card, queued — cap=1 means this must stay queued.
        let blocked_id = store::create_task(&project.pool, "blocked-auto", "", "/r", true, false).await.unwrap();
        store::move_task(&project.pool, &blocked_id, store::STATUS_QUEUED, 2.0).await.unwrap();

        // An interactive card, queued — must dispatch anyway, cap or no cap.
        let interactive_id = store::create_task(&project.pool, "chat", "", "/r", true, true).await.unwrap();
        store::move_task(&project.pool, &interactive_id, store::STATUS_QUEUED, 3.0).await.unwrap();

        struct RecordingDispatcher {
            dispatched: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl Dispatcher for RecordingDispatcher {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                self.dispatched.lock().unwrap().push(task.title.clone());
                store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                    .await
                    .map_err(|e| e.to_string())
            }
        }
        let dispatcher = RecordingDispatcher { dispatched: std::sync::Mutex::new(vec![]) };
        drain_once(&reg, &dispatcher, 1).await;

        assert_eq!(*dispatcher.dispatched.lock().unwrap(), vec!["chat".to_string()]);

        let all = store::list_tasks(&project.pool).await.unwrap();
        let status_of = |t: &str| all.iter().find(|r| r.title == t).unwrap().status.clone();
        assert_eq!(status_of("already-running"), "running");
        assert_eq!(status_of("blocked-auto"), "queued"); // cap=1 respected for the auto lane
        assert_eq!(status_of("chat"), "running"); // interactive bypassed the cap entirely
    }

    /// 好幾張卡可以同時開工時，派工要同時進行：每張都要等 claude 啟動 15–20 秒，
    /// 一張等一張的話三位工程師要一分鐘才全部開工。同一位 Agent 仍然一次一張、上限照算。
    #[tokio::test]
    async fn cards_that_can_start_together_are_dispatched_concurrently() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let project = reg.create(parent.path(), "p", "").await.unwrap();
        let mut coders = Vec::new();
        for name in ["C1", "C2", "C3"] {
            coders.push(crate::tasks::agents::create_agent(&project.pool, &crate::tasks::agents::NewAgent {
                kind: "coder", name, role: "r", prompt_prefix: "", project_dir: "/r", canvas_x: 0.0, canvas_y: 0.0,
            }).await.unwrap());
        }
        for (i, c) in coders.iter().enumerate() {
            for j in 0..2 {
                let id = store::create_task(&project.pool, &format!("{i}-{j}"), "", "/r", true, false).await.unwrap();
                crate::tasks::agents::assign_task(&project.pool, &id, Some(c)).await.unwrap();
            }
        }

        struct Slow {
            in_flight: std::sync::atomic::AtomicUsize,
            peak: std::sync::atomic::AtomicUsize,
            started: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl Dispatcher for Slow {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                use std::sync::atomic::Ordering::SeqCst;
                let now = self.in_flight.fetch_add(1, SeqCst) + 1;
                self.peak.fetch_max(now, SeqCst);
                self.started.lock().unwrap().push(task.title.clone());
                tokio::time::sleep(Duration::from_millis(200)).await;
                self.in_flight.fetch_sub(1, SeqCst);
                store::set_tab_id(&project.pool, &task.id, "tab").await.map_err(|e| e.to_string())
            }
        }
        let d = Slow { in_flight: 0.into(), peak: 0.into(), started: std::sync::Mutex::new(vec![]) };
        drain_once(&reg, &d, 5).await;

        let mut started = d.started.lock().unwrap().clone();
        started.sort();
        assert_eq!(started, vec!["0-0", "1-0", "2-0"], "每位工程師一次一張");
        assert_eq!(d.peak.load(std::sync::atomic::Ordering::SeqCst), 3, "三張要同時派");

        // 上限照算：上限 1 時只派一張。
        let reg2 = ProjectRegistry::new();
        let p2 = reg2.create(parent.path(), "p2", "").await.unwrap();
        for name in ["D1", "D2"] {
            let c = crate::tasks::agents::create_agent(&p2.pool, &crate::tasks::agents::NewAgent {
                kind: "coder", name, role: "r", prompt_prefix: "", project_dir: "/r", canvas_x: 0.0, canvas_y: 0.0,
            }).await.unwrap();
            let id = store::create_task(&p2.pool, name, "", "/r", true, false).await.unwrap();
            crate::tasks::agents::assign_task(&p2.pool, &id, Some(&c)).await.unwrap();
        }
        let d2 = Slow { in_flight: 0.into(), peak: 0.into(), started: std::sync::Mutex::new(vec![]) };
        drain_once(&reg2, &d2, 1).await;
        assert_eq!(d2.started.lock().unwrap().len(), 1);
    }

    /// 專案用到 token 上限：Agent 卡不再派工，看板一般卡片照常。
    #[tokio::test]
    async fn over_its_token_budget_a_project_stops_dispatching_agent_cards() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let project = reg.create(parent.path(), "p", "").await.unwrap();
        let coder = crate::tasks::agents::create_agent(&project.pool, &crate::tasks::agents::NewAgent {
            kind: "coder", name: "C", role: "r", prompt_prefix: "", project_dir: "/r", canvas_x: 0.0, canvas_y: 0.0,
        }).await.unwrap();
        // 之前跑過的卡用了 1000 token。
        let old = store::create_task(&project.pool, "old", "", "/r", true, false).await.unwrap();
        sqlx::query("UPDATE tasks SET status = 'done', outcome = 'success', dispatched_at = 1 WHERE id = ?").bind(&old).execute(&project.pool).await.unwrap();
        crate::tasks::usage::save(&project.path, &old, &crate::tasks::usage::TokenUsage { input: 1000, ..Default::default() });
        let agent_card = store::create_task(&project.pool, "agent-card", "", "/r", true, false).await.unwrap();
        crate::tasks::agents::assign_task(&project.pool, &agent_card, Some(&coder)).await.unwrap();
        let board_card = store::create_task(&project.pool, "board-card", "", "/r", true, false).await.unwrap();
        store::move_task(&project.pool, &board_card, store::STATUS_QUEUED, 99.0).await.unwrap();

        struct Rec(std::sync::Mutex<Vec<String>>);
        #[async_trait::async_trait]
        impl Dispatcher for Rec {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                self.0.lock().unwrap().push(task.title.clone());
                store::set_tab_id(&project.pool, &task.id, "tab").await.map_err(|e| e.to_string())
            }
        }
        crate::tasks::budget::set_budget(&project.pool, Some(1000)).await.unwrap();
        let rec = Rec(std::sync::Mutex::new(vec![]));
        drain_once(&reg, &rec, 5).await;
        assert_eq!(*rec.0.lock().unwrap(), vec!["board-card".to_string()], "超過上限的 Agent 卡先不派");

        crate::tasks::budget::set_budget(&project.pool, Some(5000)).await.unwrap();
        drain_once(&reg, &rec, 5).await;
        assert_eq!(*rec.0.lock().unwrap(), vec!["board-card".to_string(), "agent-card".to_string()], "調高上限就恢復");
    }

    /// `tests::a_blocked_solo_head_lets_another_project_start` only exercises
    /// `pick_next` + `order_heads` through a loop written inside the test, so
    /// it cannot catch a regression in `drain_once` itself. This one drives the
    /// real thing: alpha's head is a solo card that cannot start (something is
    /// already running), and beta's head must still go out.
    #[tokio::test]
    async fn drain_once_skips_a_blocked_solo_head_and_starts_another_projects_card() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let alpha = reg.create(parent.path(), "alpha", "").await.unwrap();
        let beta = reg.create(parent.path(), "beta", "").await.unwrap();

        // Something already running, so a solo head is blocked.
        let busy = store::create_task(&alpha.pool, "busy", "", "/r", true, false).await.unwrap();
        store::move_task(&alpha.pool, &busy, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&alpha.pool, &busy, "tab-busy").await;

        let solo = store::create_task(&alpha.pool, "solo", "", "/r", false, false).await.unwrap();
        store::move_task(&alpha.pool, &solo, store::STATUS_QUEUED, 2.0).await.unwrap();
        let other = store::create_task(&beta.pool, "other", "", "/r", true, false).await.unwrap();
        store::move_task(&beta.pool, &other, store::STATUS_QUEUED, 1.0).await.unwrap();

        // `created_at` has one-second resolution, so cards made in the same
        // test would tie and fall back to a random-UUID tiebreak. Pin them so
        // alpha's blocked head is definitely tried first.
        set_created_at(&alpha.pool, &solo, "2026-09-04T09:00:00Z").await;
        set_created_at(&beta.pool, &other, "2026-09-04T10:00:00Z").await;

        struct StuckDispatcher;
        #[async_trait::async_trait]
        impl Dispatcher for StuckDispatcher {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                    .await
                    .map_err(|e| e.to_string())
            }
        }
        drain_once(&reg, &StuckDispatcher, 5).await;

        assert_eq!(
            store::get_task(&alpha.pool, &solo).await.unwrap().unwrap().status,
            "queued",
            "獨佔卡片在有東西執行中時不可啟動"
        );
        assert_eq!(
            store::get_task(&beta.pool, &other).await.unwrap().unwrap().status,
            "running",
            "別的專案的隊首不該被 alpha 的獨佔卡片擋住"
        );
    }

    async fn set_created_at(pool: &sqlx::SqlitePool, id: &str, created_at: &str) {
        sqlx::query("UPDATE tasks SET created_at = ?1 WHERE id = ?2")
            .bind(created_at)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn drain_once_does_not_let_a_running_interactive_card_count_against_the_auto_cap() {
        let (reg, project, _parent) = one_project_registry().await;
        // An interactive card already running should NOT occupy the auto
        // lane's capacity slot — an auto card queued behind it must still
        // start immediately even at cap=1.
        let chat_id = store::create_task(&project.pool, "chat", "", "/r", true, true).await.unwrap();
        store::move_task(&project.pool, &chat_id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&project.pool, &chat_id, "tab-chat").await;

        let auto_id = store::create_task(&project.pool, "auto", "", "/r", true, false).await.unwrap();
        store::move_task(&project.pool, &auto_id, store::STATUS_QUEUED, 2.0).await.unwrap();

        struct FakeDispatcher;
        #[async_trait::async_trait]
        impl Dispatcher for FakeDispatcher {
            async fn dispatch(&self, project: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                store::set_tab_id(&project.pool, &task.id, &format!("fake-{}", task.id))
                    .await
                    .map_err(|e| e.to_string())
            }
        }
        drain_once(&reg, &FakeDispatcher, 1).await;

        let all = store::list_tasks(&project.pool).await.unwrap();
        let status_of = |t: &str| all.iter().find(|r| r.title == t).unwrap().status.clone();
        assert_eq!(status_of("chat"), "running");
        assert_eq!(status_of("auto"), "running"); // not blocked by the already-running interactive card
    }

    /// 派工要等 `claude` 的 TUI 起來，最久 30 秒。如果卡片在那之前還留在
    /// `queued`，第二次掃描就會再派一次同一張——實機出現過同一張卡開出
    /// 兩個 claude 行程（同一個 App、同一個工作目錄、相隔 32 秒，而卡片的
    /// `dispatched_at` 從頭到尾是空的，證明 spawn 還沒回來）。
    ///
    /// 這裡用一個「永遠不回來」的 dispatcher 模擬那 30 秒，然後再掃一次：
    /// 同一張卡不可以被派第二次。
    #[tokio::test]
    async fn a_card_being_dispatched_is_not_dispatched_again_by_the_next_sweep() {
        let (reg, project, _parent) = one_project_registry().await;
        let id = store::create_task(&project.pool, "slow", "", "/r", true, false).await.unwrap();
        store::move_task(&project.pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();

        /// 只記錄、完全不碰資料庫——模擬 spawn_and_run 還卡在等 TUI。
        #[derive(Default)]
        struct NeverFinishes {
            seen: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl Dispatcher for NeverFinishes {
            async fn dispatch(&self, _p: &ProjectHandle, task: &TaskRow) -> Result<(), String> {
                self.seen.lock().unwrap().push(task.id.clone());
                Ok(())
            }
        }

        // 包 timeout 而不是直接 await：這個 bug 退化回去的話，drain_once 的
        // 一般通道會**無限迴圈**（派工成功 → 重掃 → 卡片還是 queued → 再派），
        // 直接 await 會讓 CI 卡到整場逾時才死，看不出是誰壞的。
        let d = NeverFinishes::default();
        for _ in 0..2 {
            tokio::time::timeout(std::time::Duration::from_secs(5), drain_once(&reg, &d, 5))
                .await
                .expect("drain_once 沒有結束——卡片派工後仍留在 queued，掃描迴圈停不下來");
        }

        assert_eq!(
            d.seen.lock().unwrap().len(),
            1,
            "同一張卡被派了兩次——第二次掃描時它還留在 queued"
        );
    }

    /// 資料夾被 Finder 刪掉之後，registry 裡的 handle 不一定已經被驅逐
    /// （驅逐發生在 `projects_list`，而那只有看板開著時才會跑）。排程器
    /// 必須自己擋下來：在 Unix 上，已開啟的 SQLite 檔案被 unlink 之後
    /// inode 還在，查詢會照常「成功」——於是會派工給一個使用者已經刪掉
    /// 的專案，寫入還進到一個誰也看不到的檔案。
    #[tokio::test]
    async fn drain_once_skips_a_project_whose_folder_is_gone() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let gone = reg.create(parent.path(), "gone", "").await.unwrap();
        let alive = reg.create(parent.path(), "alive", "").await.unwrap();

        let ghost = store::create_task(&gone.pool, "ghost", "", "/r", true, false).await.unwrap();
        store::move_task(&gone.pool, &ghost, store::STATUS_QUEUED, 1.0).await.unwrap();
        let real = store::create_task(&alive.pool, "real", "", "/r", true, false).await.unwrap();
        store::move_task(&alive.pool, &real, store::STATUS_QUEUED, 1.0).await.unwrap();

        vanish(&gone).await;
        let dispatcher = RecordingDispatcher::default();
        drain_once(&reg, &dispatcher, 5).await;

        // 用派工紀錄判斷，不回頭查 gone 的資料庫——資料夾都刪了，那個
        // 連線池也已經關閉（Windows 上不關就刪不掉），查不動。
        let dispatched = dispatcher.ids();
        assert!(!dispatched.contains(&ghost), "資料夾已經不在的專案不該被派工");
        assert!(dispatched.contains(&real), "其他專案必須照常運作");
    }

    /// 互動卡片走的是另一條完全不受並行上限管制的通道，同樣不該從已消失
    /// 的專案派工。
    ///
    /// **這是行為描述，不是迴歸偵測器**——實際做過突變驗證：把上面
    /// `drain_once` 開頭的資料夾存在檢查整個拿掉，這個測試照樣是綠的。
    /// 因為互動通道的錯誤處理本來就是每個專案各自 `break`，一個讀不到的
    /// 專案只會讓它自己那圈結束，不會波及別人；真正會因為缺少檢查而全面
    /// 停擺的是下面那條一般通道（`return`），那個由
    /// `drain_once_skips_a_project_whose_folder_is_gone` 守著，而它會紅。
    /// 留著這個測試是為了釘住「互動通道也不會派工給消失的專案」這件事，
    /// 別誤以為它在保護那個過濾器。
    #[tokio::test]
    async fn drain_once_skips_a_vanished_project_in_the_interactive_lane_too() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let gone = reg.create(parent.path(), "gone", "").await.unwrap();

        let ghost = store::create_task(&gone.pool, "ghost", "", "/r", true, true).await.unwrap();
        store::move_task(&gone.pool, &ghost, store::STATUS_QUEUED, 1.0).await.unwrap();

        vanish(&gone).await;
        let dispatcher = RecordingDispatcher::default();
        drain_once(&reg, &dispatcher, 5).await;

        assert!(
            !dispatcher.ids().contains(&ghost),
            "互動卡片也不該從已消失的專案派出去"
        );
    }
}

/// `persist_outcome` 從 watch closure 抽出來就是為了讓這裡測得到——不需要
/// `AppHandle`、不需要 `PtyManager`，只需要一個真的 `SqlitePool` 與一棵
/// 假的 `~/.claude/projects` 樹。
#[cfg(test)]
mod persist_tests {
    use super::*;

    /// 獨立的記憶體 pool，帶完整 schema——比照 `store.rs` 測試模組裡
    /// `mem_pool` 的寫法（那邊是私有的，這裡自己開一份）。
    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    /// 建一棵假的 `~/.claude/projects` 樹：`<root>/<任意子資料夾>/<sid>.jsonl`。
    /// 子資料夾名字不重要——`copy_session_log` 用掃描找 session_id，不看
    /// 資料夾叫什麼名字。
    fn fake_projects_root(session_id: &str, body: &str) -> tempfile::TempDir {
        let projects = tempfile::tempdir().unwrap();
        let dir = projects.path().join("some-project");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{session_id}.jsonl")), body).unwrap();
        projects
    }

    async fn queued_and_claimed(pool: &sqlx::SqlitePool) -> String {
        let id = store::create_task(pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        assert!(store::claim_for_dispatch(pool, &id).await.unwrap());
        id
    }

    /// 驗證整條路徑真的會發生：找到 session 檔、複製進卡片資料夾、把路徑
    /// 寫回 DB，跟結案欄位一起持久化。
    #[tokio::test]
    async fn happy_path_copies_the_session_log_and_records_its_path() {
        let pool = mem_pool().await;
        let task_id = queued_and_claimed(&pool).await;

        let project = tempfile::tempdir().unwrap();
        let projects_root = fake_projects_root("SID", "{\"type\":\"user\"}\n");

        persist_outcome(
            &pool,
            Some(projects_root.path()),
            Some("SID"),
            project.path(),
            &task_id,
            Some("raw transcript"),
            &monitor::TaskOutcome::Success,
        )
        .await;

        let row = store::get_task(&pool, &task_id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("success"));
        let session_path = row.session_path.expect("session_path 沒被寫入");
        // 存的是相對於專案資料夾的路徑，不是絕對路徑——專案搬走才不會斷。
        assert_eq!(session_path, format!("tasks/{task_id}/session.jsonl"));
        let resolved = crate::tasks::resolve_stored_path(project.path(), &session_path);
        assert_eq!(
            resolved.parent().unwrap(),
            crate::tasks::task_dir(project.path(), &task_id),
            "沒有存進卡片資料夾"
        );
        assert_eq!(std::fs::read_to_string(&resolved).unwrap(), "{\"type\":\"user\"}\n");
    }

    /// 沒有 session id（`claude_command` 不是 claude，或使用者沒設定）：
    /// 不複製，但卡片照樣結案，帶著原本的 transcript／錯誤訊息。
    #[tokio::test]
    async fn no_session_id_skips_the_copy_but_still_finishes() {
        let pool = mem_pool().await;
        let task_id = queued_and_claimed(&pool).await;
        let projects_root = tempfile::tempdir().unwrap(); // 有根，但沒有 session id 就不會被碰
        let project = tempfile::tempdir().unwrap();

        persist_outcome(
            &pool,
            Some(projects_root.path()),
            None,
            project.path(),
            &task_id,
            Some("raw transcript"),
            &monitor::TaskOutcome::Failed("boom".to_string()),
        )
        .await;

        let row = store::get_task(&pool, &task_id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("failed"));
        assert_eq!(row.error_message.as_deref(), Some("boom"));
        assert_eq!(row.transcript_path.as_deref(), Some("raw transcript"));
        assert!(row.session_path.is_none(), "沒有 session id 卻還是複製了");
    }

    /// 只能觀察最終狀態：`session_path` 與結案欄位都必須被寫入。**這條不
    /// 證明複製先於結案發生**——那個順序是靠 `persist_outcome` 裡的敘述
    /// 順序保證的，不是這個測試。如果有人把 `finish_task` 搬到複製區塊
    /// 之前，這條測試不會注意到（兩個寫入的最終狀態不變，順序本身沒有
    /// 可觀察的副作用）。
    #[tokio::test]
    async fn both_the_session_path_and_the_finish_fields_end_up_persisted() {
        let pool = mem_pool().await;
        let task_id = queued_and_claimed(&pool).await;

        let project = tempfile::tempdir().unwrap();
        let projects_root = fake_projects_root("SID2", "{}\n");

        persist_outcome(
            &pool,
            Some(projects_root.path()),
            Some("SID2"),
            project.path(),
            &task_id,
            None,
            &monitor::TaskOutcome::Cancelled,
        )
        .await;

        let row = store::get_task(&pool, &task_id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("cancelled"));
        assert!(row.session_path.is_some(), "session_path 沒被寫入");
    }

    /// 一張指派給 Agent、正在執行的卡片。
    async fn running_agent_card(pool: &sqlx::SqlitePool, agent: &str, interactive: bool, sid: Option<&str>, order: f64) -> String {
        let id = store::create_task(pool, "t", "", "/r", true, interactive).await.unwrap();
        sqlx::query("UPDATE tasks SET agent_id = ?, status = 'running', session_id = ?, sort_order = ?, reply_wait_at = 5, quota_wait_at = 6 WHERE id = ?")
            .bind(agent).bind(sid).bind(order).bind(&id).execute(pool).await.unwrap();
        id
    }

    async fn agent_named(pool: &sqlx::SqlitePool, name: &str) -> String {
        crate::tasks::agents::create_agent(pool, &crate::tasks::agents::NewAgent {
            kind: "coder", name, role: "r", prompt_prefix: "", project_dir: "/r", canvas_x: 0.0, canvas_y: 0.0,
        }).await.unwrap()
    }

    /// Codex 卡片結束碼非 0：從 rollout 判斷。額度用完 → 回佇列（接著做、記等待時間、排在這位 Agent 最前面）；
    /// 其他錯誤 → 失敗並換成 Codex 的訊息；讀不到記錄 → 照原結果。
    #[tokio::test]
    async fn a_codex_card_that_hit_its_usage_limit_goes_back_to_the_queue() {
        let pool = mem_pool().await;
        let a = agent_named(&pool, "Codex").await;
        let queued = store::create_task(&pool, "next", "", "/r", true, false).await.unwrap();
        crate::tasks::agents::assign_task(&pool, &queued, Some(&a)).await.unwrap();
        store::move_task(&pool, &queued, store::STATUS_QUEUED, 5.0).await.unwrap();
        let id = running_agent_card(&pool, &a, false, Some("01a1"), 9.0).await;
        let dir = tempfile::tempdir().unwrap();
        let rollout = dir.path().join("rollout-2026-01a1.jsonl");
        std::fs::write(&rollout, r#"{"type":"event_msg","payload":{"type":"task_complete","error":{"message":"You've hit your usage limit.","codex_error_info":"usage_limit_exceeded"}}}"#).unwrap();
        store::set_session_path(&pool, &id, rollout.to_str().unwrap()).await.unwrap();

        let ok = codex_verdict(&pool, &id, monitor::TaskOutcome::Success).await;
        assert!(matches!(ok, CodexVerdict::Outcome(monitor::TaskOutcome::Success)), "成功就照走");
        let v = codex_verdict(&pool, &id, monitor::TaskOutcome::Failed("exit 1".into())).await;
        assert!(matches!(v, CodexVerdict::Requeue));
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.resume_session, row.tab_id.as_deref()), ("queued", true, None));
        assert!(row.quota_wait_at.is_some(), "畫面要看得到在等額度");
        assert!(row.sort_order < 5.0, "排在這位 Agent 原本的下一張前面");

        std::fs::write(&rollout, r#"{"type":"event_msg","payload":{"type":"task_complete","error":{"message":"Model 'x' not found"}}}"#).unwrap();
        match codex_verdict(&pool, &id, monitor::TaskOutcome::Failed("exit 1".into())).await {
            CodexVerdict::Outcome(monitor::TaskOutcome::Failed(m)) => assert_eq!(m, "Model 'x' not found"),
            _ => panic!("其他錯誤要失敗並帶 Codex 的訊息"),
        }
        std::fs::remove_file(&rollout).unwrap();
        match codex_verdict(&pool, &id, monitor::TaskOutcome::Failed("exit 1".into())).await {
            CodexVerdict::Outcome(monitor::TaskOutcome::Failed(m)) => assert_eq!(m, "exit 1"),
            _ => panic!("讀不到記錄照原結果"),
        }
    }

    #[test]
    fn quota_cooling_lasts_the_retry_interval() {
        assert!(!quota_cooling(None, 1000));
        assert!(quota_cooling(Some(1000), 1000 + 599));
        assert!(!quota_cooling(Some(1000), 1000 + 600));
    }

    /// App 重開：Codex 卡片看 SessionStart 記下的 rollout 在不在決定接著做；沒答的問題作廢。
    #[tokio::test]
    async fn after_a_restart_codex_cards_resume_from_their_rollout() {
        let pool = mem_pool().await;
        let a = agent_named(&pool, "Codex").await;
        crate::tasks::agents::set_cli(&pool, &a, "codex").await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let rollout = dir.path().join("rollout-x-s1.jsonl");
        std::fs::write(&rollout, "{}\n").unwrap();
        let live = running_agent_card(&pool, &a, false, Some("s1"), 1.0).await;
        store::set_session_path(&pool, &live, rollout.to_str().unwrap()).await.unwrap();
        crate::tasks::questions::create(&pool, &live, Some(&a), &crate::tasks::questions::NewQuestion {
            question: "Q?".into(), options: vec![], recommended: None, context: None,
        }).await.unwrap();
        let gone = running_agent_card(&pool, &a, false, Some("s2"), 2.0).await;
        store::set_session_path(&pool, &gone, dir.path().join("rollout-missing.jsonl").to_str().unwrap()).await.unwrap();

        // projects_root 給 None：Codex 卡不靠 Claude 的記錄資料夾。
        let out = recover_after_restart(&pool, None).await.unwrap();
        assert_eq!((out.resumed, out.restarted), (1, 1));
        assert!(store::get_task(&pool, &live).await.unwrap().unwrap().resume_session);
        assert!(!store::get_task(&pool, &gone).await.unwrap().unwrap().resume_session);
        let status: String = sqlx::query_scalar("SELECT status FROM questions WHERE task_id = ?").bind(&live).fetch_one(&pool).await.unwrap();
        assert_eq!(status, crate::tasks::questions::CANCELLED, "沒人在等的問題要作廢");
    }

    #[tokio::test]
    async fn after_a_restart_agent_cards_resume_or_start_over_instead_of_being_cancelled() {
        let pool = mem_pool().await;
        let root = fake_projects_root("SID-LIVE", "{}\n");
        let a1 = agent_named(&pool, "A1").await;
        let a2 = agent_named(&pool, "A2").await;
        let a3 = agent_named(&pool, "A3").await;
        // A1：有對話記錄 → 接著做，排在它自己佇列的最前面。
        let waiting = store::create_task(&pool, "下一張", "", "/r", true, false).await.unwrap();
        sqlx::query("UPDATE tasks SET agent_id = ?, status = 'queued', sort_order = 10 WHERE id = ?").bind(&a1).bind(&waiting).execute(&pool).await.unwrap();
        let live = running_agent_card(&pool, &a1, false, Some("SID-LIVE"), 50.0).await;
        // A2：claude 沒啟動起來（沒有對話記錄）→ 從頭再來。
        let never = running_agent_card(&pool, &a2, false, Some("SID-NONE"), 1.0).await;
        // A3：互動卡 → 照舊中斷並暫停 Agent。
        let inter = running_agent_card(&pool, &a3, true, Some("SID-LIVE"), 1.0).await;
        // 看板一般卡片 → 照舊中斷。
        let board = store::create_task(&pool, "看板", "", "/r", true, false).await.unwrap();
        sqlx::query("UPDATE tasks SET status = 'running' WHERE id = ?").bind(&board).execute(&pool).await.unwrap();

        let out = recover_after_restart(&pool, Some(root.path())).await.unwrap();
        assert_eq!(out, RecoverSummary { resumed: 1, restarted: 1, cancelled: 2 });

        let get = |id: String| { let pool = pool.clone(); async move { store::get_task(&pool, &id).await.unwrap().unwrap() } };
        let l = get(live.clone()).await;
        assert_eq!((l.status.as_str(), l.resume_session, l.reply_wait_at, l.quota_wait_at), ("queued", true, None, None));
        assert!(l.sort_order < get(waiting).await.sort_order, "要排在同一位 Agent 原本的下一張之前");
        let n = get(never).await;
        assert_eq!((n.status.as_str(), n.resume_session), ("queued", false));
        assert_eq!(get(inter.clone()).await.outcome.as_deref(), Some("cancelled"));
        assert_eq!(get(board).await.outcome.as_deref(), Some("cancelled"));
        let paused = |id: String| { let pool = pool.clone(); async move { crate::tasks::agents::get_agent(&pool, &id).await.unwrap().unwrap().paused_task_id } };
        assert_eq!(paused(a1).await, None, "接著做的 Agent 不暫停");
        assert_eq!(paused(a2).await, None);
        assert_eq!(paused(a3).await.as_deref(), Some(inter.as_str()));
    }

    #[tokio::test]
    async fn the_token_usage_is_saved_with_the_card_when_it_finishes() {
        let pool = mem_pool().await;
        let task_id = queued_and_claimed(&pool).await;
        let project = tempfile::tempdir().unwrap();
        let usage_line = r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":3,"cache_creation_input_tokens":0,"cache_read_input_tokens":40,"output_tokens":5}}}"#;
        let projects_root = fake_projects_root("SID3", usage_line);

        persist_outcome(
            &pool,
            Some(projects_root.path()),
            Some("SID3"),
            project.path(),
            &task_id,
            None,
            &monitor::TaskOutcome::Cancelled,
        )
        .await;

        let saved = crate::tasks::usage::load(project.path(), &task_id).expect("usage.json 沒被寫入");
        assert_eq!((saved.input, saved.cache_read, saved.output, saved.calls), (3, 40, 5, 1));
    }
}

#[cfg(test)]
mod prepare_worktree_tests {
    use super::*;
    use std::fs;
    use std::process::Command as StdCommand;

    fn init_repo(dir: &std::path::Path) {
        StdCommand::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.email", "test@test.com"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.name", "Test"]).current_dir(dir).status().unwrap();
        fs::write(dir.join("a.txt"), "hello\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir).status().unwrap();
    }

    #[test]
    fn card_override_beats_the_global_setting_in_both_directions() {
        // 沒有覆寫：兩個方向都要跟著全域走。
        assert!(resolve_isolation(None, true), "沒覆寫時應該沿用全域的 true");
        assert!(!resolve_isolation(None, false), "沒覆寫時應該沿用全域的 false");

        // 有覆寫：兩個方向都要勝過全域。只測一個方向的話，把實作寫成
        // 「全域關就一律關」也會通過。
        assert!(resolve_isolation(Some(true), false), "卡片說要隔離就該隔離，即使全域關著");
        assert!(!resolve_isolation(Some(false), true), "卡片說不要隔離就該不隔離，即使全域開著");
    }

    #[tokio::test]
    async fn isolation_turned_off_runs_directly_in_the_project_dir() {
        // 即使是 git repo，關掉隔離就該走跟「非 git repo」完全同一條路徑。
        let project_dir = tempfile::tempdir().unwrap();
        init_repo(project_dir.path());
        let storage_dir = tempfile::tempdir().unwrap();

        let (effective_dir, info) = prepare_worktree(
            storage_dir.path(),
            "t3",
            &project_dir.path().to_string_lossy(),
            false,
        ).await;

        assert_eq!(effective_dir, project_dir.path(), "關掉隔離就該直接在專案目錄跑");
        assert!(info.is_none(), "沒有 worktree 就不該回報 path/branch，否則合併按鈕會冒出來");
        assert!(
            !storage_dir.path().join("t3").join("worktree").exists(),
            "不該留下任何 worktree 目錄"
        );
    }

    #[tokio::test]
    async fn non_git_project_dir_is_not_isolated() {
        let project_dir = tempfile::tempdir().unwrap();
        let storage_dir = tempfile::tempdir().unwrap();

        let (effective_dir, info) = prepare_worktree(
            storage_dir.path(),
            "t1",
            &project_dir.path().to_string_lossy(),
            true,
        ).await;

        assert_eq!(effective_dir, project_dir.path());
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn git_project_dir_gets_an_isolated_worktree() {
        let project_dir = tempfile::tempdir().unwrap();
        init_repo(project_dir.path());
        let storage_dir = tempfile::tempdir().unwrap();

        let (effective_dir, info) = prepare_worktree(
            storage_dir.path(),
            "t2",
            &project_dir.path().to_string_lossy(),
            true,
        ).await;

        let (wt_path, wt_branch) = info.expect("git repo 應該要被隔離");
        assert_eq!(effective_dir.to_string_lossy(), wt_path);
        assert_eq!(wt_branch, "aiterm-task/t2");
        assert!(effective_dir.join("a.txt").exists());
        assert_ne!(effective_dir, project_dir.path(), "隔離後不該還是原本的資料夾");
    }
}

#[cfg(test)]
mod agent_dispatch_tests {
    use super::*;
    use crate::tasks::agents;
    use std::fs;
    use std::process::Command as StdCommand;

    fn init_repo(dir: &std::path::Path) {
        StdCommand::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.email", "test@test.com"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.name", "Test"]).current_dir(dir).status().unwrap();
        fs::write(dir.join("a.txt"), "hello\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir).status().unwrap();
    }

    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    async fn agent_for(pool: &sqlx::SqlitePool, repo: &std::path::Path) -> agents::AgentRow {
        let id = agents::create_agent(
            pool,
            &agents::NewAgent {
                kind: "coder", name: "C1", role: "r", prompt_prefix: "p",
                project_dir: &repo.to_string_lossy(), canvas_x: 0.0, canvas_y: 0.0,
            },
        )
        .await
        .unwrap();
        agents::get_agent(pool, &id).await.unwrap().unwrap()
    }

    fn head(dir: &std::path::Path) -> String {
        let out = StdCommand::new("git").args(["rev-parse", "HEAD"]).current_dir(dir).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn role_prefix_goes_first_and_an_empty_one_adds_nothing() {
        assert_eq!(with_role_prefix("  ", "做 A"), "做 A");
        assert_eq!(with_role_prefix("你是資深工程師", "做 A"), "你是資深工程師\n\n做 A");
    }

    #[test]
    fn model_flag_accepts_model_names_and_refuses_shell_specials() {
        assert_eq!(model_flag(Some("opus")).as_deref(), Some("--model opus"));
        assert_eq!(model_flag(Some(" claude-sonnet-5-5 ")).as_deref(), Some("--model claude-sonnet-5-5"));
        assert_eq!(model_flag(Some("opus[1m]")).as_deref(), Some("--model opus[1m]"));
        for none in [None, Some(""), Some("   ")] {
            assert_eq!(model_flag(none), None);
        }
        for bad in ["opus; rm -rf ~", "a b", "$(x)", "`x`", "a\"b", "a|b", "a&b"] {
            assert_eq!(model_flag(Some(bad)), None, "{bad}");
        }
    }

    #[test]
    fn extra_args_join_only_present_parts() {
        assert_eq!(join_args([Some("--model opus".to_string()), None, Some("--settings \"/x\"".to_string())]).as_deref(), Some("--model opus --settings \"/x\""));
        assert_eq!(join_args([None, None]), None);
    }

    #[test]
    fn non_interactive_agent_cards_are_told_not_to_stop_and_ask() {
        let auto = agent_prompt("你是工程師", "做 A", false, false);
        assert!(auto.starts_with("你是工程師\n\n做 A"));
        assert!(auto.ends_with(AUTONOMY_NOTE) && AUTONOMY_NOTE.contains("不要停下來詢問"));
        assert_eq!(agent_prompt("你是工程師", "做 A", true, false), "你是工程師\n\n做 A", "互動卡本來就有人在旁邊");
        // 掛上 ask_human 時：要決策的事用工具問，問完結束這一輪。
        let ask = agent_prompt("你是工程師", "做 A", false, true);
        assert!(ask.ends_with(AUTONOMY_NOTE_ASK) && AUTONOMY_NOTE_ASK.contains("ask_human"));
        // Codex 卡片：工具會等答案，不能叫它「結束這一輪」（那樣 codex exec 就退出了）。
        let codex = codex_autonomy(&ask);
        assert!(codex.ends_with(AUTONOMY_NOTE_ASK_WAIT) && !codex.contains("結束這一輪"), "{codex}");
    }

    #[test]
    fn path_flags_quote_the_path_and_refuse_shell_specials() {
        assert_eq!(
            crate::tasks::quota::path_flag("--mcp-config", std::path::Path::new("/a b/mcp.json")).as_deref(),
            Some(r#"--mcp-config "/a b/mcp.json""#)
        );
        assert_eq!(crate::tasks::quota::path_flag("--mcp-config", std::path::Path::new("/a$b")), None);
    }

    #[tokio::test]
    async fn the_agent_worktree_is_created_once_then_reused() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let storage = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        let agent = agent_for(&pool, repo.path()).await;

        let first = prepare_agent_worktree(&pool, storage.path(), &agent).await.unwrap();
        assert!(first.join("a.txt").exists());
        let stored = agents::get_agent(&pool, &agent.id).await.unwrap().unwrap();
        assert_eq!(stored.branch.as_deref(), Some(agents::branch_name(&agent.id).as_str()));

        // 在 worktree 裡留一個標記：第二次如果重建，標記就會不見。
        fs::write(first.join("marker.txt"), "x").unwrap();
        let second = prepare_agent_worktree(&pool, storage.path(), &stored).await.unwrap();
        assert_eq!(second, first);
        assert!(second.join("marker.txt").exists());
    }

    #[tokio::test]
    async fn a_deleted_worktree_folder_is_reattached_to_the_existing_branch() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let storage = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        let agent = agent_for(&pool, repo.path()).await;
        let wt = prepare_agent_worktree(&pool, storage.path(), &agent).await.unwrap();
        fs::write(wt.join("b.txt"), "work").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(&wt).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "work"]).current_dir(&wt).status().unwrap();
        let work_sha = head(&wt);
        fs::remove_dir_all(&wt).unwrap();

        let stored = agents::get_agent(&pool, &agent.id).await.unwrap().unwrap();
        let again = prepare_agent_worktree(&pool, storage.path(), &stored).await.unwrap();
        assert_eq!(head(&again), work_sha, "重新接上後要回到原本分支的最新 commit");
    }

    async fn coder(pool: &sqlx::SqlitePool, repo: &std::path::Path, name: &str) -> agents::AgentRow {
        let id = agents::create_agent(
            pool,
            &agents::NewAgent {
                kind: "coder", name, role: "r", prompt_prefix: "p",
                project_dir: &repo.to_string_lossy(), canvas_x: 0.0, canvas_y: 0.0,
            },
        )
        .await
        .unwrap();
        agents::get_agent(pool, &id).await.unwrap().unwrap()
    }

    fn commit_file(dir: &std::path::Path, file: &str, text: &str) -> String {
        fs::write(dir.join(file), text).unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", file]).current_dir(dir).status().unwrap();
        head(dir)
    }

    /// 工程師 X 做完並審過模組 A（審過的版本 = `approved`），工程師 Y 有一張依賴 A 的卡。
    /// 回傳 (pool, Y 的卡, X 的 worktree, Y 的 worktree, approved)。
    async fn prerequisite_setup(
        repo: &std::path::Path,
        storage: &std::path::Path,
        a_text: &str,
    ) -> (sqlx::SqlitePool, TaskRow, std::path::PathBuf, std::path::PathBuf, String) {
        init_repo(repo);
        let pool = mem_pool().await;
        let x = coder(&pool, repo, "X").await;
        let y = coder(&pool, repo, "Y").await;
        let x_wt = prepare_agent_worktree(&pool, storage, &x).await.unwrap();
        let y_wt = prepare_agent_worktree(&pool, storage, &y).await.unwrap();
        let approved = commit_file(&x_wt, "a.txt", a_text);
        let dep = store::create_task(&pool, "A 模組", "", &repo.to_string_lossy(), true, false).await.unwrap();
        agents::assign_task(&pool, &dep, Some(&x.id)).await.unwrap();
        store::dispatch_for_test(&pool, &dep, "t").await;
        store::set_end_sha(&pool, &dep, &approved).await.unwrap();
        store::finish_task(&pool, &dep, "success", None, None).await.unwrap();
        let card = store::create_task(&pool, "整合", "", &repo.to_string_lossy(), true, false).await.unwrap();
        agents::assign_task(&pool, &card, Some(&y.id)).await.unwrap();
        sqlx::query("UPDATE tasks SET depends_on = ? WHERE id = ?")
            .bind(serde_json::to_string(&[&dep]).unwrap())
            .bind(&card)
            .execute(&pool)
            .await
            .unwrap();
        let card = store::get_task(&pool, &card).await.unwrap().unwrap();
        (pool, card, x_wt, y_wt, approved)
    }

    #[tokio::test]
    async fn premerge_brings_in_the_approved_version_of_another_coders_prerequisite() {
        let repo = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let (pool, card, x_wt, y_wt, approved) = prerequisite_setup(repo.path(), storage.path(), "A 的成果\n").await;
        // X 之後又做了還沒審的東西：不能跟著合進來。
        commit_file(&x_wt, "unreviewed.txt", "還沒審\n");

        let merged = premerge_prerequisites(&pool, &card, &y_wt).await.unwrap();

        assert_eq!(merged, crate::tasks::workflow::deps_of(&card));
        assert_eq!(fs::read_to_string(y_wt.join("a.txt")).unwrap(), "A 的成果\n");
        assert!(!y_wt.join("unreviewed.txt").exists(), "只合審過的版本，不是對方分支的最新狀態");
        let ok = StdCommand::new("git").args(["merge-base", "--is-ancestor", &approved, "HEAD"]).current_dir(&y_wt).status().unwrap();
        assert!(ok.success());
    }

    #[tokio::test]
    async fn a_conflicting_prerequisite_is_rolled_back_and_reported() {
        let repo = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let (pool, card, _x_wt, y_wt, _) = prerequisite_setup(repo.path(), storage.path(), "X 的版本\n").await;
        let before = commit_file(&y_wt, "a.txt", "Y 的版本\n");

        let err = premerge_prerequisites(&pool, &card, &y_wt).await.unwrap_err();

        assert!(err.contains("a.txt") && err.contains("A 模組"), "{err}");
        assert_eq!(head(&y_wt), before, "衝突時要還原，不能停在合併一半");
        let merging = StdCommand::new("git").args(["rev-parse", "-q", "--verify", "MERGE_HEAD"]).current_dir(&y_wt).status().unwrap();
        assert!(!merging.success());
    }

    /// 一輪合併完，工程師開新卡時要從整合結果開始（不然會在舊版上做）。
    #[tokio::test]
    async fn a_coder_card_starts_from_the_last_integration_result() {
        let repo = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let pool = mem_pool().await;
        let x = coder(&pool, repo.path(), "X").await;
        let x_wt = prepare_agent_worktree(&pool, storage.path(), &x).await.unwrap();
        let card = store::create_task(&pool, "第二輪", "", &repo.path().to_string_lossy(), true, false).await.unwrap();
        agents::assign_task(&pool, &card, Some(&x.id)).await.unwrap();
        let card = store::get_task(&pool, &card).await.unwrap().unwrap();

        // 還沒有任何合併：什麼都不做。
        assert!(!sync_with_integration(&pool, &card, &x_wt).await.unwrap());

        // 協調者合併出的整合結果：多了別人的檔案。
        StdCommand::new("git").args(["checkout", "-q", "-b", "integration"]).current_dir(repo.path()).status().unwrap();
        let integrated = commit_file(repo.path(), "other.txt", "別人的模組\n");
        let merge = store::create_task(&pool, "合併", "", &repo.path().to_string_lossy(), true, false).await.unwrap();
        sqlx::query("UPDATE tasks SET card_kind = 'merge', status = 'done', outcome = 'success', end_sha = ?, finished_at = 1 WHERE id = ?")
            .bind(&integrated)
            .bind(&merge)
            .execute(&pool)
            .await
            .unwrap();

        assert!(sync_with_integration(&pool, &card, &x_wt).await.unwrap());
        assert!(x_wt.join("other.txt").exists());
        // 已經包含：不再合。
        assert!(!sync_with_integration(&pool, &card, &x_wt).await.unwrap());
    }

    #[tokio::test]
    async fn prerequisites_of_the_same_coder_are_not_merged() {
        let repo = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let (pool, card, _x_wt, y_wt, _) = prerequisite_setup(repo.path(), storage.path(), "A\n").await;
        let dep = &crate::tasks::workflow::deps_of(&card)[0];
        sqlx::query("UPDATE tasks SET agent_id = ? WHERE id = ?").bind(&card.agent_id).bind(dep).execute(&pool).await.unwrap();
        assert!(premerge_prerequisites(&pool, &card, &y_wt).await.unwrap().is_empty());
        assert_eq!(fs::read_to_string(y_wt.join("a.txt")).unwrap(), "hello\n", "a.txt 維持初始版本");
    }

    #[tokio::test]
    async fn a_non_git_project_dir_is_an_error_not_a_fallback() {
        let not_repo = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        let agent = agent_for(&pool, not_repo.path()).await;
        assert!(prepare_agent_worktree(&pool, storage.path(), &agent).await.is_err());
    }

    async fn card(pool: &sqlx::SqlitePool) -> String {
        store::create_task(pool, "A 模組", "", "/r", true, false).await.unwrap()
    }

    #[tokio::test]
    async fn success_commits_leftover_changes_and_records_end_sha() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let pool = mem_pool().await;
        let agent = agent_for(&pool, repo.path()).await;
        let task = card(&pool).await;
        let start = head(repo.path());
        fs::write(repo.path().join("new.txt"), "module A").unwrap();

        let out = finalize_agent_card(&pool, &agent.id, &task, "Module: A 模組", repo.path(), monitor::TaskOutcome::Success).await;

        assert!(matches!(out, monitor::TaskOutcome::Success));
        let end = head(repo.path());
        assert_ne!(end, start);
        let log = StdCommand::new("git").args(["log", "-1", "--format=%s"]).current_dir(repo.path()).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "Module: A 模組");
        assert_eq!(store::get_task(&pool, &task).await.unwrap().unwrap().end_sha.as_deref(), Some(end.as_str()));
        assert_eq!(agents::get_agent(&pool, &agent.id).await.unwrap().unwrap().paused_task_id, None);
    }

    #[tokio::test]
    async fn success_without_changes_keeps_end_equal_to_head() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let pool = mem_pool().await;
        let agent = agent_for(&pool, repo.path()).await;
        let task = card(&pool).await;
        let start = head(repo.path());

        let out = finalize_agent_card(&pool, &agent.id, &task, "A", repo.path(), monitor::TaskOutcome::Success).await;

        assert!(matches!(out, monitor::TaskOutcome::Success));
        assert_eq!(store::get_task(&pool, &task).await.unwrap().unwrap().end_sha.as_deref(), Some(start.as_str()));
    }

    #[tokio::test]
    async fn failure_pauses_the_agent_and_keeps_the_outcome() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let pool = mem_pool().await;
        let agent = agent_for(&pool, repo.path()).await;
        let task = card(&pool).await;
        fs::write(repo.path().join("half.txt"), "half done").unwrap();

        let out = finalize_agent_card(&pool, &agent.id, &task, "A", repo.path(), monitor::TaskOutcome::Cancelled).await;

        assert!(matches!(out, monitor::TaskOutcome::Cancelled));
        assert_eq!(agents::get_agent(&pool, &agent.id).await.unwrap().unwrap().paused_task_id.as_deref(), Some(task.as_str()));
        assert!(repo.path().join("half.txt").exists(), "半成品要留著讓使用者決定");
        assert_eq!(store::get_task(&pool, &task).await.unwrap().unwrap().end_sha, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_rejecting_commit_hook_turns_success_into_failure_and_pauses() {
        use std::os::unix::fs::PermissionsExt;
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let hook = repo.path().join(".git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\necho lint failed >&2\nexit 1\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        let pool = mem_pool().await;
        let agent = agent_for(&pool, repo.path()).await;
        let task = card(&pool).await;
        fs::write(repo.path().join("new.txt"), "x").unwrap();

        let out = finalize_agent_card(&pool, &agent.id, &task, "A", repo.path(), monitor::TaskOutcome::Success).await;

        match out {
            monitor::TaskOutcome::Failed(msg) => assert!(msg.contains("自動 commit 失敗"), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(agents::get_agent(&pool, &agent.id).await.unwrap().unwrap().paused_task_id.is_some());
    }

    #[test]
    fn commit_messages_tell_module_fix_and_integration_apart() {
        assert_eq!(commit_message_for("work", "A"), "Module: A");
        assert_eq!(commit_message_for("fix", "修正：A"), "Fix: 修正：A");
        assert_eq!(commit_message_for("merge", "合併"), "Integrate: 合併");
    }

    async fn review_card(pool: &sqlx::SqlitePool, repo: &std::path::Path, end: &str) -> TaskRow {
        let id = store::create_task(pool, "審查：A", "", &repo.to_string_lossy(), true, false).await.unwrap();
        store::set_end_sha(pool, &id, end).await.unwrap();
        store::get_task(pool, &id).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn the_review_worktree_is_checked_out_at_the_reviewed_version_and_removed_after() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let reviewed = head(repo.path());
        fs::write(repo.path().join("later.txt"), "newer work").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(repo.path()).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "later"]).current_dir(repo.path()).status().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        let reviewer = agent_for(&pool, repo.path()).await;
        let card = review_card(&pool, repo.path(), &reviewed).await;

        let wt = prepare_review_worktree(storage.path(), &reviewer.id, &card).await.unwrap();
        assert_eq!(head(&wt), reviewed, "要停在受審版本，不是 Coder 之後的 commit");
        assert!(!wt.join("later.txt").exists());

        // 審查寫了判定 → 成功並寫入 verdict，目錄被清掉。
        let json = crate::tasks::workflow::review_json_path(storage.path(), &card.id);
        fs::write(&json, r#"{"verdict":"fail","reasons":["少了權限檢查"]}"#).unwrap();
        let out = finalize_review_card(&pool, storage.path(), &reviewer.id, &card.id, &repo.path().to_string_lossy(), &wt, monitor::TaskOutcome::Success).await;
        assert!(matches!(out, monitor::TaskOutcome::Success));
        let row = store::get_task(&pool, &card.id).await.unwrap().unwrap();
        assert_eq!(row.verdict.as_deref(), Some("fail"));
        assert_eq!(row.review_reasons.as_deref(), Some(r#"["少了權限檢查"]"#));
        assert!(!wt.exists(), "審查目錄要清掉");
        assert_eq!(agents::get_agent(&pool, &reviewer.id).await.unwrap().unwrap().paused_task_id, None);
    }

    #[tokio::test]
    async fn a_review_without_review_json_fails_and_pauses_the_reviewer() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let storage = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        let reviewer = agent_for(&pool, repo.path()).await;
        let card = review_card(&pool, repo.path(), &head(repo.path())).await;
        let wt = prepare_review_worktree(storage.path(), &reviewer.id, &card).await.unwrap();

        let out = finalize_review_card(&pool, storage.path(), &reviewer.id, &card.id, &repo.path().to_string_lossy(), &wt, monitor::TaskOutcome::Success).await;

        match out {
            monitor::TaskOutcome::Failed(m) => assert!(m.contains("review.json"), "{m}"),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(agents::get_agent(&pool, &reviewer.id).await.unwrap().unwrap().paused_task_id.as_deref(), Some(card.id.as_str()));
        assert_eq!(store::get_task(&pool, &card.id).await.unwrap().unwrap().verdict, None);
    }

    #[tokio::test]
    async fn a_verify_card_records_its_verdict_stops_leftover_servers_and_cleans_up() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let storage = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        let verifier = agent_for(&pool, repo.path()).await;
        let card = review_card(&pool, repo.path(), &head(repo.path())).await;
        let wt = prepare_review_worktree(storage.path(), &verifier.id, &card).await.unwrap();
        let task_dir = crate::tasks::task_dir(storage.path(), &card.id);
        let json = crate::tasks::workflow::verify_json_path(storage.path(), &card.id);
        fs::write(&json, r#"{"verdict":"pass","items":[{"criterion":"登入","status":"fail","module_id":"m"},{"criterion":"付款","status":"manual"}]}"#).unwrap();
        // 驗收者忘了關的伺服器。
        let port = crate::tasks::verify_ports::pick_free_ports(1)[0];
        crate::tasks::verify_ports::save_ports(&task_dir, &[port]);
        #[cfg(unix)]
        let mut server = StdCommand::new("python3")
            .args(["-c", &format!("import socket,time\ns=socket.socket()\ns.bind(('127.0.0.1',{port}))\ns.listen()\ntime.sleep(60)")])
            .spawn()
            .unwrap();
        #[cfg(unix)]
        {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        let out = finalize_verify_card(&pool, storage.path(), &verifier.id, &card.id, &repo.path().to_string_lossy(), &wt, monitor::TaskOutcome::Success).await;

        assert!(matches!(out, monitor::TaskOutcome::Success));
        let row = store::get_task(&pool, &card.id).await.unwrap().unwrap();
        assert_eq!(row.verdict.as_deref(), Some("fail"), "有 fail 項就是 fail");
        assert_eq!(row.review_reasons.as_deref(), Some(r#"["登入"]"#));
        assert!(!wt.exists(), "驗收目錄要清掉");
        #[cfg(unix)]
        assert!(!server.wait().unwrap().success(), "留下來的伺服器要被結束");
    }

    #[tokio::test]
    async fn a_verify_card_without_verify_json_fails_and_pauses_the_verifier() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let storage = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        let verifier = agent_for(&pool, repo.path()).await;
        let card = review_card(&pool, repo.path(), &head(repo.path())).await;
        let wt = prepare_review_worktree(storage.path(), &verifier.id, &card).await.unwrap();

        let out = finalize_verify_card(&pool, storage.path(), &verifier.id, &card.id, &repo.path().to_string_lossy(), &wt, monitor::TaskOutcome::Success).await;

        match out {
            monitor::TaskOutcome::Failed(m) => assert!(m.contains("verify.json"), "{m}"),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(agents::get_agent(&pool, &verifier.id).await.unwrap().unwrap().paused_task_id.as_deref(), Some(card.id.as_str()));
        assert!(!wt.exists());
    }

    #[tokio::test]
    async fn retrying_a_review_rebuilds_its_worktree() {
        let repo = tempfile::tempdir().unwrap();
        init_repo(repo.path());
        let storage = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        let reviewer = agent_for(&pool, repo.path()).await;
        let card = review_card(&pool, repo.path(), &head(repo.path())).await;
        let first = prepare_review_worktree(storage.path(), &reviewer.id, &card).await.unwrap();
        fs::write(first.join("scribble.txt"), "x").unwrap();
        let again = prepare_review_worktree(storage.path(), &reviewer.id, &card).await.unwrap();
        assert_eq!(again, first);
        assert!(!again.join("scribble.txt").exists(), "重試要從乾淨的受審版本開始");
    }
}
