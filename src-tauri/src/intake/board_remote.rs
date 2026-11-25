//! 連到 server 時的共用看板：server 上的卡片對應成看板畫面用的 `TaskWithAttachments`，
//! 看板指令（列表、建立、編輯、移動、封存、刪除、附件）改走 server API，**畫面元件不用改**。
//!
//! - server 的看板專案以 `remote:<專案 id>` 出現在專案清單，指令收到這個前綴的 id 就走這裡。
//! - 畫面不知道 `version`：列出卡片時把每張卡的版本記在 `RemoteState`，編輯與移動帶上它；
//!   別人先改過就會被 server 擋下（409），這裡把原因講出來，畫面重新載入就看到最新的。
//! - 只屬於一台電腦的欄位（終端機分頁、對話紀錄、worktree…）在對應時留空；領取者自己的電腦另有影子卡片。
use super::remote::{self, RemoteState};
use crate::commands::tasks::TaskWithAttachments;
use crate::tasks::store::{AttachmentRow, TaskRow};
use aiterm_intake_client::api::Api;
use aiterm_intake_client::ClientError;
use aiterm_intake_core::board_store::{BoardAgent, BoardAttachment, BoardCard, BoardCardView, BoardNote};
use reqwest::Method;
use serde_json::json;

pub const PREFIX: &str = "remote:";

/// `remote:<專案 id>` → server 專案 id；不是遠端看板專案回 `None`。
pub fn parse(project_id: &str) -> Option<&str> {
    project_id.strip_prefix(PREFIX)
}

pub fn project_id(server_project_id: &str) -> String {
    format!("{PREFIX}{server_project_id}")
}

fn enc(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32 & 0xFF) }).collect()
}

fn stamp(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default()
}

/// server 的卡片 → 看板畫面的卡片列。`project_dir` 是這位成員自己電腦上的專案資料夾（沒設定就空）。
pub fn to_task(c: &BoardCard, project_dir: &str) -> TaskRow {
    TaskRow {
        id: c.id.clone(),
        title: c.title.clone(),
        body: c.body.clone(),
        project_dir: project_dir.to_string(),
        status: c.status.clone(),
        parallel_ok: c.parallel_ok,
        interactive: c.interactive,
        sort_order: c.sort_order,
        outcome: c.outcome.clone(),
        tab_id: None,
        transcript_path: None,
        error_message: c.error_message.clone(),
        created_at: stamp(c.created_at),
        dispatched_at: c.dispatched_at,
        finished_at: c.finished_at,
        ai_summary: c.ai_summary.clone(),
        archived_at: c.archived_at,
        session_id: None,
        session_path: None,
        use_bridge: false,
        bridge_tiers: None,
        label: c.label.clone(),
        worktree_path: None,
        isolate_worktree: None,
        worktree_branch: None,
        agent_id: c.agent_id.clone(),
        start_sha: c.start_sha.clone(),
        end_sha: c.end_sha.clone(),
        card_kind: if c.card_kind.is_empty() { "work".into() } else { c.card_kind.clone() },
        module_id: c.module_id.clone(),
        review_of: c.review_of.clone(),
        verdict: c.verdict.clone(),
        review_reasons: c.review_reasons.clone(),
        module_override: c.module_override,
        is_spec: c.is_spec,
        from_spec: c.from_spec.clone(),
        quota_wait_at: None,
        depends_on: c.depends_on.clone(),
        assigned_from: None,
        assigned_at: None,
        premerged: None,
        reply_wait_at: None,
        resume_session: false,
        cli: None,
        model: None,
        provides: None,
        fast: false,
        claimed_by: c.claimed_by.clone(),
    }
}

fn to_attachment(a: &BoardAttachment, local_path: String) -> AttachmentRow {
    AttachmentRow { id: a.id.clone(), task_id: a.card_id.clone(), filename: a.filename.clone(), stored_path: local_path }
}

/// 附件下載到本機的快取資料夾（畫面要用本機路徑預覽）；已經在就不重抓。
async fn cache_attachment(api: &Api, rs: &RemoteState, a: &BoardAttachment) -> String {
    let name = std::path::Path::new(&a.filename).file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    let dir = rs.attach_root().join("board").join(&a.id);
    let path = dir.join(&name);
    if !path.exists() {
        let Ok(bytes) = api.download(&format!("/v1/board/attachments/{}/file", enc(&a.id))).await else { return String::new() };
        if std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, bytes)).is_err() {
            return String::new();
        }
    }
    path.to_string_lossy().into_owned()
}

