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

/// 設定頁要的資料：該專案的設定，加上「是否存過」與「是不是 Telegram 預設專案」。
#[derive(Serialize)]
pub struct ConfigView {
    pub config: Config,
    pub saved: bool,
    pub telegram_default: bool,
}

#[tauri::command]
pub async fn intake_get_config(db: State<'_, IntakeDb>, project_id: String) -> Result<ConfigView, String> {
    let mut config = config::load(&db.pool, &project_id).await;
    let global = config::load_global(&db.pool).await;
    config.web_port = global.web_port; // 埠號是所有專案共用的
    Ok(ConfigView {
        saved: config::exists(&db.pool, &project_id).await,
        telegram_default: !project_id.is_empty() && global.telegram_default_project == project_id,
        config,
    })
}

/// 儲存前檢查會不會讓同一封信有兩個歸屬（同一信箱下別名／主旨標籤相同、專用信箱被共用）。
async fn check_conflicts(app: &AppHandle, pool: &sqlx::SqlitePool, project_id: &str, cfg: &Config) -> Result<(), String> {
    let mut others = super::open_projects(app);
    others.push((String::new(), "未歸屬".to_string()));
    for (id, name) in others {
        if id == project_id {
            continue;
        }
        let other = config::load(pool, &id).await;
        if let Some(field) = config::conflict(cfg, &other) {
            let what = match field {
                "dedicated" => "這個信箱已被設為另一個專案的專用信箱，同一個信箱不能同時給多個專案",
                "alias" => "收件別名跟另一個專案相同",
                _ => "主旨標籤跟另一個專案相同",
            };
            return Err(format!("與專案「{name}」衝突：{what}"));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn intake_set_config(app: AppHandle, db: State<'_, IntakeDb>, project_id: String, config: Config, telegram_default: bool) -> Result<(), String> {
    check_conflicts(&app, &db.pool, &project_id, &config).await?;
    let mut global = config::load_global(&db.pool).await;
    global.web_port = config.web_port;
    if telegram_default && !project_id.is_empty() {
        global.telegram_default_project = project_id.clone();
    } else if global.telegram_default_project == project_id {
        global.telegram_default_project.clear();
    }
    config::save_global(&db.pool, &global).await.map_err(e)?;
    config::save(&db.pool, &project_id, &config).await.map_err(e)?;
    // 網頁表單的開關／埠號可能變了。
    super::web::restart(&app).await;
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

#[derive(Serialize, Deserialize)]
pub struct SenderDto {
    pub address: String,
    pub level: Level,
    pub skip_auth: bool,
}

#[tauri::command]
pub async fn intake_senders(db: State<'_, IntakeDb>, project_id: String) -> Result<Vec<SenderDto>, String> {
    Ok(store::list_senders(&db.pool, &project_id).await.map_err(e)?.into_iter().map(|s| SenderDto { address: s.address, level: s.level, skip_auth: s.skip_auth }).collect())
}

#[tauri::command]
pub async fn intake_upsert_sender(db: State<'_, IntakeDb>, project_id: String, sender: SenderDto) -> Result<(), String> {
    if !super::rules::valid_address(&sender.address) {
        return Err("請輸入有效的電子郵件地址，或 Telegram 聊天室 tg:<chat_id>".into());
    }
    store::upsert_sender(&db.pool, &project_id, &Sender { address: sender.address, level: sender.level, skip_auth: sender.skip_auth }).await.map_err(e)
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
pub fn intake_tabs(rt: State<'_, Arc<IntakeRuntime>>) -> Vec<TabDto> {
    rt.registry.tabs().into_iter().map(|t| TabDto { project_id: rt.registry.project_of(&t).unwrap_or_default(), tab_id: t }).collect()
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

pub const PREVIEW_BYTES: usize = 64 * 1024;

pub fn text_preview(root: &std::path::Path, row: &store::AttachmentRow) -> Result<String, String> {
    use std::io::Read;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = std::path::Path::new(&row.path).canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("附件路徑不在允許的目錄內".into());
    }
    let mut buf = Vec::new();
    std::fs::File::open(&path).and_then(|f| f.take(PREVIEW_BYTES as u64 + 4).read_to_end(&mut buf)).map_err(|e| e.to_string())?;
    // 以內容判斷，不信資料庫裡的 mime 欄位。
    if !super::attach::looks_like_text(&buf) {
        return Err("這個檔案不是純文字，無法預覽".into());
    }
    let cut = buf.len() > PREVIEW_BYTES;
    buf.truncate(PREVIEW_BYTES);
    // 截斷可能切在多位元組字元中間：用 lossy 轉換，只會多一個替代字元。
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if cut {
        text.push_str("\n…（內容太長，只顯示前 64KB）");
    }
    Ok(text)
}

pub fn data_url(root: &std::path::Path, row: &store::AttachmentRow) -> Result<String, String> {
    use base64::Engine;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = std::path::Path::new(&row.path).canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("附圖路徑不在允許的目錄內".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    // 以內容判斷 MIME，不信資料庫裡的欄位。
    let (mime, _) = super::attach::sniff(&bytes).ok_or("檔案內容不是支援的圖片")?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
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
