//! Tauri 指令：設定、白名單、清單、派送、分頁接收端。
use super::config::{self, Config};
use super::rules::{Level, Sender};
use super::runtime::{self, IntakeRuntime};
use super::store::{self, IntakeItem};
use super::{IntakeDb, CHANGED_EVENT};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

fn e(err: sqlx::Error) -> String {
    err.to_string()
}

fn changed(app: &AppHandle) {
    let _ = app.emit(CHANGED_EVENT, ());
}

pub use aiterm_intake_core::ops::{ConfigView, SenderDto};

#[tauri::command]
pub async fn intake_get_config(db: State<'_, IntakeDb>, project_id: String) -> Result<ConfigView, String> {
    Ok(aiterm_intake_core::ops::get_config(&db.pool, &project_id).await)
}

#[tauri::command]
pub async fn intake_set_config(app: AppHandle, db: State<'_, IntakeDb>, project_id: String, config: Config, telegram_default: bool) -> Result<(), String> {
    aiterm_intake_core::ops::set_config(&db.pool, &project_id, config, telegram_default, &super::open_projects(&app)).await?;
    // 網頁表單的開關／埠號可能變了。
    super::web::restart(&app).await;
    super::line_server::restart(&app).await;
    changed(&app);
    Ok(())
}

/// 「未歸屬」的舊設定與白名單複製到這個專案（不刪除舊的）。
#[tauri::command]
pub async fn intake_copy_legacy_config(app: AppHandle, db: State<'_, IntakeDb>, project_id: String) -> Result<(), String> {
    if project_id.is_empty() {
        return Err("請先選一個專案".into());
    }
    let mut cfg = config::load(&db.pool, "").await;
    cfg.enabled = false; // 複製來的設定先停用，免得跟舊設定的別名／標籤衝突
    config::save(&db.pool, &project_id, &cfg).await.map_err(e)?;
    for s in store::list_senders(&db.pool, "").await.map_err(e)? {
        store::upsert_sender(&db.pool, &project_id, &s).await.map_err(e)?;
    }
    changed(&app);
    Ok(())
}

#[derive(Serialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    /// 專案固定的程式庫目錄（看板設定的 `repo_dir`）；沒設定為 `None`。
    pub repo_dir: Option<String>,
    /// 需要你處理的題數（待派送＋待寄出草稿）與總數。
    pub open_count: i64,
    pub total: i64,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct ProjectsView {
    pub projects: Vec<ProjectSummary>,
    /// 尚未歸屬的題數（升級前的舊資料）。為 0 就不顯示「未歸屬」。
    pub unassigned: i64,
    /// 「未歸屬」底下還留著升級前的設定或白名單（可複製到專案）。
    pub legacy_settings: bool,
}

#[tauri::command]
pub async fn intake_projects(app: AppHandle, db: State<'_, IntakeDb>, reg: State<'_, crate::projects::ProjectRegistry>) -> Result<ProjectsView, String> {
    let counts = store::project_counts(&db.pool).await.map_err(e)?;
    let count_of = |id: &str| counts.iter().find(|(p, _, _)| p == id).map(|(_, o, t)| (*o, *t)).unwrap_or((0, 0));
    let mut handles = reg.all();
    handles.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));

    // 升級後只有一個專案：把舊資料直接歸給它，使用者不用做任何事。
    let (legacy_open, legacy_total) = count_of("");
    let _ = legacy_open;
    if handles.len() == 1 && (legacy_total > 0 || config::exists(&db.pool, "").await) {
        let id = handles[0].id.clone();
        adopt(&db.pool, &id).await.map_err(e)?;
        changed(&app);
    }
    let counts = store::project_counts(&db.pool).await.map_err(e)?;
    let count_of = |id: &str| counts.iter().find(|(p, _, _)| p == id).map(|(_, o, t)| (*o, *t)).unwrap_or((0, 0));

    let mut projects = Vec::new();
    for h in handles {
        let (open_count, total) = count_of(&h.id);
        let repo_dir = crate::tasks::repo_check::saved_project_repo(&h.pool).await.ok().flatten();
        projects.push(ProjectSummary { enabled: config::load(&db.pool, &h.id).await.enabled, id: h.id, name: h.name, repo_dir, open_count, total });
    }
    let legacy_settings = config::exists(&db.pool, "").await || !store::list_senders(&db.pool, "").await.map_err(e)?.is_empty();
    Ok(ProjectsView { projects, unassigned: count_of("").1, legacy_settings })
}