async fn rows(api: &Api, rs: &RemoteState, views: Vec<BoardCardView>, project_dir: &str) -> Vec<TaskWithAttachments> {
    let mut out = Vec::with_capacity(views.len());
    for v in views {
        rs.set_board_version(&v.card.id, v.card.version);
        let mut attachments = Vec::new();
        for a in &v.attachments {
            attachments.push(to_attachment(a, cache_attachment(api, rs, a).await));
        }
        out.push(TaskWithAttachments { task: to_task(&v.card, project_dir), attachments });
    }
    out
}

/// 看板上的卡片（`archived` 為真只列封存的）。
pub async fn list(api: &Api, rs: &RemoteState, server_pid: &str, archived: bool) -> Result<Vec<TaskWithAttachments>, ClientError> {
    let views: Vec<BoardCardView> = api.get(&format!("/v1/board/projects/{}/cards?archived={archived}", enc(server_pid))).await?;
    Ok(rows(api, rs, views, &rs.local_dir_of(server_pid)).await)
}

pub struct CardFields<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub parallel_ok: bool,
    pub interactive: bool,
    pub label: Option<&'a str>,
}

pub async fn create(api: &Api, rs: &RemoteState, server_pid: &str, f: CardFields<'_>) -> Result<String, ClientError> {
    let c: BoardCard = api
        .typed(Method::POST, &format!("/v1/board/projects/{}/cards", enc(server_pid)), Some(json!({"title": f.title, "body": f.body, "parallel_ok": f.parallel_ok, "interactive": f.interactive, "label": f.label})))
        .await?;
    rs.set_board_version(&c.id, c.version);
    Ok(c.id)
}

fn stale_hint(e: ClientError) -> ClientError {
    e
}

pub async fn update(api: &Api, rs: &RemoteState, id: &str, f: CardFields<'_>) -> Result<(), ClientError> {
    let version = rs.board_version(id).unwrap_or(0);
    let c: BoardCard = api
        .typed(Method::PUT, &format!("/v1/board/cards/{}", enc(id)), Some(json!({"version": version, "title": f.title, "body": f.body, "parallel_ok": f.parallel_ok, "interactive": f.interactive, "label": f.label})))
        .await
        .map_err(stale_hint)?;
    rs.set_board_version(&c.id, c.version);
    Ok(())
}

pub async fn move_card(api: &Api, rs: &RemoteState, id: &str, to_status: &str, sort_order: f64) -> Result<(), ClientError> {
    let version = rs.board_version(id).unwrap_or(0);
    let c: BoardCard = api.typed(Method::POST, &format!("/v1/board/cards/{}/move", enc(id)), Some(json!({"version": version, "to_status": to_status, "sort_order": sort_order}))).await?;
    rs.set_board_version(&c.id, c.version);
    Ok(())
}

pub async fn get_card(api: &Api, rs: &RemoteState, id: &str) -> Result<BoardCard, ClientError> {
    let c: BoardCard = api.get(&format!("/v1/board/cards/{}", enc(id))).await?;
    rs.set_board_version(&c.id, c.version);
    Ok(c)
}

/// 只改標籤：用最新的版本與目前的內容更新（標籤是改一個欄位，不該因為版本過期失敗）。
pub async fn set_label(api: &Api, rs: &RemoteState, id: &str, label: Option<&str>) -> Result<(), ClientError> {
    let cur = get_card(api, rs, id).await?;
    update(api, rs, id, CardFields { title: &cur.title, body: &cur.body, parallel_ok: cur.parallel_ok, interactive: cur.interactive, label }).await
}

pub async fn delete(api: &Api, id: &str) -> Result<(), ClientError> {
    api.request(Method::DELETE, &format!("/v1/board/cards/{}", enc(id)), None).await.map(|_| ())
}

