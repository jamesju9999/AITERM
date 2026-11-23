//! AITERM（Tauri 指令）與 aiterm-intake-server（HTTP API）共用的操作：設定、白名單。
//! 兩邊只負責「怎麼收到請求、怎麼通知」，商業規則都在這裡，才能保證設定對等。
use crate::config::{self, Config};
use crate::rules::{self, Level, Sender};
use crate::{line_store, store};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

fn e(err: sqlx::Error) -> String {
    err.to_string()
}

/// 設定頁要的資料：該專案的設定，加上「是否存過」與「是不是 Telegram 預設專案」。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigView {
    pub config: Config,
    pub saved: bool,
    pub telegram_default: bool,
}

pub async fn get_config(pool: &SqlitePool, project_id: &str) -> ConfigView {
    let mut config = config::load(pool, project_id).await;
    let global = config::load_global(pool).await;
    config.web_port = global.web_port; // 埠號是所有專案共用的
    config.line_port = global.line_port;
    config.line_retention_days = global.line_retention_days;
    ConfigView {
        saved: config::exists(pool, project_id).await,
        telegram_default: !project_id.is_empty() && global.telegram_default_project == project_id,
        config,
    }
}

/// 與其他**專案**衝突就拒絕並指出是哪個；與升級前遺留的「未歸屬」設定衝突，則直接把那份舊設定停用。
/// 舊設定在畫面上看不到也關不掉，不能讓使用者被一個看不見的東西擋住；它的用途已經由新專案接手。
pub async fn resolve_conflicts(pool: &SqlitePool, project_id: &str, cfg: &Config, projects: &[(String, String)]) -> Result<(), String> {
    for (id, name) in projects {
        if id == project_id {
            continue;
        }
        let other = config::load(pool, id).await;
        if let Some(field) = config::conflict(cfg, &other) {
            let what = match field {
                "dedicated" => "這個信箱已被設為另一個專案的專用信箱，同一個信箱不能同時給多個專案",
                "alias" => "收件別名跟另一個專案相同",
                _ => "主旨標籤跟另一個專案相同",
            };
            return Err(format!("與專案「{name}」衝突：{what}"));
        }
    }
    if !project_id.is_empty() {
        let mut legacy = config::load(pool, "").await;
        if config::conflict(cfg, &legacy).is_some() {
            legacy.enabled = false;
            config::save(pool, "", &legacy).await.map_err(e)?;
        }
    }
    Ok(())
}

/// 儲存設定：清理參考資料夾、檢查歸屬衝突、同步全域埠號與保留天數、Telegram 預設專案。
/// 呼叫端存完之後自己負責重啟收訊與通知。
pub async fn set_config(pool: &SqlitePool, project_id: &str, config: Config, telegram_default: bool, projects: &[(String, String)]) -> Result<(), String> {
    let mut config = config;
    config.reference_folders = config.reference_folders.cleaned();
    resolve_conflicts(pool, project_id, &config, projects).await?;
    let mut global = config::load_global(pool).await;
    global.web_port = config.web_port;
    global.line_port = config.line_port;
    global.line_retention_days = config.line_retention_days;
    if telegram_default && !project_id.is_empty() {
        global.telegram_default_project = project_id.to_string();
    } else if global.telegram_default_project == project_id {
        global.telegram_default_project.clear();
    }
    config::save_global(pool, &global).await.map_err(e)?;
    config::save(pool, project_id, &config).await.map_err(e)?;
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SenderDto {
    pub address: String,
    pub level: Level,
    pub skip_auth: bool,
    /// 備註名稱（例如群組叫什麼）；沒有就是空字串。
    #[serde(default)]
    pub label: String,
}

pub async fn list_senders(pool: &SqlitePool, project_id: &str) -> Result<Vec<SenderDto>, String> {
    let labels = store::sender_labels(pool, project_id).await.map_err(e)?;
    Ok(store::list_senders(pool, project_id)
        .await
        .map_err(e)?
        .into_iter()
        .map(|s| {
            let label = labels.get(&rules::normalize(&s.address)).cloned().unwrap_or_default();
            SenderDto { address: s.address, level: s.level, skip_auth: s.skip_auth, label }
        })
        .collect())
}

pub async fn upsert_sender(pool: &SqlitePool, project_id: &str, sender: SenderDto) -> Result<(), String> {
    if !rules::valid_address(&sender.address) {
        return Err("請輸入有效的電子郵件地址，、Telegram 聊天室 tg:<chat_id>，或 LINE userId（U 開頭共 33 字元）".into());
    }
    let address = sender.address.clone();
    store::upsert_sender(pool, project_id, &Sender { address: sender.address, level: sender.level, skip_auth: sender.skip_auth }).await.map_err(e)?;
    // 有給備註就存；沒給時，LINE 群組／使用者若已知道名稱（LINE 查得到或「最近出現」記過），就用那個名稱，免得清單上只剩一串編號。
    let mut label = sender.label;
    if label.trim().is_empty() && store::sender_label(pool, project_id, &address).await.map_err(e)?.is_none() {
        if let Some(id) = rules::normalize(&address).strip_prefix("line:") {
            label = line_store::get_name(pool, id).await.map_err(e)?.or(line_store::unknown_name(pool, id).await.map_err(e)?).unwrap_or_default();
        }
    }
    if !label.trim().is_empty() {
        store::set_sender_label(pool, project_id, &address, &label).await.map_err(e)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        p
    }

    #[tokio::test]
    async fn config_round_trips_and_mirrors_the_global_ports() {
        let p = pool().await;
        let cfg = Config { line_port: 9123, line_retention_days: 7, ..Config::default() };
        set_config(&p, "p1", cfg, false, &[("p1".into(), "甲".into())]).await.unwrap();
        let v = get_config(&p, "p1").await;
        assert!(v.saved);
        assert_eq!((v.config.line_port, v.config.line_retention_days), (9123, 7));
        assert!(!get_config(&p, "p2").await.saved, "沒存過的專案不算已儲存");
    }

    #[tokio::test]
    async fn telegram_default_is_set_and_cleared() {
        let p = pool().await;
        set_config(&p, "p1", Config::default(), true, &[]).await.unwrap();
        assert!(get_config(&p, "p1").await.telegram_default);
        set_config(&p, "p1", Config::default(), false, &[]).await.unwrap();
        assert!(!get_config(&p, "p1").await.telegram_default);
    }

    #[tokio::test]
    async fn an_invalid_sender_address_is_refused_and_labels_are_kept() {
        let p = pool().await;
        let bad = SenderDto { address: "not an address".into(), level: Level::External, skip_auth: false, label: String::new() };
        assert!(upsert_sender(&p, "p1", bad).await.is_err());
        let ok = SenderDto { address: "bob@corp.com".into(), level: Level::Internal, skip_auth: false, label: "小明".into() };
        upsert_sender(&p, "p1", ok.clone()).await.unwrap();
        assert_eq!(list_senders(&p, "p1").await.unwrap(), vec![ok]);
    }
}