/// 舊設定（若新專案還沒有自己的）、白名單、題目與丟棄計數歸給專案。
async fn adopt(pool: &sqlx::SqlitePool, project_id: &str) -> Result<(), sqlx::Error> {
    if !config::exists(pool, project_id).await && config::exists(pool, "").await {
        let cfg = config::load(pool, "").await;
        config::save(pool, project_id, &cfg).await?;
    }
    store::adopt_legacy(pool, project_id).await?;
    Ok(())
}

/// 把所有「未歸屬」的舊資料歸給這個專案。
#[tauri::command]
pub async fn intake_adopt_legacy(app: AppHandle, db: State<'_, IntakeDb>, project_id: String) -> Result<(), String> {
    if project_id.is_empty() {
        return Err("請先選一個專案".into());
    }
    adopt(&db.pool, &project_id).await.map_err(e)?;
    changed(&app);
    Ok(())
}

/// 把單一未歸屬的題目指派到專案。
#[tauri::command]
pub async fn intake_assign(app: AppHandle, db: State<'_, IntakeDb>, id: String, project_id: String) -> Result<bool, String> {
    let r = store::assign_item(&db.pool, &id, &project_id).await.map_err(e)?;
    changed(&app);
    Ok(r)
}

#[tauri::command]
pub async fn intake_list(db: State<'_, IntakeDb>, project_id: String, status: Option<String>) -> Result<Vec<IntakeItem>, String> {
    store::list_items(&db.pool, &project_id, status.as_deref()).await.map_err(e)
}

#[tauri::command]
pub async fn intake_senders(db: State<'_, IntakeDb>, project_id: String) -> Result<Vec<SenderDto>, String> {
    aiterm_intake_core::ops::list_senders(&db.pool, &project_id).await
}

#[tauri::command]
pub async fn intake_upsert_sender(db: State<'_, IntakeDb>, project_id: String, sender: SenderDto) -> Result<(), String> {
    aiterm_intake_core::ops::upsert_sender(&db.pool, &project_id, sender).await
}

/// 設定（或清除）白名單項目的備註名稱。
#[tauri::command]
pub async fn intake_set_sender_label(db: State<'_, IntakeDb>, project_id: String, address: String, label: String) -> Result<(), String> {
    store::set_sender_label(&db.pool, &project_id, &address, &label).await.map_err(e)
}

#[tauri::command]
pub async fn intake_delete_sender(db: State<'_, IntakeDb>, project_id: String, address: String) -> Result<bool, String> {
    store::delete_sender(&db.pool, &project_id, &address).await.map_err(e)
}

#[tauri::command]
pub async fn intake_dropped(db: State<'_, IntakeDb>, project_id: String) -> Result<Vec<(String, i64)>, String> {
    store::dropped_counts(&db.pool, &project_id).await.map_err(e)
}

#[derive(Serialize)]
pub struct RefreshResult {
    /// 有去信箱收信（這個專案有設定收信帳號才會）。
    pub polled: bool,
    /// 收信是否成功；沒去收信時為 true。
    pub ok: bool,
    /// 這次重新整理新增的題目數（這個專案）。
    pub new_items: i64,
    pub error: Option<String>,
}