pub async fn archive(api: &Api, id: &str, archived: bool) -> Result<(), ClientError> {
    api.request(Method::POST, &format!("/v1/board/cards/{}/archive", enc(id)), Some(json!({"archived": archived}))).await.map(|_| ())
}

pub async fn archive_done(api: &Api, server_pid: &str) -> Result<u64, ClientError> {
    api.typed(Method::POST, &format!("/v1/board/projects/{}/archive-done", enc(server_pid)), None).await
}

pub async fn add_attachment(api: &Api, rs: &RemoteState, card_id: &str, filename: &str, bytes: &[u8]) -> Result<AttachmentRow, ClientError> {
    use base64::Engine;
    let a: BoardAttachment = api
        .typed(Method::POST, &format!("/v1/board/cards/{}/attachments", enc(card_id)), Some(json!({"name": filename, "data_b64": base64::engine::general_purpose::STANDARD.encode(bytes)})))
        .await?;
    let path = cache_attachment(api, rs, &a).await;
    Ok(to_attachment(&a, path))
}

pub async fn remove_attachment(api: &Api, attachment_id: &str) -> Result<(), ClientError> {
    api.request(Method::DELETE, &format!("/v1/board/attachments/{}", enc(attachment_id)), None).await.map(|_| ())
}

/// 領取：成功回傳領到的卡片；已被領走或還不是待派送回 409（原因在錯誤訊息裡）。
pub async fn claim(api: &Api, rs: &RemoteState, id: &str) -> Result<BoardCard, ClientError> {
    let c: BoardCard = api.typed(Method::POST, &format!("/v1/board/cards/{}/claim", enc(id)), None).await?;
    rs.set_board_version(&c.id, c.version);
    Ok(c)
}

pub async fn heartbeat(api: &Api, id: &str) -> Result<bool, ClientError> {
    api.typed(Method::POST, &format!("/v1/board/cards/{}/heartbeat", enc(id)), None).await
}

pub async fn release(api: &Api, id: &str) -> Result<bool, ClientError> {
    api.typed(Method::POST, &format!("/v1/board/cards/{}/release", enc(id)), None).await
}

/// 交結果：`outcome` 是 `success` 或 `failed`。
pub async fn result(api: &Api, id: &str, outcome: &str, summary: Option<&str>, error: Option<&str>) -> Result<bool, ClientError> {
    api.typed(Method::POST, &format!("/v1/board/cards/{}/result", enc(id)), Some(json!({"outcome": outcome, "summary": summary, "error": error}))).await
}

// ---- Agent 協作：Agent、指派、記事本、簡介 ----

/// server 的 Agent → 畫布用的 `AgentRow`。只屬於一台電腦的欄位（worktree、分支、技能）留空；`project_dir` 用這位成員自己設定的資料夾。
pub fn to_agent(a: &BoardAgent, project_dir: &str) -> crate::tasks::agents::AgentRow {
    crate::tasks::agents::AgentRow {
        id: a.id.clone(),
        kind: a.kind.clone(),
        name: a.name.clone(),
        role: a.role.clone(),
        prompt_prefix: a.prompt_prefix.clone(),
        project_dir: project_dir.to_string(),
        canvas_x: a.canvas_x,
        canvas_y: a.canvas_y,
        worktree_path: None,
        branch: None,
        paused_task_id: a.paused_card_id.clone(),
        created_at: stamp(a.created_at),
        last_merge_signature: None,
        produces_spec: a.produces_spec,
        model: a.model.clone(),
        cli: a.cli.clone(),
        db_skill: None,
        custom_skills: None,
        systems: None,
    }
}

pub async fn agents(api: &Api, rs: &RemoteState, server_pid: &str) -> Result<Vec<BoardAgent>, ClientError> {
    let list: Vec<BoardAgent> = api.get(&format!("/v1/board/projects/{}/agents", enc(server_pid))).await?;
    for a in &list {
        rs.set_board_version(&a.id, a.version);
    }
    Ok(list)
}

pub struct AgentFields<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub role: &'a str,
    pub prompt_prefix: &'a str,
    pub canvas_x: f64,
    pub canvas_y: f64,
    pub produces_spec: bool,
    pub model: Option<&'a str>,
    pub cli: &'a str,
}

pub async fn create_agent(api: &Api, rs: &RemoteState, server_pid: &str, f: AgentFields<'_>) -> Result<String, ClientError> {
    let a: BoardAgent = api
        .typed(Method::POST, &format!("/v1/board/projects/{}/agents", enc(server_pid)), Some(json!({"kind": f.kind, "name": f.name, "role": f.role, "prompt_prefix": f.prompt_prefix, "canvas_x": f.canvas_x, "canvas_y": f.canvas_y, "produces_spec": f.produces_spec, "model": f.model, "cli": f.cli})))
        .await?;
    rs.set_board_version(&a.id, a.version);
    Ok(a.id)
}

/// 畫布上拖動節點也會送更新：內容（名稱、角色…）沒變就只改位置（不帶版本、不會讓別人的編輯過期），有變才用版本更新內容。
pub async fn update_agent(api: &Api, rs: &RemoteState, server_pid: &str, id: &str, f: AgentFields<'_>) -> Result<(), ClientError> {
    // 先記下畫面最後看到的版本：下面重新讀取會把它更新成最新的，那樣就擋不住「別人先改過」了。
    let seen = rs.board_version(id);
    let cur = agents(api, rs, server_pid).await?.into_iter().find(|a| a.id == id);
    if let Some(cur) = &cur {
        let same = cur.name == f.name && cur.role == f.role && cur.prompt_prefix == f.prompt_prefix && cur.produces_spec == f.produces_spec && cur.model.as_deref() == f.model && cur.cli == f.cli;
        if !same {
            let version = seen.unwrap_or(cur.version);
            let a: BoardAgent = api
                .typed(Method::PUT, &format!("/v1/board/agents/{}", enc(id)), Some(json!({"version": version, "name": f.name, "role": f.role, "prompt_prefix": f.prompt_prefix, "produces_spec": f.produces_spec, "model": f.model, "cli": f.cli})))
                .await?;
            rs.set_board_version(&a.id, a.version);
        }
    }
    if cur.as_ref().map(|a| (a.canvas_x, a.canvas_y)) != Some((f.canvas_x, f.canvas_y)) {
        api.request(Method::POST, &format!("/v1/board/agents/{}/position", enc(id)), Some(json!({"x": f.canvas_x, "y": f.canvas_y}))).await?;
    }
    Ok(())
}

pub async fn delete_agent(api: &Api, id: &str) -> Result<(), ClientError> {
    api.request(Method::DELETE, &format!("/v1/board/agents/{}", enc(id)), None).await.map(|_| ())
}

/// 指派（排進 Agent 的佇列）或取消指派；用最新的版本（指派是改一個欄位，不該因為版本過期失敗）。
pub async fn assign(api: &Api, rs: &RemoteState, card_id: &str, agent_id: Option<&str>) -> Result<(), ClientError> {
    let cur = get_card(api, rs, card_id).await?;
    let c: BoardCard = api.typed(Method::POST, &format!("/v1/board/cards/{}/assign", enc(card_id)), Some(json!({"version": cur.version, "agent_id": agent_id}))).await?;
    rs.set_board_version(&c.id, c.version);
    Ok(())
}

pub async fn notes(api: &Api, server_pid: &str) -> Result<Vec<BoardNote>, ClientError> {
    api.get(&format!("/v1/board/projects/{}/notes", enc(server_pid))).await
}

pub fn to_note(n: &BoardNote) -> crate::tasks::notes::NoteRow {
    crate::tasks::notes::NoteRow { id: n.id, author: n.author.clone(), text: n.text.clone(), created_at: stamp(n.created_at) }
}

pub async fn add_note(api: &Api, server_pid: &str, text: &str) -> Result<BoardNote, ClientError> {
    api.typed(Method::POST, &format!("/v1/board/projects/{}/notes", enc(server_pid)), Some(json!({"text": text}))).await
}

pub async fn delete_note(api: &Api, id: i64) -> Result<(), ClientError> {
    api.request(Method::DELETE, &format!("/v1/board/notes/{id}"), None).await.map(|_| ())
}

pub async fn brief(api: &Api, server_pid: &str) -> Result<Option<String>, ClientError> {
    api.get(&format!("/v1/board/projects/{}/brief", enc(server_pid))).await
}