/// 「重新整理」：立刻去這個專案設定的信箱收一次信，收完再通知畫面重新載入。
/// 做法是重啟該信箱的背景任務——它連上線後一定先同步一次——並等同步結束的訊號。
#[tauri::command]
pub async fn intake_refresh(app: AppHandle, db: State<'_, IntakeDb>, project_id: String) -> Result<RefreshResult, String> {
    let cfg = config::load(&db.pool, &project_id).await;
    let before = store::list_items(&db.pool, &project_id, None).await.map_err(e)?.len() as i64;
    let mut out = RefreshResult { polled: false, ok: true, new_items: 0, error: None };
    if cfg.enabled && !cfg.account_id.is_empty() {
        use tauri::Manager;
        out.polled = true;
        let mut rx = app.state::<crate::mail::manager::MailSyncSignals>().0.subscribe();
        // 先訂閱再重啟，才不會漏掉很快就結束的同步。
        crate::mail::poller::restart_account(&app, &cfg.account_id).await;
        let wait = async {
            loop {
                match rx.recv().await {
                    Ok((id, ok)) if id == cfg.account_id => return Some(ok),
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return None,
                }
            }
        };
        match tokio::time::timeout(std::time::Duration::from_secs(45), wait).await {
            Ok(Some(true)) => {}
            Ok(Some(false)) => {
                out.ok = false;
                out.error = Some("信箱連線或收信失敗，請檢查信箱帳號設定".into());
            }
            _ => {
                out.ok = false;
                out.error = Some("等了 45 秒還沒收完，會在背景繼續收".into());
            }
        }
    }
    out.new_items = store::list_items(&db.pool, &project_id, None).await.map_err(e)?.len() as i64 - before;
    changed(&app);
    Ok(out)
}

#[tauri::command]
pub async fn intake_approve(app: AppHandle, db: State<'_, IntakeDb>, id: String) -> Result<bool, String> {
    let r = store::approve(&db.pool, &id).await.map_err(e)?;
    changed(&app);
    Ok(r)
}

#[tauri::command]
pub async fn intake_reject(app: AppHandle, db: State<'_, IntakeDb>, id: String) -> Result<bool, String> {
    let r = store::reject(&db.pool, &id).await.map_err(e)?;
    changed(&app);
    Ok(r)
}

#[tauri::command]
pub async fn intake_delete(app: AppHandle, db: State<'_, IntakeDb>, id: String) -> Result<bool, String> {
    let r = store::delete_item(&db.pool, &id).await.map_err(e)?;
    if r {
        super::attach::remove_dir(&super::attach::root(), &id);
    }
    changed(&app);
    Ok(r)
}

#[tauri::command]
pub async fn intake_update_draft(app: AppHandle, db: State<'_, IntakeDb>, id: String, draft: String) -> Result<bool, String> {
    let r = store::update_draft(&db.pool, &id, &draft).await.map_err(e)?;
    changed(&app);
    Ok(r)
}

#[tauri::command]
pub async fn intake_send_reply(app: AppHandle, db: State<'_, IntakeDb>, id: String) -> Result<(), String> {
    let r = runtime::send_reply(&app, &id).await;
    if let Err(msg) = &r {
        store::set_error(&db.pool, &id, &format!("寄出失敗：{msg}")).await.ok();
        changed(&app);
    }
    r
}

/// 這個分頁開始接收問題。
#[tauri::command]
pub async fn intake_tab_start(app: AppHandle, rt: State<'_, Arc<IntakeRuntime>>, tab_id: String, project_id: String) -> Result<(), String> {
    rt.registry.register(&tab_id, &project_id);
    changed(&app);
    runtime::tick(&app).await;
    Ok(())
}

/// 這個分頁停止接收；進行中的問題退回 pending。
#[tauri::command]
pub async fn intake_tab_stop(app: AppHandle, db: State<'_, IntakeDb>, rt: State<'_, Arc<IntakeRuntime>>, tab_id: String) -> Result<(), String> {
    if let Some(id) = rt.registry.unregister(&tab_id) {
        store::fail(&db.pool, &id, "已停止問題收集").await.map_err(e)?;
    }
    changed(&app);
    Ok(())
}