pub async fn set_brief(api: &Api, server_pid: &str, brief: Option<&str>) -> Result<(), ClientError> {
    api.request(Method::PUT, &format!("/v1/board/projects/{}/brief", enc(server_pid)), Some(json!({"brief": brief}))).await.map(|_| ())
}

/// 這個專案現在是不是「由 server 執行」（server 自己跑 Coder → Reviewer → 待核准合併）。
pub async fn orchestration(api: &Api, server_pid: &str) -> Result<bool, ClientError> {
    let v: serde_json::Value = api.get(&format!("/v1/board/projects/{}/orchestration", enc(server_pid))).await?;
    Ok(v["enabled"].as_bool().unwrap_or(false))
}

/// 開關「由 server 執行」（server 只准管理員，其他人會被拒絕並回原因）。
pub async fn set_orchestration(api: &Api, server_pid: &str, enabled: bool) -> Result<(), ClientError> {
    api.request(Method::PUT, &format!("/v1/board/projects/{}/orchestration", enc(server_pid)), Some(json!({"enabled": enabled}))).await.map(|_| ())
}

/// 核准合併：server 把各 Coder 的分支合併進專案資料夾，回傳結果說明（管理員限定）。
pub async fn approve_merge(api: &Api, card_id: &str) -> Result<String, ClientError> {
    api.request(Method::POST, &format!("/v1/board/cards/{}/merge", enc(card_id)), None).await.map(|v| v.as_str().unwrap_or_default().to_string())
}

/// 專案清單用：每個 server 專案一筆，卡片數從 server 算。
pub async fn project_infos(api: &Api) -> Vec<(String, String, [i64; 4])> {
    let projects: Vec<serde_json::Value> = api.get("/v1/projects").await.unwrap_or_default();
    let mut out = Vec::new();
    for p in projects {
        let (Some(id), Some(name)) = (p["id"].as_str(), p["name"].as_str()) else { continue };
        let views: Vec<BoardCardView> = api.get(&format!("/v1/board/projects/{}/cards", enc(id))).await.unwrap_or_default();
        let mut counts = [0i64; 4];
        for v in &views {
            match v.card.status.as_str() {
                "planning" => counts[0] += 1,
                "queued" => counts[1] += 1,
                "running" => counts[2] += 1,
                "done" => counts[3] += 1,
                _ => {}
            }
        }
        out.push((id.to_string(), name.to_string(), counts));
    }
    out
}

/// 讓指令層少寫一層：連線中才有 Api，否則回 `LOCAL_ONLY` 風格的錯誤。
pub fn require(rs: &RemoteState) -> Result<Api, String> {
    rs.active().ok_or_else(|| "尚未連到 server".to_string())
}