#[derive(Serialize)]
pub struct TabDto {
    pub tab_id: String,
    pub project_id: String,
    /// 這個分頁現在能收提問（有 Claude Code 在跑）。false 時畫面提示使用者先啟動 claude。
    pub claude_ready: bool,
}

/// 把這個分頁切換到專案的專案目錄。只在分頁閒置在 shell 提示字元時才送。
/// 路徑從專案設定讀，不接受前端傳來的路徑，避免被拿去在終端機裡執行任意字串。
#[tauri::command]
pub async fn intake_tab_cd(
    reg: State<'_, crate::projects::ProjectRegistry>,
    pty: State<'_, Arc<crate::pty::PtyManager>>,
    tab_id: String,
    project_id: String,
) -> Result<String, String> {
    let handle = reg.get(&project_id).ok_or("找不到這個專案")?;
    let dir = crate::tasks::repo_check::saved_project_repo(&handle.pool)
        .await
        .map_err(e)?
        .ok_or("這個專案還沒有設定專案目錄（請在 Agent 協作設定）")?;
    if !std::path::Path::new(&dir).is_dir() {
        return Err(format!("專案目錄不存在：{dir}"));
    }
    super::tabcd::cd_tab(&pty, &tab_id, &dir).await?;
    Ok(dir)
}

#[tauri::command]
pub fn intake_tabs(rt: State<'_, Arc<IntakeRuntime>>, pty: State<'_, Arc<crate::pty::PtyManager>>) -> Vec<TabDto> {
    rt.registry
        .tabs()
        .into_iter()
        .map(|t| TabDto {
            project_id: rt.registry.project_of(&t).unwrap_or_default(),
            claude_ready: runtime::can_type_prompt(&rt.registry, &pty, &t).is_ok(),
            tab_id: t,
        })
        .collect()
}

#[tauri::command]
pub async fn intake_attachments(db: State<'_, IntakeDb>, item_id: String) -> Result<Vec<store::AttachmentRow>, String> {
    store::list_attachments(&db.pool, &item_id).await.map_err(e)
}

/// 讀出附圖內容，回傳 `data:` URL 給前端顯示。只讀我們自己存放目錄底下的檔案。
#[tauri::command]
pub async fn intake_attachment_data(db: State<'_, IntakeDb>, id: String) -> Result<String, String> {
    let row = store::get_attachment(&db.pool, &id).await.map_err(e)?.ok_or("找不到附圖")?;
    data_url(&super::attach::root(), &row)
}

/// 純文字附件（log、txt、json…）的前 64KB，給畫面預覽。只讀我們自己存放目錄底下的檔案。
#[tauri::command]
pub async fn intake_attachment_text(db: State<'_, IntakeDb>, id: String) -> Result<String, String> {
    let row = store::get_attachment(&db.pool, &id).await.map_err(e)?.ok_or("找不到附件")?;
    text_preview(&super::attach::root(), &row)
}

pub use aiterm_intake_core::attach::{data_url, text_preview, PREVIEW_BYTES};

#[cfg(test)]
mod conflict_tests {
    use super::*;
    use crate::intake::rules::MatchMode;
    use aiterm_intake_core::ops::resolve_conflicts;
    use sqlx::sqlite::SqlitePoolOptions;

    fn cfg(tag: &str) -> Config {
        Config { enabled: true, account_id: "acc".into(), mode: MatchMode::SubjectTag, tag: tag.into(), ..Config::default() }
    }
    async fn pool() -> sqlx::SqlitePool {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        p
    }

    #[tokio::test]
    async fn a_leftover_unassigned_config_is_switched_off_instead_of_blocking_the_save() {
        let p = pool().await;
        config::save(&p, "", &cfg("[AITerm]")).await.unwrap();
        let projects = vec![("a".to_string(), "專案A".to_string())];
        resolve_conflicts(&p, "a", &cfg("[AITerm]"), &projects).await.expect("舊的未歸屬設定不該擋住新專案");
        assert!(!config::load(&p, "").await.enabled, "衝突的舊設定被停用，否則同一封信會有兩個接收者");
    }

    #[tokio::test]
    async fn a_non_conflicting_leftover_is_left_alone_and_a_real_project_conflict_still_blocks() {
        let p = pool().await;
        config::save(&p, "", &cfg("[Old]")).await.unwrap();
        config::save(&p, "b", &cfg("[Same]")).await.unwrap();
        let projects = vec![("a".to_string(), "專案A".to_string()), ("b".to_string(), "專案B".to_string())];
        resolve_conflicts(&p, "a", &cfg("[New]"), &projects).await.unwrap();
        assert!(config::load(&p, "").await.enabled, "沒有衝突就不動舊設定");
        let err = resolve_conflicts(&p, "a", &cfg("[same]"), &projects).await.unwrap_err();
        assert!(err.contains("專案B") && err.contains("主旨標籤"), "{err}");
        assert!(config::load(&p, "").await.enabled, "被擋下的這次不能順便動到舊設定");
    }
}

#[cfg(test)]
mod adopt_tests {
    use super::*;
    use crate::intake::store::NewItem;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn adopting_moves_config_senders_items_and_drops_together() {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        config::save(&p, "", &Config { enabled: true, tag: "[OLD]".into(), ..Config::default() }).await.unwrap();
        store::upsert_sender(&p, "", &Sender { address: "a@b.co".into(), level: Level::Internal, skip_auth: false }).await.unwrap();
        store::bump_dropped(&p, "", "not_allowed").await.unwrap();
        store::insert_item(&p, NewItem { project_id: "", id: None, source: "email", sender: "a@b.co", level: Level::Internal, subject: "s", body: "b", message_id: "m", status: store::PENDING }).await.unwrap();

        adopt(&p, "proj").await.unwrap();

        assert_eq!(config::load(&p, "proj").await.tag, "[OLD]");
        assert!(config::exists(&p, "proj").await);
        assert_eq!(store::list_senders(&p, "proj").await.unwrap().len(), 1);
        assert!(store::list_senders(&p, "").await.unwrap().is_empty());
        assert_eq!(store::list_items(&p, "proj", None).await.unwrap().len(), 1);
        assert_eq!(store::dropped_counts(&p, "proj").await.unwrap().len(), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &std::path::Path) -> store::AttachmentRow {
        store::AttachmentRow { id: "a".into(), item_id: "i".into(), filename: "f".into(), path: path.to_string_lossy().into(), mime: "image/png".into(), size: 1 }
    }

    #[test]
    fn serves_images_under_the_root_and_refuses_anything_else() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1];
        let inside = root.path().join("i.png");
        std::fs::write(&inside, png).unwrap();
        assert!(data_url(root.path(), &row(&inside)).unwrap().starts_with("data:image/png;base64,"));

        let elsewhere = outside.path().join("secret.png");
        std::fs::write(&elsewhere, png).unwrap();
        assert!(data_url(root.path(), &row(&elsewhere)).is_err(), "資料庫被竄改也不能讀到目錄外的檔案");

        let sneaky = root.path().join("..").join(outside.path().file_name().unwrap()).join("secret.png");
        assert!(data_url(root.path(), &row(&sneaky)).is_err(), "用 .. 繞出去也不行");