pub fn fail(rs: &RemoteState, e: ClientError) -> String {
    remote::fail(rs, e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiterm_intake_core::api::Role;
    use aiterm_intake_server::testkit::TestServer;

    struct Rig {
        srv: TestServer,
        pid: String,
        api: Api,
        other: Api,
        rs: RemoteState,
        _tmp: tempfile::TempDir,
    }

    async fn rig() -> Rig {
        let srv = TestServer::start().await;
        let pid = srv.project("範例科技").await;
        let (api, other) = (srv.paired("小華", Role::Member).await, srv.paired("小明", Role::Member).await);
        let rs = RemoteState::default();
        let tmp = tempfile::tempdir().unwrap();
        rs.set_attach_root(tmp.path().to_path_buf());
        Rig { srv, pid, api, other, rs, _tmp: tmp }
    }

    fn fields<'a>(title: &'a str) -> CardFields<'a> {
        CardFields { title, body: "內容", parallel_ok: true, interactive: false, label: None }
    }

    #[test]
    fn remote_project_ids_round_trip_and_local_ids_are_not_remote() {
        assert_eq!(parse(&project_id("abc-1")), Some("abc-1"));
        assert_eq!(parse("e6d1c2f0-local-uuid"), None);
        assert_eq!(parse("unopened:/x"), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_server_card_appears_as_an_ordinary_board_card_with_local_only_fields_empty() {
        let r = rig().await;
        let id = create(&r.api, &r.rs, &r.pid, CardFields { label: Some("急"), ..fields("印表機卡紙") }).await.unwrap();
        let cards = list(&r.other, &RemoteState::default(), &r.pid, false).await.unwrap();
        assert_eq!(cards.len(), 1, "另一位成員看到同一張");
        let t = &cards[0].task;
        assert_eq!((t.id.as_str(), t.title.as_str(), t.status.as_str(), t.label.as_deref(), t.card_kind.as_str(), t.parallel_ok), (id.as_str(), "印表機卡紙", "planning", Some("急"), "work", true));
        assert!(t.tab_id.is_none() && t.worktree_path.is_none() && t.session_path.is_none() && t.agent_id.is_none(), "只屬於一台電腦的欄位是空的");
        assert!(t.created_at.len() == 19, "建立時間用看板畫面慣用的格式：{}", t.created_at);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_edit_from_a_stale_view_is_refused_with_the_reason_and_works_after_reloading() {
        let r = rig().await;
        let id = create(&r.api, &r.rs, &r.pid, fields("原標題")).await.unwrap();
        // 小明在小華不知道的時候改了這張卡。
        let theirs = RemoteState::default();
        list(&r.other, &theirs, &r.pid, false).await.unwrap();
        update(&r.other, &theirs, &id, fields("小明改的")).await.unwrap();

        let e = update(&r.api, &r.rs, &id, fields("小華改的")).await.unwrap_err().to_string();
        assert!(e.contains("已被別人修改"), "要明講原因：{e}");
        assert_eq!(list(&r.api, &r.rs, &r.pid, false).await.unwrap()[0].task.title, "小明改的", "沒有靜默覆蓋別人的修改");
        update(&r.api, &r.rs, &id, fields("小華改的")).await.unwrap(); // 重新載入後版本是新的
        assert_eq!(list(&r.api, &r.rs, &r.pid, false).await.unwrap()[0].task.title, "小華改的");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn moving_uses_the_sort_order_the_board_computed_and_the_label_changes_without_a_version_fight() {
        let r = rig().await;
        let (a, b) = (create(&r.api, &r.rs, &r.pid, fields("a")).await.unwrap(), create(&r.api, &r.rs, &r.pid, fields("b")).await.unwrap());
        move_card(&r.api, &r.rs, &a, "queued", 5.0).await.unwrap();
        move_card(&r.api, &r.rs, &b, "queued", 2.5).await.unwrap();
        let titles: Vec<String> = list(&r.api, &r.rs, &r.pid, false).await.unwrap().into_iter().map(|t| t.task.title).collect();
        assert_eq!(titles, vec!["b", "a"], "依畫面算好的排序值排列");
        assert!(move_card(&r.api, &r.rs, &a, "done", 1.0).await.unwrap_err().to_string().contains("不能從"));
        // 別人改了內容之後，我只改標籤仍然成功。
        let theirs = RemoteState::default();
        list(&r.other, &theirs, &r.pid, false).await.unwrap();
        update(&r.other, &theirs, &a, fields("a 改過")).await.unwrap();
        set_label(&r.api, &r.rs, &a, Some("緊急")).await.unwrap();
        let cards = list(&r.api, &r.rs, &r.pid, false).await.unwrap();
        let a_row = cards.iter().find(|t| t.task.id == a).unwrap();
        assert_eq!((a_row.task.title.as_str(), a_row.task.label.as_deref()), ("a 改過", Some("緊急")), "標籤用最新的內容改，不會蓋掉別人的標題");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attachments_are_cached_locally_for_preview_and_removed_with_the_server_copy() {
        let r = rig().await;
        let id = create(&r.api, &r.rs, &r.pid, fields("有附件")).await.unwrap();
        let att = add_attachment(&r.api, &r.rs, &id, "err.log", b"ERROR boom\n").await.unwrap();
        assert_eq!(std::fs::read_to_string(&att.stored_path).unwrap(), "ERROR boom\n", "附件在本機有一份，畫面才預覽得到");
        let cards = list(&r.other, &RemoteState::default(), &r.pid, false).await.unwrap();
        assert_eq!(cards[0].attachments.len(), 1, "另一位成員也看得到");
        remove_attachment(&r.api, &att.id).await.unwrap();
        assert!(list(&r.api, &r.rs, &r.pid, false).await.unwrap()[0].attachments.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn archiving_deleting_and_the_project_list_counts_follow_the_server() {
        let r = rig().await;
        let done = create(&r.api, &r.rs, &r.pid, fields("做完")).await.unwrap();
        let waiting = create(&r.api, &r.rs, &r.pid, fields("等著")).await.unwrap();
        move_card(&r.api, &r.rs, &waiting, "queued", 1.0).await.unwrap();
        // 把「做完」那張走完領取流程（直接打 API，領取的 AITerm 端第 3 步才做）。
        move_card(&r.api, &r.rs, &done, "queued", 2.0).await.unwrap();
        r.api.request(Method::POST, &format!("/v1/board/cards/{done}/claim"), None).await.unwrap();
        r.api.request(Method::POST, &format!("/v1/board/cards/{done}/result"), Some(json!({"outcome": "success"}))).await.unwrap();

        let infos = project_infos(&r.api).await;
        let (_, name, counts) = infos.iter().find(|(id, _, _)| id == &r.pid).unwrap();
        assert_eq!((name.as_str(), counts), ("範例科技", &[0, 1, 0, 1]), "計畫中／待派送／執行中／完成");

        archive(&r.api, &done, true).await.unwrap();
        assert_eq!(list(&r.api, &r.rs, &r.pid, false).await.unwrap().len(), 1);
        let archived = list(&r.api, &r.rs, &r.pid, true).await.unwrap();
        assert_eq!((archived.len(), archived[0].task.id.as_str()), (1, done.as_str()));
        archive(&r.api, &done, false).await.unwrap();
        assert_eq!(archive_done(&r.srv.paired("管理員", Role::Admin).await, &r.pid).await.unwrap(), 1);
        delete(&r.api, &waiting).await.unwrap();
        assert_eq!(list(&r.api, &r.rs, &r.pid, false).await.unwrap().len(), 0);
        assert_eq!(list(&r.api, &r.rs, &r.pid, true).await.unwrap().len(), 1);
    }

    fn agent_fields<'a>(name: &'a str) -> AgentFields<'a> {
        AgentFields { kind: "coder", name, role: "Senior Coder", prompt_prefix: "多寫測試", canvas_x: 10.0, canvas_y: 20.0, produces_spec: false, model: None, cli: "claude" }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agents_are_shared_and_a_canvas_drag_only_moves_the_node_without_touching_the_content() {
        let r = rig().await;
        let id = create_agent(&r.api, &r.rs, &r.pid, agent_fields("Coder A")).await.unwrap();
        let theirs = RemoteState::default();
        let seen = agents(&r.other, &theirs, &r.pid).await.unwrap();
        assert_eq!(seen.len(), 1, "另一位成員看到同一個 Agent");
        let row = to_agent(&seen[0], "/my/repo");
        assert_eq!((row.id.as_str(), row.kind.as_str(), row.project_dir.as_str(), row.worktree_path.clone(), row.cli.as_str()), (id.as_str(), "coder", "/my/repo", None, "claude"), "只屬於本機的欄位留空，資料夾用我自己的");

        // 小明在小華不知道的時候改了名稱；小華只是拖動節點，不能蓋掉小明的修改。
        update_agent(&r.other, &theirs, &r.pid, &id, agent_fields("小明改的名字")).await.unwrap();
        update_agent(&r.api, &r.rs, &r.pid, &id, AgentFields { canvas_x: 300.0, canvas_y: 40.0, ..agent_fields("Coder A") }).await.expect_err("內容跟 server 不同，會被當成內容編輯；版本過期要擋下");
        let after = agents(&r.api, &r.rs, &r.pid).await.unwrap();
        assert_eq!((after[0].name.as_str(), after[0].canvas_x), ("小明改的名字", 10.0), "沒有靜默覆蓋，位置也沒被半套改掉");
        // 重新載入之後（畫布現在顯示的是 server 的內容）再拖動，只改位置。
        let latest = after[0].name.clone();
        update_agent(&r.api, &r.rs, &r.pid, &id, AgentFields { canvas_x: 300.0, canvas_y: 40.0, ..agent_fields(&latest) }).await.unwrap();
        let moved = agents(&r.api, &r.rs, &r.pid).await.unwrap();
        assert_eq!((moved[0].canvas_x, moved[0].canvas_y, moved[0].version), (300.0, 40.0, after[0].version), "拖動不算內容變動，版本不變");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn assigning_notes_and_the_brief_go_through_the_server() {
        let r = rig().await;
        let agent = create_agent(&r.api, &r.rs, &r.pid, agent_fields("Coder A")).await.unwrap();
        let card = create(&r.api, &r.rs, &r.pid, fields("要做的模組")).await.unwrap();
        // 別人先改了卡片：指派仍然成功（指派只改一個欄位，用最新版本）。
        let theirs = RemoteState::default();
        list(&r.other, &theirs, &r.pid, false).await.unwrap();
        update(&r.other, &theirs, &card, fields("模組（改過）")).await.unwrap();
        assign(&r.api, &r.rs, &card, Some(&agent)).await.unwrap();
        let c = &list(&r.api, &r.rs, &r.pid, false).await.unwrap()[0].task;
        assert_eq!((c.title.as_str(), c.status.as_str(), c.agent_id.as_deref()), ("模組（改過）", "queued", Some(agent.as_str())));
        assign(&r.api, &r.rs, &card, None).await.unwrap();
        assert_eq!(list(&r.api, &r.rs, &r.pid, false).await.unwrap()[0].task.agent_id, None);
        assert!(assign(&r.api, &r.rs, &card, Some("不存在")).await.is_err());

        let n = add_note(&r.api, &r.pid, "  先做登入  ").await.unwrap();
        let rows: Vec<_> = notes(&r.other, &r.pid).await.unwrap().iter().map(to_note).collect();
        assert_eq!((rows.len(), rows[0].text.as_str(), rows[0].author.as_str(), rows[0].created_at.len()), (1, "先做登入", "小華", 19));
        delete_note(&r.other, n.id).await.unwrap();
        assert!(notes(&r.api, &r.pid).await.unwrap().is_empty());

        assert_eq!(brief(&r.api, &r.pid).await.unwrap(), None);
        set_brief(&r.other, &r.pid, Some("記帳 App")).await.unwrap();
        assert_eq!(brief(&r.api, &r.pid).await.unwrap().as_deref(), Some("記帳 App"));
        set_brief(&r.api, &r.pid, None).await.unwrap();
        assert_eq!(brief(&r.other, &r.pid).await.unwrap(), None);

        delete_agent(&r.api, &agent).await.unwrap();
        assert!(agents(&r.api, &r.rs, &r.pid).await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn orchestration_toggle_and_merge_approval_are_admin_only_and_errors_carry_the_servers_reason() {
        let r = rig().await;
        let admin = r.srv.paired("管理員", Role::Admin).await;
        assert!(!orchestration(&admin, &r.pid).await.unwrap(), "預設是關的");
        // 專案沒設工作資料夾：server 拒絕，原因要原樣回來。
        let e = set_orchestration(&admin, &r.pid, true).await.unwrap_err();
        assert!(e.to_string().contains("工作資料夾"), "{e}");
        sqlx::query("UPDATE projects SET work_dir = ? WHERE id = ?").bind(std::env::temp_dir().to_string_lossy().to_string()).bind(&r.pid).execute(&r.srv.state().pools.server).await.unwrap();
        set_orchestration(&admin, &r.pid, true).await.unwrap();
        assert!(orchestration(&r.other, &r.pid).await.unwrap(), "另一位成員也看得到");
        let card0 = create(&admin, &r.rs, &r.pid, fields("先建一張")).await.unwrap();
        assert!(set_orchestration(&r.other, &r.pid, false).await.is_err(), "成員不能關");
        assert!(approve_merge(&r.other, &card0).await.is_err(), "成員不能核准");
        assert!(orchestration(&admin, &r.pid).await.unwrap());
        // 不是合併卡：不能核准。
        let card = create(&r.api, &r.rs, &r.pid, fields("一般卡片")).await.unwrap();
        assert!(approve_merge(&admin, &card).await.is_err());
        set_orchestration(&admin, &r.pid, false).await.unwrap();
        assert!(!orchestration(&r.other, &r.pid).await.unwrap());
    }
}