        let text = root.path().join("n.png");
        std::fs::write(&text, b"not an image").unwrap();
        assert!(data_url(root.path(), &row(&text)).is_err(), "副檔名是 png 但內容不是圖片");
    }

    #[test]
    fn text_preview_reads_only_text_under_the_root_and_cuts_long_files() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let log = root.path().join("a.log");
        std::fs::write(&log, "ERROR boom\n中文也行\n").unwrap();
        assert_eq!(text_preview(root.path(), &row(&log)).unwrap(), "ERROR boom\n中文也行\n");

        let big = root.path().join("big.log");
        std::fs::write(&big, "x".repeat(PREVIEW_BYTES + 100)).unwrap();
        let t = text_preview(root.path(), &row(&big)).unwrap();
        assert!(t.starts_with("xxx") && t.ends_with("只顯示前 64KB）") && t.len() < PREVIEW_BYTES + 100, "{}", t.len());

        let bin = root.path().join("b.bin");
        std::fs::write(&bin, b"\0\x01\x02binary").unwrap();
        assert!(text_preview(root.path(), &row(&bin)).is_err(), "二進位不預覽");

        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "密碼").unwrap();
        assert!(text_preview(root.path(), &row(&secret)).is_err(), "目錄外的檔案不能讀");
    }
}

/// 檢查參考資料夾：存不存在、是不是資料夾、Claude Code 讀得到／讀不了的檔案有多少。
/// 相對路徑接在這個專案的程式庫資料夾後面（沒有就對不上）。
#[tauri::command]
pub async fn intake_check_folders(app: AppHandle, project_id: String, paths: Vec<String>) -> Result<Vec<super::refs::FolderCheck>, String> {
    use tauri::Manager;
    let base: Option<std::path::PathBuf> = match app.try_state::<crate::projects::ProjectRegistry>().and_then(|r| r.get(&project_id)) {
        Some(h) => crate::tasks::formations::repo_path(&h.pool).await.ok().flatten().map(std::path::PathBuf::from),
        None => None,
    };
    tauri::async_runtime::spawn_blocking(move || paths.iter().map(|p| super::refs::check(p, base.as_deref())).collect())
        .await
        .map_err(|e| e.to_string())
}

/// 最近出現、但還沒加入白名單的 LINE 來源（群組／使用者），給設定頁一鍵加入。
#[tauri::command]
pub async fn intake_line_unknown(db: State<'_, IntakeDb>) -> Result<Vec<super::line_store::Unknown>, String> {
    super::line_store::list_unknown(&db.pool).await.map_err(e)
}

/// 從「未加入白名單」清單移除一筆（加入白名單後，或使用者選擇忽略）。
#[tauri::command]
pub async fn intake_line_forget_unknown(db: State<'_, IntakeDb>, id: String) -> Result<bool, String> {
    super::line_store::forget_unknown(&db.pool, &id.to_lowercase()).await.map_err(e)
}

/// LINE 設定頁要的狀態：憑證是否已設定（憑證本身不外傳）、埠號、伺服器是否在聽。
#[tauri::command]
pub async fn intake_line_status(app: AppHandle) -> Result<super::line_server::Status, String> {
    Ok(super::line_server::status(&app).await)
}

/// 儲存 LINE 憑證。空字串代表「不變」（畫面不會回填已存的憑證）。存完重啟 webhook 伺服器。
#[tauri::command]
pub async fn intake_line_set_credentials(app: AppHandle, secrets: State<'_, Arc<crate::secret::SecretStore>>, channel_secret: String, access_token: String) -> Result<(), String> {
    for (id, value) in [(super::line_server::LINE_SECRET_ID, channel_secret), (super::line_server::LINE_TOKEN_ID, access_token)] {
        let v = value.trim();
        if !v.is_empty() {
            secrets.set(id, v).map_err(|e| e.to_string())?;
        }
    }
    super::line_server::restart(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn intake_line_clear_credentials(app: AppHandle, secrets: State<'_, Arc<crate::secret::SecretStore>>) -> Result<(), String> {
    for id in [super::line_server::LINE_SECRET_ID, super::line_server::LINE_TOKEN_ID] {
        secrets.delete(id).map_err(|e| e.to_string())?;
    }
    super::line_server::restart(&app).await;
    Ok(())
}
